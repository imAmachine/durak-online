import assert from 'node:assert/strict';

import {
  VALUES,
  Ledger,
  beats,
  attachOptions,
  defenseOptions,
  transferOptions,
  dumpOptions,
  attackChoices,
  chooseMove,
} from '../mcp/strategy.mjs';

let tests = 0;
const card = id => ({ id, rank: id.slice(0, -1), suit: id.slice(-1) });
const hand = list => list.map(card);
const check = (label, fn) => {
  fn();
  tests += 1;
  console.log(`ok   ${label}`);
};

function state(overrides = {}) {
  const base = {
    room: { id: 'ROOM01', deckSize: 36, status: 'playing', mode: 'classic', bet: 0 },
    you: { id: 'me', role: 'attacker', finished: false, left: false, hand: [] },
    players: [
      { id: 'me', name: 'Я', cards: 0, role: 'attacker' },
      { id: 'op', name: 'Оп', cards: 6, role: 'defender' },
    ],
    trump: '♠',
    trumpCard: card('6♠'),
    deck: 24,
    table: [],
    attackerId: 'me',
    defenderId: 'op',
    turnId: 'me',
    hand: [],
    canAttack: false,
    canDefend: false,
    canTransfer: false,
    canPass: false,
    canTake: false,
    takePending: false,
    finished: null,
    log: [],
  };
  const merged = { ...base, ...overrides };
  merged.you = { ...base.you, ...(overrides.you || {}) };
  merged.you.hand = (overrides.you && overrides.you.hand) || overrides.hand || [];
  merged.hand = merged.you.hand;
  merged.players = overrides.players || base.players;
  return merged;
}

check('VALUES: полный порядок 52 ранга', () => {
  assert.equal(VALUES.size, 13);
  assert.ok(VALUES.get('Т') > VALUES.get('К'));
  assert.ok(VALUES.get('2') < VALUES.get('6'));
});

check('beats: масть и козырь', () => {
  assert.equal(beats(card('6♥'), card('7♥'), '♠'), true);
  assert.equal(beats(card('7♥'), card('6♥'), '♠'), false);
  assert.equal(beats(card('Т♥'), card('6♠'), '♠'), true);
  assert.equal(beats(card('6♠'), card('Т♥'), '♠'), false);
  assert.equal(beats(card('6♠'), card('Т♠'), '♠'), true);
});

check('attachOptions: только ранги стола', () => {
  const s = state({
    hand: hand(['6♥', '7♦', '8♣']),
    table: [{ attack: card('6♦'), defense: null }],
    canAttack: true,
  });
  assert.deepEqual(attachOptions(s).map(item => item.id), ['6♥']);
});

check('attackChoices: козырь не подкидывается в середине партии', () => {
  const s = state({
    hand: hand(['6♥', '6♠']),
    table: [{ attack: card('6♦'), defense: null }],
    canAttack: true,
    players: [
      { id: 'me', name: 'Я', cards: 2 },
      { id: 'op', name: 'Оп', cards: 6 },
    ],
  });
  assert.deepEqual(attackChoices(s).map(item => item.id), ['6♥']);
});

check('attackChoices: козырь допустим, когда колода пуста', () => {
  const s = state({
    hand: hand(['6♠']),
    table: [{ attack: card('6♦'), defense: null }],
    canAttack: true,
    deck: 0,
  });
  assert.deepEqual(attackChoices(s).map(item => item.id), ['6♠']);
});

check('defenseOptions: сначала некозырные, младшие', () => {
  const s = state({
    hand: hand(['7♥', '8♥', '7♠']),
    table: [{ attack: card('6♥'), defense: null }],
    canDefend: true,
  });
  assert.deepEqual(defenseOptions(s).map(item => item.card.id), ['7♥', '8♥', '7♠']);
});

check('chooseMove: бьёт младшей некозырной', () => {
  const s = state({
    hand: hand(['7♥', '8♥', '7♠']),
    table: [{ attack: card('6♥'), defense: null }],
    canDefend: true,
    canTake: true,
    you: { id: 'me', role: 'defender', hand: hand(['7♥', '8♥', '7♠']) },
    attackerId: 'op',
    defenderId: 'me',
  });
  const move = chooseMove(s);
  assert.equal(move.action, 'defend');
  assert.equal(move.extra.cardId, '7♥');
  assert.equal(move.extra.targetCardId, '6♥');
});

check('chooseMove: бережёт последний козырь при полной колоде', () => {
  const s = state({
    hand: hand(['6♠']),
    table: [{ attack: card('Т♥'), defense: null }],
    canDefend: true,
    canTake: true,
    you: { id: 'me', role: 'defender', hand: hand(['6♠']) },
    attackerId: 'op',
    defenderId: 'me',
  });
  const move = chooseMove(s);
  assert.equal(move.action, 'take');
});

check('chooseMove: тратит козырь, когда колода пуста', () => {
  const s = state({
    hand: hand(['6♠']),
    table: [{ attack: card('Т♥'), defense: null }],
    canDefend: true,
    canTake: true,
    deck: 0,
    you: { id: 'me', role: 'defender', hand: hand(['6♠']) },
    attackerId: 'op',
    defenderId: 'me',
  });
  assert.equal(chooseMove(s).action, 'defend');
});

