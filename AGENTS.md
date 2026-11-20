# AGENTS.md

Browser multiplayer card game "Дурак" (Russian Durak). Rust HTTP server with
Server-Sent Events, plain browser JavaScript frontend. Rust `platform-server` and
`platform-mcp` are the backend and MCP implementations. All user-facing text is Russian.

## Commands

```bash
npm start        # Rust platform-server; PORT env (default 4173), HOST env (default 0.0.0.0)
npm run mcp      # Rust platform-mcp; MCP stdio server over the HTTP API
TEST_BACKEND=rust npm test
TEST_BACKEND=rust BASE_URL=http://127.0.0.1:4174 npm test
```

- There is no lint, typecheck, format, or build step. There is no CI config.
- Tests are not self-contained: start the Rust `platform-server` first in another terminal, then
  run `npm test`. The helpers reject every `TEST_BACKEND` value except `rust`; integration
  tests use `BASE_URL` when set, otherwise `http://localhost:4173`. These commands document
  the available procedure; they do not claim that an integration run has passed.
- `npm test` chains ten `.mjs` scripts with `&&`, so the first failing suite stops the
  rest. Scripts exit non-zero on failure and print `ok`/`FAIL` lines.
- Tests use global `fetch` and top-level `await`, so they need Node 18+. No package
  dependencies exist; there is no lockfile.
- `/api/status` returns online clients and rooms; handy for checking a running server.

## Architecture

### Rust backend

The backend is `rust/crates/platform-server` with `platform-api`; gameplay rules are
implemented by `rust/crates/durak-engine`.


The Rust backend stores rooms and accounts in SQLite and maintains active connections in memory.


- `GET /api/watch?id=<id>&token=<token>&since=<seq>&until=<mode>&timeout=<ms>` — the
  server-side hook: a long-poll that returns as soon as the room's event sequence passes
  `since` and `until` matches (`any | game | your-turn | opponent | finished`), otherwise
  `{ hit: false }` after `timeout` (200-60000 ms). It answers with the journal slice, the
  room sequence, and a per-viewer snapshot, so an agent can wait for its turn without
  polling or chat messages. The current room journal keeps up to 40 events and this
  endpoint returns at most the last 12; an old `since` value does not guarantee a full
  replay.
- `POST /api/profile`, `POST /api/leave`, `POST /api/admin`, `GET /api/status`.
- Authenticated gameplay POSTs carry `clientId` and `body.token`; any POST supplying a
  `clientId` is also checked against `sessions` (including admin requests).
- Static serving is allowlisted to `index.html`, `admin.html`, `app.js`, and `style.css`
  (`/` → `index.html`, `/admin` → `admin.html`).

Players log in with the name and password of a club account through
`POST /api/auth/temp/login`, which answers `{ clientId: "temp:...", token }`; the token is a
hex digest (64 lowercase hex chars) and every SSE/POST keeps using that shape. Admins mint
accounts with `POST /api/admin` and `op: create_temp` (name + password, 4-128 chars). Startup
accepts a saved playing room when each player is either a linked Discord identity or a
`temp:` account with a stored password hash.

`/api/admin` is a single POST endpoint with
`op: list | create_temp | update | reset | balance | delete` and a `key` body field checked
against `ADMIN_KEY` (env; unset disables the panel). It can create club accounts (password
optional, generated when omitted), rename players, set balance/stats, reset to the starting
balance, change a temporary account's password (which also revokes its sessions and kicks the
live stream with `{ type: 'noroom', reason: 'auth' }`), and delete an account together with its
identity row and finished game snapshots. `list` rows carry `kind` (`temp | discord | legacy`)
and `hasPassword`. Temporary passwords are stored as salted PBKDF2-SHA256 hashes through
`password.rs`; the old unsalted SHA-256 hashes (`sha256$<hex>`) still authenticate and are
upgraded on the next login. Deleting a player who still sits at a playing table is 409; errors
are 403 for a wrong key, 404 for an unknown player.

API responses are JSON; errors use `{ error: "<Russian message>" }` with status 400/403/404,
or 503 when the admin panel is disabled.

The engine validates actions and manages dealing, tricks, refill, and settlement. The API
persists transitions, accounts, and history, then publishes per-player snapshots and public
events. Snapshots include only the viewer's hand; preserve this privacy when adding fields.
Stakes are 0, 50, 100, 250, or 500; the starting balance is 1000. Offline cancellation
refunds stakes without recording a game.

