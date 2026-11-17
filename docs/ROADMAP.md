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

## Status (2026-11-17)

Phases 1 to 3 and Phase 4a are merged into `main` and live at
https://pokerladder.app (a home PC behind a Cloudflare Tunnel; see
`deploy/home/README.md`). Phase 4a (PR #9) added Glicko-2, monthly UTC seasons,
season standings and the repeat-opponent cap; ratings were reset to 1500 when it
deployed. `deploy/home/reset-ladder.sh` wipes ranks and match history (keeping
accounts) to test the rating system from scratch.

**Phase 4b** (integrity) is on branch `phase-4b-integrity`: a read-only
chip-dumping report (`deploy/home/integrity-report.sh`). The owner chose a CLI report
over an admin page, and no IP/device collection (manual identity check before any
payout instead), so the privacy notice is unchanged. **Next:** competition terms and
the payout process (Phase 5), with the solicitor review.

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
- [x] SMTP mail (`SMTP_URL`, lettre, sent in the background, at most 8 at once);
      the server refuses to start with an https `PUBLIC_URL` and no SMTP, and logs an
      error (but still starts) if the SMTP login check fails. Without SMTP, emails
      are logged (development). Per recipient: at most 3 verification/reset emails
      per 2 hours, and separately 1 "already registered" notice per 6 hours (anyone
      can trigger those, so they mustn't use up the owner's allowance).
- [x] Rate limits (in-memory token buckets, single process): login per IP (IPv6 per
      /64), failed logins per account, register, verify-email, password reset;
      per-connection WebSocket message limit (burst 20, 10/s; disconnect after 100
      refused in a row). Argon2 capped at one hash per CPU; oversized logins are
      refused before any limiter. Client IP from the peer or `TRUSTED_PROXY_HEADER`
      (rightmost entry; see `.env.example` for CDN setups).
      The per-account bucket is keyed by the login as typed (name and email are
      separate buckets), never by the account it resolves to, so a 429 can't confirm
      which email belongs to a player. A token is taken before hashing (concurrent
      guesses can't slip through) and refunded on success (only failures count).
      Trade-off: 10 wrong passwords lock that login string for ~30 s per further
      attempt, so a stranger can keep a player's name locked out (their email still
      works); per-IP limits bound how cheaply. Revisit with CAPTCHA or per-device
      trust if it is abused.
- [x] Registration doesn't reveal registered emails: the same 202 response, the
      address owner gets an "already registered" email, and the requested name is
      reserved for 48 h (as long as an unverified account holds its name) so a
      follow-up sign-up can't probe it either. Reset requests are processed in the
      background so response time doesn't reveal registered emails.
- [x] Password reset (`/api/password-reset/{request,confirm}`, 1 h single-use
      link): always 202; on success sets the password, verifies the email, ends all
      sessions and closes the live socket. Never-verified accounts older than 48 h
      with no matches give up their name/email to a new sign-up, so an email can't
      be squatted.
- [ ] If `finish_match` fails 5 times (~15 s) the result is lost and the match is
      later marked abandoned; consider a pending-results table + reconciler.
- [ ] Single-instance only: startup marks every `in_progress` match abandoned.
- [x] Graceful shutdown (SIGINT/SIGTERM): the queue closes, everyone online gets a
      `restarting` notice, matches in progress play to the end while players can
      still reconnect, then sockets close. Waits up to `DRAIN_TIMEOUT_SECS`
      (default 45 min); a second signal stops at once.
- [x] Draft privacy notice (`/privacy`) discloses audit-log retention.
      **Needs legal review**, together with competition terms (not yet written).
- [x] `GET /api/matches`, `GET /api/matches/{id}/hands` (own seat's view only;
      404 for other players' matches), `POST /api/resend-verification`.
- [x] Serves the built client (`STATIC_DIR`, SPA fallback) with CSP (WebSocket
      origin from `PUBLIC_URL`), no-referrer, nosniff and frame-deny headers;
      hashed `/assets` cached immutably, missing assets 404.

### 3. Web client
- [x] Svelte 5 + Vite + TypeScript SPA (`web/`), ~29 kB gzipped JS. Pages: landing
      + leaderboard, register / login / verify-email, lobby + queue, table, match
      and hand history, privacy. Protocol types generated from Rust (`ts-rs`).
- [x] Table: four-colour deck, bets and pot, dealer button, decision clock, hand
      log, bet sizing (min / ½ pot / pot / all-in, slider, keyboard F/C/R),
      auto-muck toggle, result overlay with rating change. Usable at phone width.
- [x] Reconnects automatically (snapshot rebuilds the table, including auto-muck);
      "opened in another tab" handling; a match that ended while offline is cleared
      with a pointer to History. One action in flight at a time (no double-acting
      from double clicks or held keys).
- [x] Table state is a pure reducer, unit-tested and checked step by step against
      seeded matches played by the real engine (`engine-fixture.json`, generated by
      `crates/server/tests/client_fixture.rs`), including chip conservation.
- [x] Forgot / reset password pages.
- [x] Browser tests (Playwright): two players sign up, queue, play a whole match
      through the UI, see the result and their own-seat hand history; phone layout.
- [x] `README.md` rewritten for the new project; `.env.example`.
- [ ] Nice to have: sounds, animations, spectating, a hand replayer, per-player
      profile pages.

### 4. Seasons & integrity
- [x] Monthly seasons (UTC calendar months, `season_ratings`): everyone restarts at
      1500 each month; 20 rated matches to be ranked; past seasons stay viewable.
- [x] Glicko-2 (`rating.rs`, checked against Glickman's worked example): one
      rating period per match; each idle day counts as 6 empty periods. Standings
      rank by rating - 2 RD, so sitting on a lead doesn't pay (20 idle days cost a
      settled player about 150 points of score). Volatility per 1-game period is
      noisier than Glickman intends, accepted for instant updates. A separate lifetime
      Glicko-2 rating drives matchmaking only. Beta Elo ratings were reset.
- [x] Repeat opponents: only a pair's first 3 matches in any 24 hours are rated;
      later ones are played unrated and the table says so before the first hand.
- [x] `deploy/home/reset-ladder.sh`: wipe matches, hands, seasons and ratings
      (accounts stay), with a backup first; refuses while a match is running.
- [x] **4b. Chip-dumping report** (`crates/server/src/integrity/`, CLI
      `--bin integrity -- [--season YYYY-MM] [--all]`, production wrapper
      `deploy/home/integrity-report.sh`). Read-only, for a person to review before any
      payout; no automatic bans. Reads `hands.audit_events` (both players' hole cards)
      for one season: a match belongs to the season it was rated in, else the UTC
      month it started. For each ordered pair "P may be dumping to O":
      - **strong folds**: hands where P faced a bet holding a strong hand (preflop top
        10% of combinations; postflop ≥ 80% equity against a random holding), and how
        many P folded. Folds the server made for a player who timed out or sat out
        are kept apart and listed on their own line (a disconnect isn't a choice,
        though sitting out could be a way to dump);
      - **weak big commitments**: P's first action whose voluntary chips (blinds
        excluded, so a short stack's forced call isn't one) reach ≥ 50% of their
        starting stack, and how many were with a weak hand (preflop bottom 30%;
        postflop < 30% against a random holding);
      - **results**: P's rated wins against O vs the Elo expectation from the stored
        season ratings (z-score; flags at z ≤ -3).
      Strength is always against a *random* holding, never the opponent's actual
      cards (nobody can see those). Each rate is tested against P's own rate against
      everyone else, shrunk towards the population rate (without the pair under test)
      by 10 pseudo-hands (a second account with little other play gets compared with
      everyone; the population rate is a Jeffreys estimate, never exactly 0). The
      test is an exact beta-binomial tail: the baseline is a Beta distribution with
      that mean whose spread reflects how few hands stand behind it, so a thin
      baseline (0 of 30) can't make 3 events look impossible. A flag needs ≥ 3
      events and p < 0.001. Per pair: short matches (≤ 5 hands) tested against each
      player's own short-match record, keeping the more lenient result, and a
      note (not a flag) for pairs that met ≥ 10 times and make up half of either
      player's matches: in a small ladder, matchmaking alone does that. Flagged pairs
      list example hands with ids, the board, pot odds, strength, and equity against
      the cards the opponent actually held. Hands are treated as independent, which
      they aren't quite, so p-values rank leads; they don't prove anything.
      **Tests**: a seeded bot population played through the real engine (honest
      players, a maniac who shoves every hand, a dumper who folds to and shoves trash
      into one partner, and one who times out every decision against one opponent);
      across 3 seeds only the dumper is flagged (e.g. strong folds 21/21 against
      0/31 vs others); a dumper who dumps on only half their decisions is still
      flagged on 3 more seeds; the maniac and the absent player aren't; an
      all-honest season flags nothing; hand-built event sequences pin the
      bookkeeping (timeouts, blinds, pot odds); synthetic matches show the results
      and short-match flags firing; the Postgres round-trip gives the same report.
      Mutation-tested: 8 planted bugs (opponent's cards for strength, wrong amount
      to call, pair in its own baseline, swapped seats, timeouts ignored, blinds
      counted as voluntary, rating index swapped, short-match baseline min instead of
      max) each fail a test. The CLI's database session is read-only.
      **Equity** (`engine::eval::equity`): exact `vs_hand` and `vs_random` by
      enumeration on the 2+2 table (runouts outer, opponent holdings inner, so each
      board is walked once). Preflop `vs_random` reads a precomputed 169-class table
      (`preflop_vs_random.in`, exact counts; an `--ignored` test recomputes it in
      ~4 min on 16 threads). Checked against brute force with the direct evaluator
      on every street. Criterion, one call: `vs_random` preflop 18 ns, flop 3.2 ms
      (1.07 M showdowns, ~340 M/s), turn 73 µs, river 1.8 µs; `vs_hand` preflop
      6.4 ms (1.7 M runouts), flop 2.4 µs. A report costs ~3 ms per distinct
      (seat, flop) where that player faced a bet (cached per street within a hand),
      single-threaded; if seasons get big, analyse matches in parallel.
- [x] **One account per person**: owner decision 2026-11-17: no IP or device
      collection; an identity check (manual) before any payout. Revisit only with a
      privacy-notice change.

### 5. Prizes
- **Model: free entry, sponsor-funded prize.** No payment of any kind is required to
  play or to qualify.
- **Correction (Nov 2026): free entry alone is not enough.** Under the Gambling Act 2005
  s6(4)(b) a person plays a game of chance "for a prize" whether or not they risk
  anything, and a prize is "money or money's worth" (s6(5)), which includes gift
  cards, vouchers and crypto. Poker counts as a game of chance (s6(2)(a)). Providing
  facilities for that needs a licence (s33), and s36(3) applies because the server is
  in Great Britain. The free-entry rule (Sch 2 para 8) only applies to lotteries and
  prize draws. **No prize with a money value until a solicitor confirms a model.**
  Recognition-only prizes (titles, hall of fame) are fine now. Draft terms with the
  options and questions for the solicitor: `docs/competition-terms-draft.md`.
- Paid buy-ins remain out of scope (would need a Gambling Commission licence).

## Open decisions
- Hosting (a single small VPS is sufficient initially).
