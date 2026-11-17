//! Hand evaluation.
//!
//! Every 5–7 card hand maps to a [`HandRank`]: one of the 7462 distinct 5-card
//! equivalence classes, numbered `1..=7462` so that a plain integer comparison decides
//! the winner (higher is better, equal is a split).
//!
//! Two evaluators produce identical results:
//! - [`direct`] — straightforward bitmask logic. Used to build and verify the table.
//! - [`twoplustwo`] — the Two Plus Two lookup table: 7 array reads per 7-card hand.
//!   This is what the game uses via [`evaluate`].

pub mod direct;
pub mod equity;
pub mod twoplustwo;

use std::fmt;
use std::num::NonZeroU16;
use std::sync::LazyLock;

use crate::card::{Card, Rank};

pub use twoplustwo::TwoPlusTwo;

/// Builds the shared Two Plus Two table (~124 MiB, a few seconds) if it isn't built yet.
///
/// Call this once at startup — from a blocking context such as
/// `tokio::task::spawn_blocking`, before accepting games — so that no game ever waits on
/// table generation inside an async task.
pub fn init() {
    twoplustwo::global();
}

/// Evaluates 5, 6 or 7 distinct cards with the shared Two Plus Two table.
///
/// Panics on duplicate cards or a wrong card count (both are engine bugs). Generates
/// the table on first use; see [`init`].
#[inline]
pub fn evaluate(cards: &[Card]) -> HandRank {
    twoplustwo::global().evaluate(cards)
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Category {
    HighCard = 0,
    Pair,
    TwoPair,
    Trips,
    Straight,
    Flush,
    FullHouse,
    Quads,
    StraightFlush,
}

impl Category {
    const ALL: [Category; 9] = [
        Category::HighCard,
        Category::Pair,
        Category::TwoPair,
        Category::Trips,
        Category::Straight,
        Category::Flush,
        Category::FullHouse,
        Category::Quads,
        Category::StraightFlush,
    ];

    /// Highest [`HandRank`] value in each category (cumulative class counts:
    /// 1277, 2860, 858, 858, 10, 1277, 156, 156, 10).
    const UPPER: [u16; 9] = [1277, 4137, 4995, 5853, 5863, 7140, 7296, 7452, 7462];
}

/// Strength of a hand: `1..=7462`, higher wins. The invalid value 0 is unrepresentable,
/// which also makes `Option<HandRank>` two bytes.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HandRank(NonZeroU16);

impl HandRank {
    pub const MAX: u16 = 7462;

    /// `Some` for values in `1..=7462`.
    pub const fn from_value(v: u16) -> Option<HandRank> {
        match NonZeroU16::new(v) {
            Some(nz) if v <= Self::MAX => Some(HandRank(nz)),
            _ => None,
        }
    }

    /// From a table entry the caller knows to be a rank. A 0 here means the table was
    /// fed duplicate cards that slipped past the entry-point checks — a bug.
    #[inline]
    pub(crate) fn from_table(v: u32) -> HandRank {
        match NonZeroU16::new(v as u16) {
            Some(nz) => HandRank(nz),
            None => invalid_sink(),
        }
    }

    #[inline]
    pub const fn value(self) -> u16 {
        self.0.get()
    }

    pub fn category(self) -> Category {
        let i = Category::UPPER.partition_point(|&upper| upper < self.value());
        Category::ALL[i]
    }

    /// Category and the five ranks that define the hand (unused slots are 0).
    fn parts(self) -> (Category, [Rank; 5]) {
        let raw = classes()[self.value() as usize - 1];
        let ks = [16, 12, 8, 4, 0].map(|shift| ((raw >> shift) & 0xF) as Rank);
        (self.category(), ks)
    }

    /// PokerStars-style description, e.g. `"a pair of Tens"`,
    /// `"a full house, Kings full of Fours"`, `"a straight, Five to Nine"`.
    pub fn describe(self) -> String {
        let (cat, k) = self.parts();
        let one = |r: Rank| RANK_NAMES[r as usize];
        let many = |r: Rank| RANK_PLURALS[r as usize];
        let low_end = |top: Rank| if top == 3 { 12 } else { top - 4 };
        match cat {
            Category::HighCard => format!("high card {}", one(k[0])),
            Category::Pair => format!("a pair of {}", many(k[0])),
            Category::TwoPair => format!("two pair, {} and {}", many(k[0]), many(k[1])),
            Category::Trips => format!("three of a kind, {}", many(k[0])),
            Category::Straight => format!("a straight, {} to {}", one(low_end(k[0])), one(k[0])),
            Category::Flush => format!("a flush, {} high", one(k[0])),
            Category::FullHouse => format!("a full house, {} full of {}", many(k[0]), many(k[1])),
            Category::Quads => format!("four of a kind, {}", many(k[0])),
            Category::StraightFlush if k[0] == 12 => "a Royal Flush".to_owned(),
            Category::StraightFlush => {
                format!("a straight flush, {} to {}", one(low_end(k[0])), one(k[0]))
            }
        }
    }
}

#[cold]
#[inline(never)]
fn invalid_sink() -> ! {
    panic!("evaluator reached the invalid sink (duplicate cards)")
}

impl fmt::Display for HandRank {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.describe())
    }
}

