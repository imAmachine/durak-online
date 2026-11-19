import { registerClubAccounts, referenceIds } from './helpers/accounts.mjs';
const base = process.env.BASE_URL || 'http://localhost:4173';
function tokenFor(id) {
  return tokens.get(id) || '';
}
const players = {};
let failures = 0;
import http from 'node:http';
import crypto from 'node:crypto';

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
      if (payload.type === 'lobby') players[id].online = payload.online;
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

function waitForSeq(ids, seq) {
  return waitFor(() => ids.every(id => Number.isInteger(stateOf(id)?.seq) && stateOf(id).seq >= seq));
}

const labels = await registerClubAccounts(['a', 'b', 'c']);
const tokens = new Map(labels.map(item => [item.clientId, item.token]));
const a = labels[0].clientId, b = labels[1].clientId, c = labels[2].clientId;

await connect(a, 'Алиса');
await connect(b, 'Борис');
await connect(c, 'Вера');

const created = await post('/api/rooms', { clientId: a, name: 'Тест переводной', mode: 'transfer', deckSize: 36, playerName: 'Алиса' });
check('лобби создано', created.status === 201, JSON.stringify(created.data));
const code = created.data.code;
check('код из 6 символов', typeof code === 'string' && code.length === 6, String(code));

const joined = await post('/api/join', { clientId: b, roomId: code.toLowerCase(), playerName: 'Борис' });
check('вход по коду в нижнем регистре', joined.status === 200, JSON.stringify(joined.data));

const third = await post('/api/join', { clientId: c, roomId: code, playerName: 'Вера' });
check('третий игрок вошёл', third.status === 200);
await waitFor(() => stateOf(a) && stateOf(a).players.length === 3);
check('в лобби три игрока', stateOf(a).players.length === 3, String(stateOf(a).players.length));

const notHost = await post('/api/action', { clientId: b, action: 'start' });
check('не хозяин не может начать', notHost.status === 400, JSON.stringify(notHost.data));

await post('/api/join', { clientId: a, roomId: code });
const started = await post('/api/action', { clientId: a, action: 'start' });
check('старт игры', started.status === 200, JSON.stringify(started.data));

const game = await waitFor(() => (stateOf(a)?.room.status === 'playing' ? stateOf(a) : null));
await waitForSeq([a, b, c], game.seq);
check('каждому по 6 карт', [a, b, c].every(id => stateOf(id).hand.length === 6), [a, b, c].map(id => stateOf(id).hand.length).join(','));
check('в колоде 36 - 18 = 18 карт', game.deck === 18, String(game.deck));

const ids = [a, b, c];
const attackerId = game.attackerId;
const attacker = ids.find(id => id === attackerId);
const defenderId = game.defenderId;
const defender = ids.find(id => id === defenderId);

function handOf(id) { return stateOf(id).hand; }
function findCard(id, predicate) { return handOf(id).find(predicate); }

const wrongTurn = await post('/api/action', { clientId: defender, action: 'attack', cardId: handOf(defender)[0].id });
check('защищающийся не может атаковать', wrongTurn.status === 400, JSON.stringify(wrongTurn.data));

const bystander = ids.find(id => id !== attacker && id !== defender);
const premature = await post('/api/action', { clientId: bystander, action: 'attack', cardId: handOf(bystander)[0].id });
check('первый ход принадлежит только атакующему', premature.status === 400, JSON.stringify(premature.data));

const shareRank = handOf(attacker).find(card => handOf(defender).some(item => item.rank === card.rank));
const firstCard = shareRank || handOf(attacker)[0];
const attackSeq = stateOf(a).seq;
const attacked = await post('/api/action', { clientId: attacker, action: 'attack', cardId: firstCard.id });
check('атака принята', attacked.status === 200, JSON.stringify(attacked.data));
if (attacked.status === 200) await waitForSeq(ids, attackSeq + 1);

const tableOnlyDefendsTheirRank = await post('/api/action', {
  clientId: attacker,
  action: 'attack',
  cardId: (findCard(attacker, card => card.id !== firstCard.id && card.rank !== firstCard.rank) || {}).id,
});
check('подкидывать чужим достоинством нельзя', tableOnlyDefendsTheirRank.status === 400, JSON.stringify(tableOnlyDefendsTheirRank.data));

