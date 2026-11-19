function readCredentials() {
  try {
    const clientId = sessionStorage.getItem('durak.client');
    const token = sessionStorage.getItem('durak.token');
    if (/^(discord|temp):[A-Za-z0-9_-]+$/.test(clientId || '') && /^[a-f0-9]{64}$/.test(token || '')) return { clientId, token };
  } catch (error) {
    return null;
  }
  return null;
}

let credentials = readCredentials();
let clientId = credentials?.clientId || '';
let sessionToken = credentials?.token || '';

const RANK_ORDER = ['2', '3', '4', '5', '6', '7', '8', '9', '10', 'В', 'Д', 'К', 'Т'];
const SUIT_ORDER = ['♠', '♥', '♦', '♣'];
const AVATAR_TINTS = ['#dceadf', '#f0dfe2', '#f2e7ca', '#dbe7f1', '#e6e0f1', '#f6e2d6'];
const BET_OPTIONS = [0, 50, 100, 250, 500];
const EMPTY_ACCOUNT = { balance: 0, games: 0, wins: 0, losses: 0 };
const adminPage = document.body.dataset.admin === 'page';
const adminState = { players: [], query: '', selected: null, loading: false, filter: 'all', sort: 'recent' };
const ADMIN_FILTERS = [
  ['all', 'Все'],
  ['temp', 'Клубные'],
  ['discord', 'Discord'],
  ['legacy', 'Старые'],
  ['online', 'Онлайн'],
];

const state = {
  profile: loadProfile(),
  account: { ...EMPTY_ACCOUNT },
  lobby: { rooms: [], online: 0, history: [] },
  game: null,
  filter: 'all',
  view: 'lobby',
  autoOpen: true,
  source: null,
  connected: false,
  actionPending: false,
  leavingRoom: false,
  lastEventSeq: 0,
  lastEventRoom: null,
  renderedGame: null,
  suppressClick: false,
};

function loadProfile() {
  try {
    const saved = JSON.parse(localStorage.getItem('durak.profile') || '{}');
    return { name: saved.name || 'Игрок', tint: Number(saved.tint) || 0 };
  } catch (error) {
    return { name: 'Игрок', tint: 0 };
  }
}

function saveProfile() {
  try {
    localStorage.setItem('durak.profile', JSON.stringify(state.profile));
  } catch (error) {
    /* сохранение недоступно — профиль останется на текущую сессию */
  }
}

function esc(value) {
  return String(value).replace(/[&<>"']/g, symbol => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[symbol]));
}

function chips(value) {
  return new Intl.NumberFormat('ru-RU').format(Number(value) || 0);
}

function betLabel(bet) {
  return bet ? `${chips(bet)} фишек` : 'Без ставки';
}

function wordForm(count, forms) {
  const rest10 = count % 10;
  const rest100 = count % 100;
  if (rest10 === 1 && rest100 !== 11) return forms[0];
  if (rest10 >= 2 && rest10 <= 4 && (rest100 < 12 || rest100 > 14)) return forms[1];
  return forms[2];
}

function initials(name) {
  return String(name).trim().split(/\s+/).slice(0, 2).map(part => part[0] || '').join('').toUpperCase() || '?';
}

function tintOf(person) {
  if (person && typeof person === 'object' && person.id === clientId) return AVATAR_TINTS[state.profile.tint % AVATAR_TINTS.length];
  let hash = 0;
  const name = typeof person === 'string' ? person : (person && person.name) || '?';
  for (const symbol of name) hash = (hash * 31 + symbol.charCodeAt(0)) % 1000;
  return AVATAR_TINTS[hash % AVATAR_TINTS.length];
}

function avatarHTML(person, className = 'mini-avatar') {
  const name = typeof person === 'string' ? person : (person && person.name) || '?';
  const tint = tintOf(person);
  return `<span class="${className} avatar-tint" style="background:${tint}">${esc(initials(name))}</span>`;
}

async function api(path, body) {
  const response = await fetch(path, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ ...body, ...(body.clientId ? { token: sessionToken } : {}) }) });
  const data = await response.json().catch(() => ({}));
  if (!response.ok) throw new Error(data.error || 'Не удалось выполнить действие');
  return data;
}

let toastTimer = null;
function toast(message) {
  const element = document.querySelector('#toast');
  element.textContent = message;
  element.classList.add('show');
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => element.classList.remove('show'), 2600);
}

/* modal */
const backdrop = document.querySelector('#modalBackdrop');
let modalTrigger = null;
function openModal(html, onMount) {
  if (!backdrop) return;
  if (!backdrop.classList.contains('open')) modalTrigger = document.activeElement;
  const body = document.querySelector('#modalBody');
  body.innerHTML = html;
  backdrop.inert = false;
  backdrop.setAttribute('aria-hidden', 'false');
  backdrop.classList.add('open');
  if (document.querySelector('#appShell')) document.querySelector('#appShell').inert = true;
  document.body.classList.add('modal-open');
  const heading = body.querySelector('h2');
  if (heading) {
    heading.id = 'modalTitle';
    backdrop.querySelector('.modal').setAttribute('aria-labelledby', 'modalTitle');
  }
  if (onMount) onMount(body);
  (body.querySelector('input, select, button') || backdrop.querySelector('#modalClose')).focus();
}
function closeModal() {
  if (!backdrop) return;
  backdrop.classList.remove('open');
  if (document.querySelector('#appShell')) document.querySelector('#appShell').inert = false;
  document.body.classList.remove('modal-open');
  if (modalTrigger && modalTrigger.isConnected) modalTrigger.focus();
  backdrop.inert = true;
  backdrop.setAttribute('aria-hidden', 'true');
}
if (document.querySelector('#modalClose')) document.querySelector('#modalClose').addEventListener('click', closeModal);
let backdropPressStarted = false;
if (backdrop) backdrop.addEventListener('pointerdown', event => {
  backdropPressStarted = event.target === backdrop;
});
if (backdrop) backdrop.addEventListener('pointerup', event => {
  if (backdropPressStarted && event.target === backdrop) closeModal();
  backdropPressStarted = false;
});
if (backdrop) backdrop.addEventListener('pointercancel', () => {
  backdropPressStarted = false;
});
if (backdrop) backdrop.addEventListener('click', event => {
  if (event.target === backdrop) event.preventDefault();
});
document.addEventListener('keydown', event => {
  if (!backdrop || !backdrop.classList.contains('open')) return;
  if (event.key === 'Escape') closeModal();
  if (event.key === 'Tab') {
    const controls = [...backdrop.querySelectorAll('button:not(:disabled), input, select, [tabindex="0"]')].filter(element => element.getClientRects().length);
    const first = controls[0];
    const last = controls[controls.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  }
});

/* profile */
if (document.querySelector('#profileBtn')) document.querySelector('#profileBtn').addEventListener('click', () => {
  renderCabinet();
  setView('cabinet');
});

async function authRequest(path, body) {
  const response = await fetch(path, body ? { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) } : undefined);
  const data = await response.json().catch(() => ({}));
  if (!response.ok) throw new Error(data.error || 'Вход через Discord недоступен');
  return data;
}

async function prepareTemporaryLogin() {
  const button = document.querySelector('#tempLogin');
  if (!button) return;
  const submit = async () => {
    const name = document.querySelector('#tempLoginName').value.trim();
    const password = document.querySelector('#tempLoginPassword').value;
    button.disabled = true;
    try {
      const data = await authRequest('/api/auth/temp/login', { name, password });
      if (!/^temp:[A-Za-z0-9_-]{1,120}$/.test(data.clientId || '') || !/^[a-f0-9]{64}$/.test(data.token || '')) throw new Error('Сервер вернул неверные данные входа');
      sessionStorage.setItem('durak.client', data.clientId);
      sessionStorage.setItem('durak.token', data.token);
      location.reload();
    } catch (error) {
      button.disabled = false;
      setDiscordLoginState(true, `Не удалось войти: ${error.message}`);
    }
  };
  button.addEventListener('click', submit);
  document.querySelector('#tempLoginPassword').addEventListener('keydown', event => {
    if (event.key === 'Enter') submit();
  });
}

function setDiscordLoginState(enabled, message) {
  const button = document.querySelector('#discordLogin');
  const status = document.querySelector('#discordStatus');
  if (!button || !status) return;
  button.disabled = !enabled;
  button.textContent = enabled ? 'Войти через Discord' : 'Вход через Discord';
  status.textContent = message;
}

function isDiscordCallback(url) {
  return url.searchParams.has('state') && (url.searchParams.has('code') || url.searchParams.has('error'));
}

function inviteCodeFromUrl(url) {
  return isDiscordCallback(url) ? null : url.searchParams.get('code');
}

function dropInviteQuery() {
  const url = new URL(location.href);
  url.searchParams.delete('code');
  history.replaceState(null, document.title, `${url.pathname}${url.search}${url.hash}`);
}

function clearAuthQuery() {
  const url = new URL(location.href);
  url.searchParams.delete('code');
  url.searchParams.delete('state');
  url.searchParams.delete('error');
  url.searchParams.delete('error_description');
  try {
    const invite = sessionStorage.getItem('durak.discord.invite');
    if (invite) url.searchParams.set('code', invite);
    sessionStorage.removeItem('durak.discord.invite');
  } catch (error) {}
  history.replaceState(null, document.title, `${url.pathname}${url.search}${url.hash}`);
}

async function completeDiscordLogin() {
  const url = new URL(location.href);
  if (!isDiscordCallback(url)) return false;
  const code = url.searchParams.get('code');
  const stateValue = url.searchParams.get('state');
  let flowSecret;
  try {
    flowSecret = sessionStorage.getItem('durak.discord.flowSecret');
  } catch (error) {
    clearAuthQuery();
    setDiscordLoginState(false, 'Не удалось получить доступ к хранилищу браузера. Разрешите хранение данных и попробуйте снова.');
    return false;
  }
  clearAuthQuery();
  if (!code || !stateValue || !flowSecret) {
    setDiscordLoginState(false, 'Вход через Discord отменён или не подтверждён. Попробуйте ещё раз.');
    try { sessionStorage.removeItem('durak.discord.flowSecret'); } catch (error) {}
    return false;
  }
  try {
    const data = await authRequest('/api/auth/discord/complete', { code, state: stateValue, flowSecret });
    if (!/^(discord|temp):[A-Za-z0-9_-]{1,120}$/.test(data.clientId || '') || !/^[a-f0-9]{64}$/.test(data.token || '')) throw new Error('Сервер вернул неверные данные входа');
    try {
      sessionStorage.setItem('durak.client', data.clientId);
      sessionStorage.setItem('durak.token', data.token);
      sessionStorage.removeItem('durak.discord.flowSecret');
    } catch (storageError) {
      try { sessionStorage.removeItem('durak.client'); sessionStorage.removeItem('durak.token'); } catch (cleanupError) {}
      throw new Error('Разрешите хранение данных в браузере и попробуйте снова');
    }
    location.reload();
    return true;
  } catch (error) {
    try { sessionStorage.removeItem('durak.discord.flowSecret'); } catch (storageError) {}
    setDiscordLoginState(false, `Не удалось войти: ${error.message}`);
    return false;
  }
}

async function prepareDiscordLogin() {
  const button = document.querySelector('#discordLogin');
  if (button) button.addEventListener('click', async () => {
    if (button.disabled) return;
    button.disabled = true;
    try {
      const data = await authRequest('/api/auth/discord/start', {});
      if (!data.url || !data.flowSecret) throw new Error('Сервер не вернул данные входа');
      const invite = inviteCodeFromUrl(new URL(location.href));
      if (invite) sessionStorage.setItem('durak.discord.invite', invite);
      else sessionStorage.removeItem('durak.discord.invite');
      sessionStorage.setItem('durak.discord.flowSecret', data.flowSecret);
      location.assign(data.url);
    } catch (error) {
      button.disabled = false;
      setDiscordLoginState(true, `Не удалось начать вход: ${error.message}`);
    }
  });
  try {
    const data = await authRequest('/api/auth/providers');
    if (Array.isArray(data.providers) && data.providers.includes('discord')) {
      const status = document.querySelector('#discordStatus');
      setDiscordLoginState(true, status.textContent.includes('Не удалось') || status.textContent.includes('отменён') || status.textContent.includes('истекла') || status.textContent.includes('хранилищу') ? status.textContent : 'Войдите, чтобы присоединиться к клубу.');
    } else {
      setDiscordLoginState(false, 'Вход через Discord пока не настроен');
    }
  } catch (error) {
    setDiscordLoginState(false, 'Вход через Discord недоступен');
  }
}
function renderProfile() {
  const name = state.account.name || state.profile.name;
  const nameNode = document.querySelector('#profileName');
  const avatar = document.querySelector('#profileAvatar');
  if (!nameNode || !avatar) return;
  nameNode.textContent = name;
  avatar.textContent = initials(name);
  avatar.style.background = AVATAR_TINTS[state.profile.tint % AVATAR_TINTS.length];
  avatar.style.backgroundImage = state.account.avatarUrl ? `url("${state.account.avatarUrl}")` : '';
  avatar.classList.toggle('has-image', Boolean(state.account.avatarUrl));
  const balance = document.querySelector('#profileBalance');
  if (balance) balance.textContent = `${chips(state.account.balance)} фишек`;
}
renderProfile();

