import { spawn } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const base = process.env.BASE_URL || 'http://localhost:4173';
const adminKey = process.env.ADMIN_KEY || 'test-only-secret';
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const sessions = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'durak-mcp-')), 'sessions.json');
let failures = 0;

function check(label, condition, extra = '') {
  if (condition) {
    console.log(`ok   ${label}`);
  } else {
    failures += 1;
    console.log(`FAIL ${label} ${extra}`);
  }
}

function start() {
  const child = spawn(process.execPath, [path.join(root, 'mcp/server.mjs')], {
    cwd: root,
    stdio: ['pipe', 'pipe', 'pipe'],
    env: { ...process.env, DURAK_URL: base, DURAK_ADMIN_KEY: adminKey, ADMIN_KEY: adminKey, DURAK_MCP_SESSIONS: sessions },
  });
  let buffer = '';
  let nextId = 1;
  const pending = new Map();
  let stderrText = '';
  child.stdout.setEncoding('utf8');
  child.stdout.on('data', chunk => {
    buffer += chunk;
    let index;
    while ((index = buffer.indexOf('\n')) !== -1) {
      const line = buffer.slice(0, index).trim();
      buffer = buffer.slice(index + 1);
      if (!line) continue;
      let message;
      try { message = JSON.parse(line); } catch (error) { continue; }
      if (message.id !== undefined && pending.has(message.id)) {
        const settle = pending.get(message.id);
        pending.delete(message.id);
        settle(message);
      }
    }
  });
  child.stderr.setEncoding('utf8');
  child.stderr.on('data', chunk => { stderrText += chunk; });
  const rpc = (method, params, timeout = 20000) => {
    const id = nextId;
    nextId += 1;
    child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id, method, params })}\n`);
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        pending.delete(id);
        reject(new Error(`timeout ${method}`));
      }, timeout);
      pending.set(id, message => {
        clearTimeout(timer);
        if (message.error) reject(new Error(message.error.message));
        else resolve(message.result);
      });
    });
  };
  const call = async (name, args = {}) => {
    const result = await rpc('tools/call', { name, arguments: args });
    return {
      text: result?.content?.[0]?.text || '',
      data: result?.structuredContent || {},
      isError: Boolean(result?.isError),
    };
  };
  const stop = () => new Promise(resolve => {
    child.on('exit', resolve);
    child.stdin.end();
    setTimeout(() => child.kill('SIGKILL'), 3000).unref();
  });
  return { rpc, call, stop, stderr: () => stderrText };
}

try {
  const first = start();
  await first.rpc('initialize', { protocolVersion: '2024-11-05', capabilities: {}, clientInfo: { name: 'tests/mcp-sessions.mjs', version: '1.0.0' } });
  first.rpc('tools/list', {});

  const probe = await first.call('durak_connect', { seat: 'persist', create: true });
  if (probe.isError) {
    console.log(`mcp-sessions: пропущен — сервис не создаёт клубные аккаунты (${probe.text.slice(0, 120)})`);
    await first.stop();
    process.exit(0);
  }
  check('connect: место открыто', probe.data.connected === true, probe.text);
  const clientId = probe.data.clientId;
  check('sessions: файл мест создан', fs.existsSync(sessions), sessions);
  const saved = fs.existsSync(sessions) ? JSON.parse(fs.readFileSync(sessions, 'utf8')) : [];
  check('sessions: место записано в файл', saved.some(item => item.label === 'persist' && item.id === clientId), JSON.stringify(saved));
  check('sessions: токен сохранён в файле', saved.every(item => /^[a-f0-9]{64}$/.test(item.token || '')), JSON.stringify(saved.map(item => item.token)));
  await first.stop();

  const second = start();
  await second.rpc('initialize', { protocolVersion: '2024-11-05', capabilities: {}, clientInfo: { name: 'tests/mcp-sessions.mjs', version: '1.0.0' } });
  const restored = await second.call('durak_seats');
  const seat = (restored.data.seats || []).find(item => item.seat === 'persist');
  check('sessions: место восстановлено после рестарта', Boolean(seat), restored.text);
  check('sessions: тот же clientId', seat?.clientId === clientId, JSON.stringify(seat));
  check('sessions: поток восстановлен', seat?.connected === true, JSON.stringify(seat));

  const created = await second.call('durak_room_create', { seat: 'persist', name: 'MCP сессии' });
  check('room_create: стол создан', !created.isError && /^[0-9A-F]{6}$/.test(created.data.code || ''), created.text);
  check('room_create: учёт карт только в игре', created.data.snapshot?.hidden === undefined, JSON.stringify(created.data.snapshot?.hidden));

  const partner = await second.call('durak_connect', { seat: 'persist2', create: true });
  check('connect: второе место открыто', partner.data.connected === true, partner.text);
  const joined = await second.call('durak_room_join', { seat: 'persist2', code: created.data.code });
  check('room_join: второй игрок за столом', !joined.isError, joined.text);
  const started = await second.call('durak_start', { seat: 'persist' });
  check('start: партия идёт', started.data.state?.room?.status === 'playing', started.text);

  const turn = await second.call('durak_turn', { seat: 'persist' });
  check('turn: инструмент отвечает', !turn.isError && turn.data.seat === 'persist', turn.text);
  check('turn: снимок без полного состояния', turn.data.state === undefined && turn.data.snapshot?.room === created.data.code, JSON.stringify(Object.keys(turn.data)));
  check('turn: учёт карт в снимке', Number.isInteger(turn.data.snapshot?.hidden?.unseen), JSON.stringify(turn.data.snapshot?.hidden));
  check('turn: учёт карт в подсказке', Number.isInteger(turn.data.hidden?.unseen), turn.text);
  const attacker = started.data.state?.attackerId;
  check('turn: рекомендация у атакующего', attacker === clientId ? turn.data.recommended?.action === 'attack' : turn.data.recommended === null, JSON.stringify(turn.data.recommended));

  const hints = await second.call('durak_hints', { seat: 'persist' });
  check('hints: учёт карт в подсказке', Number.isInteger(hints.data.hidden?.unseen), hints.text);

  await second.call('durak_room_leave', { seat: 'persist2' });
  await second.call('durak_room_leave', { seat: 'persist' });
  const closed = await second.call('durak_disconnect', { seat: 'persist' });
  check('disconnect: место закрыто', !closed.isError, closed.text);
  const after = fs.existsSync(sessions) ? JSON.parse(fs.readFileSync(sessions, 'utf8')) : [];
  check('sessions: закрытое место удалено из файла', !after.some(item => item.label === 'persist'), JSON.stringify(after));
  await second.stop();
  fs.rmSync(path.dirname(sessions), { recursive: true, force: true });
} catch (error) {
  failures += 1;
  console.log(`FAIL ${error.message}`);
}

if (failures) process.exit(1);
console.log('mcp-sessions: места переживают рестарт MCP');