### Client (`app.js`, 2329 lines)

Single non-module script loaded at the end of `index.html`; all DOM ids it uses are defined
in `index.html`. Global mutable `state` object; server snapshots drive rendering
(`connect()` → EventSource handlers → `renderGame()` / `renderLobby()`).

- `connect()` opens `/events` with the client id from `sessionStorage['durak.client']` and the
  64-hex session token from `sessionStorage['durak.token']`
  (generated locally, no `crypto.randomUUID` dependency). Lobby and game updates arrive as
  SSE messages with `type: lobby | game | noroom`.
- First `game` message auto-opens the game view (`state.autoOpen`); reopening later uses the
  "Вернуться в игру" button (`updateResumeButton`).
- `playCard()` performs client-side UX validation, then posts `attack`/`defend` (with
  `targetCardId` for a specific row). Server is authoritative; the client re-implements
  `beats()` only to give instant toasts.
- `state.account` mirrors the server wallet/stats. The server pushes
  `{ type: 'account' }` on connect and after every room broadcast, so the sidebar balance
  and the profile modal use it directly; do not duplicate balance locally.
- Admin panel lives on its own page (`admin.html`, served at `/admin`). `body[data-admin="page"]`
  switches the script to admin mode (`bootstrapAdminPage()`); the key is asked once per tab and
  kept in `sessionStorage['durak.admin']`, and `renderAdminPage()` / `renderAdminEditor()` list
  accounts and edit them through `/api/admin`. The main app has no admin entry point.
- Personal cabinet: the `data-view="cabinet"` nav item (and the sidebar profile button) shows
  `#cabinetView` through `renderCabinet()` with balance, stats, club history and profile/logout
  actions; `signOut()` clears the session keys and returns to the login screen.
- Hand cards sit in `.hand-slot` wrappers so the hover lift never moves the hover target:
  `.hand-slot:hover .hand-card` raises the card inside a box that stays put, which is what
  stopped the up/down flicker when the cursor rested on the lower part of a card. The row is
  rebuilt only when `#myHand`'s signature (card ids, dim flags, trump, room) changes, so an
  opponent's move no longer recreates the cards under the cursor.
- `fitHand()` computes one `--overlap` for the hand row (a visible step of at least
  `max(24, min(44, cardWidth * .33))`, so heavy hands overlap without becoming unreadable)
  and, when even the minimum step does not fit, sets `zoom` on `#myHand`; a scrollable hand
  (`overflow-x: auto` below 760px) is spaced for reading instead of being zoomed. `fitTable()`
  does the same for up to six table pairs against the row cap in `--row-max` (88%, 96% below
  760px). Both measure the parent box rather than clearing `zoom` first and only write changed
  values, so a render never makes the layout jump or oscillate. Call both after rendering and
  on resize.
- Rendering uses `innerHTML` with template strings; every user-controlled value is passed
  through `esc()`. Keep doing that.
- Profile lives in `localStorage['durak.profile']`. The interface uses one fixed dark theme. Card identity is per-tab
  (`sessionStorage`), so two tabs are two players.
- Invite links are `/?code=XXXXXX`; `openJoinModal(code)` pre-fills the join modal.

### Rust MCP

The production MCP is `rust/crates/platform-mcp`; its strategy is in `src/strategy.rs`.

A dependency-free MCP server on stdio (newline-delimited JSON-RPC) that drives the HTTP API,
so an agent can open seats, play, and exercise the service. Project `.crushrc` registers it as
`durak` with `cargo run --manifest-path rust/Cargo.toml -p platform-mcp --locked`; `DURAK_URL` (default `http://127.0.0.1:4173`) selects the
service and `DURAK_ADMIN_KEY`/`ADMIN_KEY` supplies the club key. Nothing needs the repo cwd:
the tools only speak HTTP.

`src/strategy.rs` contains strategy and card accounting, covered by Rust unit tests.

- `Seat` holds one player: a 64-hex session `token`, an open SSE request to `/events`, and the
  latest `account`/`game`/`lobby` payloads plus a `Ledger`. It reconnects every second while
  `wantOpen`, and `open()` resolves only once the stream is registered, because every gameplay
  POST is rejected until then. Seats live in the module-level `seats` map keyed by tool label
  (`p1`, `auto1`, …); `seatFromArgs` defaults to the only open seat.
