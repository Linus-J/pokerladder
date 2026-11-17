//! Facts about one hand, read from its stored audit events (the unfiltered
//! [`WireEvent`] stream, which has both players' hole cards).
//!
//! Hand strength is always measured against a *random* opponent holding with the board
//! the player could see, never against the opponent's actual cards: a player can't
//! know those, so folding a hand that happens to be ahead is not suspicious in itself.
//! What the report compares is how often a player does it against one opponent versus
//! everyone else.

use std::sync::LazyLock;

use pokerladder_engine::Card;
use pokerladder_engine::eval::equity::{self, PREFLOP_VS_RANDOM};

use crate::protocol::{WireAction, WireEvent};

/// Preflop "strong" means a starting hand in the top 10% of the 1326 combinations by
/// equity against a random hand (88+, A9s+, ATo+ and similar).
pub const STRONG_PREFLOP_TOP: f64 = 0.10;
/// Postflop "strong" means beating at least 80% of random holdings (counting the
/// remaining runouts).
pub const STRONG_POSTFLOP_SHARE: f64 = 0.80;
/// Preflop "weak" means a starting hand in the bottom 30% of combinations.
pub const WEAK_PREFLOP_BOTTOM: f64 = 0.30;
/// Postflop "weak" means less than 30% equity against a random holding.
pub const WEAK_POSTFLOP_SHARE: f64 = 0.30;
/// A commitment is "big" once a player has put at least this share of their stack
/// at the start of the hand into the pot.
pub const BIG_COMMIT_SHARE: f64 = 0.5;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Street {
    Preflop,
    Flop,
    Turn,
    River,
}

impl Street {
    fn from_board(n: usize) -> Street {
        match n {
            0 => Street::Preflop,
            3 => Street::Flop,
            4 => Street::Turn,
            _ => Street::River,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Street::Preflop => "preflop",
            Street::Flop => "on the flop",
            Street::Turn => "on the turn",
            Street::River => "on the river",
        }
    }
}

/// Equity against a random holding; preflop also the starting hand's rank among all
/// 1326 combinations.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Strength {
    /// Pot share against a random holding, 0..=1.
    pub share: f64,
    /// Preflop only: the fraction of starting combinations that are strictly
    /// stronger (0 = aces).
    pub top: Option<f64>,
}

impl Strength {
    pub fn of(hole: [Card; 2], board: &[Card]) -> Strength {
        let share = equity::vs_random(hole, board).share();
        let top = board
            .is_empty()
            .then(|| PREFLOP_TOP[equity::preflop_class(hole)]);
        Strength { share, top }
    }

    pub fn is_strong(self) -> bool {
        match self.top {
            Some(top) => top < STRONG_PREFLOP_TOP,
            None => self.share >= STRONG_POSTFLOP_SHARE,
        }
    }

    pub fn is_weak(self) -> bool {
        match self.top {
            Some(top) => top >= 1.0 - WEAK_PREFLOP_BOTTOM,
            None => self.share < WEAK_POSTFLOP_SHARE,
        }
    }
}

/// For each preflop class: the fraction of the 1326 combinations in strictly stronger
/// classes.
static PREFLOP_TOP: LazyLock<[f64; 169]> = LazyLock::new(|| {
    let share = |k: usize| {
        let [w, t] = PREFLOP_VS_RANDOM[k];
        w as f64 + t as f64 / 2.0
    };
    let combos = |k: usize| match (k / 13).cmp(&(k % 13)) {
        std::cmp::Ordering::Equal => 6.0,
        std::cmp::Ordering::Greater => 4.0,
        std::cmp::Ordering::Less => 12.0,
    };
    std::array::from_fn(|k| {
        (0..169)
            .filter(|&j| share(j) > share(k))
            .map(combos)
            .sum::<f64>()
            / 1326.0
    })
});

/// A decision worth reporting: where it happened and what the player held.
#[derive(Clone, Debug, PartialEq)]
pub struct Spot {
    pub street: Street,
    pub hole: [Card; 2],
    pub board: Vec<Card>,
    /// The opponent's hole cards (from the audit log), for the reviewer.
    pub opponent_hole: [Card; 2],
    /// Chips needed to call before acting.
    pub to_call: u64,
    /// Chips in the pot before acting (including both players' bets this street).
    pub pot: u64,
    pub action: WireAction,
    pub strength: Strength,
}

