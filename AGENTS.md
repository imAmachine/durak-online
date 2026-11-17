# AGENTS.md

Browser multiplayer card game "Дурак" (Russian Durak). Plain Node.js HTTP server with
Server-Sent Events, plain browser JavaScript frontend. No framework, no bundler, no
dependencies, no build step. All user-facing text is Russian.

## Commands

```bash
npm start        # node server.js; PORT env (default 4173), HOST env (default 0.0.0.0)
npm test
BASE_URL=http://127.0.0.1:4174 npm test
```

- There is no lint, typecheck, format, or build step. There is no CI config.
- Tests are not self-contained: start the server first in another terminal, then run
  `npm test`. Integration tests use `BASE_URL` when set, otherwise `http://localhost:4173`.
- `npm test` chains seven `.mjs` scripts with `&&`, so the first failing suite stops the
  rest. Scripts exit non-zero on failure and print `ok`/`FAIL` lines.
- Tests use global `fetch` and top-level `await`, so they need Node 18+. No package
  dependencies exist; there is no lockfile.
- `/api/status` returns online clients and rooms; handy for checking a running server.

## Architecture

### Server (`server.js`, ~945 lines)

Single-file Node HTTP server. All state is in in-memory structures:

- `clients: Map<clientId, { id, name, res, roomId }>` — one entry per open SSE stream.
- `sessions: Map<clientId, token>` — session token bound to the client id; tokens must match
  `/^[a-f0-9]{64}$/`.
- `rooms: Map<roomCode, room>` — room code is 6 uppercase hex chars.
- `history: []` — finished games, unbounded; lobby payload sends only the last 8 reversed.
- `accounts: Map<clientId, { id, name, balance, games, wins, losses }>` — club profile and
  wallet per client id, created on first SSE connect. Not persisted anywhere.

Entry points:

- `GET /events?id=<clientId>&token=<token>&name=<name>` — the only real-time channel (SSE).
  Registers the client only with a valid token, attaches it to its room if it is already a
  player, sends an initial `game` or `noroom` event, then `lobby`. On disconnect the player is
  marked `connected: false` but stays in the room. The browser reconnects automatically
  (`retry: 2000`).
- `POST /api/rooms` — create room; the caller becomes host and first player.
- `POST /api/join` — join by code (case-insensitive); reconnecting existing players update
  name/connection instead of adding a new seat.
- `POST /api/action` — all gameplay actions, delegated to `act()`:
  `start | attack | defend | transfer | take | pass`.
- `POST /api/profile`, `POST /api/leave`, `POST /api/admin`, `GET /api/status`.
- Authenticated gameplay POSTs carry `clientId` and `body.token`; any POST supplying a
  `clientId` is also checked against `sessions` (including admin requests).
- Static serving is allowlisted to `index.html`, `app.js`, and `style.css` (`/` → `index.html`).

`/api/admin` is a single POST endpoint with `op: list | update | reset | balance` and a `key`
body field checked against `ADMIN_KEY` (env; unset disables the panel). It can rename players,
set balance/stats, and reset to the starting balance. Errors are 403 for a wrong key, 404 for
an unknown player.

API responses are JSON; errors use `{ error: "<Russian message>" }` with status 400/403/404,
or 503 when the admin panel is disabled.

Game flow inside `server.js`:

- `createRoom()` → `startRound()` (shuffle, deal 6, choose the lowest trump, or lowest card
  when nobody has trump) → `act()`.
- `act()` validates and mutates the room, then calls `broadcastRoom()`.
- After a trick: `pass` (all cards beaten) or `collectTable()` (defender took cards) clears
  the table, calls `refillHands()` before rotating roles, then `settle()` → `normalizeRoles()` →
  `updateTurn()` → `broadcastRoom()`.
- `finishGame(room, loser, message)` sets `room.finished` and `finishedAt`, appends history,
  and calls `settleAccounts()` (stats + bet payout). `settle()` and active `leaveRoom()` use it.
  Offline cancellation instead uses `cancelGame()` to refund stakes without stats or history.

Money model: `room.bet` is one of `BETS` (`0/50/100/250/500`, anything else becomes 0),
`room.pot` is collected at `start` by `chargeStakes()` (host starts, all connected players
must afford it), and `settleAccounts()` credits `room.lastPayouts` per player from the pot
to everyone except the loser. Starting balance is `START_BALANCE = 1000`. An active player leaving during play loses and
ends the game; a player already finished may leave while remaining in the roster for settlement.

