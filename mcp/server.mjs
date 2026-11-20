#!/usr/bin/env node
import http from 'node:http';
import readline from 'node:readline';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

import {
  Ledger,
  cardText,
  handText,
  attachOptions,
  defenseOptions,
  transferOptions,
  dumpOptions,
  chooseMove,
} from './strategy.mjs';

const SERVER_NAME = 'durak';
const SERVER_VERSION = '1.1.0';
const BASE_URL = (process.env.DURAK_URL || 'http://127.0.0.1:4173').replace(/\/+$/, '');
const ADMIN_KEY = (process.env.DURAK_ADMIN_KEY || process.env.ADMIN_KEY || '').trim();
const MAX_PLAYERS = 6;
const BETS = [0, 50, 100, 250, 500];
const SESSION_FILE = process.env.DURAK_MCP_SESSIONS || path.join(os.homedir(), '.local', 'state', 'durak-mcp', 'sessions.json');

// ── HTTP ────────────────────────────────────────────────────────────────────

async function api(pathname, body) {
  const options = body === undefined
    ? {}
    : { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) };
  let response;
  try {
    response = await fetch(BASE_URL + pathname, options);
  } catch (error) {
    throw new Error(`Сервис недоступен по ${BASE_URL} (${error.message}). Запустите npm start.`);
  }
  const text = await response.text();
  let data = {};
  try {
    data = text ? JSON.parse(text) : {};
  } catch (error) {
    data = { raw: text };
  }
  return { status: response.status, ok: response.ok, data };
}

// ── SSE seat ────────────────────────────────────────────────────────────────

class Seat {
  constructor({ label, name, id, token }) {
    this.label = label;
    this.playerName = name;
    this.id = id;
    this.token = token;
    this.wantOpen = false;
    this.streaming = false;
    this.req = null;
    this.res = null;
    this.account = null;
    this.game = null;
    this.lobby = null;
    this.noroom = false;
    this.seq = 0;
    this.gameSeq = 0;
    this.seenSeq = 0;
    this.events = [];
    this.waiters = new Set();
    this.reconnectTimer = null;
    this.lastError = null;
    this.lastEventAt = null;
    this.ledger = new Ledger();
    this._pendingOpen = null;
  }

  open() {
    if (this.streaming) return Promise.resolve();
    this.wantOpen = true;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this._pendingOpen = null;
        reject(new Error(`не удалось открыть поток /events для ${this.label} за 5000 мс`));
      }, 5000);
      timer.unref?.();
      this._pendingOpen = {
        resolve: () => { clearTimeout(timer); resolve(); },
        reject: error => { clearTimeout(timer); reject(error); },
      };
      this._attach();
    });
  }

  close() {
    this.wantOpen = false;
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    try { this.req?.destroy(); } catch (error) {}
    try { this.res?.destroy(); } catch (error) {}
    this.req = null;
    this.res = null;
    this.streaming = false;
    const pending = this._pendingOpen;
    this._pendingOpen = null;
    pending?.reject(new Error('соединение закрыто'));
  }

  _attach() {
    const url = new URL('/events', BASE_URL);
    url.searchParams.set('id', this.id);
    url.searchParams.set('token', this.token);
    url.searchParams.set('name', this.playerName);
    const req = http.get(url, response => {
      if (response.statusCode !== 200) {
        let raw = '';
        response.setEncoding('utf8');
        response.on('data', chunk => { raw += chunk; });
        response.on('end', () => {
          let message = raw.trim();
          try { message = JSON.parse(raw).error || message; } catch (error) {}
          this.lastError = `${response.statusCode}: ${message}`;
          this.streaming = false;
          this._failOpen(new Error(`/events ${this.lastError}`));
          this._scheduleReconnect();
        });
        return;
      }
      this.streaming = true;
      this.lastError = null;
      this.res = response;
      let buffer = '';
      response.setEncoding('utf8');
      response.on('data', chunk => {
        buffer += chunk;
        let index;
        while ((index = buffer.indexOf('\n\n')) !== -1) {
          const block = buffer.slice(0, index);
          buffer = buffer.slice(index + 2);
          for (const line of block.split('\n')) {
            if (!line.startsWith('data: ')) continue;
            let payload;
            try { payload = JSON.parse(line.slice(6)); } catch (error) { continue; }
            this._receive(payload);
          }
        }
      });
      response.on('end', () => { this.streaming = false; this._scheduleReconnect(); });
      response.on('error', () => { this.streaming = false; this._scheduleReconnect(); });
      const pending = this._pendingOpen;
      this._pendingOpen = null;
      pending?.resolve();
    });
    req.on('error', error => {
      this.lastError = error.message;
      this.streaming = false;
      this._failOpen(new Error(`/events ${error.message}`));
      this._scheduleReconnect();
    });
    this.req = req;
  }

  _failOpen(error) {
    const pending = this._pendingOpen;
    this._pendingOpen = null;
    pending?.reject(error);
  }

  _scheduleReconnect() {
    if (!this.wantOpen || this.reconnectTimer) return;
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null;
      if (!this.wantOpen) return;
      this._attach();
    }, 1000);
    this.reconnectTimer.unref?.();
  }

  _receive(payload) {
    this.lastEventAt = Date.now();
    this.seq += 1;
    payload.streamSeq = this.seq;
    if (payload.type === 'game') {
      this.game = payload.state;
      this.gameSeq += 1;
      this.noroom = false;
      this.ledger.observe(payload.state);
    } else if (payload.type === 'account') {
      this.account = payload.account;
    } else if (payload.type === 'lobby') {
      this.lobby = payload;
    } else if (payload.type === 'noroom') {
      this.game = null;
      this.noroom = true;
    }
    this.events.push(payload);
    if (this.events.length > 40) this.events.shift();
    for (const waiter of [...this.waiters]) waiter();
  }

  waitFor(predicate, timeoutMs = 5000) {
    return new Promise(resolve => {
      let settled = false;
      const waiter = () => {
        if (settled) return true;
        let hit = false;
        try { hit = Boolean(predicate(this)); } catch (error) { hit = false; }
        if (!hit) return false;
        settled = true;
        clearTimeout(timer);
        this.waiters.delete(waiter);
        resolve(true);
        return true;
      };
      const timer = setTimeout(() => {
        if (settled) return;
        settled = true;
        this.waiters.delete(waiter);
        resolve(false);
      }, timeoutMs);
      timer.unref?.();
      if (waiter()) return;
      this.waiters.add(waiter);
    });
  }
}

const seats = new Map();

function sessionPath() {
  return SESSION_FILE;
}

function saveSessions() {
  const target = sessionPath();
  if (!target) return;
  const list = [...seats.values()]
    .filter(seat => seat.id && seat.token)
    .map(seat => ({ label: seat.label, name: seat.playerName, id: seat.id, token: seat.token }));
  try {
    fs.mkdirSync(path.dirname(target), { recursive: true, mode: 0o700 });
    fs.writeFileSync(target, `${JSON.stringify(list, null, 2)}\n`, { mode: 0o600 });
  } catch (error) {
    process.stderr.write(`durak-mcp: не удалось сохранить места в ${target}: ${error.message}\n`);
  }
}

function loadSessions() {
  const target = sessionPath();
  if (!target || !fs.existsSync(target)) return [];
  try {
    const list = JSON.parse(fs.readFileSync(target, 'utf8'));
    if (!Array.isArray(list)) return [];
    return list.filter(item => item && typeof item.label === 'string' && typeof item.id === 'string' && typeof item.token === 'string');
  } catch (error) {
    process.stderr.write(`durak-mcp: не удалось прочитать ${target}: ${error.message}\n`);
    return [];
  }
}

