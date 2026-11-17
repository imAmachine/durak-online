'use strict';

const http = require('http');
const fs = require('fs');
const path = require('path');
const crypto = require('crypto');

const PORT = Number(process.env.PORT) || 4173;
const HOST = process.env.HOST || '0.0.0.0';
const clients = new Map();
const sessions = new Map();
const rooms = new Map();
const history = [];
const accounts = new Map();

const SUITS = ['♠', '♥', '♦', '♣'];
const RANKS_36 = ['6', '7', '8', '9', '10', 'В', 'Д', 'К', 'Т'];
const RANKS_52 = ['2', '3', '4', '5', '6', '7', '8', '9', '10', 'В', 'Д', 'К', 'Т'];
const VALUES = new Map(RANKS_52.map((rank, index) => [rank, index]));
const MAX_TABLE_CARDS = 6;
const MAX_PLAYERS = 6;
const START_BALANCE = 1000;
const BETS = [0, 50, 100, 250, 500];
const ADMIN_KEY = (process.env.ADMIN_KEY || '').trim();

function buildDeck(size) {
  const ranks = size === 52 ? RANKS_52 : RANKS_36;
  return SUITS.flatMap(suit => ranks.map(rank => ({ id: rank + suit, rank, suit })));
}

function shuffle(list) {
  const deck = [...list];
  for (let index = deck.length - 1; index > 0; index -= 1) {
    const swap = Math.floor(Math.random() * (index + 1));
    [deck[index], deck[swap]] = [deck[swap], deck[index]];
  }
  return deck;
}

function syncTrumpCard(room) {
  if (!room.deck.length) room.trumpCard = null;
}

function findPlayer(room, id) {
  return room.players.find(player => player.id === id) || null;
}

function accountOf(id) {
  let account = accounts.get(id);
  if (!account) {
    account = { id, name: 'Игрок', balance: START_BALANCE, games: 0, wins: 0, losses: 0, createdAt: Date.now(), activeAt: Date.now() };
    accounts.set(id, account);
  }
  return account;
}

function publicAccount(account) {
  return {
    id: account.id,
    name: account.name,
    balance: account.balance,
    games: account.games,
    wins: account.wins,
    losses: account.losses,
    online: clients.has(account.id),
  };
}

function renameAccount(id, name) {
  const account = accountOf(id);
  account.name = name;
  account.activeAt = Date.now();
}

function roomOfPlayer(id) {
  return [...rooms.values()].find(room => room.players.some(player => player.id === id && !player.left)) || null;
}

function pushAccount(id) {
  const client = clients.get(id);
  if (!client) return;
  send(client, { type: 'account', account: publicAccount(accountOf(id)) });
}

function nextActive(room, player) {
  if (!player) return null;
  const index = room.players.indexOf(player);
  if (index === -1) return null;
  for (let step = 1; step <= room.players.length; step += 1) {
    const candidate = room.players[(index + step) % room.players.length];
    if (!candidate.finished && !candidate.left) return candidate;
  }
  return null;
}

function canBeat(attack, defense, trump) {
  if (attack.suit === defense.suit) return VALUES.get(defense.rank) > VALUES.get(attack.rank);
  return defense.suit === trump && attack.suit !== trump;
}

function tableRanks(room) {
  const ranks = new Set();
  room.table.forEach(row => {
    ranks.add(row.attack.rank);
    if (row.defense) ranks.add(row.defense.rank);
  });
  return ranks;
}

function createRoom({ name, mode, deckSize, bet, host }) {
  let id = crypto.randomBytes(3).toString('hex').toUpperCase();
  while (rooms.has(id)) id = crypto.randomBytes(3).toString('hex').toUpperCase();
  const stake = Number(bet);
  const room = {
    id,
    name: String(name || 'Новая партия').slice(0, 40),
    mode: mode === 'transfer' ? 'transfer' : 'classic',
    deckSize: Number(deckSize) === 52 ? 52 : 36,
    bet: BETS.includes(stake) ? stake : 0,
    pot: 0,
    lastPayouts: null,
    hostId: host.id,
    players: [host],
    status: 'waiting',
    deck: [],
    trump: null,
    trumpCard: null,
    table: [],
    discard: [],
    trickAttackerId: null,
    finishedAt: null,
    attackerId: null,
    defenderId: null,
    turnId: null,
    takePending: null,
    message: 'Ожидаем игроков. Отправьте друзьям код приглашения',
    finished: null,
    createdAt: Date.now(),
    seq: 0,
    events: [],
    hooks: new Set(),
    nextEvent: null,
  };
  rooms.set(id, room);
  return room;
}

