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

const clientId = readClientId();
try {
  sessionStorage.setItem('durak.client', clientId);
} catch (error) {
  /* идентификатор будет жить только до перезагрузки */
}

const RANK_ORDER = ['2', '3', '4', '5', '6', '7', '8', '9', '10', 'В', 'Д', 'К', 'Т'];
const SUIT_ORDER = ['♠', '♥', '♦', '♣'];
const AVATAR_TINTS = ['#dceadf', '#f0dfe2', '#f2e7ca', '#dbe7f1', '#e6e0f1', '#f6e2d6'];

const state = {
  profile: loadProfile(),
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
  const response = await fetch(path, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) });
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
}
renderProfile();

function openProfileModal() {
  openModal(`
    <div class="modal-label">УЧЁТНАЯ ЗАПИСЬ</div>
    <h2>Мой профиль</h2>
    <p class="modal-subtitle">Имя видят другие игроки в лобби и за столом</p>
    <label>Имя игрока<input id="mName" maxlength="20" value="${esc(state.profile.name)}"></label>
    <label>Цвет аватара
      <div class="swatches" id="mSwatches">
        ${AVATAR_TINTS.map((tint, index) => `<button class="swatch ${index === state.profile.tint ? 'active' : ''}" data-tint="${index}" style="background:${tint}" aria-label="Цвет ${index + 1}"></button>`).join('')}
      </div>
    </label>
    <div class="modal-actions">
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
    root.querySelector('#mCancel').addEventListener('click', closeModal);
    root.querySelector('#mSave').addEventListener('click', async () => {
      const name = root.querySelector('#mName').value.trim().slice(0, 20);
      if (!name) return toast('Введите имя');
      const button = root.querySelector('#mSave');
      if (button.disabled) return;
      button.disabled = true;
      try {
        await api('/api/profile', { clientId, name });
        state.profile = { name, tint };
        saveProfile();
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
function renderLobby() {
  document.querySelector('#onlineCount').textContent = state.lobby.online;
  const grid = document.querySelector('#gamesGrid');
  const rooms = state.lobby.rooms.filter(room => state.filter === 'all' || room.mode === state.filter);
  document.querySelector('#roomCount').textContent = rooms.length ? String(rooms.length).padStart(2, '0') : '';
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
    return `
      <article class="game-card">
        <div class="game-head">
          <div>
            <h3 class="game-title">${esc(room.name)}</h3>
            <div class="game-meta">Создатель: ${esc(room.host)}</div>
          </div>
          <span class="mode ${room.mode}">${modeLabel}</span>
        </div>
        <div class="game-foot">
          <div class="stacked">${room.players.slice(0, 4).map(player => avatarHTML(player.name)).join('')}</div>
          <span class="slots">${room.players.length} / ${room.seats} · ${room.deckSize} карт</span>
          <button class="join-btn" data-room="${esc(room.id)}" ${playing && !mine ? 'disabled' : ''}>${mine ? 'Вернуться' : playing ? 'Идёт игра' : 'Войти'}</button>
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
      </label>
      <label>Колода
        <select id="mDeck">
          <option value="36">36 карт</option>
          <option value="52">52 карты</option>
        </select>
      </label>
    </div>
    <button class="primary-btn full" id="mCreate">Создать лобби</button>
  `, root => {
    root.querySelector('#mCreate').addEventListener('click', async () => {
      const button = root.querySelector('#mCreate');
      button.disabled = true;
      try {
        const data = await api('/api/rooms', {
          clientId,
          name: root.querySelector('#mRoomName').value.trim() || 'Партия',
          mode: root.querySelector('#mMode').value,
          deckSize: Number(root.querySelector('#mDeck').value),
          playerName: state.profile.name,
        });
        state.autoOpen = true;
        closeModal();
        openGameView();
        toast(`Лобби создано. Код: ${data.code}`);
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
    ${items.length ? `<ul class="history-list">${items.map(item => `<li><span>${esc(item.room)}<br><small>${item.mode === 'transfer' ? 'Переводной' : 'Подкидной'} · ${new Date(item.at).toLocaleString('ru-RU')}</small></span><strong>${esc(item.loser)}</strong></li>`).join('')}</ul>` : '<div class="empty-state">Пока ни одна партия не завершилась</div>'}
  `);
});

