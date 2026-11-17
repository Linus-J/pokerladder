//! The chip-dumping report against planted dumping: a seeded population of bots plays
//! whole matches through the real engine, one of them dumping chips to a partner, and
//! the report must single out exactly that pair. A "maniac" who shoves every hand
//! against everyone must *not* be flagged for weak commitments (their baseline is their
//! own play), and honest pairs must not be flagged at all.
//!
//! `season_report_from_postgres_matches_in_memory` needs `DATABASE_URL`, like the
//! other database tests.

use std::time::Duration;

use pokerladder_engine::eval::{self, Category};
use pokerladder_engine::holdem::{Action, Legal, Seat};
use pokerladder_engine::{Card, Deck, Match, MatchConfig, MatchEvent};
use pokerladder_server::integrity::hand::Strength;
use pokerladder_server::integrity::{Analyzer, Flag, MatchInfo, Report, SignalKind, load};
use pokerladder_server::protocol::WireEvent;
use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};
use sqlx::PgPool;
use sqlx::types::Json;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum Style {
    /// Plays by hand strength, with some randomness.
    Honest,
    /// Shoves every hand against everyone.
    Maniac,
    /// Honest, except against `partner`, where on `percent`% of decisions it dumps:
    /// folds to any bet (strong hands too) and shoves trash.
    Dumper { partner: usize, percent: u32 },
    /// Honest, except that against `against` it never acts (disconnected): every
    /// decision times out and the server checks or folds.
    Absent { against: usize },
}

const MANIAC: usize = 6;
const DUMPER: usize = 7;
const PARTNER: usize = 8;
const ABSENT: usize = 9;
const PLAYERS: usize = 10;

/// The population with a dumper who dumps on `percent`% of decisions.
fn styles(percent: u32) -> [Style; PLAYERS] {
    let mut s = [Style::Honest; PLAYERS];
    s[MANIAC] = Style::Maniac;
    s[DUMPER] = Style::Dumper {
        partner: PARTNER,
        percent,
    };
    s[ABSENT] = Style::Absent { against: 0 };
    s
}

/// Season ratings differ by player, so a swapped seat index shows up in results.
fn rating(p: usize) -> f64 {
    1400.0 + 25.0 * p as f64
}

fn names() -> Vec<String> {
    (0..PLAYERS)
        .map(|i| match i {
            MANIAC => "maniac".to_owned(),
            DUMPER => "dumper".to_owned(),
            PARTNER => "partner".to_owned(),
            ABSENT => "absent".to_owned(),
            i => format!("honest{i}"),
        })
        .collect()
}

fn raise_to(legal: &Legal, pot: u64, rng: &mut StdRng) -> Option<Action> {
    let r = legal.raise?;
    let target = r.min_to.max(pot * rng.random_range(5..=10) / 10);
    Some(Action::RaiseTo(target.min(r.max_to)))
}

fn passive(legal: &Legal) -> Action {
    if legal.can_check {
        Action::Check
    } else {
        Action::Call
    }
}

fn honest(legal: &Legal, hole: [Card; 2], board: &[Card], pot: u64, rng: &mut StdRng) -> Action {
    let roll = rng.random_range(0..100);
    if board.is_empty() {
        let top = Strength::of(hole, &[]).top.unwrap();
        return if top < 0.15 && roll < 70 {
            raise_to(legal, pot.max(60), rng).unwrap_or(passive(legal))
        } else if top < 0.65 || roll < 10 || legal.can_check {
            passive(legal)
        } else {
            Action::Fold
        };
    }
    let mut cards = vec![hole[0], hole[1]];
    cards.extend_from_slice(board);
    let cat = eval::evaluate(&cards).category();
    if cat >= Category::TwoPair {
        if roll < 60 {
            raise_to(legal, pot, rng).unwrap_or(passive(legal))
        } else {
            passive(legal)
        }
    } else if cat == Category::Pair || legal.can_check || roll < 15 {
        if legal.can_check && roll < 15 {
            raise_to(legal, pot, rng).unwrap_or(Action::Check)
        } else {
            passive(legal)
        }
    } else {
        Action::Fold
    }
}

