//! Direct bitmask evaluator: the readable reference implementation.
//!
//! It computes a *raw score*: `category << 20` followed by the five deciding ranks,
//! 4 bits each, most significant first. Raw scores compare correctly as integers but
//! are sparse; [`super::dense`] maps them onto the 7462 dense [`HandRank`]s.

use super::{Category, HandRank, dense};
use crate::card::{Card, Rank, assert_distinct};

/// Evaluates 5, 6 or 7 distinct cards. Panics on duplicates or a wrong card count.
pub fn evaluate(cards: &[Card]) -> HandRank {
    assert!((5..=7).contains(&cards.len()), "need 5 to 7 cards");
    assert_distinct(cards);
    let mut acc = Accumulator::default();
    for &c in cards {
        acc.add(c.rank(), Some(c.suit()));
    }
    dense(acc.raw_score())
}

/// Running counts for a hand. Cards may be added without a suit, meaning "this card
/// cannot contribute to a flush" — the Two Plus Two generator relies on that.
#[derive(Default, Clone, Copy)]
pub(crate) struct Accumulator {
    counts: [u8; 13],
    rank_mask: u16,
    suit_masks: [u16; 4],
}

impl Accumulator {
    #[inline]
    pub(crate) fn add(&mut self, rank: Rank, suit: Option<u8>) {
        self.counts[rank as usize] += 1;
        self.rank_mask |= 1 << rank;
        if let Some(s) = suit {
            self.suit_masks[s as usize] |= 1 << rank;
        }
    }

    pub(crate) fn raw_score(&self) -> u32 {
        raw_score(&self.counts, self.rank_mask, &self.suit_masks)
    }
}

fn pack(cat: Category, ranks: &[Rank]) -> u32 {
    debug_assert!(ranks.len() <= 5);
    let mut v = (cat as u32) << 20;
    for (i, &r) in ranks.iter().enumerate() {
        v |= (r as u32) << (16 - 4 * i);
    }
    v
}

/// Rank of the top card of the highest straight in `mask`, if any. The wheel
/// (A-2-3-4-5) returns 3, i.e. Five-high.
fn straight_top(mask: u16) -> Option<Rank> {
    // Bit 0 = ace-low, bits 1..=13 = deuce..ace.
    let m = ((mask as u32) << 1) | ((mask as u32 >> 12) & 1);
    (4..=13u32)
        .rev()
        .find(|&top| (m >> (top - 4)) & 0x1F == 0x1F)
        .map(|top| (top - 1) as Rank)
}

/// Up to `n` highest ranks set in `mask`, descending.
fn top_ranks(mut mask: u16, n: usize) -> ([Rank; 5], usize) {
    let mut out = [0; 5];
    let mut len = 0;
    while len < n && mask != 0 {
        let r = 15 - mask.leading_zeros() as Rank;
        out[len] = r;
        len += 1;
        mask &= !(1 << r);
    }
    (out, len)
}

fn with_kickers(cat: Category, made: &[Rank], kicker_mask: u16, n: usize) -> u32 {
    let (k, len) = top_ranks(kicker_mask, n);
    let mut all = [0; 5];
    all[..made.len()].copy_from_slice(made);
    all[made.len()..made.len() + len].copy_from_slice(&k[..len]);
    pack(cat, &all[..made.len() + len])
}

pub(crate) fn raw_score(counts: &[u8; 13], rank_mask: u16, suit_masks: &[u16; 4]) -> u32 {
    let flush = suit_masks.iter().copied().find(|m| m.count_ones() >= 5);

    if let Some(fm) = flush
        && let Some(top) = straight_top(fm)
    {
        return pack(Category::StraightFlush, &[top]);
    }

    // Ranks grouped by multiplicity, scanned from aces down.
    let mut quad = None;
    let mut trips: [Rank; 2] = [0; 2];
    let mut n_trips = 0;
    let mut pairs: [Rank; 3] = [0; 3];
    let mut n_pairs = 0;
    for r in (0..13u8).rev() {
        match counts[r as usize] {
            4 => quad = quad.or(Some(r)),
            3 => {
                trips[n_trips] = r;
                n_trips += 1;
            }
            2 => {
                pairs[n_pairs] = r;
                n_pairs += 1;
            }
            _ => {}
        }
    }
    let without = |ranks: &[Rank]| ranks.iter().fold(rank_mask, |m, &r| m & !(1 << r));

    if let Some(q) = quad {
        return with_kickers(Category::Quads, &[q], without(&[q]), 1);
    }
    if n_trips >= 1 && (n_trips >= 2 || n_pairs >= 1) {
        // With two sets of trips the lower one plays as the pair.
        let pair = if n_trips >= 2 {
            trips[1].max(pairs[0])
        } else {
            pairs[0]
        };
        return pack(Category::FullHouse, &[trips[0], pair]);
    }
    if let Some(fm) = flush {
        let (k, _) = top_ranks(fm, 5);
        return pack(Category::Flush, &k);
    }
    if let Some(top) = straight_top(rank_mask) {
        return pack(Category::Straight, &[top]);
    }
    if n_trips == 1 {
        return with_kickers(Category::Trips, &[trips[0]], without(&[trips[0]]), 2);
    }
    if n_pairs >= 2 {
        let (hi, lo) = (pairs[0], pairs[1]);
        return with_kickers(Category::TwoPair, &[hi, lo], without(&[hi, lo]), 1);
    }
    if n_pairs == 1 {
        return with_kickers(Category::Pair, &[pairs[0]], without(&[pairs[0]]), 3);
    }
    with_kickers(Category::HighCard, &[], rank_mask, 5)
}

