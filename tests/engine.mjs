import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import vm from 'node:vm';

const require = createRequire(import.meta.url);
const source = readFileSync(new URL('../server.js', import.meta.url), 'utf8');
let seed = 123456;
const math = Object.create(Math);
math.random = () => ((seed = (seed * 1664525 + 1013904223) >>> 0) / 2 ** 32);
let handler;
let cleanup;
let now = 1700000000000;
class Clock extends Date {
  static now() { return now; }
}
const context = vm.createContext({
  require: name => name === 'http' ? { createServer: callback => {
    handler = callback;
    return { listen() {} };
  } } : require(name),
  process: { env: {} }, console, URL, Math: math, Date: Clock,
  __dirname: new URL('..', import.meta.url).pathname,
  setInterval: callback => { cleanup = callback; return { unref() {} }; },
});
vm.runInContext(source + '\n globalThis.engine = { buildDeck, canBeat, createRoom, startRound, act, gameState, findPlayer, refillHands, settle, leaveRoom, finishGame, chargeStakes, accountOf, clients, rooms, accounts };', context);
const e = context.engine;
const card = id => ({ id, rank: id.slice(0, -1), suit: id.slice(-1) });
function fixture(count = 3, options = {}) {
  const players = Array.from({ length: count }, (_, index) => ({ id: `p${index}`, name: `Игрок ${index}`, hand: [], connected: true, finished: false }));
  e.clients.clear();
  e.rooms.clear();
  e.accounts.clear();
  const room = e.createRoom({ name: 'Тест', mode: 'classic', deckSize: 36, bet: 0, host: players[0], ...options });
  room.players = players;
  for (const player of players) {
    e.clients.set(player.id, { id: player.id, name: player.name, roomId: room.id, res: { write() {} } });
  }
  return room;
}
function setup(hands, options = {}) {
  const room = fixture(hands.length, options);
  room.status = 'playing';
  room.trump = '♣';
  room.deck = [card('Т♣')];
  room.attackerId = 'p0';
  room.defenderId = 'p1';
  room.turnId = 'p0';
  room.trickAttackerId = 'p0';
  hands.forEach((hand, index) => { room.players[index].hand = hand.map(card); });
  return room;
}
const act = (id, action, values = {}) => e.act(e.clients.get(id), { action, ...values });
const ok = result => assert.equal(result.error, undefined, result.error);
const fails = result => assert.equal(typeof result.error, 'string');
let tests = 0;
function test(name, run) {
  run();
  tests += 1;
  console.log(`ok ${name}`);
}

for (const deckSize of [36, 52]) {
  for (let count = 2; count <= 6; count += 1) {
    test(`Колода ${deckSize}, игроков ${count}: раздача и сохранность карт`, () => {
      const room = fixture(count, { deckSize });
      e.startRound(room);
      const cards = [...room.deck, ...room.players.flatMap(player => player.hand)];
      assert.equal(cards.length, deckSize);
      assert.equal(new Set(cards.map(value => value.id)).size, deckSize);
      assert.equal(room.deck.length, deckSize - count * 6);
      assert.ok(room.players.every(player => player.hand.length === 6));
      assert.notEqual(room.attackerId, room.defenderId);
    });
  }
}
test('Правила старшинства и козырей', () => {
  assert.equal(e.canBeat(card('6♠'), card('7♠'), '♣'), true);
  assert.equal(e.canBeat(card('Т♠'), card('6♣'), '♣'), true);
  assert.equal(e.canBeat(card('6♣'), card('Т♠'), '♣'), false);
  assert.equal(e.canBeat(card('6♠'), card('Т♥'), '♣'), false);
});