/// What one seat did in one hand, as far as the integrity signals are concerned.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SeatFacts {
    /// Faced a bet at least once while holding a strong hand.
    pub strong_faced: bool,
    /// Folded a strong hand facing a bet.
    pub strong_fold: Option<Spot>,
    /// Ran out of time (or was sitting out) facing a bet with a strong hand, so the
    /// server folded it. Kept apart from `strong_faced`/`strong_fold`: a disconnected
    /// player isn't choosing to fold, though sitting out could itself be a way to
    /// dump, so the report shows these separately.
    pub strong_timeout: bool,
    /// The first action whose *voluntary* chips (beyond the blind) reached
    /// [`BIG_COMMIT_SHARE`] of this seat's starting stack. Counting voluntary chips
    /// only keeps a short stack's forced call of a few chips out of it.
    pub big_commit: Option<Spot>,
    /// Chips won (positive) or lost over the hand.
    pub net: i64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HandFacts {
    pub seats: [SeatFacts; 2],
}

#[derive(Debug, PartialEq, Eq)]
pub struct BadHand(pub String);

impl std::fmt::Display for BadHand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn bad(msg: impl Into<String>) -> BadHand {
    BadHand(msg.into())
}

fn seat(s: u8) -> Result<usize, BadHand> {
    match s {
        0 | 1 => Ok(s as usize),
        _ => Err(bad(format!("seat {s}"))),
    }
}

fn parse_card(s: &str) -> Result<Card, BadHand> {
    s.parse().map_err(|_| bad(format!("card {s:?}")))
}

fn parse_pair(c: &[String; 2]) -> Result<[Card; 2], BadHand> {
    Ok([parse_card(&c[0])?, parse_card(&c[1])?])
}

/// Rejects a stream that repeats a card (the evaluator would panic on it).
fn check_distinct(hole: &[Option<[Card; 2]>; 2], board: &[Card]) -> Result<(), BadHand> {
    let mut cards: Vec<Card> = hole.iter().flatten().flatten().copied().collect();
    cards.extend_from_slice(board);
    match pokerladder_engine::card::CardSet::from_cards(&cards) {
        Some(_) => Ok(()),
        None => Err(bad("duplicate cards")),
    }
}