async function restoreSessions() {
  const saved = loadSessions();
  if (!saved.length) return;
  const restored = await Promise.all(saved.map(async item => {
    if (seats.has(item.label)) return null;
    const seat = new Seat({ label: item.label, name: String(item.name || item.label).slice(0, 20), id: item.id, token: item.token });
    seats.set(item.label, seat);
    try {
      await seat.open();
      await seat.waitFor(payload => payload.lobby || payload.game || payload.noroom, 2000);
      return null;
    } catch (error) {
      seats.delete(item.label);
      seat.close();
      return item.label;
    }
  }));
  if (restored.some(Boolean)) saveSessions();
}

function seatFromArgs(args) {
  const label = args?.seat ? String(args.seat) : '';
  if (label) {
    const seat = seats.get(label) || [...seats.values()].find(item => item.id === label || item.id.startsWith(label));
    if (!seat) {
      const known = [...seats.keys()].join(', ') || '—';
      throw new Error(`Нет места «${label}». Открытые места: ${known}`);
    }
    return seat;
  }
  if (seats.size === 1) return [...seats.values()][0];
  if (!seats.size) throw new Error('Нет открытых мест: сначала вызовите durak_connect');
  throw new Error(`Открыто несколько мест (${[...seats.keys()].join(', ')}): укажите seat`);
}

// ── formatting ──────────────────────────────────────────────────────────────

function flagsOf(state) {
  const flags = [];
  if (state.canAttack) flags.push('можно подкинуть');
  if (state.canDefend) flags.push('можно отбиваться');
  if (state.canTransfer) flags.push('можно перевести');
  if (state.canTake) flags.push('можно взять карты');
  if (state.canPass) flags.push('можно сказать «бито»');
  if (state.canStart) flags.push('можно начать игру');
  if (state.takePending) flags.push('решено взять карты');
  return flags;
}

function tableText(state) {
  if (!state.table.length) return 'пуст';
  return state.table.map(row => `${cardText(row.attack)}${row.defense ? '←' + cardText(row.defense) : '!'}`).join(' ');
}

function withoutLog(state) {
  if (!state) return null;
  const { log, ...rest } = state;
  return rest;
}

function compactState(state, ledger = null) {
  if (!state) return null;
  return {
    room: state.room.id,
    status: state.room.status,
    role: state.you ? state.you.role : 'spectator',
    turn: state.turnName || null,
    attacker: state.attackerName || null,
    defender: state.defenderName || null,
    trump: state.trump,
    deck: state.deck,
    table: tableText(state),
    hand: state.hand.map(cardText),
    counts: state.players.map(player => `${player.name}:${player.cards}`),
    actions: flagsOf(state),
    message: state.message,
    seq: state.seq,
    finished: state.finished,
    hidden: ledger && state.room.status === 'playing' ? ledger.counts(state) : undefined,
  };
}

function formatState(seat, state, options = {}) {
  if (!state) {
    return `${seat.label} (${seat.playerName}): вне стола${seat.noroom ? ' (в лобби)' : ''}`;
  }
  const lines = [];
  lines.push(`${seat.label} (${seat.playerName}) · стол ${state.room.id} «${state.room.name}» · ${state.room.status}`);
  lines.push(`Режим: ${state.room.mode === 'transfer' ? 'переводной' : 'подкидной'} · колода ${state.room.deckSize} · ставка ${state.room.bet}`);
  lines.push(`Козырь ${state.trumpCard ? cardText(state.trumpCard) : state.trump || '—'} · в колоде ${state.deck}`);
  lines.push(`Атакует ${state.attackerName || '—'} · защищается ${state.defenderName || '—'} · ход ${state.turnName || '—'}`);
  lines.push(`Вы: ${state.you ? `${state.you.role}, карт ${state.hand.length}` : 'зритель'}`);
  lines.push(`Стол: ${tableText(state)}`);
  lines.push(`Рука: ${handText(state.hand)}`);
  lines.push(`Действия: ${flagsOf(state).join(', ') || '—'}`);
  if (seat?.ledger && state.room.status === 'playing') lines.push(`Учёт карт: ${seat.ledger.summary(state).line}`);
  if (state.message) lines.push(`Сообщение: ${state.message}`);
  if (state.finished) lines.push(`Итог: дурак — ${state.finished.loser || '—'}`);
  if (options.verbose) {
    lines.push('');
    lines.push('Состояние:');
    lines.push(JSON.stringify(withoutLog(state), null, 2));
  }
  return lines.join('\n');
}

function ok(text, structured) {
  const result = { content: [{ type: 'text', text }] };
  if (structured && typeof structured === 'object') result.structuredContent = structured;
  return result;
}

function fail(text, structured) {
  return { ...ok(text, structured), isError: true };
}

function seatSummary(seat) {
  return {
    seat: seat.label,
    name: seat.playerName,
    clientId: seat.id,
    connected: seat.streaming,
    account: seat.account,
    room: seat.game ? seat.game.room : null,
    status: seat.game ? seat.game.room.status : seat.noroom ? 'lobby' : 'connecting',
    lastError: seat.lastError,
  };
}

async function postAction(seat, action, extra = {}) {
  const payload = { clientId: seat.id, token: seat.token, action, ...extra };
  const result = await api('/api/action', payload);
  if (!result.ok || result.data.error) {
    const error = new Error(`Сервер отклонил ${action}: ${result.data.error || `HTTP ${result.status}`}`);
    error.status = result.status;
    error.rejected = true;
    throw error;
  }
  return result.data;
}

async function actAndWait(seat, action, extra = {}) {
  const before = seat.seq;
  let data;
  try {
    data = await postAction(seat, action, extra);
  } catch (error) {
    if (!error.rejected) throw error;
    await seat.waitFor(item => item.seq > before, 800);
    data = await postAction(seat, action, extra);
  }
  await seat.waitFor(item => item.seq > before, 3000);
  markSeen(seat);
  return data;
}

// ── server hooks (/api/watch long-poll) ─────────────────────────────────────

const WATCH_MODES = ['any', 'game', 'your-turn', 'opponent', 'finished'];

const EVENT_VERBS = {
  attack: 'ходит',
  defend: 'отбивает',
  transfer: 'переводит',
  take: 'берёт карты',
  pass: 'говорит «бито»',
  start: 'начинает партию',
  create: 'создаёт стол',
  join: 'садится за стол',
  leave: 'покидает стол',
  connect: 'подключается',
  disconnect: 'отключается',
  rename: 'меняет имя',
  admin: 'правит профиль',
  cancel: 'отменяет партию',
  sync: 'обновляет стол',
};

function yourTurn(state) {
  return Boolean(state && state.you && (state.canAttack || state.canDefend || state.canTransfer || state.canTake || state.canPass || state.canStart));
}

function describeEvent(event) {
  const who = event.actorName || 'кто-то';
  const verb = EVENT_VERBS[event.action] || event.action;
  const card = event.cardId ? ` ${event.cardId}` : '';
  const target = event.targetCardId ? ` на ${event.targetCardId}` : '';
  return `${who} ${verb}${card}${target}`;
}

function describeEvents(events) {
  if (!events || !events.length) return 'новых действий не было';
  return events.map(describeEvent).join('; ');
}

function applySnapshot(seat, state) {
  if (!state) return;
  seat.game = state;
  seat.gameSeq += 1;
  seat.noroom = false;
  seat.ledger.observe(state);
}

async function waitSse(seat, until, timeoutMs) {
  const before = seat.seq;
  const checks = {
    any: () => true,
    game: item => Boolean(item.game),
    'your-turn': item => yourTurn(item.game),
    opponent: item => Boolean(item.game),
    finished: item => Boolean(item.game && (item.game.finished || item.game.room.status === 'finished')),
  };
  const hit = await seat.waitFor(item => item.seq > before && checks[until](item), timeoutMs);
  return { via: 'sse', hit, seq: Number(seat.game?.seq) || 0, since: Number(seat.game?.seq) || 0, events: [], state: seat.game, roomId: seat.game ? seat.game.room.id : null };
}

