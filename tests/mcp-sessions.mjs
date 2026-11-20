import fs from 'node:fs';
import { runMcpTest } from './helpers/mcp.mjs';
import { TestFailure } from './helpers/accounts.mjs';

function check(label, condition) {
  console.log(`${condition ? 'ok  ' : 'FAIL'} ${label}`);
  if (!condition) process.exitCode = 1;
}

const initialize = { protocolVersion: '2024-11-05', capabilities: {}, clientInfo: { name: 'tests/mcp-sessions.mjs', version: '1.0.0' } };

await runMcpTest(async ({ start, sessions, account }) => {

  const first = start();
  await first.rpc('initialize', initialize);
  const primary = await account('persist');
  const opened = await first.call('durak_connect', { seat: 'persist', ...primary });
  if (opened.isError || opened.data.connected !== true) throw new TestFailure('durak_connect: место persist не открыто');
  check('connect: своя клубная сессия', opened.data.clientId === primary.clientId);
  const saved = JSON.parse(fs.readFileSync(sessions(), 'utf8'));
  const savedSeat = saved.find(item => item.label === 'persist');
  check('sessions: место записано в файл', savedSeat?.id === primary.clientId);
  check('sessions: записан тот же токен', savedSeat?.token === primary.token);
  await first.stop();

  const second = start();
  await second.rpc('initialize', initialize);
  const restored = await second.call('durak_seats');
  const seat = (restored.data.seats || []).find(item => item.seat === 'persist');
  check('sessions: место восстановлено после рестарта', !restored.isError && Boolean(seat));
  check('sessions: тот же clientId', seat?.clientId === primary.clientId);
  check('sessions: поток восстановлен', seat?.connected === true);

  const created = await second.call('durak_room_create', { seat: 'persist', name: 'MCP сессии' });
  check('room_create: стол создан', !created.isError && /^[0-9A-F]{6}$/.test(created.data.code || ''));
  check('room_create: учёт карт только в игре', created.data.snapshot?.hidden === undefined);

  const peer = await account('persist2');
  const partner = await second.call('durak_connect', { seat: 'persist2', ...peer });
  if (partner.isError || partner.data.connected !== true) throw new TestFailure('durak_connect: место persist2 не открыто');
  check('connect: своя клубная сессия у второго места', partner.data.clientId === peer.clientId && partner.data.clientId !== seat?.clientId);
  const joined = await second.call('durak_room_join', { seat: 'persist2', code: created.data.code });
  check('room_join: второй игрок за столом', !joined.isError);
  const started = await second.call('durak_start', { seat: 'persist' });
  check('start: партия идёт', started.data.state?.room?.status === 'playing');

  const turn = await second.call('durak_turn', { seat: 'persist' });
  check('turn: инструмент отвечает', !turn.isError && turn.data.seat === 'persist');
  check('turn: снимок без полного состояния', turn.data.state === undefined && turn.data.snapshot?.room === created.data.code);
  check('turn: учёт карт в снимке', Number.isInteger(turn.data.snapshot?.hidden?.unseen));
  check('turn: учёт карт в подсказке', Number.isInteger(turn.data.hidden?.unseen));
  const verboseTurn = await second.call('durak_turn', { seat: 'persist', verbose: true });
  check('turn: полное состояние по запросу', !verboseTurn.isError && verboseTurn.data.state?.room?.id === created.data.code && verboseTurn.data.state?.log === undefined);
  const attacker = started.data.state?.attackerId;
  check('turn: рекомендация у атакующего', attacker === primary.clientId ? turn.data.recommended?.action === 'attack' : turn.data.recommended === null);

  const hints = await second.call('durak_hints', { seat: 'persist' });
  check('hints: учёт карт в подсказке', Number.isInteger(hints.data.hidden?.unseen));

  for (const seat of ['persist2', 'persist']) {
    const closed = await second.call('durak_disconnect', { seat });
    check(`disconnect: место ${seat} закрыто`, !closed.isError && closed.data.seat === seat);
  }
  const after = JSON.parse(fs.readFileSync(sessions(), 'utf8'));
  check('sessions: закрытые места удалены из файла', Array.isArray(after) && after.length === 0);
});
