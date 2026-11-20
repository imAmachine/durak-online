import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';

const sharedSource = fs.readFileSync(new URL('../js/shared.js', import.meta.url), 'utf8').replace(/export const /g, 'const ').replace(/export function /g, 'function ');
const cardMarkupSource = fs.readFileSync(new URL('../js/card-markup.js', import.meta.url), 'utf8').replace(/import[^;]+;\n/g, '').replace(/export function /g, 'function ');
const apiSource = fs.readFileSync(new URL('../js/api.js', import.meta.url), 'utf8').replace(/export (async )?function /g, '$1function ');
const source = fs.readFileSync(new URL('../js/app-core.js', import.meta.url), 'utf8').replace(/import[^;]+;\n/g, '').replace(/export (async )?function /g, '$1function ');
const adminSource = fs.readFileSync(new URL('../js/admin.js', import.meta.url), 'utf8')
  .replace(/import \{ ADMIN_FILTERS, AVATAR_TINTS, esc, chips \} from '\.\/shared\.js';\n/, `const AVATAR_TINTS = ['#dceadf', '#f0dfe2', '#f2e7ca', '#dbe7f1', '#e6e0f1', '#f6e2d6'];
const ADMIN_FILTERS = [['all', 'Все'], ['temp', 'Клубные'], ['discord', 'Discord'], ['legacy', 'Старые'], ['online', 'Онлайн']];
function esc(value) { return String(value).replace(/[&<>"']/g, symbol => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[symbol])); }
function chips(value) { return new Intl.NumberFormat('ru-RU').format(Number(value) || 0); }
`)
  .replace(/import \{ createApi \} from '\.\/api\.js';\n/, '')
  .replace("const api = createApi(() => '');", "const adminRequest = createApi(() => '');")
  .replace(/\bapi\(/g, 'adminRequest(');
const storage = new Map();
const calls = [];
function loadClient(admin = false) {
  const elements = Object.fromEntries((admin
    ? ['#adminGate', '#adminLogin', '#adminPanel', '#aKey', '#aCancel', '#aEnter']
    : ['#createBtn', '#heroCreateBtn', '#joinRoomBtn', '#historyBtn', '#gameRules', '#cabinetBack', '#myHand',
      '#mastheadWeekday', '#mastheadDay', '#mastheadTime', '#onlineCount', '#gamesGrid', '#roomCount', '#mastheadNote']
  ).map(id => [id, { listeners: {}, addEventListener(event, callback) { this.listeners[event] = callback; }, setAttribute() {}, focus() {} }]));
  if (admin) elements['#adminGate'].querySelector = selector => elements[selector];
  const context = vm.createContext({
    URL,
    URLSearchParams,
    location: { href: `https://club.test/${admin ? 'admin' : ''}` },
    document: {
      body: { dataset: admin ? { admin: 'page' } : {} },
      addEventListener() {},
      querySelector: selector => elements[selector] || null,
      querySelectorAll: () => [],
    },
    window: { addEventListener() {} },
    ResizeObserver: class { observe() {} },
    setInterval() {},
    localStorage: { getItem: () => null },
    sessionStorage: { getItem: key => storage.get(key), setItem: (key, value) => storage.set(key, value), removeItem: key => storage.delete(key) },
  });
  const started = vm.runInContext(`${apiSource}\n${admin ? adminSource.replace(/export (async )?function /g, '$1function ') : `${cardMarkupSource}\n${source}`}\n${admin ? 'startAdmin();' : 'startMain();'}`, context, { filename: admin ? 'js/admin.js' : 'js/app-core.js' });
  if (!admin) context.api = vm.runInContext('api', context);
  return { context, elements, started };
}
const { context, elements: mainElements, started } = loadClient();
await started;
assert.equal(typeof mainElements['#createBtn'].listeners.click, 'function');
const { context: adminContext, elements: adminElements } = loadClient(true);
assert.equal(adminElements['#adminPanel'].hidden, true);
assert.match(adminElements['#adminGate'].innerHTML, /id="aKey"/);
const state = vm.runInContext('state', context);
Object.assign(context, {
  toast: message => calls.push(['toast', message]),
  act: (...args) => calls.push(args),
});
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
vm.runInContext('clearAuthQuery()', context);
assert.equal(replaced, '/?code=ABC123');
assert.equal(storage.has('durak.discord.invite'), false);
let loginMessage = '';
context.setDiscordLoginState = (enabled, message) => { loginMessage = message; };
context.sessionStorage.getItem = () => { throw new Error('denied'); };
context.sessionStorage.removeItem = () => { throw new Error('denied'); };
context.location.href = 'https://club.test/?code=oauth-code&state=oauth-state';
assert.equal(await vm.runInContext('completeDiscordLogin()', context), false);
assert.match(loginMessage, /хранилищу браузера/);
assert.equal(replaced, '/');

const elements = {
  '#connectionStatus': { textContent: '', dataset: {} },
  '#retryConnection': { hidden: true },
};
context.document = { querySelector: selector => elements[selector] };
context.renderLobby = () => {};
context.EventSource = class {
  static CLOSED = 2;
  constructor(url) { this.url = url; this.readyState = 2; }
  close() {}
};
vm.runInContext("clientId = 'discord:12345'; sessionToken = '" + 'a'.repeat(64) + "';", context);
vm.runInContext('connect()', context);
vm.runInContext('state.source.onerror()', context);
assert.equal(vm.runInContext('clientId', context), 'discord:12345');
assert.equal(vm.runInContext('sessionToken', context), 'a'.repeat(64));
assert.equal(elements['#retryConnection'].hidden, false);
assert.match(elements['#connectionStatus'].textContent, /повторите попытку/);
vm.runInContext('connect()', context);
vm.runInContext('state.source.onopen()', context);
assert.equal(elements['#retryConnection'].hidden, true);

const gameEffects = [];
context.document.querySelector = selector => selector === '#gameScreen'
  ? { setAttribute: (...args) => gameEffects.push(args) }
  : elements[selector];
context.setView = view => { state.view = view; gameEffects.push(view); };
context.renderGame = () => gameEffects.push('render');
context.updateResumeButton = () => gameEffects.push('resume');
state.game = { room: { id: 'ABC123' }, seq: 5 };
state.actionPending = true;
state.actionPendingSeq = 5;
state.restoreJoin = { takeMissedEvents: seq => gameEffects.push(seq) };
const gameEvent = (id, seq) => state.source.onmessage({ data: JSON.stringify({ type: 'game', state: { room: { id }, seq } }) });
await gameEvent('ABC123', 4);
assert.equal(state.game.seq, 5);
assert.deepEqual(gameEffects, []);
await gameEvent('ABC123', 6);
assert.equal(state.actionPending, false);
assert.deepEqual(gameEffects, [['aria-busy', 'false'], 'game', 'render', 'resume', 6]);
assert.equal(state.restoreJoin, null);
await gameEvent('OTHER1', 1);
assert.equal(state.game.room.id, 'OTHER1');
context.fetch = async path => {
  const url = new URL(path, 'https://club.test');
  assert.equal(url.pathname, '/api/watch');
  assert.deepEqual(Object.fromEntries(url.searchParams), { id: 'discord:12345', token: 'a'.repeat(64), since: '1', until: 'any', timeout: '1500' });
  return { ok: true, json: async () => ({ hit: true, state: { room: { id: 'ABC123' }, seq: 0 } }) };
};
await context.catchUpRoom('ABC123', 1);
assert.equal(state.game.room.id, 'OTHER1');
state.actionPending = false;

const moveCard = (game, cardId, target) => {
  const result = context.cardMove(game, cardId, target);
  return result && JSON.parse(JSON.stringify(result));
};
const sixHearts = { id: '6♥', rank: '6', suit: '♥' };
const sevenHearts = { id: '7♥', rank: '7', suit: '♥' };
const attackSix = { id: '6♠', rank: '6', suit: '♠' };
const attackSeven = { id: '7♥', rank: '7', suit: '♥' };
const moveGame = {
  you: { role: 'attacker' }, hand: [sixHearts, sevenHearts], players: [],
  table: [], canAttack: true, trump: '♥',
};
assert.equal(moveCard(null, '6♥'), undefined);
assert.equal(moveCard(moveGame, 'missing'), undefined);
assert.deepEqual(moveCard(moveGame, '6♥'), { args: ['attack', '6♥'] });
const table = [{ attack: attackSix, defense: sevenHearts }];
assert.deepEqual(moveCard({ ...moveGame, table }, '7♥'), { args: ['attack', '7♥'] });
assert.deepEqual(moveCard({ ...moveGame, table: [{ attack: attackSix }] }, '7♥'), { message: 'Можно подкидывать только 6' });
assert.deepEqual(moveCard({ ...moveGame, canAttack: false }, '6♥'), { message: 'Сейчас подкидывать нечем' });
assert.deepEqual(moveCard({ ...moveGame, canAttack: false, table: Array(6).fill({ attack: attackSix }) }, '6♥'), { message: 'На столе уже шесть пар — больше не подкинуть' });
assert.deepEqual(moveCard({ ...moveGame, canAttack: false, table: [{ attack: attackSix }], players: [{ role: 'defender', cards: 1 }] }, '6♥'), { message: 'Сопернику нечем отбиваться: у него 1 карта — больше не подкинуть' });
assert.deepEqual(moveCard({ ...moveGame, you: { role: 'attacker', finished: true } }, '6♥'), { message: 'Сейчас ходит другой игрок' });

const defending = { ...moveGame, you: { role: 'defender' }, canDefend: true, table: [{ attack: attackSix }, { attack: attackSeven }] };
assert.deepEqual(moveCard(defending, '7♥'), { args: ['defend', '7♥', '6♠'] });
assert.deepEqual(moveCard({ ...defending, table: [...defending.table].reverse() }, '7♥'), { args: ['defend', '7♥', '6♠'] });
assert.deepEqual(moveCard(defending, '7♥', '7♥'), { message: 'Карта 7♥ не бьёт 7♥' });
assert.deepEqual(moveCard(defending, '7♥', 'missing'), { message: 'Сейчас отбивать нечего' });
assert.deepEqual(moveCard({ ...defending, table: [{ attack: attackSeven }] }, '6♥'), { message: 'Карта 6♥ не бьёт 7♥' });
assert.deepEqual(moveCard({ ...defending, canDefend: false }, '7♥'), { message: 'Сейчас отбивать нечего' });
assert.deepEqual(moveCard({ ...defending, takePending: true }, '7♥'), { message: 'Вы уже решили взять карты' });
assert.deepEqual(moveCard({ ...defending, canTransfer: true, takePending: true }, '6♥'), { args: ['transfer', '6♥'] });
assert.deepEqual(moveCard({ ...defending, canTransfer: true }, '6♥', '6♠'), { args: ['defend', '6♥', '6♠'] });

const game = {
  you: { role: 'attacker' },
  hand: [{ id: '6♥', rank: '6', suit: '♥' }],
  canAttack: true,
  takePending: true,
  table: [{ attack: { rank: '6', suit: '♠' }, defense: null }],
};
state.game = game;
await vm.runInContext('playCard("6♥")', context);
assert.equal(calls.at(-1)[0], 'attack');
game.you.role = 'idle';
await vm.runInContext('playCard("6♥")', context);
assert.equal(calls.at(-1)[0], 'attack');
game.canAttack = false;
game.players = [{ role: 'defender', cards: 1 }];
await vm.runInContext('playCard("6♥")', context);
assert.equal(calls.at(-1)[0], 'toast');
const players = [
  { id: 'temp:a', name: 'Борис', createdAt: 1, online: true },
  { id: 'discord:b', name: 'Анна', createdAt: 2 },
  { id: 'c', name: 'Вера', online: true },
  { id: 'd', name: 'Денис', kind: 'other' },
];
assert.equal(JSON.stringify(adminContext.countAdminPlayers(players)), '{"all":4,"online":2,"temp":1,"discord":1,"legacy":1}');
assert.deepEqual(Array.from(adminContext.filterAdminPlayers(players, '', 'all', 'recent'), player => player.id), ['discord:b', 'temp:a', 'c', 'd']);
assert.deepEqual(Array.from(adminContext.filterAdminPlayers(players, ' TEMP: ', 'online', 'name'), player => player.id), ['temp:a']);
assert.deepEqual(Array.from(context.opponentSeats({ players, you: players[1] }), player => player.id), ['c', 'd', 'temp:a']);
const opponent = { id: 'op"1', name: '<Анна>', role: 'defender', cards: 2, connected: true };
const seatGame = { room: { status: 'playing' }, turnId: opponent.id };
const seatHTML = context.opponentSeatHTML(opponent, seatGame);
assert.match(seatHTML, /<button class="seat turn" data-seat="op&quot;1" type="button" aria-haspopup="dialog" aria-label="Профиль игрока &lt;Анна&gt;, 2 карты, Защищается">/);
assert.match(seatHTML, /<span class="seat-main"><strong>&lt;Анна&gt;<\/strong><\/span>/);
assert.equal((seatHTML.match(/class="card-back /g) || []).length, 2);
assert.match(seatHTML, /class="role-label defender"[\s\S]*Защищается · Ходит<\/span>/);
const waitingSeatHTML = context.opponentSeatHTML({ ...opponent, role: '', connected: false }, { room: { status: 'waiting' } });
assert.match(waitingSeatHTML, /aria-label="Профиль игрока &lt;Анна&gt;, Не в сети"/);
assert.match(waitingSeatHTML, /<span class="seat-state">Не в сети<\/span>/);
assert.doesNotMatch(waitingSeatHTML, /seat-cards|card-back|role-label/);
assert.equal(context.handView({ ...game, room: { id: 'ABC123' }, you: { turn: true }, canAttack: true }).dimmed({ rank: '7' }), true);

const actionButtons = (game, pending = false) => JSON.parse(JSON.stringify(context.gameActionButtons(game, pending)));
const playingActions = { room: { status: 'playing' }, canTake: true, canPass: true };
assert.deepEqual(actionButtons(playingActions), [
  { label: 'Взять карты', action: 'take', style: 'secondary-btn' },
  { label: 'Бито', action: 'pass', style: 'primary-btn' },
]);
assert.deepEqual(actionButtons({ ...playingActions, takePending: true }), [
  { label: 'Забрать карты', action: 'take', style: 'primary-btn' },
  { label: 'Хватит, забирайте', action: 'pass', style: 'primary-btn' },
]);
assert.deepEqual(actionButtons(playingActions, true), []);
const waitingActions = {
  room: { status: 'waiting', hostId: 'host' },
  players: [{ connected: true }, { connected: true }, { connected: false }],
};
assert.deepEqual(actionButtons({ ...waitingActions, you: { id: 'host' } }), [
  { label: 'Ждём игроков (2/6)', action: null, style: 'secondary-btn', disabled: true },
]);
assert.deepEqual(actionButtons({ ...waitingActions, you: { id: 'guest' } }), [
  { label: 'Ждём, пока создатель начнёт игру', action: null, style: 'secondary-btn', disabled: true },
]);
assert.deepEqual(actionButtons({ ...waitingActions, you: { id: 'host' }, canStart: true }), [
  { label: 'Начать игру', action: 'start', style: 'primary-btn' },
]);

const requests = [];
context.fetch = async (path, options) => {
  requests.push([path, options?.method, options?.headers['Content-Type'], options && JSON.parse(options.body)]);
  return { ok: true, json: async () => ({}) };
};
await context.api('/api/action', { clientId: 'discord:12345', action: 'start' });
adminContext.fetch = context.fetch;
await adminContext.adminApi({ op: 'list' });
await context.authRequest('/api/auth/temp/login', { name: 'Тест', password: 'fake' });
assert.deepEqual(requests, [
  ['/api/action', 'POST', 'application/json', { clientId: 'discord:12345', action: 'start', token: 'a'.repeat(64) }],
  ['/api/admin', 'POST', 'application/json', { key: '', op: 'list' }],
  ['/api/auth/temp/login', 'POST', 'application/json', { name: 'Тест', password: 'fake' }],
]);
context.fetch = async () => ({ ok: false, status: 403, json: async () => ({ error: 'Отказ' }) });
await assert.rejects(context.api('/api/action', {}), /Отказ/);
const malformedJSON = async () => { throw new SyntaxError(); };
context.fetch = async () => ({ ok: false, status: 500, json: malformedJSON });
await assert.rejects(context.api('/api/action', {}), /Не удалось выполнить действие/);
await assert.rejects(context.authRequest('/api/auth/temp/login', {}), /Вход через Discord недоступен/);
context.fetch = async () => ({ ok: true, json: malformedJSON });
assert.equal(Object.keys(await context.api('/api/action', {})).length, 0);
const epoch = new Date(2025, 0, 2, 3, 4).getTime();
assert.equal(adminContext.clubMoment(epoch), '02.01.2025 03:04');
assert.equal(adminContext.clubDate(epoch / 1000, true), '02.01.2025 03:04');
assert.equal(adminContext.clubDate(epoch / 1000), '02.01.2025');
assert.equal(adminContext.clubDate(-1, true), '');
assert.equal(adminContext.clubMoment(Infinity), '');
assert.deepEqual([1, 2, 5, 11, 22].map(context.cardsWord), ['карта', 'карты', 'карт', 'карт', 'карты']);

let modalHTML;
let mount;
context.openModal = (html, onMount) => { modalHTML = html; mount = onMount; };
game.hand.push({ id: '6♦', rank: '6', suit: '♦' });

context.openCreateModal();
assert.match(modalHTML, /<h2>Создать лобби<\/h2>/);
assert.deepEqual([...modalHTML.matchAll(/<option value="(36|52)">/g)].map(match => match[1]), ['36', '52']);
assert.deepEqual([...modalHTML.matchAll(/<option value="(0|50|100|250|500)"/g)].map(match => match[1]), ['0', '50', '100', '250', '500']);
assert.match(modalHTML, /class="primary-btn full" id="mCreate"/);
context.openJoinModal('A"B');
assert.match(modalHTML, /id="mCode"[^>]+value="A&quot;B"/);
assert.match(modalHTML, /class="primary-btn full" id="mJoin"/);
adminContext.openModal = context.openModal;
adminContext.crypto = { getRandomValues: values => values.fill(7) };
adminContext.openTempAccountForm();
assert.match(modalHTML, /id="tPassword" type="text" minlength="4" maxlength="128" value="[^"]+" autocomplete="new-password"/);
adminContext.openPasswordReset({ name: 'Тест', kind: 'temp' });
assert.match(modalHTML, /id="pPassword" type="password" minlength="4" maxlength="128" value="[^"]+" autocomplete="new-password"/);
const sampleCard = { id: '6"♥', rank: '6', suit: '♥' };
assert.match(context.cardHTML(sampleCard, 'attack target', true), /^<div data-target-card="6&quot;♥" class="card-face red attack target" data-card-id="6&quot;♥">/);
assert.doesNotMatch(context.cardHTML(sampleCard, 'defense'), /data-target-card/);

context.avatarHTML = () => '';
context.chips = String;
state.game.room = { id: 'ABC123' };
context.api = async (path, payload) => {
  assert.equal(path, '/api/player-profile');
  assert.equal(payload.clientId, 'discord:12345');
  assert.equal(payload.roomId, 'ABC123');
  assert.equal(payload.playerId, 'guest');
  return { balance: 0, games: 5, wins: 2, losses: 3 };
};
vm.runInContext('openPlayerPreview({ id: "guest", name: "Гость", cards: 1 })', context);
assert.match(modalHTML, /Загружаем статистику/);
const stats = { textContent: '' };
await mount({ querySelector: () => stats });
assert.match(stats.textContent, /Баланс: 0 фишек.*Партий: 5.*Побед: 2.*Поражений: 3/);
context.api = async () => { throw new Error('offline'); };
await mount({ querySelector: () => stats });
assert.match(stats.textContent, /Не удалось загрузить/);

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
await vm.runInContext('playCard("6♦")', context);
assert.deepEqual(calls.at(-1), ['transfer', '6♦']);
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