function openProfileModal() {
  const account = state.account;
  openModal(`
    <div class="modal-label">УЧЁТНАЯ ЗАПИСЬ</div>
    <h2>Мой профиль</h2>
    <p class="modal-subtitle">Имя видят другие игроки в лобби и за столом</p>
    <div class="wallet">
      <div class="wallet-main">
        <span>Баланс клуба</span>
        <strong>${chips(account.balance)}<small>фишек</small></strong>
      </div>
      <div class="wallet-stats">
        <span><b>${account.games}</b>партий</span>
        <span><b>${account.wins}</b>побед</span>
        <span><b>${account.losses}</b>поражений</span>
      </div>
    </div>
    <label>Имя игрока<input id="mName" maxlength="20" value="${esc(account.name || state.profile.name)}"></label>
    <label>Цвет аватара
      <div class="swatches" id="mSwatches">
        ${AVATAR_TINTS.map((tint, index) => `<button class="swatch ${index === state.profile.tint ? 'active' : ''}" data-tint="${index}" style="background:${tint}" aria-label="Цвет ${index + 1}"></button>`).join('')}
      </div>
    </label>
    <div class="modal-actions">
      <button class="secondary-btn" id="mCancel">Отмена</button>
      <button class="primary-btn" id="mSave">Сохранить</button>
    </div>
    <p class="profile-foot"><button class="link-btn" id="mQuit">Выйти из аккаунта</button></p>
  `, root => {
    let tint = state.profile.tint;
    root.querySelector('#mSwatches').addEventListener('click', event => {
      const button = event.target.closest('.swatch');
      if (!button) return;
      tint = Number(button.dataset.tint);
      root.querySelectorAll('.swatch').forEach(item => item.classList.toggle('active', item === button));
    });
    root.querySelector('#mCancel').addEventListener('click', closeModal);
    root.querySelector('#mQuit').addEventListener('click', signOut);
    root.querySelector('#mSave').addEventListener('click', async () => {
      const name = root.querySelector('#mName').value.trim().slice(0, 20);
      if (!name) return toast('Введите имя');
      const button = root.querySelector('#mSave');
      if (button.disabled) return;
      button.disabled = true;
      try {
        const data = await api('/api/profile', { clientId, name });
        state.profile = { name, tint };
        saveProfile();
        if (data.account) state.account = data.account;
        renderProfile();
        closeModal();
        toast('Профиль сохранён');
      } catch (error) {
        button.disabled = false;
        toast(error.message);
      }
    });
  });
}

/* personal cabinet */
function winRate(account) {
  const games = Number(account.games) || 0;
  return games ? Math.round(((Number(account.wins) || 0) / games) * 100) : 0;
}

function renderCabinet() {
  const account = state.account;
  const history = Array.isArray(state.lobby.history) ? state.lobby.history : [];
  const recent = history.slice(0, 8);
  const build = document.querySelector('#cabBuild');
  if (build) {
    build.innerHTML = `
      <div class="cab-hero">
        <span class="cab-avatar avatar" style="background:${AVATAR_TINTS[state.profile.tint % AVATAR_TINTS.length]}${account.avatarUrl ? `;background-image:url("${esc(account.avatarUrl)}")` : ''}">${esc(initials(account.name || state.profile.name))}</span>
        <div class="cab-identity">
          <p class="eyebrow">ЛИЧНЫЙ КАБИНЕТ</p>
          <h1>${esc(account.name || state.profile.name)}</h1>
          <p class="cab-sub">${account.id ? (String(account.id).startsWith('temp:') ? 'Аккаунт клуба' : 'Аккаунт Discord') : 'Гость клуба'}${account.createdAt ? ` · в клубе с ${esc(clubDate(account.createdAt))}` : ''}</p>
          ${account.id ? `<div class="cab-meta"><span class="cab-meta-label">id</span>${copyButtonHTML(account.id, 'id профиля')}</div>` : ''}
        </div>
        <div class="cab-balance">
          <span>Баланс</span>
          <strong>${chips(account.balance)}<small>фишек</small></strong>
        </div>
      </div>
      <div class="cab-stats">
        <article><span>Партии</span><strong>${account.games || 0}</strong></article>
        <article><span>Победы</span><strong>${account.wins || 0}</strong></article>
        <article><span>Поражения</span><strong>${account.losses || 0}</strong></article>
        <article><span>Доля побед</span><strong>${winRate(account)}%</strong></article>
      </div>
      <div class="cab-columns">
        <section class="cab-card">
          <h2>Последние партии клуба</h2>
          ${recent.length ? `<ul class="cab-history">${recent.map(item => `
            <li>
              <span class="cab-history-main"><strong>${esc(item.room || 'Партия')}</strong><small>${item.mode === 'transfer' ? 'Переводной' : 'Подкидной'}${item.bet ? ` · ставка ${chips(item.bet)}` : ''}</small></span>
              <span class="cab-history-side"><em>${esc(item.loser || '—')}</em><small>${esc(clubMoment(item.at))}</small></span>
            </li>`).join('')}</ul>` : '<p class="cab-empty">Партий пока не было. Соберите стол.</p>'}
        </section>
        <section class="cab-card">
          <h2>Как распорядиться фишками</h2>
          <ul class="cab-tips">
            <li><b>Ставка за столом</b><span>Каждый вносит ставку на старте, банк делят все, кроме дурака.</span></li>
            <li><b>Без ставки</b><span>Игра в своё удовольствие: фишки не списываются.</span></li>
            <li><b>Пополнение</b><span>Баланс меняет администратор клуба в панели.</span></li>
          </ul>
          <div class="cab-actions">
            <button class="primary-btn" id="cabOpenGame">${state.game ? 'Вернуться в игру' : 'Создать стол'}</button>
            <button class="secondary-btn" id="cabEdit">Изменить профиль</button>
            <button class="ghost-btn" id="cabQuit">Выйти из аккаунта</button>
          </div>
        </section>
      </div>
    `;
    const openGame = build.querySelector('#cabOpenGame');
    openGame.addEventListener('click', () => { if (state.game) setView('game'); else openCreateModal(); });
    bindCopyButtons(build);
    build.querySelector('#cabEdit').addEventListener('click', openProfileModal);
    build.querySelector('#cabQuit').addEventListener('click', signOut);
  }
}

function signOut() {
  try {
    sessionStorage.removeItem('durak.client');
    sessionStorage.removeItem('durak.token');
    sessionStorage.removeItem('durak.admin');
    sessionStorage.removeItem('durak.room');
  } catch (error) {
    /* хранилище недоступно: выйти из аккаунта нельзя */
  }
  if (state.source) state.source.close();
  location.href = '/';
}

/* lobby */
function renderMasthead() {
  const now = new Date();
  const weekday = now.toLocaleDateString('ru-RU', { weekday: 'long' });
  document.querySelector('#mastheadWeekday').textContent = weekday.charAt(0).toUpperCase() + weekday.slice(1);
  document.querySelector('#mastheadDay').textContent = now.toLocaleDateString('ru-RU', { day: 'numeric', month: 'long' });
  document.querySelector('#mastheadTime').textContent = now.toLocaleTimeString('ru-RU', { hour: '2-digit', minute: '2-digit' });
}

function renderLobby() {
  document.querySelector('#onlineCount').textContent = state.lobby.online;
  const grid = document.querySelector('#gamesGrid');
  const rooms = state.lobby.rooms.filter(room => state.filter === 'all' || room.mode === state.filter);
  document.querySelector('#roomCount').textContent = rooms.length ? String(rooms.length).padStart(2, '0') : '';
  const tables = state.lobby.rooms.length;
  const online = Number(state.lobby.online) || 0;
  document.querySelector('#mastheadNote').textContent = tables
    ? `${tables} ${wordForm(tables, ['стол', 'стола', 'столов'])} · ${online} ${wordForm(online, ['игрок', 'игрока', 'игроков'])}`
    : 'Клуб открыт, столы свободны';
  grid.setAttribute('aria-busy', String(!state.connected));
  if (!state.connected) {
    grid.innerHTML = '<div class="empty-state"><span class="empty-symbol" aria-hidden="true">♠</span><h3>Готовим столы</h3><p>Ждём соединения с сервером. Подключимся автоматически.</p></div>';
    return;
  }
  if (!rooms.length) {
    const filtered = state.filter !== 'all';
    grid.innerHTML = `<div class="empty-state"><span class="empty-symbol" aria-hidden="true">♠</span><div><h3>${filtered ? 'Таких столов пока нет' : 'Первый стол сегодня за вами'}</h3><p>${filtered ? 'Выберите другой режим или создайте свой стол.' : 'Пригласите друзей. Для хорошей партии достаточно двоих.'}</p></div><button class="secondary-btn" id="emptyCreateBtn">Создать стол <span aria-hidden="true">↗</span></button></div>`;
    grid.querySelector('#emptyCreateBtn').addEventListener('click', openCreateModal);
    return;
  }
  const markup = rooms.map(room => {
    const modeLabel = room.mode === 'transfer' ? 'Переводной' : 'Подкидной';
    const playing = room.status === 'playing';
    const mine = Boolean(state.game && state.game.room.id === room.id);
    const bet = Number(room.bet) || 0;
    const extra = room.players.length - 4;
    const canAfford = !bet || mine || !state.account.id || bet <= state.account.balance;
    const label = mine ? 'Вернуться' : playing ? 'Идёт игра' : canAfford ? 'Войти' : 'Мало фишек';
    return `
      <article class="game-card">
        <div class="game-head">
          <div>
            <h3 class="game-title">${esc(room.name)}</h3>
            <div class="game-meta">Создатель: ${esc(room.host)}</div>
          </div>
          <div class="game-flags">
            <span class="mode ${room.mode}">${modeLabel}</span>
            ${bet ? `<span class="bet-pill" title="Ставка за столом">${chips(bet)} фишек</span>` : ''}
          </div>
        </div>
        <div class="game-foot">
          <div class="stacked">${room.players.slice(0, 4).map(player => avatarHTML(player)).join('')}${extra > 0 ? `<span class="mini-avatar more" title="Ещё ${extra} за столом">+${extra}</span>` : ''}</div>
          <span class="slots">${room.players.length} / ${room.seats} · ${room.deckSize} карт</span>
          <button class="join-btn" data-room="${esc(room.id)}" ${(playing && !mine) || !canAfford ? 'disabled' : ''}>${label}</button>
        </div>
      </article>
    `;
  }).join('');
  if (grid.innerHTML === markup) return;
  grid.innerHTML = markup;
  grid.querySelectorAll('.join-btn').forEach(button => {
    button.addEventListener('click', () => {
      if (state.game && state.game.room.id === button.dataset.room) return setView('game');
      joinRoom(button.dataset.room);
    });
  });
}

if (document.querySelector('#lobbyFilter')) document.querySelector('#lobbyFilter').addEventListener('click', event => {
  const button = event.target.closest('button[data-filter]');
  if (!button) return;
  state.filter = button.dataset.filter;
  document.querySelectorAll('#lobbyFilter button').forEach(item => {
    item.classList.toggle('filter-active', item === button);
    item.setAttribute('aria-pressed', String(item === button));
  });
  renderLobby();
});

async function joinRoom(code) {
  try {
    const data = await api('/api/join', { clientId, roomId: String(code).toUpperCase() });
    if (data.account) state.account = data.account;
    if (data.room) rememberSeatRoom(data.room.id);
    renderProfile();
    openGameView();
    toast(`Вы вошли в «${data.room.name}»`);
    return true;
  } catch (error) {
    toast(error.message);
    return false;
  }
}

function openGameView() {
  if (state.game) {
    state.autoOpen = false;
    setView('game');
    return;
  }
  state.autoOpen = true;
}

/* create / join / history / rules / settings modals */
function openCreateModal() {
  const defaultName = `${state.account.name || state.profile.name} — партия`;
  const options = BET_OPTIONS.map(bet => `<option value="${bet}" ${bet > state.account.balance ? 'disabled' : ''}>${betLabel(bet)}</option>`).join('');
  openModal(`
    <div class="modal-label">НОВАЯ ИГРА</div>
    <h2>Создать лобби</h2>
    <p class="modal-subtitle">После создания вы получите код приглашения для друзей</p>
    <label>Название лобби<input id="mRoomName" maxlength="40" value="${esc(defaultName)}"></label>
    <div class="modal-grid">
      <label>Режим
        <select id="mMode">
          <option value="classic">Подкидной</option>
          <option value="transfer">Переводной</option>
        </select>
        <small class="field-hint">Подкидывать можно в обоих режимах; переводной добавляет перевод хода</small>
      </label>
      <label>Колода
        <select id="mDeck">
          <option value="36">36 карт</option>
          <option value="52">52 карты</option>
        </select>
      </label>
      <label class="wide">Ставка
        <select id="mBet">${options}</select>
      </label>
    </div>
    <p class="modal-note">Ставку вносит каждый игрок при старте партии. Дурак остаётся без своей ставки, остальные делят банк. Баланс: ${chips(state.account.balance)} фишек.</p>
    <button class="primary-btn full" id="mCreate">Создать лобби</button>
  `, root => {
    root.querySelector('#mBet').addEventListener('change', event => {
      if (Number(event.target.value) > state.account.balance) {
        event.target.value = '0';
        toast('Недостаточно фишек для такой ставки');
      }
    });
    root.querySelector('#mCreate').addEventListener('click', async () => {
      const button = root.querySelector('#mCreate');
      button.disabled = true;
      try {
        const data = await api('/api/rooms', {
          clientId,
          name: root.querySelector('#mRoomName').value.trim() || 'Партия',
          mode: root.querySelector('#mMode').value,
          deckSize: Number(root.querySelector('#mDeck').value),
          bet: Number(root.querySelector('#mBet').value),
        });
        if (data.account) state.account = data.account;
        if (data.room) rememberSeatRoom(data.room.id);
        renderProfile();
        state.autoOpen = true;
        closeModal();
        openGameView();
        toast(data.room.bet ? `Лобби создано. Код: ${data.code} · ставка ${chips(data.room.bet)}` : `Лобби создано. Код: ${data.code}`);
      } catch (error) {
        button.disabled = false;
        toast(error.message);
      }
    });
  });
}

