import { registerClubAccounts, referenceIds } from './helpers/accounts.mjs';
const base = process.env.BASE_URL || 'http://localhost:4173';
const players = {};
let failures = 0;
import http from 'node:http';
import crypto from 'node:crypto';

function tokenFor(id) {
  return tokens.get(id) || '';
}

function sse(url, onMessage, onError) {
  const request = http.get(url, response => {
    let buffer = '';
    response.setEncoding('utf8');
    response.on('data', chunk => {
      buffer += chunk;
      let index = buffer.indexOf('\n\n');
      while (index !== -1) {
        const block = buffer.slice(0, index);
        buffer = buffer.slice(index + 2);
        const line = block.split('\n').find(item => item.startsWith('data: '));
        if (line) {
          try {
            onMessage(JSON.parse(line.slice(6)));
          } catch (error) {}
        }
        index = buffer.indexOf('\n\n');
      }
    });
  });
  request.on('error', onError);
  return request;
}

function check(label, condition, extra = '') {
  if (condition) {
    console.log(`ok   ${label}`);
  } else {
    failures += 1;
    console.log(`FAIL ${label} ${extra}`);
  }
}

function stateOf(id) {
  return players[id] ? players[id].state : null;
}

async function post(path, body) {
  const requestBody = body.clientId ? { ...body, token: body.token || tokenFor(body.clientId) } : body;
  const response = await fetch(base + path, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(requestBody) });
  return { status: response.status, data: await response.json().catch(() => ({})) };
}

function connect(id, name) {
  return new Promise((resolve, reject) => {
    players[id] = { state: null, source: null };
    players[id].source = sse(`${base}/events?id=${encodeURIComponent(id)}&token=${tokenFor(id)}&name=${encodeURIComponent(name)}`, payload => {
      if (payload.type === 'game') players[id].state = payload.state;
    }, reject);
    setTimeout(resolve, 400);
  });
}

function waitFor(predicate, timeout = 2000) {
  const started = Date.now();
  return new Promise((resolve, reject) => {
    const tick = () => {
      const value = predicate();
      if (value) return resolve(value);
      if (Date.now() - started > timeout) return reject(new Error('timeout'));
      setTimeout(tick, 40);
    };
    tick();
  });
}

function watch(id, { since, until = 'any', timeout = 3000, token } = {}) {
  const url = new URL('/api/watch', base);
  url.searchParams.set('id', id);
  url.searchParams.set('token', token || tokenFor(id));
  if (since !== undefined) url.searchParams.set('since', String(since));
  url.searchParams.set('until', until);
  url.searchParams.set('timeout', String(timeout));
  return fetch(url).then(async response => ({ status: response.status, data: await response.json().catch(() => ({})) }));
}

const labels = await registerClubAccounts(['a', 'b']);
const tokens = new Map(labels.map(item => [item.clientId, item.token]));
const a = labels[0].clientId, b = labels[1].clientId;

