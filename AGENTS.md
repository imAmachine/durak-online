# AGENTS.md

Browser multiplayer card game "Дурак" (Russian Durak). Plain Node.js HTTP server with
Server-Sent Events, plain browser JavaScript frontend. No framework, no bundler, no
dependencies, no build step. All user-facing text is Russian.

## Commands

```bash
npm start        # node server.js; PORT env (default 4173), HOST env (default 0.0.0.0)
npm test         # integration tests against a server already running on localhost:4173
```

- There is no lint, typecheck, format, or build step. There is no CI config.
- Tests are not self-contained: start the server first in another terminal, then run
  `npm test`. The base URL `http://localhost:4173` is hardcoded in every test file.
- `npm test` chains three `.mjs` scripts with `&&`, so the first failing suite stops the
  rest. Scripts exit non-zero on failure and print `ok`/`FAIL` lines.
- Tests use global `fetch` and top-level `await`, so they need Node 18+. No package
  dependencies exist; there is no lockfile.
- `/api/status` returns online clients and rooms; handy for checking a running server.

## Architecture

### Server (`server.js`, ~675 lines)

Single-file Node HTTP server. All state is in three in-memory structures:

- `clients: Map<clientId, { id, name, res, roomId }>` — one entry per open SSE stream.
- `rooms: Map<roomCode, room>` — room code is 6 uppercase hex chars.
- `history: []` — finished games, unbounded; lobby payload sends only the last 8 reversed.

Entry points:

- `GET /events?id=<clientId>&name=<name>` — the only real-time channel (SSE). Registers the
  client, attaches it to its room if it is already a player, sends an initial `game` or
  `noroom` event, then `lobby`. On disconnect the player is marked `connected: false` but
  stays in the room. The browser reconnects automatically (`retry: 2000`).
- `POST /api/rooms` — create room; the caller becomes host and first player.
- `POST /api/join` — join by code (case-insensitive); reconnecting existing players update
  name/connection instead of adding a new seat.
- `POST /api/action` — all gameplay actions, delegated to `act()`:
  `start | attack | defend | transfer | take | pass`.
- `POST /api/profile`, `POST /api/leave`, `GET /api/status`.
- Anything else is served as a static file from the repo root (`/` → `index.html`).

Every API response is JSON; errors are `{ error: "<Russian message>" }` with status 400/404.

Game flow inside `server.js`:

- `createRoom()` → `startRound()` (shuffle, deal 6, pick random attacker) → `act()`.
- `act()` validates and mutates the room, then calls `broadcastRoom()`.
- After a trick: `pass` (all cards beaten) or `collectTable()` (defender took cards) clears
  the table, rotates roles, calls `refillHands()`, then `settle()` → `normalizeRoles()` →
  `updateTurn()` → `broadcastRoom()`.

`gameState(room, clientId)` builds a per-viewer snapshot: own hand only, opponents' card
counts and roles, table, trump, deck size, derived permission flags
(`canAttack`, `canDefend`, `canTransfer`, `canPass`, `canTake`, `canStart`), and messages.
`broadcastRoom()` sends each player a snapshot tailored to their own id; there is no shared
broadcast payload, so hidden information never leaks by construction. Preserve that when
adding fields.

### Client (`app.js`, ~709 lines)

Single non-module script loaded at the end of `index.html`; all DOM ids it uses are defined
in `index.html`. Global mutable `state` object; server snapshots drive rendering
(`connect()` → EventSource handlers → `renderGame()` / `renderLobby()`).

- `connect()` opens `/events` with the client id from `sessionStorage['durak.client']`
  (generated locally, no `crypto.randomUUID` dependency). Lobby and game updates arrive as
  SSE messages with `type: lobby | game | noroom`.
- First `game` message auto-opens the game view (`state.autoOpen`); reopening later uses the
  "Вернуться в игру" button (`updateResumeButton`).
- `playCard()` performs client-side UX validation, then posts `attack`/`defend` (with
  `targetCardId` for a specific row). Server is authoritative; the client re-implements
  `beats()` only to give instant toasts.
- Rendering uses `innerHTML` with template strings; every user-controlled value is passed
  through `esc()`. Keep doing that.
- Profile lives in `localStorage['durak.profile']`, theme in `localStorage['durak.theme']`
  (applied via `document.documentElement.dataset.theme`). Card identity is per-tab
  (`sessionStorage`), so two tabs are two players.
- Invite links are `/?code=XXXXXX`; `openJoinModal(code)` pre-fills the join modal.

### Styles (`style.css`)

One stylesheet with a unified token system: deep felt green, warm ivory, and brass accents.
Dark is the default (set on `<html data-theme="dark">`; `storedTheme()` also falls back to
dark), and light overrides live in `[data-theme="light"]`. `applyTheme()` persists the
choice and syncs `<meta name="theme-color">`. Display type is Cormorant Garamond, UI type
is Manrope, both loaded from Google Fonts with Georgia/system fallbacks. Responsive
layouts and `prefers-reduced-motion` rules live near the end of the stylesheet.

The hero banner and playing table are intentionally dark in both themes, so they use the
fixed `--felt-ink` / `--felt-brass` tokens instead of `--ivory` / `--brass` (those flip in
light mode and would become unreadable). Table geometry is hand-tuned: seats are placed by
`.pos-1`…`.pos-5`, the deck sits at the lower left, and table cards form one nowrap row with
negative pair margins. Keep seats, deck, cards, message, and hand non-overlapping; the
current layout is verified from 320px to 1600px. On narrow screens the hero card fan moves
into a bottom band, which relies on the extra bottom padding on `.hero-banner`.

