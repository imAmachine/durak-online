import { spawn } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';
import { adminKey, base, clubCredentialsFor, deleteClubAccount, leaveTestRoom, TestFailure, testBackend } from './accounts.mjs';

const manifest = fileURLToPath(new URL('../../rust/crates/platform-mcp/Cargo.toml', import.meta.url));

function startMcp(sessions) {
  const child = spawn('cargo', ['run', '--quiet', '--locked', '--manifest-path', manifest], {
    stdio: ['pipe', 'pipe', 'ignore'],
    env: { ...process.env, DURAK_URL: base, DURAK_ADMIN_KEY: adminKey, DURAK_MCP_SESSIONS: sessions },
  });
  const pending = new Map();
  let nextId = 1;
  let failure;
  const fail = error => {
    failure ??= error;
    for (const settle of pending.values()) settle(failure);
  };
  child.on('error', () => fail(new TestFailure('не удалось запустить MCP')));
  child.stdin.on('error', () => fail(new TestFailure('не удалось записать запрос MCP')));
  const closed = new Promise(resolve => child.once('close', (code, signal) => {
    fail(new TestFailure(`MCP завершился (${signal || code})`));
    resolve({ code, signal });
  }));
  createInterface({ input: child.stdout }).on('line', line => {
    let message;
    try { message = JSON.parse(line); } catch { return; }
    pending.get(message?.id)?.(null, message);
  });

  const rpc = (method, params, timeout = 20000) => {
    if (failure) return Promise.reject(failure);
    const id = nextId++;
    const operation = method === 'tools/call' ? params.name : method;
    return new Promise((resolve, reject) => {
      const settle = (error, message) => {
        clearTimeout(timer);
        pending.delete(id);
        if (error) reject(error);
        else if (message.error) reject(new TestFailure(`${operation}: ошибка JSON-RPC`));
        else resolve(message.result);
      };
      const timer = setTimeout(() => settle(new TestFailure(`${operation}: таймаут MCP`)), timeout);
      pending.set(id, settle);
      child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id, method, params })}\n`);
    });
  };
  return {
    rpc,
    notify(method) {
      child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', method })}\n`);
    },
    async call(name, args = {}, timeout) {
      const result = await rpc('tools/call', { name, arguments: args }, timeout);
      return { text: result?.content?.[0]?.text || '', data: result?.structuredContent || {}, isError: Boolean(result?.isError) };
    },
    async stop() {
      child.stdin.end();
      const timer = setTimeout(() => child.kill('SIGKILL'), 2000);
      try {
        const { code, signal } = await closed;
        if (code !== 0) throw new TestFailure(`MCP завершился (${signal || code})`);
      } finally {
        clearTimeout(timer);
      }
    },
  };
}

export async function runMcpTest(run) {
  const children = new Set();
  const createdIds = new Set();
  const credentials = [];
  const report = (context, error) => {
    console.log(`FAIL ${context}${error instanceof TestFailure ? `: ${error.message}` : ''}`);
    process.exitCode = 1;
  };
  const clean = async (context, action) => {
    try { await action(); } catch (error) { report(context, error); }
  };
  let dir;
  try {
    await testBackend();
    const sessions = () => {
      dir ??= fs.mkdtempSync(path.join(os.tmpdir(), 'durak-mcp-test-'));
      return path.join(dir, 'sessions.json');
    };
    const start = () => {
      const child = startMcp(sessions());
      children.add(child);
      return {
        ...child,
        async stop() {
          await child.stop();
          children.delete(child);
        },
      };
    };
    const account = async label => {
      const result = await clubCredentialsFor(label, createdIds);
      if (credentials.some(item => item.clientId === result.clientId || item.token === result.token)) {
        throw new TestFailure('тестовые аккаунты должны иметь разные id и токены');
      }
      credentials.push(result);
      return result;
    };
    await run({ start, sessions, account });
  } catch (error) {
    report('проверка MCP', error);
  } finally {
    for (const child of children) await clean('остановка MCP', () => child.stop());
    for (const account of credentials) await clean('очистка тестового стола', () => leaveTestRoom(account));
    for (const id of createdIds) await clean('очистка тестового аккаунта', () => deleteClubAccount(id));
    if (dir) await clean('удаление файлов сессий MCP', () => fs.rmSync(dir, { recursive: true, force: true }));
  }
}
