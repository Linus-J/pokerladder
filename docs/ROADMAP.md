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
- [ ] Hand-history output (PokerStars-style text so existing replayers work).

### 2. Server
- [ ] Accounts: argon2 hashes, server-side email verification, session tokens.
- [ ] Matchmaking queue (server pairs players by rating; players can't pick opponents).
- [ ] One tokio task (actor) per match; WebSocket protocol with typed messages.
- [ ] Action timer + time bank; disconnect → time bank then auto-fold (never auto-win);
      reconnect resumes the match.
- [ ] Persist hands and match results; rating update in the same DB transaction.
- [ ] Audit log: store every hand's *unfiltered* event stream (includes all hole cards,
      folded and mucked). Clients only ever get events filtered by `Event::visibility`;
      player-facing hand histories show "mucks hand". Disclose this retention in the
      competition terms / privacy notice.

### 3. Web client
- [ ] Lobby / queue, table view, action controls, hand-history viewer, leaderboard.

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