function markSeen(seat) {
  seat.seenSeq = seat.seq;
}

function freshStream(seat) {
  return seat.events.filter(item => (item.streamSeq || 0) > seat.seenSeq);
}

function backlogEvents(seat) {
  const seen = new Set();
  const events = [];
  for (const item of freshStream(seat)) {
    for (const event of item.state?.log || []) {
      if (seen.has(event.seq)) continue;
      seen.add(event.seq);
      events.push(event);
    }
  }
  return events.sort((a, b) => a.seq - b.seq);
}

function untilReached(state, until) {
  if (!state) return false;
  if (until === 'your-turn') return yourTurn(state);
  if (until === 'finished') return Boolean(state.finished || state.room.status === 'finished');
  return false;
}

function catchUp(seat, until) {
  const fresh = freshStream(seat);
  if (!fresh.length) return null;
  const anyGame = fresh.some(item => item.type === 'game');
  const missed = until === 'any' || ((until === 'game' || until === 'opponent') && anyGame);
  if (!missed && !untilReached(seat.game, until)) return null;
  const events = backlogEvents(seat);
  return {
    via: 'cache',
    hit: true,
    seq: Number(seat.game?.seq) || 0,
    since: events.length ? events[0].seq - 1 : Number(seat.game?.seq) || 0,
    events,
    state: seat.game,
    roomId: seat.game ? seat.game.room.id : null,
  };
}

async function waitHook(seat, until, timeoutMs) {
  const since = Number(seat.game?.seq) || 0;
  const url = new URL('/api/watch', BASE_URL);
  url.searchParams.set('id', seat.id);
  url.searchParams.set('token', seat.token);
  url.searchParams.set('since', String(since));
  url.searchParams.set('until', until);
  url.searchParams.set('timeout', String(Math.round(timeoutMs)));
  let response;
  try {
    response = await fetch(url, { signal: AbortSignal.timeout(Math.round(timeoutMs) + 5000) });
  } catch (error) {
    return waitSse(seat, until, timeoutMs);
  }
  if (response.status === 404) return waitSse(seat, until, timeoutMs);
  const data = await response.json().catch(() => ({}));
  if (!response.ok) throw new Error(`/api/watch ${response.status}: ${data.error || 'ошибка'}`);
  applySnapshot(seat, data.state);
  return {
    via: 'hook',
    hit: Boolean(data.hit),
    seq: Number(data.seq) || 0,
    since,
    roomId: data.roomId || null,
    events: (data.events || []).filter(event => event.seq > since),
    state: data.state || seat.game,
  };
}

async function waitForEvent(seat, until, { totalMs, chunkMs }) {
  const startedAt = Date.now();
  const deadline = startedAt + Math.max(200, totalMs);
  const chunk = Math.max(200, Math.min(60000, chunkMs));
  let result = catchUp(seat, until);
  if (!result && untilReached(seat.game, until)) {
    result = { via: 'cache', hit: true, seq: Number(seat.game?.seq) || 0, since: Number(seat.game?.seq) || 0, events: backlogEvents(seat), state: seat.game, roomId: seat.game ? seat.game.room.id : null };
  }
  if (!result) {
    result = { via: 'hook', hit: false, events: [], state: seat.game, seq: Number(seat.game?.seq) || 0, since: Number(seat.game?.seq) || 0 };
    while (!result.hit) {
      const left = deadline - Date.now();
      if (left <= 0) break;
      const budget = Math.min(chunk, left);
      result = seat.game ? await waitHook(seat, until, budget) : await waitSse(seat, until, budget);
    }
  }
  markSeen(seat);
  return { ...result, until, waitedMs: Date.now() - startedAt };
}

function waitReport(seat, until, result, options = {}) {
  const seconds = (result.waitedMs / 1000).toFixed(1);
  const source = result.via === 'hook' ? 'хук /api/watch' : result.via === 'cache' ? 'уже случилось до вызова' : 'SSE';
  const lines = [];
  if (result.hit && result.via === 'cache') {
    lines.push(`Условие «${until}» выполнилось ещё до вызова: отдаю накопленные события и текущий снимок.`);
  } else {
    lines.push(result.hit
      ? `Хук «${until}» сработал через ${seconds} с (${source}).`
      : `Хук «${until}» не сработал за ${seconds} с (${source}).`);
  }
  lines.push(`События: ${describeEvents(result.events)}`);
  if (seat.game) {
    if (yourTurn(seat.game)) {
      lines.push('');
      lines.push(...hintData(seat, seat.game).lines);
    }
    lines.push('');
    lines.push(formatState(seat, seat.game, options));
  } else {
    lines.push(`${seat.label} (${seat.playerName}): вне стола.`);
  }
  return lines.join('\n');
}

function shortEvents(events, seat) {
  return (events || []).map(event => ({
    seq: event.seq,
    action: event.action,
    actor: event.actorId === seat.id ? 'я' : event.actorName,
    card: event.cardId,
    target: event.targetCardId,
  }));
}

function waitResult(seat, until, result, options = {}) {
  const verbose = Boolean(options.verbose);
  return ok(waitReport(seat, until, result, { verbose }), {
    seat: seat.label,
    until,
    hit: result.hit,
    via: result.via,
    seq: result.seq,
    since: result.since,
    roomId: result.roomId,
    waitedMs: result.waitedMs,
    events: shortEvents(result.events, seat),
    yourTurn: yourTurn(seat.game),
    snapshot: compactState(seat.game, seat.ledger),
    state: verbose ? withoutLog(seat.game) : undefined,
    lastError: seat.lastError,
  });
}

// ── tools ───────────────────────────────────────────────────────────────────

const seatArg = { type: 'string', description: 'Метка места из durak_connect (например p1). Можно не указывать, если место одно.' };
const verboseArg = { type: 'boolean', description: 'Полный JSON-снимок в ответе (по умолчанию краткий отчёт).' };