/// Reads one hand's audit events. Fails on a stream that doesn't describe a whole
/// hand with both players' hole cards (it can't come from the server, but the report
/// should skip it rather than guess).
pub fn analyse(events: &[WireEvent]) -> Result<HandFacts, BadHand> {
    let mut start: Option<[u64; 2]> = None;
    let mut end: Option<[u64; 2]> = None;
    let mut hole: [Option<[Card; 2]>; 2] = [None, None];
    let mut board: Vec<Card> = Vec::with_capacity(5);
    let mut street_total = [0u64; 2];
    let mut committed = [0u64; 2];
    let mut voluntary = [0u64; 2];
    // Set by TimedOut; the engine's automatic check/fold follows for that seat.
    let mut timed_out: Option<usize> = None;
    // Strength per seat per board size (0, 3, 4, 5 cards): one flop enumeration is
    // ~3 ms, and a bet/raise/reraise would otherwise repeat it.
    let mut strength_cache: [[Option<Strength>; 4]; 2] = [[None; 4]; 2];
    let mut facts = HandFacts::default();

    for e in events {
        match e {
            WireEvent::HandStarted { stacks, .. } => {
                if start.is_some() {
                    return Err(bad("two HandStarted events"));
                }
                start = Some(*stacks);
            }
            WireEvent::BlindPosted {
                seat: s, amount, ..
            } => {
                let s = seat(*s)?;
                street_total[s] += amount;
                committed[s] += amount;
            }
            WireEvent::HoleCards { seat: s, cards } => {
                let s = seat(*s)?;
                if hole[s].is_some() {
                    return Err(bad("two HoleCards events for one seat"));
                }
                hole[s] = Some(parse_pair(cards)?);
                check_distinct(&hole, &board)?;
            }
            WireEvent::Board { cards, .. } => {
                for c in cards {
                    board.push(parse_card(c)?);
                }
                if !matches!(board.len(), 3..=5) {
                    return Err(bad(format!("board of {} cards", board.len())));
                }
                check_distinct(&hole, &board)?;
                street_total = [0, 0];
            }
            WireEvent::TimedOut { seat: s } => timed_out = Some(seat(*s)?),
            WireEvent::Acted {
                seat: s,
                action,
                facing,
                added,
                street_total: total,
                ..
            } => {
                let s = seat(*s)?;
                let start = start.ok_or_else(|| bad("action before HandStarted"))?;
                let (Some(mine), Some(theirs)) = (hole[s], hole[1 - s]) else {
                    return Err(bad("action before both hands were dealt"));
                };
                let to_call = facing.saturating_sub(street_total[s]);
                let pot = committed[0] + committed[1];
                let seat_facts = &mut facts.seats[s];
                let after = committed[s] + added;
                let was_timeout = timed_out.take() == Some(s);
                let big = *added > 0
                    && seat_facts.big_commit.is_none()
                    && (voluntary[s] + added) as f64 >= BIG_COMMIT_SHARE * start[s] as f64;
                let folded = matches!(action, WireAction::Fold);
                if to_call > 0 || big {
                    let cached = &mut strength_cache[s][board.len().saturating_sub(2)];
                    let strength = *cached.get_or_insert_with(|| Strength::of(mine, &board));
                    let spot = || Spot {
                        street: Street::from_board(board.len()),
                        hole: mine,
                        board: board.clone(),
                        opponent_hole: theirs,
                        to_call,
                        pot,
                        action: *action,
                        strength,
                    };
                    if to_call > 0 && strength.is_strong() && was_timeout {
                        seat_facts.strong_timeout = true;
                    } else if to_call > 0 && strength.is_strong() {
                        seat_facts.strong_faced = true;
                        if folded {
                            seat_facts.strong_fold = Some(spot());
                        }
                    }
                    if big {
                        seat_facts.big_commit = Some(spot());
                    }
                }
                street_total[s] = *total;
                committed[s] = after;
                voluntary[s] += added;
            }
            WireEvent::UncalledReturned { seat: s, amount } => {
                let s = seat(*s)?;
                committed[s] = committed[s].saturating_sub(*amount);
            }
            WireEvent::Ended { stacks } => end = Some(*stacks),
            WireEvent::HandsTabled { .. }
            | WireEvent::Showdown { .. }
            | WireEvent::Mucked { .. }
            | WireEvent::Won { .. }
            | WireEvent::MatchOver { .. } => {}
        }
    }
    let (Some(start), Some(end)) = (start, end) else {
        return Err(bad("hand without HandStarted and Ended"));
    };
    if hole.iter().any(Option::is_none) {
        return Err(bad("missing hole cards"));
    }
    for s in 0..2 {
        facts.seats[s].net = end[s] as i64 - start[s] as i64;
    }
    if facts.seats[0].net + facts.seats[1].net != 0 {
        return Err(bad("chips not conserved"));
    }
    Ok(facts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cards(s: &str) -> [Card; 2] {
        pokerladder_engine::card::parse_cards(s)
            .unwrap()
            .try_into()
            .unwrap()
    }

    fn started() -> WireEvent {
        WireEvent::HandStarted {
            number: 1,
            level: 1,
            button: 0,
            small_blind: 10,
            big_blind: 20,
            stacks: [1500, 1500],
            time_banks_ms: [60_000, 60_000],
        }
    }

    fn blinds() -> [WireEvent; 2] {
        [
            WireEvent::BlindPosted {
                seat: 0,
                amount: 10,
                big: false,
                all_in: false,
            },
            WireEvent::BlindPosted {
                seat: 1,
                amount: 20,
                big: true,
                all_in: false,
            },
        ]
    }

    fn hole(seat: u8, a: &str, b: &str) -> WireEvent {
        WireEvent::HoleCards {
            seat,
            cards: [a.into(), b.into()],
        }
    }

    fn acted(seat: u8, action: WireAction, facing: u64, added: u64, total: u64) -> WireEvent {
        WireEvent::Acted {
            seat,
            action,
            facing,
            added,
            street_total: total,
            all_in: false,
        }
    }

    /// The big blind checking aces after a limp never faced a bet; the limper folding
    /// trash to a flop bet did, but wasn't strong.
    #[test]
    fn checking_the_option_is_not_facing_a_bet() {
        pokerladder_engine::eval::init();
        let mut events = vec![started()];
        events.extend(blinds());
        events.extend([
            hole(0, "7c", "2d"),
            hole(1, "As", "Ah"),
            acted(0, WireAction::Call, 20, 10, 20),
            acted(1, WireAction::Check, 20, 0, 20),
            WireEvent::Board {
                street: crate::protocol::WireStreet::Flop,
                cards: vec!["Kc".into(), "8d".into(), "3h".into()],
            },
            acted(1, WireAction::RaiseTo { amount: 40 }, 0, 40, 40),
            acted(0, WireAction::Fold, 40, 0, 0),
            WireEvent::UncalledReturned {
                seat: 1,
                amount: 40,
            },
            WireEvent::Won {
                seat: 1,
                amount: 40,
            },
            WireEvent::Ended {
                stacks: [1480, 1520],
            },
        ]);
        let f = analyse(&events).unwrap();
        assert_eq!(
            f.seats[1],
            SeatFacts {
                net: 20,
                ..SeatFacts::default()
            }
        );
        assert_eq!(
            f.seats[0],
            SeatFacts {
                net: -20,
                ..SeatFacts::default()
            }
        );
    }

    /// Aces raise, trash shoves (a big, weak commitment), aces fold (a strong fold).
    #[test]
    fn strong_fold_and_weak_shove_are_recorded_exactly() {
        pokerladder_engine::eval::init();
        let mut events = vec![started()];
        events.extend(blinds());
        events.extend([
            hole(0, "As", "Ad"),
            hole(1, "7c", "2d"),
            acted(0, WireAction::RaiseTo { amount: 60 }, 20, 50, 60),
            WireEvent::Acted {
                seat: 1,
                // The engine stores a shove as the raise it resolved to.
                action: WireAction::RaiseTo { amount: 1500 },
                facing: 60,
                added: 1480,
                street_total: 1500,
                all_in: true,
            },
            acted(0, WireAction::Fold, 1500, 0, 60),
            WireEvent::UncalledReturned {
                seat: 1,
                amount: 1440,
            },
            WireEvent::Won {
                seat: 1,
                amount: 120,
            },
            WireEvent::Ended {
                stacks: [1440, 1560],
            },
        ]);
        let f = analyse(&events).unwrap();
        let aces = Strength::of(cards("AsAd"), &[]);
        let trash = Strength::of(cards("7c2d"), &[]);
        assert!(aces.is_strong() && trash.is_weak());
        assert_eq!(
            f.seats[0],
            SeatFacts {
                // The opening raise faced the big blind (10 to call) holding aces.
                strong_faced: true,
                strong_fold: Some(Spot {
                    street: Street::Preflop,
                    hole: cards("AsAd"),
                    board: vec![],
                    opponent_hole: cards("7c2d"),
                    to_call: 1440,
                    pot: 1560,
                    action: WireAction::Fold,
                    strength: aces,
                }),
                strong_timeout: false,
                big_commit: None,
                net: -60,
            }
        );
        assert_eq!(
            f.seats[1],
            SeatFacts {
                strong_faced: false,
                strong_fold: None,
                strong_timeout: false,
                big_commit: Some(Spot {
                    street: Street::Preflop,
                    hole: cards("7c2d"),
                    board: vec![],
                    opponent_hole: cards("AsAd"),
                    to_call: 40,
                    pot: 80,
                    action: WireAction::RaiseTo { amount: 1500 },
                    strength: trash,
                }),
                net: 60,
            }
        );
    }

    /// A seat that timed out (e.g. disconnected) facing a bet with aces didn't choose
    /// to fold: recorded apart from strong folds.
    #[test]
    fn timed_out_folds_are_not_strong_folds() {
        pokerladder_engine::eval::init();
        let mut events = vec![started()];
        events.extend(blinds());
        events.extend([
            hole(0, "As", "Ad"),
            hole(1, "7c", "2d"),
            WireEvent::TimedOut { seat: 0 },
            acted(0, WireAction::Fold, 20, 0, 10),
            WireEvent::Won {
                seat: 1,
                amount: 30,
            },
            WireEvent::Ended {
                stacks: [1490, 1510],
            },
        ]);
        let f = analyse(&events).unwrap();
        assert_eq!(
            f.seats[0],
            SeatFacts {
                strong_timeout: true,
                net: -10,
                ..SeatFacts::default()
            }
        );
        // Without the timeout it is an ordinary strong fold.
        events.remove(5);
        let f = analyse(&events).unwrap();
        assert!(f.seats[0].strong_fold.is_some() && !f.seats[0].strong_timeout);
    }

    /// A short stack calling a few chips off after posting the big blind isn't a big
    /// commitment; a deep stack putting most of it in voluntarily is.
    #[test]
    fn forced_blinds_dont_make_a_commitment_big() {
        pokerladder_engine::eval::init();
        let big = |bb_stack: u64, call: u64| {
            let mut events = vec![WireEvent::HandStarted {
                number: 1,
                level: 1,
                button: 0,
                small_blind: 10,
                big_blind: 20,
                stacks: [3000 - bb_stack, bb_stack],
                time_banks_ms: [0, 0],
            }];
            events.extend(blinds());
            events.extend([
                hole(0, "As", "Ad"),
                hole(1, "7c", "2d"),
                WireEvent::Acted {
                    seat: 0,
                    action: WireAction::RaiseTo { amount: 20 + call },
                    facing: 20,
                    added: 10 + call,
                    street_total: 20 + call,
                    all_in: false,
                },
                WireEvent::Acted {
                    seat: 1,
                    action: WireAction::Call,
                    facing: 20 + call,
                    added: call,
                    street_total: 20 + call,
                    all_in: true,
                },
                WireEvent::Ended {
                    stacks: [3000 - bb_stack, bb_stack],
                },
            ]);
            analyse(&events).unwrap().seats[1].big_commit.is_some()
        };
        assert!(!big(25, 5), "5 chips voluntarily from a 25-chip stack");
        assert!(big(100, 80), "80 of 100 chips voluntarily");
    }

    #[test]
    fn malformed_hands_are_rejected() {
        let mut events = vec![started()];
        events.extend(blinds());
        events.push(hole(0, "As", "Ad"));
        events.push(WireEvent::Ended {
            stacks: [1490, 1510],
        });
        assert_eq!(analyse(&events), Err(bad("missing hole cards")));
        events.insert(3, hole(1, "7c", "2d"));
        assert!(analyse(&events).is_ok());
        let last = events.len() - 1;
        events[last] = WireEvent::Ended {
            stacks: [1490, 1500],
        };
        assert_eq!(analyse(&events), Err(bad("chips not conserved")));
        events[3] = hole(1, "As", "2d");
        assert_eq!(analyse(&events), Err(bad("duplicate cards")));
        events[3] = hole(1, "Zz", "2d");
        assert!(analyse(&events).is_err());
        assert!(analyse(&[]).is_err());
    }

    #[test]
    fn preflop_ranks_put_aces_first_and_trash_last() {
        pokerladder_engine::eval::init();
        let top = |s: &str| Strength::of(cards(s), &[]).top.unwrap();
        assert_eq!(top("AsAh"), 0.0);
        assert!(top("KsKh") < 0.01);
        assert!(top("7c2d") > 0.95);
        // The top 10% is a tight range: 99 and AKo are in, KTo and 22 are not.
        assert!(Strength::of(cards("9s9h"), &[]).is_strong());
        assert!(Strength::of(cards("AsKh"), &[]).is_strong());
        assert!(!Strength::of(cards("KsTh"), &[]).is_strong());
        assert!(!Strength::of(cards("2s2h"), &[]).is_strong());
        assert!(Strength::of(cards("7c2d"), &[]).is_weak());
        assert!(!Strength::of(cards("AsKh"), &[]).is_weak());
    }
}
