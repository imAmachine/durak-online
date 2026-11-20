import assert from 'node:assert/strict';

import { chooseMove } from '../mcp/strategy.mjs';

const card = id => ({ id, rank: id.slice(0, -1), suit: id.slice(-1) });

const state = {
  room: { id: 'R', status: 'playing', deckSize: 36, mode: 'classic', bet: 0 },
  you: { id: 'c', role: 'attacker', hand: [card('6♥')] },
  hand: [card('6♥')],
  players: [
    { id: 'c', name: 'C', cards: 1, role: 'attacker' },
    { id: 'a', name: 'A', cards: 6, role: 'idle' },
    { id: 'b', name: 'B', cards: 6, role: 'defender' },
  ],
  attackerId: 'a',
  defenderId: 'b',
  trump: '♣',
  deck: 12,
  table: [{ attack: card('6♠'), defense: card('7♠') }],
  canAttack: true,
  canPass: true,
  canDefend: false,
  canTake: false,
  canTransfer: false,
  takePending: false,
  finished: null,
  log: [],
};

assert.equal(chooseMove(state).action, 'attack');
assert.equal(chooseMove({ ...state, canAttack: false }).action, 'pass');
assert.equal(chooseMove({ ...state, canAttack: false, canPass: false }), null);
assert.equal(chooseMove({ ...state, takePending: true, canAttack: false, canPass: true }).action, 'pass');
assert.equal(chooseMove({ ...state, takePending: true, canPass: false, canAttack: false }), null);
assert.equal(chooseMove(null), null);

console.log('MCP follow: приоритет и pass другого подкидывающего проверены');
