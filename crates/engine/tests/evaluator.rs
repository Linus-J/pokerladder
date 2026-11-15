//! Exhaustive and cross-library verification of the hand evaluators.

use std::time::Instant;

use pokerladder_engine::eval::twoplustwo::{self, Partial};
use pokerladder_engine::eval::{Category, HandRank, direct};
use pokerladder_engine::{Card, Deck};
use rand::SeedableRng;
use rand::rngs::StdRng;
use rs_poker::core::{Hand, Rankable};

fn cards(idx: &[u8]) -> Vec<Card> {
    idx.iter().map(|&i| Card::from_index(i)).collect()
}

fn category_counts(ranks: &[u64]) -> [u64; 9] {
    let mut out = [0u64; 9];
    for (r, &n) in ranks.iter().enumerate().skip(1) {
        out[HandRank::from_value(r as u16).unwrap().category() as usize] += n;
    }
    out
}

#[test]
fn table_size_and_build_time() {
    let start = Instant::now();
    let t = twoplustwo::global();
    eprintln!(
        "2+2 table: {} entries ({} states, {:.1} MiB), ready in {:.2?}",
        t.len(),
        t.len() / 53,
        (t.len() * 4) as f64 / (1024.0 * 1024.0),
        start.elapsed()
    );
}

/// Every 7-card hand, walked incrementally through the table. Category totals are the
/// well-known exact counts; 4824 distinct classes are reachable with 7 cards.
#[test]
fn all_7_card_hands_have_known_category_counts() {
    let t = twoplustwo::global();
    let mut per_rank = vec![0u64; HandRank::MAX as usize + 1];
    let add = |p, i: u8| t.add(p, Card::from_index(i));
    for a in 0..52 {
        let pa = add(Partial::EMPTY, a);
        for b in a + 1..52 {
            let pb = add(pa, b);
            for c in b + 1..52 {
                let pc = add(pb, c);
                for d in c + 1..52 {
                    let pd = add(pc, d);
                    for e in d + 1..52 {
                        let pe = add(pd, e);
                        for f in e + 1..52 {
                            let pf = add(pe, f);
                            for g in f + 1..52 {
                                let r = t.finish_with(pf, Card::from_index(g));
                                per_rank[r.value() as usize] += 1;
                            }
                        }
                    }
                }
            }
        }
    }
    assert_eq!(per_rank[0], 0, "some hands hit the invalid sink");
    assert_eq!(
        category_counts(&per_rank),
        [
            23_294_460, 58_627_800, 31_433_400, 6_461_620, 6_180_020, 4_047_644, 3_473_184,
            224_848, 41_584
        ]
    );
    assert_eq!(per_rank.iter().filter(|&&n| n > 0).count(), 4824);
}

/// Table and direct evaluator agree on every 5- and 6-card hand, and 5-card category
/// totals match the textbook counts.
#[test]
fn table_matches_direct_on_all_5_and_6_card_hands() {
    let t = twoplustwo::global();
    let mut per_rank5 = vec![0u64; HandRank::MAX as usize + 1];
    let mut idx = [0u8; 6];
    for a in 0..52 {
        idx[0] = a;
        for b in a + 1..52 {
            idx[1] = b;
            for c in b + 1..52 {
                idx[2] = c;
                for d in c + 1..52 {
                    idx[3] = d;
                    for e in d + 1..52 {
                        idx[4] = e;
                        let h5 = cards(&idx[..5]);
                        let r5 = t.evaluate(&h5);
                        assert_eq!(r5, direct::evaluate(&h5), "{h5:?}");
                        per_rank5[r5.value() as usize] += 1;
                        for f in e + 1..52 {
                            idx[5] = f;
                            let h6 = cards(&idx);
                            assert_eq!(t.evaluate(&h6), direct::evaluate(&h6), "{h6:?}");
                        }
                    }
                }
            }
        }
    }
    assert_eq!(
        category_counts(&per_rank5),
        [
            1_302_540, 1_098_240, 123_552, 54_912, 10_200, 5_108, 3_744, 624, 40
        ]
    );
    assert_eq!(per_rank5.iter().filter(|&&n| n > 0).count(), 7462);
}