try {
  await connect(a, 'Хук A');
  await connect(b, 'Хук B');

  const created = await post('/api/rooms', { clientId: a, name: 'Хуки', bet: 0 });
  check('стол создан', created.status === 201 && /^[0-9A-F]{6}$/.test(created.data.code || ''), JSON.stringify(created.data));
  const code = created.data.code;

  const joined = await post('/api/join', { clientId: b, roomId: code, playerName: 'Хук B' });
  check('второй игрок вошёл', joined.status === 200, JSON.stringify(joined.data));

  await waitFor(() => stateOf(a)?.players?.length === 2 && stateOf(b)?.players?.length === 2);

  const started = await post('/api/action', { clientId: a, action: 'start' });
  check('партия началась', started.status === 200 && started.data.ok === true, JSON.stringify(started.data));
  await waitFor(() => stateOf(a)?.room?.status === 'playing' && stateOf(b)?.room?.status === 'playing');

  const first = stateOf(a);
  check('снимок содержит seq', Number.isInteger(first.seq) && first.seq > 0, String(first.seq));
  check('снимок содержит журнал', Array.isArray(first.log) && first.log.length > 0, JSON.stringify(first.log));

  const attackerId = first.attackerId;
  const defenderId = first.defenderId;
  const attackerSeat = attackerId === a ? a : b;
  const defenderSeat = defenderId === a ? a : b;

  const silent = await watch(attackerSeat, { since: stateOf(attackerSeat).seq, until: 'any', timeout: 300 });
  check('watch: тишина даёт hit=false', silent.status === 200 && silent.data.hit === false, JSON.stringify(silent.data));
  check('watch: снимок в таймауте', silent.data.state?.room?.id === code, JSON.stringify(silent.data.state?.room));

  const withToken = await watch(attackerSeat, { token: crypto.randomBytes(32).toString('hex'), timeout: 300 });
  check('watch: чужой токен отклонён', withToken.status === 403, String(withToken.status));

  const noId = await fetch(`${base}/api/watch?until=any&timeout=300`);
  check('watch: без id ошибка', noId.status === 400, String(noId.status));

  const badUntil = await watch(attackerSeat, { until: 'what', timeout: 300 });
  check('watch: неизвестное until отклонено', badUntil.status === 400, String(badUntil.status));

  const attackCard = stateOf(attackerSeat).hand[0];
  const attacked = await post('/api/action', { clientId: attackerSeat, action: 'attack', cardId: attackCard.id });
  check('атакующий подкинул карту', attacked.status === 200 && attacked.data.ok === true, JSON.stringify(attacked.data));
  await waitFor(() => stateOf(defenderSeat)?.canTake === true);
  check('снимок защитника: можно взять', stateOf(defenderSeat).canTake === true);
  check('журнал: карта хода', stateOf(defenderSeat).log.some(event => event.action === 'attack' && event.cardId === attackCard.id), JSON.stringify(stateOf(defenderSeat).log));

  const turnWatch = watch(attackerSeat, { since: stateOf(attackerSeat).seq, until: 'your-turn', timeout: 6000 });
  const took = await post('/api/action', { clientId: defenderSeat, action: 'take' });
  check('защитник взял карты', took.status === 200 && took.data.ok === true, JSON.stringify(took.data));
  const turned = await turnWatch;
  check('watch: хук поймал свой ход', turned.data.hit === true && turned.data.until === 'your-turn', JSON.stringify({ hit: turned.data.hit }));
  check('watch: ход у атакующего', turned.data.state?.canPass === true, JSON.stringify(turned.data.state?.you));
  check('watch: журнал показывает взятие', (turned.data.events || []).some(event => event.action === 'take' && event.actorId === defenderId), JSON.stringify(turned.data.events));
  check('watch: seq вырос', turned.data.seq > Number(turned.data.since), JSON.stringify({ seq: turned.data.seq, since: turned.data.since }));

  const replay = await watch(attackerSeat, { since: stateOf(attackerSeat).seq - 1, until: 'any', timeout: 300 });
  check('watch: пропущенное событие отдаётся сразу', replay.data.hit === true, JSON.stringify({ hit: replay.data.hit }));

  const finishedWatch = watch(attackerSeat, { since: stateOf(attackerSeat).seq, until: 'finished', timeout: 5000 });
  const left = await post('/api/leave', { clientId: defenderSeat });
  check('защитник покинул партию', left.status === 200, JSON.stringify(left.data));
  const finished = await finishedWatch;
  check('watch: партия завершена по уходу', finished.data.hit === true && finished.data.state?.room?.status === 'finished', JSON.stringify(finished.data.state?.room));
  check('watch: дурак — ушедший', finished.data.state?.finished?.loserId === defenderId, JSON.stringify(finished.data.state?.finished));

  players[a].source?.destroy();
  players[b].source?.destroy();
} catch (error) {
  failures += 1;
  console.log(`FAIL ${error.stack || error.message}`);
  for (const player of Object.values(players)) player.source?.destroy();
}

if (failures) {
  console.log(`\n${failures} проверок не прошло`);
  process.exit(1);
}
console.log('hooks: все проверки прошли');