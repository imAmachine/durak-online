import http from 'node:http';
import https from 'node:https';
import crypto from 'node:crypto';

const base = process.env.BASE_URL || 'http://localhost:4173';
const tokens = new Map();
const streams = new Set();
let failures = 0;
function tokenFor(id) {
  if (!tokens.has(id)) tokens.set(id, crypto.randomBytes(32).toString('hex'));
  return tokens.get(id);
}
function check(label, ok, extra = '') {
  console.log(`${ok ? 'ok  ' : 'FAIL'} ${label}${extra ? ` ${extra}` : ''}`);
  if (!ok) failures += 1;
}
const wait = ms => new Promise(resolve => setTimeout(resolve, ms));
async function waitFor(predicate, label, timeout = 3000) {
  const start = Date.now();
  while (!predicate()) {
    if (Date.now() - start > timeout) throw new Error(`timeout: ${label}`);
    await wait(20);
  }
}
async function post(path, body, authenticated = true) {
  const payload = authenticated && body.clientId ? { ...body, token: tokenFor(body.clientId) } : body;
  const response = await fetch(base + path, {
    method: 'POST', headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(payload), signal: AbortSignal.timeout(5000),
  });
  return { status: response.status, data: await response.json().catch(() => ({})) };
}
function connect(id, token = tokenFor(id)) {
  return new Promise((resolve, reject) => {
    const query = new URLSearchParams({ id, name: id });
    if (token !== null) query.set('token', token);
    const url = new URL(`/events?${query}`, base);
    const transport = url.protocol === 'https:' ? https : http;
    const stream = { messages: [], closed: false, state: null };
    const request = transport.get(url, response => {
      stream.status = response.statusCode;
      stream.response = response;
      resolve(stream);
      if (response.statusCode !== 200) { response.resume(); return; }
      let buffer = '';
      response.setEncoding('utf8');
      response.on('data', chunk => {
        buffer += chunk;
        let index;
        while ((index = buffer.indexOf('\n\n')) !== -1) {
          const block = buffer.slice(0, index);
          buffer = buffer.slice(index + 2);
          const line = block.split('\n').find(item => item.startsWith('data: '));
          if (!line) continue;
          const payload = JSON.parse(line.slice(6));
          stream.messages.push(payload);
          if (payload.type === 'game') stream.state = payload.state;
        }
      });
      response.on('close', () => { stream.closed = true; });
      response.on('error', () => { stream.closed = true; });
    });
    stream.request = request;
    streams.add(stream);
    request.on('error', reject);
    request.setTimeout(5000, () => request.destroy(new Error('SSE timeout')));
  });
}
async function ready(id) {
  const stream = await connect(id);
  if (stream.status !== 200) throw new Error(`SSE ${id}: ${stream.status}`);
  await waitFor(() => stream.messages.some(p => p.type === 'lobby'), 'initial lobby');
  return stream;
}
async function create(id, name) {
  const result = await post('/api/rooms', { clientId: id, name, mode: 'classic', deckSize: 36 });
  if (result.status !== 201) throw new Error(`create: ${JSON.stringify(result)}`);
  return result.data.code;
}
const prefix = `security-${Date.now()}-${crypto.randomBytes(4).toString('hex')}`;
const a = `${prefix}-a`, b = `${prefix}-b`;
const members = [a, b];
try {
  let owner = await ready(a);
  const peer = await ready(b);
  const home = await create(a, 'Исходная комната');
  await post('/api/join', { clientId: b, roomId: home });
  await waitFor(() => owner.state?.players.length === 2, 'two members');

  const protectedCalls = [
    ['/api/profile', { name: 'Подмена' }],
    ['/api/leave', {}],
    ['/api/rooms', { name: 'Подмена' }],
    ['/api/join', { roomId: home }],
    ['/api/action', { action: 'start' }],
  ];
  for (const [path, body] of protectedCalls) {
    for (const [label, token] of [['без токена', undefined], ['чужой токен', tokenFor(b)], ['неверный формат', 'bad']]) {
      const result = await post(path, { ...body, clientId: a, ...(token === undefined ? {} : { token }) }, false);
      check(`${path}: ${label}`, result.status === 401 || result.status === 403, String(result.status));
    }
  }
  for (const [label, token] of [['без токена', null], ['короткий токен', 'bad'], ['чужой токен', tokenFor(b)]]) {
    const rejected = await connect(a, token);
    check(`SSE: ${label} отклонён`, rejected.status >= 400 && rejected.status < 500, String(rejected.status));
    rejected.request.destroy();
  }
  const before = owner.messages.length;
  const profile = await post('/api/profile', { clientId: b, name: 'Проверка вещания' });
  check('правильный POST разрешён', profile.status === 200);
  await waitFor(() => owner.messages.slice(before).some(p => p.type === 'game'), 'broadcast after hijack');
  check('перехват не закрыл исходный SSE', !owner.closed);
  check('подмена не изменила комнату', owner.state.room.id === home && owner.state.players.length === 2 && owner.state.room.status !== 'playing');

  const old = owner;
  const peerBefore = peer.messages.length;
  owner = await ready(a);
  await waitFor(() => owner.state?.room.id === home, 'reconnect state');
  await waitFor(() => peer.messages.slice(peerBefore).some(p => p.type === 'game'), 'reconnect broadcast to peer');
  check('легальный reconnect восстановил комнату', owner.state.players.length === 2);
  check('reconnect сообщил остальным connected=true', peer.state.players.find(p => p.id === a)?.connected === true);
  await waitFor(() => old.closed, 'old stream closed');
  const next = owner.messages.length;
  await post('/api/profile', { clientId: b, name: 'После reconnect' });
  await waitFor(() => owner.messages.slice(next).some(p => p.type === 'game'), 'new stream broadcast');
  check('новое соединение получает дальнейшие события', !owner.closed);

  async function failedJoin(label, roomId) {
    const result = await post('/api/join', { clientId: a, roomId });
    check(`${label}: вход отклонён`, result.status >= 400 && result.status < 500, JSON.stringify(result));
    const marker = owner.messages.length;
    await post('/api/profile', { clientId: b, name: label });
    await waitFor(() => owner.messages.slice(marker).some(p => p.type === 'game'), `${label}: home broadcast`);
    check(`${label}: прежняя комната сохранена`, owner.state.room.id === home && owner.state.players.some(p => p.id === a));
    const rejoin = await post('/api/join', { clientId: a, roomId: home });
    check(`${label}: прежнее членство доступно`, rejoin.status === 200);
  }
  await failedJoin('несуществующая комната', `missing-${prefix}`);

  const targetIds = Array.from({ length: 6 }, (_, i) => `${prefix}-target-${i}`);
  for (const id of targetIds) { members.push(id); await ready(id); }
  const target = await create(targetIds[0], 'Полная комната');
  for (const id of targetIds.slice(1)) await post('/api/join', { clientId: id, roomId: target });
  await failedJoin('полная комната', target);
  const start = await post('/api/action', { clientId: targetIds[0], action: 'start' });
  check('целевая партия началась', start.status === 200);
  await failedJoin('начавшаяся партия', target);
} catch (error) {
  check('сценарии безопасности завершились', false, error.stack || String(error));
} finally {
  for (const id of members) await post('/api/leave', { clientId: id }).catch(() => {});
  for (const stream of streams) stream.request.destroy();
}
console.log(failures ? `\n${failures} проверок провалено` : '\nпроверки безопасности пройдены');
process.exitCode = failures ? 1 : 0;
