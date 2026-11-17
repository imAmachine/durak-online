# platform-mcp (незавершённый каркас)

Запуск: `cargo run -p platform-mcp` из каталога `rust`. Сервер обменивается JSON-RPC по одной JSON-строке на stdin/stdout, поддерживает initialize, ping и tools/list. Схемы инструментов перенесены из Node MCP.

Все tools/call возвращают ошибку «не реализован». HTTP-клиент, SSE, состояние мест и игровые действия ещё не перенесены. Этот каркас не заменяет действующий `mcp/server.mjs` и не взаимодействует с Rust API.