function startRound(room) {
  const deck = shuffle(buildDeck(room.deckSize));
  room.deck = deck;
  room.trumpCard = deck[0];
  room.trump = deck[0].suit;
  room.table = [];
  room.discard = [];
  room.finishedAt = null;
  room.takePending = null;
  room.status = 'playing';
  room.finished = null;
  room.pot = 0;
  room.lastPayouts = null;
  room.players.forEach(player => {
    player.finished = false;
    player.left = false;
    player.hand = [];
  });
  for (let round = 0; round < 6; round += 1) {
    room.players.forEach(player => {
      const card = room.deck.pop();
      if (card) player.hand.push(card);
    });
  }
  syncTrumpCard(room);
  const held = room.players.flatMap(player => player.hand.map(card => ({ player, card })));
  const trumps = held.filter(entry => entry.card.suit === room.trump);
  const candidates = trumps.length ? trumps : held;
  const attacker = candidates.reduce((lowest, entry) => VALUES.get(entry.card.rank) < VALUES.get(lowest.card.rank) ? entry : lowest).player;
  const defender = nextActive(room, attacker);
  room.attackerId = attacker.id;
  room.trickAttackerId = attacker.id;
  room.defenderId = defender.id;
  room.turnId = attacker.id;
  room.message = `${attacker.name} атакует. ${defender.name} отбивается. Козырь ${room.trump}`;
}

function stakeShortfall(room, players) {
  const bet = room.bet || 0;
  if (!bet) return null;
  const poor = players.find(player => accountOf(player.id).balance < bet);
  return poor ? `У игрока «${poor.name}» не хватает фишек на ставку ${bet}` : null;
}

function chargeStakes(room) {
  const bet = room.bet || 0;
  if (!bet) return;
  room.players.forEach(player => {
    accountOf(player.id).balance -= bet;
  });
  room.pot = bet * room.players.length;
}

function settleAccounts(room, loserId) {
  const bet = room.bet || 0;
  const winners = room.players.filter(player => player.id !== loserId);
  room.players.forEach(player => {
    const account = accountOf(player.id);
    account.games += 1;
    if (player.id === loserId) account.losses += 1;
    else if (loserId) account.wins += 1;
  });
  room.lastPayouts = {};
  if (bet && room.pot > 0) {
    if (winners.length) {
      const share = Math.floor(room.pot / winners.length);
      let remainder = room.pot - share * winners.length;
      winners.forEach(player => {
        const extra = remainder > 0 ? 1 : 0;
        remainder -= extra;
        const amount = share + extra;
        accountOf(player.id).balance += amount;
        room.lastPayouts[player.id] = amount - bet;
      });
    }
    if (loserId) room.lastPayouts[loserId] = -bet;
  }
  room.pot = 0;
  room.players.forEach(player => pushAccount(player.id));
}

function finishGame(room, loser, message) {
  if (room.status === 'finished') return;
  room.finishedAt = Date.now();
  room.status = 'finished';
  room.finished = { loser: loser ? loser.name : '—', loserId: loser ? loser.id : null, draw: !loser };
  room.message = message || (loser ? `${loser.name} — дурак, партия окончена` : 'Ничья. Ставки возвращены');
  history.push({ room: room.name, mode: room.mode, at: Date.now(), loser: room.finished.loser, bet: room.bet || 0 });
  settleAccounts(room, room.finished.loserId);
}

function refillHands(room) {
  const attacker = findPlayer(room, room.trickAttackerId || room.attackerId);
  const defender = findPlayer(room, room.defenderId);
  const order = [];
  let cursor = attacker;
  for (let step = 0; step < room.players.length && cursor; step += 1) {
    if (!cursor.finished && !cursor.left && !order.includes(cursor) && cursor !== defender) order.push(cursor);
    cursor = nextActive(room, cursor);
  }
  if (defender && !defender.finished && !defender.left && !order.includes(defender)) order.push(defender);
  order.forEach(player => {
    while (player.hand.length < 6 && room.deck.length) player.hand.push(room.deck.pop());
  });
  syncTrumpCard(room);
}

function normalizeRoles(room) {
  let attacker = findPlayer(room, room.attackerId);
  let defender = findPlayer(room, room.defenderId);
  if (!attacker || attacker.finished) {
    const replacement = attacker ? nextActive(room, attacker) : room.players.find(player => !player.finished);
    if (!replacement) return;
    room.attackerId = replacement.id;
    attacker = replacement;
    const nextDefender = nextActive(room, attacker);
    room.defenderId = nextDefender ? nextDefender.id : null;
    room.turnId = attacker.id;
    defender = nextDefender;
  }
  if (!defender || defender.finished) {
    const nextDefender = nextActive(room, attacker);
    room.defenderId = nextDefender ? nextDefender.id : null;
  }
  const turnPlayer = findPlayer(room, room.turnId);
  if (!turnPlayer || turnPlayer.finished) room.turnId = room.attackerId;
}

function settle(room) {
  if (room.deck.length === 0) {
    room.players.forEach(player => {
      if (!player.finished && player.hand.length === 0) player.finished = true;
    });
  }
  const active = room.players.filter(player => !player.finished && !player.left);
  if (active.length <= 1) {
    const durak = active[0] || null;
    finishGame(room, durak, durak ? `${durak.name} — дурак, партия окончена` : 'Ничья. Ставки возвращены');
    return;
  }
  normalizeRoles(room);
}

function updateTurn(room) {
  if (!room.players.length) return;
  if (room.takePending) {
    room.turnId = room.attackerId;
    return;
  }
  if (!room.table.length) {
    room.turnId = room.attackerId;
    return;
  }
  room.turnId = room.table.some(row => !row.defense) ? room.defenderId : room.attackerId;
}