const TOOLS = [
  {
    name: 'durak_connect',
    description: 'Открыть место игрока: по клубному аккаунту (login+password), по готовой сессии (clientId+token) или открыть временный аккаунт ключом администратора.',
    inputSchema: {
      type: 'object',
      properties: {
        seat: { type: 'string', description: 'Метка места, по умолчанию pN.' },
        name: { type: 'string', description: 'Имя игрока (до 20 символов).' },
        clientId: { type: 'string', description: 'ID готовой сессии; либо DURAK_CLIENT_ID.' },
        token: { type: 'string', description: 'Токен сессии; либо DURAK_TOKEN.' },
        login: { type: 'string', description: 'Имя клубного аккаунта; либо DURAK_LOGIN.' },
        password: { type: 'string', description: 'Пароль клубного аккаунта; либо DURAK_PASSWORD.' },
        create: { type: 'boolean', description: 'Создать новый временный аккаунт ключом администратора.' },
        key: { type: 'string', description: 'Ключ администратора; либо DURAK_ADMIN_KEY/ADMIN_KEY.' },
      },
    },
  },
  {
    name: 'durak_disconnect',
    description: 'Закрыть место игрока. leave=true дополнительно выходит из стола (во время партии это засчитывает поражение).',
    inputSchema: {
      type: 'object',
      properties: { seat: seatArg, leave: { type: 'boolean', description: 'Выйти из стола перед закрытием.' } },
    },
  },
  {
    name: 'durak_seats',
    description: 'Список открытых мест: аккаунт, стол, статус и последняя ошибка потока.',
    inputSchema: { type: 'object', properties: {} },
  },
  {
    name: 'durak_lobby',
    description: 'Лобби: онлайн, открытые столы, история последних партий (открытые столы из потока места, иначе из /api/status).',
    inputSchema: { type: 'object', properties: { seat: { ...seatArg, description: 'Место, из чьего потока брать лобби (необязательно).' } } },
  },
  {
    name: 'durak_room_create',
    description: 'Создать стол. Создатель становится хостом и первым игроком.',
    inputSchema: {
      type: 'object',
      properties: {
        seat: seatArg,
        name: { type: 'string', description: 'Название стола (до 40 символов).' },
        mode: { type: 'string', enum: ['classic', 'transfer'], description: 'classic — подкидной, transfer — переводной.' },
        deckSize: { type: 'integer', enum: [36, 52], description: 'Размер колоды.' },
        bet: { type: 'integer', enum: BETS, description: 'Ставка в фишках (0, 50, 100, 250, 500).' },
      },
    },
  },
  {
    name: 'durak_room_join',
    description: 'Войти в стол по шестисимвольному коду.',
    inputSchema: {
      type: 'object',
      properties: { seat: seatArg, code: { type: 'string', description: 'Код стола, например A1B2C3.' } },
      required: ['code'],
    },
  },
  {
    name: 'durak_room_leave',
    description: 'Покинуть стол, оставаясь на месте.',
    inputSchema: { type: 'object', properties: { seat: seatArg } },
  },
  {
    name: 'durak_start',
    description: 'Начать партию. Доступно хосту, когда подключены минимум два игрока.',
    inputSchema: { type: 'object', properties: { seat: seatArg } },
  },
  {
    name: 'durak_action',
    description: 'Игровое действие: attack (cardId), defend (cardId, targetCardId), transfer (cardId), take, pass.',
    inputSchema: {
      type: 'object',
      properties: {
        seat: seatArg,
        action: { type: 'string', enum: ['attack', 'defend', 'transfer', 'take', 'pass'] },
        cardId: { type: 'string', description: 'Своя карта, например "6♠" или "Т♥".' },
        targetCardId: { type: 'string', description: 'Атакующая карта на столе для defend.' },
        verbose: verboseArg,
      },
      required: ['action'],
    },
  },
  {
    name: 'durak_state',
    description: 'Текущее состояние стола для места: рука, стол, козырь, роли, флаги действий.',
    inputSchema: { type: 'object', properties: { seat: seatArg, verbose: verboseArg } },
  },
  {
    name: 'durak_wait',
    description: 'Дождаться события через серверный хук /api/watch: any (любое), game (новый снимок), your-turn (есть ход), opponent (сходил соперник), finished (партия окончена). Возвращает журнал действий и снимок стола.',
    inputSchema: {
      type: 'object',
      properties: {
        seat: seatArg,
        until: { type: 'string', enum: WATCH_MODES, description: 'Условие ожидания.' },
        timeoutMs: { type: 'integer', description: 'Общий таймаут в миллисекундах (по умолчанию 5000, максимум 120000).' },
        verbose: verboseArg,
      },
    },
  },
  {
    name: 'durak_watch',
    description: 'Ждать действие сервера: long-poll /api/watch до своего хода (по умолчанию), хода соперника или конца партии. Возвращает, кто что сыграл, подсказки и снимок стола; ждёт до totalMs даже без сообщений в чате.',
    inputSchema: {
      type: 'object',
      properties: {
        seat: seatArg,
        until: { type: 'string', enum: WATCH_MODES, description: 'any — любое действие, game — новый снимок, your-turn — ваш ход, opponent — сходил соперник, finished — партия окончена.' },
        totalMs: { type: 'integer', description: 'Общее ожидание в мс (по умолчанию 180000, максимум 600000).' },
        chunkMs: { type: 'integer', description: 'Длина одного long-poll запроса в мс (по умолчанию 25000, максимум 60000).' },
        verbose: verboseArg,
      },
    },
  },
  {
    name: 'durak_follow',
    description: 'Играть своим местом до конца партии без вмешательства человека: сам решает defend/attack/take/pass, ждёт ход через серверный хук и возвращает один краткий отчёт со всеми ходами.',
    inputSchema: {
      type: 'object',
      properties: {
        seat: seatArg,
        until: { type: 'string', enum: ['finished', 'turn'], description: 'finished — довести партию до конца, turn — остановиться, когда ход переходит сопернику.' },
        totalMs: { type: 'integer', description: 'Общий лимит времени в мс (по умолчанию 240000, максимум 900000).' },
        maxSteps: { type: 'integer', description: 'Лимит собственных действий (по умолчанию 400).' },
        verbose: verboseArg,
      },
    },
  },
  {
    name: 'durak_hints',
    description: 'Легальные ходы по текущему снимку: чем можно атаковать, чем отбить каждую карту стола, варианты перевода.',
    inputSchema: { type: 'object', properties: { seat: seatArg } },
  },
  {
    name: 'durak_turn',
    description: 'Один запрос вместо state+hints: снимок стола, легальные ходы, учёт вышедших карт и рекомендованный ход с обоснованием. Ничего не меняет на сервере.',
    inputSchema: { type: 'object', properties: { seat: seatArg, verbose: verboseArg } },
  },
  {
    name: 'durak_profile',
    description: 'Переименовать игрока (имя до 20 символов).',
    inputSchema: {
      type: 'object',
      properties: { seat: seatArg, name: { type: 'string', description: 'Новое имя.' } },
      required: ['name'],
    },
  },
  {
    name: 'durak_admin',
    description: 'Панель клуба: list, create_temp, update (имя, пароль, баланс, статистика), reset, balance, delete. Ключ берётся из аргумента key, иначе из DURAK_ADMIN_KEY/ADMIN_KEY.',
    inputSchema: {
      type: 'object',
      properties: {
        op: { type: 'string', enum: ['list', 'create_temp', 'update', 'reset', 'balance', 'delete'] },
        key: { type: 'string', description: 'Ключ администратора.' },
        id: { type: 'string', description: 'clientId игрока для update/reset/balance/delete.' },
        name: { type: 'string', description: 'Имя игрока; для create_temp — новой учётки.' },
        password: { type: 'string', description: 'Пароль для create_temp/update; без него create_temp сгенерирует пароль. Хранится как хеш.' },
        balance: { type: 'integer' },
        games: { type: 'integer' },
        wins: { type: 'integer' },
        losses: { type: 'integer' },
        delta: { type: 'integer', description: 'Пополнение для op=balance.' },
      },
      required: ['op'],
    },
  },
  {
    name: 'durak_status',
    description: 'Служебный статус сервиса: онлайн, клиенты, столы (/api/status).',
    inputSchema: { type: 'object', properties: {} },
  },
  {
    name: 'durak_autoplay',
    description: 'Играет партию с отдельной авторизованной сессией Discord для каждого игрока.',
    inputSchema: {
      type: 'object',
      properties: {
        seats: { type: 'integer', description: 'Количество мест, от 2 до 6.' },
        credentials: { type: 'array', description: 'Отдельные авторизованные сессии Discord для каждого игрока; либо JSON в DURAK_ACCOUNTS.', items: { type: 'object', properties: { clientId: { type: 'string' }, token: { type: 'string' } }, required: ['clientId', 'token'] } },
        mode: { type: 'string', enum: ['classic', 'transfer'], description: 'Режим партии.' },
        deckSize: { type: 'integer', enum: [36, 52], description: 'Размер колоды.' },
        bet: { type: 'integer', enum: BETS, description: 'Ставка в фишках.' },
        name: { type: 'string', description: 'Базовое имя ботов.' },
        maxSteps: { type: 'integer', description: 'Лимит действий, по умолчанию 300.' },
        keepSeats: { type: 'boolean', description: 'Оставить места открытыми после партии.' },
      },
    },
  },
];

const READ_ONLY = { readOnlyHint: true, destructiveHint: false, idempotentHint: true, openWorldHint: false };
const LOCAL_MUTATION = { readOnlyHint: false, destructiveHint: false, idempotentHint: false, openWorldHint: false };

