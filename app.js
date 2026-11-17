function newId() {
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') return crypto.randomUUID();
  const bytes = new Uint8Array(16);
  if (typeof crypto !== 'undefined' && typeof crypto.getRandomValues === 'function') crypto.getRandomValues(bytes);
  else for (let index = 0; index < bytes.length; index += 1) bytes[index] = Math.floor(Math.random() * 256);
  bytes[6] = (bytes[6] & 0x0f) | 0x40;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  const hex = Array.from(bytes, byte => byte.toString(16).padStart(2, '0')).join('');
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

function readClientId() {
  try {
    const saved = sessionStorage.getItem('durak.client');
    if (saved) return saved;
  } catch (error) {
    /* приватный режим может запрещать sessionStorage */
  }
  return newId();
}

function readSessionToken() {
  let saved = null;
  try {
    saved = sessionStorage.getItem('durak.token');
  } catch (error) {
    saved = null;
  }
  if (/^[a-f0-9]{64}$/.test(saved || '')) return saved;
  const bytes = new Uint8Array(32);
  crypto.getRandomValues(bytes);
  const token = Array.from(bytes, byte => byte.toString(16).padStart(2, '0')).join('');
  try {
    sessionStorage.setItem('durak.token', token);
  } catch (error) {
    return token;
  }
  return token;
}

const clientId = readClientId();
const sessionToken = readSessionToken();
try {
  sessionStorage.setItem('durak.client', clientId);
} catch (error) {
  /* идентификатор будет жить только до перезагрузки */
}

const RANK_ORDER = ['2', '3', '4', '5', '6', '7', '8', '9', '10', 'В', 'Д', 'К', 'Т'];
const SUIT_ORDER = ['♠', '♥', '♦', '♣'];
const AVATAR_TINTS = ['#dceadf', '#f0dfe2', '#f2e7ca', '#dbe7f1', '#e6e0f1', '#f6e2d6'];
const BET_OPTIONS = [0, 50, 100, 250, 500];
const EMPTY_ACCOUNT = { balance: 0, games: 0, wins: 0, losses: 0 };

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

function avatarHTML(name, className = 'mini-avatar') {
  const tint = name === state.profile.name ? AVATAR_TINTS[state.profile.tint % AVATAR_TINTS.length] : tintOf(name);
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

/* theme */
function applyTheme(theme) {
  document.documentElement.dataset.theme = theme;
  try {
    localStorage.setItem('durak.theme', theme);
  } catch (error) {
    /* тема не сохранится между визитами */
  }
  const button = document.querySelector('#themeToggle');
  if (button) button.textContent = theme === 'dark' ? 'Светлая тема' : 'Тёмная тема';
  const meta = document.querySelector('meta[name="theme-color"]');
  if (meta) meta.content = theme === 'dark' ? '#080d0a' : '#efe9dc';
}
function storedTheme() {
  try {
    return localStorage.getItem('durak.theme') || 'dark';
  } catch (error) {
    return 'dark';
  }
}
applyTheme(storedTheme());
document.querySelector('#themeToggle').addEventListener('click', () => {
  applyTheme(document.documentElement.dataset.theme === 'dark' ? 'light' : 'dark');
});

/* modal */
const backdrop = document.querySelector('#modalBackdrop');
let modalTrigger = null;
function openModal(html, onMount) {
  if (!backdrop.classList.contains('open')) modalTrigger = document.activeElement;
  const body = document.querySelector('#modalBody');
  body.innerHTML = html;
  backdrop.inert = false;
  backdrop.setAttribute('aria-hidden', 'false');
  backdrop.classList.add('open');
  document.querySelector('#appShell').inert = true;
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
  backdrop.classList.remove('open');
  document.querySelector('#appShell').inert = false;
  document.body.classList.remove('modal-open');
  if (modalTrigger && modalTrigger.isConnected) modalTrigger.focus();
  backdrop.inert = true;
  backdrop.setAttribute('aria-hidden', 'true');
}
document.querySelector('#modalClose').addEventListener('click', closeModal);
let backdropPressStarted = false;
backdrop.addEventListener('pointerdown', event => {
  backdropPressStarted = event.target === backdrop;
});
backdrop.addEventListener('pointerup', event => {
  if (backdropPressStarted && event.target === backdrop) closeModal();
  backdropPressStarted = false;
});
backdrop.addEventListener('pointercancel', () => {
  backdropPressStarted = false;
});
backdrop.addEventListener('click', event => {
  if (event.target === backdrop) event.preventDefault();
});
document.addEventListener('keydown', event => {
  if (!backdrop.classList.contains('open')) return;
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
document.querySelector('#profileBtn').addEventListener('click', openProfileModal);
function renderProfile() {
  document.querySelector('#profileName').textContent = state.profile.name;
  const avatar = document.querySelector('#profileAvatar');
  avatar.textContent = initials(state.profile.name);
  avatar.style.background = AVATAR_TINTS[state.profile.tint % AVATAR_TINTS.length];
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
    <label>Имя игрока<input id="mName" maxlength="20" value="${esc(state.profile.name)}"></label>
    <label>Цвет аватара
      <div class="swatches" id="mSwatches">
        ${AVATAR_TINTS.map((tint, index) => `<button class="swatch ${index === state.profile.tint ? 'active' : ''}" data-tint="${index}" style="background:${tint}" aria-label="Цвет ${index + 1}"></button>`).join('')}
      </div>
    </label>
    <div class="modal-actions">
      <button class="ghost-btn admin-link" id="mAdmin">Панель клуба</button>
      <button class="secondary-btn" id="mCancel">Отмена</button>
      <button class="primary-btn" id="mSave">Сохранить</button>
    </div>
  `, root => {
    let tint = state.profile.tint;
    root.querySelector('#mSwatches').addEventListener('click', event => {
      const button = event.target.closest('.swatch');
      if (!button) return;
      tint = Number(button.dataset.tint);
      root.querySelectorAll('.swatch').forEach(item => item.classList.toggle('active', item === button));
    });
    root.querySelector('#mAdmin').addEventListener('click', openAdminPanel);
    root.querySelector('#mCancel').addEventListener('click', closeModal);
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
          <div class="stacked">${room.players.slice(0, 4).map(player => avatarHTML(player.name)).join('')}${extra > 0 ? `<span class="mini-avatar more" title="Ещё ${extra} за столом">+${extra}</span>` : ''}</div>
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

document.querySelector('#lobbyFilter').addEventListener('click', event => {
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
    const data = await api('/api/join', { clientId, roomId: String(code).toUpperCase(), playerName: state.profile.name });
    if (data.account) state.account = data.account;
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
  state.autoOpen = true;
  if (state.game) setView('game');
}

/* create / join / history / rules / settings modals */
function openCreateModal() {
  const defaultName = `${state.profile.name} — партия`;
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
          playerName: state.profile.name,
        });
        if (data.account) state.account = data.account;
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

document.querySelector('#createBtn').addEventListener('click', openCreateModal);
document.querySelector('#heroCreateBtn').addEventListener('click', openCreateModal);
document.querySelector('#joinRoomBtn').addEventListener('click', () => openJoinModal());

document.querySelector('#historyBtn').addEventListener('click', () => {
  const items = state.lobby.history;
  openModal(`
    <div class="modal-label">АРХИВ</div>
    <h2>История игр</h2>
    <p class="modal-subtitle">Последние завершённые партии на сервере</p>
    ${items.length ? `<ul class="history-list">${items.map(item => `<li><span>${esc(item.room)}<br><small>${item.mode === 'transfer' ? 'Переводной' : 'Подкидной'}${item.bet ? ` · ставка ${chips(item.bet)}` : ''} · ${new Date(item.at).toLocaleString('ru-RU')}</small></span><strong>${esc(item.loser)}</strong></li>`).join('')}</ul>` : '<div class="empty-state">Пока ни одна партия не завершилась</div>'}
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

function openSettingsModal() {
  openModal(`
    <div class="modal-label">НАСТРОЙКИ</div>
    <h2>Настройки</h2>
    <p class="modal-subtitle">Тема клуба. Имя и аватар меняются в карточке профиля.</p>
    <label>Тема
      <select id="mTheme">
        <option value="light" ${document.documentElement.dataset.theme === 'light' ? 'selected' : ''}>Светлая</option>
        <option value="dark" ${document.documentElement.dataset.theme === 'dark' ? 'selected' : ''}>Тёмная</option>
      </select>
    </label>
    <button class="primary-btn full" id="mSettingsDone">Готово</button>
  `, root => {
    root.querySelector('#mTheme').addEventListener('change', event => applyTheme(event.target.value));
    root.querySelector('#mSettingsDone').addEventListener('click', closeModal);
  });
}

/* admin */
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

function syncAdminNav() {
  const item = document.querySelector('.nav-item[data-view="admin"]');
  if (item) item.hidden = !adminKey();
}

function adminApi(payload) {
  return api('/api/admin', { key: adminKey(), ...payload });
}

function openAdminPanel() {
  if (!adminKey()) return openAdminGate();
  adminApi({ op: 'list' }).then(data => {
    const players = data.players || [];
    const online = players.filter(player => player.online).length;
    openModal(`
      <div class="modal-label">ПАНЕЛЬ КЛУБА</div>
      <h2>Игроки</h2>
      <p class="modal-subtitle">${players.length} записей · ${online} онлайн · балансы и статистика правятся вручную</p>
      <label>Поиск<input id="aSearch" placeholder="Имя или ID" autocomplete="off"></label>
      <div class="admin-list" id="aList"></div>
    `, root => {
      const search = root.querySelector('#aSearch');
      const list = root.querySelector('#aList');
      const draw = () => {
        const term = search.value.trim().toLowerCase();
        const visible = players.filter(player => !term || player.name.toLowerCase().includes(term) || player.id.toLowerCase().includes(term));
        list.innerHTML = visible.length
          ? visible.map(player => `
            <button class="admin-row" data-player="${esc(player.id)}" aria-label="Профиль ${esc(player.name)}">
              ${avatarHTML(player.name)}
              <span class="admin-main">
                <strong>${esc(player.name)}</strong>
                <small>${esc(player.id.slice(0, 8))}${player.room ? ` · стол «${esc(player.room)}»` : ''}</small>
              </span>
              <span class="admin-money">
                <b>${chips(player.balance)}</b>
                <small>${player.games} игр · ${player.wins} побед · ${player.online ? 'онлайн' : 'офлайн'}</small>
              </span>
            </button>
          `).join('')
          : '<div class="admin-empty">Никого не нашли</div>';
        list.querySelectorAll('[data-player]').forEach(row => {
          row.addEventListener('click', () => {
            const player = players.find(item => item.id === row.dataset.player);
            if (player) openAdminPlayer(player);
          });
        });
      };
      search.addEventListener('input', draw);
      draw();
    });
  }).catch(error => {
    if (error.message.toLowerCase().includes('ключ')) {
      storeAdminKey('');
      syncAdminNav();
    }
    toast(error.message);
  });
}

function openAdminGate() {
  openModal(`
    <div class="modal-label">ДОСТУП</div>
    <h2>Панель клуба</h2>
    <p class="modal-subtitle">Ключ задаётся переменной окружения ADMIN_KEY на сервере</p>
    <label>Ключ администратора<input id="aKey" type="password" autocomplete="off" placeholder="Ключ"></label>
    <div class="modal-actions">
      <button class="secondary-btn" id="aCancel">Отмена</button>
      <button class="primary-btn" id="aEnter">Войти</button>
    </div>
  `, root => {
    const input = root.querySelector('#aKey');
    const submit = async () => {
      const key = input.value.trim();
      if (!key) return toast('Введите ключ');
      const button = root.querySelector('#aEnter');
      if (button.disabled) return;
      button.disabled = true;
      try {
        await api('/api/admin', { key, op: 'list' });
        storeAdminKey(key);
        syncAdminNav();
        openAdminPanel();
      } catch (error) {
        button.disabled = false;
        toast(error.message);
      }
    };
    root.querySelector('#aCancel').addEventListener('click', closeModal);
    root.querySelector('#aEnter').addEventListener('click', submit);
    input.addEventListener('keydown', event => {
      if (event.key === 'Enter') {
        event.preventDefault();
        submit();
      }
    });
  });
}

function openAdminPlayer(player) {
  openModal(`
    <div class="modal-label">ПРОФИЛЬ ИГРОКА</div>
    <h2>${esc(player.name)}</h2>
    <p class="modal-subtitle">ID ${esc(player.id)}${player.room ? ` · стол «${esc(player.room)}»` : ''}</p>
    <div class="modal-grid">
      <label>Имя<input id="aName" maxlength="20" value="${esc(player.name)}"></label>
      <label>Баланс, фишек<input id="aBalance" type="number" min="0" step="10" value="${player.balance}"></label>
    </div>
    <div class="modal-grid modal-grid-3">
      <label>Партии<input id="aGames" type="number" min="0" value="${player.games}"></label>
      <label>Победы<input id="aWins" type="number" min="0" value="${player.wins}"></label>
      <label>Поражения<input id="aLosses" type="number" min="0" value="${player.losses}"></label>
    </div>
    <div class="modal-actions">
      <button class="ghost-btn" id="aBack">К списку</button>
      <button class="secondary-btn" id="aReset">Сбросить прогресс</button>
      <button class="primary-btn" id="aSave">Сохранить</button>
    </div>
  `, root => {
    root.querySelector('#aBack').addEventListener('click', openAdminPanel);
    let armed = false;
    const reset = root.querySelector('#aReset');
    reset.addEventListener('click', async () => {
      if (!armed) {
        armed = true;
        reset.textContent = 'Точно сбросить?';
        return;
      }
      reset.disabled = true;
      try {
        await adminApi({ op: 'reset', id: player.id });
        toast('Прогресс игрока сброшен');
        openAdminPanel();
      } catch (error) {
        reset.disabled = false;
        toast(error.message);
      }
    });
    root.querySelector('#aSave').addEventListener('click', async () => {
      const name = root.querySelector('#aName').value.trim();
      if (!name) return toast('Введите имя');
      const button = root.querySelector('#aSave');
      if (button.disabled) return;
      button.disabled = true;
      const number = id => Number(root.querySelector(id).value);
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
        openAdminPanel();
      } catch (error) {
        button.disabled = false;
        toast(error.message);
      }
    });
  });
}

/* navigation */
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
      if (state.game) setView('game');
      else openModal(`<div class="modal-label">МОИ ИГРЫ</div><h2>Активных игр нет</h2><p class="modal-subtitle">Создайте лобби или войдите по коду друга</p><button class="primary-btn full" id="mEmptyCreate">Создать лобби</button>`, root => {
        root.querySelector('#mEmptyCreate').addEventListener('click', openCreateModal);
      });
    } else if (view === 'rules') {
      openRulesModal();
    } else if (view === 'settings') {
      openSettingsModal();
    } else if (view === 'admin') {
      openAdminPanel();
    }
  });
});
syncAdminNav();

