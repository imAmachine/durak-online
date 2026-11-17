# План миграции движка, API и MCP на Rust

Статус: подготовка, ветка `prep/rust-platform`.

## Решение

Переносим на Rust:

- доменное ядро игр;
- application services для столов, партий, аккаунтов и расчётов;
- совместимый HTTP API, SSE и `/api/watch`;
- MCP stdio-сервис и его tools.

Оставляем на JavaScript:

- `index.html`;
- `app.js`;
- `style.css`;
- существующий browser wire contract.

Node.js остаётся работающим reference implementation до завершения parity-проверок. Big bang migration не используется.

## Целевой стек

| Область | Решение | Причина |
|---|---|---|
| Runtime | stable Rust + Tokio | async SSE, long-poll и таймеры |
| HTTP | Axum + Tower | маршруты, middleware, SSE и cancellation |
| JSON | Serde | типизированные DTO при сохранении JSON-контракта |
| Persistence | SQLite через `rusqlite` в single-writer task | один файл, транзакции, WAL и простой deployment |
| SQL migrations | версионированные SQL-файлы | схема и миграции рядом с кодом |
| Logging | tracing + tracing-subscriber | structured logs и request id |
| MCP | собственный stdio JSON-RPC adapter на Tokio | сохранить текущий протокол без обязательной зависимости от MCP SDK |
| Tests | cargo test, Axum service tests, reqwest black-box tests | unit, contract и real SSE coverage |

Redis и внешняя распределённая СУБД не входят в первый production этап. SQLite остаётся источником истины для аккаунтов, партий, истории и финансов. SQLite рассчитан на один основной экземпляр Rust-сервиса; при переходе к нескольким writer-инстансам потребуется отдельная архитектура координации.

## Целевая структура workspace

```text
rust/
  Cargo.toml
  crates/
    game-core/       # общие типы команд, событий и game registry
    durak-engine/    # правила подкидного и переводного Дурака
    platform-domain/ # users, sessions, tables, matches, settlements
    platform-app/    # use cases, repositories, idempotency
    platform-api/    # Axum routes, legacy DTO, SSE and watch
    platform-mcp/    # stdio JSON-RPC and HTTP client/tools
    platform-server/ # binary, config, migrations and static assets
  migrations/
```

`game-core` не содержит HTTP и SQL. `durak-engine` не знает об аккаунтах и соединениях. `platform-api` не меняет игру напрямую. `platform-mcp` не реализует правила и не является источником истины.

## Доменная модель

Общие типы:

- `UserId`, `SessionId`, `TableId`, `MatchId`, `CommandId` — новые типы вместо строк;
- `GameType`, `RulesVersion`, `MatchStatus`;
- `Table` — lobby metadata и места;
- `Match` — конкретная партия и её lifecycle;
- `GameState` — типизированное состояние конкретной игры;
- `GameCommand` — команда игрока с `expected_seq` и `command_id`;
- `GameEvent` — публичное событие с `(match_id, seq)`;
- `ViewerSnapshot` — projection, собранная для конкретного пользователя;
- `Settlement` — одноразовый итог с idempotency key.

Durak engine:

- `Card`, `Rank`, `Suit`, `Deck`, `TableRow`;
- `Classic` и `Transfer` rules;
- `apply(command, state, context) -> Transition`;
- `Transition` содержит новый state, public events и только settlement intent;
  engine не меняет balance, ledger, accounts или history;
- невалидная команда возвращает ошибку без изменения state;
- RNG и clock передаются через traits для детерминированных тестов;
- public projection никогда не содержит чужие карты.

Будущая игра добавляется отдельным crate, реализующим registry contract. Общий `Match` не получает поля Durak вроде `trump`, `defender` или `take_pending`.

## Совместимый контракт

Сохраняются legacy routes и формы запросов:

- `GET /events?id=&token=&name=`;
- `GET /api/watch?id=&token=&since=&until=&timeout=`;
- `POST /api/rooms`;
- `POST /api/join`;
- `POST /api/action`;
- `POST /api/profile`;
- `POST /api/leave`;
- `POST /api/admin`;
- `GET /api/status`.

Сохраняются типы SSE `account`, `game`, `lobby`, `noroom`, формат `data: JSON\n\n`, `retry: 2000`, русские ошибки, статус-коды, ключи snapshots, permission flags и ограничение viewer-specific hand.

Legacy `room.seq` остаётся непрерывным cursor стола и используется текущими SSE/watch
клиентами. Новый `match_seq` принадлежит партии, а `lobby_seq` — изменениям стола;
они не смешиваются с legacy cursor. `streamSeq` MCP — локальный счётчик входящих SSE frames.
Legacy `/api/watch` сохраняет ограниченный журнал без гарантии полного replay. В новом
API cursor за пределами журнала возвращает `resync_required: true` и актуальный
viewer-specific snapshot. Legacy `/api/watch` возвращает `hit: false` при timeout
и поддерживает `any`, `game`, `your-turn`, `opponent`, `finished`.

Новые внутренние поля:

- `command_id` для повторяемости команд;
- `expected_seq` для защиты от stale actions;
- `rules_version` для воспроизводимости партий;
- `request_id` для трассировки.

Они добавляются через внутренний DTO или optional fields, чтобы старый JS-клиент продолжал работать. Legacy-запрос не содержит стабильного `command_id` или `expected_seq`: серверная генерация идентификатора не обеспечивает дедупликацию повторной отправки; эти гарантии доступны только клиентам нового контракта. Публичные breaking changes идут только в `/api/v2`.

