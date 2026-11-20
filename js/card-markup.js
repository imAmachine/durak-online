import { esc } from './shared.js';

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

export function cardBodyHTML(card) {
  if (card.rank === 'Т') return `<span class="card-ace">${card.suit}</span>`;
  if (card.rank === 'В' || card.rank === 'Д' || card.rank === 'К') {
    return `<span class="card-court"><i>${esc(card.rank)}</i></span>`;
  }
  const pips = PIP_LAYOUTS[card.rank] || [[3, 2]];
  return `<span class="card-pips">${pips.map(([row, column]) => `<i style="grid-area:${row}/${column}">${card.suit}</i>`).join('')}</span>`;
}

export function cardCornersHTML(card, showSuit = false) {
  const faceRank = showSuit || 'ВДКТ'.includes(card.rank);
  const corner = `<span class="card-corner">${esc(card.rank)}${faceRank ? `<i>${card.suit}</i>` : ''}</span>`;
  return `${corner}${cardBodyHTML(card)}<span class="card-corner bottom" aria-hidden="true">${esc(card.rank)}${faceRank ? `<i>${card.suit}</i>` : ''}</span>`;
}

export function cardHTML(card, extraClass = '', target = false) {
  const attrs = target ? `data-target-card="${esc(card.id)}" ` : '';
  const red = card.suit === '♥' || card.suit === '♦';
  return `<div ${attrs}class="card-face ${red ? 'red' : ''} ${extraClass}" data-card-id="${esc(card.id)}">${cardCornersHTML(card)}</div>`;
}

export function cardAriaLabel(card) {
  const suits = { '♠': 'пик', '♣': 'треф', '♥': 'червей', '♦': 'бубен' };
  const ranks = { 'В': 'валет', 'Д': 'дама', 'К': 'король', 'Т': 'туз' };
  return `${ranks[card.rank] || card.rank} ${suits[card.suit] || card.suit}`;
}

export function handCardHTML(card, dimmed) {
  const red = card.suit === '♥' || card.suit === '♦';
  return `<div class="hand-slot"><button class="hand-card ${red ? 'red' : ''} ${dimmed ? 'dim' : ''}" data-card="${esc(card.id)}">${cardCornersHTML(card)}</button></div>`;
}

export function cardBackHTML(extraClass = '') {
  return `<span class="card-back ${extraClass}" aria-hidden="true"><i></i><b>♠</b></span>`;
}