const thirdCard = findCard(bystander, card => card.rank === firstCard.rank);
if (thirdCard) {
  const seq = stateOf(a).seq;
  const thrown = await post('/api/action', { clientId: bystander, action: 'attack', cardId: thirdCard.id });
  check('третий активный игрок может подкинуть после первого хода', thrown.status === 200, JSON.stringify(thrown.data));
  if (thrown.status === 200) await waitForSeq(ids, seq + 1);
} else {
  console.log('skip подкидывание третьим: нет карты нужного достоинства');
}

const sameRank = findCard(attacker, card => card.rank === firstCard.rank);
if (sameRank) {
  const seq = stateOf(a).seq;
  const double = await post('/api/action', { clientId: attacker, action: 'attack', cardId: sameRank.id });
  check('подкидывание тем же достоинством', double.status === 200, JSON.stringify(double.data));
  if (double.status === 200) await waitForSeq(ids, seq + 1);
}

const transferCard = findCard(defender, card => card.rank === firstCard.rank);
if (transferCard) {
  const missingCard = await post('/api/action', { clientId: defender, action: 'transfer' });
  check('перевод без cardId отклонён', missingCard.status === 400, JSON.stringify(missingCard.data));
  const before = { attacker: stateOf(a).attackerId, defender: stateOf(a).defenderId };
  const seq = stateOf(a).seq;
  const transferred = await post('/api/action', { clientId: defender, action: 'transfer', cardId: transferCard.id });
  check('перевод хода картой того же достоинства', transferred.status === 200, JSON.stringify(transferred.data));
  if (transferred.status === 200) await waitForSeq(ids, seq + 1);
  await waitFor(() => stateOf(a).defenderId !== before.defender);
  check('перевод меняет защищающегося', stateOf(a).defenderId !== before.defender, `${before.defender} → ${stateOf(a).defenderId}`);
  check('атакующим стал переводивший', stateOf(a).attackerId === defender, `${stateOf(a).attackerId} vs ${defender}`);
} else {
  console.log('skip перевод: у защищающегося нет карты нужного достоинства');
}

const currentDefender = stateOf(a).defenderId;
const currentAttacker = stateOf(a).attackerId;
const takeSeq = stateOf(a).seq;
const takeResult = await post('/api/action', { clientId: currentDefender, action: 'take' });
check('защищающийся объявляет «беру»', takeResult.status === 200, JSON.stringify(takeResult.data));
if (takeResult.status === 200) await waitForSeq(ids, takeSeq + 1);
await waitFor(() => stateOf(a).takePending === true);
check('состояние помечает окно подкидывания', stateOf(a).takePending === true);
const handBeforeTake = stateOf(currentDefender).hand.length;
const finishSeq = stateOf(a).seq;
const finishTake = await post('/api/action', { clientId: currentAttacker, action: 'pass' });
if (finishTake.status === 200) {
  const others = [a, b, c].filter(id => id !== currentAttacker && !stateOf(id).finished);
  for (const id of others) {
    if (stateOf(id).takePending) await post('/api/action', { clientId: id, action: 'pass' });
  }
}
check('атакующий завершает взятие', finishTake.status === 200, JSON.stringify(finishTake.data));
if (finishTake.status === 200) await waitForSeq(ids, finishSeq + 1);
await waitFor(() => stateOf(a).takePending === false && [a, b, c].every(id => stateOf(id).table.length === 0), 1500);
const afterTake = stateOf(a);
check('стол очищен после взятия', afterTake.table.length === 0, String(afterTake.table.length));
check('защитник забрал карты', stateOf(currentDefender).hand.length !== handBeforeTake || afterTake.deck === 0);
check('роли переходят дальше', afterTake.attackerId && afterTake.defenderId && afterTake.attackerId !== afterTake.defenderId);

check('взятые карты остаются в руке', [a, b, c].some(id => stateOf(id).hand.length >= 6), [a, b, c].map(id => stateOf(id).hand.length).join(','));

const leave = await post('/api/leave', { clientId: c });
check('выход из лобби', leave.status === 200);
await waitFor(() => stateOf(a).room.status === 'finished');
check('выход активного игрока завершает партию', stateOf(a).room.status === 'finished');
check('ушедшему засчитано поражение', stateOf(a).finished?.loserId === c, JSON.stringify(stateOf(a).finished));

players[a].source.destroy();
players[b].source.destroy();
players[c].source.destroy();

console.log(failures ? `\n${failures} проверок провалено` : '\nвсе проверки пройдены');
process.exit(failures ? 1 : 0);