- Seats survive an MCP restart: `saveSessions()` writes `{label,name,id,token}` to
  `DURAK_MCP_SESSIONS` (default `~/.local/state/durak-mcp/sessions.json`, mode 0600) on connect
  and disconnect, and `restoreSessions()` reopens them in parallel before the first request is
  handled, so the SSE streams reattach on their own. Stale entries are dropped.
- `durak_action` posts one action and returns a fresh snapshot. Rejected actions are reported without retrying or substituting another action.
- `durak_watch` (and `durak_wait`) drive the server hook `/api/watch`: they long-poll with
  the seat's last `game.seq`, so the call returns the moment the condition matches and
  replays the events the seat missed. `durak_watch` loops chunks for up to `totalMs`
  (default 180 s) and reports who played what plus hints when it is your turn; `durak_wait`
  is the single short wait. Without a room both fall back to the SSE stream. `.crushrc`
  raises the MCP call timeout to 300 s so the long wait is not cut off.
- A seat keeps its own SSE journal plus a `seenSeq` cursor: tools that hand the model a
  snapshot mark it seen, so `durak_watch`/`durak_wait` answer at once (`via: 'cache'`) when
  the move arrived before the call. They replay the room events already sitting in the
  journal instead of sleeping until the timeout, and return immediately when `your-turn` or
  `finished` already holds in the current snapshot. `durak_follow` uses the same catch-up,
  so a move that lands between its own actions is never lost and never costs a full poll.
- `durak_follow` plays one seat to the end on its own: `chooseMove()` decides, the loop acts,
  then waits on `/api/watch` (`your-turn`) for the next decision. Two concurrent calls on two
  seats finish a whole game, which is how `tests/mcp.mjs` covers it. A no-move state that also
  produces no new events backs off and gives up after three spins (`reason: 'no-move'`) instead
  of busy-looping until the deadline. It returns one compact report: outcome, own move count,
  the last events, and the final snapshot.
- `chooseMove` policy: beat with the cheapest non-trump card; transfer instead of taking when a
  transfer card exists and no cheap defense does; dump a non-trump card when the defender has
  already taken; never feed trumps mid-deck; spend a last trump only when the deck is empty,
  three rows are unbeaten, or at most one unseen trump remains.
- `Ledger` reconstructs card accounting from the public journal: it marks cards seen (own hand,
  trump card, table) and cards dead (a `pass` that cleared a fully beaten table), and records
  who took a row. Snapshots expose `hidden: { unseen, trumps, out }` and the text report prints
  `Скрыто N: в колоде K, в чужих руках M · козырей не видел T · вне игры D`. It replays only
  events above the last applied `seq`, so observing the same snapshot twice is harmless.
- `durak_hints` lists attackable, defending, transferable, and (under `takePending`) dumpable
  cards from the same helpers, without a server round trip.
- `durak_turn` answers one call instead of `durak_state` + `durak_hints`: flags, legal moves,
  card accounting, and `recommended` with the reason the policy picked it. It is read-only.
- Tools carry MCP `annotations` (`readOnlyHint` for `state/hints/turn/seats/lobby/status/watch`),
  so a client can auto-approve the reads. `durak_admin` and `durak_autoplay` are deliberately
  absent from `.crushrc`'s `permissions allow`, because they move balances and mint accounts.
- `durak_autoplay` opens 2–6 seats, creates and joins a room, starts, and plays through the same
  `chooseMove` policy, rotating seats so the turn owner acts first. It gates each step on every
  seat having received `gameSeq >= baseline + steps`; actions broadcast one game event per
  player, so a lagging seat would otherwise act on a stale table. It waits for `account.games`
  before reporting balances. Seats are closed unless `keepSeats` is set.
- `durak_wait` returns `hit: false` on timeout instead of an error, so an agent can poll events
  without treating an idle table as a failure. Seats live in the MCP process (and on disk), so
  a game survives between chat turns and across restarts.
- `.crushrc` lists the read and gameplay `mcp_durak_*` names in `permissions allow`; without
  that list Crush prompts for every MCP call. It also raises the MCP call timeout, because
  `durak_watch` and `durak_follow` long-poll for minutes.
- Tool output is a short Russian summary; `structuredContent` carries a compact
  `snapshot` (room, roles, table as text, hand, counts, flags, `hidden`) instead of the whole
  game state, and `verbose: true` adds the full JSON. Snapshots in MCP output drop `log`,
  because `durak_watch` already reports the events that arrived; this keeps per-move responses
  small enough that a game does not stall on context size. The server is authoritative: the MCP
  side only mirrors beat/rank rules to pre-compute hints and the recommended move, and the
  server still rejects anything illegal.