const ANNOTATIONS = {
  durak_connect: { readOnlyHint: false, destructiveHint: false, idempotentHint: false, openWorldHint: true },
  durak_disconnect: { readOnlyHint: false, destructiveHint: true, idempotentHint: false, openWorldHint: false },
  durak_seats: READ_ONLY,
  durak_lobby: READ_ONLY,
  durak_status: READ_ONLY,
  durak_room_create: LOCAL_MUTATION,
  durak_room_join: LOCAL_MUTATION,
  durak_room_leave: { readOnlyHint: false, destructiveHint: true, idempotentHint: true, openWorldHint: false },
  durak_start: LOCAL_MUTATION,
  durak_action: LOCAL_MUTATION,
  durak_state: READ_ONLY,
  durak_wait: READ_ONLY,
  durak_watch: READ_ONLY,
  durak_follow: LOCAL_MUTATION,
  durak_hints: READ_ONLY,
  durak_turn: READ_ONLY,
  durak_profile: LOCAL_MUTATION,
  durak_admin: { readOnlyHint: false, destructiveHint: true, idempotentHint: false, openWorldHint: true },
  durak_autoplay: { readOnlyHint: false, destructiveHint: false, idempotentHint: false, openWorldHint: true },
};

for (const tool of TOOLS) {
  if (ANNOTATIONS[tool.name]) tool.annotations = ANNOTATIONS[tool.name];
}

async function connectSeat(args) {
  const label = String(args.seat || `p${seats.size + 1}`);
  if (seats.has(label)) throw new Error(`Место «${label}» уже открыто`);
  if (seats.size >= 12) throw new Error('Слишком много открытых мест (12): закройте лишние через durak_disconnect');
  const name = String(args.name || `MCP ${label}`).slice(0, 20);
  let id = args.clientId ?? process.env.DURAK_CLIENT_ID;
  let token = args.token ?? process.env.DURAK_TOKEN;
  if (args.create) {
    const key = String(args.key || ADMIN_KEY);
    if (!key) throw new Error('Создание аккаунта требует ключ администратора: key или DURAK_ADMIN_KEY');
    const generated = `bot-${Math.random().toString(36).slice(2, 10)}`;
    const password = `${Math.random().toString(36).slice(2, 10)}${Math.random().toString(36).slice(2, 6)}`;
    const created = await api('/api/admin', { key, op: 'create_temp', name: generated, password });
    if (!created.ok) throw new Error(`Не удалось создать аккаунт: ${created.data.error || `HTTP ${created.status}`}`);
    args.login = created.data.account.name;
    args.password = password;
  }
  if (args.login ?? process.env.DURAK_LOGIN) {
    const login = String(args.login ?? process.env.DURAK_LOGIN).trim();
    const password = String(args.password ?? process.env.DURAK_PASSWORD ?? '');
    if (!password) throw new Error('Укажите пароль клубного аккаунта: password или DURAK_PASSWORD');
    const session = await api('/api/auth/temp/login', { name: login, password });
    if (!session.ok) throw new Error(`Не удалось войти в клуб: ${session.data.error || `HTTP ${session.status}`}`);
    id = session.data.clientId;
    token = session.data.token;
  }
  if (typeof id !== 'string' || !id.trim() || typeof token !== 'string' || !token.trim()) throw new Error('Укажите login+password, create=true с ключом администратора или clientId+token готовой сессии');
  const seat = new Seat({ label, name, id, token });
  seats.set(label, seat);
  try {
    await seat.open();
  } catch (error) {
    seats.delete(label);
    seat.close();
    throw new Error(`Не удалось открыть место ${label}: сервис недоступен по ${BASE_URL} (${error.message}). Запустите npm start.`);
  }
  await seat.waitFor(item => item.lobby || item.game || item.noroom, 5000);
  saveSessions();
  return ok(
    `Открыто место ${label} (${seat.playerName}), клиент ${seat.id.slice(0, 8)}. ${seat.game ? `Стол ${seat.game.room.id}.` : 'Пока вне стола.'}\n\n` +
    JSON.stringify(seatSummary(seat), null, 2),
    seatSummary(seat),
  );
}

async function disconnectSeat(args) {
  const seat = seatFromArgs(args);
  if (args.leave) {
    await api('/api/leave', { clientId: seat.id, token: seat.token }).catch(() => null);
    seat.game = null;
  }
  const summary = seatSummary(seat);
  seat.close();
  seats.delete(seat.label);
  saveSessions();
  return ok(`Место ${seat.label} закрыто${args.leave ? ' и стол покинут' : ''}.`, summary);
}

async function roomCreate(args) {
  const seat = seatFromArgs(args);
  const body = { clientId: seat.id, token: seat.token, playerName: seat.playerName };
  if (args.name !== undefined) body.name = String(args.name).slice(0, 40);
  if (args.mode !== undefined) body.mode = args.mode;
  if (args.deckSize !== undefined) body.deckSize = Number(args.deckSize);
  if (args.bet !== undefined) body.bet = Number(args.bet);
  const result = await api('/api/rooms', body);
  if (!result.ok) return fail(`Не удалось создать стол: ${result.data.error || `HTTP ${result.status}`}`, result.data);
  const code = result.data.code;
  await seat.waitFor(item => item.game && item.game.room.id === code, 3000);
  return ok(`Стол ${code} создан на месте ${seat.label}.\n\n${formatState(seat, seat.game, { verbose: args.verbose })}`, {
    seat: seat.label,
    code,
    room: result.data.room,
    account: result.data.account,
    snapshot: compactState(seat.game, seat.ledger),
    state: seat.game,
  });
}

async function roomJoin(args) {
  const seat = seatFromArgs(args);
  const code = String(args.code || '').trim().toUpperCase();
  if (!code) throw new Error('Укажите code стола');
  const result = await api('/api/join', { clientId: seat.id, token: seat.token, playerName: seat.playerName, roomId: code });
  if (!result.ok) return fail(`Не удалось войти в стол ${code}: ${result.data.error || `HTTP ${result.status}`}`, result.data);
  await seat.waitFor(item => item.game && item.game.room.id === code, 3000);
  return ok(`Место ${seat.label} вошло в стол ${code}.\n\n${formatState(seat, seat.game, { verbose: args.verbose })}`, {
    seat: seat.label,
    code,
    room: result.data.room,
    account: result.data.account,
    snapshot: compactState(seat.game, seat.ledger),
    state: seat.game,
  });
}

async function roomLeave(args) {
  const seat = seatFromArgs(args);
  await api('/api/leave', { clientId: seat.id, token: seat.token });
  seat.game = null;
  seat.noroom = true;
  return ok(`Место ${seat.label} покинуло стол.`, seatSummary(seat));
}

async function startGame(args) {
  const seat = seatFromArgs(args);
  try {
    await actAndWait(seat, 'start');
  } catch (error) {
    return fail(error.message, { seat: seat.label });
  }
  return ok(`Партия началась.\n\n${formatState(seat, seat.game, { verbose: args.verbose })}`, {
    seat: seat.label,
    snapshot: compactState(seat.game, seat.ledger),
    state: withoutLog(seat.game),
  });
}

async function gameAction(args) {
  const seat = seatFromArgs(args);
  const action = String(args.action || '');
  if (!['attack', 'defend', 'transfer', 'take', 'pass'].includes(action)) {
    throw new Error('action должен быть attack, defend, transfer, take или pass');
  }
  const extra = {};
  if (args.cardId !== undefined) extra.cardId = String(args.cardId);
  if (args.targetCardId !== undefined) extra.targetCardId = String(args.targetCardId);
  try {
    await actAndWait(seat, action, extra);
  } catch (error) {
    return fail(error.message, { seat: seat.label, action });
  }
  return ok(`${action}: принято.\n\n${formatState(seat, seat.game, { verbose: args.verbose })}`, {
    seat: seat.label,
    action,
    snapshot: compactState(seat.game, seat.ledger),
    state: args.verbose ? withoutLog(seat.game) : undefined,
    account: seat.account,
  });
}

