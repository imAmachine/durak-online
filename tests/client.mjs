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

context.avatarHTML = () => '';
context.chips = String;
context.state.game.room = { id: 'ABC123' };
context.api = async (path, payload) => {
  assert.equal(path, '/api/player-profile');
  assert.equal(payload.clientId, 'discord:12345');
  assert.equal(payload.roomId, 'ABC123');
  assert.equal(payload.playerId, 'guest');
  return { balance: 0, games: 5, wins: 2, losses: 3 };
};
vm.runInContext(extract('function openPlayerPreview(', '\nif (!adminPage)'), context);
vm.runInContext('openPlayerPreview({ id: "guest", name: "Гость", cards: 1 })', context);
assert.match(modalHTML, /Загружаем статистику/);
const stats = { textContent: '' };
await mount({ querySelector: () => stats });
assert.match(stats.textContent, /Баланс: 0 фишек.*Партий: 5.*Побед: 2.*Поражений: 3/);
context.api = async () => { throw new Error('offline'); };
await mount({ querySelector: () => stats });
assert.match(stats.textContent, /Не удалось загрузить/);

vm.runInContext(extract('function animateCardFlight(', '\nlet cardDrag'), context);
const flights = [];
const arrivals = [];
const destinationCard = left => ({
  getBoundingClientRect: () => ({ left, right: left + 40, top: 30, width: 40, height: 60 }),
  animate: (_, options) => arrivals.push(options.delay),
});
context.document = {
  body: { appendChild() {} },
  createElement: () => ({ classList: { add() {} }, setAttribute() {}, style: {}, animate: (frames, options) => {
    flights.push([frames[2].translate, options.delay]);
    return {};
  } }),
};
context.deal = {
  game: { players: [{ id: 'opponent' }, { id: 'me' }], you: { id: 'me' } },
  source: { left: 100, top: 200, width: 80, height: 100 },
  seats: { querySelectorAll: () => [{ dataset: { seat: 'opponent' }, querySelectorAll: () => [destinationCard(20), destinationCard(60)] }] },
  hand: { getBoundingClientRect: () => ({ left: 0, right: 300 }), querySelectorAll: () => [destinationCard(200), destinationCard(320)] },
};
vm.runInContext('animateDeal(deal.game, deal.source, deal.hand, deal.seats)', context);
assert.deepEqual(flights, [['-100px -190px', 0], ['80px -190px', 90], ['-60px -190px', 180]]);
assert.deepEqual(arrivals, [360, 450, 540]);

vm.runInContext(extract('function canTransferCard(', '\nfunction renderGame()'), context);
context.RANK_ORDER = ['6', '7', '8'];
vm.runInContext(extract('function beats(', '\nfunction cardsWord('), context);
const zone = { closest: selector => selector === '#tableDropZone' ? zone : null };
const pair = { dataset: { attack: '6♠' }, closest: selector => selector === '#tableCards .pair' ? pair : null };
context.document.querySelector = () => zone;
const drop = (card, element) => {
  context.card = card;
  context.element = element;
  return vm.runInContext('dragMode(card, element)', context);
};
const trumpSix = game.hand[0];
Object.assign(game, {
  you: { role: 'defender', turn: true },
  trump: '♥',
  takePending: false,
  canDefend: true,
  canTransfer: true,
  table: [{ attack: { id: '6♠', rank: '6', suit: '♠' }, defense: null }],
});
assert.equal(drop(trumpSix, null), null);
assert.equal(drop(trumpSix, pair)?.mode, 'defend');
assert.equal(drop(trumpSix, pair)?.row, '6♠');
assert.equal(drop(trumpSix, zone)?.mode, 'transfer');
assert.equal(drop(game.hand[1], pair), null);
assert.equal(drop({ rank: '7', suit: '♠' }, zone), null);
await vm.runInContext('playCard("6♥", "6♠")', context);
assert.deepEqual(calls.at(-1), ['defend', '6♥', '6♠']);
game.canTransfer = false;
assert.equal(drop(trumpSix, zone), null);
assert.equal(drop(trumpSix, pair)?.mode, 'defend');
Object.assign(game, { you: { role: 'attacker', turn: true }, canAttack: true, canDefend: false, table: [] });
assert.equal(drop(trumpSix, zone)?.mode, 'attack');
assert.equal(drop(trumpSix, null), null);
console.log('Клиент: вход, приглашения, подкидывание, перевод и перетаскивание проверены');
