//! Pokerladder game engine: cards, hand evaluation and (soon) the heads-up
//! No-Limit Hold'em state machine. Pure logic — no I/O, no networking.

pub mod card;
pub mod eval;
pub mod history;
pub mod holdem;
pub mod sng;

pub use card::{Card, Deck};
pub use eval::{Category, HandRank, evaluate};
pub use history::{HistoryError, HistoryMeta, Viewer, format_hand};
pub use holdem::{
    Action, Chips, ConfigError, Event, Hand, HandConfig, HandError, Legal, PublicState, Seat,
    Street, Visibility,
};
pub use sng::{Level, Match, MatchConfig, MatchError, MatchEvent, TimeControl};
