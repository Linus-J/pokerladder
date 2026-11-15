//! The Two Plus Two lookup-table evaluator.
//!
//! The evaluator is a finite-state machine over "cards seen so far", flattened into a
//! single `u32` array. Every state owns a block of 53 slots:
//!
//! - slot `0`: the hand's [`HandRank`] if evaluation stops here (5- and 6-card states);
//! - slot `1 + card.index()`: the block offset of the state reached by adding that card,
//!   or — from a 6-card state — the final 7-card [`HandRank`] itself.
//!
//! Block 0 is all zeroes and acts as a sink for transitions that can't happen (a card
//! already held). It keeps every read in bounds, but it does **not** detect duplicates
//! in general: once a suit is canonicalised away (below), re-adding one of its cards
//! looks like a new card of the same rank. Duplicates are therefore rejected at every
//! public entry point instead.
//!
//! **State canonicalisation.** States are multisets of cards with one reduction: once a
//! suit can no longer reach five cards within seven, its cards are stored rank-only.
//! This merges states that can never differ in outcome, giving 612,978 states —
//! 32,487,834 entries (124 MiB), the same size as the original 2+2 table.

use std::sync::LazyLock;

use rustc_hash::FxHashMap;

use super::HandRank;
use super::dense;
use super::direct::Accumulator;
use crate::card::{Card, CardSet, assert_distinct, duplicate_card};

const BLOCK: usize = 53;
const ROOT: u32 = BLOCK as u32;
const DEAD_SUIT: u8 = 4;

/// The shared table, generated on first access (see [`super::init`]).
pub fn global() -> &'static TwoPlusTwo {
    static TABLE: LazyLock<TwoPlusTwo> = LazyLock::new(TwoPlusTwo::generate);
    &TABLE
}

pub struct TwoPlusTwo {
    table: Box<[u32]>,
}

/// A partially evaluated hand: the table offset after 0–6 cards.
///
/// Walking a shared prefix once (e.g. the board) and branching from it is where this
/// evaluator is fastest.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Partial {
    offset: u32,
    cards: u8,
    seen: CardSet,
}

impl Partial {
    pub const EMPTY: Partial = Partial {
        offset: ROOT,
        cards: 0,
        seen: CardSet::EMPTY,
    };

    pub fn len(self) -> usize {
        self.cards as usize
    }

    pub fn is_empty(self) -> bool {
        self.cards == 0
    }
}

impl TwoPlusTwo {
    /// Number of `u32` entries in the table.
    pub fn len(&self) -> usize {
        self.table.len()
    }

    pub fn is_empty(&self) -> bool {
        self.table.is_empty()
    }

    /// Evaluates exactly seven distinct cards: seven dependent reads.
    /// Panics on duplicate cards.
    #[inline]
    pub fn eval7(&self, cards: &[Card; 7]) -> HandRank {
        assert_distinct(cards);
        self.eval7_trusted(cards)
    }

    /// [`eval7`](Self::eval7) without the duplicate check, for hot loops whose cards
    /// are distinct by construction. Duplicates are caught in debug builds; in release
    /// they give an unspecified rank (or a panic) but never undefined behaviour.
    #[inline]
    pub fn eval7_trusted(&self, cards: &[Card; 7]) -> HandRank {
        debug_assert!(
            CardSet::from_cards(cards).is_some(),
            "duplicate in {cards:?}"
        );
        let t = &*self.table;
        let mut p = ROOT;
        for c in cards {
            debug_assert!(p as usize + BLOCK <= t.len());
            // SAFETY: `table` is private and only `generate` builds it. There, the entry
            // for a transition out of a d-card state (d < 6) is either the offset of a
            // (d+1)-card state's block, `id * 53` with `id < blocks`, or 0 (the all-zero
            // sink block, which maps every card back to 0). So before each of the seven
            // reads `p` is a block offset `<= len - 53`, and `c.index() < 52` (by
            // construction of `Card`) keeps `p + 1 + c.index() <= len - 1`. Only 6-card
            // blocks hold ranks, and the seventh read's result is never used as an
            // index. This holds for any input, duplicates included. If the table is ever
            // loaded from disk, these invariants must be validated first.
            p = unsafe { *t.get_unchecked(p as usize + 1 + c.index() as usize) };
        }
        HandRank::from_table(p)
    }