function collectTable(room, defender) {
  room.table.forEach(row => {
    defender.hand.push(row.attack);
    if (row.defense) defender.hand.push(row.defense);
  });
  room.table = [];
  room.takePending = null;
  refillHands(room);
  const nextAttacker = nextActive(room, defender);
  if (!nextAttacker) return { error: 'Другой игрок уже вышел' };
  room.attackerId = nextAttacker.id;
  const nextDefender = nextActive(room, nextAttacker);
  room.defenderId = nextDefender ? nextDefender.id : null;
  room.message = `${defender.name} забрал карты. Ход ${nextAttacker.name}`;
  settle(room);
  room.trickAttackerId = room.attackerId;
  updateTurn(room);
  broadcastRoom(room);
  return { ok: true };
}

function gameState(room, clientId) {
  const me = findPlayer(room, clientId);
  const attacker = findPlayer(room, room.attackerId);
  const defender = findPlayer(room, room.defenderId);
  const turn = findPlayer(room, room.turnId);
  const allRanks = tableRanks(room);
  const attackRanks = new Set(room.table.map(row => row.attack.rank));
  const allUnbeaten = room.table.every(row => !row.defense);
  const hasUnbeaten = room.table.some(row => !row.defense);
  const unbeatenRows = room.table.filter(row => !row.defense).length;
  const playing = room.status === 'playing';
  const isAttacking = Boolean(me && me.id === room.attackerId);
  const isDefending = Boolean(me && me.id === room.defenderId);
  const eligible = Boolean(me && !me.finished && !me.left && me.hand.length);
  const nextDefender = me ? nextActive(room, me) : null;
  const canAttack = Boolean(
    eligible && playing && !isDefending && (isAttacking || room.table.length > 0) && defender && defender.hand.length > unbeatenRows &&
    room.table.length < MAX_TABLE_CARDS &&
    (room.table.length === 0 || me.hand.some(card => allRanks.has(card.rank)))
  );
  const canDefend = Boolean(me && playing && isDefending && !room.takePending && hasUnbeaten);
  const canTake = Boolean(me && playing && isDefending && room.table.length > 0 && !room.takePending && hasUnbeaten);
  const canPass = Boolean(me && playing && isAttacking && room.table.length > 0 && (room.takePending || !hasUnbeaten));
  const canTransfer = Boolean(
    eligible && playing && room.mode === 'transfer' && isDefending && !room.takePending &&
    room.table.length > 0 && room.table.length < MAX_TABLE_CARDS && allUnbeaten && attackRanks.size === 1 &&
    nextDefender && nextDefender !== me && nextDefender.hand.length >= room.table.length + 1 &&
    me.hand.some(card => card.rank === [...attackRanks][0])
  );
  const canPlay = canAttack || canDefend;
  const connected = room.players.filter(player => player.connected && !player.left).length;
  return {
    room: {
      id: room.id,
      name: room.name,
      mode: room.mode,
      deckSize: room.deckSize,
      bet: room.bet || 0,
      status: room.status,
      hostId: room.hostId,
    },
    players: room.players.map((player, seat) => ({
      id: player.id,
      name: player.name,
      seat,
      cards: player.hand.length,
      connected: player.connected,
      finished: player.finished,
      left: Boolean(player.left),
      role: player.id === room.attackerId ? 'attacker' : player.id === room.defenderId ? 'defender' : 'idle',
    })),
    you: me
      ? {
          id: me.id,
          name: me.name,
          finished: me.finished,
          left: Boolean(me.left),
          role: isAttacking ? 'attacker' : isDefending ? 'defender' : 'idle',
          turn: canPlay,
          canPlay,
          hand: me.hand,
        }
      : null,
    hand: me ? me.hand : [],
    trump: room.trump,
    trumpCard: room.trumpCard,
    deck: room.deck.length,
    table: room.table.map(row => ({ attack: row.attack, defense: row.defense })),
    attackerId: room.attackerId,
    defenderId: room.defenderId,
    turnId: room.turnId,
    attackerName: attacker ? attacker.name : '',
    defenderName: defender ? defender.name : '',
    turnName: turn ? turn.name : '',
    message: room.message,
    canAttack,
    canDefend,
    canTransfer,
    canPass,
    canTake,
    takePending: Boolean(room.takePending),
    canStart: Boolean(me && !me.left && room.hostId === me.id && room.status !== 'playing' && connected >= 2),
    payout: room.lastPayouts && me ? room.lastPayouts[me.id] || 0 : 0,
    finished: room.finished,
    seq: room.seq || 0,
    log: (room.events || []).slice(-8),
  };
}

function roomSummary(room) {
  const host = findPlayer(room, room.hostId);
  return {
    id: room.id,
    name: room.name,
    mode: room.mode,
    deckSize: room.deckSize,
    bet: room.bet || 0,
    status: room.status,
    host: host ? host.name : '—',
    players: room.players.filter(player => player.connected).map(player => ({ id: player.id, name: player.name, cards: player.hand.length, connected: player.connected })),
    seats: MAX_PLAYERS,
    createdAt: room.createdAt,
  };
}

function send(client, payload) {
  try {
    client.res.write(`data: ${JSON.stringify(payload)}\n\n`);
  } catch (error) {
    clients.delete(client.id);
  }
}