function gameState(args) {
  const seat = seatFromArgs(args);
  if (!seat.game) {
    return ok(`${seat.label} (${seat.playerName}): вне стола.`, { seat: seat.label, state: null, snapshot: null });
  }
  const verbose = args.verbose === undefined ? true : Boolean(args.verbose);
  markSeen(seat);
  return ok(formatState(seat, seat.game, { verbose }), { seat: seat.label, snapshot: compactState(seat.game, seat.ledger), state: seat.game });
}

function watchArgs(args) {
  const until = String(args.until || 'any');
  if (!WATCH_MODES.includes(until)) throw new Error(`until должен быть ${WATCH_MODES.join(', ')}`);
  return until;
}

async function waitEvent(args) {
  const seat = seatFromArgs(args);
  const until = watchArgs(args);
  const timeoutMs = Math.max(200, Math.min(120000, Number(args.timeoutMs) || 5000));
  const result = await waitForEvent(seat, until, { totalMs: timeoutMs, chunkMs: timeoutMs });
  return waitResult(seat, until, result, { verbose: args.verbose });
}

async function watchTurn(args) {
  const seat = seatFromArgs(args);
  const until = String(args.until || 'your-turn');
  if (!WATCH_MODES.includes(until)) throw new Error(`until должен быть ${WATCH_MODES.join(', ')}`);
  const totalMs = Math.max(200, Math.min(600000, Number(args.totalMs) || 180000));
  const chunkMs = Math.max(200, Math.min(60000, Number(args.chunkMs) || 25000));
  const result = await waitForEvent(seat, until, { totalMs, chunkMs });
  return waitResult(seat, until, result, { verbose: args.verbose });
}

function myMove(seat, state) {
  return chooseMove(state, { ledger: seat?.ledger || null });
}

async function follow(args) {
  const seat = seatFromArgs(args);
  const verbose = Boolean(args.verbose);
  const until = String(args.until || 'finished');
  if (!['finished', 'turn'].includes(until)) throw new Error('until должен быть finished или turn');
  if (!seat.game) return fail(`${seat.label} (${seat.playerName}): вне стола.`, { seat: seat.label, finished: false });
  const totalMs = Math.max(1000, Math.min(900000, Number(args.totalMs) || 240000));
  const maxSteps = Math.max(1, Math.min(2000, Number(args.maxSteps) || 400));
  const startedAt = Date.now();
  const deadline = startedAt + totalMs;
  const log = [];
  let steps = 0;
  let reason = 'timeout';
  let spins = 0;
  while (Date.now() < deadline) {
    const state = seat.game;
    if (!state) {
      reason = 'no-room';
      break;
    }
    if (state.room.status === 'finished' || state.finished) {
      reason = 'finished';
      break;
    }
    if (steps >= maxSteps) {
      reason = 'steps';
      break;
    }
    const move = myMove(seat, state);
    if (!move) {
      if (until === 'turn') {
        reason = 'opponent';
        break;
      }
      const left = deadline - Date.now();
      if (left <= 0) break;
      const condition = state.room.status === 'playing' ? 'your-turn' : 'any';
      const seqBefore = seat.seq;
      const wait = catchUp(seat, condition) || await waitHook(seat, condition, Math.min(25000, Math.max(300, left)));
      const arrived = wait.events.length > 0 || seat.seq > seqBefore;
      wait.events.forEach(event => {
        if (event.actorId !== seat.id) log.push(describeEvent(event));
      });
      markSeen(seat);
      if (arrived) {
        spins = 0;
        continue;
      }
      spins += 1;
      if (spins >= 3) {
        reason = 'no-move';
        break;
      }
      await sleep(Math.min(400, 50 * spins));
      continue;
    }
    const seqBefore = seat.seq;
    const gameSeqBefore = seat.gameSeq;
    try {
      await postAction(seat, move.action, move.extra);
    } catch (error) {
      log.push(`ошибка: ${error.message}`);
      reason = 'error';
      break;
    }
    steps += 1;
    log.push(`я ${move.note}${move.reason ? ` (${move.reason})` : ''}`);
    await seat.waitFor(item => item.gameSeq > gameSeqBefore, 3000);
    for (const event of freshStream(seat).flatMap(item => item.state?.log || [])) {
      if (event.actorId !== seat.id && !log.includes(describeEvent(event))) log.push(describeEvent(event));
    }
    markSeen(seat);
  }
  if (log.length > 60) log.splice(0, log.length - 60);
  const state = seat.game;
  const finished = Boolean(state && (state.room.status === 'finished' || state.finished));
  const seconds = ((Date.now() - startedAt) / 1000).toFixed(1);
  const lines = [];
  lines.push(`Стол ${state ? state.room.id : '—'}: ${finished ? 'партия завершена' : `партия не завершена (${reason})`}, ${seconds} с, моих действий ${steps}`);
  if (state) {
    lines.push(`Роль ${state.you ? state.you.role : '—'} · козырь ${state.trump} · в колоде ${state.deck} · стол: ${tableText(state)}`);
    if (state.finished) lines.push(`Дурак: ${state.finished.loser || '—'} · моя выплата: ${state.payout ?? 0}`);
  }
  lines.push('Ходы:');
  lines.push(...(log.length ? log.slice(-25).map(item => `  ${item}`) : ['  —']));
  if (state) {
    lines.push('');
    lines.push(formatState(seat, state, { verbose }));
  }
  return ok(lines.join('\n'), {
    seat: seat.label,
    roomId: state ? state.room.id : null,
    steps,
    finished,
    reason,
    loser: state && state.finished ? state.finished.loser : null,
    payout: state ? state.payout ?? 0 : 0,
    waitedMs: Date.now() - startedAt,
    actions: log,
    snapshot: compactState(state, seat.ledger),
    state: verbose ? withoutLog(state) : undefined,
  });
}

function hintData(seat, state) {
  const attacks = attachOptions(state).map(card => ({ card: cardText(card), id: card.id, trump: card.suit === state.trump }));
  const defenses = defenseOptions(state).map(item => ({
    attack: cardText(item.row.attack),
    targetCardId: item.row.attack.id,
    card: cardText(item.card),
    cardId: item.card.id,
    trump: item.trump,
  }));
  const transfers = transferOptions(state).map(card => ({ card: cardText(card), id: card.id }));
  const dumps = dumpOptions(state).map(card => ({ card: cardText(card), id: card.id }));
  const lines = [];
  lines.push(`${seat.label} (${seat.playerName}) · стол ${state.room.id} · ${state.you ? state.you.role : 'зритель'}`);
  lines.push(`Рука: ${handText(state.hand)} · козырь ${state.trump}`);
  lines.push(`Атака: ${attacks.length ? attacks.map(item => item.card).join(' ') : 'нет'}`);
  if (state.canDefend) {
    lines.push(`Защита: ${defenses.length ? defenses.map(item => `${item.attack}→${item.card}`).join(', ') : 'нечем отбиться'}`);
  }
  if (state.canTransfer) lines.push(`Перевод: ${transfers.length ? transfers.map(item => item.card).join(' ') : 'нет'}`);
  if (state.takePending && dumps.length) lines.push(`Догрузка под «беру»: ${dumps.map(item => item.card).join(' ')}`);
  lines.push(`Доступно: take=${state.canTake}, pass=${state.canPass}, transfer=${state.canTransfer}`);
  if (seat?.ledger && state.room.status === 'playing') lines.push(`Учёт карт: ${seat.ledger.summary(state).line}`);
  return {
    lines,
    data: {
      seat: seat.label,
      role: state.you ? state.you.role : null,
      hand: state.hand,
      trump: state.trump,
      attack: attacks,
      defend: defenses,
      transfer: transfers,
      dump: dumps,
      canTake: state.canTake,
      canPass: state.canPass,
      canTransfer: state.canTransfer,
      hidden: seat?.ledger ? seat.ledger.counts(state) : undefined,
    },
  };
}