    /// Evaluates 5, 6 or 7 distinct cards. Panics on duplicates or a wrong card count.
    #[inline]
    pub fn evaluate(&self, cards: &[Card]) -> HandRank {
        match cards.len() {
            7 => self.eval7(cards.try_into().unwrap()),
            5 | 6 => {
                let p = cards.iter().fold(Partial::EMPTY, |p, &c| self.add(p, c));
                self.finish(p)
            }
            n => panic!("need 5 to 7 cards, got {n}"),
        }
    }

    /// Adds a card to a partial hand of at most 5 cards. Panics on a duplicate card.
    #[inline]
    pub fn add(&self, p: Partial, c: Card) -> Partial {
        if p.seen.contains(c) {
            duplicate_card(&[c]);
        }
        self.add_trusted(p, c)
    }

    /// [`add`](Self::add) without the duplicate check (debug builds still check).
    #[inline]
    pub fn add_trusted(&self, mut p: Partial, c: Card) -> Partial {
        assert!(p.cards < 6, "use finish_with for the 7th card");
        let fresh = p.seen.insert(c);
        debug_assert!(fresh, "duplicate card {c}");
        p.offset = self.table[p.offset as usize + 1 + c.index() as usize];
        p.cards += 1;
        p
    }

    /// Rank of a 5- or 6-card partial hand.
    #[inline]
    pub fn finish(&self, p: Partial) -> HandRank {
        assert!(p.cards == 5 || p.cards == 6, "finish needs 5 or 6 cards");
        HandRank::from_table(self.table[p.offset as usize])
    }

    /// Rank of a 6-card partial hand plus one more card. Panics on a duplicate card.
    #[inline]
    pub fn finish_with(&self, p: Partial, c: Card) -> HandRank {
        if p.seen.contains(c) {
            duplicate_card(&[c]);
        }
        self.finish_with_trusted(p, c)
    }

    /// [`finish_with`](Self::finish_with) without the duplicate check (debug builds
    /// still check).
    #[inline]
    pub fn finish_with_trusted(&self, p: Partial, c: Card) -> HandRank {
        assert!(p.cards == 6, "finish_with needs a 6-card partial hand");
        debug_assert!(!p.seen.contains(c), "duplicate card {c}");
        HandRank::from_table(self.table[p.offset as usize + 1 + c.index() as usize])
    }

    /// Builds the table by breadth-first search from the empty hand. Private: use the
    /// shared [`global`] table rather than allocating another 124 MiB.
    fn generate() -> Self {
        const ENTRIES: usize = 32_487_834;
        let mut ids: FxHashMap<Key, u32> = FxHashMap::default();
        ids.reserve(ENTRIES / BLOCK);
        let mut keys: Vec<Key> = Vec::with_capacity(ENTRIES / BLOCK);
        keys.extend([Key::INVALID, Key::EMPTY]);
        ids.insert(Key::EMPTY, 1);
        let mut table: Vec<u32> = Vec::with_capacity(ENTRIES);
        table.resize(2 * BLOCK, 0);

        let mut i = 1;
        while i < keys.len() {
            let key = keys[i];
            let n = key.len();
            let base = i * BLOCK;
            if n >= 5 {
                table[base] = key.rank().value() as u32;
            }
            for c in Card::all() {
                let Some(child) = key.with(c) else { continue };
                let slot = base + 1 + c.index() as usize;
                table[slot] = if n == 6 {
                    child.rank().value() as u32
                } else {
                    let id = *ids.entry(child).or_insert_with(|| {
                        keys.push(child);
                        table.resize(table.len() + BLOCK, 0);
                        (keys.len() - 1) as u32
                    });
                    id * BLOCK as u32
                };
            }
            i += 1;
        }
        debug_assert_eq!(table.len(), ENTRIES);
        TwoPlusTwo {
            table: table.into_boxed_slice(),
        }
    }
}