The Rust adapter `rust/crates/platform-mcp` mirrors the same policy in
`src/strategy.rs` (`attach_options`, `defense_options`, `transfer_options`, `dump_options`,
`attack_choices`, `choose_move`, `Ledger`) and exposes the same `durak_turn` tool, so both
servers make identical decisions and report the same `hidden` counts. Its unit tests cover
beat order, trump economy, transfer-vs-defense, dumping, and ledger accounting. Keep the two
implementations in step: change the policy and its Rust tests together
at the table.

### Styles (`style.css`)

One stylesheet with a unified token system: deep felt green, warm ivory, and brass accents.
The interface uses a fixed dark theme. Display type is Cormorant Garamond, UI type
is Manrope, both loaded from Google Fonts with Georgia/system fallbacks. Responsive
layouts and `prefers-reduced-motion` rules live near the end of the stylesheet.

The hero banner and playing table use the fixed `--felt-ink` / `--felt-brass` tokens.
Opponent seats form one centered row along the top of the table. Each fan scales to its
seat width, and deal animations use the actual card positions. The deck sits on the left,
and table cards form one nowrap row. Check narrow screens with five opponents for fan
clipping and deck overlap. On narrow screens the hero card fan
moves into a bottom band, which relies on the extra bottom padding on `.hero-banner` (it is
sized so the fan and its `.hero-note` badge never cover the hero footer). The fan follows the
table's corner convention, a rank with the suit centered under it and the pair mirrored in
the opposite corner, and scales from .84 down to .48 across the breakpoints.

Lobby chrome: the page header carries only the title and `.club-masthead` (weekday, date,
clock, and a live "N столов · N игроков" line written by `renderMasthead()` / `renderLobby()`).
Every lobby action (history, join by code, create) and the mode `.filter` live in
`.lobby-console` under the "Открытые столы" heading, and `updateResumeButton()` prepends the
"Вернуться в игру" button to `.console-actions`. Keep actions out of `.topbar`; below 960px
the actions collapse into a two-column grid.

Page backdrop: a fixed `.ambient` layer sits at `z-index: 0` behind `.app-shell` (`z-index: 1`).
`startAmbient()` spawns random props into three depth layers: `.ambient-far` (dimmest, slowest,
and deliberately sharp so its ranks still read), `.ambient-mid` (behind the `.ambient-glass`
tone wash, blurred 4px), and `.ambient-near` (above the wash, largest and fastest, blurred
6px). `.ambient-glass` is only the tone wash and `.ambient-frost` only the matte veil plus
grain (`::after`, soft-light noise); there is no full-screen `backdrop-filter`, so the blur
belongs to the mid and near props and the distant cards stay legible. Faces are built with
`cardCornersHTML()` inside a real `.card-face`, so corners, pips, courts and aces are the same as
at the table; cards are rendered at their final size (never transform-scaled, which aliases the
hairlines) with `--w` driving the card variables. Props are fully opaque and get depth from the
pane's blur plus per-layer brightness (`--dim`) instead of alpha, so a nearer card occludes
everything behind it. Any card-level filter sits on the card or chip itself, never on the drifting
`.amb-prop`: the wrapper's box is smaller than the rotated card, so filtering it clipped the card.
Every prop starts and ends fully offscreen (`ambDrift` starts at `-(--w + 8vw)` and runs on a
three-point arc so the path curves instead of tracking a straight diagonal) and is
respawned on the drift's `animationiteration` (the handler checks `event.target` and
`animationName`, because animation events bubble up from the swaying card) with a new random
pattern, while `ambSway` adds rotation and a wobble in px on top of the drift. `.ambient-haze`
carries the matte grain (`::after`, blended). The layer is `aria-hidden`, `pointer-events: none`,
and becomes static under `prefers-reduced-motion` (placed at the `--rx` / `--ry` percentages).
Keep it purely decorative: no focusable or clickable content, and keep it behind all app content.

The logo block (`.brand`) is locked against text selection and keeps the default cursor on
hover. Wallet and admin components (`.wallet`, `.admin-row`, `.bet-pill`, `.finish-payout`)
reuse the same tokens; the admin list is a scroll-free simple list, so long player lists can
grow the modal which already scrolls.

