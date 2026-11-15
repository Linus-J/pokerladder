# Pokerladder — Rewrite Roadmap

Goal: a heads-up No-Limit Hold'em ladder where players are matched by rating and a
monthly season decides a winner of a **free-entry, sponsor-funded prize**. The original
C# WinForms P2P client (now in `legacy/`) is retired; this is a ground-up rewrite with an
**authoritative server**.

## Why rewrite (summary of legacy audit)

- Clients connected directly to MySQL with embedded credentials → anyone could edit ratings / read hashes.
- One peer shuffled and sent the full deck (both players' hole cards) in plaintext UDP.
- Clients computed and wrote their own rating; disconnecting awarded the disconnector a win.
- Login never read the password field (`pass = ""`) → any account was accessible.
- Hand evaluator missed straights after a break (e.g. K-9-8-7-6-5), never detected the wheel,
  mis-detected straight flushes, and compared flushes incorrectly.
- Betting state lived in UI labels with magic values; no min-raise or uncalled-bet handling.

## Architecture

```
browser (Svelte + TS) ──WebSocket──► server (Rust: axum/tokio) ──► Postgres
                                        │
                                        └── engine crate (pure, no I/O)
```

- **Server is the single source of truth**: shuffles (OS CSPRNG), deals, sends each player
  only their own hole cards, validates every action, enforces timers, writes results.
- **Clients render and submit intents only.** Nothing a client says is trusted.

```
crates/engine   cards, Two Plus Two evaluator, NLHE heads-up state machine, hand history
crates/server   auth, sessions, matchmaking, table actors, ratings, persistence
web/            Svelte 5 + TypeScript + Vite SPA, served as static files by the server
legacy/         original C# code (reference only)
```

Crates: `axum`, `tokio`, `sqlx` (Postgres), `argon2`, `rand` (`OsRng`/`ChaCha20Rng`),
`serde`, `proptest`, `criterion` (benches), `rs_poker` (dev-only, as an independent oracle).

**Frontend choice — Svelte 5 + Vite (plain SPA, not SvelteKit):** small bundles and
fine-grained reactivity suit a table that updates many small pieces of state per action;
no SSR needed since everything lives behind a WebSocket. Message types are generated
from Rust (`ts-rs` or similar) so client and server can't drift.

## Hand evaluator: Two Plus Two lookup table

The classic 2+2 evaluator is a state machine flattened into one `u32` array. Each state
(a canonicalised partial hand) owns a block of 53 slots: slot `0` holds the hand's value
if evaluation stops here (5/6 cards), slots `1..=52` hold the offset of the next state
after adding that card — or, from a 6-card state, the final 7-card value. A 7-card
evaluation is therefore exactly 7 dependent array reads, no branching.

- Values are dense equivalence-class ranks `1..=7462` (higher wins), stored as
  `NonZeroU16` so an invalid rank is unrepresentable.
- Suit canonicalisation: once a suit can no longer make a flush within 7 cards, its
  cards are stored rank-only. Result: **612,978 states / 32,487,834 entries (124 MiB)** —
  identical in size to the original 2+2 table.
- Built by `engine::eval::init()` (~5 s, single-threaded). The server must call it from a
  blocking context at startup, never lazily inside an async game task.
- Incremental API (`Partial`, `add`, `finish`, `finish_with`): walk a shared prefix
  such as the board once and branch from it.
- **Duplicate cards panic** at every checked entry point (`evaluate`, `eval7`, `add`,
  `finish_with`). The table alone can't detect them: after a suit is canonicalised away,
  re-adding one of its cards looks like a new card of that rank (found in review).
- `_trusted` variants (`eval7_trusted`, `add_trusted`, `finish_with_trusted`) skip the
  duplicate check for hot loops whose cards are distinct by construction (equity
  enumeration). Debug builds still check; release gives an unspecified rank for
  duplicates, never undefined behaviour.

**Verification (all passing):**
- All 133,784,560 7-card hands: exact textbook category counts; 4824 distinct classes.
- All 2,598,960 5-card hands: textbook counts; 7462 classes.
- Table == direct evaluator on every 5-, 6- and 7-card hand (7-card run is `--ignored`, ~25 s).
- Same showdown winner as `rs_poker` on 300k random heads-up showdowns.
- Duplicate inputs panic on every checked path, including the cases the table misses.
- Independent code review: the `unsafe` unchecked indexing is sound for all inputs.

**Benchmarks** (`cargo bench -p pokerladder-engine`; one run on a loaded machine, so
compare within a row rather than across runs):

| Workload | 2+2 checked | 2+2 trusted | direct | rs_poker |
|---|---|---|---|---|
| Random independent 7-card hands | 31 M/s | 55 M/s | 10 M/s | 47–76 M/s |
| Enumeration (all rivers × all opponent holdings) | 1.07 G/s | **1.24 G/s** | 21 M/s | — |

- Unchecked indexing in the 7-read loop: +15%, sound by table invariants only
  `generate()` can establish.
- Duplicate check costs ~12 ns per random 7-card hand, ~15% in enumeration.
- Random access is bound by cache misses into the 124 MiB table, which is why
  `rs_poker`'s compact perfect-hash evaluator is competitive there; 2+2 dominates when a
  prefix is shared (equity calculation). For the game server, either is far faster than
  needed: one showdown per hand.

Possible later improvements: parallel generation (rayon), or an on-disk cache with
invariant validation to cut startup time.

## Status (2026-11-15)

Phases 1 (engine) and 2 (server) are merged into `main`. Phase 3 (web client) is
on branch `phase-3-web`: a playable Svelte client served by the server, plus the
Phase 2 launch blockers (SMTP mail, rate limiting, no email enumeration, history
API). Browser tests play a whole match between two real browsers.

**Before a public launch (needs the owner):** an SMTP provider and DNS (SPF/DKIM)
for the sending domain; a host + domain + TLS reverse proxy; solicitor review of the
privacy notice (draft in the app) and the competition terms (not yet written).

## Phases

### 0. Hygiene
- [x] Old MySQL server decommissioned (confirmed) — leaked credential is dead.
- [x] C# sources moved to `legacy/`; `bin/` and `obj/` removed from the tree; `.gitignore`.
- [ ] (Optional) scrub `bin/` from git history — low priority now the server is gone.

### 1. Engine (no networking)
- [x] Card/deck/`CardSet` types; CSPRNG shuffle (`Deck::shuffled` requires a `CryptoRng`).
- [x] Direct (bitmask) evaluator — the reference used to build and test the table.
- [x] Two Plus Two table generator + lookup API; exhaustive tests; benches.
- [x] PokerStars-style hand descriptions (`HandRank::describe`).
- [x] Heads-up NLHE hand state machine (`holdem::Hand`): button = small blind, acts
      first preflop / last postflop; BB option; min-raise from last full raise; short
      all-ins; raises capped at the effective stack (`Action::AllIn` shoves it);
      uncalled bets returned; hands tabled before all-in runouts; showdown with auto-muck
      for losing hands (cards stay in the server audit log); split pots (always even heads-up). Server helpers:
      `Event::visibility`, redacted hole cards in `Debug`, `Legal::passive_default`
      (timeouts), `Hand::public_state` (reconnects). Chips capped at 2^53 (JS-safe).
- [x] Sit-and-go match (`sng::Match`): 1500 chips, blinds 10/20 rising ~50% every
      10 hands (12 levels, last one forces all-in), alternating button, 15 s per action
      + 60 s bank (+10 s per 10 hands), timeouts check/fold, auto-muck preference.
      Property test plays 300 whole random matches.
- [x] Property tests (4000 random hands): chip conservation every step, illegal
      actions rejected without state change, independent min-raise model, event log
      balances, termination.
- [x] Hand-history output (`history::format_hand`): PokerStars-style text with
      Player / Spectator / Audit views (mucked cards only in Audit). Header uses the
      site name "Pokerladder"; replayers that key on a specific site prefix may need
      a small converter.

### 2. Server
- [x] Accounts (`auth.rs`): Argon2id hashes on the blocking pool; email verification
      by single-use 48 h token (only verified accounts may queue); 30-day session tokens
      in an `HttpOnly; SameSite=Strict` cookie. Only SHA-256 hashes of tokens are
      stored. Unknown logins verify against a dummy hash (no timing oracle).
- [x] Display names `[A-Za-z0-9_-]{1,20}` (validated, and a DB `CHECK`); names and
      emails unique case-insensitively.
- [x] Matchmaking queue: server pairs by rating; each player's acceptable gap starts
      at 100 and widens by 20/s; longest-waiting chooses first; seats and first button
      are random. Players can't pick opponents.
- [x] One tokio task per match (`table.rs`) owning `sng::Match`; WebSocket protocol
      with typed JSON messages, TypeScript generated by `ts-rs`
      (`web/src/lib/protocol.ts`).
- [x] Think time measured from when a decision is offered, never reset by rejected
      actions; deadline = `Match::time_allowed`; only events allowed by
      `MatchEvent::visibility` reach each player (tests check every message).
- [x] Disconnects: the clock keeps running (free time, then bank, then check/fold).
      A player still away at their deadline is *sitting out*: later decisions
      check/fold after 1 s until they reconnect, so an abandoned match ends quickly and
      absence never wins. Reconnect gets a `MatchState` snapshot (public state + that
      player's filtered hand events + clock). One connection per user; a new one
      replaces the old.
- [x] Every hand persisted when it ends: unfiltered audit events (JSONB, all hole
      cards incl. folded/mucked) plus rendered histories for each seat and the audit
      view. Match result + Elo update (K=32) in one transaction, idempotent on retry.
      Matches interrupted by a restart are marked `abandoned` (unrated).
- [x] SMTP mail (`SMTP_URL`, lettre, sent in the background); the server refuses
      to start with an https `PUBLIC_URL` and no SMTP, and checks the SMTP login at
      startup. Without SMTP, emails are logged (development).
- [x] Rate limits (in-memory token buckets): login per IP and per account,
      register, verify-email, resend-verification; per-connection WebSocket message
      limit (burst 20, 10/s; disconnect after 100 refused). Argon2 capped at one hash
      per CPU. Client IP from the peer or `TRUSTED_PROXY_HEADER`.
- [x] Registration doesn't reveal registered emails: the same 202 response, and
      the address owner gets an "already registered" email instead.
- [ ] If `finish_match` fails 5 times (~15 s) the result is lost and the match is
      later marked abandoned; consider a pending-results table + reconciler.
- [ ] Single-instance only: startup marks every `in_progress` match abandoned.
- [x] Draft privacy notice (`/privacy`) discloses audit-log retention.
      **Needs legal review**, together with competition terms (not yet written).
- [x] `GET /api/matches`, `GET /api/matches/{id}/hands` (own seat's view only;
      404 for other players' matches), `POST /api/resend-verification`.
- [x] Serves the built client (`STATIC_DIR`, SPA fallback) with CSP, no-referrer,
      nosniff and frame-deny headers.

### 3. Web client
- [x] Svelte 5 + Vite + TypeScript SPA (`web/`), ~29 kB gzipped JS. Pages: landing
      + leaderboard, register / login / verify-email, lobby + queue, table, match
      and hand history, privacy. Protocol types generated from Rust (`ts-rs`).
- [x] Table: four-colour deck, bets and pot, dealer button, decision clock, hand
      log, bet sizing (min / ½ pot / pot / all-in, slider, keyboard F/C/R),
      auto-muck toggle, result overlay with rating change. Usable at phone width.
- [x] Reconnects automatically (snapshot rebuilds the table); "opened in another
      tab" handling; table state is a pure reducer with unit tests (vitest).
- [x] Browser tests (Playwright): two players sign up, queue, play a whole match
      through the UI, see the result and their own-seat hand history; phone layout.
- [x] `README.md` rewritten for the new project; `.env.example`.
- [ ] Nice to have: sounds, animations, spectating, a hand replayer, per-player
      profile pages.

### 4. Seasons & integrity
- [ ] Monthly seasons; minimum matches to qualify.
- [ ] Rating with uncertainty (Glicko-2) so early leaders can't "sit" on a rating.
- [ ] Anti-abuse: diminishing gains vs repeat opponents, chip-dumping heuristics
      run over the audit log (e.g. folding strong hands to a specific opponent),
      one account per person, identity check before any payout.

### 5. Prizes
- **Model: free entry, sponsor-funded prize.** No payment of any kind is required to
  play or to qualify, which keeps it away from paid-entry gaming under the UK Gambling
  Act 2005. Still get a quick sign-off from a gambling solicitor before the first payout,
  and publish clear competition terms (eligibility, tie-breaks, payout method).
- Paid buy-ins remain out of scope (would need a Gambling Commission licence).

## Open decisions
- Hosting (a single small VPS is sufficient initially).
