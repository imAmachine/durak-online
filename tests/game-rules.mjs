import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import vm from 'node:vm';

const require = createRequire(import.meta.url);
const source = readFileSync(new URL('../server.js', import.meta.url), 'utf8');
let handler;
let cleanup;
let now = 1700000000000;
let seed = 987654321;
const math = Object.create(Math);
math.random = () => ((seed = (seed * 1664525 + 1013904223) >>> 0) / 2 ** 32);
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
  const players = Array.from({ length: count }, (_, index) => ({ id: `p${index}`, name: `Игрок ${index}`, hand: [], connected: true, finished: false, left: false }));
  e.clients.clear();
  e.rooms.clear();
  e.accounts.clear();
  const room = e.createRoom({ name: 'Правила', mode: 'classic', deckSize: 36, bet: 0, host: players[0], ...options });
  room.players = players;
  for (const player of players) e.clients.set(player.id, { id: player.id, name: player.name, roomId: room.id, res: { write() {} } });
  return room;
}
function setup(hands, options = {}) {
  const room = fixture(hands.length, options);
  room.status = 'playing';
  room.trump = '♣';
  room.trumpCard = card('6♣');
  room.deck = [card('Т♣'), card('К♣')];
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
function allCards(room) {
  return [...room.deck, ...room.discard, ...room.players.flatMap(player => player.hand), ...room.table.flatMap(row => [row.attack, row.defense].filter(Boolean))];
}
function test(name, run) {
  run();
  console.log(`ok ${name}`);
}

test('Колоды содержат ровно заявленный набор без повторов', () => {
  for (const size of [36, 52]) {
    const deck = e.buildDeck(size);
    assert.equal(deck.length, size);
    assert.equal(new Set(deck.map(value => value.id)).size, size);
    assert.equal(new Set(deck.map(value => value.suit)).size, 4);
    assert.ok(deck.every(value => value.id === value.rank + value.suit));
  }
});

test('Старт раздаёт шесть карт и выбирает младшую подходящую карту', () => {
  const room = fixture(2, { deckSize: 36 });
  e.startRound(room);
  assert.ok(room.trump);
  assert.ok(room.players.every(player => player.hand.length === 6));
  const held = room.players.flatMap(player => player.hand.map(value => ({ player, value })));
  const candidates = held.filter(entry => entry.value.suit === room.trump);
  const pool = candidates.length ? candidates : held;
  const ranks = ['6', '7', '8', '9', '10', 'В', 'Д', 'К', 'Т'];
  const lowest = pool.reduce((a, b) => ranks.indexOf(a.value.rank) <= ranks.indexOf(b.value.rank) ? a : b);
  assert.equal(room.attackerId, lowest.player.id);
  assert.equal(allCards(room).length, 36);
  assert.equal(new Set(allCards(room).map(value => value.id)).size, 36);
});

test('Козырь бьёт некозырную карту, масть сама по себе не меняет старшинство', () => {
  assert.equal(e.canBeat(card('6♠'), card('7♠'), '♣'), true);
  assert.equal(e.canBeat(card('Т♠'), card('6♣'), '♣'), true);
  assert.equal(e.canBeat(card('6♣'), card('Т♠'), '♣'), false);
  assert.equal(e.canBeat(card('6♠'), card('Т♥'), '♣'), false);
  assert.equal(e.canBeat(card('6♣'), card('7♣'), '♣'), true);
});

test('Подкидывать могут все кроме защитника, только по достоинству стола и до шести карт', () => {
  const room = setup([['6♠', '6♥', '8♥'], ['7♠', '8♠', '9♠', '10♠', 'В♠', 'Д♠'], ['6♦']]);
  fails(act('p2', 'attack', { cardId: '6♦' }));
  ok(act('p0', 'attack', { cardId: '6♠' }));
  ok(act('p2', 'attack', { cardId: '6♦' }));
  fails(act('p2', 'attack', { cardId: '7♠' }));
  room.table = ['6♠', '6♦', '7♦', '8♦', '9♦', '10♦'].map((id, index) => ({ attack: card(id), defense: index < 5 ? card(['7♠', '8♠', '9♠', '10♠', 'В♠'][index]) : null }));
  assert.equal(room.table.length, 6);
  fails(act('p0', 'attack', { cardId: '6♥' }));
});

test('После объявления «беру» атакующий может подкинуть совпадающее достоинство, затем защитник забирает всё', () => {
  const room = setup([['6♠', '6♥'], ['7♠', '8♠'], ['9♠']]);
  ok(act('p0', 'attack', { cardId: '6♠' }));
  ok(act('p1', 'take'));
  assert.equal(room.takePending, 'p1');
  ok(act('p0', 'attack', { cardId: '6♥' }));
  ok(act('p0', 'pass'));
  assert.equal(room.table.length, 0);
  assert.ok(['6♥', '6♠', '7♠'].every(id => room.players[1].hand.some(value => value.id === id)));
  assert.equal(room.attackerId, 'p2');
  assert.equal(new Set(allCards(room).map(value => value.id)).size, allCards(room).length);
});

test('Нельзя завершить ход, пока есть неотбитая карта; после полного отбоя карты уходят в сброс', () => {
  const room = setup([['6♠'], ['7♠']]);
  ok(act('p0', 'attack', { cardId: '6♠' }));
  fails(act('p0', 'pass'));
  ok(act('p1', 'defend', { cardId: '7♠', targetCardId: '6♠' }));
  ok(act('p0', 'pass'));
  assert.equal(room.table.length, 0);
  assert.equal(room.discard.length, 2);
  assert.equal(room.attackerId, 'p1');
  assert.equal(room.defenderId, 'p0');
});

test('Добор идёт по кругу от атакующего, защитник добирает последним, козырная карта исчезает после опустошения колоды', () => {
  const room = setup([['6♠', '8♠', '9♠', '10♠', 'В♠'], ['7♠', '8♥', '9♥', '10♥', 'В♥'], ['8♦', '9♦', '10♦', 'В♦', 'Д♦']]);
  room.deck = [card('Т♣'), card('К♣'), card('Д♣'), card('В♣'), card('10♣'), card('9♣')];
  room.trumpCard = room.deck[0];
  ok(act('p0', 'attack', { cardId: '6♠' }));
  ok(act('p1', 'defend', { cardId: '7♠' }));
  ok(act('p0', 'pass'));
  assert.equal(room.players[0].hand.length, 6);
  assert.equal(room.players[1].hand.length, 6);
  assert.equal(room.players[2].hand.length, 6);
  assert.ok(room.players[0].hand.some(value => value.id === '9♣'));
  assert.ok(room.players[0].hand.some(value => value.id === '10♣'));
  assert.ok(room.players[2].hand.some(value => value.id === 'В♣'));
  assert.ok(room.players[1].hand.some(value => value.id === 'Д♣'));
  assert.ok(room.players[1].hand.some(value => value.id === 'К♣'));
  assert.equal(room.deck.length, 1);
  room.deck = [];
  e.refillHands(room);
  assert.equal(room.trumpCard, null);
});

test('Перевод переносит роль защитника только при равенстве достоинств и вместимости следующей руки', () => {
  const room = setup([['6♠'], ['6♥', '7♥'], ['8♠', '9♠']], { mode: 'transfer' });
  ok(act('p0', 'attack', { cardId: '6♠' }));
  assert.equal(e.gameState(room, 'p1').canTransfer, true);
  ok(act('p1', 'transfer', { cardId: '6♥' }));
  assert.equal(room.attackerId, 'p1');
  assert.equal(room.defenderId, 'p2');
  assert.equal(room.trickAttackerId, 'p0');
  assert.equal(room.table.length, 2);
  assert.equal(room.players[1].hand.length, 1);
  const blocked = setup([['6♠'], ['6♥'], ['8♠']], { mode: 'transfer' });
  ok(act('p0', 'attack', { cardId: '6♠' }));
  assert.equal(e.gameState(blocked, 'p1').canTransfer, false);
  fails(act('p1', 'transfer', { cardId: '6♥' }));
});

test('Последняя карта не завершает игру до разрешения стола, а после взятия проигрывает оставшийся активный игрок', () => {
  const room = setup([['6♠'], ['7♠']]);
  room.deck = [];
  ok(act('p0', 'attack', { cardId: '6♠' }));
  assert.equal(room.status, 'playing');
  ok(act('p1', 'take'));
  ok(act('p0', 'pass'));
  assert.equal(room.status, 'finished');
  assert.equal(room.finished.loserId, 'p1');
  assert.equal(room.finished.draw, false);
  assert.equal(room.players[0].finished, true);
});

test('Уход активного игрока немедленно завершает партию его поражением и делит банк между остальными', () => {
  const room = setup([['6♠'], ['7♠'], ['8♠']], { bet: 100 });
  e.chargeStakes(room);
  e.leaveRoom(e.clients.get('p0'));
  assert.equal(room.status, 'finished');
  assert.equal(room.finished.loserId, 'p0');
  assert.equal(room.players.length, 3);
  assert.equal(room.pot, 0);
  assert.equal(e.accountOf('p0').balance, 900);
  assert.equal(e.accountOf('p1').balance, 1050);
  assert.equal(e.accountOf('p2').balance, 1050);
});

console.log('Пройдено: 10');