function hints(args) {
  const seat = seatFromArgs(args);
  const state = seat.game;
  if (!state) return fail(`${seat.label}: вне стола.`, { seat: seat.label, state: null });
  const { lines, data } = hintData(seat, state);
  return ok(lines.join('\n'), data);
}

function turn(args) {
  const seat = seatFromArgs(args);
  const state = seat.game;
  if (!state) return fail(`${seat.label} (${seat.playerName}): вне стола.`, { seat: seat.label, state: null, snapshot: null });
  markSeen(seat);
  const { lines, data } = hintData(seat, state);
  const move = myMove(seat, state);
  const advice = move
    ? `Рекомендую: ${move.action}${move.extra?.cardId ? ` ${move.extra.cardId}` : ''}${move.extra?.targetCardId ? ` на ${move.extra.targetCardId}` : ''} — ${move.reason || move.note}`
    : state.room.status === 'playing'
      ? 'Ход не ваш: ждите через durak_watch (until=your-turn).'
      : 'Партия не идёт.';
  return ok([...lines, advice, '', formatState(seat, state, { verbose: args.verbose })].join('\n'), {
    seat: seat.label,
    ...data,
    recommended: move
      ? { action: move.action, cardId: move.extra?.cardId || null, targetCardId: move.extra?.targetCardId || null, reason: move.reason || move.note }
      : null,
    snapshot: compactState(state, seat.ledger),
    state: args.verbose ? withoutLog(state) : undefined,
  });
}

async function updateProfile(args) {
  const seat = seatFromArgs(args);
  const name = String(args.name || '').trim().slice(0, 20);
  if (!name) throw new Error('Имя не может быть пустым');
  const result = await api('/api/profile', { clientId: seat.id, token: seat.token, name });
  if (!result.ok) return fail(`Не удалось переименовать: ${result.data.error || `HTTP ${result.status}`}`, result.data);
  seat.playerName = name;
  return ok(`Имя изменено на ${name}.`, { seat: seat.label, name, account: result.data.account });
}

async function adminOp(args) {
  const body = { key: String(args.key || ADMIN_KEY), op: String(args.op || 'list') };
  for (const field of ['id', 'name', 'password', 'balance', 'games', 'wins', 'losses', 'delta']) {
    if (args[field] !== undefined) body[field] = args[field];
  }
  const result = await api('/api/admin', body);
  if (!result.ok) return fail(`Админ-операция отклонена: ${result.data.error || `HTTP ${result.status}`}`, result.data);
  const text = body.op === 'list'
    ? `Игроков: ${result.data.players?.length ?? 0}\n\n${JSON.stringify(result.data.players, null, 2)}`
    : body.op === 'create_temp'
      ? `Учётная запись создана: ${result.data.account.name}, пароль ${result.data.account.password} (показывается один раз)`
      : body.op === 'delete'
        ? `Учётная запись ${result.data.name} удалена`
        : `Игрок: ${JSON.stringify(result.data.player)}`;
  return ok(text, result.data);
}

async function status() {
  const result = await api('/api/status');
  if (!result.ok) return fail(`Статус недоступен: HTTP ${result.status}`, result.data);
  return ok(JSON.stringify(result.data, null, 2), result.data);
}

function lobbyView(args) {
  let seat = null;
  try { seat = seatFromArgs(args); } catch (error) { seat = null; }
  const lobby = seat?.lobby;
  if (!lobby) {
    const known = [...seats.values()].map(item => item.lobby).filter(Boolean).pop();
    if (!known) return fail('Лобби ещё не получено: откройте место через durak_connect.', { lobby: null });
    return ok(JSON.stringify(known, null, 2), known);
  }
  const lines = [
    `Онлайн: ${lobby.online}`,
    `Столы: ${(lobby.rooms || []).map(room => `${room.id} «${room.name}» (${room.status}, ${room.players?.length || 0} игр., ставка ${room.bet})`).join('; ') || 'нет'}`,
    `История: ${(lobby.history || []).length} записей`,
    '',
    JSON.stringify(lobby, null, 2),
  ];
  return ok(lines.join('\n'), lobby);
}

// ── autoplay ────────────────────────────────────────────────────────────────

function sleep(ms) {
  return new Promise(resolve => setTimeout(resolve, ms));
}

function logStep(log, seat, action, detail) {
  log.push({ seat: seat.label, action, detail });
  if (log.length > 40) log.shift();
}

async function autoplay(args) {
  const count = Math.max(2, Math.min(MAX_PLAYERS, Number(args.seats) || 2));
  let credentials;
  try { credentials = args.credentials ?? JSON.parse(process.env.DURAK_ACCOUNTS || 'null'); } catch { throw new Error('DURAK_ACCOUNTS должен содержать JSON-массив сессий'); }
  if ((!Array.isArray(credentials) || credentials.length < count) && ADMIN_KEY) {
    credentials = [];
    for (let index = 0; index < count; index += 1) {
      const name = `bot-${Math.random().toString(36).slice(2, 10)}`;
      const password = `pw-${Math.random().toString(36).slice(2, 12)}`;
      const created = await api('/api/admin', { key: ADMIN_KEY, op: 'create_temp', name, password });
      if (!created.ok) throw new Error(`Не удалось создать аккаунт бота: ${created.data.error || `HTTP ${created.status}`}`);
      const session = await api('/api/auth/temp/login', { name, password });
      if (!session.ok) throw new Error(`Не удалось войти ботом: ${session.data.error || `HTTP ${session.status}`}`);
      credentials.push(session.data);
    }
  }
  if (!Array.isArray(credentials) || credentials.length < count || credentials.slice(0, count).some(c => !c || typeof c.clientId !== 'string' || !c.clientId.trim() || typeof c.token !== 'string' || !c.token.trim()) || new Set(credentials.slice(0, count).map(c => c.clientId)).size !== count || new Set(credentials.slice(0, count).map(c => c.token)).size !== count) throw new Error('Для autoplay нужны отдельные сессии: credentials, DURAK_ACCOUNTS или ключ администратора, чтобы создать клубные аккаунты');
  const maxSteps = Math.max(10, Math.min(2000, Number(args.maxSteps) || 300));
  const bet = BETS.includes(Number(args.bet)) ? Number(args.bet) : 0;
  const mode = args.mode === 'transfer' ? 'transfer' : 'classic';
  const deckSize = Number(args.deckSize) === 52 ? 52 : 36;
  const base = String(args.name || 'Бот').slice(0, 16);
  const created = [];
  const log = [];
  try {
    for (let index = 0; index < count; index += 1) {
      let label = `auto${index + 1}`;
      if (seats.has(label)) label = `${label}_${Date.now() % 1000}`;
      const seat = new Seat({ label, name: `${base} ${index + 1}`.slice(0, 20), id: credentials[index].clientId, token: credentials[index].token });
      seats.set(label, seat);
      created.push(seat);
      await seat.open();
      await seat.waitFor(item => item.noroom || item.game || item.lobby, 5000);
    }
    const host = created[0];
    const room = await api('/api/rooms', {
      clientId: host.id,
      token: host.token,
      playerName: host.playerName,
      name: `MCP тест ${new Date().toISOString().slice(11, 19)}`,
      mode,
      deckSize,
      bet,
    });
    if (!room.ok) throw new Error(`Не удалось создать стол: ${room.data.error || `HTTP ${room.status}`}`);
    const code = room.data.code;
    for (const seat of created.slice(1)) {
      const joined = await api('/api/join', { clientId: seat.id, token: seat.token, playerName: seat.playerName, roomId: code });
      if (!joined.ok) throw new Error(`Не удалось посадить ${seat.label}: ${joined.data.error || `HTTP ${joined.status}`}`);
      await seat.waitFor(item => item.game && item.game.room.id === code, 3000);
    }
    await actAndWait(host, 'start');
    await Promise.all(created.map(seat => seat.waitFor(item => item.game && item.game.room.status === 'playing', 5000)));
    const baseline = new Map(created.map(seat => [seat.label, seat.gameSeq]));

    let steps = 0;
    let idle = 0;
    let finalState = null;
    while (steps < maxSteps) {
      const states = created.map(seat => seat.game).filter(state => state && state.room.id === code);
      const finished = states.find(state => state.finished || state.room.status === 'finished');
      if (finished) {
        finalState = finished;
        await Promise.all(created.map(seat => seat.waitFor(item => (item.account?.games || 0) >= 1, 1500)));
        break;
      }
      if (!states.length) {
        await sleep(100);
        if (++idle > 40) break;
        continue;
      }
      const moved = await botStep(created, baseline, steps, log);
      if (moved) {
        steps += 1;
        idle = 0;
      } else {
        await sleep(50);
        if (++idle > 40) break;
      }
    }
    if (!finalState) finalState = created.map(seat => seat.game).find(state => state && state.room.id === code) || null;
    const finishedOk = Boolean(finalState && (finalState.finished || finalState.room.status === 'finished'));
    const lines = [
      `Стол ${code}: ${finishedOk ? 'партия завершена' : 'партия не завершена'}`,
      `Действий: ${steps}${finishedOk ? '' : ` (лимит ${maxSteps})`}`,
      finalState?.finished ? `Дурак: ${finalState.finished.loser || '—'}` : 'Дурак: —',
      `Карты: в колоде ${finalState?.deck ?? '—'}, на столе ${finalState?.table?.length ?? 0}, в руках ${(finalState?.players || []).reduce((sum, player) => sum + player.cards, 0)}`,
      `Балансы: ${created.map(seat => `${seat.label} ${seat.account?.balance ?? '—'}`).join(', ')}`,
      '',
      'Последние действия:',
      ...log.slice(-12).map(item => `  ${item.seat}: ${item.action}${item.detail ? ` ${item.detail}` : ''}`),
    ];
    return ok(lines.join('\n'), {
      code,
      seats: created.map(seat => seat.label),
      steps,
      finished: finishedOk,
      loser: finalState?.finished?.loser || null,
      deck: finalState?.deck ?? null,
      table: finalState?.table?.length ?? 0,
      hands: (finalState?.players || []).reduce((sum, player) => sum + player.cards, 0),
      balances: created.map(seat => ({ seat: seat.label, balance: seat.account?.balance ?? null, payout: seat.game?.payout ?? null })),
      actions: log,
      state: finalState,
    });
  } finally {
    if (!args.keepSeats) {
      for (const seat of created) {
        seat.close();
        seats.delete(seat.label);
      }
    }
  }
}

