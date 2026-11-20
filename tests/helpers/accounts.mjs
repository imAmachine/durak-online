import crypto from 'node:crypto';

export const base = (process.env.BASE_URL || 'http://localhost:4173').replace(/\/+$/, '');
export const adminKey = process.env.ADMIN_KEY || 'test-only-secret';

export class TestFailure extends Error {}

async function request(path, body) {
  try {
    const response = await fetch(base + path, {
      ...(body === undefined ? {} : {
        method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body),
      }),
      redirect: 'error', signal: AbortSignal.timeout(5000),
    });
    return { status: response.status, data: await response.json().catch(() => null) };
  } catch (error) {
    const reason = error.name === 'TimeoutError' ? 'таймаут' : 'ошибка соединения';
    throw new TestFailure(`${path}: ${reason}`);
  }
}

export async function testBackend() {
  const mode = process.env.TEST_BACKEND;
  if (mode !== undefined && mode !== 'rust') throw new TestFailure('TEST_BACKEND должен быть rust');
  const { status, data } = await request('/readyz');
  if (status !== 200 || data?.ready !== true) throw new TestFailure(`/readyz: Rust стенд не готов (HTTP ${status})`);
  return 'rust';
}

export async function clubCredentialsFor(label, createdIds = new Set()) {
  const suffix = crypto.randomBytes(4).toString('hex');
  const prefix = `test-${label}-`;
  const name = `${prefix.slice(0, Math.max(1, 20 - suffix.length))}${suffix}`;
  const password = crypto.randomBytes(16).toString('hex');
  const created = await request('/api/admin', { key: adminKey, op: 'create_temp', name, password });
  if (created.status !== 200) throw new TestFailure(`create_temp: HTTP ${created.status}`);
  const id = created.data?.account?.id;
  if (typeof id !== 'string' || !/^temp:[\w-]+$/.test(id) || createdIds.has(id)) {
    throw new TestFailure('create_temp: неверный или повторный id аккаунта');
  }
  createdIds.add(id);
  const login = await request('/api/auth/temp/login', { name, password });
  if (login.status !== 200) throw new TestFailure(`/api/auth/temp/login: HTTP ${login.status}`);
  if (login.data?.clientId !== id || !/^[a-f0-9]{64}$/.test(login.data?.token || '')) {
    throw new TestFailure('/api/auth/temp/login: неверная сессия');
  }
  return { clientId: id, token: login.data.token };
}

export async function leaveTestRoom({ clientId, token }) {
  const { status, data } = await request('/api/leave', { clientId, token });
  if (status !== 200 || data?.ok !== true) throw new TestFailure(`выход из тестового стола: HTTP ${status}`);
}

export async function deleteClubAccount(id) {
  const { status, data } = await request('/api/admin', { key: adminKey, op: 'delete', id });
  if (status !== 200 || data?.deleted !== id) throw new TestFailure(`удаление тестового аккаунта: HTTP ${status}`);
}

export async function cleanupClubAccounts(credentials, createdIds) {
  const errors = [];
  for (const account of credentials) {
    try { await leaveTestRoom(account); } catch (error) { errors.push(error); }
  }
  for (const id of createdIds) {
    try { await deleteClubAccount(id); } catch (error) { errors.push(error); }
  }
  if (errors.length) throw new AggregateError(errors, errors.map(error => error.message).join('; '));
}

export async function registerClubAccounts(labels) {
  await testBackend();
  const createdIds = new Set();
  const credentials = [];
  try {
    for (const label of labels) credentials.push(await clubCredentialsFor(label, createdIds));
  } catch (error) {
    try { await cleanupClubAccounts(credentials, createdIds); } catch (cleanupError) {
      throw new AggregateError([error, cleanupError], `${error.message}; очистка: ${cleanupError.message}`);
    }
    throw error;
  }
  return { credentials, cleanup: () => cleanupClubAccounts(credentials, createdIds) };
}
