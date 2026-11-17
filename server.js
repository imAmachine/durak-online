'use strict';

const http = require('http');
const fs = require('fs');
const path = require('path');
const crypto = require('crypto');

const PORT = Number(process.env.PORT) || 4173;
const HOST = process.env.HOST || '0.0.0.0';
const clients = new Map();
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
const ADMIN_KEY = process.env.ADMIN_KEY || 'durak-admin';

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
  return [...rooms.values()].find(room => room.players.some(player => player.id === id)) || null;
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
    if (!candidate.finished) return candidate;
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
    attackerId: null,
    defenderId: null,
    turnId: null,
    takePending: null,
    message: 'Ожидаем игроков. Отправьте друзьям код приглашения',
    finished: null,
    createdAt: Date.now(),
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
  room.takePending = null;
  room.status = 'playing';
  room.finished = null;
  room.pot = 0;
  room.lastPayouts = null;
  room.players.forEach(player => {
    player.finished = false;
    player.hand = [];
  });
  for (let round = 0; round < 6; round += 1) {
    room.players.forEach(player => {
      const card = room.deck.pop();
      if (card) player.hand.push(card);
    });
  }
  syncTrumpCard(room);
  const attacker = room.players[Math.floor(Math.random() * room.players.length)];
  const defender = nextActive(room, attacker);
  room.attackerId = attacker.id;
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
    else account.wins += 1;
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
}

function finishGame(room, loser, message) {
  room.status = 'finished';
  room.finished = { loser: loser ? loser.name : '—', loserId: loser ? loser.id : null };
  room.message = message || (loser ? `${loser.name} — дурак, партия окончена` : 'Игра завершена');
  history.push({ room: room.name, mode: room.mode, at: Date.now(), loser: room.finished.loser, bet: room.bet || 0 });
  settleAccounts(room, room.finished.loserId);
}