fn decide(
    style: Style,
    opponent: usize,
    legal: &Legal,
    hole: [Card; 2],
    board: &[Card],
    pot: u64,
    rng: &mut StdRng,
) -> Option<Action> {
    Some(match style {
        Style::Maniac => {
            if legal.raise.is_some() {
                Action::AllIn
            } else {
                passive(legal)
            }
        }
        Style::Dumper { partner, percent }
            if partner == opponent && rng.random_range(0..100) < percent =>
        {
            if legal.can_fold {
                Action::Fold
            } else if Strength::of(hole, board).is_weak() && legal.raise.is_some() {
                Action::AllIn
            } else {
                Action::Check
            }
        }
        Style::Absent { against } if against == opponent => return None,
        _ => honest(legal, hole, board, pot, rng),
    })
}

struct SimMatch {
    players: [usize; 2],
    winner: u8,
    hands: Vec<Vec<WireEvent>>,
}

fn play(players: [usize; 2], styles: &[Style], rng: &mut StdRng) -> SimMatch {
    let button = if rng.random() { Seat::Zero } else { Seat::One };
    let mut m = Match::new(MatchConfig::standard(), button).unwrap();
    let mut hands = Vec::new();
    while m.winner().is_none() {
        let mut log: Vec<MatchEvent> = m.start_hand(Deck::shuffled(rng)).unwrap();
        while let Some(h) = m.hand() {
            let Some(legal) = h.legal() else { break };
            let s = legal.seat.index();
            let hole = h.hole_cards(legal.seat);
            let board = h.board().to_vec();
            let pot = h.pot();
            let action = decide(
                styles[players[s]],
                players[1 - s],
                &legal,
                hole,
                &board,
                pot,
                rng,
            );
            log.extend(match action {
                Some(a) => m.act(legal.seat, a, Duration::ZERO).unwrap(),
                None => m.timeout(legal.seat).unwrap(),
            });
        }
        hands.push(log.iter().map(WireEvent::from).collect());
    }
    SimMatch {
        players,
        winner: m.winner().unwrap().index() as u8,
        hands,
    }
}

/// Everyone plays everyone twice; the dumper and partner meet 12 more times.
fn season(seed: u64, styles: &[Style; PLAYERS]) -> Vec<SimMatch> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut out = Vec::new();
    for a in 0..PLAYERS {
        for b in a + 1..PLAYERS {
            for _ in 0..2 {
                out.push(play([a, b], styles, &mut rng));
            }
        }
    }
    for i in 0..12 {
        let seats = if i % 2 == 0 {
            [DUMPER, PARTNER]
        } else {
            [PARTNER, DUMPER]
        };
        out.push(play(seats, styles, &mut rng));
    }
    out
}

/// Feeds a simulated season to an analyzer as the database would hold it: match ids
/// from 1, hand ids from 1, all rated at each player's fixed [`rating`]. `ids` maps bot to user id.
fn analyse(sim: &[SimMatch], ids: &[i64]) -> Report {
    let mut a = Analyzer::new();
    for (i, name) in names().into_iter().enumerate() {
        a.add_name(ids[i], name);
    }
    let mut hand_id = 0;
    for (i, m) in sim.iter().enumerate() {
        let id = i as i64 + 1;
        a.add_match(match_info(id, m, ids));
        for (n, events) in m.hands.iter().enumerate() {
            hand_id += 1;
            a.add_hand(hand_id, id, n as i32 + 1, events);
        }
    }
    a.finish("2026-11")
}

fn match_info(id: i64, m: &SimMatch, ids: &[i64]) -> MatchInfo {
    MatchInfo {
        id,
        players: m.players.map(|p| ids[p]),
        status: "finished".into(),
        winner_seat: Some(m.winner as i16),
        hands_played: Some(m.hands.len() as i32),
        rated: true,
        ratings_before: Some(m.players.map(rating)),
    }
}

fn plain_ids() -> Vec<i64> {
    (1..=PLAYERS as i64).collect()
}