function broadcastLobby() {
  const payload = {
    type: 'lobby',
    online: clients.size,
    rooms: [...rooms.values()].filter(room => room.status !== 'finished' && room.players.some(player => clients.has(player.id))).map(roomSummary),
    history: history.slice(-8).reverse(),
  };
  clients.forEach(client => send(client, payload));
}

function markEvent(room, actorId, action, data) {
  room.nextEvent = {
    actorId: actorId || null,
    action: action || 'sync',
    cardId: data && data.cardId ? String(data.cardId) : null,
    targetCardId: data && data.targetCardId ? String(data.targetCardId) : null,
  };
}

function recordEvent(room) {
  const pending = room.nextEvent || { action: 'sync' };
  const actor = pending.actorId ? findPlayer(room, pending.actorId) : null;
  room.nextEvent = null;
  room.seq = (room.seq || 0) + 1;
  room.events.push({
    seq: room.seq,
    at: Date.now(),
    action: pending.action,
    actorId: pending.actorId || null,
    actorName: actor ? actor.name : '',
    cardId: pending.cardId || null,
    targetCardId: pending.targetCardId || null,
    status: room.status,
    turnId: room.turnId || null,
    attackerId: room.attackerId || null,
    defenderId: room.defenderId || null,
    message: room.message,
  });
  if (room.events.length > 40) room.events.shift();
}

const WATCH_MODES = ['any', 'game', 'your-turn', 'opponent', 'finished'];

function matchesWatch(until, room, clientId, since = 0) {
  if (until === 'your-turn') {
    const state = gameState(room, clientId);
    return Boolean(state.you && (state.canAttack || state.canDefend || state.canTransfer || state.canTake || state.canPass || state.canStart));
  }
  if (until === 'finished') return room.status === 'finished' || Boolean(gameState(room, clientId).finished);
  if (until === 'opponent') {
    return room.events.some(event => event.seq > since && event.actorId && event.actorId !== clientId);
  }
  return true;
}

function releaseHooks(room) {
  if (!room.hooks || !room.hooks.size) return;
  [...room.hooks].forEach(hook => {
    try { hook(); } catch (error) { room.hooks.delete(hook); }
  });
}

function broadcastRoom(room) {
  recordEvent(room);
  room.players.forEach(player => {
    const client = clients.get(player.id);
    if (!client || client.roomId !== room.id || player.left) return;
    send(client, { type: 'game', state: gameState(room, player.id) });
  });
  broadcastLobby();
  releaseHooks(room);
}

function leaveRoom(client) {
  const room = rooms.get(client.roomId);
  client.roomId = null;
  if (!room) {
    broadcastLobby();
    return;
  }
  const player = findPlayer(room, client.id);
  if (!player) {
    broadcastLobby();
    return;
  }
  if (room.status === 'playing') {
    player.connected = false;
    player.left = true;
    player.offlineAt = Date.now();
    if (room.hostId === player.id) {
      const host = room.players.find(item => !item.left && item.connected);
      if (host) room.hostId = host.id;
    }
    if (!player.finished) finishGame(room, player, `${player.name} покинул партию, игра окончена`);
    markEvent(room, player.id, 'leave');
    broadcastRoom(room);
    return;
  }
  room.players = room.players.filter(item => item.id !== client.id);
  if (!room.players.length) {
    rooms.delete(room.id);
    broadcastLobby();
    return;
  }
  if (room.hostId === client.id) room.hostId = room.players[0].id;
  room.message = `${player.name} покинул лобби`;
  markEvent(room, player.id, 'leave');
  broadcastRoom(room);
}

function act(client, data) {
  const room = rooms.get(client.roomId);
  const result = actInner(client, data);
  if (result && result.error && room) room.nextEvent = null;
  return result;
}