New hand/table cards animate with the Web Animations API only when they arrive, not on
every SSE update. Modal focus is trapped, background content becomes inert, and focus
returns to the trigger on dismissal. Preserve these keyboard and reduced-motion behaviors.

## Game model details (non-obvious)

- Suits: `♠ ♥ ♦ ♣`. Ranks are Russian: `6 7 8 9 10 В Д К Т` (36-card deck) plus `2 3 4 5`
  for the 52-card deck. Card ids are `rank + suit`, e.g. `Т♠`.
- Card strength is the index in `RANKS_52`; `VALUES` is built from the full 52-rank list so
  both deck sizes share one ordering. The client duplicates this as `RANK_ORDER` in
  `app.js`; keep the two lists identical.
- `buildDeck()` orders suits first, ranks second. `startRound()` keeps `deck[0]` as the
  visible trump card and does not remove it: cards are dealt with `pop()` from the end, so
  the trump card is dealt last. `syncTrumpCard()` nulls the display once the deck empties.
- Deal: six rounds, one card per player per round in seat order.
- `refillHands()` refills the attacker first, then clockwise, defender last.
- `settle()` only marks players finished when the deck is empty and their hand is empty.
  The last remaining active player is the durak; the game ends with `status: 'finished'`
  and `finished.loser`. Restart is the ordinary `start` action (host, ≥2 connected).
- Attack: only `attackerId`; cards must match a rank already on the table; max 6 rows.
  Defense: only `defenderId`, card must beat the chosen unattacked row. Beat rule:
  same suit and higher rank, or trump beats non-trump.
- `take` is two-phase: the defender sets `takePending`, the attacker may keep adding cards
  of the table rank, then the attacker's `pass` calls `collectTable()` and the defender
  picks everything up. Calling `take` twice is an error.
- `transfer` exists only in `transfer` mode: the defender may play a card matching the
  single table rank while nothing is beaten, becoming the new attacker and pushing the
  defense to the next active player.
- Turn order is the array order of `room.players`; `nextActive()` skips finished players.
- Limits: `MAX_PLAYERS = 6`, `MAX_TABLE_CARDS = 6`; player names truncated to 20 chars,
  room names to 40.

## Gotchas

- **SSE connection is a prerequisite for every POST.** Requests look up `clients.get(clientId)`
  and return "Нет соединения с сервером, обновите страницу" if the stream is not open. This
  is why tests wait ~300–400 ms after opening `/events` before calling the API.
- **Identity is client-supplied.** `clientId` comes from the query string/sessionStorage and
  is not authenticated or validated beyond SSE registration. Do not treat it as a security
  boundary.
- **Everything is in memory.** Restarting the server loses rooms, players, and history. A
  30-second interval deletes rooms with no connected players (immediately if not playing, or
  once the room is older than 60 seconds) and finished rooms older than 30 minutes.
- **Some rules live only in the permission flags.** `gameState()` contains constraints that
  `act()` does not re-check (for example, `canAttack` requires the defender to hold at least
  as many cards as the table rows, but `act('attack')` only enforces the 6-row cap and rank
  match). When changing a rule, update `gameState()`, the corresponding `act()` branch, the
  client pre-check in `playCard()`, and duplicate rank/beat helpers as needed.
- **`canStart` requires the host plus two connected players**; `start` filters the roster to
  connected players only. `join` is rejected while `status === 'playing'`.
- **Leaving during play** clears the table, transfers host if needed, and normalizes roles;
  if fewer than two players remain the game ends immediately with the survivor as loser.
- **Asset cache busting is manual.** Script/style URLs in `index.html` carry `?v=N`
  (`app.js?v=7`, `style.css?v=7`). Bump the matching version when changing those files, or
  browsers may serve stale assets (static files are served without cache headers).
- **Russian everywhere.** UI copy, server messages, README, and test labels are Russian.
  New user-facing strings should be Russian.
- **Style:** 2-space indentation, semicolons, single quotes, arrow functions, `const`/`let`;
  comments are sparse and Russian. No external packages are used; prefer adding none.
- UI work: the repo includes a design skill at `.crush/skills/frontend-design/SKILL.md`.

## Testing notes

- Tests are scenario/integration scripts, not unit tests:
  - `tests/moves.mjs` — lobby, join by lower-case code, turn order, attack/defend,
    transfer, two-phase take, leave.
  - `tests/rules-fixes.mjs` — beat rules (including trump), multi-card attacks, adding a
    card after "take", repeated over up to 30 randomized attempts because hands are random.
  - `tests/full-game.mjs` — simulates a full 2-player game step by step (also checks deck
    conservation), then restarts and asserts 6-card hands and a 24-card deck.
- Each script has its own SSE helper, `check(label, condition, extra)` reporter, unique
  client ids (`Date.now()` prefix), and fixed `setTimeout` waits. Randomized hands mean the
  tests may skip a scenario or need several attempts; do not assume deterministic decks.
- Tests mutate the running server: they create rooms and players and add entries to history.
  Run them against a throwaway server instance.
- After a failed assertion, later actions can depend on state that never changed, so read
  the first failure rather than the last.