const RULES_HTML = `
  <div class="modal-label">ПРАВИЛА</div>
  <h2>Как играть</h2>
  <p class="modal-subtitle">Сервер сам проверяет ходы, подкидывание, козыри и перевод</p>
  <ul class="rules-list">
    <li>Козырь определяется по нижней карте колоды и бьёт любую некозырную масть.</li>
    <li>Внутри одной масти старшая карта бьёт младшую.</li>
    <li>Атакующий подкидывает карты того же достоинства, что уже есть на столе, включая карты, которыми отбивались, но не больше шести за ход.</li>
    <li>Атакующий может выложить сразу несколько карт одного достоинства, не дожидаясь ответа защитника.</li>
    <li>Защищающийся отбивает карты по одной или объявляет «беру»: после этого атакующий может подкинуть ещё карты, а затем карты со стола уходят защитнику.</li>
    <li>«Бито» доступно, когда все карты на столе отбиты; тогда атакующим становится защищавшийся.</li>
    <li>В переводном режиме защищающийся может перевести ход картой того же достоинства, пока карты не отбиты.</li>
    <li>После каждого хода игроки добирают карты до шести: атакующий первым, защищающийся последним.</li>
    <li>Вышедший без карт игрок выбывает. Последний оставшийся — дурак.</li>
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

/* navigation */
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
    }
  });
});

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
    button.className = 'primary-btn';
    document.querySelector('.top-actions').prepend(button);
  }
  button.textContent = `Вернуться в игру «${state.game.room.name}»`;
  button.onclick = () => setView('game');
  button.style.display = state.view === 'game' ? 'none' : '';
}

/* game */
function connect() {
  if (state.source) state.source.close();
  state.source = new EventSource(`/events?id=${encodeURIComponent(clientId)}&name=${encodeURIComponent(state.profile.name)}`);
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

function sortHand(hand) {
  return [...hand].sort((a, b) => {
    if (a.suit !== b.suit) return SUIT_ORDER.indexOf(a.suit) - SUIT_ORDER.indexOf(b.suit);
    return RANK_ORDER.indexOf(a.rank) - RANK_ORDER.indexOf(b.rank);
  });
}

function cardHTML(card, extraClass = '') {
  const red = card.suit === '♥' || card.suit === '♦';
  return `<div class="card-face ${red ? 'red' : ''} ${extraClass}"><span class="card-corner">${esc(card.rank)}<i>${card.suit}</i></span><span class="card-suit">${card.suit}</span></div>`;
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
  document.querySelector('#tableMode').textContent = `${modeLabel} · ${game.room.deckSize} карт${game.trump ? ` · козырь ${game.trump}` : ''}`;
  document.querySelector('#roomCode').textContent = `КОД ${game.room.id}`;

  const trump = document.querySelector('#trumpCard');
  trump.innerHTML = game.trumpCard ? `${esc(game.trumpCard.rank)}${game.trumpCard.suit}<small>Козырь</small>` : '<small>Козырь</small>';
  const trumpIsRed = Boolean(game.trumpCard && (game.trumpCard.suit === '♥' || game.trumpCard.suit === '♦'));
  trump.classList.toggle('red', trumpIsRed);
  trump.classList.toggle('dark', Boolean(game.trumpCard && !trumpIsRed));
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

  const hand = sortHand(game.hand || []);
  const ranksOnTable = new Set();
  game.table.forEach(row => {
    ranksOnTable.add(row.attack.rank);
    if (row.defense) ranksOnTable.add(row.defense.rank);
  });
  const attacking = Boolean(game.you && game.you.role === 'attacker');
  const dimmed = card => {
    if (!(game.you && game.you.turn)) return true;
    if (attacking && game.table.length) return !ranksOnTable.has(card.rank);
    return false;
  };
  document.querySelector('#myHand').innerHTML = hand.map(card => {
    const red = card.suit === '♥' || card.suit === '♦';
    return `<button class="hand-card ${red ? 'red' : ''} ${dimmed(card) ? 'dim' : ''}" data-card="${esc(card.id)}"><span class="hand-corner">${esc(card.rank)}<i>${card.suit}</i></span><span class="hand-suit">${card.suit}</span></button>`;
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
  renderFinish(game);
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
      act(action);
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
  if (game.you.role === 'attacker') {
    if (game.takePending) return toast('Соперник уже забирает карты — подкинуть не выйдет');
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
  overlay.innerHTML = `
    <div class="finish-card">
      <h2>Игра окончена</h2>
      <p>${esc(game.finished.loser)} — дурак</p>
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

/* start */
renderLobby();
connect();

const params = new URLSearchParams(location.search);
const inviteCode = params.get('code');
if (inviteCode) openJoinModal(inviteCode.toUpperCase());