function actInner(client, data) {
  const room = rooms.get(client.roomId);
  if (!room) return { error: 'Вы не находитесь в лобби' };
  const me = findPlayer(room, client.id);
  if (!me || me.left) return { error: 'Вы не участник этой игры' };
  const action = data.action;
  markEvent(room, me.id, action, data);

  if (action === 'start') {
    if (room.status === 'playing') return { error: 'Партия уже идёт' };
    if (room.hostId !== me.id) return { error: 'Игру начинает создатель лобби' };
    const ready = room.players.filter(player => player.connected && !player.left);
    if (ready.length < 2) return { error: 'Нужно минимум два игрока' };
    const shortfall = stakeShortfall(room, ready);
    if (shortfall) return { error: shortfall };
    room.players = ready;
    startRound(room);
    chargeStakes(room);
    if (room.bet) {
      room.message += `. Банк ${room.pot} фишек`;
      room.players.forEach(player => pushAccount(player.id));
    }
    broadcastRoom(room);
    return { ok: true };
  }

  if (room.status !== 'playing') return { error: 'Игра ещё не идёт' };
  const attacker = findPlayer(room, room.attackerId);
  const defender = findPlayer(room, room.defenderId);

  const isAttacker = me.id === room.attackerId;
  const isDefender = me.id === room.defenderId;

  if (me.finished) return { error: 'Вы уже закончили игру' };

  if (action === 'attack') {
    if (isDefender || (!isAttacker && !room.table.length)) return { error: 'Открыть ход может только атакующий, подкидывать — все кроме защищающегося' };
    const card = me.hand.find(item => item.id === data.cardId);
    if (!card) return { error: 'Такой карты нет в руке' };
    if (!defender || defender.hand.length <= room.table.filter(row => !row.defense).length) return { error: 'У защищающегося недостаточно карт' };
    if (room.table.length >= MAX_TABLE_CARDS) return { error: 'Больше шести карт за ход не подкидывают' };
    if (room.table.length && !tableRanks(room).has(card.rank)) {
      return { error: 'Подкидывать можно только карты достоинства, которое уже есть на столе' };
    }
    me.hand = me.hand.filter(item => item.id !== card.id);
    room.table.push({ attack: card, defense: null });
    room.message = room.takePending
      ? `${me.name} подкидывает ${card.rank}${card.suit}. ${defender.name}, забирайте карты`
      : `${me.name} ходит ${card.rank}${card.suit}. ${defender.name}, отбивайтесь`;
    updateTurn(room);
    broadcastRoom(room);
    return { ok: true };
  }

  if (action === 'defend') {
    if (!isDefender) return { error: 'Отбиваться может только защищающийся' };
    if (room.takePending) return { error: 'Вы уже решили взять карты' };
    const row = room.table.find(item => !item.defense && (!data.targetCardId || item.attack.id === data.targetCardId));
    if (!row) return { error: 'Выберите неотбитую карту для защиты' };
    const card = me.hand.find(item => item.id === data.cardId);
    if (!card) return { error: 'Такой карты нет в руке' };
    if (!canBeat(row.attack, card, room.trump)) return { error: `Карта ${card.rank}${card.suit} не бьёт ${row.attack.rank}${row.attack.suit}` };
    me.hand = me.hand.filter(item => item.id !== card.id);
    row.defense = card;
    room.message = room.table.some(item => !item.defense)
      ? `${me.name} отбивает карту. Остались неотбитые карты`
      : `${me.name} отбился. ${attacker.name}, подкидывайте или говорите «Бито»`;
    updateTurn(room);
    broadcastRoom(room);
    return { ok: true };
  }

  if (action === 'transfer') {
    if (room.mode !== 'transfer') return { error: 'Перевод доступен только в переводном режиме' };
    if (!isDefender) return { error: 'Переводить может только защищающийся' };
    if (room.takePending) return { error: 'Вы уже решили взять карты' };
    const ranks = new Set(room.table.map(row => row.attack.rank));
    if (!room.table.length || room.table.some(row => row.defense) || ranks.size !== 1) {
      return { error: 'Переводить можно, пока карты на столе не отбиты' };
    }
    const rank = [...ranks][0];
    const card = me.hand.find(item => item.id === data.cardId && item.rank === rank);
    if (!card) return { error: `Нужна карта достоинства ${rank}` };
    const nextDefender = nextActive(room, me);
    if (!nextDefender || nextDefender === me) return { error: 'Переводить больше некому' };
    if (room.table.length >= MAX_TABLE_CARDS || nextDefender.hand.length < room.table.length + 1) return { error: 'У следующего игрока недостаточно карт для перевода' };
    me.hand = me.hand.filter(item => item.id !== card.id);
    room.table.push({ attack: card, defense: null });
    room.attackerId = me.id;
    room.defenderId = nextDefender.id;
    room.message = `${me.name} перевёл ход картой ${card.rank}${card.suit}. ${nextDefender.name}, отбивайтесь`;
    updateTurn(room);
    broadcastRoom(room);
    return { ok: true };
  }

  if (action === 'take') {
    if (!isDefender) return { error: 'Взять карты может только защищающийся' };
    if (!room.table.length) return { error: 'На столе нет карт' };
    if (!room.takePending && !room.table.some(row => !row.defense)) {
      return { error: 'Все карты отбиты: забирать нечего' };
    }
    if (!room.takePending) {
      room.takePending = me.id;
      room.message = `${me.name} берёт карты. ${attacker.name}, можно подкинуть ещё`;
      updateTurn(room);
      broadcastRoom(room);
      return { ok: true };
    }
    return { error: 'Дождитесь решения атакующего' };
  }

  if (action === 'pass') {
    if (!isAttacker) return { error: 'Закончить ход может атакующий' };
    if (!room.table.length) return { error: 'Стол пуст' };
    if (room.takePending) return collectTable(room, defender);
    if (room.table.some(row => !row.defense)) return { error: 'Сначала отбейте все карты' };
    room.discard.push(...room.table.flatMap(row => [row.attack, row.defense]));
    room.table = [];
    refillHands(room);
    const newAttacker = defender;
    const newDefender = nextActive(room, newAttacker);
    room.attackerId = newAttacker.id;
    room.defenderId = newDefender ? newDefender.id : null;
    room.message = `Бито. ${newAttacker.name} атакует${newDefender ? `, ${newDefender.name} отбивается` : ''}`;
    settle(room);
    room.trickAttackerId = room.attackerId;
    updateTurn(room);
    broadcastRoom(room);
    return { ok: true };
  }

  return { error: 'Неизвестное действие' };
}