const RANK_NAMES: [&str; 13] = [
    "Deuce", "Three", "Four", "Five", "Six", "Seven", "Eight", "Nine", "Ten", "Jack", "Queen",
    "King", "Ace",
];
const RANK_PLURALS: [&str; 13] = [
    "Deuces", "Threes", "Fours", "Fives", "Sixes", "Sevens", "Eights", "Nines", "Tens", "Jacks",
    "Queens", "Kings", "Aces",
];

/// All 7462 distinct raw scores (see [`direct::raw_score`]) in ascending order.
/// `HandRank(n)` is the class at index `n - 1`.
fn classes() -> &'static [u32] {
    static CLASSES: LazyLock<Vec<u32>> = LazyLock::new(direct::enumerate_classes);
    &CLASSES
}

/// Maps a raw score to its dense rank. Panics if `raw` is not a real hand class.
#[inline]
fn dense(raw: u32) -> HandRank {
    let i = classes()
        .binary_search(&raw)
        .expect("raw score is not a valid hand class");
    HandRank::from_table(i as u32 + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::parse_cards;

    fn rank(s: &str) -> HandRank {
        direct::evaluate(&parse_cards(s).unwrap())
    }

    #[test]
    fn there_are_7462_classes_with_known_category_sizes() {
        let cls = classes();
        assert_eq!(cls.len(), 7462);
        for (i, cat) in Category::ALL.iter().enumerate() {
            let lo = if i == 0 { 0 } else { Category::UPPER[i - 1] };
            let hi = Category::UPPER[i];
            for r in lo + 1..=hi {
                assert_eq!(
                    HandRank::from_value(r).unwrap().category(),
                    *cat,
                    "rank {r}"
                );
                assert_eq!(cls[r as usize - 1] >> 20, *cat as u32, "rank {r}");
            }
        }
    }

    #[test]
    fn extremes() {
        assert_eq!(rank("7c 5d 4h 3s 2c").value(), 1);
        assert_eq!(rank("As Ks Qs Js Ts").value(), HandRank::MAX);
    }

    #[test]
    fn every_rank_describes_without_panicking() {
        for v in 1..=HandRank::MAX {
            let r = HandRank::from_value(v).unwrap();
            assert!(!r.describe().is_empty());
        }
        assert!(HandRank::from_value(0).is_none());
        assert!(HandRank::from_value(HandRank::MAX + 1).is_none());
    }

    #[test]
    fn descriptions() {
        let cases = [
            ("As Kd 9h 7c 3s", "high card Ace"),
            ("Ts Td 9h 7c 3s", "a pair of Tens"),
            ("6s 6d 9h 9c 3s", "two pair, Nines and Sixes"),
            ("2s 2d 2h 9c 3s", "three of a kind, Deuces"),
            ("As 2d 3h 4c 5s", "a straight, Ace to Five"),
            ("9s Td Jh Qc Ks", "a straight, Nine to King"),
            ("As 9s 7s 4s 2s", "a flush, Ace high"),
            ("Ks Kd Kh 4c 4s", "a full house, Kings full of Fours"),
            ("7s 7d 7h 7c 3s", "four of a kind, Sevens"),
            ("5h 6h 7h 8h 9h", "a straight flush, Five to Nine"),
            ("Ah 2h 3h 4h 5h", "a straight flush, Ace to Five"),
            ("Ah Kh Qh Jh Th", "a Royal Flush"),
        ];
        for (cards, want) in cases {
            assert_eq!(rank(cards).describe(), want, "{cards}");
        }
    }
}