function openJoinModal(code = '') {
  openModal(`
    <div class="modal-label">ПРИСОЕДИНИТЬСЯ</div>
    <h2>Войти в лобби</h2>
    <p class="modal-subtitle">Введите код, который прислал создатель игры</p>
    <label>Код приглашения<input id="mCode" maxlength="12" placeholder="например, A1B2C3" value="${esc(code)}" style="text-transform:uppercase;letter-spacing:2px"></label>
    <button class="primary-btn full" id="mJoin">Войти в игру</button>
  `, root => {
    const input = root.querySelector('#mCode');
    input.focus();
    const submit = async () => {
      const value = input.value.trim();
      if (!value) return toast('Введите код приглашения');
      const button = root.querySelector('#mJoin');
      if (button.disabled) return;
      button.disabled = true;
      const joined = await joinRoom(value);
      if (joined) closeModal();
      else button.disabled = false;
    };
    root.querySelector('#mJoin').addEventListener('click', submit);
    input.addEventListener('keydown', event => {
      if (event.key === 'Enter') {
        event.preventDefault();
        submit();
      }
    });
  });
}

function bindMainPage() {
document.querySelector('#createBtn').addEventListener('click', openCreateModal);
document.querySelector('#heroCreateBtn').addEventListener('click', openCreateModal);
document.querySelector('#joinRoomBtn').addEventListener('click', () => openJoinModal());

document.querySelector('#historyBtn').addEventListener('click', () => {
  const items = state.lobby.history;
  openModal(`
    <div class="modal-label">АРХИВ</div>
    <h2>История игр</h2>
    <p class="modal-subtitle">Последние завершённые партии на сервере</p>
    ${items.length ? `<ul class="history-list">${items.map(item => `<li><span>${esc(item.room)}<br><small>${item.mode === 'transfer' ? 'Переводной' : 'Подкидной'}${item.bet ? ` · ставка ${chips(item.bet)}` : ''} · ${clubMoment(item.at)}</small></span><strong>${item.loser === 'Ничья' ? 'Ничья' : `${esc(item.loser)} проиграл`}</strong></li>`).join('')}</ul>` : '<div class="empty-state">Пока ни одна партия не завершилась</div>'}
  `);
});

const RULES_HTML = `
  <div class="modal-label">ПРАВИЛА</div>
  <h2>Как играть</h2>
  <p class="modal-subtitle">Сервер сам проверяет ходы, подкидывание, козыри и перевод</p>
  <ul class="rules-list">
    <li>Козырь определяется по нижней карте колоды и бьёт любую некозырную масть.</li>
    <li>Внутри одной масти старшая карта бьёт младшую. Первым ходит владелец младшего козыря.</li>
    <li>После первой атаки подкидывать могут все активные игроки, кроме защитника. Достоинство должно быть на столе (включая отбитые карты). Лимит: шесть атак за ход и не больше неотбитых карт, чем карт в руке защитника.</li>
    <li>Атакующий может выложить сразу несколько карт одного достоинства, не дожидаясь ответа защитника.</li>
    <li>Защищающийся отбивает карты по одной или объявляет «беру»: после этого атакующий может подкинуть ещё карты, а затем карты со стола уходят защитнику.</li>
    <li>«Бито» доступно, когда все карты на столе отбиты; тогда атакующим становится защищавшийся.</li>
    <li>В переводном режиме можно выбрать карту того же достоинства и перевести ход, пока ни одна карта не отбита. У следующего защитника должно хватать карт на все атаки.</li>
    <li>После каждого хода игроки добирают карты до шести: атакующий первым, защищающийся последним.</li>
    <li>Игрок без карт выбывает после завершения хода, если колода пуста. Последний оставшийся с картами проигрывает; если карт нет ни у кого, это ничья.</li>
    <li>Выход активного игрока из комнаты завершает партию его поражением. Если все отключились, через пять минут партия отменяется с возвратом ставок.</li>
  </ul>
`;

function openRulesModal() {
  openModal(RULES_HTML);
}
document.querySelector('#gameRules').addEventListener('click', openRulesModal);
}

if (!adminPage) bindMainPage();

function adminKey() {
  try {
    return sessionStorage.getItem('durak.admin') || '';
  } catch (error) {
    return '';
  }
}

function storeAdminKey(key) {
  try {
    if (key) sessionStorage.setItem('durak.admin', key);
    else sessionStorage.removeItem('durak.admin');
  } catch (error) {
    /* ключ не сохранится между перезагрузками */
  }
}

function adminApi(payload) {
  return api('/api/admin', { key: adminKey(), ...payload });
}

if (!adminPage && new URL(location.href).searchParams.get('admin') === '1') {
  location.replace(adminKey() ? '/admin' : '/admin.html');
}

function bootstrapAdminPage() {
  const screen = document.querySelector('#adminLogin');
  const panel = document.querySelector('#adminPanel');
  if (!adminKey()) {
    if (screen) screen.hidden = false;
    if (panel) panel.hidden = true;
    return;
  }
  if (screen) screen.hidden = true;
  if (panel) panel.hidden = false;
  loadAdminPlayers();
}

function adminGateHTML() {
  return `
    <div class="modal-label">ДОСТУП</div>
    <h2>Панель клуба</h2>
    <p class="modal-subtitle">Ключ задаётся переменной окружения ADMIN_KEY на сервере</p>
    <label>Ключ администратора<input id="aKey" type="password" autocomplete="off" placeholder="Ключ"></label>
    <div class="modal-actions">
      <button class="secondary-btn" id="aCancel">На главную</button>
      <button class="primary-btn" id="aEnter">Войти</button>
    </div>
  `;
}

function bindAdminGate(root, onEnter) {
  const input = root.querySelector('#aKey');
  const status = document.querySelector('#adminLoginStatus');
  if (status) status.textContent = 'Введите ключ, чтобы управлять игроками.';
  input.focus();
  const submit = async () => {
    const key = input.value.trim();
    if (!key) return toast('Введите ключ');
    const button = root.querySelector('#aEnter');
    if (button.disabled) return;
    button.disabled = true;
    try {
      await api('/api/admin', { key, op: 'list' });
      storeAdminKey(key);
      if (status) status.textContent = 'Ключ принят.';
      onEnter(key);
    } catch (error) {
      button.disabled = false;
      toast(error.message);
    }
  };
  root.querySelector('#aCancel').addEventListener('click', () => { location.href = '/'; });
  root.querySelector('#aEnter').addEventListener('click', submit);
  input.addEventListener('keydown', event => {
    if (event.key === 'Enter') {
      event.preventDefault();
      submit();
    }
  });
}

async function loadAdminPlayers() {
  if (!adminKey()) return signOutAdmin();
  adminState.loading = true;
  renderAdminPage();
  try {
    const data = await adminApi({ op: 'list' });
    adminState.players = data.players || [];
  } catch (error) {
    toast(error.message);
    if (/ключ/i.test(error.message)) signOutAdmin();
  } finally {
    adminState.loading = false;
    renderAdminPage();
  }
}

function signOutAdmin() {
  storeAdminKey('');
  if (adminPage) {
    adminState.players = [];
    adminState.selected = null;
    const screen = document.querySelector('#adminLogin');
    const panel = document.querySelector('#adminPanel');
    if (screen) screen.hidden = false;
    if (panel) panel.hidden = true;
    const gate = screen && screen.querySelector('#adminGate');
    if (gate && !gate.dataset.ready) {
      gate.dataset.ready = '1';
      gate.innerHTML = adminGateHTML();
      bindAdminGate(gate, key => { storeAdminKey(key); if (screen) screen.hidden = true; if (panel) panel.hidden = false; loadAdminPlayers(); });
    }
    return;
  }
  closeModal();
  toast('Доступ к панели закрыт');
}

function adminFilteredPlayers() {
  const term = adminState.query.trim().toLowerCase();
  const players = adminState.players.filter(player => {
    if (adminState.filter === 'online' && !player.online) return false;
    if (adminState.filter !== 'all' && adminState.filter !== 'online' && adminKind(player) !== adminState.filter) return false;
    if (!term) return true;
    return player.name.toLowerCase().includes(term) || player.id.toLowerCase().includes(term);
  });
  const sorters = {
    recent: (a, b) => (Number(b.createdAt) || 0) - (Number(a.createdAt) || 0) || a.name.localeCompare(b.name, 'ru'),
    name: (a, b) => a.name.localeCompare(b.name, 'ru'),
    balance: (a, b) => b.balance - a.balance || a.name.localeCompare(b.name, 'ru'),
    games: (a, b) => b.games - a.games || b.wins - a.wins || a.name.localeCompare(b.name, 'ru'),
    online: (a, b) => Number(b.online) - Number(a.online) || a.name.localeCompare(b.name, 'ru'),
  };
  return players.sort(sorters[adminState.sort] || sorters.recent);
}

function adminFilterCount(kind) {
  if (kind === 'all') return adminState.players.length;
  if (kind === 'online') return adminState.players.filter(player => player.online).length;
  return adminState.players.filter(player => adminKind(player) === kind).length;
}

function copyIconHTML() {
  return '<svg width="11" height="11" viewBox="0 0 16 16" aria-hidden="true"><rect x="5.5" y="5.5" width="8" height="8" rx="1.5" fill="none" stroke="currentColor" stroke-width="1.4"/><path d="M10.5 3.5v-1a1 1 0 0 0-1-1h-6a1 1 0 0 0-1 1v6a1 1 0 0 0 1 1h1" fill="none" stroke="currentColor" stroke-width="1.4"/></svg>';
}

function copyButtonHTML(value, label) {
  return `<button type="button" class="admin-copy" data-copy="${esc(value)}" title="Скопировать: ${esc(value)}" aria-label="Скопировать ${esc(label || 'id')}">${copyIconHTML()}<span>${esc(value)}</span></button>`;
}

function bindCopyButtons(root) {
  root.querySelectorAll('[data-copy]').forEach(button => {
    button.addEventListener('click', async event => {
      event.stopPropagation();
      await copyText(button.dataset.copy);
    });
  });
}

async function copyText(value) {
  try {
    if (navigator.clipboard && navigator.clipboard.writeText) {
      await navigator.clipboard.writeText(value);
      toast('Скопировано');
      return;
    }
  } catch (error) {
    /* ниже резервный способ */
  }
  const area = document.createElement('textarea');
  area.value = value;
  area.setAttribute('readonly', '');
  area.style.position = 'fixed';
  area.style.opacity = '0';
  document.body.appendChild(area);
  area.select();
  let copied = false;
  try {
    copied = document.execCommand('copy');
  } catch (error) {
    copied = false;
  }
  area.remove();
  toast(copied ? 'Скопировано' : 'Не удалось скопировать');
}

