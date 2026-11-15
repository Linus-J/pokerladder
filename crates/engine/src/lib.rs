//! Pokerladder game engine: cards, hand evaluation and (soon) the heads-up
//! No-Limit Hold'em state machine. Pure logic — no I/O, no networking.

pub mod card;
pub mod eval;
pub mod holdem;

pub use card::{Card, Deck};
pub use eval::{Category, HandRank, evaluate};
pub use holdem::{
    Action, Chips, ConfigError, Event, Hand, HandConfig, HandError, Legal, PublicState, Seat,
    Street, Visibility,
};
