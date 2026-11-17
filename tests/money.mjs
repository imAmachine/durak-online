const base = process.env.BASE_URL || 'http://localhost:4173';
const adminKey = process.env.ADMIN_KEY || 'test-only-secret';
const tokens = new Map();
function tokenFor(id) {
  if (!tokens.has(id)) tokens.set(id, crypto.randomBytes(32).toString('hex'));
  return tokens.get(id);
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

function accountOf(id) {
  return players[id] ? players[id].account : null;
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
    players[id] = { state: null, account: null, source: null };
    players[id].source = sse(`${base}/events?id=${encodeURIComponent(id)}&token=${tokenFor(id)}&name=${encodeURIComponent(name)}`, payload => {
      if (payload.type === 'game') players[id].state = payload.state;
      if (payload.type === 'account') players[id].account = payload.account;
    }, reject);
    setTimeout(resolve, 400);
  });
}

function waitFor(predicate, timeout = 2500) {
  const started = Date.now();
  return new Promise((resolve, reject) => {
    const tick = () => {
      let value = null;
      try { value = predicate(); } catch (error) {}
      if (value) return resolve(value);
      if (Date.now() - started > timeout) return reject(new Error('timeout'));
      setTimeout(tick, 25);
    };
    tick();
  });
}

const a = `money-a-${Date.now()}`;
const b = `money-b-${Date.now()}`;
await connect(a, 'Анна');
await connect(b, 'Богдан');
check('при подключении приходит баланс', accountOf(a) && accountOf(a).balance === 1000, JSON.stringify(accountOf(a)));

const created = await post('/api/rooms', { clientId: a, name: 'Ставки', mode: 'classic', deckSize: 36, bet: 100, playerName: 'Анна' });
check('лобби со ставкой создано', created.status === 201 && created.data.room.bet === 100, JSON.stringify(created.data));
const code = created.data.code;

const joined = await post('/api/join', { clientId: b, roomId: code, playerName: 'Богдан' });
check('второй игрок вошёл в платное лобби', joined.status === 200, JSON.stringify(joined.data));
check('ставка не списывается до старта', accountOf(b).balance === 1000, String(accountOf(b).balance));

const started = await post('/api/action', { clientId: a, action: 'start' });
check('платная партия стартовала', started.status === 200, JSON.stringify(started.data));
await waitFor(() => stateOf(a) && stateOf(a).room.status === 'playing');
await waitFor(() => accountOf(a).balance === 900 && accountOf(b).balance === 900);
check('ставка списана у обоих', accountOf(a).balance === 900 && accountOf(b).balance === 900, `${accountOf(a).balance}/${accountOf(b).balance}`);
check('банк собран', stateOf(a).room.bet === 100, JSON.stringify(stateOf(a).room));

const attacker = stateOf(a).attackerId === a ? a : b;
const defender = attacker === a ? b : a;
const attack = await post('/api/action', { clientId: attacker, action: 'attack', cardId: stateOf(attacker).hand[0].id });
check('первая карта на стол', attack.status === 200, JSON.stringify(attack.data));

const left = await post('/api/leave', { clientId: defender });
check('защищающийся покинул партию', left.status === 200);
await waitFor(() => stateOf(attacker) && stateOf(attacker).room.status === 'finished');
const finish = stateOf(attacker);
check('партия завершена досрочно', finish.room.status === 'finished', finish.room.status);
check('покинувший объявлен дураком', finish.finished && finish.finished.loserId === defender, JSON.stringify(finish.finished));
await waitFor(() => accountOf(attacker).balance === 1100 && accountOf(defender).balance === 900);
check('победитель забрал банк', accountOf(attacker).balance === 1100, String(accountOf(attacker).balance));
check('покинувший потерял ставку', accountOf(defender).balance === 900, String(accountOf(defender).balance));
check('выплата показана победителю', finish.payout === 100, String(finish.payout));
check('статистика победителя', accountOf(attacker).wins >= 1 && accountOf(attacker).games >= 1, JSON.stringify(accountOf(attacker)));
check('поражение записано', accountOf(defender).losses >= 1, JSON.stringify(accountOf(defender)));

await post('/api/leave', { clientId: attacker });

const denied = await post('/api/admin', { key: 'wrong', op: 'list' });
check('панель закрыта без ключа', denied.status === 403, JSON.stringify(denied.data));

const adminList = await post('/api/admin', { key: adminKey, op: 'list' });
check('админ видит игроков', adminList.status === 200 && adminList.data.players.some(player => player.id === a), JSON.stringify(adminList.data).slice(0, 120));

const before = adminList.data.players.find(player => player.id === a);
const updated = await post('/api/admin', { key: adminKey, op: 'update', id: a, name: 'Анна Клубная', balance: 5000, games: 7, wins: 5, losses: 2 });
check('админ правит профиль', updated.status === 200 && updated.data.player.balance === 5000 && updated.data.player.name === 'Анна Клубная', JSON.stringify(updated.data));
await waitFor(() => accountOf(a).name === 'Анна Клубная');
check('правка доходит до игрока', accountOf(a).balance === 5000, JSON.stringify(accountOf(a)));

const cheap = await post('/api/admin', { key: adminKey, op: 'update', id: b, balance: 40 });
check('баланс можно понизить', cheap.status === 200 && cheap.data.player.balance === 40, JSON.stringify(cheap.data));
const richRoom = await post('/api/rooms', { clientId: a, name: 'Дорогой стол', mode: 'classic', deckSize: 36, bet: 500, playerName: 'Анна Клубная' });
check('дорогой стол создан', richRoom.status === 201, JSON.stringify(richRoom.data));
const poor = await post('/api/join', { clientId: b, roomId: richRoom.data.code, playerName: 'Богдан' });
check('без баланса не пускают', poor.status === 400, JSON.stringify(poor.data));
await post('/api/leave', { clientId: a });

const bigBet = await post('/api/rooms', { clientId: a, name: 'Нет такой ставки', mode: 'classic', deckSize: 36, bet: 9999, playerName: 'Анна Клубная' });
check('недопустимая ставка обнуляется', bigBet.status === 201 && bigBet.data.room.bet === 0, JSON.stringify(bigBet.data));
await post('/api/leave', { clientId: a });

const reset = await post('/api/admin', { key: adminKey, op: 'reset', id: a });
check('сброс прогресса', reset.status === 200 && reset.data.player.balance === 1000 && reset.data.player.games === 0, JSON.stringify(reset.data));
await waitFor(() => accountOf(a).balance === 1000);
check('сброс виден игроку', accountOf(a).games === 0 && accountOf(a).balance === 1000, JSON.stringify(accountOf(a)));

console.log(`стартовый баланс был ${before.balance}`);
players[a].source.destroy();
players[b].source.destroy();
console.log(failures ? `\n${failures} проверок провалено` : '\nденьги и админка работают');
process.exitCode = failures ? 1 : 0;
