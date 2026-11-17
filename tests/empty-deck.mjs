import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import vm from 'node:vm';

const require = createRequire(import.meta.url);
const source = readFileSync(new URL('../server.js', import.meta.url), 'utf8');
const context = vm.createContext({
  require: name => name === 'http' ? { createServer: callback => ({ listen() {} }) } : require(name),
  process: { env: {} }, console, URL, Math, Date,
  __dirname: new URL('..', import.meta.url).pathname,
  setInterval: callback => ({ unref() {} }),
});
vm.runInContext(source + '\n globalThis.engine = { createRoom, startRound, refillHands };', context);
const { createRoom, startRound, refillHands } = context.engine;
const players = Array.from({ length: 6 }, (_, index) => ({
  id: `p${index}`,
  name: `Игрок ${index}`,
  hand: [],
  connected: true,
  finished: false,
  left: false,
}));
const room = createRoom({ name: 'Пустая колода', mode: 'classic', deckSize: 36, bet: 0, host: players[0] });
room.players = players;
startRound(room);
assert.equal(room.deck.length, 0);
assert.equal(room.trumpCard, null);
assert.ok(room.players.every(player => player.hand.length === 6));
room.deck = [{ id: '6♠', rank: '6', suit: '♠' }];
room.trumpCard = { id: 'Т♣', rank: 'Т', suit: '♣' };
refillHands(room);
assert.deepEqual(room.trumpCard, room.deck[0]);
room.deck = [];
refillHands(room);
assert.equal(room.trumpCard, null);
console.log('ok Пустая колода скрывает козырную карту после раздачи и добора');