function clubMoment(epochMs) {
  const value = Number(epochMs);
  if (!Number.isFinite(value) || value <= 0) return '';
  const date = new Date(value);
  const pad = number => String(number).padStart(2, '0');
  return `${pad(date.getDate())}.${pad(date.getMonth() + 1)}.${date.getFullYear()} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

function clubDate(seconds, withTime) {
  const value = Number(seconds);
  if (!seconds || !Number.isFinite(value) || value <= 0) return '';
  const date = new Date(value * 1000);
  const pad = number => String(number).padStart(2, '0');
  const day = `${pad(date.getDate())}.${pad(date.getMonth() + 1)}.${date.getFullYear()}`;
  return withTime ? `${day} ${pad(date.getHours())}:${pad(date.getMinutes())}` : day;
}

const ADMIN_SORTS = [
  ['recent', 'Сначала новые'],
  ['online', 'Сначала онлайн'],
  ['name', 'По имени'],
  ['balance', 'По балансу'],
  ['games', 'По партиям'],
];

function renderAdminToolbar() {
  const host = document.querySelector('#adminFilters');
  if (!host) return;
  host.innerHTML = `
    <div class="admin-filters" role="group" aria-label="Фильтр игроков">
      ${ADMIN_FILTERS.map(([key, label]) => {
        const count = adminFilterCount(key);
        return `<button type="button" data-filter="${key}" class="${adminState.filter === key ? 'is-active' : ''}" aria-pressed="${adminState.filter === key}"${count ? '' : ' disabled'}><span>${label}</span><b>${count}</b></button>`;
      }).join('')}
    </div>
    <label class="admin-sort">Сортировка
      <select id="adminSort">
        ${ADMIN_SORTS.map(([key, label]) => `<option value="${key}"${adminState.sort === key ? ' selected' : ''}>${label}</option>`).join('')}
      </select>
    </label>
  `;
  host.querySelectorAll('[data-filter]').forEach(button => {
    button.addEventListener('click', () => {
      adminState.filter = button.dataset.filter;
      renderAdminPage();
    });
  });
  const sort = host.querySelector('#adminSort');
  if (sort) sort.addEventListener('change', () => {
    adminState.sort = sort.value;
    renderAdminPage();
  });
}

function renderAdminPage() {
  if (!adminPage) return;
  const list = document.querySelector('#adminList');
  const meta = document.querySelector('#adminMeta');
  if (!list) return;
  const online = adminState.players.filter(player => player.online).length;
  const players = adminFilteredPlayers();
  if (meta) meta.textContent = adminState.loading
    ? 'Обновляем список…'
    : `${players.length} из ${adminState.players.length} профилей · ${online} онлайн · баланс всего ${chips(adminState.players.reduce((sum, player) => sum + player.balance, 0))} фишек`;
  list.classList.toggle('is-loading', adminState.loading);
  renderAdminToolbar();
  list.innerHTML = players.length
    ? players.map(player => `
      <div class="admin-row${adminState.selected && adminState.selected.id === player.id ? ' is-current' : ''}">
        <button class="admin-row-open" data-player="${esc(player.id)}" aria-label="Профиль ${esc(player.name)}">
          ${avatarHTML(player)}
          <span class="admin-main">
            <strong>${esc(player.name)}${player.online ? '<i class="admin-online" aria-label="онлайн"></i>' : ''}</strong>
            ${copyButtonHTML(player.id, player.name)}
          </span>
          <span class="admin-row-side">
            <small>${adminKindLabel(player)}${player.hasPassword ? '' : ' · без пароля'}</small>
            <small>${esc(clubDate(player.createdAt) || 'дата не сохранена')}</small>
          </span>
        </button>
        <button class="admin-row-action" data-key="${esc(player.id)}" title="Сменить пароль" aria-label="Сменить пароль ${esc(player.name)}">✦</button>
        <button class="admin-row-action danger" data-remove="${esc(player.id)}" title="Удалить учётную запись" aria-label="Удалить ${esc(player.name)}">✕</button>
      </div>
    `).join('')
    : `<div class="admin-empty">${adminState.players.length ? 'Никого не нашли. <button type="button" class="link-btn" id="adminResetFilters">Сбросить фильтры</button>' : 'В клубе пока нет учётных записей'}</div>`;
  const resetFilters = document.querySelector('#adminResetFilters');
  if (resetFilters) resetFilters.addEventListener('click', () => {
    adminState.query = '';
    adminState.filter = 'all';
    adminState.sort = 'recent';
    const search = document.querySelector('#adminSearch');
    if (search) search.value = '';
    renderAdminPage();
  });
  bindCopyButtons(list);
  list.querySelectorAll('[data-player]').forEach(row => {
    row.addEventListener('click', () => openAdminPlayer(row.dataset.player));
  });
  list.querySelectorAll('[data-key]').forEach(button => {
    button.addEventListener('click', () => {
      const player = adminState.players.find(item => item.id === button.dataset.key);
      if (player) openPasswordReset(player);
    });
  });
  list.querySelectorAll('[data-remove]').forEach(button => {
    button.addEventListener('click', () => removeAdminPlayer(button.dataset.remove, button));
  });
  const editor = document.querySelector('#adminEditor');
  if (editor) renderAdminEditor(editor);
}

function openAdminPlayer(id) {
  const player = adminState.players.find(item => item.id === id);
  if (!player) return;
  adminState.selected = player;
  renderAdminPage();
}

async function removeAdminPlayer(id, button) {
  const player = adminState.players.find(item => item.id === id);
  if (!player || button.disabled) return;
  if (!button.dataset.armed) {
    button.dataset.armed = '1';
    button.classList.add('armed');
    button.title = 'Нажмите ещё раз, чтобы удалить';
    setTimeout(() => {
      if (!button.isConnected) return;
      delete button.dataset.armed;
      button.classList.remove('armed');
      button.title = 'Удалить учётную запись';
    }, 4000);
    return;
  }
  button.disabled = true;
  try {
    await adminApi({ op: 'delete', id });
    if (adminState.selected && adminState.selected.id === id) adminState.selected = null;
    toast(`Учётная запись «${player.name}» удалена`);
    loadAdminPlayers();
  } catch (error) {
    button.disabled = false;
    toast(error.message);
  }
}

function adminCreatedLabel(player) {
  return clubDate(player.createdAt, true) || 'дата не сохранена';
}

function adminKind(player) {
  return player.kind || (player.id.startsWith('temp:') ? 'temp' : player.id.startsWith('discord:') ? 'discord' : 'legacy');
}

function adminKindLabel(player) {
  const kind = adminKind(player);
  return kind === 'temp' ? 'Клубный вход' : kind === 'discord' ? 'Discord' : 'Старый профиль';
}

function generatePassword() {
  const alphabet = 'abcdefghijkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789';
  const size = globalThis.crypto && globalThis.crypto.getRandomValues ? globalThis.crypto.getRandomValues(new Uint8Array(14)) : Array.from({ length: 14 }, () => Math.floor(Math.random() * 256));
  return Array.from(size, byte => alphabet[byte % alphabet.length]).join('');
}

function openPasswordReset(player) {
  const needsName = adminKind(player) === 'legacy' || adminKind(player) === 'temp';
  openModal(`
    <div class="modal-label">ПАРОЛЬ</div>
    <h2>${esc(player.name)}</h2>
    <p class="modal-subtitle">${needsName ? 'Старый пароль перестанет действовать, открытые сессии закроются. Задайте ещё и настоящее имя: сейчас здесь заглушка.' : 'Старый пароль перестанет действовать, открытые сессии закроются.'}</p>
    ${needsName ? `<label>Имя<input id="pName" maxlength="20" value="${esc(player.name)}" autocomplete="off"></label>` : ''}
    <label>Новый пароль<input id="pPassword" type="password" minlength="4" maxlength="128" value="${esc(generatePassword())}" autocomplete="new-password"></label>
    <div class="modal-actions"><button class="secondary-btn" id="pCancel">Отмена</button><button class="primary-btn" id="pSave">Сменить пароль</button></div>
  `, root => {
    root.querySelector('#pCancel').addEventListener('click', closeModal);
    root.querySelector('#pSave').addEventListener('click', async () => {
      const button = root.querySelector('#pSave');
      if (button.disabled) return;
      const nameInput = root.querySelector('#pName');
      const password = root.querySelector('#pPassword').value;
      if (password.length < 4 || password.length > 128) return toast('Пароль от 4 до 128 символов');
      button.disabled = true;
      try {
        await adminApi({ op: 'update', id: player.id, ...(nameInput ? { name: nameInput.value.trim() } : {}), password });
        openModal(`<div class="modal-label">ПАРОЛЬ ОБНОВЛЁН</div><h2>${esc(nameInput ? nameInput.value.trim() || player.name : player.name)}</h2><p class="modal-subtitle">Передайте новый пароль игроку. Прежний больше не действует.</p><div class="wallet"><strong>Пароль: ${esc(password)}</strong></div><div class="modal-actions"><button class="primary-btn" id="pDone">Готово</button></div>`, done => done.querySelector('#pDone').addEventListener('click', closeModal));
        loadAdminPlayers();
      } catch (error) {
        button.disabled = false;
        toast(error.message);
      }
    });
  });
}

function renderAdminEditor(host) {
  const player = adminState.selected;
  if (!player) {
    host.innerHTML = '<div class="admin-empty">Выберите игрока слева, чтобы изменить имя, пароль, баланс и статистику.</div>';
    return;
  }
  const kind = adminKind(player);
  host.innerHTML = `
    <div class="modal-label">ПРОФИЛЬ ИГРОКА</div>
    <h2>${esc(player.name)}</h2>
    <p class="modal-subtitle">${adminKindLabel(player)}${player.room ? ` · стол «${esc(player.room)}»` : ' · вне стола'}${player.online ? ' · онлайн' : ' · офлайн'}</p>
    <div class="admin-facts">
      <span class="admin-facts-label">id</span>${copyButtonHTML(player.id, player.name)}
      <span class="admin-facts-label">создан</span><span class="admin-facts-value">${esc(adminCreatedLabel(player))}</span>
    </div>
    <div class="admin-form">
      <label>Имя<input id="aName" maxlength="20" value="${esc(player.name)}"></label>
      <label>Баланс, фишек<input id="aBalance" type="number" min="0" step="10" value="${player.balance}"></label>
      <label>Партии<input id="aGames" type="number" min="0" value="${player.games}"></label>
      <label>Победы<input id="aWins" type="number" min="0" value="${player.wins}"></label>
      <label>Поражения<input id="aLosses" type="number" min="0" value="${player.losses}"></label>
    </div>
    <p class="admin-note">${player.hasPassword ? 'Пароль хранится в виде хеша, поэтому его нельзя показать.' : 'Пароль не задан: вход открыт только по Discord.'}</p>
    <div class="admin-quick">
      <button class="secondary-btn" id="aPlus100">+100 фишек</button>
      <button class="secondary-btn" id="aPlus500">+500 фишек</button>
      <button class="secondary-btn" id="aMinus100">−100 фишек</button>
    </div>
    <div class="admin-actions">
      <button class="primary-btn" id="aSave">Сохранить</button>
      <button class="secondary-btn" id="aPassword">Сменить пароль</button>
      <button class="secondary-btn" id="aReset">Сбросить прогресс</button>
      <button class="secondary-btn danger" id="aDelete">Удалить</button>
    </div>
  `;
  let armed = false;
  const armButton = (button, label) => {
    if (!armed) {
      armed = true;
      button.textContent = label;
      setTimeout(() => { armed = false; button.textContent = label === 'Точно удалить?' ? 'Удалить' : 'Сбросить прогресс'; }, 4000);
      return false;
    }
    return true;
  };
  const reset = host.querySelector('#aReset');
  reset.addEventListener('click', async () => {
    if (!armButton(reset, 'Точно сбросить?')) return;
    reset.disabled = true;
    try {
      await adminApi({ op: 'reset', id: player.id });
      toast('Прогресс игрока сброшен');
      loadAdminPlayers();
    } catch (error) {
      reset.disabled = false;
      toast(error.message);
    }
  });
  const remove = host.querySelector('#aDelete');
  remove.addEventListener('click', async () => {
    if (!armButton(remove, 'Точно удалить?')) return;
    remove.disabled = true;
    try {
      await adminApi({ op: 'delete', id: player.id });
      adminState.selected = null;
      toast('Учётная запись удалена');
      loadAdminPlayers();
    } catch (error) {
      remove.disabled = false;
      toast(error.message);
    }
  });
  bindCopyButtons(host);
  [['#aPlus100', 100], ['#aPlus500', 500], ['#aMinus100', -100]].forEach(([selector, delta]) => {
    const button = host.querySelector(selector);
    if (!button) return;
    button.addEventListener('click', async () => {
      if (button.disabled) return;
      button.disabled = true;
      try {
        await adminApi({ op: 'balance', id: player.id, delta });
        toast(delta > 0 ? `Начислено ${delta} фишек` : `Списано ${Math.abs(delta)} фишек`);
        loadAdminPlayers();
      } catch (error) {
        button.disabled = false;
        toast(error.message);
      }
    });
  });
  host.querySelector('#aPassword').addEventListener('click', () => openPasswordReset(player));
  host.querySelector('#aSave').addEventListener('click', async () => {
    const name = host.querySelector('#aName').value.trim();
    if (!name) return toast('Введите имя');
    const button = host.querySelector('#aSave');
    if (button.disabled) return;
    button.disabled = true;
    const number = id => Number(host.querySelector(id).value);
    try {
      await adminApi({
        op: 'update',
        id: player.id,
        name,
        balance: number('#aBalance'),
        games: number('#aGames'),
        wins: number('#aWins'),
        losses: number('#aLosses'),
      });
      toast('Данные игрока обновлены');
      loadAdminPlayers();
    } catch (error) {
      button.disabled = false;
      toast(error.message);
    }
  });
}

function openTempAccountForm() {
  openModal(`
    <div class="modal-label">НОВЫЙ ДОСТУП</div>
    <h2>Учётная запись</h2>
    <p class="modal-subtitle">Передайте игроку имя и пароль. Пароль сохраняется только в виде хеша и показывается один раз.</p>
    <label>Имя игрока<input id="tName" maxlength="20" autocomplete="off"></label>
    <label>Пароль<input id="tPassword" type="text" minlength="4" maxlength="128" value="${esc(generatePassword())}" autocomplete="new-password"></label>
    <div class="modal-actions"><button class="secondary-btn" id="tGenerate">Другой пароль</button><button class="secondary-btn" id="tCancel">Отмена</button><button class="primary-btn" id="tCreate">Создать</button></div>
  `, root => {
    root.querySelector('#tName').focus();
    root.querySelector('#tGenerate').addEventListener('click', () => { root.querySelector('#tPassword').value = generatePassword(); });
    root.querySelector('#tCancel').addEventListener('click', closeModal);
    root.querySelector('#tCreate').addEventListener('click', async () => {
      const button = root.querySelector('#tCreate');
      if (button.disabled) return;
      const name = root.querySelector('#tName').value.trim();
      if (!name) return toast('Введите имя');
      button.disabled = true;
      try {
        const data = await adminApi({ op: 'create_temp', name, password: root.querySelector('#tPassword').value });
        openModal(`<div class="modal-label">ДОСТУП СОЗДАН</div><h2>${esc(data.account.name)}</h2><p class="modal-subtitle">Сохраните эти данные и передайте игроку.</p><div class="wallet"><strong>Имя: ${esc(data.account.name)}<br>Пароль: ${esc(data.account.password)}</strong></div><div class="modal-actions"><button class="primary-btn" id="tDone">Готово</button></div>`, done => done.querySelector('#tDone').addEventListener('click', closeModal));
        loadAdminPlayers();
      } catch (error) {
        button.disabled = false;
        toast(error.message);
      }
    });
  });
}

function bindAdminPage() {
  const search = document.querySelector('#adminSearch');
  if (search) search.addEventListener('input', () => { adminState.query = search.value; renderAdminPage(); });
  const create = document.querySelector('#adminCreate');
  if (create) create.addEventListener('click', openTempAccountForm);
  const refresh = document.querySelector('#adminRefresh');
  if (refresh) refresh.addEventListener('click', loadAdminPlayers);
  const gate = document.querySelector('#adminGate');
  if (gate && !adminKey()) {
    gate.innerHTML = adminGateHTML();
    bindAdminGate(gate, key => {
      const screen = document.querySelector('#adminLogin');
      const panel = document.querySelector('#adminPanel');
      if (screen) screen.hidden = true;
      if (panel) panel.hidden = false;
      loadAdminPlayers();
    });
  }
  const exit = document.querySelector('#adminExit');
  if (exit) exit.addEventListener('click', signOutAdmin);
}
if (adminPage) {
  bindAdminPage();
  bootstrapAdminPage();
}

/* navigation */
function openPlayerPreview(player) {
  const role = player.left ? 'Покинул стол' : player.finished ? 'Закончил игру' : player.role === 'attacker' ? 'Атакует' : player.role === 'defender' ? 'Защищается' : 'За столом';
  openModal(`
    <div class="profile-preview">
      ${avatarHTML(player, 'profile-avatar')}
      <h2>${esc(player.name)}</h2>
      <p class="modal-subtitle">${role} · ${player.connected ? 'онлайн' : 'не в сети'}</p>
      <div class="profile-facts">${Number(player.cards)} ${cardsWord(player.cards)} в руке</div>
      <p class="modal-subtitle" data-player-stats role="status">Загружаем статистику…</p>
    </div>
  `, async body => {
    const stats = body.querySelector('[data-player-stats]');
    try {
      const profile = await api('/api/player-profile', { clientId, roomId: state.game.room.id, playerId: player.id });
      stats.textContent = `Баланс: ${chips(profile.balance)} фишек. Партий: ${chips(profile.games)} · Побед: ${chips(profile.wins)} · Поражений: ${chips(profile.losses)}`;
    } catch {
      stats.textContent = 'Не удалось загрузить баланс и статистику';
    }
  });
}

if (!adminPage) {
  document.querySelector('#cabinetBack').addEventListener('click', () => setView('lobby'));
  new ResizeObserver(() => {
    fitHand();
    fitTable();
  }).observe(document.querySelector('#myHand'));
  document.querySelectorAll('.nav-item').forEach(item => {
  item.addEventListener('click', () => {
    const view = item.dataset.view;
    if (view === 'lobby') {
      setView('lobby');
    } else if (view === 'mygames') {
      restoreSavedRoom(readSeatRoom()).then(restored => {
        if (restored) return;
        openModal(`<div class="modal-label">МОИ ИГРЫ</div><h2>Активных игр нет</h2><p class="modal-subtitle">Создайте лобби или войдите по коду друга</p><button class="primary-btn full" id="mEmptyCreate">Создать лобби</button>`, root => {
          root.querySelector('#mEmptyCreate').addEventListener('click', openCreateModal);
        });
      });
    } else if (view === 'cabinet') {
      renderCabinet();
      setView('cabinet');
    } else if (view === 'rules') {
      openRulesModal();
    }
  });
  });
}

function setView(view) {
  state.view = view;
  const gameActive = view === 'game' && Boolean(state.game);
  document.querySelector('#lobbyView').style.display = gameActive || view === 'cabinet' ? 'none' : '';
  document.querySelector('#cabinetView').hidden = view !== 'cabinet';
  document.querySelector('#gameScreen').classList.toggle('active', gameActive);
  document.querySelectorAll('.nav-item').forEach(item => {
    const active = item.dataset.view === view || (view === 'game' && item.dataset.view === 'mygames');
    item.classList.toggle('active', active);
    if (active) item.setAttribute('aria-current', 'page');
    else item.removeAttribute('aria-current');
  });
  updateResumeButton();
  if (gameActive) {
    fitHand();
    fitTable();
  } else {
    cancelCardDrag();
  }
  if (view === 'lobby') renderLobby();
  if (view === 'cabinet') renderCabinet();
}

function updateResumeButton() {
  let button = document.querySelector('#resumeBtn');
  if (!state.game) {
    if (button) button.remove();
    return;
  }
  if (!button) {
    button = document.createElement('button');
    button.id = 'resumeBtn';
    button.className = 'ghost-btn resume-btn';
    document.querySelector('.console-actions').prepend(button);
  }
  button.textContent = `Вернуться в игру «${state.game.room.name}»`;
  button.onclick = () => setView('game');
  button.style.display = state.view === 'game' ? 'none' : '';
}

/* game */
function setRetryConnection(visible) {
  const button = document.querySelector('#retryConnection');
  if (button) button.hidden = !visible;
}
if (document.querySelector('#retryConnection')) document.querySelector('#retryConnection').addEventListener('click', () => {
  setRetryConnection(false);
  connect();
});

async function restoreLeftSeat() {
  if (!state.game || !state.game.room || state.game.room.status !== 'playing') return false;
  if (await rejoinRoom(state.game.room.id)) return true;
  forgetSeatRoom();
  return false;
}

async function restoreSavedRoom(roomId) {
  if (!roomId || !clientId || !sessionToken) return false;
  return rejoinRoom(roomId);
}

async function rejoinRoom(roomId) {
  try {
    const data = await api('/api/join', { clientId, roomId });
    if (data.account) state.account = data.account;
    if (!data.room || data.room.status !== 'playing') {
      forgetSeatRoom();
      return false;
    }
    const room = data.room.id;
    rememberSeatRoom(room);
    state.autoOpen = true;
    state.restoreJoin = {
      async takeMissedEvents(seq) {
        const from = seq || await roomSeq(room);
        if (!from) return;
        catchUpRoom(room, from).catch(() => {});
      }
    };
    return true;
  } catch (error) {
    if (/кодом|началась|мест|лобби/i.test(error.message)) forgetSeatRoom();
    return false;
  }
}

function rememberSeatRoom(roomId) {
  try {
    sessionStorage.setItem('durak.room', roomId);
  } catch (error) {}
}

function readSeatRoom() {
  try {
    return sessionStorage.getItem('durak.room');
  } catch (error) {
    return null;
  }
}

function forgetSeatRoom() {
  try {
    sessionStorage.removeItem('durak.room');
  } catch (error) {}
}

function roomSeq(roomId) {
  return new Promise(resolve => {
    let settled = false;
    const finish = value => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      resolve(value);
    };
    const timer = setTimeout(() => finish(null), 400);
    const current = state.game && state.game.room && state.game.room.id === roomId ? Number(state.game.seq) || 0 : 0;
    if (current) finish(current);
    else if (state.source && state.source.readyState === EventSource.OPEN) finish(0);
    else if (state.source) state.source.addEventListener('open', () => finish(0), { once: true });
    else finish(null);
  });
}

async function catchUpRoom(roomId, since) {
  const params = new URLSearchParams({ id: clientId, token: sessionToken, since: String(since), until: 'any', timeout: '1500' });
  const response = await fetch(`/api/watch?${params}`);
  if (!response.ok) return;
  const data = await response.json();
  if (!data.hit || !data.state || data.state.room.id !== roomId) return;
  if (state.game && data.state.seq < (Number(state.game.seq) || 0)) return;
  state.game = data.state;
  state.autoOpen = false;
  setView('game');
  renderGame();
  updateResumeButton();
}

function showAbandonedRoom(restored) {
  if (restored) toast('Вы вернулись за стол');
  else toast('Лобби больше не существует');
}

window.addEventListener('pagehide', () => {
  if (state.source) state.source.close();
});

function connect() {
  if (!clientId || !sessionToken) return;
  if (state.source) state.source.close();
  state.source = new EventSource(`/events?id=${encodeURIComponent(clientId)}&token=${encodeURIComponent(sessionToken)}`);
  state.source.onopen = () => {
    state.connected = true;
    document.querySelector('#retryConnection').hidden = true;
    document.querySelector('#connectionStatus').textContent = 'Столы обновляются в реальном времени';
    document.querySelector('#connectionStatus').dataset.status = 'online';
    renderLobby();
  };
  state.source.onmessage = async event => {
    let payload;
    try {
      payload = JSON.parse(event.data);
    } catch (error) {
      return;
    }
    if (payload.type === 'lobby') {
      state.lobby = payload;
      renderLobby();
    } else if (payload.type === 'account') {
      state.account = payload.account;
      renderProfile();
      if (state.view === 'lobby') renderLobby();
      if (state.view === 'cabinet') renderCabinet();
    } else if (payload.type === 'game') {
      const incomingSeq = Number(payload.state && payload.state.seq) || 0;
      const currentSeq = Number(state.game && state.game.seq) || 0;
      if (state.game && payload.state && payload.state.room && state.game.room && payload.state.room.id === state.game.room.id && incomingSeq < currentSeq) return;
      if (state.leavingRoom) return;
      state.game = payload.state;
      if (state.actionPending && incomingSeq > state.actionPendingSeq) {
        state.actionPending = false;
        state.actionPendingSeq = 0;
        document.querySelector('#gameScreen').setAttribute('aria-busy', 'false');
      }
      if (state.autoOpen) {
        state.autoOpen = false;
        setView('game');
      }
      renderGame();
      updateResumeButton();
      if (state.view === 'game' && state.restoreJoin) {
        const pending = state.restoreJoin;
        state.restoreJoin = null;
        pending.takeMissedEvents(incomingSeq);
      }    } else if (payload.type === 'noroom' && payload.reason === 'auth') {
      sessionStorage.removeItem('durak.client');
      sessionStorage.removeItem('durak.token');
      location.reload();
    } else if (payload.type === 'noroom') {
      if (state.leavingRoom) {
        state.game = null;
        state.restoreJoin = null;
        forgetSeatRoom();
        document.querySelector('#finishOverlay').classList.remove('open');
        setView('lobby');
        return;
      }
      if (state.game || state.restoreJoin) {
        const restored = await restoreLeftSeat();
        state.game = null;
        state.restoreJoin = null;
        document.querySelector('#finishOverlay').classList.remove('open');
        if (document.querySelector('#confirmLeave')) closeModal();
        if (!restored) setView('lobby');
        updateResumeButton();
        showAbandonedRoom(restored);
      }
    }
  };
  state.source.onerror = () => {
    if (state.source.readyState === EventSource.CLOSED) {
      state.connected = false;
      document.querySelector('#connectionStatus').textContent = 'Не удалось подключиться. Проверьте связь и повторите попытку.';
      document.querySelector('#connectionStatus').dataset.status = 'offline';
      document.querySelector('#retryConnection').hidden = false;
      renderLobby();
      return;
    }
    state.connected = false;
    document.querySelector('#connectionStatus').textContent = 'Связь потеряна. Переподключаемся…';
    document.querySelector('#connectionStatus').dataset.status = 'offline';
    document.querySelector('#retryConnection').hidden = true;
    renderLobby();
  };
}

function sortHand(hand, trump) {
  const suitOrder = suit => (suit === trump ? SUIT_ORDER.length : SUIT_ORDER.indexOf(suit));
  return [...hand].sort((a, b) => {
    if (a.suit !== b.suit) return suitOrder(a.suit) - suitOrder(b.suit);
    return RANK_ORDER.indexOf(a.rank) - RANK_ORDER.indexOf(b.rank);
  });
}

const PIP_LAYOUTS = {
  2: [[1, 1], [5, 3]],
  3: [[1, 1], [3, 2], [5, 3]],
  4: [[1, 1], [1, 3], [5, 1], [5, 3]],
  5: [[1, 1], [1, 3], [3, 2], [5, 1], [5, 3]],
  6: [[1, 1], [1, 3], [3, 1], [3, 3], [5, 1], [5, 3]],
  7: [[1, 1], [1, 3], [3, 1], [3, 2], [3, 3], [5, 1], [5, 3]],
  8: [[1, 1], [1, 3], [2, 1], [2, 3], [4, 1], [4, 3], [5, 1], [5, 3]],
  9: [[1, 1], [1, 3], [2, 1], [2, 3], [3, 2], [4, 1], [4, 3], [5, 1], [5, 3]],
  10: [[1, 1], [1, 3], [2, 1], [2, 3], [2, 2], [4, 2], [4, 1], [4, 3], [5, 1], [5, 3]]
};

function cardBodyHTML(card) {
  if (card.rank === 'Т') return `<span class="card-ace">${card.suit}</span>`;
  if (card.rank === 'В' || card.rank === 'Д' || card.rank === 'К') {
    return `<span class="card-court"><i>${esc(card.rank)}</i></span>`;
  }
  const pips = PIP_LAYOUTS[card.rank] || [[3, 2]];
  return `<span class="card-pips">${pips.map(([row, column]) => `<i style="grid-area:${row}/${column}">${card.suit}</i>`).join('')}</span>`;
}

function cardCornersHTML(card, showSuit = false) {
  const faceRank = showSuit || 'ВДКТ'.includes(card.rank);
  const corner = `<span class="card-corner">${esc(card.rank)}${faceRank ? `<i>${card.suit}</i>` : ''}</span>`;
  return `${corner}${cardBodyHTML(card)}<span class="card-corner bottom" aria-hidden="true">${esc(card.rank)}${faceRank ? `<i>${card.suit}</i>` : ''}</span>`;
}

function cardHTML(card, extraClass = '') {
  const red = card.suit === '♥' || card.suit === '♦';
  return `<div class="card-face ${red ? 'red' : ''} ${extraClass}" data-card-id="${esc(card.id)}">${cardCornersHTML(card)}</div>`;
}

function cardBackHTML(extraClass = '') {
  return `<span class="card-back ${extraClass}" aria-hidden="true"><i></i><b>♠</b></span>`;
}

function roleIcon(role) {
  if (role === 'attacker') return '<svg class="role-icon attacker-icon" viewBox="0 0 24 24" aria-hidden="true"><path d="M9 11l7-7 5-1-1 5-7 7Z"/><path d="m7 9 8 8m-4-4-6 6m-2-2 4 4"/></svg>';
  if (role === 'defender') return '<svg class="role-icon defender-icon" viewBox="0 0 24 24" aria-hidden="true"><path d="M15 3 22 6v5c0 5-3 8-7 10-4-2-7-5-7-10V6l7-3Z"/><path d="m3 6 2 2M2 11h3m-2 5 2-2"/></svg>';
  return '';
}

function flyCard(node, from) {
  if (!node || !from || !from.width) return;
  const to = node.getBoundingClientRect();
  if (!to.width) return;
  const dx = from.left + from.width / 2 - (to.left + to.width / 2);
  const dy = from.top + from.height / 2 - (to.top + to.height / 2);
  const scale = Math.max(.3, Math.min(1, from.width / to.width));
  node.animate([
    { translate: `${dx}px ${dy}px`, scale: `${scale}`, opacity: .35 },
    { translate: '0 0', scale: '1', opacity: 1 }
  ], { duration: 380, easing: 'cubic-bezier(.22,1,.36,1)' });
}

function animateCardFlight(ghost, from, to, delay, duration, waitingOpacity) {
  if (!from?.width || !to?.width) return;
  ghost.classList.add('card-ghost');
  ghost.setAttribute('aria-hidden', 'true');
  Object.assign(ghost.style, { left: `${from.left}px`, top: `${from.top}px`, width: `${from.width}px`, height: `${from.height}px` });
  document.body.appendChild(ghost);
  const dx = to.left - from.left + (to.width - from.width) / 2;
  const dy = to.top - from.top + (to.height - from.height) / 2;
  const animation = ghost.animate([
    { translate: '0 0', opacity: waitingOpacity, offset: 0 },
    { translate: '0 0', opacity: 1, offset: .04 },
    { translate: `${dx}px ${dy}px`, scale: .36, opacity: 1, offset: .86 },
    { translate: `${dx}px ${dy}px`, scale: .25, opacity: 0 }
  ], { duration, delay, fill: 'backwards', easing: 'cubic-bezier(.35,0,.3,1)' });
  animation.onfinish = animation.oncancel = () => ghost.remove();
}

function roundRefills(previous, game) {
  if (previous.deck <= game.deck) return [];
  const first = previous.players.findIndex(player => player.id === (previous.trickAttackerId || previous.attackerId));
  const start = Math.max(0, first);
  const order = previous.players.slice(start).concat(previous.players.slice(0, start));
  const defender = order.find(player => player.id === previous.defenderId);
  const taken = previous.takePending ? previous.table.reduce((total, row) => total + 1 + Number(Boolean(row.defense)), 0) : 0;
  return [...order.filter(player => player !== defender), ...(defender ? [defender] : [])].flatMap(player => {
    const current = game.players.find(item => item.id === player.id);
    const count = (current?.cards ?? player.cards) - player.cards - (player === defender ? taken : 0);
    return count > 0 ? [{ id: player.id, count }] : [];
  });
}

function animateDeal(game, source, handRoot, seatRoot) {
  if (!source.width) return;
  const targets = new Map([...seatRoot.querySelectorAll('.seat')].map(seat => [seat.dataset.seat, seat.querySelectorAll('.opponent-fan .card-back')]));
  targets.set(game.you?.id, handRoot.querySelectorAll('.hand-card'));
  const handBounds = handRoot.getBoundingClientRect();
  let delay = 0;
  for (let round = 0; round < 6; round += 1) {
    for (const player of game.players) {
      const card = targets.get(player.id)?.[round];
      if (!card) continue;
      const target = card.getBoundingClientRect();
      const visible = player.id !== game.you?.id || (target.right > handBounds.left && target.left < handBounds.right);
      if (target.width && visible) {
        const ghost = document.createElement('span');
        ghost.className = 'card-back';
        animateCardFlight(ghost, source, target, delay, 420, 0);
        card.animate([{ opacity: 0 }, { opacity: 1 }], { delay: delay + 360, duration: 60, fill: 'backwards' });
      }
      delay += 90;
    }
  }
}

function animateRoundEnd(previous, game, cards, source, handRoot, seatRoot) {
  const targets = new Map([...seatRoot.querySelectorAll('.seat')].map(seat => [seat.dataset.seat, seat.getBoundingClientRect()]));
  targets.set(game.you?.id, handRoot.querySelector('.hand-slot:last-child')?.getBoundingClientRect() || handRoot.getBoundingClientRect());
  const destination = previous.takePending ? targets.get(previous.defenderId) : { left: innerWidth / 2, top: -120, width: 1, height: 1 };
  const sweepDuration = 520;
  const sweepStep = 45;
  cards.forEach(({ node, box, cardWidth }, index) => {
    const ghost = node.cloneNode(true);
    ghost.style.setProperty('--card-w', `${cardWidth}px`);
    ghost.style.setProperty('--card-h', `${cardWidth * 1.39}px`);
    ghost.style.setProperty('--corner-size', `${cardWidth * .3}px`);
    ghost.style.setProperty('--pip-size', `${cardWidth * .21}px`);
    ghost.style.setProperty('--ace-size', `${cardWidth * .6}px`);
    ghost.style.setProperty('--court-size', `${cardWidth * .46}px`);
    ghost.style.setProperty('--def-x', `${cardWidth * .34}px`);
    ghost.style.setProperty('--def-y', `${cardWidth * .26}px`);
    animateCardFlight(ghost, box, destination, index * sweepStep, sweepDuration, 1);
  });
  let delay = sweepDuration + Math.max(0, cards.length - 1) * sweepStep;
  for (const { id, count } of roundRefills(previous, game)) {
    for (let index = 0; index < count; index += 1) {
      const ghost = document.createElement('span');
      ghost.className = 'card-back';
      animateCardFlight(ghost, source, targets.get(id), delay + index * 80, 400, 0);
    }
    delay += 400 + (count - 1) * 80 + 120;
  }
}

let cardDrag = null;

function canTransferCard(game, card) {
  return game.canTransfer && game.table[0]?.attack.rank === card.rank;
}

function tableDropMode(game, card) {
  if (game.you.role === 'defender') return canTransferCard(game, card) ? 'transfer' : null;
  if (!game.canAttack) return null;
  return !game.table.length || game.table.some(row => row.attack.rank === card.rank || row.defense?.rank === card.rank) ? 'attack' : null;
}

function dragMode(card, element) {
  const game = state.game;
  if (!game?.you?.turn || state.actionPending) return null;
  const pair = element?.closest('#tableCards .pair');
  const zone = element?.closest('#tableDropZone');
  if (game.you.role === 'defender' && pair) {
    const row = game.table.find(row => row.attack.id === pair.dataset.attack);
    if (game.canDefend && row && !row.defense && beats(row.attack, card, game.trump)) {
      return { mode: 'defend', row: row.attack.id, node: pair };
    }
  }
  const mode = tableDropMode(game, card);
  return mode && (zone || (mode === 'attack' && pair))
    ? { mode, node: document.querySelector('#tableDropZone') }
    : null;
}

function updateDragTarget(clientX, clientY) {
  const { card, zone, target } = cardDrag;
  const game = state.game;
  const mode = game?.you?.turn && !state.actionPending ? tableDropMode(game, card) : null;
  if (zone.dataset.mode !== (mode || '')) {
    zone.dataset.mode = mode || '';
    zone.textContent = mode === 'transfer' ? 'Перевести ход' : game.table.length ? 'Подкинуть карту' : 'Сыграть карту';
    zone.hidden = !mode;
    zone.setAttribute('aria-hidden', String(!mode));
  }
  const next = dragMode(card, document.elementFromPoint(clientX, clientY));
  if (target?.node !== next?.node) {
    target?.node.classList.remove('drop-target');
    next?.node.classList.add('drop-target');
  }
  cardDrag.target = next;
}

function moveCardDrag(event) {
  const drag = cardDrag;
  if (!drag || event.pointerId !== drag.pointerId) return;
  const dx = event.clientX - drag.startX;
  const dy = event.clientY - drag.startY;
  if (!drag.ghost) {
    if (Math.hypot(dx, dy) < 7) return;
    const ghost = document.createElement('div');
    ghost.className = `hand-card drag-ghost${drag.button.classList.contains('red') ? ' red' : ''}`;
    ghost.setAttribute('aria-hidden', 'true');
    ghost.innerHTML = drag.button.innerHTML;
    const styles = getComputedStyle(drag.button);
    const slot = drag.button.parentElement.getBoundingClientRect();
    ghost.style.width = styles.width;
    ghost.style.height = styles.height;
    ghost.style.setProperty('--card-w', styles.getPropertyValue('--card-w'));
    drag.scale = slot.width / parseFloat(styles.width);
    drag.grabX = (drag.startX - drag.box.left) * slot.width / drag.box.width;
    drag.grabY = (drag.startY - drag.box.top) * slot.height / drag.box.height;
    drag.ghost = ghost;
    document.body.appendChild(ghost);
    document.body.classList.add('is-dragging');
    drag.button.classList.add('drag-source');
  }
  if (event.cancelable) event.preventDefault();
  drag.ghost.style.transform = `translate3d(${event.clientX - drag.grabX}px, ${event.clientY - drag.grabY}px, 0) scale(${drag.scale})`;
  drag.clientX = event.clientX;
  drag.clientY = event.clientY;
  updateDragTarget(drag.clientX, drag.clientY);
}

function endCardDrag(event) {
  const drag = cardDrag;
  if (!drag || event.pointerId !== drag.pointerId) return;
  const target = drag.ghost && dragMode(drag.card, document.elementFromPoint(event.clientX, event.clientY));
  cancelCardDrag(event);
  if (!drag.ghost) return;
  state.suppressClick = true;
  setTimeout(() => { state.suppressClick = false; }, 0);
  if (target) act(target.mode, drag.card.id, target.row);
}

function cancelCardDrag(event) {
  const drag = cardDrag;
  if (!drag || (event?.pointerId != null && event.pointerId !== drag.pointerId)) return;
  cardDrag = null;
  window.removeEventListener('pointermove', moveCardDrag);
  window.removeEventListener('pointerup', endCardDrag);
  window.removeEventListener('pointercancel', cancelCardDrag);
  window.removeEventListener('blur', cancelCardDrag);
  drag.ghost?.remove();
  drag.button.classList.remove('drag-source');
  drag.target?.node.classList.remove('drop-target');
  drag.zone.hidden = true;
  drag.zone.dataset.mode = '';
  drag.zone.setAttribute('aria-hidden', 'true');
  document.body.classList.remove('is-dragging');
}

function startCardDrag(event, button) {
  if (event.button !== 0 || cardDrag || state.actionPending) return;
  const game = state.game;
  if (!game?.you?.turn) return;
  const card = game.hand.find(item => item.id === button.dataset.card);
  if (!card) return;
  const canDefend = game.canDefend && game.table.some(row => !row.defense && beats(row.attack, card, game.trump));
  if (!canDefend && !tableDropMode(game, card)) return;
  cardDrag = {
    card,
    button,
    zone: document.querySelector('#tableDropZone'),
    pointerId: event.pointerId,
    startX: event.clientX,
    startY: event.clientY,
    box: button.getBoundingClientRect()
  };
  window.addEventListener('pointermove', moveCardDrag, { passive: false });
  window.addEventListener('pointerup', endCardDrag);
  window.addEventListener('pointercancel', cancelCardDrag);
  window.addEventListener('blur', cancelCardDrag);
}

function renderGame() {
  const game = state.game;
  if (!game) return;
  const previous = state.renderedGame?.room.id === game.room.id ? state.renderedGame : null;
  state.renderedGame = game;
  const handNodes = [...document.querySelectorAll('#myHand .hand-card')];
  const previousHand = new Set(handNodes.map(button => button.dataset.card));
  const handBoxes = new Map(handNodes.map(button => [button.dataset.card, button.getBoundingClientRect()]));
  const seatBoxes = new Map([...document.querySelectorAll('.seat[data-seat]')].map(seat => [seat.dataset.seat, seat.getBoundingClientRect()]));
  const leavingNodes = [...document.querySelectorAll('#tableCards [data-card-id]')].map(card => {
    const box = card.getBoundingClientRect();
    return { node: card, box, cardWidth: box.width };
  });
  const previousTable = new Set(leavingNodes.map(item => item.node.dataset.cardId));
  const events = Array.isArray(game.log) ? game.log : [];
  const lastEvent = events.length ? events[events.length - 1] : null;
  const lastSeq = Number(lastEvent && lastEvent.seq) || 0;
  const fresh = Boolean(lastEvent) && (state.lastEventRoom !== game.room.id || lastSeq > Number(state.lastEventSeq || 0));
  if (lastEvent && fresh) {
    state.lastEventSeq = lastSeq;
    state.lastEventRoom = game.room.id;
  }
  const lastActor = lastEvent && lastEvent.actorId ? lastEvent.actorId : null;
  const myId = game.you && game.you.id;
  const motion = !matchMedia('(prefers-reduced-motion: reduce)').matches;
  const roundEnded = fresh && lastEvent.action === 'pass' && previous?.table.length && !game.table.length;
  document.querySelector('#tableName').textContent = game.room.name;
  const modeLabel = game.room.mode === 'transfer' ? 'Переводной' : 'Подкидной';
  document.querySelector('#tableMode').textContent = `${modeLabel}${game.room.bet ? ` · Ставка ${chips(game.room.bet)}` : ''}`;
  document.querySelector('#roomCode').textContent = `КОД ${game.room.id}`;

  const trump = document.querySelector('#trumpCard');
  trump.innerHTML = game.trumpCard ? cardHTML(game.trumpCard, 'trump-face') : '';
  trump.classList.toggle('empty', !game.trumpCard);
  const trumpSuit = document.querySelector('#trumpSuit');
  if (trumpSuit) {
    trumpSuit.textContent = game.trumpCard ? '' : game.trump || '';
    trumpSuit.classList.toggle('red', game.trump === '♥' || game.trump === '♦');
  }
  const deckBox = document.querySelector('#deckBox');
  const deckCard = deckBox.querySelector('.deck-back');
  const deckSource = roundEnded && motion ? deckCard.getBoundingClientRect() : null;
  const dealing = motion && fresh && lastEvent?.action === 'start' && previous && previous.room.status !== 'playing';
  deckBox.classList.toggle('empty', !game.deck);
  deckBox.style.setProperty('--depth', String(Math.max(2, Math.min(7, Math.round(game.deck / 4) + 1))));
  document.querySelector('#deckCount').textContent = game.deck;
  const table = document.querySelector('#tableCards');
  table.querySelectorAll('.pair').forEach(pair => pair.remove());
  table.insertAdjacentHTML('beforeend', game.table.map((row, index) => `<div class="pair" data-pair="${index}" data-attack="${esc(row.attack.id)}">${cardHTML(row.attack, `attack ${row.defense ? '' : 'target'}`).replace('<div ', `<div data-target-card="${esc(row.attack.id)}" `)}${row.defense ? cardHTML(row.defense, 'defense') : ''}</div>`).join(''));
  table.querySelectorAll('.target').forEach(target => {
    target.addEventListener('click', event => {
      event.stopPropagation();
      const cardId = document.querySelector('.hand-card:focus')?.dataset.card;
      if (cardId) playCard(cardId, target.dataset.targetCard);
    });
  });

  const turnPlayer = game.players.find(player => player.id === game.turnId);
  const defenderPlayer = game.players.find(player => player.role === 'defender');
  const tableMessage = game.room.status === 'waiting'
    ? 'Ожидаем игроков'
    : game.room.status === 'playing' && turnPlayer
      ? `${turnPlayer.name} ходит${defenderPlayer ? ` · ${defenderPlayer.name} защищается` : ''}`
      : game.message;
  document.querySelector('#tableMessage').textContent = tableMessage;

  const myIndex = game.players.findIndex(player => player.id === (game.you && game.you.id));
  const seats = [];
  const total = game.players.length;
  for (let step = 1; step < total; step += 1) {
    const index = myIndex === -1 ? step - 1 : (myIndex + step) % total;
    seats.push(game.players[index]);
  }
  if (myIndex === -1) seats.push(game.players[0]);
  const seatRoot = document.querySelector('#seats');
  const focusedSeat = document.activeElement?.dataset.seat;
  const previewSeat = modalTrigger?.dataset.seat;
  seatRoot.innerHTML = seats.slice(0, 5).map(player => {
    const role = player.role === 'attacker' ? 'attacker' : player.role === 'defender' ? 'defender' : '';
    const status = player.left ? 'Ушёл' : player.finished ? 'Закончил' : !player.connected ? 'Не в сети' : '';
    const roleLabel = role === 'attacker' ? 'Атакует' : role === 'defender' ? 'Защищается' : '';
    const turnLabel = player.id === game.turnId ? ' · Ходит' : '';
    const cardsLabel = game.room.status === 'waiting' ? '' : `, ${Number(player.cards)} ${cardsWord(player.cards)}`;
    const cardsHTML = game.room.status === 'waiting' ? '' : `<span class="seat-cards"><span class="opponent-fan" aria-hidden="true">${Array.from({ length: Number(player.cards) || 0 }, (_, cardIndex) => `<i style="--fan-index:${cardIndex};--fan-count:${Number(player.cards) || 0}">${cardBackHTML()}</i>`).join('')}</span><b aria-hidden="true">${Number(player.cards)}</b>${roleLabel ? `<span class="role-label ${role}">${roleIcon(role)}${roleLabel}${turnLabel}</span>` : ''}</span>`;
    return `<button class="seat ${player.id === game.turnId ? 'turn' : ''}" data-seat="${esc(player.id)}" type="button" aria-haspopup="dialog" aria-label="Профиль игрока ${esc(player.name)}${cardsLabel}${roleLabel ? `, ${roleLabel}` : status ? `, ${status}` : ''}">
      ${avatarHTML(player)}
      <span class="seat-main"><strong>${esc(player.name)}</strong>${status ? `<span class="seat-state">${status}</span>` : ''}</span>
      ${cardsHTML.replace(roleLabel ? `>${roleIcon(role)}${roleLabel}</span>` : '', roleLabel ? `>${roleIcon(role)}${roleLabel}${turnLabel}</span>` : '')}
    </button>`;
  }).join('');
  seatRoot.querySelectorAll('.seat').forEach(seat => {
    if (seat.dataset.seat === focusedSeat) seat.focus({ preventScroll: true });
    if (seat.dataset.seat === previewSeat) modalTrigger = seat;
    seat.addEventListener('click', () => {
      const player = state.game?.players.find(item => item.id === seat.dataset.seat);
      if (player) openPlayerPreview(player);
    });
  });


  const hand = sortHand(game.hand || [], game.trump);
  const ranksOnTable = new Set();
  game.table.forEach(row => {
    ranksOnTable.add(row.attack.rank);
    if (row.defense) ranksOnTable.add(row.defense.rank);
  });
  const attacking = Boolean(game.canAttack);
  const dimmed = card => {
    if (!(game.you && game.you.turn)) return false;
    if (attacking && game.table.length) return !ranksOnTable.has(card.rank);
    return false;
  };
  const handRoot = document.querySelector('#myHand');
  const handSignature = `${game.room.id}:` + hand.map(card => `${card.id}${dimmed(card) ? '·' : ''}`).join(' ') + `|${game.trump || ''}|${game.you && game.you.turn ? 'turn' : 'idle'}|${game.you && game.you.role || ''}|${game.takePending ? 'take' : ''}`;
  if (handRoot.dataset.signature !== handSignature) {
    cancelCardDrag();
    const focusedCard = handRoot.contains(document.activeElement) ? document.activeElement.dataset.card : null;
    handRoot.dataset.signature = handSignature;
    handRoot.innerHTML = hand.map(card => {
      const red = card.suit === '♥' || card.suit === '♦';
      return `<div class="hand-slot"><button class="hand-card ${red ? 'red' : ''} ${dimmed(card) ? 'dim' : ''}" data-card="${esc(card.id)}">${cardCornersHTML(card)}</button></div>`;
    }).join('');
    handRoot.querySelectorAll('.hand-card').forEach((button, index) => {
      const card = hand[index];
      const suits = { '♠': 'пик', '♣': 'треф', '♥': 'червей', '♦': 'бубен' };
      const ranks = { 'В': 'валет', 'Д': 'дама', 'К': 'король', 'Т': 'туз' };
      button.setAttribute('aria-label', `${ranks[card.rank] || card.rank} ${suits[card.suit] || card.suit}`);
      if (!previousHand.has(button.dataset.card) && motion && !roundEnded && !dealing) {
        button.animate([{ opacity: 0, transform: 'translateY(18px)' }, { opacity: 1, transform: 'translateY(0)' }], { duration: 380, delay: Math.min(index * 35, 210), easing: 'cubic-bezier(.22,1,.36,1)', fill: 'backwards' });
      }
      button.draggable = false;
      button.addEventListener('pointerdown', event => startCardDrag(event, button));
      button.addEventListener('click', () => {
        if (state.suppressClick) {
          state.suppressClick = false;
          return;
        }
        playCard(button.dataset.card);
      });
    });
    if (focusedCard) {
      const replacement = [...handRoot.querySelectorAll('.hand-card')].find(button => button.dataset.card === focusedCard);
      if (replacement) replacement.focus();
    }
  }

  fitHand();
  fitTable();
  if (motion) {
    const actorBox = lastActor ? seatBoxes.get(lastActor) : null;
    table.querySelectorAll('[data-card-id]').forEach(card => {
      if (previousTable.has(card.dataset.cardId)) return;
      const source = lastActor === myId ? handBoxes.get(card.dataset.cardId) : actorBox;
      if (source) flyCard(card, source);
      else card.animate([{ opacity: 0, transform: 'translateY(12px) rotate(-4deg)' }, { opacity: 1, transform: 'none' }], { duration: 300, easing: 'cubic-bezier(.22,1,.36,1)' });
    });
    if (dealing) animateDeal(game, deckCard.getBoundingClientRect(), handRoot, seatRoot);
    if (roundEnded) animateRoundEnd(previous, game, leavingNodes, deckSource, handRoot, seatRoot);
  }
  renderActions(game);
  if (cardDrag?.ghost) updateDragTarget(cardDrag.clientX, cardDrag.clientY);
  renderFinish(game);
}

function fitTable() {
  const row = document.querySelector('#tableCards');
  const message = document.querySelector('#tableMessage');
  if (!row || !row.parentElement || !message) return;
  const table = row.parentElement;
  const tableH = table.clientHeight;
  if (!tableH) return;
  const share = (parseFloat(getComputedStyle(row).getPropertyValue('--row-max')) || 88) / 100;
  const scale = Math.min(1, share * table.clientWidth / Math.max(1, row.offsetWidth));
  const zoom = scale < .999 ? scale.toFixed(3) : '';
  if (row.style.zoom !== zoom) row.style.zoom = zoom;
  const rowScale = Number(zoom) || 1;
  const tableTop = table.getBoundingClientRect().top + table.clientTop;
  const hand = table.querySelector('.my-hand');
  const handTop = hand ? hand.getBoundingClientRect().top - tableTop : tableH;
  const msgH = message.offsetHeight || 40;
  const gap = 12;
  const messageBottom = Math.max(msgH + 8, tableH - handTop + gap);
  const messageValue = `${Math.round(messageBottom)}px`;
  if (message.style.getPropertyValue('--message-bottom') !== messageValue) message.style.setProperty('--message-bottom', messageValue);
  const limit = tableH - messageBottom - msgH - gap;
  const current = row.style.getPropertyValue('--cards-shift');
  if (current) row.style.setProperty('--cards-shift', '');
  const base = row.getBoundingClientRect().bottom - tableTop;
  const shift = Math.max(0, Math.round((base - limit) / rowScale));
  const shiftValue = shift ? `${shift}px` : '';
  row.style.setProperty('--cards-shift', shiftValue);
}

function fitHand() {
  const hand = document.querySelector('#myHand');
  if (!hand) return;
  const cards = hand.querySelectorAll('.hand-card');
  const styles = getComputedStyle(hand);
  const padding = (parseFloat(styles.paddingLeft) || 0) + (parseFloat(styles.paddingRight) || 0);
  const box = hand.parentElement ? hand.parentElement.clientWidth : hand.clientWidth;
  const available = Math.max(0, box - padding);
  const cardWidth = cards.length ? parseFloat(getComputedStyle(cards[0]).width) || cards[0].offsetWidth : 0;
  const apply = (property, value) => {
    if (hand.style.getPropertyValue(property) !== value) hand.style.setProperty(property, value);
  };
  if (cards.length < 2 || !cardWidth || !available) {
    apply('--overlap', '');
    apply('--hand-zoom', '');
    return;
  }
  const count = cards.length;
  const minStep = Math.max(24, Math.min(44, cardWidth * .33));
  const scrolls = styles.overflowX === 'auto' || styles.overflowX === 'scroll';
  const needed = (count * cardWidth - available) / (count - 1);
  const overlap = Math.max(0, Math.min(cardWidth - minStep, Math.round(Math.max(scrolls ? 0 : needed, cardWidth * (scrolls ? .24 : .1)))));
  const total = cardWidth + (count - 1) * (cardWidth - overlap);
  const zoom = !scrolls && total > available + 8 ? Math.max(.7, available / total) : 1;
  apply('--overlap', `${overlap}px`);
  apply('--hand-zoom', zoom < .999 ? zoom.toFixed(3) : '');
}

window.addEventListener('resize', () => {
  fitHand();
  fitTable();
});

function renderActions(game) {
  const actions = document.querySelector('#gameActions');
  const buttons = [];
  if (game.room.status === 'playing' && !state.actionPending) {
    if (game.canTake) buttons.push({ label: game.takePending ? 'Забрать карты' : 'Взять карты', action: 'take', style: game.takePending ? 'primary-btn' : 'secondary-btn' });
    if (game.canPass) buttons.push({ label: game.takePending ? 'Хватит, забирайте' : 'Бито', action: 'pass', style: 'primary-btn' });

  } else {
    if (game.canStart) buttons.push({ label: game.room.status === 'finished' ? 'Играть снова' : 'Начать игру', action: 'start', style: 'primary-btn' });
    else if (game.room.status === 'waiting') {
      const isHost = Boolean(game.you && game.you.id === game.room.hostId);
      const connected = game.players.filter(player => player.connected).length;
      buttons.push({ label: isHost ? `Ждём игроков (${connected}/6)` : 'Ждём, пока создатель начнёт игру', action: null, style: 'secondary-btn', disabled: true });
    }

  }
  const signature = JSON.stringify(buttons);
  if (actions.dataset.signature === signature) return;
  actions.dataset.signature = signature;
  actions.innerHTML = buttons.map(button => `<button class="${button.style}" ${button.disabled ? 'disabled' : ''} data-action="${button.action || ''}">${esc(button.label)}</button>`).join('');
  actions.querySelectorAll('button[data-action]').forEach(element => {
    const action = element.dataset.action;
    if (!action) return;
    element.addEventListener('click', () => {
      if (action === 'copy') return copyInvite();
      if (action === 'transfer') return toast('Перетащите карту нужного достоинства на стол');
      act(action);
    });
  });
}

function selectTransferCard() {
  const game = state.game;
  if (!game || !game.canTransfer || !game.table.length) return;
  const rank = game.table[0].attack.rank;
  const cards = game.hand.filter(card => card.rank === rank);
  openModal(`<h2>Перевести ход</h2><p>Выберите карту для перевода.</p><div class="modal-actions">${cards.map(card => `<button class="secondary-btn" data-transfer-card="${esc(card.id)}">${esc(card.rank + card.suit)}</button>`).join('')}</div>`, () => {
    document.querySelectorAll('[data-transfer-card]').forEach(button => {
      button.addEventListener('click', () => {
        const cardId = button.dataset.transferCard;
        closeModal();
        act('transfer', cardId);
      });
    });
  });
}

function beats(attack, defense, trump) {
  if (attack.suit === defense.suit) return RANK_ORDER.indexOf(defense.rank) > RANK_ORDER.indexOf(attack.rank);
  return defense.suit === trump && attack.suit !== trump;
}

function cardsWord(count) {
  const mod10 = count % 10;
  const mod100 = count % 100;
  if (mod10 === 1 && mod100 !== 11) return 'карта';
  if (mod10 >= 2 && mod10 <= 4 && (mod100 < 12 || mod100 > 14)) return 'карты';
  return 'карт';
}

async function playCard(cardId, targetCardId = null) {
  const game = state.game;
  if (!game || !game.you) return;
  const card = (game.hand || []).find(item => item.id === cardId);
  if (!card) return;
  if (game.you.role !== 'defender' && !game.you.finished) {
    if (!game.canAttack) {
      const defender = game.players.find(player => player.role === 'defender');
      const openRows = game.table.filter(row => !row.defense).length;
      if (game.table.length >= 6) return toast('На столе уже шесть пар — больше не подкинуть');
      if (defender && defender.cards <= openRows) {
        return toast(`Сопернику нечем отбиваться: у него ${defender.cards} ${cardsWord(defender.cards)} — больше не подкинуть`);
      }
      return toast('Сейчас подкидывать нечем');
    }
    if (game.table.length && !game.table.some(row => row.attack.rank === card.rank || (row.defense && row.defense.rank === card.rank))) {
      return toast(`Можно подкидывать только ${[...new Set(game.table.flatMap(row => [row.attack.rank, row.defense ? row.defense.rank : row.attack.rank]))].join(', ')}`);
    }
    return act('attack', cardId);
  }
  if (game.you.role === 'defender') {
    if (!targetCardId && canTransferCard(game, card)) {
      return act('transfer', cardId);
    }
    if (game.takePending) return toast('Вы уже решили взять карты');
    if (!game.canDefend) return toast('Сейчас отбивать нечего');
    const row = targetCardId
      ? game.table.find(item => !item.defense && item.attack.id === targetCardId)
      : game.table.find(item => !item.defense && beats(item.attack, card, game.trump)) || game.table.find(item => !item.defense);
    if (!row) return toast('Сейчас отбивать нечего');
    if (!beats(row.attack, card, game.trump)) {
      return toast(`Карта ${card.rank}${card.suit} не бьёт ${row.attack.rank}${row.attack.suit}`);
    }
    return act('defend', cardId, row.attack.id);
  }
  toast('Сейчас ходит другой игрок');
}

async function act(action, cardId, targetCardId = null) {
  if (state.actionPending) return;
  if (!state.connected) return toast('Нет связи с сервером. Переподключаемся…');
  state.actionPending = true;
  state.actionPendingSeq = Number(state.game?.seq) || 0;
  renderActions(state.game);
  document.querySelector('#gameScreen').setAttribute('aria-busy', 'true');
  try {
    await api('/api/action', { clientId, action, cardId, targetCardId });
  } catch (error) {
    state.actionPending = false;
    state.actionPendingSeq = 0;
    renderActions(state.game);
    document.querySelector('#gameScreen').setAttribute('aria-busy', 'false');
    toast(error.message);
  }
}

function renderFinish(game) {
  const overlay = document.querySelector('#finishOverlay');
  if (game.room.status !== 'finished' || !game.finished) {
    overlay.classList.remove('open');
    overlay.innerHTML = '';
    return;
  }
  overlay.classList.add('open');
  const payout = Number(game.payout) || 0;
  const bet = Number(game.room.bet) || 0;
  overlay.innerHTML = `
    <div class="finish-card">
      <h2>Игра окончена</h2>
      <p>${game.finished.cancelled ? 'Партия отменена, ставки возвращены' : game.finished.loserId ? `${esc(game.finished.loser)} проиграл` : 'Ничья'}</p>
      ${bet ? `<div class="finish-payout ${payout < 0 ? 'down' : 'up'}">${payout > 0 ? '+' : ''}${chips(payout)}<small>фишек</small></div>` : ''}
      <div class="modal-actions">
        ${game.canStart ? '<button class="primary-btn" id="finishRestart">Играть снова</button>' : ''}
        <button class="secondary-btn" id="finishLeave">В лобби</button>
      </div>
    </div>
  `;
  const restart = overlay.querySelector('#finishRestart');
  if (restart) restart.addEventListener('click', () => act('start'));
  overlay.querySelector('#finishLeave').addEventListener('click', leaveRoom);
}

function copyInvite() {
  const game = state.game;
  if (!game) return;
  const link = `${location.origin}/?code=${game.room.id}`;
  const write = navigator.clipboard ? navigator.clipboard.writeText(link) : Promise.reject(new Error('no clipboard'));
  write.then(() => toast('Ссылка с кодом скопирована')).catch(() => toast(`Код приглашения: ${game.room.id}`));
}

if (document.querySelector('#roomCode')) document.querySelector('#roomCode').addEventListener('click', copyInvite);

async function leaveRoom() {
  state.leavingRoom = true;
  try {
    await api('/api/leave', { clientId });
  } catch (error) {
    toast(error.message);
    state.leavingRoom = false;
    return false;
  }
  state.game = null;
  forgetSeatRoom();
  document.querySelector('#finishOverlay').classList.remove('open');
  setView('lobby');
  setTimeout(() => { state.leavingRoom = false; }, 0);
  return true;
}
if (document.querySelector('#leaveRoom')) document.querySelector('#leaveRoom').addEventListener('click', () => {
  if (!state.game || state.game.room.status !== 'playing') return leaveRoom();
  openModal('<div class="modal-label">ПОКИНУТЬ СТОЛ</div><h2>Уже уходите?</h2><p class="modal-subtitle">Если вы ещё играете, выход завершит партию и будет засчитан как поражение.</p><div class="modal-actions"><button class="secondary-btn" id="stayAtTable">Остаться</button><button class="primary-btn" id="confirmLeave">Покинуть стол</button></div>', root => {
    root.querySelector('#stayAtTable').addEventListener('click', closeModal);
    root.querySelector('#confirmLeave').addEventListener('click', async () => {
      if (await leaveRoom()) closeModal();
    });
  });
});

/* ambient backdrop: random cards and chips that spawn offscreen and drift behind the glass */
const AMBIENT_LAYERS = {
  far: { count: 4, width: [110, 200], dim: .35, duration: [190, 270] },
  mid: { count: 4, width: [160, 280], dim: .46, duration: [120, 180] },
  near: { count: 3, width: [220, 400], dim: .62, duration: [78, 115] }
};

function randomBetween(min, max) {
  return min + Math.random() * (max - min);
}

function pick(items) {
  return items[Math.floor(Math.random() * items.length)];
}

function ambientObjectHTML(width) {
  const suit = pick(SUIT_ORDER);
  const roll = Math.random();
  if (roll < .3) {
    return `<span class="amb-chip ${pick(['amb-chip--red', 'amb-chip--green', ''])}" style="--w:${Math.round(width * .56)}px"></span>`;
  }
  if (roll < .58) return `<span class="amb-card amb-card--back"><i>${suit}</i></span>`;
  const rank = pick(RANK_ORDER.slice(6));
  const red = suit === '♥' || suit === '♦';
  return `<span class="card-face amb-card${red ? ' red' : ''}">${cardCornersHTML({ rank, suit })}</span>`;
}

function ambientPropHTML(config, initial) {
  const cap = Math.max(72, Math.min(innerWidth * .3, config.width[1]));
  const width = Math.round(randomBetween(Math.min(config.width[0], cap), Math.max(config.width[0], cap)));
  const duration = randomBetween(config.duration[0], config.duration[1]);
  const style = [
    `--w:${width}px`,
    `--y:${randomBetween(2, 86).toFixed(1)}%`,
    `--dy:${randomBetween(-20, 14).toFixed(1)}vh`,
    `--dur:${duration.toFixed(1)}s`,
    `--delay:${(initial ? -randomBetween(0, duration) : 0).toFixed(1)}s`,
    `--dim:${Math.min(1, config.dim * randomBetween(.9, 1.1)).toFixed(2)}`,
    `--rx:${randomBetween(2, 82).toFixed(1)}%`,
    `--ry:${randomBetween(2, 86).toFixed(1)}%`,
    `--tilt:${randomBetween(-22, 22).toFixed(1)}deg`,
    `--turn:${randomBetween(-22, 22).toFixed(1)}deg`,
    `--sway:${randomBetween(14, 30).toFixed(1)}s`,
    `--sway-delay:${(-randomBetween(0, 20)).toFixed(1)}s`,
    `--wobble-x:${randomBetween(10, 34).toFixed(0)}px`,
    `--wobble-y:${randomBetween(8, 26).toFixed(0)}px`
  ].join(';');
  return `<span class="amb-prop" style="${style}">${ambientObjectHTML(width)}</span>`;
}

function startAmbient() {
  Object.entries(AMBIENT_LAYERS).forEach(([name, config]) => {
    const layer = document.querySelector(`.ambient-${name}`);
    if (!layer) return;
    const spawn = initial => {
      layer.insertAdjacentHTML('beforeend', ambientPropHTML(config, initial));
      const prop = layer.lastElementChild;
      const recycle = event => {
        if (event.target !== prop || event.animationName !== 'ambDrift') return;
        prop.removeEventListener('animationiteration', recycle);
        prop.remove();
        spawn(false);
      };
      prop.addEventListener('animationiteration', recycle);
    };
    for (let index = 0; index < config.count; index += 1) spawn(true);
  });
}

/* start */
async function bootstrap() {
  startAmbient();
  renderMasthead();
  setInterval(renderMasthead, 30000);
  renderLobby();
  if (await completeDiscordLogin()) return;
  await prepareTemporaryLogin();
  if (!clientId || !sessionToken) return;
  try {
    const profile = await api('/api/profile', { clientId });
    if (profile.account) state.account = profile.account;
  } catch (error) {
    if (/токен/i.test(error.message)) return signOut();
  }
  document.querySelector('#loginScreen').hidden = true;
  document.querySelector('#appShell').hidden = false;
  connect();
  const inviteCode = inviteCodeFromUrl(new URL(location.href));
  const savedRoom = readSeatRoom();
  if (inviteCode) {
    const joined = await joinRoom(inviteCode);
    dropInviteQuery();
    if (!joined) openJoinModal(inviteCode.toUpperCase());
  } else if (savedRoom && (await restoreSavedRoom(savedRoom))) {
    document.querySelector('#appShell').hidden = false;
  }
}
if (adminPage) startAmbient();
else bootstrap();