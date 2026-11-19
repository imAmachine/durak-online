import crypto from 'node:crypto';

export const base = process.env.BASE_URL || 'http://localhost:4173';
export const adminKey = process.env.ADMIN_KEY || 'test-only-secret';

const accounts = new Map();
let counter = 0;

async function post(path, body) {
  const response = await fetch(base + path, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) });
  return { status: response.status, data: await response.json().catch(() => ({})) };
}

export function credentialsFor(label, name) {
  if (!accounts.has(label)) accounts.set(label, { clientId: null, token: crypto.randomBytes(32).toString('hex'), name: name || label });
  return accounts.get(label);
}

export async function clubCredentialsFor(label, name) {
  const account = credentialsFor(label, name);
  if (account.clientId) return account;
  const password = `pw-${crypto.randomBytes(6).toString('hex')}`;
  const created = await post('/api/admin', { key: adminKey, op: 'create_temp', name: account.name, password });
  if (created.status !== 200) throw new Error(`клубный аккаунт ${account.name}: ${JSON.stringify(created.data)}`);
  const login = await post('/api/auth/temp/login', { name: account.name, password });
  if (login.status !== 200) throw new Error(`вход ${account.name}: ${JSON.stringify(login.data)}`);
  account.clientId = login.data.clientId;
  account.token = login.data.token;
  return account;
}

export function uniqueId(prefix) {
  counter += 1;
  return `${prefix}-${process.pid}-${counter}`;
}

export async function referenceIds(labels) {
  return labels.map(label => {
    const id = uniqueId(`ref-${label}`);
    credentialsFor(id, id);
    return { clientId: id, token: credentialsFor(id, id).token };
  });
}

export async function registerClubAccounts(labels) {
  try {
    return await clubAccounts(labels);
  } catch (error) {
    if (!/Панель администратора отключена|Игрок не найден|Неизвестная операция|create_temp/.test(String(error.message))) throw error;
    console.warn(`клубные аккаунты недоступны (${error.message}), используем временные id reference-сервера`);
    return referenceIds(labels);
  }
}

async function clubAccounts(labels) {
  const list = await Promise.all(labels.map(label => clubCredentialsFor(`club-${label}`, `suite-${label}-${crypto.randomBytes(3).toString('hex')}`)));
  return list;
}
