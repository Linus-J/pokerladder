//! Writes `web/src/lib/engine-fixture.json`: seeded matches played by the real engine
//! with random legal actions, as one seat sees them (events filtered by their
//! `visibility()`, the same rule the match actor applies), with the engine's own
//! public state after every step. The web client's
//! `engine-fixture.test.ts` replays the events through its reducer and checks it lands
//! on the same numbers, so the reducer is tested against the engine, not against
//! hand-written expectations.
//!
//! Regenerate with `UPDATE_FIXTURE=1 cargo test -p pokerladder-server --test
//! client_fixture`; without it the test fails if the checked-in file is stale.

use std::time::Duration;

use pokerladder_engine::holdem::{Action, Event, Hand, HandConfig, Seat, Visibility};
use pokerladder_engine::{Deck, Match, MatchConfig, MatchEvent};
use pokerladder_server::protocol::{WireEvent, WirePublicState};
use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};
use serde::Serialize;

#[derive(Serialize)]
struct Step {
    /// What the viewer receives for this step.
    events: Vec<WireEvent>,
    /// The engine's public state of the current hand afterwards (including a hand
    /// that has just completed).
    state: Option<WirePublicState>,
    /// Match stacks afterwards.
    stacks: [u64; 2],
}

#[derive(Serialize)]
struct Game {
    seed: u64,
    seat: u8,
    names: [String; 2],
    steps: Vec<Step>,
}

const MAX_HANDS: u32 = 40;

fn wire_state(h: &Hand) -> WirePublicState {
    (&h.public_state()).into()
}

fn play(seed: u64, viewer: Seat) -> Game {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut m = Match::new(MatchConfig::standard(), Seat::Zero).unwrap();
    let mut steps = Vec::new();
    // `Match` drops a hand once it completes, so a shadow `Hand` with the same deck and
    // actions supplies the public state at the end of each hand. It must agree with
    // the match's own hand event for event.
    let mut record =
        |m: &Match, shadow: &Hand, events: Vec<MatchEvent>, shadow_events: Vec<Event>| {
            let from_match: Vec<&Event> = events
                .iter()
                .filter_map(|e| match e {
                    MatchEvent::Hand(e) => Some(e),
                    _ => None,
                })
                .collect();
            assert_eq!(from_match, shadow_events.iter().collect::<Vec<_>>());
            if let Some(h) = m.hand() {
                assert_eq!(wire_state(h), wire_state(shadow));
            }
            let events = events
                .iter()
                .filter(|e| match e.visibility() {
                    Visibility::Public => true,
                    Visibility::Private(s) => s == viewer,
                })
                .map(WireEvent::from)
                .collect();
            steps.push(Step {
                events,
                state: Some(wire_state(shadow)),
                stacks: m.stacks(),
            });
        };
    while m.winner().is_none() && m.hands_played() < MAX_HANDS {
        let deck_seed = rng.random();
        let events = m
            .start_hand(Deck::shuffled(&mut StdRng::seed_from_u64(deck_seed)))
            .unwrap();
        let Some(MatchEvent::HandStarted {
            button,
            small_blind,
            big_blind,
            stacks,
            ..
        }) = events.first().cloned()
        else {
            panic!("a hand starts with HandStarted")
        };
        let config = HandConfig {
            button,
            stacks,
            small_blind,
            big_blind,
            auto_muck: [m.auto_muck(Seat::Zero), m.auto_muck(Seat::One)],
        };
        let (mut shadow, shadow_events) = Hand::new(
            config,
            Deck::shuffled(&mut StdRng::seed_from_u64(deck_seed)),
        )
        .unwrap();
        record(&m, &shadow, events, shadow_events);
        while let Some(legal) = m.hand().and_then(|h| h.legal()) {
            let roll: u32 = rng.random_range(0..100);
            let action = match (roll, legal.raise) {
                (0..12, _) if legal.can_fold => Action::Fold,
                (0..72, _) => {
                    if legal.can_check {
                        Action::Check
                    } else {
                        Action::Call
                    }
                }
                // Mostly small raises, so matches last long enough to cover a lot.
                (72..96, Some(r)) => {
                    let hi = r.max_to.min(r.min_to * 3);
                    Action::RaiseTo(rng.random_range(r.min_to..=hi))
                }
                (_, Some(_)) => Action::AllIn,
                _ => legal.passive_default(),
            };
            // Now and then a player runs out of time instead.
            let (events, shadow_events) = if rng.random_range(0..40) == 0 {
                let shadow_events = shadow.act(legal.seat, legal.passive_default()).unwrap();
                (m.timeout(legal.seat).unwrap(), shadow_events)
            } else {
                let shadow_events = shadow.act(legal.seat, action).unwrap();
                (
                    m.act(legal.seat, action, Duration::ZERO).unwrap(),
                    shadow_events,
                )
            };
            record(&m, &shadow, events, shadow_events);
        }
        assert!(shadow.is_complete());
    }
    Game {
        seed,
        seat: viewer.index() as u8,
        names: ["alice".into(), "bob".into()],
        steps,
    }
}

#[test]
fn engine_fixture_for_the_web_client() {
    pokerladder_engine::eval::init();
    let games: Vec<Game> = (0..8)
        .map(|i| play(1000 + i, if i % 2 == 0 { Seat::Zero } else { Seat::One }))
        .collect();
    assert!(
        games.iter().any(|g| g
            .steps
            .iter()
            .flat_map(|s| &s.events)
            .any(|e| matches!(e, WireEvent::Showdown { .. }))),
        "the fixture should include showdowns"
    );
    let json = serde_json::to_string(&games).unwrap() + "\n";
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../web/src/lib/engine-fixture.json"
    );
    if std::env::var_os("UPDATE_FIXTURE").is_some() {
        std::fs::write(path, json).unwrap();
    } else {
        let current = std::fs::read_to_string(path).unwrap_or_default();
        assert!(
            current == json,
            "{path} is stale: rerun with UPDATE_FIXTURE=1"
        );
    }
}
