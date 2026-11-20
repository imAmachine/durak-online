import { registerClubAccounts } from './helpers/accounts.mjs';
import http from 'node:http';
const base = process.env.BASE_URL || 'http://localhost:4173';
function tokenFor(id) {
  return tokens.get(id) || '';
}
const players = {};
let failures = 0;
function check(label, ok, extra = '') {
  if (ok) console.log(`ok   ${label}`);
  else { failures += 1; console.log(`FAIL ${label} ${extra}`); }
}
function sse(url, onMessage) {
  return http.get(url, res => {
    let buf = '';
    res.setEncoding('utf8');
    res.on('data', chunk => {
      buf += chunk;
      let i;
      while ((i = buf.indexOf('\n\n')) !== -1) {
        const line = buf.slice(0, i).split('\n').find(l => l.startsWith('data: '));
        buf = buf.slice(i + 2);
        if (line) { try { onMessage(JSON.parse(line.slice(6))); } catch (e) {} }
      }
    });
  });
}
function connect(id) { return new Promise(r => { players[id] = {}; players[id].req = sse(`${base}/events?id=${encodeURIComponent(id)}&token=${tokenFor(id)}`, p => { if (p.type === 'game') players[id].state = p.state; }); setTimeout(r, 300); }); }
async function post(path, body) {
  const requestBody = body.clientId ? { ...body, token: body.token || tokenFor(body.clientId) } : body;
  const r = await fetch(base + path, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(requestBody) });
  return { status: r.status, data: await r.json().catch(() => ({})) };
}
const wait = ms => new Promise(r => setTimeout(r, ms));
const RANKS = ['6','7','8','9','10','В','Д','К','Т'];
const canBeat = (attack, defense, trump) => attack.suit === defense.suit ? RANKS.indexOf(defense.rank) > RANKS.indexOf(attack.rank) : defense.suit === trump && attack.suit !== trump;

const { credentials: labels, cleanup } = await registerClubAccounts(['a', 'b']);
const tokens = new Map(labels.map(item => [item.clientId, item.token]));
try {
const a = labels[0].clientId, b = labels[1].clientId;
await connect(a); await connect(b);

async function newRoom(mode = 'classic') {
  const room = await post('/api/rooms', { clientId: a, name: `Тест ${Math.random().toString(36).slice(2, 6)}`, mode, deckSize: 36, playerName: 'Алиса' });
  const code = room.data.code;
  await post('/api/join', { clientId: b, roomId: code, playerName: 'Борис' });
  await wait(120);
  await post('/api/action', { clientId: a, action: 'start' });
  const ready = async () => [a, b].every(id => players[id].state && players[id].state.room.id === code && players[id].state.room.status === 'playing');
  const started = Date.now();
  while (!(await ready()) && Date.now() - started < 3000) await wait(30);
  const state = players[a].state;
  const attackerId = state.attackerId;
  const defenderId = state.defenderId;
  return { code: room.data.code, state, attackerId, defenderId };
}
async function closeRoom(code) {
  await post('/api/leave', { clientId: a });
  await post('/api/leave', { clientId: b });
  await wait(90);
}

/* 1. нельзя покрыть неправильной картой, включая козырь некозырной */
let trumpCases = 0;
for (let attempt = 0; attempt < 24; attempt += 1) {
  const { code, state, attackerId, defenderId } = await newRoom();
  const attack = players[attackerId].state.you.hand[attempt % 6];
  const defense = players[defenderId].state.you.hand[(attempt * 3 + 1) % 6];
  const expected = canBeat(attack, defense, state.trump);
  if (attack.suit === state.trump && defense.suit !== state.trump) trumpCases += 1;
  const attackRes = await post('/api/action', { clientId: attackerId, action: 'attack', cardId: attack.id });
  const defendRes = await post('/api/action', { clientId: defenderId, action: 'defend', cardId: defense.id });
  const accepted = defendRes.status === 200;
  check(`отбой ${attack.rank}${attack.suit} картой ${defense.rank}${defense.suit}${expected ? '' : ' (нельзя)'}`, accepted === expected, `ожидали ${expected ? 'приём' : 'отказ'}, получили ${defendRes.status} ${JSON.stringify(defendRes.data)}`);
  check('первая атака принята', attackRes.status === 200);
  await closeRoom(code);
}
check('среди проверок были козырь-против-некозыря', trumpCases > 0, String(trumpCases));

/* 2. можно подкинуть несколько карт одного достоинства сразу */
let multi = false;
for (let attempt = 0; attempt < 30 && !multi; attempt += 1) {
  const { code, state, attackerId, defenderId } = await newRoom();
  const hand = players[attackerId].state.you.hand;
  const first = hand[0];
  const second = hand.find(card => card.id !== first.id && card.rank === first.rank);
  if (!second) { await closeRoom(code); continue; }
  await post('/api/action', { clientId: attackerId, action: 'attack', cardId: first.id });
  await wait(110);
  const secondRes = await post('/api/action', { clientId: attackerId, action: 'attack', cardId: second.id });
  await wait(150);
  const table = players[attackerId].state.table;
  check('вторая карта того же достоинства принимается без хода защитника', secondRes.status === 200, JSON.stringify(secondRes.data));
  check('на столе две атаки', table.length === 2, String(table.length));
  check('защитник может отбить обе', players[defenderId].state.canDefend === true);
  multi = secondRes.status === 200 && table.length === 2;
  await closeRoom(code);
}
check('сценарий с несколькими картами найден', multi);

/* 3. можно докинуть карту после того, как защитник решил взять */
let pendingOk = false;
for (let attempt = 0; attempt < 30 && !pendingOk; attempt += 1) {
  const { code, state, attackerId, defenderId } = await newRoom();
  const hand = players[attackerId].state.you.hand;
  const first = hand[0];
  const second = hand.find(card => card.id !== first.id && card.rank === first.rank);
  if (!second) { await closeRoom(code); continue; }
  await post('/api/action', { clientId: attackerId, action: 'attack', cardId: first.id });
  await wait(110);
  const takeRes = await post('/api/action', { clientId: defenderId, action: 'take' });
  check('защитник объявляет «беру»', takeRes.status === 200, JSON.stringify(takeRes.data));
  await wait(140);
  check('состояние помечает «беру»', players[attackerId].state.takePending === true);
  const handBefore = players[defenderId].state.you.hand.length;
  const extraRes = await post('/api/action', { clientId: attackerId, action: 'attack', cardId: second.id });
  check('подкидывание после «беру» разрешено', extraRes.status === 200, JSON.stringify(extraRes.data));
  const finishRes = await post('/api/action', { clientId: attackerId, action: 'pass' });
  check('«хватит» завершает взятие', finishRes.status === 200, JSON.stringify(finishRes.data));
  await wait(140);
  const after = players[defenderId].state;
  check('защитник забрал карты со стола', after.you.hand.length >= handBefore + 2, `${handBefore} → ${after.you.hand.length}`);
  check('стол очищен', after.table.length === 0, String(after.table.length));
  pendingOk = extraRes.status === 200 && after.table.length === 0;
  await closeRoom(code);
}
check('сценарий «беру» и подкидывание найден', pendingOk);

players[a].req.destroy(); players[b].req.destroy();
console.log(failures ? `\n${failures} проверок провалено` : '\nвсе проверки пройдены');
process.exitCode = failures ? 1 : 0;

} finally {
  for (const player of Object.values(players)) {
    player.source?.destroy();
    player.req?.destroy();
  }
  try { await cleanup(); } catch (error) {
    console.log(`FAIL очистка тестовых аккаунтов: ${error.message}`);
    process.exitCode = 1;
  }
}