test('Начинает младший козырь в руках', () => {
  for (const deckSize of [36, 52]) {
    for (let count = 2; count <= 6; count += 1) {
      const room = fixture(count, { deckSize });
      e.startRound(room);
      const held = room.players.flatMap(player => player.hand.map(value => ({ player, value })));
      const trumps = held.filter(entry => entry.value.suit === room.trump);
      const candidates = trumps.length ? trumps : held;
      const ranks = ['2', '3', '4', '5', '6', '7', '8', '9', '10', 'В', 'Д', 'К', 'Т'];
      const lowest = candidates.reduce((a, b) => ranks.indexOf(a.value.rank) <= ranks.indexOf(b.value.rank) ? a : b);
      assert.equal(room.attackerId, lowest.player.id);
    }
  }
});
test('Начало без козырей использует младшую карту', () => {
  const room = fixture(2);
  vm.runInContext('globalThis.originalShuffle = shuffle; shuffle = list => { const trumps = list.filter(card => card.suit === "♠"); const others = list.filter(card => card.suit !== "♠"); return [...trumps, ...others]; };', context);
  e.startRound(room);
  const ranks = ['6', '7', '8', '9', '10', 'В', 'Д', 'К', 'Т'];
  const held = room.players.flatMap(player => player.hand.map(value => ({ player, value })));
  assert.ok(held.every(entry => entry.value.suit !== room.trump));
  const lowest = held.reduce((a, b) => ranks.indexOf(a.value.rank) <= ranks.indexOf(b.value.rank) ? a : b);
  assert.equal(room.attackerId, lowest.player.id);
  vm.runInContext('shuffle = originalShuffle;', context);
});
test('Подкидывание всеми, запрет открытия и завершения посторонним', () => {
  const room = setup([['6♠', '8♠'], ['7♠', '9♠', '10♠'], ['6♥', '8♥']]);
  fails(act('p2', 'attack', { cardId: '6♥' }));
  ok(act('p0', 'attack', { cardId: '6♠' }));
  assert.equal(e.gameState(room, 'p2').canAttack, true);
  ok(act('p2', 'attack', { cardId: '6♥' }));
  fails(act('p1', 'attack', { cardId: '7♠' }));
  fails(act('p2', 'pass'));
  fails(act('p2', 'attack', { cardId: '8♥' }));
  fails(act('p0', 'pass'));
  ok(act('p1', 'take'));
  fails(act('p1', 'take'));
  ok(act('p0', 'pass'));
  assert.equal(room.table.length, 0);
  assert.equal(room.players[1].hand.some(value => value.id === '6♥'), true);
});
test('Лимит зависит от неотбитых строк, а не от всех строк', () => {
  const room = setup([['6♠', '6♥', '7♥'], ['7♠', '8♣']]);
  ok(act('p0', 'attack', { cardId: '6♠' }));
  ok(act('p1', 'defend', { cardId: '7♠', targetCardId: '6♠' }));
  assert.equal(e.gameState(room, 'p0').canAttack, true);
  ok(act('p0', 'attack', { cardId: '7♥' }));
  assert.equal(e.gameState(room, 'p0').canAttack, false);
  fails(act('p0', 'attack', { cardId: '6♥' }));
  ok(act('p1', 'take'));
  fails(act('p0', 'attack', { cardId: '6♥' }));
});
test('Добор до ротации после бито и взятия', () => {
  for (const take of [false, true]) {
    const room = setup([['6♠'], ['7♠'], ['8♠']]);
    room.deck = [card('Т♣'), card('К♣')];
    ok(act('p0', 'attack', { cardId: '6♠' }));
    ok(act('p1', take ? 'take' : 'defend', { cardId: '7♠' }));
    ok(act('p0', 'pass'));
    assert.equal(room.players[0].hand.length, 2);
    assert.equal(room.players[0].hand[0].id, 'К♣');
    assert.equal(room.attackerId, 'p2');
    assert.equal(room.trumpCard, null);
    assert.equal(room.discard.length, take ? 0 : 2);
  }
});
test('Перевод требует выбранную карту и вместимость следующей руки', () => {
  const room = setup([['6♠', '8♠'], ['6♥', '6♦'], ['9♠']], { mode: 'transfer' });
  ok(act('p0', 'attack', { cardId: '6♠' }));
  fails(act('p1', 'transfer'));
  fails(act('p1', 'transfer', { cardId: '6♦' }));
  assert.equal(e.gameState(room, 'p1').canTransfer, false);
  room.players[2].hand.push(card('10♠'));
  assert.equal(e.gameState(room, 'p1').canTransfer, true);
  ok(act('p1', 'transfer', { cardId: '6♦' }));
  assert.equal(room.table[1].attack.id, '6♦');
  assert.equal(room.players[1].hand[0].id, '6♥');
  assert.equal(room.defenderId, 'p2');
  assert.equal(room.trickAttackerId, 'p0');
  ok(act('p2', 'take'));
  ok(act('p1', 'pass'));
  assert.equal(room.players[0].hand.some(value => value.id === 'Т♣'), true);
});
test('Перевод запрещён после защиты, решения взять и вне режима', () => {
  const room = setup([['6♠', '8♠'], ['6♥', '7♠', '9♠'], ['6♦', '10♠', 'В♠']], { mode: 'transfer' });
  ok(act('p0', 'attack', { cardId: '6♠' }));
  room.mode = 'classic';
  fails(act('p1', 'transfer', { cardId: '6♥' }));
  room.mode = 'transfer';
  ok(act('p1', 'defend', { cardId: '7♠' }));
  fails(act('p1', 'transfer', { cardId: '6♥' }));
  room.table[0].defense = null;
  room.takePending = 'p1';
  fails(act('p1', 'transfer', { cardId: '6♥' }));
  assert.equal(e.gameState(room, 'p1').canTransfer, false);
});
test('Последняя карта не заканчивает незавершённую защиту', () => {
  const room = setup([['6♠'], ['7♠']]);
  room.deck = [];
  ok(act('p0', 'attack', { cardId: '6♠' }));
  assert.equal(room.status, 'playing');
  ok(act('p1', 'defend', { cardId: '7♠' }));
  assert.equal(room.status, 'playing');
  ok(act('p0', 'pass'));
  assert.equal(room.status, 'finished');
  assert.equal(room.finished.loserId, null);
  assert.equal(room.discard.length, 2);
});
test('Последняя карта при взятии оставляет проигравшего и выплачивает банк один раз', () => {
  const room = setup([['6♠'], ['7♠']], { bet: 50 });
  room.deck = [];
  e.chargeStakes(room);
  ok(act('p0', 'attack', { cardId: '6♠' }));
  ok(act('p1', 'take'));
  ok(act('p0', 'pass'));
  assert.equal(room.finished.loserId, 'p1');
  assert.equal(e.accountOf('p0').balance, 1050);
  assert.equal(e.accountOf('p1').balance, 950);
  e.finishGame(room, room.players[1]);
  assert.equal(e.accountOf('p0').games, 1);
});
test('Уход активного завершает партию, сохраняет карты и выплачивает всем остальным', () => {
  const room = setup([['6♠'], ['7♠'], ['8♠']], { bet: 100 });
  e.chargeStakes(room);
  ok(act('p0', 'attack', { cardId: '6♠' }));
  e.leaveRoom(e.clients.get('p0'));
  assert.equal(room.status, 'finished');
  assert.equal(room.finished.loserId, 'p0');
  assert.equal(room.players.length, 3);
  assert.equal(room.table.length, 1);
  assert.equal(e.accountOf('p0').losses, 1);
  assert.equal(e.accountOf('p0').balance, 900);
  assert.equal(e.accountOf('p1').balance, 1050);
  assert.equal(e.accountOf('p2').balance, 1050);
  assert.equal(room.pot, 0);
});
test('Ушедший финишировавший остаётся участником и получает выплату', () => {
  const room = setup([[], ['7♠'], ['8♠']], { bet: 100 });
  room.deck = [];
  room.players[0].finished = true;
  room.attackerId = 'p1';
  room.defenderId = 'p2';
  e.chargeStakes(room);
  e.leaveRoom(e.clients.get('p0'));
  assert.equal(room.status, 'playing');
  assert.equal(room.players.length, 3);
  assert.equal(room.attackerId, 'p1');
  assert.equal(e.accountOf('p0').losses, 0);
  e.finishGame(room, room.players[2]);
  assert.equal(e.accountOf('p0').balance, 1050);
  assert.equal(e.accountOf('p0').wins, 1);
});
test('Cleanup сохраняет игру до grace последнего отключения и возвращает все ставки', () => {
  const room = setup([['6♠'], ['7♠'], ['8♠']], { bet: 100 });
  e.chargeStakes(room);
  room.createdAt = now - 10 ** 8;
  e.clients.clear();
  room.players.forEach(player => { player.connected = false; player.offlineAt = now - 299999; });
  room.players[2].offlineAt = now;
  cleanup();
  assert.equal(room.status, 'playing');
  now += 299999;
  cleanup();
  assert.equal(room.status, 'playing');
  now += 1;
  cleanup();
  assert.equal(room.status, 'finished');
  assert.equal(room.finished.cancelled, true);
  assert.ok(e.rooms.has(room.id));
  for (const player of room.players) {
    assert.equal(e.accountOf(player.id).balance, 1000);
    assert.equal(e.accountOf(player.id).games, 0);
  }
  now += 30 * 60 * 1000 - 1;
  cleanup();
  assert.ok(e.rooms.has(room.id));
  now += 1;
  cleanup();
  assert.equal(e.rooms.has(room.id), false);
});
async function request(method, path, body) {
  const listeners = {};
  const response = { status: null, text: '', writeHead(status) { this.status = status; }, write(value) { this.text += value; }, end(value = '') { this.text += value; } };
  const req = { method, url: path, headers: { host: 'localhost' }, on(event, listener) {
    listeners[event] = listener;
    if (event === 'end') queueMicrotask(() => { listeners.data?.(JSON.stringify(body)); listener(); });
  } };
  await handler(req, response);
  return { ...response, close: () => listeners.close?.() };
}
async function asyncTest(name, run) {
  await run();
  tests += 1;
  console.log(`ok ${name}`);
}
await asyncTest('SSE и все POST защищены токеном, верный token позволяет reconnect', async () => {
  e.clients.clear();
  e.rooms.clear();
  const token = 'a'.repeat(64);
  assert.equal((await request('GET', '/events?id=identity')).status, 400);
  assert.equal((await request('GET', `/events?id=identity&token=${'A'.repeat(64)}`)).status, 400);
  const stream = await request('GET', `/events?id=identity&token=${token}`);
  assert.equal(stream.status, 200);
  assert.equal((await request('GET', `/events?id=identity&token=${'b'.repeat(64)}`)).status, 403);
  const previousClient = e.clients.get('identity');
  const replacement = await request('GET', `/events?id=identity&token=${token}`);
  assert.equal(replacement.status, 200);
  const replacementClient = e.clients.get('identity');
  assert.notEqual(replacementClient, previousClient);
  stream.close();
  assert.equal(e.clients.get('identity'), replacementClient);
  for (const path of ['/api/rooms', '/api/join', '/api/action', '/api/profile', '/api/leave', '/api/admin']) {
    assert.equal((await request('POST', path, { clientId: 'identity', token: 'b'.repeat(64) })).status, 403);
    assert.equal((await request('POST', path, { clientId: 'identity' })).status, 403);
  }
  assert.equal((await request('POST', '/api/profile', { clientId: 'identity', token, name: 'Имя' })).status, 200);
  replacement.close();
  assert.equal((await request('GET', `/events?id=identity&token=${'b'.repeat(64)}`)).status, 403);
  assert.equal((await request('GET', `/events?id=identity&token=${token}`)).status, 200);
});
await asyncTest('Неуспешный join не покидает текущую комнату, reconnect уведомляет всех', async () => {
  e.clients.clear();
  e.rooms.clear();
  const token = 'c'.repeat(64);
  const owner = await request('GET', `/events?id=owner&token=${token}`);
  await request('GET', `/events?id=guest&token=${token}`);
  const created = await request('POST', '/api/rooms', { clientId: 'owner', token });
  const firstId = JSON.parse(created.text).code;
  const second = await request('POST', '/api/rooms', { clientId: 'guest', token });
  const secondId = JSON.parse(second.text).code;
  e.rooms.get(secondId).status = 'playing';
  assert.equal((await request('POST', '/api/join', { clientId: 'owner', token, roomId: secondId })).status, 400);
  assert.equal(e.clients.get('owner').roomId, firstId);
  assert.equal(e.rooms.get(firstId).players.length, 1);
  e.rooms.get(secondId).status = 'waiting';
  e.rooms.get(secondId).bet = 500;
  e.accountOf('owner').balance = 0;
  assert.equal((await request('POST', '/api/join', { clientId: 'owner', token, roomId: secondId })).status, 400);
  assert.equal(e.clients.get('owner').roomId, firstId);
  e.accountOf('owner').balance = 1000;
  assert.equal((await request('POST', '/api/join', { clientId: 'owner', token, roomId: secondId })).status, 200);
  owner.close();
  let notices = 0;
  const guest = e.clients.get('guest');
  guest.res.write = value => { if (value.includes('"type":"game"')) notices += 1; };
  await request('GET', `/events?id=owner&token=${token}`);
  assert.ok(notices > 0);
  assert.equal(e.findPlayer(e.rooms.get(secondId), 'owner').connected, true);
});


