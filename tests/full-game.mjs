import http from 'node:http';
import crypto from 'node:crypto';

const base = process.env.BASE_URL || 'http://localhost:4173';
const tokens = new Map();
function tokenFor(id) {
  if (!tokens.has(id)) tokens.set(id, crypto.randomBytes(32).toString('hex'));
  return tokens.get(id);
}
const players = {};
let failures = 0;
let steps = 0;

function check(label, condition, extra = '') {
  if (condition) console.log(`ok   ${label}`);
  else { failures += 1; console.log(`FAIL ${label} ${extra}`); }
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
        if (line) { try { onMessage(JSON.parse(line.slice(6))); } catch (error) {} }
        index = buffer.indexOf('\n\n');
      }
    });
  });
  request.on('error', onError);
  return request;
}

function connect(id, name) {
  return new Promise(resolve => {
    players[id] = { state: null, source: null };
    players[id].source = sse(`${base}/events?id=${encodeURIComponent(id)}&token=${tokenFor(id)}&name=${encodeURIComponent(name)}`, payload => {
      if (payload.type === 'game') { players[id].state = payload.state; }
    }, () => {});
    setTimeout(resolve, 350);
  });
}

async function post(path, body) {
  const requestBody = body.clientId ? { ...body, token: body.token || tokenFor(body.clientId) } : body;
  const response = await fetch(base + path, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(requestBody) });
  return { status: response.status, data: await response.json().catch(() => ({})) };
}

function waitFor(predicate, timeout = 2500) {
  const started = Date.now();
  return new Promise((resolve, reject) => {
    const tick = () => {
      let value = null;
      try { value = predicate(); } catch (error) {}
      if (value) return resolve(value);
      if (Date.now() - started > timeout) return reject(new Error('timeout'));
      setTimeout(tick, 20);
    };
    tick();
  });
}

function waitForSeq(ids, seq) {
  return waitFor(() => ids.every(id => Number.isInteger(players[id].state?.seq) && players[id].state.seq >= seq));
}

const a = `sim-a-${Date.now()}`;
const b = `sim-b-${Date.now()}`;
await connect(a, 'Алиса');
await connect(b, 'Борис');

const created = await post('/api/rooms', { clientId: a, name: 'Симуляция', mode: 'classic', deckSize: 36, playerName: 'Алиса' });
await post('/api/join', { clientId: b, roomId: created.data.code, playerName: 'Борис' });
await post('/api/action', { clientId: a, action: 'start' });
await waitFor(() => players[a].state && players[a].state.room.status === 'playing');
await waitForSeq([a, b], players[a].state.seq);

let discarded = 0;
const RANKS = ['6', '7', '8', '9', '10', 'В', 'Д', 'К', 'Т'];
const value = rank => RANKS.indexOf(rank);
function beats(attack, defense, trump) {
  if (attack.suit === defense.suit) return value(defense.rank) > value(attack.rank);
  return defense.suit === trump && attack.suit !== trump;
}
function totalCards(state) {
  const hands = state.players.reduce((sum, player) => sum + player.cards, 0);
  const table = state.table.reduce((sum, row) => sum + 1 + (row.defense ? 1 : 0), 0);
  return state.deck + hands + table;
}

while (players[a].state.room.status === 'playing' && steps < 400) {
  steps += 1;
  const state = players[a].state;
  check(`шаг ${steps}: целостность колоды`, totalCards(state) + discarded === 36, `${totalCards(state)} + ${discarded}`);
  if (totalCards(state) + discarded !== 36) break;

  const turnId = state.turnId;
  const me = turnId === a ? a : turnId === b ? b : null;
  if (!me) { check('есть игрок с ходом', false, String(turnId)); break; }
  const myState = players[me].state;
  if (myState.seq !== state.seq) throw new Error(`Несогласованные seq: ${state.seq}, ${myState.seq}`);
  const commit = turnId === myState.attackerId ? 'attacker' : 'defender';
  const role = myState.takePending && commit === 'defender' ? 'attacker' : commit;
  const seq = state.seq;

  let action = null;
  if (role === 'attacker') {
    const hand = myState.hand;
    if (!state.table.length) {
      const card = [...hand].sort((x, y) => value(x.rank) - value(y.rank))[0];
      action = { action: 'attack', cardId: card.id };
    } else if (myState.canPass) {
      const ranks = new Set(state.table.flatMap(row => [row.attack.rank, ...(row.defense ? [row.defense.rank] : [])]));
      const extra = hand.find(card => ranks.has(card.rank));
      const unbeaten = state.table.filter(row => !row.defense).length;
      const defenderCards = state.players.find(player => player.id === state.defenderId).cards;
      action = extra && myState.canAttack && state.table.length < 6 && unbeaten < defenderCards
        ? { action: 'attack', cardId: extra.id } : { action: 'pass' };
    } else {
      break;
    }
  } else {
    const row = state.table.find(item => !item.defense);
    const card = myState.hand.find(item => beats(row.attack, item, myState.trump));
    action = card ? { action: 'defend', cardId: card.id } : { action: state.takePending ? 'pass' : 'take' };
  }

  if (action.action === 'pass' && !state.takePending) discarded += state.table.reduce((sum, row) => sum + 1 + (row.defense ? 1 : 0), 0);
  const result = await post('/api/action', { clientId: me, ...action });
  if (result.status !== 200) { check(`шаг ${steps}: ${action.action} принят`, false, JSON.stringify(result.data)); break; }
  await waitForSeq([a, b], seq + 1);
}

const final = players[a].state;
check('игра завершилась', final.room.status === 'finished', final.room.status);
check('определён дурак', Boolean(final.finished && final.finished.loser), JSON.stringify(final.finished));
check('карты на руках у оставшихся', final.players.some(player => player.cards > 0));
check('всего карт учтено', totalCards(final) + discarded === 36, `${totalCards(final)} + ${discarded}`);
console.log(`шагов сыграно: ${steps}, дурак: ${final.finished ? final.finished.loser : '—'}`);

const finalSeq = final.seq;
const restart = await post('/api/action', { clientId: a, action: 'start' });
check('перезапуск партии', restart.status === 200, JSON.stringify(restart.data));
await waitForSeq([a, b], finalSeq + 1);
await waitFor(() => players[a].state.room.status === 'playing');
await waitFor(() => players[a].state.hand.length === 6);
check('после рестарта по шесть карт', players[a].state.hand.length === 6 && players[b].state.hand.length === 6);
check('колода пересоздана', players[a].state.deck === 24, String(players[a].state.deck));

players[a].source.destroy();
players[b].source.destroy();
console.log(failures ? `\n${failures} проверок провалено` : '\nсимуляция пройдена без ошибок');
process.exit(failures ? 1 : 0);