`gameState(room, clientId)` builds a per-viewer snapshot: own hand only, opponents' card
counts and roles, table, trump, deck size, `room.bet`, `payout` (this viewer's last net
result), derived permission flags (`canAttack`, `canDefend`, `canTransfer`, `canPass`,
`canTake`, `canStart`), and messages.
`broadcastRoom()` sends each player a snapshot tailored to their own id; there is no shared
broadcast payload, so hidden information never leaks by construction. Preserve that when
adding fields.

### Client (`app.js`, ~1238 lines)

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
- Admin panel: `openAdminGate()` asks for the key once per tab (`sessionStorage['durak.admin']`),
  `syncAdminNav()` shows the `data-view="admin"` nav item, `openAdminPanel()` lists accounts
  and `openAdminPlayer()` edits them through `/api/admin`.
- `fitHand()` computes one `--overlap` for the hand row and, when even the minimum card step
  does not fit, sets `zoom` on `#myHand` so a 24+ card hand stays on the table. `fitTable()`
  does the same for six table pairs (caps the row at its `max-width: 88%`). Call both after
  rendering and on resize.
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
`.pos-1`…`.pos-5`, the deck sits at the lower right, and table cards form one nowrap row.
Keep seats, deck, cards, message, and hand non-overlapping; the layout is verified from
320px to 1600px, including the 961–1180px range where seats shrink via their own breakpoint
and five opponents plus six pairs are the tightest case. On narrow screens the hero card fan
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
the only layer with its own blur), `.ambient-mid` (behind the frosted `.ambient-glass` pane), and
`.ambient-near` (above the pane, sharp, softened by `.ambient-haze` so suits stay readable,
largest and fastest). The pane keeps only a light `backdrop-filter` blur, so mid cards stay
legible instead of reading as a full blur, and depth comes from tone, scale and speed. Faces are built with
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
underneath and sticking out to the right; its content is padded left so it lands in the
visible part. Table rows are `.pair` blocks where the defense card is offset down-right and
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
- **Everything is in memory.** Restarting the server loses rooms, players, history, and club
  accounts (balances and stats). A 30-second cleanup interval cancels a playing room only when
  all players have been offline for 5 minutes, refunds every stake without game stats, and
  retains the finished room for 30 minutes using `finishedAt`; waiting rooms with the same
  offline grace are removed.
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
  (`app.js?v=26`, `style.css?v=47`). Bump the matching version when changing those files, or
  browsers may serve stale assets (static files are served without cache headers).
- **Russian everywhere.** UI copy, server messages, README, and test labels are Russian.
  New user-facing strings should be Russian.
- **Style:** 2-space indentation, semicolons, single quotes, arrow functions, `const`/`let`;
  do not add code comments. No external packages are used; prefer adding none.
- UI work: the repo includes a design skill at `.crush/skills/frontend-design/SKILL.md`.

## Testing notes

- `engine` and `client` are self-contained VM-based regression suites; the other five scripts
  exercise a running server:
  - `tests/engine.mjs` — engine rules, transfer card/capacity checks, leave and offline cleanup.
  - `tests/client.mjs` — client session token generation/storage and transfer UI behavior.
  - `tests/security.mjs` — token format/binding, SSE replacement protection, and authenticated
    POSTs.
  - `tests/moves.mjs` — lobby, join by lower-case code, turn order, attack/defend,
    transfer, two-phase take, leave.
  - `tests/rules-fixes.mjs` — beat rules (including trump), multi-card attacks, adding a
    card after "take", repeated over up to 30 randomized attempts because hands are random.
  - `tests/money.mjs` — club accounts: starting balance, stake charge on start, payout to
    the winner when the opponent leaves, admin list/update/reset, bet validation.
  - `tests/full-game.mjs` — simulates a full 2-player game step by step (also checks deck
    conservation), then restarts and asserts 6-card hands and a 24-card deck.
- Integration scripts have their own SSE helpers, `check(label, condition, extra)` reporters, unique
  client ids, and fixed `setTimeout` waits. Integration suites use `process.env.BASE_URL` when
  set, otherwise `http://localhost:4173`. Randomized hands mean the tests may skip a scenario
  or need several attempts; do not assume deterministic decks.
- Integration tests mutate the running server: they create rooms and players and add entries to history.
  Run them against a throwaway server instance.
- After a failed assertion, later actions can depend on state that never changed, so read
  the first failure rather than the last.