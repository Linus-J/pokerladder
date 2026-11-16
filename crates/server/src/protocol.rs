//! The WebSocket protocol: JSON messages tagged by `type`. TypeScript definitions for
//! the web client are generated from these types by `ts-rs` (see the `export_bindings`
//! test), so client and server can't drift.
//!
//! Cards are strings such as `"As"`, `"Td"`; seats are `0`/`1`; durations are whole
//! milliseconds; chips are plain numbers (the engine caps them at 2^53, so they are
//! exact in JavaScript).

use std::time::Duration;

use pokerladder_engine::holdem::{Action, Event, Legal, PublicState, Seat, Street};
use pokerladder_engine::{Card, MatchEvent};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub type UserId = i64;
pub type MatchId = i64;

/// Messages from the browser. Nothing here is trusted: the server knows who the user
/// is from their session and which seat they hold.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    /// Join the matchmaking queue.
    QueueJoin,
    QueueLeave,
    /// Act in the user's current match.
    Act {
        action: WireAction,
    },
    /// Muck losing hands at showdown (the default) or show them. From the next hand.
    SetAutoMuck {
        on: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    Hello {
        #[ts(type = "number")]
        user_id: UserId,
        name: String,
        rating: f64,
        email_verified: bool,
        /// Whether the user is seated in a live match (a `MatchState` follows if so).
        /// A client still showing a table when this is false missed the match's end.
        in_match: bool,
    },
    Queued,
    QueueLeft,
    /// Full table state: sent when a match starts and on every reconnect. Replaces
    /// anything the client had for this match.
    MatchState(Box<MatchSnapshot>),
    /// One event of the match, already filtered for this player.
    Event {
        #[ts(type = "number")]
        match_id: MatchId,
        event: WireEvent,
    },
    /// Someone must act. Sent after every change of decision (and in `MatchState`).
    Turn {
        #[ts(type = "number")]
        match_id: MatchId,
        turn: TurnInfo,
    },
    MatchEnded {
        #[ts(type = "number")]
        match_id: MatchId,
        winner: u8,
        /// Indexed by seat; `None` if the result could not be recorded.
        ratings: Option<[RatingChange; 2]>,
    },
    /// A request was refused; nothing changed.
    Error {
        code: ErrorCode,
        message: String,
    },
    /// This connection was superseded by a newer one for the same account and is about
    /// to close.
    Replaced,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    BadMessage,
    EmailNotVerified,
    AlreadyInMatch,
    NotInMatch,
    IllegalAction,
    Unavailable,
    RateLimited,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MatchSnapshot {
    #[ts(type = "number")]
    pub match_id: MatchId,
    /// The receiving player's seat.
    pub seat: u8,
    /// Indexed by seat.
    pub names: [String; 2],
    pub ratings: [f64; 2],
    pub hands_played: u32,
    #[ts(type = "[number, number]")]
    pub stacks: [u64; 2],
    #[ts(type = "[number, number]")]
    pub time_banks_ms: [u64; 2],
    #[ts(type = "number")]
    pub per_action_ms: u64,
    /// The hand in progress, if any.
    pub hand: Option<HandSnapshot>,
    pub turn: Option<TurnInfo>,
    pub winner: Option<u8>,
    /// The receiving player's auto-muck setting (takes effect from the next hand).
    pub auto_muck: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct HandSnapshot {
    pub state: WirePublicState,
    /// This hand's events so far, filtered for the receiving player (so it includes
    /// their own hole cards and anything already tabled or shown).
    pub events: Vec<WireEvent>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct TurnInfo {
    pub seat: u8,
    pub legal: WireLegal,
    /// Time left for this decision when the message was sent (free time + bank).
    #[ts(type = "number")]
    pub remaining_ms: u64,
}

#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct RatingChange {
    pub before: f64,
    pub after: f64,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WireAction {
    Fold,
    Check,
    Call,
    /// Bet or raise to a street total of `amount`.
    RaiseTo {
        #[ts(type = "number")]
        amount: u64,
    },
    AllIn,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum WireStreet {
    Preflop,
    Flop,
    Turn,
    River,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct WireLegal {
    pub seat: u8,
    pub can_fold: bool,
    pub can_check: bool,
    #[ts(type = "number | null")]
    pub call: Option<u64>,
    #[ts(type = "number | null")]
    pub raise_min_to: Option<u64>,
    #[ts(type = "number | null")]
    pub raise_max_to: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct WirePublicState {
    pub button: u8,
    pub street: WireStreet,
    pub board: Vec<String>,
    #[ts(type = "[number, number]")]
    pub stacks: [u64; 2],
    #[ts(type = "[number, number]")]
    pub street_bets: [u64; 2],
    #[ts(type = "[number, number]")]
    pub committed: [u64; 2],
    #[ts(type = "number")]
    pub pot: u64,
    pub complete: bool,
}

/// A [`MatchEvent`] on the wire. The server also stores the unfiltered stream of these
/// as each hand's audit log.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WireEvent {
    HandStarted {
        number: u32,
        level: u32,
        button: u8,
        #[ts(type = "number")]
        small_blind: u64,
        #[ts(type = "number")]
        big_blind: u64,
        #[ts(type = "[number, number]")]
        stacks: [u64; 2],
        #[ts(type = "[number, number]")]
        time_banks_ms: [u64; 2],
    },
    BlindPosted {
        seat: u8,
        #[ts(type = "number")]
        amount: u64,
        big: bool,
        all_in: bool,
    },
    /// Only ever sent to `seat` itself.
    HoleCards {
        seat: u8,
        cards: [String; 2],
    },
    Acted {
        seat: u8,
        action: WireAction,
        #[ts(type = "number")]
        facing: u64,
        #[ts(type = "number")]
        added: u64,
        #[ts(type = "number")]
        street_total: u64,
        all_in: bool,
    },
    HandsTabled {
        cards: [[String; 2]; 2],
    },
    Board {
        street: WireStreet,
        cards: Vec<String>,
    },
    UncalledReturned {
        seat: u8,
        #[ts(type = "number")]
        amount: u64,
    },
    Showdown {
        seat: u8,
        cards: [String; 2],
        /// 1..=7462, higher wins.
        rank: u16,
        description: String,
    },
    Mucked {
        seat: u8,
    },
    Won {
        seat: u8,
        #[ts(type = "number")]
        amount: u64,
    },
    Ended {
        #[ts(type = "[number, number]")]
        stacks: [u64; 2],
    },
    TimedOut {
        seat: u8,
    },
    MatchOver {
        winner: u8,
    },
}

pub fn seat_u8(s: Seat) -> u8 {
    s.index() as u8
}

pub fn seat_from_u8(s: u8) -> Option<Seat> {
    match s {
        0 => Some(Seat::Zero),
        1 => Some(Seat::One),
        _ => None,
    }
}

pub fn millis(d: Duration) -> u64 {
    d.as_millis().min(u64::MAX as u128) as u64
}

fn card(c: Card) -> String {
    c.to_string()
}

fn pair(c: [Card; 2]) -> [String; 2] {
    c.map(card)
}

impl From<Street> for WireStreet {
    fn from(s: Street) -> Self {
        match s {
            Street::Preflop => WireStreet::Preflop,
            Street::Flop => WireStreet::Flop,
            Street::Turn => WireStreet::Turn,
            Street::River => WireStreet::River,
        }
    }
}

impl From<Action> for WireAction {
    fn from(a: Action) -> Self {
        match a {
            Action::Fold => WireAction::Fold,
            Action::Check => WireAction::Check,
            Action::Call => WireAction::Call,
            Action::RaiseTo(amount) => WireAction::RaiseTo { amount },
            Action::AllIn => WireAction::AllIn,
        }
    }
}

impl From<WireAction> for Action {
    fn from(a: WireAction) -> Self {
        match a {
            WireAction::Fold => Action::Fold,
            WireAction::Check => Action::Check,
            WireAction::Call => Action::Call,
            WireAction::RaiseTo { amount } => Action::RaiseTo(amount),
            WireAction::AllIn => Action::AllIn,
        }
    }
}

impl From<Legal> for WireLegal {
    fn from(l: Legal) -> Self {
        WireLegal {
            seat: seat_u8(l.seat),
            can_fold: l.can_fold,
            can_check: l.can_check,
            call: l.call,
            raise_min_to: l.raise.map(|r| r.min_to),
            raise_max_to: l.raise.map(|r| r.max_to),
        }
    }
}

impl From<&PublicState> for WirePublicState {
    fn from(p: &PublicState) -> Self {
        WirePublicState {
            button: seat_u8(p.button),
            street: p.street.into(),
            board: p.board.iter().copied().map(card).collect(),
            stacks: p.stacks,
            street_bets: p.street_bets,
            committed: p.committed,
            pot: p.pot,
            complete: p.complete,
        }
    }
}

impl From<&MatchEvent> for WireEvent {
    fn from(e: &MatchEvent) -> Self {
        match e {
            MatchEvent::HandStarted {
                number,
                level,
                button,
                small_blind,
                big_blind,
                stacks,
                time_banks,
            } => WireEvent::HandStarted {
                number: *number,
                level: *level,
                button: seat_u8(*button),
                small_blind: *small_blind,
                big_blind: *big_blind,
                stacks: *stacks,
                time_banks_ms: time_banks.map(millis),
            },
            MatchEvent::TimedOut { seat } => WireEvent::TimedOut {
                seat: seat_u8(*seat),
            },
            MatchEvent::MatchOver { winner } => WireEvent::MatchOver {
                winner: seat_u8(*winner),
            },
            MatchEvent::Hand(e) => match e {
                Event::BlindPosted {
                    seat,
                    amount,
                    big,
                    all_in,
                } => WireEvent::BlindPosted {
                    seat: seat_u8(*seat),
                    amount: *amount,
                    big: *big,
                    all_in: *all_in,
                },
                Event::HoleCards { seat, cards } => WireEvent::HoleCards {
                    seat: seat_u8(*seat),
                    cards: pair(*cards.reveal()),
                },
                Event::Acted {
                    seat,
                    action,
                    facing,
                    added,
                    street_total,
                    all_in,
                } => WireEvent::Acted {
                    seat: seat_u8(*seat),
                    action: (*action).into(),
                    facing: *facing,
                    added: *added,
                    street_total: *street_total,
                    all_in: *all_in,
                },
                Event::HandsTabled { cards } => WireEvent::HandsTabled {
                    cards: cards.map(pair),
                },
                Event::Board { street, cards } => WireEvent::Board {
                    street: (*street).into(),
                    cards: cards.iter().copied().map(card).collect(),
                },
                Event::UncalledReturned { seat, amount } => WireEvent::UncalledReturned {
                    seat: seat_u8(*seat),
                    amount: *amount,
                },
                Event::Showdown { seat, cards, rank } => WireEvent::Showdown {
                    seat: seat_u8(*seat),
                    cards: pair(*cards),
                    rank: rank.value(),
                    description: rank.describe(),
                },
                Event::Mucked { seat } => WireEvent::Mucked {
                    seat: seat_u8(*seat),
                },
                Event::Won { seat, amount } => WireEvent::Won {
                    seat: seat_u8(*seat),
                    amount: *amount,
                },
                Event::Ended { stacks } => WireEvent::Ended { stacks: *stacks },
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes the TypeScript definitions to `web/src/lib/protocol.ts`. Run with
    /// `cargo test -p pokerladder-server export_bindings` after changing the protocol;
    /// the test also fails if the checked-in file is stale.
    #[test]
    fn export_bindings() {
        let decls = [
            ClientMsg::decl(),
            ServerMsg::decl(),
            ErrorCode::decl(),
            MatchSnapshot::decl(),
            HandSnapshot::decl(),
            TurnInfo::decl(),
            RatingChange::decl(),
            WireAction::decl(),
            WireStreet::decl(),
            WireLegal::decl(),
            WirePublicState::decl(),
            WireEvent::decl(),
            crate::auth::MeResponse::decl(),
            crate::auth::RegisterResponse::decl(),
            crate::db::LeaderboardRow::decl(),
            crate::history::MatchSummary::decl(),
            crate::history::HandHistory::decl(),
        ];
        let mut out = String::from(
            "// Generated from crates/server/src/protocol.rs by the `export_bindings` test.\n\
             // Do not edit by hand.\n",
        );
        for d in decls {
            out.push_str("\nexport ");
            out.push_str(&d);
            out.push('\n');
        }
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../web/src/lib/protocol.ts");
        let current = std::fs::read_to_string(path).unwrap_or_default();
        if current != out {
            std::fs::create_dir_all(std::path::Path::new(path).parent().unwrap()).unwrap();
            std::fs::write(path, &out).unwrap();
            if std::env::var_os("CI").is_some() {
                panic!("web/src/lib/protocol.ts was stale; regenerated it");
            }
        }
    }

    #[test]
    fn messages_round_trip_as_tagged_json() {
        let m = ClientMsg::Act {
            action: WireAction::RaiseTo { amount: 60 },
        };
        let json = serde_json::to_string(&m).unwrap();
        assert_eq!(
            json,
            r#"{"type":"act","action":{"kind":"raise_to","amount":60}}"#
        );
        assert_eq!(serde_json::from_str::<ClientMsg>(&json).unwrap(), m);
        assert_eq!(
            serde_json::from_str::<ClientMsg>(r#"{"type":"queue_join"}"#).unwrap(),
            ClientMsg::QueueJoin
        );
        assert!(serde_json::from_str::<ClientMsg>(r#"{"type":"nope"}"#).is_err());
    }

    #[test]
    fn hole_cards_convert_with_cards_revealed() {
        // Build a real hand to get a genuine HoleCards event.
        use pokerladder_engine::Deck;
        use pokerladder_engine::holdem::{Hand, HandConfig};
        let (_, events) = Hand::new(
            HandConfig {
                button: Seat::Zero,
                stacks: [100, 100],
                small_blind: 1,
                big_blind: 2,
                auto_muck: [true; 2],
            },
            Deck::shuffled(&mut rand::rng()),
        )
        .unwrap();
        let hole: Vec<WireEvent> = events
            .iter()
            .filter(|e| matches!(e, Event::HoleCards { .. }))
            .map(|e| WireEvent::from(&MatchEvent::Hand(e.clone())))
            .collect();
        assert_eq!(hole.len(), 2);
        for e in hole {
            let WireEvent::HoleCards { cards, .. } = e else {
                panic!()
            };
            for c in cards {
                assert_eq!(c.len(), 2);
                assert!(c.parse::<Card>().is_ok());
            }
        }
    }
}