function readBody(req) {
  return new Promise(resolve => {
    let body = '';
    req.on('data', chunk => {
      body += chunk;
      if (body.length > 1e6) req.destroy();
    });
    req.on('end', () => {
      try {
        resolve(JSON.parse(body || '{}'));
      } catch (error) {
        resolve({});
      }
    });
  });
}

function json(res, status, payload) {
  res.writeHead(status, { 'Content-Type': 'application/json' });
  res.end(JSON.stringify(payload));
}

const MIME = { '.html': 'text/html', '.js': 'application/javascript', '.css': 'text/css', '.svg': 'image/svg+xml' };

const server = http.createServer(async (req, res) => {
  const url = new URL(req.url, `http://${req.headers.host || 'localhost'}`);

  if (url.pathname === '/events') {
    const id = url.searchParams.get('id');
    const name = (url.searchParams.get('name') || 'Игрок').slice(0, 20);
    const token = url.searchParams.get('token');
    if (!id || !/^[a-f0-9]{64}$/.test(token || '')) {
      res.writeHead(400);
      res.end();
      return;
    }
    if (sessions.has(id) && sessions.get(id) !== token) {
      return json(res, 403, { error: 'Неверный токен сессии' });
    }
    const previous = clients.get(id);
    sessions.set(id, token);
    const client = { id, name, res, roomId: null };
    clients.set(id, client);
    if (previous) {
      try {
        previous.res.end();
      } catch (error) {}
    }
    renameAccount(id, name);
    res.writeHead(200, {
      'Content-Type': 'text/event-stream',
      'Cache-Control': 'no-cache, no-transform',
      Connection: 'keep-alive',
      'X-Accel-Buffering': 'no',
    });
    res.write('retry: 2000\n\n');
    send(client, { type: 'account', account: publicAccount(accountOf(id)) });
    const room = roomOfPlayer(id);
    if (room) {
      client.roomId = room.id;
      const player = findPlayer(room, id);
      if (player) {
        player.connected = true;
        player.name = name;
        player.offlineAt = null;
      }
      markEvent(room, id, 'connect');
      broadcastRoom(room);
    } else {
      send(client, { type: 'noroom' });
    }
    broadcastLobby();
    req.on('close', () => {
      if (clients.get(id) !== client) return;
      clients.delete(id);
      const current = roomOfPlayer(id);
      if (current) {
        const player = findPlayer(current, id);
        if (player) {
          player.connected = false;
          player.offlineAt = Date.now();
        }
        markEvent(current, id, 'disconnect');
        broadcastRoom(current);
      }
      broadcastLobby();
    });
    return;
  }

  if (url.pathname === '/api/watch') {
    const id = url.searchParams.get('id');
    const token = url.searchParams.get('token');
    if (!id || !/^[a-f0-9]{64}$/.test(token || '')) return json(res, 400, { error: 'Нужны id и токен сессии' });
    if (sessions.get(id) !== token) return json(res, 403, { error: 'Неверный токен сессии' });
    const until = url.searchParams.get('until') || 'any';
    if (!WATCH_MODES.includes(until)) return json(res, 400, { error: `until должен быть ${WATCH_MODES.join(', ')}` });
    const since = Math.max(0, Number(url.searchParams.get('since')) || 0);
    const timeout = Math.max(200, Math.min(60000, Number(url.searchParams.get('timeout')) || 20000));
    const room = roomOfPlayer(id);
    if (!room) return json(res, 200, { hit: false, until, since, seq: 0, roomId: null, events: [], state: null });
    if (!room.hooks) room.hooks = new Set();
    const payload = hit => ({
      hit,
      until,
      since,
      seq: room.seq,
      roomId: room.id,
      events: room.events.slice(-12),
      state: gameState(room, id),
    });
    const ready = () => room.seq > since && matchesWatch(until, room, id, since);
    if (ready()) return json(res, 200, payload(true));
    let settled = false;
    const stop = () => {
      settled = true;
      clearTimeout(timer);
      room.hooks.delete(hook);
    };
    const hook = () => {
      if (settled || !ready()) return;
      stop();
      if (!res.writableEnded) json(res, 200, payload(true));
    };
    const timer = setTimeout(() => {
      if (settled) return;
      stop();
      if (!res.writableEnded) json(res, 200, payload(false));
    }, timeout);
    timer.unref();
    room.hooks.add(hook);
    req.on('close', stop);
    return;
  }

  let requestBody;
  if (req.method === 'POST') {
    requestBody = await readBody(req);
    if (!requestBody || typeof requestBody !== 'object' || Array.isArray(requestBody)) {
      return json(res, 400, { error: 'Некорректный запрос' });
    }
    if (Object.hasOwn(requestBody, 'clientId') || ['/api/rooms', '/api/join', '/api/action', '/api/profile', '/api/leave'].includes(url.pathname)) {
      if (!sessions.has(requestBody.clientId) || sessions.get(requestBody.clientId) !== requestBody.token) {
        return json(res, 403, { error: 'Неверный токен сессии' });
      }
    }
  }

  if (req.method === 'POST' && url.pathname === '/api/rooms') {
    const body = requestBody;
    const client = clients.get(body.clientId);
    if (!client) return json(res, 400, { error: 'Нет соединения с сервером, обновите страницу' });
    if (client.roomId && rooms.has(client.roomId)) return json(res, 400, { error: 'Вы уже в лобби' });
    if (body.playerName) client.name = String(body.playerName).slice(0, 20);
    renameAccount(client.id, client.name);
    const stake = BETS.includes(Number(body.bet)) ? Number(body.bet) : 0;
    if (stake > accountOf(client.id).balance) {
      return json(res, 400, { error: `Недостаточно фишек: для ставки ${stake} нужен баланс не меньше ${stake}` });
    }
    const host = { id: client.id, name: client.name, hand: [], connected: true, finished: false };
    const room = createRoom({ name: body.name, mode: body.mode, deckSize: body.deckSize, bet: stake, host });
    client.roomId = room.id;
    markEvent(room, client.id, 'create');
    broadcastRoom(room);
    return json(res, 201, { room: roomSummary(room), code: room.id, account: publicAccount(accountOf(client.id)) });
  }

  if (req.method === 'POST' && url.pathname === '/api/join') {
    const body = requestBody;
    const client = clients.get(body.clientId);
    if (!client) return json(res, 400, { error: 'Нет соединения с сервером, обновите страницу' });
    const room = rooms.get(String(body.roomId || '').toUpperCase());
    if (!room) return json(res, 404, { error: 'Лобби с таким кодом не найдено' });
    const existing = findPlayer(room, client.id);
    if (existing && existing.left && room.status === 'playing') return json(res, 400, { error: 'Вы уже покинули эту партию' });
    if (!existing && room.status === 'playing') return json(res, 400, { error: 'Игра уже началась' });
    const now = Date.now();
    const available = room.players.filter(player => player.connected || now - (player.offlineAt ?? now) < 60 * 1000);
    if (!existing && available.length >= MAX_PLAYERS) return json(res, 400, { error: 'В лобби нет свободных мест' });
    if (!existing && room.bet && accountOf(client.id).balance < room.bet) {
      return json(res, 400, { error: `Недостаточно фишек: ставка за столом ${room.bet}` });
    }
    if (client.roomId && client.roomId !== room.id && rooms.has(client.roomId)) leaveRoom(client);
    if (body.playerName) client.name = String(body.playerName).slice(0, 20);
    renameAccount(client.id, client.name);
    if (existing) {
      existing.connected = true;
      existing.left = false;
      existing.offlineAt = null;
      existing.name = client.name;
      client.roomId = room.id;
      markEvent(room, client.id, 'join');
      broadcastRoom(room);
      return json(res, 200, { room: roomSummary(room), account: publicAccount(accountOf(client.id)) });
    }
    room.players = available;
    if (!findPlayer(room, room.hostId)) room.hostId = room.players[0]?.id || client.id;
    room.players.push({ id: client.id, name: client.name, hand: [], connected: true, finished: false });
    client.roomId = room.id;
    room.message = 'Игроки готовы. Создатель лобби может начать игру';
    markEvent(room, client.id, 'join');
    broadcastRoom(room);
    return json(res, 200, { room: roomSummary(room), account: publicAccount(accountOf(client.id)) });
  }

  if (req.method === 'POST' && url.pathname === '/api/action') {
    const body = requestBody;
    const client = clients.get(body.clientId);
    if (!client) return json(res, 400, { error: 'Нет соединения с сервером, обновите страницу' });
    const result = act(client, body);
    return json(res, result.error ? 400 : 200, result);
  }

  if (req.method === 'POST' && url.pathname === '/api/profile') {
    const body = requestBody;
    const client = clients.get(body.clientId);
    if (!client) return json(res, 400, { error: 'Нет соединения с сервером' });
    const name = String(body.name || '').trim().slice(0, 20);
    if (!name) return json(res, 400, { error: 'Имя не может быть пустым' });
    client.name = name;
    renameAccount(client.id, name);
    rooms.forEach(room => {
      const player = findPlayer(room, client.id);
      if (player) player.name = name;
    });
    const room = rooms.get(client.roomId);
    if (room) {
      markEvent(room, client.id, 'rename');
      broadcastRoom(room);
    } else broadcastLobby();
    return json(res, 200, { ok: true, name, account: publicAccount(accountOf(client.id)) });
  }

  if (req.method === 'POST' && url.pathname === '/api/admin') {
    const body = requestBody;
    if (!ADMIN_KEY) return json(res, 503, { error: 'Панель администратора отключена' });
    if (String(body.key || '') !== ADMIN_KEY) return json(res, 403, { error: 'Неверный ключ администратора' });
    const op = String(body.op || 'list');

    if (op === 'list') {
      const players = [...accounts.values()]
        .sort((a, b) => (Number(clients.has(b.id)) - Number(clients.has(a.id))) || b.activeAt - a.activeAt)
        .map(account => {
          const room = roomOfPlayer(account.id);
          return Object.assign(publicAccount(account), { room: room ? room.name : null, roomId: room ? room.id : null, activeAt: account.activeAt });
        });
      return json(res, 200, { ok: true, players });
    }

    const account = accounts.get(String(body.id || ''));
    if (!account) return json(res, 404, { error: 'Игрок не найден' });

    if (op === 'update') {
      if (body.name !== undefined) {
        const name = String(body.name).trim().slice(0, 20);
        if (!name) return json(res, 400, { error: 'Имя не может быть пустым' });
        account.name = name;
        const client = clients.get(account.id);
        if (client) client.name = name;
        rooms.forEach(room => {
          const player = findPlayer(room, account.id);
          if (player) player.name = name;
        });
      }
      const asCount = (value, fallback) => {
        const number = Math.floor(Number(value));
        return Number.isFinite(number) ? Math.max(0, Math.min(1e9, number)) : fallback;
      };
      if (body.balance !== undefined) account.balance = asCount(body.balance, account.balance);
      if (body.games !== undefined) account.games = asCount(body.games, account.games);
      if (body.wins !== undefined) account.wins = asCount(body.wins, account.wins);
      if (body.losses !== undefined) account.losses = asCount(body.losses, account.losses);
      account.activeAt = Date.now();
      const room = roomOfPlayer(account.id);
      if (room) {
        markEvent(room, account.id, 'admin');
        broadcastRoom(room);
      } else broadcastLobby();
      pushAccount(account.id);
      return json(res, 200, { ok: true, player: publicAccount(account) });
    }

    if (op === 'reset') {
      account.balance = START_BALANCE;
      account.games = 0;
      account.wins = 0;
      account.losses = 0;
      account.activeAt = Date.now();
      pushAccount(account.id);
      return json(res, 200, { ok: true, player: publicAccount(account) });
    }

    if (op === 'balance') {
      const delta = Math.floor(Number(body.delta));
      if (!Number.isFinite(delta) || !delta) return json(res, 400, { error: 'Укажите сумму пополнения' });
      account.balance = Math.max(0, Math.min(1e9, account.balance + delta));
      account.activeAt = Date.now();
      pushAccount(account.id);
      return json(res, 200, { ok: true, player: publicAccount(account) });
    }

    return json(res, 400, { error: 'Неизвестная операция' });
  }

  if (req.method === 'POST' && url.pathname === '/api/leave') {
    const body = requestBody;
    const client = clients.get(body.clientId);
    if (!client) return json(res, 200, { ok: true });
    leaveRoom(client);
    send(client, { type: 'noroom' });
    broadcastLobby();
    return json(res, 200, { ok: true });
  }

  if (url.pathname === '/api/status') {
    return json(res, 200, {
      online: clients.size,
      clients: [...clients.values()].map(client => ({ id: client.id.slice(0, 8), name: client.name, roomId: client.roomId })),
      rooms: [...rooms.values()].map(room => ({ id: room.id, status: room.status, bet: room.bet || 0, pot: room.pot || 0, players: room.players.length })),
    });
  }

  const file = url.pathname === '/' ? '/index.html' : url.pathname;
  if (!['/index.html', '/app.js', '/style.css'].includes(file)) return json(res, 404, { error: 'Not found' });
  const filePath = path.join(__dirname, path.normalize(file).replace(/^(\.\.[/\\])+/, ''));
  if (fs.existsSync(filePath) && fs.statSync(filePath).isFile()) {
    res.writeHead(200, { 'Content-Type': MIME[path.extname(filePath)] || 'application/octet-stream' });
    fs.createReadStream(filePath).pipe(res);
    return;
  }
  json(res, 404, { error: 'Not found' });
});

