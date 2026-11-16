# Pokerladder

A ranked ladder for heads-up No-Limit Texas Hold'em. Players are matched with
opponents near their rating, play short sit-and-go matches in the browser, and a
monthly season decides the winner of a **free-to-enter, sponsor-funded prize** — no
payment is ever needed to play or to win.

## How it works

- **Matches:** 1,500 chips each, blinds from 10/20 rising every 10 hands, 15 seconds
  per decision plus a time bank. A match usually takes 15–20 minutes.
- **Matchmaking:** the server pairs you with someone close to your rating; you can't
  pick opponents.
- **Fair play:** the server deals with a cryptographically secure shuffle and only
  ever sends you your own cards. Disconnecting never wins a match: your hand is
  checked or folded when your clock runs out.
- **Ratings:** Elo for now (Glicko-2 with seasons is planned).

## Architecture

```
browser (Svelte 5 + TypeScript) ──WebSocket──► server (Rust: axum + tokio) ──► Postgres
                                                  │
                                                  └── engine crate (pure game rules)
```

| Path | What |
|---|---|
| `crates/engine` | Cards, Two Plus Two hand evaluator, heads-up hold'em rules, sit-and-go structure, hand histories. No I/O. |
| `crates/server` | Accounts, matchmaking, one task per match, WebSocket protocol, persistence and ratings. Serves the built web client. |
| `web/` | The browser client (Vite + Svelte 5 + TypeScript). Protocol types are generated from the Rust code. |
| `docs/ROADMAP.md` | Plan, status and design notes. |
| `legacy/` | The original C# peer-to-peer client — retired, kept for reference only. |

## Running locally

You need Rust (stable), Node.js 22+, and Docker (or any Postgres 15+).

```sh
docker compose up -d db                       # Postgres on localhost:55432
export DATABASE_URL=postgres://pokerladder:pokerladder@localhost:55432/pokerladder

# Either: production-style, one process serving the built client on :8080
(cd web && npm ci && npm run build)
cargo run -p pokerladder-server
# → http://localhost:8080

# Or: hot-reloading client on :5173, proxying to the server
PUBLIC_URL=http://localhost:5173 cargo run -p pokerladder-server
(cd web && npm run dev)
```

Without `SMTP_URL`, emails (account confirmation links) are written to the server log.
Set `REQUIRE_EMAIL_VERIFICATION=false` to skip confirmation while developing. All
settings are listed in [`.env.example`](.env.example).

## Tests

```sh
cargo test -q                    # engine + server (server DB tests need DATABASE_URL)
cargo clippy -q --all-targets && cargo fmt --check
cd web
npm run check && npm test        # type-check + unit tests
npm run build && npm run e2e     # browser tests: two players play a full match
```

The browser tests start the real server binary (`cargo build -p pokerladder-server`
first). Install a browser with `npx playwright install chromium`, or point `CHROMIUM`
at a system one (e.g. `CHROMIUM=/usr/bin/chromium npm run e2e`).

Evaluator benchmarks: `cargo bench -p pokerladder-engine`.

## Licence

See [LICENSE](LICENSE).