async function botStep(created, baseline, steps, log) {
  const behind = created.some(seat => seat.gameSeq < (baseline.get(seat.label) || 0) + steps);
  if (behind) {
    await sleep(25);
    return false;
  }
  const snapshot = created.map(seat => seat.game).find(state => state && state.room.status === 'playing');
  if (!snapshot) return false;
  const seatById = id => created.find(seat => seat.id === id);
  const ordered = [...created].sort((a, b) => Number(b.id === snapshot.turnId) - Number(a.id === snapshot.turnId));
  for (const seat of ordered) {
    const move = myMove(seat, seat.game);
    if (!move) continue;
    await actAndWait(seat, move.action, move.extra);
    logStep(log, seat, move.action, move.note);
    return true;
  }
  const defenderSeat = seatById(snapshot.defenderId);
  const defenderState = defenderSeat?.game;
  if (defenderState?.canTake && snapshot.table.some(row => !row.defense)) {
    await actAndWait(defenderSeat, 'take');
    logStep(log, defenderSeat, 'take', 'нечем отбиться');
    return true;
  }
  return false;
}

// ── MCP plumbing ────────────────────────────────────────────────────────────

const HANDLERS = {
  durak_connect: connectSeat,
  durak_disconnect: disconnectSeat,
  durak_seats: async () => {
    const list = [...seats.values()].map(seatSummary);
    return ok(list.length ? JSON.stringify(list, null, 2) : 'Мест нет. Вызовите durak_connect.', { seats: list });
  },
  durak_lobby: async args => lobbyView(args),
  durak_room_create: roomCreate,
  durak_room_join: roomJoin,
  durak_room_leave: roomLeave,
  durak_start: startGame,
  durak_action: gameAction,
  durak_state: async args => gameState(args),
  durak_wait: waitEvent,
  durak_watch: watchTurn,
  durak_follow: follow,
  durak_hints: async args => hints(args),
  durak_turn: async args => turn(args),
  durak_profile: updateProfile,
  durak_admin: adminOp,
  durak_status: async () => status(),
  durak_autoplay: autoplay,
};

function send(message) {
  process.stdout.write(`${JSON.stringify(message)}\n`);
}

function reply(id, result) {
  send({ jsonrpc: '2.0', id, result });
}

function replyError(id, code, message) {
  send({ jsonrpc: '2.0', id, error: { code, message } });
}

async function handle(message) {
  if (!message || message.jsonrpc !== '2.0') return;
  const { id, method } = message;
  const isRequest = id !== undefined && id !== null;

  if (method === 'initialize') {
    const requested = message.params?.protocolVersion;
    reply(id, {
      protocolVersion: typeof requested === 'string' ? requested : '2024-11-05',
      capabilities: { tools: { listChanged: false } },
      serverInfo: { name: SERVER_NAME, version: SERVER_VERSION },
      instructions: [
        'MCP-сервер игры «Дурак». Сначала durak_connect (SSE обязателен для ходов),',
        'затем durak_room_create/join, durak_start, durak_action, durak_hints для подсказок.',
        'Для игры без вмешательства человека вызывайте durak_follow: он сам играет своим',
        'местом до конца партии через серверный хук /api/watch и возвращает краткий отчёт.',
        'durak_watch и durak_wait нужны для точечного ожидания одного события.',
        'durak_autoplay прогоняет полную партию ботов и проверяет сервис целиком.',
      ].join(' '),
    });
    return;
  }

  if (method === 'notifications/initialized' || method === 'notifications/cancelled') return;
  if (method === 'ping') {
    if (isRequest) reply(id, {});
    return;
  }
  if (method === 'tools/list') {
    reply(id, { tools: TOOLS });
    return;
  }
  if (method === 'resources/list') {
    reply(id, { resources: [] });
    return;
  }
  if (method === 'prompts/list') {
    reply(id, { prompts: [] });
    return;
  }
  if (method === 'tools/call') {
    const name = message.params?.name;
    const handler = HANDLERS[name];
    if (!handler) {
      replyError(id, -32601, `Неизвестный инструмент: ${name}`);
      return;
    }
    try {
      reply(id, await handler(message.params?.arguments || {}));
    } catch (error) {
      reply(id, fail(error.message || String(error)));
    }
    return;
  }

  if (isRequest) replyError(id, -32601, `Метод не поддерживается: ${method}`);
}

const input = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });

const ready = restoreSessions().catch(error => {
  process.stderr.write(`durak-mcp: не удалось восстановить места: ${error.message}\n`);
});

input.on('line', line => {
  const trimmed = line.trim();
  if (!trimmed) return;
  let message;
  try {
    message = JSON.parse(trimmed);
  } catch (error) {
    send({ jsonrpc: '2.0', id: null, error: { code: -32700, message: 'Некорректный JSON' } });
    return;
  }
  ready
    .then(() => handle(message))
    .catch(error => {
      process.stderr.write(`durak-mcp: ${error.stack || error.message}\n`);
    });
});

input.on('close', () => {
  for (const seat of seats.values()) seat.close();
  process.exit(0);
});

process.stderr.write(`durak-mcp ${SERVER_VERSION}: ${BASE_URL}\n`);