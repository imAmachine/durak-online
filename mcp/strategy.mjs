export const RANKS = ['2', '3', '4', '5', '6', '7', '8', '9', '10', 'В', 'Д', 'К', 'Т'];
export const VALUES = new Map(RANKS.map((rank, index) => [rank, index]));
export const SUITS = ['♠', '♥', '♦', '♣'];
export const MAX_TABLE_CARDS = 6;

export function beats(attack, defense, trump) {
  if (!attack || !defense) return false;
  if (attack.suit === defense.suit) return VALUES.get(defense.rank) > VALUES.get(attack.rank);
  return defense.suit === trump && attack.suit !== trump;
}

export function tableRanks(state) {
  const ranks = new Set();
  for (const row of state.table || []) {
    ranks.add(row.attack.rank);
    if (row.defense) ranks.add(row.defense.rank);
  }
  return ranks;
}

export function cardText(card) {
  return card ? card.rank + card.suit : '—';
}

export function parseCard(id) {
  if (typeof id !== 'string' || id.length < 2) return null;
  const suit = id.slice(-1);
  if (!SUITS.includes(suit)) return null;
  const rank = id.slice(0, -1);
  if (!VALUES.has(rank)) return null;
  return { id, rank, suit };
}

export function handText(hand) {
  return hand && hand.length ? hand.map(cardText).join(' ') : '—';
}

export function deckCards(size) {
  const ranks = size === 52 ? RANKS : RANKS.filter(rank => VALUES.get(rank) >= VALUES.get('6'));
  return SUITS.flatMap(suit => ranks.map(rank => ({ id: rank + suit, rank, suit })));
}

export function attachOptions(state) {
  if (!state.you || !state.canAttack) return [];
  const hand = state.you.hand || [];
  if (!state.table.length) return hand.slice();
  const ranks = tableRanks(state);
  return hand.filter(card => ranks.has(card.rank));
}

export function defenseOptions(state) {
  if (!state.you || !state.canDefend) return [];
  const hand = state.you.hand || [];
  const options = [];
  for (const row of state.table || []) {
    if (row.defense) continue;
    for (const card of hand) {
      if (beats(row.attack, card, state.trump)) {
        options.push({ row, card, trump: card.suit === state.trump, value: VALUES.get(card.rank) });
      }
    }
  }
  options.sort((a, b) => (Number(a.trump) - Number(b.trump)) || (a.value - b.value));
  return options;
}

export function transferOptions(state) {
  if (!state.you || !state.canTransfer) return [];
  const ranks = tableRanks(state);
  const rank = [...ranks][0];
  return (state.you.hand || []).filter(card => card.rank === rank);
}

export function dumpOptions(state) {
  if (!state.you || !state.canAttack) return [];
  const ranks = tableRanks(state);
  return (state.you.hand || [])
    .filter(card => ranks.has(card.rank) && card.suit !== state.trump)
    .sort((a, b) => VALUES.get(a.rank) - VALUES.get(b.rank));
}

export function attackChoices(state) {
  const cards = attachOptions(state);
  if (!cards.length || state.table.length >= MAX_TABLE_CARDS) return [];
  const byValue = list => [...list].sort((a, b) => VALUES.get(a.rank) - VALUES.get(b.rank));
  const plain = byValue(cards.filter(card => card.suit !== state.trump));
  const trumps = byValue(cards.filter(card => card.suit === state.trump));
  if (!state.table.length) return [...plain, ...trumps];
  if (plain.length) return plain;
  const defender = (state.players || []).find(player => player.id === state.defenderId);
  if (state.deck === 0 || (defender && defender.cards <= 2)) return trumps;
  return [];
}

export class Ledger {
  constructor() {
    this.reset(null);
  }

  reset(roomId) {
    this.roomId = roomId;
    this.seen = new Set();
    this.dead = new Set();
    this.pending = [];
    this.lastSeq = 0;
  }

  observe(state) {
    if (!state) return this;
    const roomId = state.room ? state.room.id : null;
    if (roomId !== this.roomId) this.reset(roomId);
    for (const card of state.hand || []) this.seen.add(card.id);
    if (state.trumpCard) this.seen.add(state.trumpCard.id);
    for (const row of state.table || []) {
      this.seen.add(row.attack.id);
      if (row.defense) this.seen.add(row.defense.id);
    }
    const events = [...(state.log || [])].sort((a, b) => a.seq - b.seq);
    for (const event of events) this.applyEvent(event, state);
    if (!this.pending.length && (state.table || []).length) {
      this.pending = state.table.map(row => ({ attack: row.attack, defense: row.defense }));
    }
    return this;
  }

