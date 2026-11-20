import crypto from 'node:crypto';
const base = process.env.BASE_URL || 'http://localhost:4173';
const adminKey = process.env.ADMIN_KEY || 'test-only-secret';
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const sessionDir = fs.mkdtempSync(path.join(os.tmpdir(), 'durak-mcp-test-'));
const sessionFile = path.join(sessionDir, 'sessions.json');
let failures = 0;

function check(label, condition, extra = '') {
  if (condition) {
    console.log(`ok   ${label}`);
  } else {
    failures += 1;
    console.log(`FAIL ${label} ${extra}`);
  }
}

const child = spawn(process.execPath, [path.join(root, 'mcp/server.mjs')], {
  cwd: root,
  stdio: ['pipe', 'pipe', 'pipe'],
  env: { ...process.env, DURAK_URL: base, DURAK_ADMIN_KEY: adminKey, ADMIN_KEY: adminKey, DURAK_MCP_SESSIONS: sessionFile },
});

let buffer = '';
let nextId = 1;
let stderrText = '';
const pending = new Map();

child.stdout.setEncoding('utf8');
child.stdout.on('data', chunk => {
  buffer += chunk;
  let index;
  while ((index = buffer.indexOf('\n')) !== -1) {
    const line = buffer.slice(0, index).trim();
    buffer = buffer.slice(index + 1);
    if (!line) continue;
    let message;
    try { message = JSON.parse(line); } catch (error) { continue; }
    if (message.id !== undefined && pending.has(message.id)) {
      const settle = pending.get(message.id);
      pending.delete(message.id);
      settle(message);
    }
  }
});
child.stderr.setEncoding('utf8');
child.stderr.on('data', chunk => { stderrText += chunk; });

