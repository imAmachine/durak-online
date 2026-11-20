import { runMcpTest } from './helpers/mcp.mjs';
import { TestFailure } from './helpers/accounts.mjs';

function check(label, condition) {
  console.log(`${condition ? 'ok  ' : 'FAIL'} ${label}`);
  if (!condition) process.exitCode = 1;
}

await runMcpTest(async ({ start, account }) => {
  const { rpc, notify, call } = start();
  const init = await rpc('initialize', { protocolVersion: '2024-11-05', capabilities: {}, clientInfo: { name: 'tests/mcp.mjs', version: '1.0.0' } });
  check('initialize: server name', init?.serverInfo?.name === 'durak');
  check('initialize: tools capability', Boolean(init?.capabilities?.tools));
  notify('notifications/initialized');

  const list = await rpc('tools/list', {});
  const names = (list?.tools || []).map(tool => tool.name);
  for (const expected of ['durak_connect', 'durak_room_create', 'durak_room_join', 'durak_start', 'durak_action', 'durak_wait', 'durak_watch', 'durak_follow', 'durak_state', 'durak_hints', 'durak_turn', 'durak_admin', 'durak_autoplay']) {
    check(`tools/list: ${expected}`, names.includes(expected));
  }
  const readOnly = (list?.tools || []).filter(tool => tool.annotations?.readOnlyHint).map(tool => tool.name);
  for (const expected of ['durak_state', 'durak_hints', 'durak_turn', 'durak_seats', 'durak_watch']) {
    check(`tools/list: ${expected} помечен read-only`, readOnly.includes(expected));
  }
  const turnAnnotations = (list?.tools || []).find(tool => tool.name === 'durak_action')?.annotations;
  check('tools/list: durak_action помечен изменяющим', turnAnnotations?.readOnlyHint === false);

  const connect = async (seat, name) => {
    const credentials = await account(seat);
    const connected = await call('durak_connect', { seat, name, ...credentials });
    if (connected.isError || connected.data.connected !== true) throw new TestFailure(`durak_connect: место ${seat} не открыто`);
    credentials.connected = true;
    check(`connect ${seat}: своя сессия`, connected.data.clientId === credentials.clientId);
    return connected;
  };
  const first = await connect('p1', 'MCP Тест 1');
  check('connect p1: стартовый баланс', first.data.account?.balance === 1000);
  const second = await connect('p2', 'MCP Тест 2');
  check('connect: два разных аккаунта', first.data.clientId !== second.data.clientId);

  const duplicate = await call('durak_connect', { seat: 'p1' });
  check('connect: занятое место отклонено', duplicate.isError);

  const silentWait = await call('durak_wait', { seat: 'p1', until: 'finished', timeoutMs: 200 });
  check('wait: таймаут без ошибки', !silentWait.isError && silentWait.data.hit === false);

  const ids = { p1: first.data.clientId, p2: second.data.clientId };

  const created = await call('durak_room_create', { seat: 'p1', name: 'MCP проверка', bet: 0 });
  const code = created.data.code;
  check('room_create: код стола', !created.isError && /^[0-9A-F]{6}$/.test(code || ''));

  const joined = await call('durak_room_join', { seat: 'p2', code: code?.toLowerCase() });
  check('room_join: два игрока', !joined.isError && joined.data.state?.players?.length === 2);
  check('room_join: код в верхнем регистре', joined.data.state?.room?.id === code);

  const waiting = await call('durak_state', { seat: 'p1' });
  check('state: стол ждёт', waiting.data.state?.room?.status === 'waiting');
  check('state: рука пуста до старта', waiting.data.state?.hand?.length === 0);
  check('state: canStart у хоста', waiting.data.state?.canStart === true);

  const badAction = await call('durak_action', { seat: 'p1', action: 'attack', cardId: 'ZZ' });
  check('action: неизвестная карта отклонена', badAction.isError);

  const started = await call('durak_start', { seat: 'p1' });
  check('start: партия идёт', started.data.state?.room?.status === 'playing');
  check('start: по шесть карт', started.data.state?.players?.every(player => player.cards === 6));
  check('start: в колоде 24 карты', started.data.state?.deck === 24);

  const attackerSeat = started.data.state?.attackerId === ids.p1 ? 'p1' : 'p2';
  const hinted = await call('durak_hints', { seat: attackerSeat });
  check('hints: есть чем атаковать', hinted.data.attack?.length > 0);
  check('hints: рука в подсказке', hinted.data.hand?.length === 6);

  const defenderSeat = attackerSeat === 'p1' ? 'p2' : 'p1';
  const attackCard = hinted.data.attack[0].id;
  const watchPromise = call('durak_watch', { seat: defenderSeat, until: 'your-turn', totalMs: 8000, chunkMs: 3000 });
  await new Promise(resolve => setTimeout(resolve, 200));
  const attacked = await call('durak_action', { seat: attackerSeat, action: 'attack', cardId: attackCard });
  check('action: атака принята', !attacked.isError);
  const watched = await watchPromise;
  check('watch: хук сработал на ход соперника', !watched.isError && watched.data.hit === true);
  check('watch: ответ через серверный хук', watched.data.via === 'hook');
  check('watch: карта на столе', String(watched.data.snapshot?.table || '').includes(attackCard));
  check('watch: краткий ответ без полного снимка', watched.data.state === undefined && watched.data.snapshot?.room === code);
  check('watch: журнал действия', (watched.data.events || []).some(event => event.action === 'attack' && event.card === attackCard));
  check('watch: подсказки в отчете', watched.text.includes('Хук') && watched.text.includes('Рука:'));

  const lateWatch = await call('durak_watch', { seat: defenderSeat, until: 'your-turn', totalMs: 600, chunkMs: 600 });
  check('watch: ход уже был до вызова', !lateWatch.isError && lateWatch.data.hit === true);
  check('watch: ответ из накопленного состояния', lateWatch.data.via === 'cache');
  check('watch: без ожидания таймаута', Number(lateWatch.data.waitedMs) < 400);

  const taken = await call('durak_action', { seat: defenderSeat, action: 'take' });
  check('action: взять карты принято', !taken.isError);
  const backlog = await call('durak_watch', { seat: attackerSeat, until: 'any', totalMs: 600, chunkMs: 600 });
  check('watch: пропущенное действие отдаётся сразу', !backlog.isError && backlog.data.hit === true && backlog.data.via === 'cache');
  check('watch: пропущенное «взять» в журнале', (backlog.data.events || []).some(event => event.action === 'take'));

  const seatList = await call('durak_seats');
  check('seats: два места', seatList.data.seats?.length === 2);

  const lobby = await call('durak_lobby', { seat: 'p1' });
  check('lobby: стол виден', (lobby.data.rooms || []).some(room => room.id === code));

  const followTimeout = 120000;
  const followP1 = call('durak_follow', { seat: 'p1', until: 'finished', totalMs: 90000, maxSteps: 400 }, followTimeout);
  const followP2 = call('durak_follow', { seat: 'p2', until: 'finished', totalMs: 90000, maxSteps: 400 }, followTimeout);
  const [followed1, followed2] = await Promise.all([followP1, followP2]);
  check('follow: партия доиграна без вмешательства', followed1.data.finished === true || followed2.data.finished === true);
  check('follow: дурак определён', typeof (followed1.data.loser || followed2.data.loser) === 'string');
  check('follow: ходы в отчёте', (followed1.data.actions || []).length + (followed2.data.actions || []).length > 0);
  check('follow: краткий отчёт', followed1.text.length < 4000 && followed2.text.length < 4000);

  const adminList = await call('durak_admin', { op: 'list' });
  const adminIds = (adminList.data.players || []).map(player => player.id);
  check('admin list с ключом', !adminList.isError && adminIds.includes(ids.p1) && adminIds.includes(ids.p2));

  const status = await call('durak_status');
  check('status: сервис отвечает', !status.isError && Number.isInteger(status.data.online));

  const credentials = [];
  for (const label of ['auto1', 'auto2', 'auto3']) credentials.push(await account(label));
  const auto = await call('durak_autoplay', { credentials, seats: 3, mode: 'transfer', deckSize: 36, bet: 100, maxSteps: 400 });
  check('autoplay: партия завершена', auto.data.finished === true && auto.data.steps > 0);
  check('autoplay: дурак определён', typeof auto.data.loser === 'string' && auto.data.loser.length > 0);
  check('autoplay: стол и колода пусты', auto.data.deck === 0 && auto.data.table === 0);
  const balanceSum = (auto.data.balances || []).reduce((sum, item) => sum + (item.balance || 0), 0);
  check('autoplay: балансы сохранили банк', balanceSum === 3000);

  const afterAuto = await call('durak_seats');
  check('autoplay: места ботов закрыты', (afterAuto.data.seats || []).every(seat => !seat.seat.startsWith('auto')));

  const left = await call('durak_disconnect', { seat: 'p2', leave: true });
  check('disconnect: место закрыто', !left.isError && left.data.seat === 'p2');
  const leavingHost = await call('durak_disconnect', { seat: 'p1', leave: true });
  check('disconnect: хост закрыт', !leavingHost.isError);
  const empty = await call('durak_seats');
  check('seats: пусто после закрытия', (empty.data.seats || []).length === 0);

  const noSeat = await call('durak_state');
  check('state без места: понятная ошибка', noSeat.isError);
});