/// Every distinct raw score a 5-card hand can have, sorted ascending (7462 of them).
/// Enumerates rank multisets rather than all 2.6M hands: non-flush hands are
/// determined by ranks alone, flushes by any 5 distinct ranks in one suit.
pub(crate) fn enumerate_classes() -> Vec<u32> {
    let mut out = Vec::with_capacity(8000);
    let mut ranks = [0u8; 5];
    fn rec(out: &mut Vec<u32>, ranks: &mut [Rank; 5], depth: usize, min: Rank) {
        if depth == 5 {
            let mut counts = [0u8; 13];
            let mut mask = 0u16;
            for &r in ranks.iter() {
                counts[r as usize] += 1;
                mask |= 1 << r;
            }
            if counts.iter().any(|&c| c > 4) {
                return;
            }
            out.push(raw_score(&counts, mask, &[0; 4]));
            if mask.count_ones() == 5 {
                out.push(raw_score(&counts, mask, &[mask, 0, 0, 0]));
            }
            return;
        }
        for r in min..13 {
            ranks[depth] = r;
            rec(out, ranks, depth + 1, r);
        }
    }
    rec(&mut out, &mut ranks, 0, 0);
    out.sort_unstable();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::parse_cards;

    fn ev(s: &str) -> HandRank {
        evaluate(&parse_cards(s).unwrap())
    }

    /// Each pair: left hand strictly beats right hand.
    #[test]
    fn orderings() {
        let wins = [
            // The legacy evaluator missed this straight (run broken by the King).
            ("Kd 9s 8h 7c 6d 5s 2c", "Ad Ac Ah 7c 6d 5s 2c"),
            // Wheel beats trips; six-high straight beats the wheel.
            ("As 2d 3h 4c 5s Kd Kc", "Ks Kd Kh 4c 9s"),
            ("2d 3h 4c 5s 6d", "As 2d 3h 4c 5s"),
            // Flush kickers matter.
            ("As Js 9s 7s 3s", "As Js 9s 7s 2s"),
            ("2h 3h 4h 5h 6h", "Ah Kh Qh Jh 9h"),
            ("Ah 2h 3h 4h 5h", "Kd Kc Kh Ks 2d"),
            // Two pair: third pair can't play, but its rank can be the kicker.
            ("Ks Kd 9h 9c 8s 8d 2c", "Ks Kd 9h 9c 7s 7d 2c"),
            ("Ks Kd 9h 9c 8s 8d 2c", "Ks Kd 9h 9c 7s 6d 2c"),
            // Full house from two sets of trips uses the lower set as the pair.
            ("Ks Kd Kh 9c 9s 9d 2c", "Ks Kd Kh 8c 8s 8d Ac"),
            // Quads kicker.
            ("7s 7d 7h 7c As", "7s 7d 7h 7c Ks"),
            ("Ts Td 9h 7c 3s 2d 4c", "Ts Td 8h 7c 3s 2d 4c"),
        ];
        for (a, b) in wins {
            assert!(ev(a) > ev(b), "{a} ({}) should beat {b} ({})", ev(a), ev(b));
        }
    }

    #[test]
    fn splits() {
        // Board plays: both players hold irrelevant cards.
        assert_eq!(ev("As Ks Qs Js Ts 2c 3d"), ev("As Ks Qs Js Ts 4c 5d"));
        assert_eq!(ev("Ah Kd Qc Js 9h 2c 3d"), ev("Ah Kd Qc Js 9h 2s 3h"));
    }
}
