# REFACTOR_REPORT.md — Дурак Онлайн / BomzhClub

## Результат

Серверный runtime перенесён с JavaScript на Rust. Production backend запускается через `platform-server`, MCP через `platform-mcp`, а JavaScript оставлен во frontend и тестовых helper-скриптах. Node backend, Node MCP и reference engine удалены после переноса покрытия в Rust.

## Выполненная работа

- Перенесены игровые правила, API, SQLite persistence, SSE/watch, авторизация, клубные аккаунты, админка и MCP strategy/automation.
- Переведены `npm start`, `npm run mcp`, `.crushrc` и тестовые helpers на Rust.
- Добавлены Rust tests для engine, API, storage, SSE, MCP strategy, sessions, follow и service flow.
- Service fixture запускает заранее собранный Rust backend на динамическом порту с временной SQLite и MCP session-файлом.
- Исправлено удаление аккаунта из активной партии: живой room snapshot сохраняется, поэтому оставшийся игрок может завершить выход без ошибки sequence consistency.
- Устранены строгие Clippy warnings в тестовом коде.

## Проверки

Прошли:

- `cargo build --workspace --locked`.
- `cargo check --workspace --all-targets --locked`.
- `cargo clippy --workspace --all-targets --locked -- -D warnings` после адресных исправлений.
- `cargo test --workspace --locked` с `DURAK_SERVER_BIN`.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`.
- `cargo test -p platform-api --lib --locked`, включая regression для трёхместного take/pass.
- MCP service integration с Rust backend.
- Полный `npm test` на временном Rust backend и SQLite, включая трёхместный autoplay.
- Browser matrix на временном backend: `/`, `/admin`, ошибочный вход, admin login и клавиатурная навигация.
- 20 параллельных authenticated SSE streams получили initial events.
- Проверка ссылок на удалённые server-side JS-файлы.

## Открытые ограничения

- Nginx не установлен в окружении, поэтому реальный proxy/HTTPS прогон невозможен; конфигурация `docs/nginx.conf` проверена содержательно.
- Production Discord OAuth намеренно не проверялся.
- Масштабирование несколькими backend processes не поддерживается: комнаты, SSE clients и event coordination находятся в памяти одного процесса.
- Масштабирование несколькими backend processes не поддерживается: комнаты, SSE clients и event coordination находятся в памяти одного процесса.

Исторические baseline и подробности волн находятся в `REFACTOR_PLAN.md`.