await asyncTest('Исходники, служебные файлы и traversal не доступны через static', async () => {
  for (const path of ['/server.js', '/AGENTS.md', '/.git/config', '/tests/engine.mjs', '/%2e%2e/server.js']) {
    assert.equal((await request('GET', path)).status, 404);
  }
});
test('Флаги finished и left входят в персональный snapshot', () => {
  const room = setup([['6♠'], [], ['7♠']]);
  room.players[1].finished = true;
  assert.equal(e.gameState(room, 'p1').you.finished, true);
  room.players[0].left = true;
  room.status = 'finished';
  assert.equal(e.gameState(room, 'p0').canStart, false);
});


for (const mode of ['classic', 'transfer']) {
  for (const deckSize of [36, 52]) {
    for (let count = 2; count <= 6; count += 1) {
      test(`Полная партия ${mode}, ${deckSize}, ${count} игроков`, () => {
        const room = fixture(count, { mode, deckSize, bet: 50 });
        ok(act('p0', 'start'));
        for (let step = 0; step < 20000 && room.status === 'playing'; step += 1) {
          const attacker = e.findPlayer(room, room.attackerId);
          const defender = e.findPlayer(room, room.defenderId);
          if (!room.table.length) {
            ok(act(attacker.id, 'attack', { cardId: attacker.hand[0].id }));
          } else if (room.takePending || room.table.every(row => row.defense)) {
            ok(act(attacker.id, 'pass'));
          } else {
            const permissions = e.gameState(room, defender.id);
            if (permissions.canTransfer && math.random() < 0.5) {
              const selected = defender.hand.find(value => value.rank === room.table[0].attack.rank);
              ok(act(defender.id, 'transfer', { cardId: selected.id }));
            } else {
              const row = room.table.find(value => !value.defense);
              const selected = defender.hand.find(value => e.canBeat(row.attack, value, room.trump));
              ok(act(defender.id, selected ? 'defend' : 'take', selected ? { cardId: selected.id, targetCardId: row.attack.id } : {}));
            }
          }
          const cards = [...room.deck, ...room.discard, ...room.players.flatMap(player => player.hand), ...room.table.flatMap(row => [row.attack, row.defense].filter(Boolean))];
          assert.equal(cards.length, deckSize);
          assert.equal(new Set(cards.map(value => value.id)).size, deckSize);
          if (room.status === 'playing') {
            assert.notEqual(room.attackerId, room.defenderId);
            assert.equal(e.findPlayer(room, room.attackerId).finished, false);
            assert.equal(e.findPlayer(room, room.defenderId).finished, false);
          }
        }
        assert.equal(room.status, 'finished');
        assert.equal(room.pot, 0);
        assert.equal(room.players.reduce((sum, player) => sum + e.accountOf(player.id).balance, 0), count * 1000);
        assert.ok(room.players.every(player => e.accountOf(player.id).games === 1));
      });
    }
  }
}


