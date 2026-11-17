import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import { webcrypto } from 'node:crypto';

const source = fs.readFileSync(new URL('../app.js', import.meta.url), 'utf8');
function extract(start, end) {
  return source.slice(source.indexOf(start), source.indexOf(end, source.indexOf(start)));
}
const storage = new Map();
const calls = [];
const context = vm.createContext({
  crypto: webcrypto,
  Uint8Array,
  sessionStorage: { getItem: key => storage.get(key), setItem: (key, value) => storage.set(key, value) },
  state: { game: null },
  toast: message => calls.push(['toast', message]),
  act: (...args) => calls.push(args),
});
vm.runInContext(extract('function readSessionToken()', '\nconst clientId'), context);
const token = vm.runInContext('readSessionToken()', context);
assert.match(token, /^[a-f0-9]{64}$/);
assert.equal(vm.runInContext('readSessionToken()', context), token);
assert.equal(storage.get('durak.token'), token);
vm.runInContext(extract('function cardsWord(', '\nasync function playCard('), context);
vm.runInContext(extract('async function playCard(', '\nasync function act('), context);
const game = {
  you: { role: 'attacker' },
  hand: [{ id: '6♥', rank: '6', suit: '♥' }],
  canAttack: true,
  takePending: true,
  table: [{ attack: { rank: '6', suit: '♠' }, defense: null }],
};
context.state.game = game;
await vm.runInContext('playCard("6♥")', context);
assert.equal(calls.at(-1)[0], 'attack');
game.you.role = 'idle';
await vm.runInContext('playCard("6♥")', context);
assert.equal(calls.at(-1)[0], 'attack');
game.canAttack = false;
game.players = [{ role: 'defender', cards: 1 }];
await vm.runInContext('playCard("6♥")', context);
assert.equal(calls.at(-1)[0], 'toast');
let modalHTML;
let mount;
let handler;
let closed = false;
context.openModal = (html, onMount) => { modalHTML = html; mount = onMount; };
context.closeModal = () => { closed = true; };
context.esc = value => value;
context.document = { querySelectorAll: () => [{ dataset: { transferCard: '6♦' }, addEventListener: (event, callback) => { handler = callback; } }] };
game.canTransfer = true;
game.hand.push({ id: '6♦', rank: '6', suit: '♦' });
vm.runInContext(extract('function selectTransferCard()', '\nfunction beats('), context);
vm.runInContext('selectTransferCard()', context);
assert.ok(modalHTML.includes('data-transfer-card="6♥"'));
assert.ok(modalHTML.includes('data-transfer-card="6♦"'));
mount();
handler();
assert.equal(closed, true);
assert.equal(calls.at(-1)[0], 'transfer');
assert.equal(calls.at(-1)[1], '6♦');
console.log('Клиент: секрет сессии, подкидывание и выбор карты перевода проверены');
