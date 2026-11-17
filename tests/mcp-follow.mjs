import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';

const source = fs.readFileSync(new URL('../mcp/server.mjs', import.meta.url), 'utf8');
const start = source.indexOf('function myMove(');
const end = source.indexOf('\nasync function follow(', start);
assert.ok(start >= 0 && end > start);
const context = vm.createContext({
  MAX_TABLE_CARDS: 6,
  attachOptions: state => state.hand,
  pickAttackCard: cards => cards[0],
  cardText: card => card.id,
});
vm.runInContext(source.slice(start, end), context);
const state = {
  room: { status: 'playing' },
  you: { id: 'c' },
  attackerId: 'a',
  defenderId: 'b',
  table: [{ attack: { id: '6♠' }, defense: { id: '7♠' } }],
  hand: [{ id: '6♥' }],
  canAttack: true,
  canPass: true,
  takePending: false,
};
context.game = state;
assert.equal(vm.runInContext('myMove(null, game).action', context), 'attack');
state.canAttack = false;
assert.equal(vm.runInContext('myMove(null, game).action', context), 'pass');
state.takePending = true;
assert.equal(vm.runInContext('myMove(null, game).action', context), 'pass');
state.canPass = false;
assert.equal(vm.runInContext('myMove(null, game)', context), null);
console.log('MCP follow: приоритет и pass другого подкидывающего проверены');