#[test]
fn planted_dumping_is_flagged_and_nothing_else_is() {
    eval::init();
    for seed in [1, 2, 3] {
        let sim = season(seed, &styles(100));
        let ids = plain_ids();
        let report = analyse(&sim, &ids);
        let text = report.render(false);
        assert!(report.skipped.is_empty(), "{:?}", report.skipped);

        let pair = report
            .pair(ids[DUMPER], ids[PARTNER])
            .unwrap_or_else(|| panic!("seed {seed}: no dumper/partner pair\n{text}"));
        // The dumper has the lower id, so is index 0 of the pair.
        assert_eq!(pair.names[0], "dumper");
        for kind in [SignalKind::StrongFolds, SignalKind::WeakCommits] {
            assert!(
                pair.flags.contains(&Flag::Signal(0, kind)),
                "seed {seed}: dumper not flagged for {kind:?}\n{text}"
            );
            let sig = pair.signal(0, kind);
            assert!(!sig.evidence.is_empty());
            for e in &sig.evidence {
                match kind {
                    SignalKind::StrongFolds => assert!(e.spot.strength.is_strong()),
                    SignalKind::WeakCommits => assert!(e.spot.strength.is_weak()),
                }
            }
            // The partner did nothing wrong.
            assert!(
                !pair.flags.contains(&Flag::Signal(1, kind)),
                "seed {seed}\n{text}"
            );
            // The baseline is the dumper's play against everyone else, excluding the
            // partner: the sum over the dumper's other pairs.
            let (mut k, mut n) = (0, 0);
            for p in &report.pairs {
                if p.players.contains(&ids[DUMPER]) && !p.players.contains(&ids[PARTNER]) {
                    let i = p.players.iter().position(|&id| id == ids[DUMPER]).unwrap();
                    k += p.signal(i, kind).count.k;
                    n += p.signal(i, kind).count.n;
                }
            }
            assert_eq!((sig.others.k, sig.others.n), (k, n), "{kind:?}");
            assert!(n > 0);
        }
        // Results are the noisiest signal (whole matches, few of them): only the
        // direction is certain here.
        let r = pair.results[0];
        assert!(
            (r.wins as f64) < r.expected && r.z < -1.0,
            "seed {seed}: results\n{text}"
        );
        assert!(text.contains("FLAGGED dumper vs partner"), "{text}");
        assert!(text.contains("Example hands (dumper)"), "{text}");

        // Nobody else is flagged; in particular the maniac's shoves are their normal
        // play, and their short matches are explained by their own record.
        for p in report.flagged() {
            assert_eq!(
                p.players,
                [ids[DUMPER], ids[PARTNER]],
                "seed {seed}: unexpected flag\n{text}"
            );
        }
        let mut maniac_weak = 0;
        for p in report.pairs.iter() {
            if let Some(i) = p.players.iter().position(|&id| id == ids[MANIAC]) {
                let weak = p.signal(i, SignalKind::WeakCommits);
                maniac_weak += weak.count.k;
                assert!(!weak.flagged, "seed {seed}\n{text}");
            }
        }
        assert!(
            maniac_weak >= 5,
            "the maniac shoves weak hands: {maniac_weak}"
        );

        // The absent player's automatic folds are listed apart, not as strong folds.
        let absent = report.pair(ids[0], ids[ABSENT]).unwrap();
        assert!(absent.strong_timeouts[1] > 0, "seed {seed}\n{text}");
        assert_eq!(absent.signal(1, SignalKind::StrongFolds).count.k, 0);
        assert!(
            report
                .render(true)
                .contains("absent timed out or sat out facing a bet with a strong hand"),
        );
    }
}

/// A subtler dumper: dumps on only half of their decisions against the partner and
/// plays honestly otherwise. Still caught on every seed, on at least one hand signal.
#[test]
fn half_hearted_dumping_is_still_flagged() {
    eval::init();
    for seed in [21, 22, 23] {
        let report = analyse(&season(seed, &styles(50)), &plain_ids());
        let text = report.render(false);
        let pair = report.pair(DUMPER as i64 + 1, PARTNER as i64 + 1).unwrap();
        assert!(
            pair.flags
                .contains(&Flag::Signal(0, SignalKind::StrongFolds))
                || pair
                    .flags
                    .contains(&Flag::Signal(0, SignalKind::WeakCommits)),
            "seed {seed}\n{text}"
        );
        for p in report.flagged() {
            assert_eq!(p.names, ["dumper", "partner"], "seed {seed}\n{text}");
        }
    }
}

