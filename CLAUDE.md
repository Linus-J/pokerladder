# Pokerladder

Heads-up No-Limit Hold'em ladder: players are matched by rating, play sit-and-go
matches, and a monthly season decides a free-entry, sponsor-funded prize. The plan and status
live in `docs/ROADMAP.md` (read it first). `legacy/` is the retired C# P2P client — reference only, never patch it.

## Layout

```
crates/engine   pure game logic, no I/O (Phase 1, done)
  src/card.rs        Card (u8 index rank*4+suit), CardSet, Deck (CSPRNG-only shuffle)
  src/eval/          direct evaluator + Two Plus Two lookup table (HandRank 1..=7462)
  src/holdem.rs      one heads-up NLHE hand as a state machine (Hand, Action, Event)
  src/sng.rs         sit-and-go match: blind schedule, button, time bank (Match)
  src/history.rs     PokerStars-style hand histories (Player/Spectator/Audit views)
crates/server   authoritative game server: axum + tokio + sqlx/Postgres (Phase 2)
  src/table.rs       match actor: one task per match owns `Match`, clock, sit-out, filtering
  src/hub.rs         user -> live connection / seat registry
  src/ws.rs          /ws endpoint (cookie auth, Origin check, heartbeat)
  src/auth.rs        register / verify-email / login / logout / me (argon2id, cookies)
  src/matchmaking.rs rating-window queue; `start_match` spawns the actor
  src/db.rs          Postgres queries; `PgStore` (hands + result/rating transaction)
  src/protocol.rs    WebSocket message types; `export_bindings` test writes
                     web/src/lib/protocol.ts (ts-rs)
  migrations/        sqlx migrations (embedded, run at startup)
web/            (Phase 3) Svelte 5 + Vite + TypeScript SPA; only the generated protocol.ts so far
```

## Commands

- `docker compose up -d db` then
  `export DATABASE_URL=postgres://pokerladder:pokerladder@localhost:55432/pokerladder`
  — the server's `tests/api.rs` needs it (each test gets a fresh database).
- `cargo test -q` — all tests (~30 s; the engine is built at opt-level 3 even in dev
  because the 124 MiB 2+2 table takes ~5 s to generate).
- `cargo run -p pokerladder-server` — needs `DATABASE_URL`; optional `BIND_ADDR`,
  `PUBLIC_URL`, `REQUIRE_EMAIL_VERIFICATION` (see `src/config.rs`). Verification links
  are logged (no real mailer yet).
- `cargo test -q -p pokerladder-engine --test evaluator -- --ignored` — exhaustive
  133M-hand table-vs-direct check (~25 s).
- `cargo bench -p pokerladder-engine` — criterion evaluator benchmarks.
- `cargo clippy -q --all-targets` and `cargo fmt` must be clean before committing.

## Engine contracts the server must respect

- Call `pokerladder_engine::eval::init()` once at startup inside `spawn_blocking`,
  before accepting games (never let the first showdown build the table in an async task).
- Shuffle with `Deck::shuffled(&mut rand::rng())` — never a seeded RNG in production.
- `Hand`, `Match` and `Deck` are deliberately neither `Debug` nor `Clone` and must
  never be serialised or logged: they hold the undealt deck and both hole-card pairs.
- Forward to clients only events allowed by `MatchEvent::visibility()` /
  `Event::visibility()`. Persist the **unfiltered** stream as the audit log (it holds
  folded and mucked hole cards for collusion analysis).
- Think time for `Match::act(.., elapsed)` is measured from when the decision was
  offered and is never reset by rejected actions; schedule `Match::timeout` at
  `Match::time_allowed(seat)`. Late actions are converted to timeouts.
- Use `Hand::public_state()` to rebuild a reconnecting client's table, plus that
  player's own `hole_cards`. (The server's `MatchState` snapshot sends the public state
  plus the current hand's events filtered for that player.)
- Engine `Event`s can't be rebuilt from storage (`Hidden` has no public constructor),
  so hand histories are rendered when each hand ends and stored next to the audit log.
- Chip amounts are capped at 2^53 (`MAX_TOTAL_CHIPS`) so they are exact in JavaScript.
- Display names: restrict to `[A-Za-z0-9_-]{1,20}`.

## Conventions

- Authoritative server: nothing a client sends is trusted; all rules live in the engine.
- Errors leave state unchanged; invariants that would stall a match are `assert!`ed
  even in release.
- Performance matters to the owner: benchmark hot paths with criterion and report
  trade-offs honestly; prefer checked APIs with explicit `_trusted` fast paths.
- Each significant piece gets an independent code-review pass before committing, and
  property tests should check rules independently of the implementation (mutation-test
  them: plant a bug, confirm a test fails).
- Work on a feature branch per phase (e.g. `phase-2-server`), merged into `main` by
  PR; end commit messages with the Co-Authored-By line.
- Gambling-law sensitivity: no paid entry of any kind; prizes are sponsor-funded and
  free to enter. Flag anything that would change that.