The lobby divider (`.ornament`) is a strip of five fading hairline segments with four circular
suit medallions between them: a hairline outer ring, a faint inner ring, and a brass-tinted
fill. Spades and clubs are brass, hearts and diamonds use `--red`; sizes come from `clamp()`
so the row stays on one line down to 320px. It is decorative and `aria-hidden`, keep it that
way.

The deck is a `.deck-stack` of card backs with the face-up trump card lying crosswise
underneath, centered so its ends stick out evenly on both sides, and the count label centered
beneath the stack. Table rows are `.pair` blocks where the defense card is offset down-right and
rotated slightly (`transform-origin: 0 0`) so the attack card's rank corner stays readable;
each card renders rank corners top-left and bottom-right. Card faces come from
`cardBodyHTML()` in `app.js` (pip layouts for numeric ranks, framed courts, a single suit
for aces) and scale through the `--corner-size` / `--pip-size` / `--ace-size` /
`--court-size` variables, which the media queries override instead of touching font rules.

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
- `refillHands()` refills the original trick attacker first, then all other active players
  clockwise, with the defender last; this happens before roles are changed for the next trick.
- `settle()` only marks players finished when the deck is empty and their hand is empty.
  The last remaining active player is the durak; the game ends with `status: 'finished'`
  and `finished.loser`. Restart is the ordinary `start` action (host, ≥2 connected).
- Attack: only `attackerId` opens an empty table; afterwards all active non-defenders may add
  cards matching any table rank. Both `act()` and `gameState()` enforce the 6-row cap and
  defender capacity: the defender must have more cards than the current unbeaten rows.
  Defense: only `defenderId`, card must beat the chosen unattacked row. Beat rule:
  same suit and higher rank, or trump beats non-trump.
- `take` is two-phase: the defender sets `takePending`, active non-defenders may keep adding cards
  of the table rank, then the attacker's `pass` calls `collectTable()` and the defender
  picks everything up. Calling `take` twice is an error.
- `transfer` exists only in `transfer` mode: the defender must provide `cardId` for a card
  matching the single table rank while nothing is beaten; the next active defender must have
  capacity for the resulting table, then the defender becomes the new attacker.
- After the first card, every active player except the defender may podkidat matching-rank
  cards (subject to table and defender-hand capacity). Transfer mode adds the transfer option;
  it does not replace podkidnoy.
- Turn order is the array order of `room.players`; `nextActive()` skips finished and `left` players.
- Limits: `MAX_PLAYERS = 6`, `MAX_TABLE_CARDS = 6`; player names truncated to 20 chars,
  room names to 40.

## Gotchas

- **SSE connection and the matching session token are prerequisites for gameplay POSTs.**
  `/events` requires `token` matching `/^[a-f0-9]{64}$/`; every relevant POST sends the same
  token in its JSON body. An open SSE is still required before room/game operations can run.
- **Identity is tab-scoped and token-bound.** The client id and token are kept in
  `sessionStorage` (`durak.client` and `durak.token`); a different token cannot replace an
  existing session. A reconnect with the same token replaces the prior SSE stream.
- **Rust persistence is split between SQLite and memory.** Game snapshots, account changes,
  transitions, and finished history are persisted by `platform-api`; SSE clients, sessions,
  and pending OAuth flows remain in memory. Active rooms are restored after restart only when
  saved player identities pass the startup checks. The frontend compatibility layer still keeps
  rooms, accounts, and history in memory; restarting it loses that state. Rust cancels a room
  after five minutes with every player offline, refunds stakes without game stats, and retains
  finished rooms for 30 minutes.
- **Server rules are authoritative.** Keep `gameState()` permission flags, `act()` validation,
  and client pre-checks consistent. Attack capacity counts unbeaten rows, not already beaten
  pairs; transfer capacity includes the additional transfer card.
- **`canStart` requires the host plus two connected players**; `start` filters the roster to
  connected players only, is rejected while `status === 'playing'`, and rejects the whole
  start when any connected player cannot cover `room.bet`. `join` is rejected while
  `status === 'playing'`, and server-side capacity checks count reserved players still within
  the offline grace period against `MAX_PLAYERS`.
- **Leaving during play** marks an active leaver as `left` and ends the game with that player
  as loser. A finished leaver is retained in the room and can still receive final settlement;
  leaving a waiting room removes the player normally.
- **Asset cache busting is manual.** Script/style URLs in `index.html` carry `?v=N`
  (`app.js?v=31`, `style.css?v=53`). Bump the matching version when changing those files, or
  browsers may serve stale assets (static files are served without cache headers).
