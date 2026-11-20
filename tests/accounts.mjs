import http from 'node:http';
import crypto from 'node:crypto';
import { adminKey, base, testBackend } from './helpers/accounts.mjs';

let failures = 0;
const createdIds = new Set();
const sessionsForCleanup = new Map();
const passwordsForCleanup = new Map();

function check(label, condition, extra = '') {
  console.log(`${condition ? 'ok  ' : 'FAIL'} ${label}${condition || !extra ? '' : ` ${extra}`}`);
  if (!condition) failures += 1;
}

async function post(path, body) {
  const response = await fetch(base + path, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) });
  const data = await response.json().catch(() => ({}));
  if (path === '/api/admin' && body.op === 'create_temp' && response.status === 200 && data.account?.id) {
    createdIds.add(data.account.id);
    passwordsForCleanup.set(data.account.id, { name: data.account.name, password: body.password || data.account.password });
  }
  if (path === '/api/admin' && body.op === 'update' && response.status === 200 && passwordsForCleanup.has(body.id)) {
    const credentials = passwordsForCleanup.get(body.id);
    if (body.name) credentials.name = body.name;
    if (body.password) credentials.password = body.password;
  }
  if (path === '/api/admin' && body.op === 'delete' && response.status === 200) createdIds.delete(body.id);
  if (path === '/api/auth/temp/login' && response.status === 200 && data.clientId && data.token) sessionsForCleanup.set(data.clientId, data.token);
  return { status: response.status, data };
}

function status() {
  return new Promise((resolve, reject) => {
    http.get(`${base}/api/status`, response => {
      let body = '';
      response.on('data', chunk => { body += chunk; });
      response.on('end', () => { try { resolve(JSON.parse(body)); } catch (error) { reject(error); } });
    }).on('error', reject);
  });
}

function sse(id, token, name) {
  return new Promise((resolve, reject) => {
    const seen = { lobby: 0, online: 0, last: null };
    const request = http.get(`${base}/events?id=${encodeURIComponent(id)}&token=${token}&name=${encodeURIComponent(name)}`, response => {
      if (response.statusCode !== 200) { response.resume(); return resolve({ request, seen, status: response.statusCode }); }
      let buffer = '';
      response.setEncoding('utf8');
      response.on('data', chunk => {
        buffer += chunk;
        let index;
        while ((index = buffer.indexOf('\n\n')) !== -1) {
          const block = buffer.slice(0, index);
          buffer = buffer.slice(index + 2);
          const line = block.split('\n').find(item => item.startsWith('data: '));
          if (!line) continue;
          try {
            const payload = JSON.parse(line.slice(6));
            seen.last = payload;
            if (payload.type === 'lobby') { seen.lobby += 1; seen.online = payload.online; }
          } catch (error) {}
        }
      });
      resolve({ request, seen, status: response.statusCode });
    });
    request.on('error', reject);
  });
}

function waitFor(predicate, timeout = 4000) {
  const started = Date.now();
  return new Promise((resolve, reject) => {
    const tick = () => {
      if (predicate()) return resolve();
      if (Date.now() - started > timeout) return reject(new Error('timeout'));
      setTimeout(tick, 25);
    };
    tick();
  });
}

const suffix = crypto.randomBytes(3).toString('hex');
const names = [`гейт-a-${suffix}`, `гейт-b-${suffix}`];
const passwords = names.map(name => `пароль-${name}`);
const streams = [];

await testBackend();

