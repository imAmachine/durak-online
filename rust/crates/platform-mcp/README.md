# platform-mcp

Rust MCP adapter запускается из каталога `rust`:

```bash
DURAK_URL=http://127.0.0.1:4173 cargo run -p platform-mcp
```

Сервис обменивается JSON-RPC по одной JSON-строке через stdin/stdout, подключается к HTTP/SSE API через `DURAK_URL`, поддерживает места игроков, комнаты, игровые действия, ожидание событий, hints, turn, follow, autoplay и административные операции. Схемы tools совместимы с Node MCP.

Политика ходов и учёт вышедших карт живут в `src/strategy.rs` (`attach_options`, `defense_options`, `transfer_options`, `dump_options`, `attack_choices`, `choose_move`, `Ledger`) и совпадают с `mcp/strategy.mjs`, поэтому Node и Rust адаптеры принимают одинаковые решения. `durak_hints` и `durak_turn` возвращают `hidden` со счётчиками скрытых карт и козырей, а `durak_turn` добавляет готовый `recommended` ход с обоснованием. Меняя правила, правьте оба файла.

Для работы сначала запустите `platform-server` командой `npm start` из корня проекта. Node `mcp/server.mjs` остаётся reference-клиентом и запускается через `npm run mcp`. Старый Node backend запускается отдельно командой `node server.js` только для сравнения и тестирования; он несовместим с обязательным входом через Discord и текущим браузерным входом.

Для доступа к Rust API нужны заранее авторизованные сессии подтверждённых аккаунтов Discord. Для сборки MCP и сервера учётные данные Discord не нужны. Без настройки Discord на сервере новый вход недоступен.

Для `durak_connect` задайте `DURAK_DISCORD_CLIENT_ID` и `DURAK_DISCORD_TOKEN` существующей сессии после входа через Discord. Для нескольких мест задайте `DURAK_DISCORD_ACCOUNTS` как JSON-объект, где ключ — метка места, а значение — объект `{"clientId":"...","token":"..."}`. Для `durak_autoplay` обязательны отдельные аккаунты с ключами `auto1`, `auto2` и далее. MCP не создаёт аккаунты. Токен сессии отличается от токена Discord. Храните переменные окружения как секреты. Rust backend проверяет сессии при подключении SSE и отклоняет их без подтверждённого аккаунта Discord. Legacy Node backend по-прежнему допускает гостевые аккаунты и не подтверждает Discord.

Node MCP также принимает пары `clientId` и `token` в `durak_connect` или переменные `DURAK_CLIENT_ID` и `DURAK_TOKEN`; для autoplay принимает массив `credentials` или JSON-массив `DURAK_ACCOUNTS`.

Интеграционный тест MCP использует legacy Node backend и тестовые пары идентификаторов и токенов. Он проверяет игровой обмен, но не проверяет вход или верификацию Discord.