- **Russian everywhere.** UI copy, server messages, README, and test labels are Russian.
  New user-facing strings should be Russian.
- **Style:** 2-space indentation, semicolons, single quotes, arrow functions, `const`/`let`;
  do not add code comments. No external packages are used; prefer adding none.
- UI work: the repo includes a design skill at `.crush/skills/frontend-design/SKILL.md`.
- Rust migration planning: `docs/ARCHITECTURE.md` defines target boundaries for the
  multi-game platform and SQLite-ready account model; `docs/MIGRATION_PLAN.md`
  defines the staged engine, API, MCP and persistence migration. The browser UI stays
  JavaScript remains only on the frontend, and the API contract is preserved.

## Testing notes

- `engine` and `client` are self-contained VM-based regression suites; the others
  exercise the running Rust server. Suites that need real identities call
  `registerClubAccounts()` from `tests/helpers/accounts.mjs`, which creates temporary club
  accounts through `/api/admin` using `ADMIN_KEY` (default `test-only-secret`) and logs in.
  `tests/security.mjs` follows the same path: its `account()` helper creates and logs in
  each temporary account itself, then deletes the accounts during cleanup. No external
  `DURAK_CLIENT_ID` or `DURAK_TOKEN` values are required by that test. The helper rejects
  `TEST_BACKEND=node`; the current test backend is Rust. The helper is being corrected after
  cleanup issues were found; do not treat these notes as evidence that an integration run has
  passed:
  - `tests/engine.mjs` — engine rules, transfer card/capacity checks, leave and offline cleanup.
  - `tests/client.mjs` — client session token generation/storage and transfer UI behavior.
  - `tests/strategy.mjs` — pure MCP policy: beat/trump rules, attack and dump choices,
    transfer-vs-defense, trump economy, and `Ledger` card accounting. No server needed.
  - `tests/mcp-follow.mjs` — `chooseMove` priority for attack/pass/take, also serverless.
  - `tests/security.mjs` — token format/binding, SSE replacement protection, and authenticated
    POSTs.
  - `tests/moves.mjs` — lobby, join by lower-case code, turn order, attack/defend,
    transfer, two-phase take, leave.
  - `tests/rules-fixes.mjs` — beat rules (including trump), multi-card attacks, adding a
    card after "take", repeated over up to 30 randomized attempts because hands are random.
  - `tests/money.mjs` — club accounts: starting balance, stake charge on start, payout to
    the winner when the opponent leaves, admin list/update/reset, bet validation.
  - `tests/accounts.mjs` — club login: admin page is served, no admin button in the main
    UI, temp account creation (unique names), case-insensitive login, wrong password,
    betting table, admin balance top-up pushed over SSE, rename, password change, reset.
  - `tests/full-game.mjs` — simulates a full 2-player game step by step (also checks deck
    conservation), then restarts and asserts 6-card hands and a 24-card deck.
  - `tests/mcp.mjs` — spawns Rust `platform-mcp` and drives it over stdio: handshake, tool list
    with `readOnlyHint` annotations, two seats (`durak_connect` with `create: true` or club
    `login`+`password`), create/join/start, state/hints/admin, a `durak_watch` wait on the
    opponent's move, a full `durak_follow` game, a `durak_autoplay` run with generated club
    accounts, and cleanup. It points `DURAK_MCP_SESSIONS` at a temp dir so a developer's saved
    seats never leak into the run.
  - `tests/mcp-sessions.mjs` — seats survive an MCP restart: the file is written on connect with
    a 64-hex token, the restored seat keeps the same `clientId` and reopens its SSE stream,
    `durak_turn` and `hints` report card accounting, and disconnect removes the entry.
  - `tests/hooks.mjs` — `/api/watch`: journal and `seq` in snapshots, timeout without
    changes, token/`until` validation, firing on the opponent's action, replay of a missed
    event, and firing on `finished` after the defender leaves.
- Integration scripts have their own SSE helpers, `check(label, condition, extra)` reporters, unique
  client ids, and fixed `setTimeout` waits. Integration suites use `process.env.BASE_URL` when
  set, otherwise `http://localhost:4173`. Randomized hands mean the tests may skip a scenario
  or need several attempts; do not assume deterministic decks.
- Integration tests mutate the running server: they create rooms and players and add entries to history.
  Run them against a throwaway server instance.
- After a failed assertion, later actions can depend on state that never changed, so read
  the first failure rather than the last.