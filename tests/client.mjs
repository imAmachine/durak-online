import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';

const source = fs.readFileSync(new URL('../app.js', import.meta.url), 'utf8');
function extract(start, end) {
  return source.slice(source.indexOf(start), source.indexOf(end, source.indexOf(start)));
}
const storage = new Map();
const calls = [];
const context = vm.createContext({
  Uint8Array,
  URL,
  sessionStorage: { getItem: key => storage.get(key), setItem: (key, value) => storage.set(key, value) },
  state: { game: null },
  toast: message => calls.push(['toast', message]),
  act: (...args) => calls.push(args),
});
vm.runInContext(extract('function readCredentials()', '\nlet credentials'), context);
vm.runInContext(extract('function isDiscordCallback', '\nfunction inviteCodeFromUrl'), context);
vm.runInContext(extract('function inviteCodeFromUrl', '\nfunction clearAuthQuery'), context);
assert.equal(vm.runInContext('readCredentials()', context), null);
assert.equal(storage.size, 0);
storage.set('durak.client', 'guest');
storage.set('durak.token', 'a'.repeat(64));
assert.equal(vm.runInContext('readCredentials()', context), null);
storage.set('durak.client', 'discord:12345');
assert.equal(vm.runInContext('readCredentials().clientId', context), 'discord:12345');
storage.set('durak.token', 'invalid');
assert.equal(vm.runInContext('readCredentials()', context), null);
assert.equal(vm.runInContext('inviteCodeFromUrl(new URL("https://club.test/?code=ABC123"))', context), 'ABC123');
assert.equal(vm.runInContext('inviteCodeFromUrl(new URL("https://club.test/?code=oauth-code&state=oauth-state"))', context), null);
assert.equal(vm.runInContext('isDiscordCallback(new URL("https://club.test/?error=access_denied&state=oauth-state"))', context), true);
let replaced;
context.location = { href: 'https://club.test/?code=oauth-code&state=oauth-state' };
context.history = { replaceState: (a, b, url) => { replaced = url; } };
context.document = { title: 'Клуб' };
context.sessionStorage.removeItem = key => storage.delete(key);
storage.set('durak.discord.invite', 'ABC123');
vm.runInContext(extract('function clearAuthQuery()', '\nasync function completeDiscordLogin'), context);
vm.runInContext('clearAuthQuery()', context);
assert.equal(replaced, '/?code=ABC123');
assert.equal(storage.has('durak.discord.invite'), false);
let loginMessage = '';
context.setDiscordLoginState = (enabled, message) => { loginMessage = message; };
context.sessionStorage.getItem = () => { throw new Error('denied'); };
context.sessionStorage.removeItem = () => { throw new Error('denied'); };
context.location.href = 'https://club.test/?code=oauth-code&state=oauth-state';
vm.runInContext(extract('async function completeDiscordLogin()', '\nasync function prepareDiscordLogin'), context);
assert.equal(await vm.runInContext('completeDiscordLogin()', context), false);
assert.match(loginMessage, /хранилищу браузера/);
assert.equal(replaced, '/');

const elements = {
  '#connectionStatus': { textContent: '', dataset: {} },
  '#retryConnection': { hidden: true },
};
context.document = { querySelector: selector => elements[selector] };
context.state = { profile: { name: 'Игрок' }, source: null, connected: false };
context.renderLobby = () => {};
context.EventSource = class {
  static CLOSED = 2;
  constructor(url) { this.url = url; this.readyState = 2; }
  close() {}
};
vm.runInContext("let clientId = 'discord:12345'; let sessionToken = '" + 'a'.repeat(64) + "';", context);
vm.runInContext(extract('function connect()', '\nfunction sortHand('), context);
vm.runInContext('connect()', context);
vm.runInContext('state.source.onerror()', context);
assert.equal(vm.runInContext('clientId', context), 'discord:12345');
assert.equal(vm.runInContext('sessionToken', context), 'a'.repeat(64));
assert.equal(elements['#retryConnection'].hidden, false);
assert.match(elements['#connectionStatus'].textContent, /повторите попытку/);
vm.runInContext('connect()', context);
vm.runInContext('state.source.onopen()', context);
assert.equal(elements['#retryConnection'].hidden, true);

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
console.log('Клиент: вход, приглашения, подкидывание и выбор карты перевода проверены');