try {
  const page = await fetch(`${base}/admin.html`);
  const html = page.ok ? await page.text() : '';
  check('страница панели отдаётся', page.ok && /id="adminLogin"/.test(html) && /id="adminList"/.test(html), String(page.status));
  const lobby = await (await fetch(`${base}/`)).text();
  check('на главной нет кнопки панели', !/vhod-administratora|loginAdmin|data-view="admin"/.test(lobby));
  check('на главной есть личный кабинет', /data-view="cabinet"/.test(lobby));

  const before = await status();
  const accounts = [];
  for (const [index, name] of names.entries()) {
    const created = await post('/api/admin', { key: adminKey, op: 'create_temp', name, password: passwords[index] });
    check(`аккаунт ${name} создан`, created.status === 200 && created.data.account?.name === name, `HTTP ${created.status}`);
    accounts.push(created.data.account || {});
  }
  const duplicate = await post('/api/admin', { key: adminKey, op: 'create_temp', name: names[0], password: 'другой-пароль' });
  check('занятое имя не дублируется', duplicate.status === 409, String(duplicate.status));

  const wrong = await post('/api/auth/temp/login', { name: names[0], password: 'не-то' });
  check('неверный пароль отклонён', wrong.status === 403, String(wrong.status));

  const upper = await post('/api/auth/temp/login', { name: names[1].toUpperCase(), password: passwords[1] });
  check('имя входит без учёта регистра', upper.status === 200, String(upper.status));

  const sessions = [];
  for (const [index, name] of names.entries()) {
    const login = await post('/api/auth/temp/login', { name, password: passwords[index] });
    const valid = login.status === 200 && /^temp:[A-Za-z0-9_-]+$/.test(login.data.clientId || '') && /^[a-f0-9]{64}$/.test(login.data.token || '');
    check(`вход ${name}`, valid, `HTTP ${login.status}`);
    sessions.push(login.data);
  }

  const streams2 = [];
  for (const [index, session] of sessions.entries()) {
    const stream = await sse(session.clientId, session.token, names[index]);
    streams.push(stream.request);
    streams2.push(stream.seen);
    check(`SSE ${names[index]} открыт`, stream.status === 200 && stream.seen.lobby > 0, String(stream.status));
  }

  const owner = sessions[0];
  const token = session => ({ token: session.token });
  const room = await post('/api/rooms', { clientId: owner.clientId, ...token(owner), name: 'Стол клуба', mode: 'classic', deckSize: 36, bet: 50 });
  check('стол со ставкой создан', room.status === 201 && room.data.room?.bet === 50, String(room.status));
  const joined = await post('/api/join', { clientId: sessions[1].clientId, ...token(sessions[1]), roomId: room.data.code });
  check('второй игрок вошёл', joined.status === 200, String(joined.status));
  const started = await post('/api/action', { clientId: owner.clientId, ...token(owner), action: 'start' });
  check('партия началась, ставки списаны', started.status === 200, String(started.status));

  await waitFor(() => streams2[0].lobby > 1);
  const list = await post('/api/admin', { key: adminKey, op: 'list' });
  const player = (list.data.players || []).find(item => item.id === owner.clientId);
  check('панель видит стол игрока', list.status === 200 && player && player.balance === 950, String(list.status));

  const funded = await post('/api/admin', { key: adminKey, op: 'balance', id: owner.clientId, delta: 500 });
  check('пополнение баланса', funded.status === 200 && funded.data.player.balance === 1450, String(funded.status));
  await waitFor(() => streams2[0].last && streams2[0].last.type === 'account' && streams2[0].last.account.balance === 1450);
  check('игрок получил новый баланс по SSE', streams2[0].last.account.balance === 1450);

  const renamed = await post('/api/admin', { key: adminKey, op: 'update', id: owner.clientId, name: `клуб-${suffix}` });
  check('переименование из панели', renamed.status === 200 && renamed.data.player.name === `клуб-${suffix}`, String(renamed.status));
  const password = await post('/api/admin', { key: adminKey, op: 'update', id: owner.clientId, password: 'новый-пароль' });
  check('пароль меняется', password.status === 200, String(password.status));
  const relogin = await post('/api/auth/temp/login', { name: 'клуб-' + suffix, password: 'новый-пароль' });
  check('вход с новым паролем', relogin.status === 200, `HTTP ${relogin.status}`);

  const generated = await post('/api/admin', { key: adminKey, op: 'create_temp', name: `авто-${suffix}` });
  const generatedPassword = String(generated.data.account?.password || '');
  const generatedLogin = generatedPassword ? await post('/api/auth/temp/login', { name: `авто-${suffix}`, password: generatedPassword }) : { status: 0 };
  check('пароль генерируется автоматически', generated.status === 200 && generatedPassword.length >= 12 && generatedLogin.status === 200, String(generated.status));

  const blankPassword = await post('/api/admin', { key: adminKey, op: 'create_temp', name: `пусто-${suffix}`, password: '   ' });
  check('короткий пароль не принимается', blankPassword.status === 400, String(blankPassword.status));

  const revoked = sessions[0];
  const revokeStream = await sse(revoked.clientId, revoked.token, names[0]);
  streams.push(revokeStream.request);
  const changed = await post('/api/admin', { key: adminKey, op: 'update', id: revoked.clientId, password: 'ещё-один-пароль' });
  const afterChange = await post('/api/rooms', { clientId: revoked.clientId, token: revoked.token, name: 'Проверка', bet: 0 });
  check('смена пароля закрывает старую сессию', changed.status === 200 && afterChange.status === 403, `${changed.status}/${afterChange.status}`);
  const reloginOld = await post('/api/auth/temp/login', { name: `клуб-${suffix}`, password: 'новый-пароль' });
  check('старый пароль отклонён после смены', reloginOld.status === 403, String(reloginOld.status));
  await post('/api/auth/temp/login', { name: `клуб-${suffix}`, password: 'ещё-один-пароль' });

  const doomed = changed.data.player?.id || revoked.clientId;
  const removed = await post('/api/admin', { key: adminKey, op: 'delete', id: doomed });
  const removedLogin = await post('/api/auth/temp/login', { name: `клуб-${suffix}`, password: 'ещё-один-пароль' });
  const removedList = await post('/api/admin', { key: adminKey, op: 'list' });
  check('учётная запись удаляется', removed.status === 200 && removedLogin.status === 403 && !(removedList.data.players || []).some(item => item.id === doomed), String(removed.status));
  const removedAgain = await post('/api/admin', { key: adminKey, op: 'delete', id: doomed });
  check('повторное удаление даёт 404', removedAgain.status === 404, String(removedAgain.status));
  if (revokeStream.request) revokeStream.request.destroy();

  const reset = await post('/api/admin', { key: adminKey, op: 'reset', id: sessions[1].clientId });
  check('сброс прогресса', reset.status === 200 && reset.data.player.balance === 1000 && reset.data.player.games === 0, String(reset.status));

  const after = await status();
  check('сервер продолжает отвечать', Array.isArray(after.rooms) && after.online >= before.online - 1, String(after.online));

} catch (error) {
  check('проверки клуба завершились', false, error.message === 'timeout' ? 'таймаут' : 'ошибка запроса или ответа');
} finally {
  for (const clientId of createdIds) {
    try {
      const credentials = passwordsForCleanup.get(clientId);
      if (credentials?.name && credentials?.password) {
        const login = await post('/api/auth/temp/login', credentials);
        if (login.status !== 200 || login.data.clientId !== clientId || !login.data.token) throw new Error(`вход для очистки: HTTP ${login.status}`);
      }
      const token = sessionsForCleanup.get(clientId);
      if (!token) continue;
      const result = await post('/api/leave', { clientId, token });
      if (result.status !== 200 || result.data.ok !== true) throw new Error(`HTTP ${result.status}`);
    } catch (error) {
      check('очистка тестового стола', false, error.message);
    }
  }
  for (const request of streams) request.destroy();
  for (const id of [...createdIds]) {
    try {
      const result = await post('/api/admin', { key: adminKey, op: 'delete', id });
      if (result.status !== 200 || result.data.deleted !== id) throw new Error(`HTTP ${result.status}`);
    } catch (error) {
      check('очистка тестового аккаунта', false, error.message);
    }
  }
}

console.log(failures ? `\n${failures} проверок провалено` : '\nклубный вход и панель работают');
process.exitCode = failures ? 1 : 0;