/// A canonical state: up to 7 card codes packed one per byte, sorted descending,
/// zero-padded. Code = `rank * 5 + suit + 1`, where suit `4` means "suit irrelevant".
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
struct Key(u64);

impl Key {
    /// Placeholder for block 0; never looked up.
    const INVALID: Key = Key(u64::MAX);
    const EMPTY: Key = Key(0);

    fn codes(self) -> ([u8; 7], usize) {
        let mut out = [0u8; 7];
        let mut n = 0;
        for byte in self.0.to_be_bytes() {
            if byte != 0 {
                out[n] = byte;
                n += 1;
            }
        }
        (out, n)
    }

    fn len(self) -> usize {
        self.0.to_be_bytes().iter().filter(|&&b| b != 0).count()
    }

    fn split(code: u8) -> (u8, u8) {
        ((code - 1) / 5, (code - 1) % 5)
    }

    fn code(rank: u8, suit: u8) -> u8 {
        rank * 5 + suit + 1
    }

    /// The state after adding `card`, or `None` if that card is impossible here
    /// (already held, or a fifth card of one rank).
    fn with(self, card: Card) -> Option<Key> {
        let (mut codes, n) = self.codes();
        let (rank, suit) = (card.rank(), card.suit());
        let mut same_rank = 0;
        for &code in &codes[..n] {
            let (r, s) = Self::split(code);
            if r == rank {
                if s == suit {
                    return None;
                }
                same_rank += 1;
            }
        }
        if same_rank == 4 {
            return None;
        }
        codes[n] = Self::code(rank, suit);
        let n = n + 1;
        let remaining = 7 - n;

        // A dead suit stays dead: adding a card never raises (count + remaining).
        let mut suit_counts = [0usize; 4];
        for &code in &codes[..n] {
            let (_, s) = Self::split(code);
            if s != DEAD_SUIT {
                suit_counts[s as usize] += 1;
            }
        }
        for code in &mut codes[..n] {
            let (r, s) = Self::split(*code);
            if s != DEAD_SUIT && suit_counts[s as usize] + remaining < 5 {
                *code = Self::code(r, DEAD_SUIT);
            }
        }

        codes[..n].sort_unstable_by(|a, b| b.cmp(a));
        let mut bytes = [0u8; 8];
        bytes[8 - n..].copy_from_slice(&codes[..n]);
        Some(Key(u64::from_be_bytes(bytes)))
    }

    fn rank(self) -> HandRank {
        let (codes, n) = self.codes();
        let mut acc = Accumulator::default();
        for &code in &codes[..n] {
            let (r, s) = Self::split(code);
            acc.add(r, (s != DEAD_SUIT).then_some(s));
        }
        dense(acc.raw_score())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dead_suits_are_stripped() {
        // After 3 clubs + 2 others, diamonds/hearts/spades (≤1 card each) can't flush.
        let mut k = Key::EMPTY;
        for s in ["2c", "5c", "9c", "Kd", "Ah"] {
            k = k.with(s.parse().unwrap()).unwrap();
        }
        let (codes, n) = k.codes();
        let dead = codes[..n]
            .iter()
            .filter(|&&c| Key::split(c).1 == DEAD_SUIT)
            .count();
        assert_eq!((n, dead), (5, 2));
    }

    fn key(cards: &str) -> Key {
        crate::card::parse_cards(cards)
            .unwrap()
            .into_iter()
            .fold(Key::EMPTY, |k, c| k.with(c).unwrap())
    }

    #[test]
    fn rejects_live_duplicates() {
        assert!(key("As").with("As".parse().unwrap()).is_none());
    }

    #[test]
    fn rejects_fifth_of_a_rank() {
        // With 4 cards held, single-card suits are dead, so all four sevens are stored
        // rank-only; a "fifth" seven must still be rejected.
        let k = key("7c 7d 7h 7s");
        let (codes, _) = k.codes();
        assert!(codes[..4].iter().all(|&c| Key::split(c).1 == DEAD_SUIT));
        assert!(k.with("7c".parse().unwrap()).is_none());
    }
}