/// Card order must not matter, and the table must agree with the direct evaluator.
#[test]
fn table_matches_direct_on_random_7_card_hands_in_any_order() {
    let t = twoplustwo::global();
    let mut rng = StdRng::seed_from_u64(0x2b2);
    for _ in 0..2_000_000 {
        let mut deck = Deck::shuffled(&mut rng);
        let h: [Card; 7] = std::array::from_fn(|_| deck.deal());
        let mut sorted = h;
        sorted.sort();
        let r = t.eval7(&h);
        assert_eq!(r, t.eval7(&sorted), "{h:?}");
        assert_eq!(r, direct::evaluate(&h), "{h:?}");
    }
}

/// Full 133M-hand table-vs-direct comparison. Slow-ish; run with `--ignored`.
#[test]
#[ignore]
fn table_matches_direct_on_all_7_card_hands() {
    let t = twoplustwo::global();
    let mut idx = [0u8; 7];
    fn rec(t: &twoplustwo::TwoPlusTwo, idx: &mut [u8; 7], depth: usize, min: u8) {
        if depth == 7 {
            let h: [Card; 7] = idx.map(Card::from_index);
            assert_eq!(t.eval7(&h), direct::evaluate(&h), "{h:?}");
            return;
        }
        for i in min..52 {
            idx[depth] = i;
            rec(t, idx, depth + 1, i + 1);
        }
    }
    rec(t, &mut idx, 0, 0);
}

/// Independent oracle: `rs_poker` must order random pairs of hands the same way.
#[test]
fn agrees_with_rs_poker_on_random_showdowns() {
    let mut rng = StdRng::seed_from_u64(0xc0ffee);
    let theirs = |h: &[Card]| {
        let s: String = h.iter().map(|c| c.to_string()).collect();
        Hand::new_from_str(&s).unwrap().rank()
    };
    for _ in 0..300_000 {
        // A real heads-up showdown: shared board, two hole-card pairs.
        let mut deck = Deck::shuffled(&mut rng);
        let board: Vec<Card> = (0..5).map(|_| deck.deal()).collect();
        let a: Vec<Card> = board
            .iter()
            .copied()
            .chain([deck.deal(), deck.deal()])
            .collect();
        let b: Vec<Card> = board
            .iter()
            .copied()
            .chain([deck.deal(), deck.deal()])
            .collect();
        let ours = pokerladder_engine::evaluate(&a).cmp(&pokerladder_engine::evaluate(&b));
        assert_eq!(ours, theirs(&a).cmp(&theirs(&b)), "{a:?} vs {b:?}");
    }
}

#[test]
fn category_accessor_matches_named_examples() {
    let ev =
        |s: &str| pokerladder_engine::evaluate(&pokerladder_engine::card::parse_cards(s).unwrap());
    assert_eq!(
        ev("Ah Kh Qh Jh Th 2c 3d").category(),
        Category::StraightFlush
    );
    assert_eq!(ev("Ah 2c 3d 4s 5h Kd Kc").category(), Category::Straight);
    assert_eq!(ev("Kd 9s 8h 7c 6d 5s 2c").category(), Category::Straight);
}

/// Duplicates must never produce a plausible-looking rank. The first two cases slip
/// past the table's own sink (the repeated card's suit has been canonicalised away), so
/// they exercise the entry-point checks.
#[test]
fn duplicates_panic_at_every_entry_point() {
    use pokerladder_engine::card::parse_cards;
    use std::panic::catch_unwind;
    let t = twoplustwo::global();
    for hand in [
        "As Kc Qc Jc As",
        "2c 3d 4h 5s 2c",
        "As As Kd Qh Jc",
        "As Kc Qc Jc Th 9d As",
    ] {
        let cards = parse_cards(hand).unwrap();
        assert!(
            catch_unwind(|| t.evaluate(&cards)).is_err(),
            "table: {hand}"
        );
        assert!(
            catch_unwind(|| direct::evaluate(&cards)).is_err(),
            "direct: {hand}"
        );
        assert!(
            catch_unwind(|| pokerladder_engine::evaluate(&cards)).is_err(),
            "{hand}"
        );
    }
    let seven = parse_cards("As Kc Qc Jc Th 9d As").unwrap();
    let seven: [Card; 7] = seven.try_into().unwrap();
    assert!(catch_unwind(|| t.eval7(&seven)).is_err());
    let six = parse_cards("As Kc Qc Jc Th 9d").unwrap();
    let p = six.iter().fold(Partial::EMPTY, |p, &c| t.add(p, c));
    assert!(catch_unwind(|| t.finish_with(p, "Kc".parse().unwrap())).is_err());
}