check('chooseMove: перевод выгоднее, когда отбиться нечем', () => {
  const s = state({
    room: { id: 'ROOM01', deckSize: 36, status: 'playing', mode: 'transfer', bet: 0 },
    hand: hand(['9♦', 'Т♥']),
    table: [{ attack: card('9♥'), defense: null }],
    canDefend: false,
    canTransfer: true,
    canTake: true,
    players: [
      { id: 'op', name: 'Оп', cards: 6, role: 'attacker' },
      { id: 'me', name: 'Я', cards: 2, role: 'defender' },
      { id: 'third', name: 'Тр', cards: 6, role: 'idle' },
    ],
    you: { id: 'me', role: 'defender', hand: hand(['9♦', 'Т♥']) },
    attackerId: 'op',
    defenderId: 'me',
  });
  assert.deepEqual(transferOptions(s).map(item => item.id), ['9♦']);
  const move = chooseMove(s);
  assert.equal(move.action, 'transfer');
  assert.equal(move.extra.cardId, '9♦');
});

check('chooseMove: защита дешевле перевода', () => {
  const s = state({
    room: { id: 'ROOM01', deckSize: 36, status: 'playing', mode: 'transfer', bet: 0 },
    hand: hand(['9♦', 'Т♥']),
    table: [{ attack: card('9♥'), defense: null }],
    canDefend: true,
    canTransfer: true,
    you: { id: 'me', role: 'defender', hand: hand(['9♦', 'Т♥']) },
    attackerId: 'op',
    defenderId: 'me',
  });
  assert.equal(chooseMove(s).action, 'defend');
});

check('chooseMove: под «беру» догружает некозырную карту', () => {
  const s = state({
    hand: hand(['9♦', '9♠']),
    table: [{ attack: card('9♥'), defense: null }],
    canAttack: true,
    canPass: true,
    takePending: true,
    you: { id: 'me', role: 'attacker', hand: hand(['9♦', '9♠']) },
  });
  assert.deepEqual(dumpOptions(s).map(item => item.id), ['9♦']);
  const move = chooseMove(s);
  assert.equal(move.action, 'attack');
  assert.equal(move.extra.cardId, '9♦');
});

check('chooseMove: защитник под «беру» не действует', () => {
  const s = state({
    hand: hand(['9♦']),
    table: [{ attack: card('9♥'), defense: null }],
    takePending: true,
    you: { id: 'me', role: 'defender', hand: hand(['9♦']) },
    attackerId: 'op',
    defenderId: 'me',
  });
  assert.equal(chooseMove(s), null);
});

check('chooseMove: вне партии не ходит', () => {
  assert.equal(chooseMove(state({ room: { id: 'R', status: 'waiting', deckSize: 36 } })), null);
  assert.equal(chooseMove(null), null);
});

check('Ledger: считает скрытые карты', () => {
  const ledger = new Ledger();
  const s = state({ hand: hand(['6♥', '7♥', '8♥', '9♥', '10♥', 'В♥']) });
  ledger.observe(s);
  const summary = ledger.summary(s);
  assert.equal(summary.unseen.length, 36 - 6 - 1);
  assert.equal(summary.unseenTrumps.length, 8);
  assert.match(summary.line, /в колоде 24/);
});

check('Ledger: битый кон уходит из игры', () => {
  const ledger = new Ledger();
  const withPair = state({
    hand: hand(['6♥']),
    table: [{ attack: card('6♦'), defense: card('7♦') }],
    log: [
      { seq: 1, action: 'attack', cardId: '6♦' },
      { seq: 2, action: 'defend', cardId: '7♦' },
    ],
  });
  ledger.observe(withPair);
  const cleared = state({
    hand: hand(['6♥']),
    table: [],
    log: [
      { seq: 1, action: 'attack', cardId: '6♦' },
      { seq: 2, action: 'defend', cardId: '7♦' },
      { seq: 3, action: 'pass' },
    ],
  });
  ledger.observe(cleared);
  assert.deepEqual([...ledger.dead].sort(), ['6♦', '7♦']);
});

check('Ledger: взятые карты остаются в игре', () => {
  const ledger = new Ledger();
  const withPair = state({
    hand: hand(['6♥']),
    table: [{ attack: card('6♦'), defense: card('7♦') }],
    log: [
      { seq: 1, action: 'attack', cardId: '6♦' },
      { seq: 2, action: 'defend', cardId: '7♦' },
      { seq: 3, action: 'take' },
    ],
  });
  ledger.observe(withPair);
  assert.equal(ledger.dead.size, 0);
  assert.equal(ledger.summary(withPair).unseen.length, 36 - 4);
});

check('Ledger: новый стол сбрасывает учёт', () => {
  const ledger = new Ledger();
  ledger.observe(state({ hand: hand(['6♥']) }));
  ledger.observe(state({ room: { id: 'ROOM02', deckSize: 52, status: 'playing' }, hand: hand(['6♥']) }));
  const s = state({ room: { id: 'ROOM02', deckSize: 52, status: 'playing' }, hand: hand(['6♥']) });
  ledger.observe(s);
  assert.equal(ledger.summary(s).unseen.length, 52 - 1 - 1);
});

console.log(`Пройдено: ${tests}`);
