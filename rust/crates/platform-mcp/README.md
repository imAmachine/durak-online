# platform-mcp

Rust MCP-сервер общается с клиентом по stdio (JSON-RPC, одна строка на сообщение) и использует HTTP/SSE API игрового backend. Из корня проекта:

```bash
cargo run --manifest-path rust/Cargo.toml -p platform-server
cargo run --manifest-path rust/Cargo.toml -p platform-mcp
```

Вторую команду запускайте в отдельном терминале или настройте как команду MCP-клиента. `npm run mcp` и проектный `.crushrc` также запускают Rust MCP. `DURAK_URL` задаёт адрес backend, по умолчанию `http://127.0.0.1:4173`.

`durak_connect` принимает `clientId` и `token` готовой сессии, `login` и `password` клубного аккаунта или `create=true` с ключом администратора `key`, `DURAK_ADMIN_KEY` либо `ADMIN_KEY`. Клубный вход также поддерживает `DURAK_LOGIN` и `DURAK_PASSWORD`. Для именованных сессий доступен JSON-объект `DURAK_DISCORD_ACCOUNTS` с парами `clientId` и `token`, либо переменные `DURAK_DISCORD_CLIENT_ID` и `DURAK_DISCORD_TOKEN` для одной сессии. Токен приложения не является токеном Discord.

`durak_autoplay` принимает отдельные сессии в `credentials` или JSON-массиве `DURAK_ACCOUNTS`, использует именованные сессии `auto1`, `auto2` и далее из `DURAK_DISCORD_ACCOUNTS` либо создаёт клубные аккаунты при наличии ключа администратора. Берегите ключи и токены как секреты.

Метки, идентификаторы и токены открытых мест сохраняются в `DURAK_MCP_SESSIONS`, по умолчанию `~/.local/state/durak-mcp/sessions.json`, и восстанавливаются при запуске. Состояние игры не сохраняется в этом файле и приходит по SSE. Сохранённые токены дают доступ к аккаунтам: ограничьте доступ к файлу и каталогу.

Инструменты включают управление местами и столами, действия, подсказки и рекомендации, ожидание событий, `follow`, `autoplay` и административные операции. Текущая политика ходов находится в `src/strategy.rs`.

Рабочий backend и MCP запускаются на Rust; JavaScript используется только браузерным frontend и тестовыми helper-скриптами.