  applyEvent(event, state) {
    if (!event || typeof event.seq !== 'number' || event.seq <= this.lastSeq) return;
    this.lastSeq = event.seq;
    if (event.cardId) this.seen.add(event.cardId);
    if (event.targetCardId) this.seen.add(event.targetCardId);
    const card = parseCard(event.cardId);
    if (event.action === 'attack' || event.action === 'transfer') {
      if (card && !this.pending.some(row => row.attack.id === card.id)) this.pending.push({ attack: card, defense: null });
    } else if (event.action === 'defend') {
      const row = this.pending.find(item => item.attack.id === event.targetCardId && !item.defense)
        || this.pending.find(item => !item.defense);
      if (row && card) row.defense = card;
    } else if (event.action === 'pass') {
      if (this.pending.length && this.pending.every(row => row.defense)) {
        for (const row of this.pending) {
          this.dead.add(row.attack.id);
          this.dead.add(row.defense.id);
        }
        this.pending = [];
      }
    } else if (event.action === 'take') {
      this.pending = [];
    } else if (event.action === 'start' || event.action === 'cancel') {
      this.dead.clear();
      this.pending = [];
    }
  }

  unseen(state) {
    if (!state || !state.room) return [];
    const size = Number(state.room.deckSize) === 52 ? 52 : 36;
    const visible = new Set(this.seen);
    for (const row of state.table || []) {
      visible.add(row.attack.id);
      if (row.defense) visible.add(row.defense.id);
    }
    for (const card of state.hand || []) visible.add(card.id);
    if (state.trumpCard) visible.add(state.trumpCard.id);
    return deckCards(size).filter(card => !visible.has(card.id));
  }

  summary(state) {
    if (!state || !state.room) return { unseen: [], unseenTrumps: [], dead: [], line: 'журнал пуст' };
    const unseen = this.unseen(state);
    const trumps = unseen.filter(card => card.suit === state.trump);
    const deck = Number(state.deck) || 0;
    const inHands = Math.max(0, unseen.length - deck);
    const line = `Скрыто ${unseen.length}: в колоде ${deck}, в чужих руках ${inHands} · козырей не видел ${trumps.length} · вне игры ${this.dead.size}`;
    return {
      unseen: unseen.map(card => card.id),
      unseenTrumps: trumps.map(card => card.id),
      dead: [...this.dead],
      trumpCount: trumps.length,
      line,
    };
  }

  counts(state) {
    const summary = this.summary(state);
    if (!summary.unseen.length && summary.line === 'журнал пуст') return { unseen: 0, trumps: 0, out: 0, line: summary.line };
    return {
      unseen: summary.unseen.length,
      trumps: summary.trumpCount,
      out: summary.dead.length,
      trumpCards: summary.unseenTrumps,
      line: summary.line,
    };
  }
}

function defendDecision(state, defenses, ledger) {
  if (!defenses.length) return null;
  const plain = defenses.filter(item => !item.trump);
  const pick = item => ({
    action: 'defend',
    extra: { cardId: item.card.id, targetCardId: item.row.attack.id },
    note: `бью ${cardText(item.row.attack)} на ${cardText(item.card)}`,
    reason: item.trump ? 'козырь допустим' : 'младшая некозырная',
  });
  if (plain.length) return pick(plain[0]);
  const trumpsInHand = (state.you.hand || []).filter(card => card.suit === state.trump).length;
  const unbeaten = (state.table || []).filter(row => !row.defense).length;
  const unseenTrumps = ledger ? ledger.unseen(state).filter(card => card.suit === state.trump).length : null;
  const spend = trumpsInHand >= 2 || state.deck === 0 || unbeaten >= 3 || (unseenTrumps !== null && unseenTrumps <= 1);
  return spend ? pick(defenses[0]) : null;
}

export function chooseMove(state, options = {}) {
  const ledger = options.ledger || null;
  if (!state || !state.you || state.you.finished || state.you.left) return null;
  if (!state.room || state.room.status !== 'playing') return null;
  const mine = state.you.id;
  const isDefender = state.defenderId === mine;
  const unbeaten = (state.table || []).filter(row => !row.defense);
  const defenses = defenseOptions(state);

  if (state.takePending) {
    if (isDefender) return null;
    const dump = dumpOptions(state);
    if (state.canAttack && dump.length) {
      return { action: 'attack', extra: { cardId: dump[0].id }, note: `догружаю ${cardText(dump[0])}`, reason: 'догрузка под «беру»' };
    }
    if (state.canPass) return { action: 'pass', note: 'бито, защитник забирает карты', reason: 'закрыть кон' };
    return null;
  }

  if (unbeaten.length) {
    if (!isDefender) return null;
    if (state.canTransfer) {
      const transfers = transferOptions(state);
      const cheapDefense = defenses.some(item => !item.trump);
      if (transfers.length && (!cheapDefense || unbeaten.length >= 2)) {
        return { action: 'transfer', extra: { cardId: transfers[0].id }, note: `перевожу ${cardText(transfers[0])}`, reason: 'перевод выгоднее защиты' };
      }
    }
    const defense = defendDecision(state, defenses, ledger);
    if (defense) return defense;
    if (state.canTake) return { action: 'take', note: `беру ${unbeaten.length} карт(ы)`, reason: 'отбиться невыгодно' };
    return null;
  }

  const choices = attackChoices(state);
  if (state.canAttack && choices.length) {
    const card = choices[0];
    return {
      action: 'attack',
      extra: { cardId: card.id },
      note: state.table.length ? `подкидываю ${cardText(card)}` : `хожу ${cardText(card)}`,
      reason: 'атака',
    };
  }
  if (state.canPass) return { action: 'pass', note: 'бито', reason: 'закончить кон' };
  return null;
}
