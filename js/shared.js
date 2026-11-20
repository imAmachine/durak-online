export const RANK_ORDER = ['2', '3', '4', '5', '6', '7', '8', '9', '10', 'В', 'Д', 'К', 'Т'];
export const SUIT_ORDER = ['♠', '♥', '♦', '♣'];
export const AVATAR_TINTS = ['#dceadf', '#f0dfe2', '#f2e7ca', '#dbe7f1', '#e6e0f1', '#f6e2d6'];
export const MAX_ROOM_PLAYERS = 6;
export const MAX_TABLE_PAIRS = 6;
export const INITIAL_HAND_SIZE = 6;
export const DECK_SIZES = [36, 52];
export const SEAT_SEQUENCE_WAIT_MS = 400;
export const ROOM_WATCH_TIMEOUT_MS = 1500;
export const BET_OPTIONS = [0, 50, 100, 250, 500];
export const EMPTY_ACCOUNT = { balance: 0, games: 0, wins: 0, losses: 0 };
export const ADMIN_FILTERS = [['all', 'Все'], ['temp', 'Клубные'], ['discord', 'Discord'], ['legacy', 'Старые'], ['online', 'Онлайн']];

export function readCredentials() {
  try {
    const clientId = sessionStorage.getItem('durak.client');
    const token = sessionStorage.getItem('durak.token');
    if (/^(discord|temp):[A-Za-z0-9_-]+$/.test(clientId || '') && /^[a-f0-9]{64}$/.test(token || '')) return { clientId, token };
  } catch (error) {
    return null;
  }
  return null;
}

export function loadProfile() {
  try {
    const saved = JSON.parse(localStorage.getItem('durak.profile') || '{}');
    return { name: saved.name || 'Игрок', tint: Number(saved.tint) || 0 };
  } catch (error) {
    return { name: 'Игрок', tint: 0 };
  }
}

export function esc(value) {
  return String(value).replace(/[&<>"']/g, symbol => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[symbol]));
}
export function chips(value) { return new Intl.NumberFormat('ru-RU').format(Number(value) || 0); }
export function betLabel(bet) { return bet ? `${chips(bet)} фишек` : 'Без ставки'; }
export function wordForm(count, forms) {
  const rest10 = count % 10;
  const rest100 = count % 100;
  if (rest10 === 1 && rest100 !== 11) return forms[0];
  if (rest10 >= 2 && rest10 <= 4 && (rest100 < 12 || rest100 > 14)) return forms[1];
  return forms[2];
}
export function initials(name) { return String(name).trim().split(/\s+/).slice(0, 2).map(part => part[0] || '').join('').toUpperCase() || '?'; }
export function formatClubDate(date, withTime) {
  const pad = number => String(number).padStart(2, '0');
  const day = `${pad(date.getDate())}.${pad(date.getMonth() + 1)}.${date.getFullYear()}`;
  return withTime ? `${day} ${pad(date.getHours())}:${pad(date.getMinutes())}` : day;
}
export function clubMoment(epochMs) {
  const value = Number(epochMs);
  return Number.isFinite(value) && value > 0 ? formatClubDate(new Date(value), true) : '';
}
export function clubDate(seconds, withTime) {
  const value = Number(seconds);
  return seconds && Number.isFinite(value) && value > 0 ? formatClubDate(new Date(value * 1000), withTime) : '';
}
export function generatePassword() {
  const alphabet = 'abcdefghijkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789';
  const cryptoObject = typeof crypto !== 'undefined' ? crypto : null;
  const size = cryptoObject && cryptoObject.getRandomValues ? cryptoObject.getRandomValues(new Uint8Array(14)) : Array.from({ length: 14 }, () => Math.floor(Math.random() * 256));
  return Array.from(size, byte => alphabet[byte % alphabet.length]).join('');
}