function rpc(method, params, timeout = 20000) {
  const id = nextId;
  nextId += 1;
  child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id, method, params })}\n`);
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      pending.delete(id);
      reject(new Error(`timeout ${method}`));
    }, timeout);
    pending.set(id, message => {
      clearTimeout(timer);
      if (message.error) reject(new Error(message.error.message));
      else resolve(message.result);
    });
  });
}

async function call(name, args = {}, timeout = 20000) {
  const result = await rpc('tools/call', { name, arguments: args }, timeout);
  return {
    result,
    text: result?.content?.[0]?.text || '',
    data: result?.structuredContent || {},
    isError: Boolean(result?.isError),
  };
}

function finish() {
  child.stdin.end();
  const timer = setTimeout(() => child.kill('SIGKILL'), 2000);
  child.on('exit', () => {
    clearTimeout(timer);
    fs.rmSync(sessionDir, { recursive: true, force: true });
    if (failures) {
      if (stderrText) console.log(stderrText.trim());
      process.exit(1);
    }
    process.exit(0);
  });
}

try {
  const init = await rpc('initialize', { protocolVersion: '2024-11-05', capabilities: {}, clientInfo: { name: 'tests/mcp.mjs', version: '1.0.0' } });
  check('initialize: server name', init?.serverInfo?.name === 'durak', JSON.stringify(init?.serverInfo));
  check('initialize: tools capability', Boolean(init?.capabilities?.tools));
  child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', method: 'notifications/initialized' })}\n`);

  const list = await rpc('tools/list', {});
  const names = (list?.tools || []).map(tool => tool.name);
  for (const expected of ['durak_connect', 'durak_room_create', 'durak_room_join', 'durak_start', 'durak_action', 'durak_wait', 'durak_watch', 'durak_follow', 'durak_state', 'durak_hints', 'durak_turn', 'durak_admin', 'durak_autoplay']) {
    check(`tools/list: ${expected}`, names.includes(expected));
  }
  const readOnly = (list?.tools || []).filter(tool => tool.annotations?.readOnlyHint).map(tool => tool.name);
  for (const expected of ['durak_state', 'durak_hints', 'durak_turn', 'durak_seats', 'durak_watch']) {
    check(`tools/list: ${expected} помечен read-only`, readOnly.includes(expected), JSON.stringify(readOnly));
  }
  const turnAnnotations = (list?.tools || []).find(tool => tool.name === 'durak_action')?.annotations;
  check('tools/list: durak_action помечен изменяющим', turnAnnotations?.readOnlyHint === false, JSON.stringify(turnAnnotations));

  const createProbe = await call('durak_connect', { seat: 'probe', create: true });
  const canCreate = !createProbe.isError;
  await call('durak_disconnect', { seat: 'probe' }).catch(() => {});
  const clubLogin = process.env.MCP_CLUB_LOGIN || '';
  const clubPassword = process.env.MCP_CLUB_PASSWORD || '';
  const reference = !canCreate && !clubLogin;
  const connectArgs = seat => (clubLogin && clubPassword ? { login: clubLogin, password: clubPassword } : canCreate ? { create: true } : { clientId: `mcp-${Date.now()}-${seat}`, token: crypto.randomBytes(32).toString('hex') });
  const first = await call('durak_connect', { seat: 'p1', name: 'MCP Тест 1', ...connectArgs() });
  check('connect p1', !first.isError && first.data.connected === true, first.text);
  check('connect p1: стартовый баланс', first.data.account?.balance === 1000, JSON.stringify(first.data.account));

  const second = await call('durak_connect', { seat: 'p2', name: 'MCP Тест 2', ...connectArgs() });
  check('connect p2', !second.isError && second.data.connected === true, second.text);

  const duplicate = await call('durak_connect', { seat: 'p1' });
  check('connect: занятое место отклонено', duplicate.isError, duplicate.text);

  const silentWait = await call('durak_wait', { seat: 'p1', until: 'finished', timeoutMs: 200 });
  check('wait: таймаут без ошибки', !silentWait.isError && silentWait.data.hit === false, silentWait.text);

  const ids = { p1: first.data.clientId, p2: second.data.clientId };

  const created = await call('durak_room_create', { seat: 'p1', name: 'MCP проверка', bet: 0 });
  const code = created.data.code;
  check('room_create: код стола', !created.isError && /^[0-9A-F]{6}$/.test(code || ''), created.text);

  const joined = await call('durak_room_join', { seat: 'p2', code: code?.toLowerCase() });
  check('room_join: два игрока', !joined.isError && joined.data.state?.players?.length === 2, joined.text);
  check('room_join: код в верхнем регистре', joined.data.state?.room?.id === code, JSON.stringify(joined.data.state?.room));

  const waiting = await call('durak_state', { seat: 'p1' });
  check('state: стол ждёт', waiting.data.state?.room?.status === 'waiting', waiting.text);
  check('state: рука пуста до старта', waiting.data.state?.hand?.length === 0);
  check('state: canStart у хоста', waiting.data.state?.canStart === true);

  const badAction = await call('durak_action', { seat: 'p1', action: 'attack', cardId: 'ZZ' });
  check('action: неизвестная карта отклонена', badAction.isError, badAction.text);

  const started = await call('durak_start', { seat: 'p1' });
  check('start: партия идёт', started.data.state?.room?.status === 'playing', started.text);
  check('start: по шесть карт', started.data.state?.players?.every(player => player.cards === 6), JSON.stringify(started.data.state?.players));
  check('start: в колоде 24 карты', started.data.state?.deck === 24, String(started.data.state?.deck));

  const attackerSeat = started.data.state?.attackerId === ids.p1 ? 'p1' : 'p2';
  const hinted = await call('durak_hints', { seat: attackerSeat });
  check('hints: есть чем атаковать', hinted.data.attack?.length > 0, hinted.text);
  check('hints: рука в подсказке', hinted.data.hand?.length === 6, hinted.text);

  const defenderSeat = attackerSeat === 'p1' ? 'p2' : 'p1';
  const attackCard = hinted.data.attack[0].id;
  const watchPromise = call('durak_watch', { seat: defenderSeat, until: 'your-turn', totalMs: 8000, chunkMs: 3000 });
  await new Promise(resolve => setTimeout(resolve, 200));
  const attacked = await call('durak_action', { seat: attackerSeat, action: 'attack', cardId: attackCard });
  check('action: атака принята', !attacked.isError, attacked.text);
  const watched = await watchPromise;
  check('watch: хук сработал на ход соперника', !watched.isError && watched.data.hit === true, watched.text);
  check('watch: ответ через серверный хук', watched.data.via === 'hook', watched.text);
  check('watch: карта на столе', String(watched.data.snapshot?.table || '').includes(attackCard), JSON.stringify(watched.data.snapshot?.table));
  check('watch: краткий ответ без полного снимка', watched.data.state === undefined && watched.data.snapshot?.room === code, JSON.stringify(Object.keys(watched.data)));
  check('watch: журнал действия', (watched.data.events || []).some(event => event.action === 'attack' && event.card === attackCard), JSON.stringify(watched.data.events));
  check('watch: подсказки в отчете', watched.text.includes('Хук') && watched.text.includes('Рука:'), watched.text);

  const lateWatch = await call('durak_watch', { seat: defenderSeat, until: 'your-turn', totalMs: 600, chunkMs: 600 });
  check('watch: ход уже был до вызова', !lateWatch.isError && lateWatch.data.hit === true, lateWatch.text);
  check('watch: ответ из накопленного состояния', lateWatch.data.via === 'cache', lateWatch.text);
  check('watch: без ожидания таймаута', Number(lateWatch.data.waitedMs) < 400, String(lateWatch.data.waitedMs));

  const taken = await call('durak_action', { seat: defenderSeat, action: 'take' });
  check('action: взять карты принято', !taken.isError, taken.text);
  const backlog = await call('durak_watch', { seat: attackerSeat, until: 'any', totalMs: 600, chunkMs: 600 });
  check('watch: пропущенное действие отдаётся сразу', !backlog.isError && backlog.data.hit === true && backlog.data.via === 'cache', backlog.text);
  check('watch: пропущенное «взять» в журнале', (backlog.data.events || []).some(event => event.action === 'take'), JSON.stringify(backlog.data.events));

  const seatList = await call('durak_seats');
  check('seats: два места', seatList.data.seats?.length === 2, seatList.text);

  const lobby = await call('durak_lobby', { seat: 'p1' });
  check('lobby: стол виден', (lobby.data.rooms || []).some(room => room.id === code), lobby.text);

  const followTimeout = 120000;
  const followP1 = call('durak_follow', { seat: 'p1', until: 'finished', totalMs: 90000, maxSteps: 400 }, followTimeout);
  const followP2 = call('durak_follow', { seat: 'p2', until: 'finished', totalMs: 90000, maxSteps: 400 }, followTimeout);
  const [followed1, followed2] = await Promise.all([followP1, followP2]);
  check('follow: партия доиграна без вмешательства', followed1.data.finished === true || followed2.data.finished === true, `${followed1.text}\n${followed2.text}`);
  check('follow: дурак определён', typeof (followed1.data.loser || followed2.data.loser) === 'string', `${followed1.data.loser} / ${followed2.data.loser}`);
  check('follow: ходы в отчёте', (followed1.data.actions || []).length + (followed2.data.actions || []).length > 0, JSON.stringify(followed1.data.actions));
  check('follow: краткий отчёт', followed1.text.length < 4000 && followed2.text.length < 4000, `${followed1.text.length}/${followed2.text.length}`);

  const adminList = await call('durak_admin', { op: 'list' });
  const adminIds = (adminList.data.players || []).map(player => player.id);
  check('admin list с ключом', !adminList.isError && adminIds.includes(ids.p1) && adminIds.includes(ids.p2), adminList.text);

  const status = await call('durak_status');
  check('status: сервис отвечает', !status.isError && Number.isInteger(status.data.online), status.text);

  const auto = canCreate ? await call('durak_autoplay', { seats: 3, mode: 'transfer', deckSize: 36, bet: 100, maxSteps: 400 }) : { data: {}, text: '', isError: false };
  if (!canCreate) {
    console.log('autoplay пропущен: сервис не умеет создавать клубные аккаунты');
  } else {
    check('autoplay: партия завершена', auto.data.finished === true && auto.data.steps > 0, auto.text);
    check('autoplay: дурак определён', typeof auto.data.loser === 'string' && auto.data.loser.length > 0, auto.text);
    check('autoplay: стол и колода пусты', auto.data.deck === 0 && auto.data.table === 0, JSON.stringify({ deck: auto.data.deck, table: auto.data.table, hands: auto.data.hands }));
    const balanceSum = (auto.data.balances || []).reduce((sum, item) => sum + (item.balance || 0), 0);
    check('autoplay: балансы сохранили банк', balanceSum === 3000, String(balanceSum));
  }

  const afterAuto = await call('durak_seats');
  check('autoplay: места ботов закрыты', (afterAuto.data.seats || []).every(seat => !seat.seat.startsWith('auto')), afterAuto.text);

  const left = await call('durak_disconnect', { seat: 'p2', leave: true });
  check('disconnect: место закрыто', !left.isError && left.data.seat === 'p2', left.text);
  const leavingHost = await call('durak_disconnect', { seat: 'p1', leave: true });
  check('disconnect: хост закрыт', !leavingHost.isError, leavingHost.text);
  const empty = await call('durak_seats');
  check('seats: пусто после закрытия', (empty.data.seats || []).length === 0, empty.text);

  const noSeat = await call('durak_state');
  check('state без места: понятная ошибка', noSeat.isError, noSeat.text);
} catch (error) {
  failures += 1;
  console.log(`FAIL ${error.message}`);
  if (stderrText) console.log(stderrText.trim());
}

finish();