## Порядок миграции

### Этап 0. Контракт и baseline, 2–3 дня

1. Зафиксировать JSON fixtures для `gameState`, `roomSummary`, SSE и watch.
2. Прогнать текущие 9 suites на чистом Node reference server.
3. Добавить проверки порядка JSON-событий, viewer isolation и повторной команды.
4. Зафиксировать версии правил и таблицу legacy error/status mapping.

Готово, когда baseline повторяется и не зависит от задержек `setTimeout`.

### Этап 1. Чистый Durak engine, 6–9 дней

1. Реализовать типы карт и колоды.
2. Перенести старт раунда, `can_beat`, attack/defend/transfer/take/pass.
3. Перенести refill, settle, leave и payout intents без базы и HTTP.
4. Написать unit/property tests и golden parity fixtures.

Готово, когда engine тестируется одним `cargo test`, не открывает порт и не имеет глобального mutable state.

### Этап 2. Rust memory application, 3–5 дней

1. Добавить table/match lifecycle и session compatibility.
2. Добавить in-memory repositories и actor или single-writer match manager.
3. Вынести broadcast intents из engine; никогда не держать lock во время socket write.
4. Добавить monotonic seq, bounded event journal и cancellation-aware watch.

Готово, когда application tests покрывают конкурентные команды одного match и повторный `command_id`.

### Этап 3. Rust compatibility API, 4–6 дней

1. Реализовать Axum routes и static allowlist.
2. Реализовать per-viewer SSE snapshots и reconnect replacement.
3. Реализовать `/api/watch` с timeout, disconnect cancellation и replay cursor.
4. Запустить Rust на `PORT=4174` и прогнать текущие black-box suites через `BASE_URL`.

Готово, когда JS-фронтенд работает без изменений и проходят security, moves, hooks,
money и full-game tests. Это необходимый compatibility gate, но не достаточный production
критерий: дополнительно обязательны crash/restart recovery, transaction rollback,
повтор command/event/settlement, stale cursor и конкурентные команды.

### Этап 4. Rust MCP, 4–7 дней

1. Переписать `Seat`, SSE parser, reconnect, backlog и seen cursor.
2. Перенести 18 tool schemas и русские summaries.
3. Перенести `watch`, `wait`, `follow`, `autoplay` как orchestration над API.
4. Убрать rules mirror из MCP; hints приходят от нового application API с той же версией правил, что и партия.
5. Сохранить stdio JSON-RPC framing и `.crushrc` tool names.

Готово, когда адаптированный `tests/mcp.mjs` запускает Rust MCP и работает против Rust API. Существующий тест жёстко запускает Node MCP и сам по себе не проверяет Rust MCP. Восстановление durable session проверяется после этапа SQLite; memory-сессии при рестарте теряются.

### Этап 5. SQLite, 4–6 дней

1. Добавить migrations для users, sessions, profiles, tables, matches, participants, events, wallets и ledger.
2. Сохранять match state с optimistic locking по `seq`.
3. Сделать state, event и settlement одной транзакцией с уникальным
   `operation_id` на каждую операцию (`stake`, `payout`, `refund`, `admin_adjustment`);
   повтор той же операции должен быть no-op.
4. Добавить restart recovery, cleanup jobs и migration tests.
5. Скрыть legacy anonymous accounts за feature flag.

Готово, когда restart не теряет завершённые партии, баланс и историю, повтор
settlement не меняет результат, а failure-injection тесты покрывают сбой до commit,
после commit и до публикации SSE.

### Этап 6. Переключение и развитие, 2–4 дня

1. Сравнить Rust и Node на parity suite и нагрузочном сценарии SSE.
2. Направить MCP и browser traffic на Rust.
3. Оставить Node только как rollback reference на ограниченное время.
4. Включить новые account endpoints и затем registry для следующих игр.

## Тестовая стратегия

- `game-core` и `durak-engine`: unit, property, deterministic seeded tests;
- `platform-app`: repository contract tests, concurrency, idempotency;
- `platform-api`: DTO/status/SSE/watch contract tests;
- black-box: существующие `tests/security.mjs`, `moves.mjs`, `rules-fixes.mjs`, `money.mjs`, `full-game.mjs`, `hooks.mjs` через `BASE_URL`;
- MCP: отдельный Rust process test по stdio и `tests/mcp.mjs`, адаптированный для запуска Rust-бинарника;
- SQLite: disposable database migrations, WAL, transaction rollback and restart recovery;
- load: SSE fan-out, slow consumer, reconnect storm and concurrent commands.

Случайная раздача не сравнивается побайтно с Node. Сравниваются состояния на фиксированных
pre-dealt fixtures, legal/illegal transitions, публичные snapshots и settlement outcomes.
Отдельно проверяются production RNG, уникальность карт, 36/52 карты, `rules_version` и
воспроизводимость replay. Hints реализуются application command того же engine/rules_version;
MCP не вычисляет legal moves, а contract test сравнивает hints с engine для всех режимов.

## Решение о завершении миграции

Миграция оправдана только после получения durable accounts, повторяемых settlement, restart recovery или измеримого требования по многопоточному SSE. Если эти критерии не нужны, Node reference и модульный JS engine остаются более дешёвым вариантом. Rust-перенос не должен быть объявлен завершённым только потому, что новый бинарник запускается.