function setView(view) {
  state.view = view;
  const gameActive = view === 'game' && Boolean(state.game);
  document.querySelector('#lobbyView').style.display = gameActive ? 'none' : '';
  document.querySelector('#gameScreen').classList.toggle('active', gameActive);
  document.querySelectorAll('.nav-item').forEach(item => {
    const active = item.dataset.view === view || (view === 'game' && item.dataset.view === 'mygames');
    item.classList.toggle('active', active);
    if (active) item.setAttribute('aria-current', 'page');
    else item.removeAttribute('aria-current');
  });
  updateResumeButton();
  if (gameActive) fitHand();
  if (view === 'lobby') renderLobby();
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
function connect() {
  if (state.source) state.source.close();
  state.source = new EventSource(`/events?id=${encodeURIComponent(clientId)}&token=${encodeURIComponent(sessionToken)}&name=${encodeURIComponent(state.profile.name)}`);
  state.source.onopen = () => {
    state.connected = true;
    document.querySelector('#connectionStatus').textContent = 'Столы обновляются в реальном времени';
    document.querySelector('#connectionStatus').dataset.status = 'online';
    renderLobby();
  };
  state.source.onmessage = event => {
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
    } else if (payload.type === 'game') {
      state.game = payload.state;
      if (state.autoOpen) {
        state.autoOpen = false;
        setView('game');
      }
      renderGame();
      updateResumeButton();
    } else if (payload.type === 'noroom') {
      if (state.game) {
        state.game = null;
        document.querySelector('#finishOverlay').classList.remove('open');
        setView('lobby');
        toast('Лобби больше не существует');
      }
    }
  };
  state.source.onerror = () => {
    state.connected = false;
    document.querySelector('#connectionStatus').textContent = 'Связь потеряна. Переподключаемся…';
    document.querySelector('#connectionStatus').dataset.status = 'offline';
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

function cardCornersHTML(card) {
  const faceRank = 'ВДКТ'.includes(card.rank);
  const corner = `<span class="card-corner">${esc(card.rank)}${faceRank ? `<i>${card.suit}</i>` : ''}</span>`;
  return `${corner}${cardBodyHTML(card)}<span class="card-corner bottom" aria-hidden="true">${esc(card.rank)}${faceRank ? `<i>${card.suit}</i>` : ''}</span>`;
}

function cardHTML(card, extraClass = '') {
  const red = card.suit === '♥' || card.suit === '♦';
  return `<div class="card-face ${red ? 'red' : ''} ${extraClass}">${cardCornersHTML(card)}</div>`;
}

function cardBackHTML(extraClass = '') {
  return `<span class="card-back ${extraClass}" aria-hidden="true"><i></i><b>♠</b></span>`;
}

function renderGame() {
  const game = state.game;
  if (!game) return;
  const previousHand = new Set([...document.querySelectorAll('.hand-card')].map(button => button.dataset.card));
  const previousTable = new Set([...document.querySelectorAll('[data-target-card]')].map(card => card.dataset.targetCard));
  document.querySelector('#tableName').textContent = game.room.name;
  const modeLabel = game.room.mode === 'transfer' ? 'Переводной' : 'Подкидной';
  document.querySelector('#tableMode').textContent = `${modeLabel} · ${game.room.deckSize} карт${game.room.bet ? ` · ставка ${chips(game.room.bet)}` : ''}${game.trump ? ` · козырь ${game.trump}` : ''}`;
  document.querySelector('#roomCode').textContent = `КОД ${game.room.id}`;

  const trump = document.querySelector('#trumpCard');
  trump.innerHTML = game.trumpCard ? cardHTML(game.trumpCard) : '';
  trump.classList.toggle('empty', !game.trumpCard);
  const deckBox = document.querySelector('#deckBox');
  deckBox.classList.toggle('empty', !game.deck);
  deckBox.style.setProperty('--depth', String(Math.max(2, Math.min(7, Math.round(game.deck / 4) + 1))));
  document.querySelector('#deckCount').textContent = game.deck;

  const table = document.querySelector('#tableCards');
  table.innerHTML = game.table.length
    ? game.table.map(row => `<div class="pair">${cardHTML(row.attack, `attack ${row.defense ? '' : 'target'}`).replace('<div ', `<div data-target-card="${esc(row.attack.id)}" `)}${row.defense ? cardHTML(row.defense, 'defense') : ''}</div>`).join('')
    : '';
  table.querySelectorAll('.target').forEach(target => {
    target.addEventListener('dragover', event => event.preventDefault());
    target.addEventListener('drop', event => {
      event.preventDefault();
      const cardId = event.dataTransfer.getData('text/plain');
      if (cardId) playCard(cardId, target.dataset.targetCard);
    });
  });

  document.querySelector('#tableMessage').textContent = game.room.status === 'waiting'
    ? `Ожидаем игроков. Код приглашения: ${game.room.id}`
    : game.message;

  const myIndex = game.players.findIndex(player => player.id === (game.you && game.you.id));
  const seats = [];
  const total = game.players.length;
  for (let step = 1; step < total; step += 1) {
    const index = myIndex === -1 ? step - 1 : (myIndex + step) % total;
    seats.push(game.players[index]);
  }
  if (myIndex === -1) seats.push(game.players[0]);
  document.querySelector('#seats').innerHTML = seats.slice(0, 5).map((player, index) => `
    <div class="seat pos-${index + 1} ${player.id === game.turnId ? 'turn' : ''} ${player.connected ? '' : 'offline'}">
      <span class="mini-avatar" style="background:${tintOf(player.name)}">${esc(initials(player.name))}</span>
      <strong>${esc(player.name)}</strong>
      <span class="seat-cards">${player.finished ? 'вышел' : player.cards ? `<span class="opponent-hand">${Array.from({ length: Math.min(player.cards, 10) }, () => cardBackHTML()).join('')}</span><small>${player.cards} карт</small>` : 'нет карт'}</span>
      ${!player.finished && player.role === 'attacker' ? '<span class="seat-role attacker">атакует</span>' : ''}
      ${!player.finished && player.role === 'defender' ? '<span class="seat-role defender">защищается</span>' : ''}
    </div>
  `).join('');

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
  document.querySelector('#myHand').innerHTML = hand.map(card => {
    const red = card.suit === '♥' || card.suit === '♦';
    return `<button class="hand-card ${red ? 'red' : ''} ${dimmed(card) ? 'dim' : ''}" data-card="${esc(card.id)}">${cardCornersHTML(card)}</button>`;
  }).join('');
  document.querySelectorAll('.hand-card').forEach((button, index) => {
    button.setAttribute('aria-label', button.dataset.card);
    if (!previousHand.has(button.dataset.card) && !matchMedia('(prefers-reduced-motion: reduce)').matches) {
      button.animate([{ opacity: 0, transform: 'translateY(18px)' }, { opacity: 1, transform: 'translateY(0)' }], { duration: 380, delay: Math.min(index * 35, 210), easing: 'cubic-bezier(.22,1,.36,1)', fill: 'backwards' });
    }
    button.draggable = true;
    button.addEventListener('dragstart', event => event.dataTransfer.setData('text/plain', button.dataset.card));
    button.addEventListener('click', () => playCard(button.dataset.card));
  });

  if (!matchMedia('(prefers-reduced-motion: reduce)').matches) {
    table.querySelectorAll('[data-target-card]').forEach(card => {
      if (!previousTable.has(card.dataset.targetCard)) card.animate([{ opacity: 0, transform: 'translateY(12px) rotate(-4deg)' }, { opacity: 1, transform: 'translateY(0) rotate(0)' }], { duration: 320, easing: 'cubic-bezier(.22,1,.36,1)' });
    });
  }
  renderActions(game);
  fitHand();
  fitTable();
  renderFinish(game);
}

function fitTable() {
  const row = document.querySelector('#tableCards');
  if (!row) return;
  row.style.removeProperty('zoom');
  if (!row.querySelector('.pair') || !row.clientWidth) return;
  const content = row.scrollWidth;
  if (!content) return;
  const scale = row.clientWidth / content;
  if (scale < .999) row.style.zoom = Math.max(.7, scale).toFixed(3);
}

function fitHand() {
  const hand = document.querySelector('#myHand');
  if (!hand) return;
  hand.style.removeProperty('zoom');
  const cards = hand.querySelectorAll('.hand-card');
  const cardWidth = cards.length ? cards[0].offsetWidth : 0;
  if (cards.length < 2 || !cardWidth || !hand.clientWidth) {
    hand.style.removeProperty('--overlap');
    return;
  }
  const styles = getComputedStyle(hand);
  const padding = (parseFloat(styles.paddingLeft) || 0) + (parseFloat(styles.paddingRight) || 0);
  const available = Math.max(0, hand.clientWidth - padding);
  const minStep = Math.max(15, Math.min(34, cardWidth * .3));
  const needed = (cards.length * cardWidth - available) / (cards.length - 1);
  const overlap = Math.max(0, Math.min(cardWidth - minStep, Math.ceil(Math.max(needed, cardWidth * .12))));
  hand.style.setProperty('--overlap', `${overlap}px`);
  const total = cardWidth + (cards.length - 1) * (cardWidth - overlap);
  if (total > available + 1) hand.style.zoom = Math.max(.6, available / total).toFixed(3);
}

function renderActions(game) {
  const actions = document.querySelector('#gameActions');
  const buttons = [];
  if (game.room.status === 'playing') {
    if (game.canTake) buttons.push({ label: game.takePending ? 'Забрать карты' : 'Взять карты', action: 'take', style: game.takePending ? 'primary-btn' : 'secondary-btn' });
    if (game.canTransfer) buttons.push({ label: 'Перевести', action: 'transfer', style: 'secondary-btn' });
    if (game.canPass) buttons.push({ label: game.takePending ? 'Хватит, забирайте' : 'Бито', action: 'pass', style: 'primary-btn' });
    if (!buttons.length) buttons.push({ label: `Ход: ${game.turnName}`, action: null, style: 'secondary-btn', disabled: true });
  } else {
    if (game.canStart) buttons.push({ label: game.room.status === 'finished' ? 'Играть снова' : 'Начать игру', action: 'start', style: 'primary-btn' });
    else if (game.room.status === 'waiting') {
      const isHost = Boolean(game.you && game.you.id === game.room.hostId);
      const connected = game.players.filter(player => player.connected).length;
      buttons.push({ label: isHost ? `Ждём игроков (${connected}/6)` : 'Ждём, пока создатель начнёт игру', action: null, style: 'secondary-btn', disabled: true });
    }
    buttons.push({ label: `Код: ${game.room.id}`, action: 'copy', style: 'secondary-btn' });
  }
  actions.innerHTML = buttons.map(button => `<button class="${button.style}" ${button.disabled ? 'disabled' : ''} data-action="${button.action || ''}">${esc(button.label)}</button>`).join('');
  actions.querySelectorAll('button[data-action]').forEach(element => {
    const action = element.dataset.action;
    if (!action) return;
    element.addEventListener('click', () => {
      if (action === 'copy') return copyInvite();
      if (action === 'transfer') return selectTransferCard();
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
    if (game.takePending) return toast('Вы уже решили взять карты');
    if (!game.canDefend) return toast('Сейчас отбивать нечего');
    const row = targetCardId
      ? game.table.find(item => !item.defense && item.attack.id === targetCardId)
      : game.table.find(item => !item.defense && beats(item.attack, card, game.trump)) || game.table.find(item => !item.defense);
    if (row && !beats(row.attack, card, game.trump)) {
      return toast(`Карта ${card.rank}${card.suit} не бьёт ${row.attack.rank}${row.attack.suit}`);
    }
    return act('defend', cardId, row ? row.attack.id : targetCardId);
  }
  toast('Сейчас ходит другой игрок');
}

async function act(action, cardId, targetCardId = null) {
  if (state.actionPending) return;
  if (!state.connected) return toast('Нет связи с сервером. Переподключаемся…');
  state.actionPending = true;
  document.querySelector('#gameScreen').setAttribute('aria-busy', 'true');
  try {
    await api('/api/action', { clientId, action, cardId, targetCardId });
  } catch (error) {
    toast(error.message);
  } finally {
    state.actionPending = false;
    document.querySelector('#gameScreen').setAttribute('aria-busy', 'false');
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

document.querySelector('#roomCode').addEventListener('click', copyInvite);

async function leaveRoom() {
  try {
    await api('/api/leave', { clientId });
  } catch (error) {
    /* лобби могло быть уже удалено */
  }
  state.game = null;
  document.querySelector('#finishOverlay').classList.remove('open');
  setView('lobby');
}
document.querySelector('#leaveRoom').addEventListener('click', () => {
  if (!state.game || state.game.room.status !== 'playing') return leaveRoom();
  openModal('<div class="modal-label">ПОКИНУТЬ СТОЛ</div><h2>Уже уходите?</h2><p class="modal-subtitle">Вы выйдете из текущей партии. Другие игроки продолжат без вас.</p><div class="modal-actions"><button class="secondary-btn" id="stayAtTable">Остаться</button><button class="primary-btn" id="confirmLeave">Покинуть стол</button></div>', root => {
    root.querySelector('#stayAtTable').addEventListener('click', closeModal);
    root.querySelector('#confirmLeave').addEventListener('click', async () => {
      await leaveRoom();
      closeModal();
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
  const cap = Math.min(innerWidth * .3, config.width[1]);
  const width = Math.round(randomBetween(Math.min(config.width[0], cap), cap));
  const duration = randomBetween(config.duration[0], config.duration[1]);
  const style = [
    `--w:${width}px`,
    `--y:${randomBetween(2, 86).toFixed(1)}%`,
    `--dy:${randomBetween(-20, 14).toFixed(1)}vh`,
    `--dur:${duration.toFixed(1)}s`,
    `--delay:${(initial ? -randomBetween(0, duration) : 0).toFixed(1)}s`,
    `--dim:${Math.min(1, config.dim * randomBetween(.9, 1.1)).toFixed(2)}`,
    `--rx:${randomBetween(2, 82).toFixed(1)}%`,
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
startAmbient();
renderMasthead();
setInterval(renderMasthead, 30000);
renderLobby();
connect();

const params = new URLSearchParams(location.search);
const inviteCode = params.get('code');
if (inviteCode) openJoinModal(inviteCode.toUpperCase());