function cancelGame(room) {
  if (room.status !== 'playing') return;
  const stake = room.bet || 0;
  room.lastPayouts = {};
  if (room.pot > 0) {
    room.players.forEach(player => {
      accountOf(player.id).balance += stake;
      room.lastPayouts[player.id] = 0;
      pushAccount(player.id);
    });
  }
  room.pot = 0;
  room.status = 'finished';
  room.finishedAt = Date.now();
  room.finished = { loser: '—', loserId: null, cancelled: true };
  room.message = 'Партия отменена: игроки отключены. Ставки возвращены';
  markEvent(room, null, 'cancel');
  broadcastRoom(room);
}

setInterval(() => {
  const now = Date.now();
  let changed = false;
  rooms.forEach((room, id) => {
    const participants = room.players.filter(player => !player.left);
    const hasConnected = participants.some(player => player.connected && clients.has(player.id));
    const offlineAt = participants.length ? Math.max(...participants.map(player => player.offlineAt ?? now)) : room.finishedAt ?? room.createdAt;
    const expired = !hasConnected && now - offlineAt >= 5 * 60 * 1000;
    if (room.status === 'playing') {
      if (expired) cancelGame(room);
      return;
    }
    const stale = room.status === 'finished' && now - room.finishedAt >= 30 * 60 * 1000;
    if (stale || (room.status === 'waiting' && expired)) {
      rooms.delete(id);
      room.players.forEach(player => {
        const client = clients.get(player.id);
        if (client && client.roomId === id) {
          client.roomId = null;
          send(client, { type: 'noroom' });
        }
      });
      changed = true;
    }
  });
  if (changed) broadcastLobby();
}, 30 * 1000).unref();

server.listen(PORT, HOST, () => {
  console.log(`Durak server: http://localhost:${PORT} (доступен в локальной сети на ${HOST}:${PORT})`);
});