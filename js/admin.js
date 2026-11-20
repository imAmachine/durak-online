import { createApi } from './api.js';
import { ADMIN_FILTERS, AVATAR_TINTS, esc, chips } from './shared.js';

const adminPage = document.body.dataset.admin === 'page';
const adminState = { players: [], query: '', selected: null, loading: false, filter: 'all', sort: 'recent' };
const api = createApi(() => '');
let toastTimer = null;
function toast(message) {
  const element = document.querySelector('#toast');
  if (!element) return;
  element.textContent = message;
  element.classList.add('show');
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => element.classList.remove('show'), 2600);
}
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
  document.body.classList.add('modal-open');
  const heading = body.querySelector('h2');
  if (heading) { heading.id = 'modalTitle'; backdrop.querySelector('.modal').setAttribute('aria-labelledby', 'modalTitle'); }
  if (onMount) onMount(body);
  (body.querySelector('input, select, button') || backdrop.querySelector('#modalClose')).focus();
}
function closeModal() {
  if (!backdrop) return;
  backdrop.classList.remove('open');
  document.body.classList.remove('modal-open');
  if (modalTrigger && modalTrigger.isConnected) modalTrigger.focus();
  backdrop.inert = true;
  backdrop.setAttribute('aria-hidden', 'true');
}
if (document.querySelector('#modalClose')) document.querySelector('#modalClose').addEventListener('click', closeModal);
function modalHeadingHTML(label, title, subtitle) { return `<div class="modal-label">${esc(label)}</div><h2>${esc(title)}</h2><p class="modal-subtitle">${esc(subtitle)}</p>`; }
function primaryButtonHTML(id, label, full = false) { return `<button class="primary-btn${full ? ' full' : ''}" id="${id}">${esc(label)}</button>`; }
function passwordFieldHTML(id, label, type) { return `<label>${esc(label)}<input id="${id}" type="${type}" minlength="4" maxlength="128" value="${esc(generatePassword())}" autocomplete="new-password"></label>`; }
function initials(name) { return String(name).trim().split(/\s+/).slice(0, 2).map(part => part[0] || '').join('').toUpperCase() || '?'; }
function avatarHTML(person, className = 'mini-avatar') { const name = typeof person === 'string' ? person : (person && person.name) || '?'; return `<span class="${className} avatar-tint" style="background:${AVATAR_TINTS[0]}">${esc(initials(name))}</span>`; }
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
    ${modalHeadingHTML('ДОСТУП', 'Панель клуба', 'Ключ задаётся переменной окружения ADMIN_KEY на сервере')}
    <label>Ключ администратора<input id="aKey" type="password" autocomplete="off" placeholder="Ключ"></label>
    <div class="modal-actions">
      <button class="secondary-btn" id="aCancel">На главную</button>
      ${primaryButtonHTML('aEnter', 'Войти')}
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

function filterAdminPlayers(players, query, filter, sort) {
  const term = query.trim().toLowerCase();
  const matches = players.filter(player => {
    if (filter === 'online' ? !player.online : filter !== 'all' && adminKind(player) !== filter) return false;
    return !term || player.name.toLowerCase().includes(term) || player.id.toLowerCase().includes(term);
  });
  const sorters = {
    recent: (a, b) => (Number(b.createdAt) || 0) - (Number(a.createdAt) || 0) || a.name.localeCompare(b.name, 'ru'),
    name: (a, b) => a.name.localeCompare(b.name, 'ru'),
    balance: (a, b) => b.balance - a.balance || a.name.localeCompare(b.name, 'ru'),
    games: (a, b) => b.games - a.games || b.wins - a.wins || a.name.localeCompare(b.name, 'ru'),
    online: (a, b) => Number(b.online) - Number(a.online) || a.name.localeCompare(b.name, 'ru'),
  };
  return matches.sort(sorters[sort] || sorters.recent);
}

function countAdminPlayers(players) {
  const counts = { all: players.length, online: 0, temp: 0, discord: 0, legacy: 0 };
  for (const player of players) {
    const kind = adminKind(player);
    if (kind === 'temp' || kind === 'discord' || kind === 'legacy') counts[kind] += 1;
    if (player.online) counts.online += 1;
  }
  return counts;
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
  return Number.isFinite(value) && value > 0 ? formatClubDate(new Date(value), true) : '';
}

function clubDate(seconds, withTime) {
  const value = Number(seconds);
  return seconds && Number.isFinite(value) && value > 0 ? formatClubDate(new Date(value * 1000), withTime) : '';
}

function formatClubDate(date, withTime) {
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
  const counts = countAdminPlayers(adminState.players);
  host.innerHTML = `
    <div class="admin-filters" role="group" aria-label="Фильтр игроков">
      ${ADMIN_FILTERS.map(([key, label]) => {
        const count = counts[key];
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
  const players = filterAdminPlayers(adminState.players, adminState.query, adminState.filter, adminState.sort);
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
    ${passwordFieldHTML('pPassword', 'Новый пароль', 'password')}
    <div class="modal-actions"><button class="secondary-btn" id="pCancel">Отмена</button>${primaryButtonHTML('pSave', 'Сменить пароль')}</div>
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
        openModal(`<div class="modal-label">ПАРОЛЬ ОБНОВЛЁН</div><h2>${esc(nameInput ? nameInput.value.trim() || player.name : player.name)}</h2><p class="modal-subtitle">Передайте новый пароль игроку. Прежний больше не действует.</p><div class="wallet"><strong>Пароль: ${esc(password)}</strong></div><div class="modal-actions">${primaryButtonHTML('pDone', 'Готово')}</div>`, done => done.querySelector('#pDone').addEventListener('click', closeModal));
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
  bindAdminEditor(host, player);
}

function bindAdminEditor(host, player) {
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
    ${modalHeadingHTML('НОВЫЙ ДОСТУП', 'Учётная запись', 'Передайте игроку имя и пароль. Пароль сохраняется только в виде хеша и показывается один раз.')}
    <label>Имя игрока<input id="tName" maxlength="20" autocomplete="off"></label>
    ${passwordFieldHTML('tPassword', 'Пароль', 'text')}
    <div class="modal-actions"><button class="secondary-btn" id="tGenerate">Другой пароль</button><button class="secondary-btn" id="tCancel">Отмена</button>${primaryButtonHTML('tCreate', 'Создать')}</div>
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
        openModal(`<div class="modal-label">ДОСТУП СОЗДАН</div><h2>${esc(data.account.name)}</h2><p class="modal-subtitle">Сохраните эти данные и передайте игроку.</p><div class="wallet"><strong>Имя: ${esc(data.account.name)}<br>Пароль: ${esc(data.account.password)}</strong></div><div class="modal-actions">${primaryButtonHTML('tDone', 'Готово')}</div>`, done => done.querySelector('#tDone').addEventListener('click', closeModal));
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

export function startAdmin() {
  bindAdminPage();
  bootstrapAdminPage();
}