function refillHands(room) {
  const attacker = findPlayer(room, room.attackerId);
  const defender = findPlayer(room, room.defenderId);
  const order = [];
  let cursor = attacker;
  for (let step = 0; step < room.players.length && cursor; step += 1) {
    if (!order.includes(cursor) && cursor !== defender) order.push(cursor);
    cursor = nextActive(room, cursor);
  }
  if (defender && !order.includes(defender)) order.push(defender);
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
  const active = room.players.filter(player => !player.finished);
  if (active.length <= 1) {
    const durak = active[0] || null;
    finishGame(room, durak, durak ? `${durak.name} — дурак, партия окончена` : 'Игра завершена');
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
  const nextAttacker = nextActive(room, defender);
  if (!nextAttacker) return { error: 'Другой игрок уже вышел' };
  room.attackerId = nextAttacker.id;
  const nextDefender = nextActive(room, nextAttacker);
  room.defenderId = nextDefender ? nextDefender.id : null;
  refillHands(room);
  room.message = `${defender.name} забрал карты. Ход ${nextAttacker.name}`;
  settle(room);
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
  const canAttack = Boolean(
    me && playing && isAttacking && defender && defender.hand.length > unbeatenRows &&
    room.table.length < MAX_TABLE_CARDS &&
    (room.table.length === 0 || me.hand.some(card => allRanks.has(card.rank)))
  );
  const canDefend = Boolean(me && playing && isDefending && !room.takePending && hasUnbeaten);
  const canTake = Boolean(me && playing && isDefending && room.table.length > 0 && !room.takePending && hasUnbeaten);
  const canPass = Boolean(me && playing && isAttacking && room.table.length > 0 && (room.takePending || !hasUnbeaten));
  const canTransfer = Boolean(
    me && playing && room.mode === 'transfer' && isDefending && !room.takePending &&
    room.table.length > 0 && allUnbeaten && attackRanks.size === 1 &&
    me.hand.some(card => card.rank === [...attackRanks][0])
  );
  const canPlay = canAttack || canDefend;
  const connected = room.players.filter(player => player.connected).length;
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
      role: player.id === room.attackerId ? 'attacker' : player.id === room.defenderId ? 'defender' : 'idle',
    })),
    you: me
      ? {
          id: me.id,
          name: me.name,
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
    canStart: Boolean(me && room.hostId === me.id && room.status !== 'playing' && connected >= 2),
    payout: room.lastPayouts && me ? room.lastPayouts[me.id] || 0 : 0,
    finished: room.finished,
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

function broadcastRoom(room) {
  room.players.forEach(player => {
    const client = clients.get(player.id);
    if (!client) return;
    send(client, { type: 'game', state: gameState(room, player.id) });
    pushAccount(player.id);
  });
  broadcastLobby();
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
  const wasPlaying = room.status === 'playing';
  room.players = room.players.filter(item => item.id !== client.id);
  if (!room.players.length) {
    rooms.delete(room.id);
    broadcastLobby();
    return;
  }
  if (room.hostId === client.id) room.hostId = room.players[0].id;
  if (wasPlaying) {
    room.table = [];
    room.takePending = null;
    const account = accountOf(client.id);
    account.games += 1;
    account.losses += 1;
    if (room.players.length < 2) {
      const survivor = room.players[0];
      finishGame(room, player, `${player.name} покинул партию. ${survivor.name} остался один, игра окончена`);
    } else {
      normalizeRoles(room);
      room.message = `${player.name} покинул игру`;
    }
    pushAccount(client.id);
    broadcastRoom(room);
    return;
  }
  room.message = `${player.name} покинул лобби`;
  broadcastRoom(room);
}

function act(client, data) {
  const room = rooms.get(client.roomId);
  if (!room) return { error: 'Вы не находитесь в лобби' };
  const me = findPlayer(room, client.id);
  if (!me) return { error: 'Вы не участник этой игры' };
  const action = data.action;

  if (action === 'start') {
    if (room.status === 'playing') return { error: 'Партия уже идёт' };
    if (room.hostId !== me.id) return { error: 'Игру начинает создатель лобби' };
    const ready = room.players.filter(player => player.connected);
    if (ready.length < 2) return { error: 'Нужно минимум два игрока' };
    const shortfall = stakeShortfall(room, ready);
    if (shortfall) return { error: shortfall };
    room.players = ready;
    startRound(room);
    chargeStakes(room);
    if (room.bet) room.message += `. Банк ${room.pot} фишек`;
    broadcastRoom(room);
    return { ok: true };
  }

  if (room.status !== 'playing') return { error: 'Игра ещё не идёт' };
  const attacker = findPlayer(room, room.attackerId);
  const defender = findPlayer(room, room.defenderId);

  const isAttacker = me.id === room.attackerId;
  const isDefender = me.id === room.defenderId;

  if (action === 'attack') {
    if (!isAttacker) return { error: 'Атаковать может только ходящий игрок' };
    const card = me.hand.find(item => item.id === data.cardId);
    if (!card) return { error: 'Такой карты нет в руке' };
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
    const card = me.hand.find(item => item.rank === rank);
    if (!card) return { error: `Нужна карта достоинства ${rank}` };
    const nextDefender = nextActive(room, me);
    if (!nextDefender) return { error: 'Переводить больше некому' };
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
    room.table = [];
    const newAttacker = defender;
    const newDefender = nextActive(room, newAttacker);
    room.attackerId = newAttacker.id;
    room.defenderId = newDefender ? newDefender.id : null;
    refillHands(room);
    room.message = `Бито. ${newAttacker.name} атакует${newDefender ? `, ${newDefender.name} отбивается` : ''}`;
    settle(room);
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
    if (!id) {
      res.writeHead(400);
      res.end();
      return;
    }
    if (clients.has(id)) {
      try {
        clients.get(id).res.end();
      } catch (error) {}
    }
    const client = { id, name, res, roomId: null };
    clients.set(id, client);
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
      send(client, { type: 'game', state: gameState(room, id) });
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
        broadcastRoom(current);
      }
      broadcastLobby();
    });
    return;
  }

  if (req.method === 'POST' && url.pathname === '/api/rooms') {
    const body = await readBody(req);
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
    broadcastRoom(room);
    return json(res, 201, { room: roomSummary(room), code: room.id, account: publicAccount(accountOf(client.id)) });
  }

  if (req.method === 'POST' && url.pathname === '/api/join') {
    const body = await readBody(req);
    const client = clients.get(body.clientId);
    if (!client) return json(res, 400, { error: 'Нет соединения с сервером, обновите страницу' });
    const room = rooms.get(String(body.roomId || '').toUpperCase());
    if (!room) return json(res, 404, { error: 'Лобби с таким кодом не найдено' });
    if (client.roomId && client.roomId !== room.id && rooms.has(client.roomId)) leaveRoom(client);
    if (body.playerName) client.name = String(body.playerName).slice(0, 20);
    renameAccount(client.id, client.name);
    const existing = findPlayer(room, client.id);
    if (existing) {
      existing.connected = true;
      existing.offlineAt = null;
      existing.name = client.name;
      client.roomId = room.id;
      broadcastRoom(room);
      return json(res, 200, { room: roomSummary(room), account: publicAccount(accountOf(client.id)) });
    }
    if (room.status === 'playing') return json(res, 400, { error: 'Игра уже началась' });
    if (room.players.length >= MAX_PLAYERS) {
      const now = Date.now();
      room.players = room.players.filter(player => clients.has(player.id) || now - (player.offlineAt || now) < 60 * 1000);
      if (room.players.length >= MAX_PLAYERS) return json(res, 400, { error: 'В лобби нет свободных мест' });
    }
    if (room.bet && accountOf(client.id).balance < room.bet) {
      return json(res, 400, { error: `Недостаточно фишек: ставка за столом ${room.bet}` });
    }
    room.players.push({ id: client.id, name: client.name, hand: [], connected: true, finished: false });
    client.roomId = room.id;
    room.message = 'Игроки готовы. Создатель лобби может начать игру';
    broadcastRoom(room);
    return json(res, 200, { room: roomSummary(room), account: publicAccount(accountOf(client.id)) });
  }

  if (req.method === 'POST' && url.pathname === '/api/action') {
    const body = await readBody(req);
    const client = clients.get(body.clientId);
    if (!client) return json(res, 400, { error: 'Нет соединения с сервером, обновите страницу' });
    const result = act(client, body);
    return json(res, result.error ? 400 : 200, result);
  }

  if (req.method === 'POST' && url.pathname === '/api/profile') {
    const body = await readBody(req);
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
    if (room) broadcastRoom(room);
    else broadcastLobby();
    return json(res, 200, { ok: true, name, account: publicAccount(accountOf(client.id)) });
  }

  if (req.method === 'POST' && url.pathname === '/api/admin') {
    const body = await readBody(req);
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
      if (room) broadcastRoom(room);
      else broadcastLobby();
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
    const body = await readBody(req);
    const client = clients.get(body.clientId);
    if (!client) return json(res, 200, { ok: true });
    leaveRoom(client);
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
  const filePath = path.join(__dirname, path.normalize(file).replace(/^(\.\.[/\\])+/, ''));
  if (fs.existsSync(filePath) && fs.statSync(filePath).isFile()) {
    res.writeHead(200, { 'Content-Type': MIME[path.extname(filePath)] || 'application/octet-stream' });
    fs.createReadStream(filePath).pipe(res);
    return;
  }
  json(res, 404, { error: 'Not found' });
});

setInterval(() => {
  const now = Date.now();
  let changed = false;
  rooms.forEach((room, id) => {
    const hasConnected = room.players.some(player => clients.has(player.id));
    const stale = room.status === 'finished' && now - room.createdAt > 30 * 60 * 1000;
    if (stale) {
      rooms.delete(id);
      changed = true;
      return;
    }
    if (hasConnected) return;
    if (room.status !== 'playing' || now - room.createdAt > 60 * 1000) {
      rooms.delete(id);
      changed = true;
    }
  });
  if (changed) broadcastLobby();
}, 30 * 1000).unref();

server.listen(PORT, HOST, () => {
  console.log(`Durak server: http://localhost:${PORT} (доступен в локальной сети на ${HOST}:${PORT})`);
});