test('Ничья возвращает ставки, учитывает games без wins и losses', () => {
  const room = setup([['6♠'], ['7♠']], { bet: 100 });
  room.deck = [];
  e.chargeStakes(room);
  ok(act('p0', 'attack', { cardId: '6♠' }));
  ok(act('p1', 'defend', { cardId: '7♠' }));
  ok(act('p0', 'pass'));
  assert.equal(room.finished.draw, true);
  assert.equal(room.pot, 0);
  for (const player of room.players) {
    const account = e.accountOf(player.id);
    assert.equal(account.balance, 1000);
    assert.equal(account.games, 1);
    assert.equal(account.wins, 0);
    assert.equal(account.losses, 0);
    assert.equal(room.lastPayouts[player.id], 0);
  }
});


await asyncTest('Admin отключён без ADMIN_KEY, публичный default не работает', async () => {
  for (const key of ['', 'durak-admin', 'test-only-secret']) {
    assert.equal((await request('POST', '/api/admin', { key, op: 'list' })).status, 503);
  }
});


test('Ушедший финишировавший без offlineAt не блокирует отмену и получает refund', () => {
  const room = setup([[], ['7♠'], ['8♠']], { bet: 100 });
  e.chargeStakes(room);
  room.players[0].finished = true;
  room.players[0].left = true;
  room.players[0].connected = false;
  room.players[0].offlineAt = null;
  e.clients.clear();
  for (const player of room.players.slice(1)) {
    player.connected = false;
    player.offlineAt = now;
  }
  now += 5 * 60 * 1000 - 1;
  cleanup();
  assert.equal(room.status, 'playing');
  now += 1;
  cleanup();
  assert.equal(room.finished.cancelled, true);
  assert.equal(room.pot, 0);
  for (const player of room.players) {
    assert.equal(e.accountOf(player.id).balance, 1000);
    assert.equal(e.accountOf(player.id).games, 0);
  }
});
console.log(`Пройдено: ${tests}`);