#[test]
fn a_season_without_dumping_flags_nothing() {
    eval::init();
    // The same schedule (the "dumper" and "partner" still meet 14 times), everyone
    // honest.
    let sim = season(11, &[Style::Honest; PLAYERS]);
    let report = analyse(&sim, &plain_ids());
    let flagged: Vec<_> = report.flagged().map(|p| p.names.clone()).collect();
    assert!(flagged.is_empty(), "{flagged:?}\n{}", report.render(false));
}

#[sqlx::test(migrator = "pokerladder_server::db::MIGRATOR")]
async fn season_report_from_postgres_matches_in_memory(db: PgPool) {
    eval::init();
    let sim = season(1, &styles(100));
    let mut ids = Vec::new();
    for name in names() {
        let id = match pokerladder_server::db::create_user(&db, &name, &format!("{name}@x.io"), "h")
            .await
            .unwrap()
        {
            pokerladder_server::db::CreateUser::Created(id) => id,
            pokerladder_server::db::CreateUser::Taken(_) => panic!(),
        };
        ids.push(id);
    }
    let mut hand_id = 0i64;
    for (i, m) in sim.iter().enumerate() {
        let info = match_info(i as i64 + 1, m, &ids);
        sqlx::query(
            "INSERT INTO matches (id, player0, player1, status, winner_seat, hands_played,
                                  rating_before0, rating_before1, rated, season, started_at)
             VALUES ($1, $2, $3, 'finished', $4, $5, $6, $7, TRUE, '2026-11',
                     '2026-11-10T12:00:00Z')",
        )
        .bind(info.id)
        .bind(info.players[0])
        .bind(info.players[1])
        .bind(info.winner_seat)
        .bind(info.hands_played)
        .bind(info.ratings_before.unwrap()[0])
        .bind(info.ratings_before.unwrap()[1])
        .execute(&db)
        .await
        .unwrap();
        for (n, events) in m.hands.iter().enumerate() {
            hand_id += 1;
            sqlx::query(
                "INSERT INTO hands (id, match_id, number, started_at, audit_events,
                                    history_seat0, history_seat1, history_audit)
                 VALUES ($1, $2, $3, now(), $4, '', '', '')",
            )
            .bind(hand_id)
            .bind(info.id)
            .bind(n as i32 + 1)
            .bind(Json(events))
            .execute(&db)
            .await
            .unwrap();
        }
    }
    // Another season's match between the same pair (an unrated one dated by its start)
    // and an unreadable hand in it must not leak into November.
    sqlx::query(
        "INSERT INTO matches (id, player0, player1, status, rated, started_at)
         VALUES (100000, $1, $2, 'abandoned', FALSE, '2026-10-31T23:59:00Z')",
    )
    .bind(ids[DUMPER])
    .bind(ids[PARTNER])
    .execute(&db)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO hands (id, match_id, number, started_at, audit_events,
                            history_seat0, history_seat1, history_audit)
         VALUES (100000, 100000, 1, now(), '[{\"type\": \"nonsense\"}]', '', '', '')",
    )
    .execute(&db)
    .await
    .unwrap();

    let from_db = load::season_report(&db, "2026-11").await.unwrap();
    let in_memory = analyse(&sim, &ids);
    assert_eq!(from_db.render(true), in_memory.render(true));
    assert!(from_db.pair(ids[DUMPER], ids[PARTNER]).unwrap().flags.len() >= 2);

    // October holds just the abandoned match, whose hand is listed as unreadable.
    let october = load::season_report(&db, "2026-10").await.unwrap();
    assert_eq!((october.matches, october.hands), (1, 0));
    assert_eq!(october.skipped.len(), 1);
    assert_eq!(october.skipped[0].0, 100000);
    assert!(october.flagged().next().is_none());
}
