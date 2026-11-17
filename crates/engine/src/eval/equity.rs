//! Exact equity by enumeration, on the shared Two Plus Two table.
//!
//! - [`vs_hand`]: one hand against another known hand, over every runout of the board.
//! - [`vs_random`]: one hand against a uniformly random opponent holding (hand
//!   *strength*), over every opponent holding and every runout. Preflop this is about
//!   2.1 billion showdowns, so the 169 starting-hand classes are precomputed
//!   ([`PREFLOP_VS_RANDOM`], checked by an `--ignored` test that recomputes them).
//!
//! Both return exact win/tie counts, so results are reproducible bit for bit. Inputs
//! must be distinct cards (checked; panics otherwise) and the board must have 0, 3, 4
//! or 5 cards.

use super::HandRank;
use super::twoplustwo::{self, Partial, TwoPlusTwo};
use crate::card::{Card, CardSet, assert_distinct};

/// Outcomes counted over an enumeration, from the first hand's point of view.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Equity {
    pub wins: u64,
    pub ties: u64,
    pub total: u64,
}

impl Equity {
    /// Pot share: wins plus half of the ties, over all outcomes.
    pub fn share(self) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        (self.wins as f64 + self.ties as f64 / 2.0) / self.total as f64
    }

    #[inline]
    fn record(&mut self, mine: HandRank, theirs: HandRank) {
        self.total += 1;
        if mine > theirs {
            self.wins += 1;
        } else if mine == theirs {
            self.ties += 1;
        }
    }
}

fn check_board(board: &[Card]) {
    assert!(
        matches!(board.len(), 0 | 3 | 4 | 5),
        "board must have 0, 3, 4 or 5 cards, got {}",
        board.len()
    );
}

fn live_cards(used: &[Card]) -> Vec<Card> {
    let set = CardSet::from_cards(used).expect("checked distinct");
    Card::all().filter(|&c| !set.contains(c)).collect()
}

/// Exact equity of `a` against the known hand `b` with `board` (0, 3, 4 or 5 cards)
/// already dealt: every remaining runout, equally likely. Preflop that is 1,712,304
/// runouts. Panics on duplicate cards or a bad board size.
pub fn vs_hand(a: [Card; 2], b: [Card; 2], board: &[Card]) -> Equity {
    check_board(board);
    let mut used = vec![a[0], a[1], b[0], b[1]];
    used.extend_from_slice(board);
    assert_distinct(&used);

    let t = twoplustwo::global();
    let mut eq = Equity::default();
    if board.len() == 5 {
        let ra = t.evaluate(&[a[0], a[1], board[0], board[1], board[2], board[3], board[4]]);
        let rb = t.evaluate(&[b[0], b[1], board[0], board[1], board[2], board[3], board[4]]);
        eq.record(ra, rb);
        return eq;
    }
    let pa = board
        .iter()
        .fold(t.add(t.add(Partial::EMPTY, a[0]), a[1]), |p, &c| {
            t.add(p, c)
        });
    let pb = board
        .iter()
        .fold(t.add(t.add(Partial::EMPTY, b[0]), b[1]), |p, &c| {
            t.add(p, c)
        });
    let live = live_cards(&used);
    runouts_vs_hand(t, pa, pb, &live, 5 - board.len(), &mut eq);
    eq
}

/// Deals `need >= 1` more board cards from `live` (in increasing order, so each set
/// once) to both partial hands.
fn runouts_vs_hand(
    t: &TwoPlusTwo,
    pa: Partial,
    pb: Partial,
    live: &[Card],
    need: usize,
    eq: &mut Equity,
) {
    if need == 1 {
        for &c in live {
            eq.record(t.finish_with_trusted(pa, c), t.finish_with_trusted(pb, c));
        }
        return;
    }
    for (i, &c) in live[..live.len() + 1 - need].iter().enumerate() {
        runouts_vs_hand(
            t,
            t.add_trusted(pa, c),
            t.add_trusted(pb, c),
            &live[i + 1..],
            need - 1,
            eq,
        );
    }
}

/// Exact equity of `hole` against a uniformly random opponent holding, with `board`
/// (0, 3, 4 or 5 cards) dealt: every opponent holding from the unseen cards, and for
/// each, every runout. Preflop reads the precomputed [`PREFLOP_VS_RANDOM`]. Panics on
/// duplicate cards or a bad board size.
///
/// On the flop this is about a million showdowns (milliseconds); see the
/// `equity` benchmarks.
pub fn vs_random(hole: [Card; 2], board: &[Card]) -> Equity {
    check_board(board);
    let mut used = vec![hole[0], hole[1]];
    used.extend_from_slice(board);
    assert_distinct(&used);
    if board.is_empty() {
        let [wins, ties] = PREFLOP_VS_RANDOM[preflop_class(hole)];
        return Equity {
            wins: wins as u64,
            ties: ties as u64,
            total: PREFLOP_TOTAL,
        };
    }
    enumerate_vs_random(hole, board)
}

/// The enumeration behind [`vs_random`], for any board size (preflop takes seconds).
/// Runouts are the outer loop, so each board and the hero's rank on it are computed
/// once and shared by all opponent holdings.
fn enumerate_vs_random(hole: [Card; 2], board: &[Card]) -> Equity {
    let t = twoplustwo::global();
    let mut used = vec![hole[0], hole[1]];
    used.extend_from_slice(board);
    let live = live_cards(&used);
    let mut eq = Equity::default();
    let pboard = board.iter().fold(Partial::EMPTY, |p, &c| t.add(p, c));
    if board.len() == 5 {
        let mine = t.eval7(&[
            hole[0], hole[1], board[0], board[1], board[2], board[3], board[4],
        ]);
        opponents(t, pboard, mine, &live, CardSet::EMPTY, &mut eq);
        return eq;
    }
    let phero = board
        .iter()
        .fold(t.add(t.add(Partial::EMPTY, hole[0]), hole[1]), |p, &c| {
            t.add(p, c)
        });
    runouts_vs_random(
        t,
        phero,
        pboard,
        &live,
        &live,
        CardSet::EMPTY,
        5 - board.len(),
        &mut eq,
    );
    eq
}

#[allow(clippy::too_many_arguments)]
fn runouts_vs_random(
    t: &TwoPlusTwo,
    phero: Partial,
    pboard: Partial,
    all_live: &[Card],
    live: &[Card],
    dealt: CardSet,
    need: usize,
    eq: &mut Equity,
) {
    for (i, &c) in live[..live.len() + 1 - need].iter().enumerate() {
        let mut dealt = dealt;
        dealt.insert(c);
        let pboard = t.add_trusted(pboard, c);
        if need == 1 {
            let mine = t.finish_with_trusted(phero, c);
            opponents(t, pboard, mine, all_live, dealt, eq);
        } else {
            runouts_vs_random(
                t,
                t.add_trusted(phero, c),
                pboard,
                all_live,
                &live[i + 1..],
                dealt,
                need - 1,
                eq,
            );
        }
    }
}

/// Every opponent holding from `live` minus `dealt`, against a complete 5-card board.
fn opponents(
    t: &TwoPlusTwo,
    pboard: Partial,
    mine: HandRank,
    live: &[Card],
    dealt: CardSet,
    eq: &mut Equity,
) {
    debug_assert_eq!(pboard.len(), 5);
    for (i, &x) in live.iter().enumerate() {
        if dealt.contains(x) {
            continue;
        }
        let px = t.add_trusted(pboard, x);
        for &y in &live[i + 1..] {
            if !dealt.contains(y) {
                eq.record(mine, t.finish_with_trusted(px, y));
            }
        }
    }
}

/// Index of a starting hand's class in a 13×13 grid: `hi * 13 + lo` for suited
/// hands, `lo * 13 + hi` for offsuit, `r * 13 + r` for pairs (ranks 0 = deuce … 12 =
/// ace).
pub fn preflop_class(hole: [Card; 2]) -> usize {
    let (hi, lo) = if hole[0].rank() >= hole[1].rank() {
        (hole[0].rank(), hole[1].rank())
    } else {
        (hole[1].rank(), hole[0].rank())
    };
    let (hi, lo) = (hi as usize, lo as usize);
    if hole[0].suit() == hole[1].suit() {
        hi * 13 + lo
    } else {
        lo * 13 + hi
    }
}

/// Outcomes per class preflop: 1225 opponent holdings × 1,712,304 runouts.
pub const PREFLOP_TOTAL: u64 = 1225 * 1_712_304;

/// `[wins, ties]` against a random holding for each [`preflop_class`], out of
/// [`PREFLOP_TOTAL`]. Generated by the `preflop_table_is_exact` test (`--ignored`;
/// `UPDATE_PREFLOP=1` prints a fresh table).
pub const PREFLOP_VS_RANDOM: [[u32; 2]; 169] = include!("preflop_vs_random.in");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::parse_cards;

    fn cards<const N: usize>(s: &str) -> [Card; N] {
        parse_cards(s).unwrap().try_into().unwrap()
    }

    fn board(s: &str) -> Vec<Card> {
        parse_cards(s).unwrap()
    }

    /// Independent check: deal every runout explicitly and evaluate both 7-card hands
    /// with the direct evaluator.
    fn brute_vs_hand(a: [Card; 2], b: [Card; 2], board: &[Card]) -> Equity {
        let mut used = vec![a[0], a[1], b[0], b[1]];
        used.extend_from_slice(board);
        let live = live_cards(&used);
        let mut eq = Equity::default();
        let need = 5 - board.len();
        let mut idx: Vec<usize> = (0..need).collect();
        loop {
            let mut full = board.to_vec();
            full.extend(idx.iter().map(|&i| live[i]));
            let mut ha = full.clone();
            ha.extend(a);
            let mut hb = full;
            hb.extend(b);
            eq.record(
                super::super::direct::evaluate(&ha),
                super::super::direct::evaluate(&hb),
            );
            // Next combination in lexicographic order.
            let mut k = need;
            loop {
                if k == 0 {
                    return eq;
                }
                k -= 1;
                if idx[k] < live.len() - (need - k) {
                    idx[k] += 1;
                    for j in k + 1..need {
                        idx[j] = idx[j - 1] + 1;
                    }
                    break;
                }
            }
            if need == 0 {
                return eq;
            }
        }
    }

    #[test]
    fn vs_hand_matches_brute_force_on_flop_turn_and_river() {
        let a = cards("AhKh");
        let b = cards("7c7d");
        for b_str in ["2h9hQs", "2h9hQs3c", "2h9hQs3c7s"] {
            let bd = board(b_str);
            assert_eq!(vs_hand(a, b, &bd), brute_vs_hand(a, b, &bd), "{b_str}");
        }
    }

    #[test]
    fn vs_hand_preflop_matches_brute_force() {
        for (a, b) in [("AsAh", "KsKh"), ("AhKh", "QsQc"), ("5c4c", "AdAh")] {
            let (a, b) = (cards(a), cards(b));
            let eq = vs_hand(a, b, &[]);
            assert_eq!(eq.total, 1_712_304);
            assert_eq!(eq, brute_vs_hand(a, b, &[]));
        }
        // Published heads-up figure: AA vs KK is 81-83% depending on suits.
        let aa = vs_hand(cards("AsAh"), cards("KcKd"), &[]).share();
        assert!((0.81..0.83).contains(&aa), "{aa}");
        // Equity is symmetric.
        let (x, y) = (cards("9c8c"), cards("AdKs"));
        let p = vs_hand(x, y, &[]);
        let q = vs_hand(y, x, &[]);
        assert_eq!(p.ties, q.ties);
        assert_eq!(p.wins + q.wins + p.ties, p.total);
    }

    #[test]
    fn vs_hand_on_the_river_is_a_single_showdown() {
        let eq = vs_hand(cards("AhKh"), cards("7c7d"), &board("2h9hQs3c7s"));
        assert_eq!(
            eq,
            Equity {
                wins: 0,
                ties: 0,
                total: 1
            }
        );
    }

    /// Independent check of `vs_random` on the turn and river: enumerate opponent
    /// holdings and rivers with the direct evaluator.
    fn brute_vs_random(hole: [Card; 2], board: &[Card]) -> Equity {
        let mut used = vec![hole[0], hole[1]];
        used.extend_from_slice(board);
        let live = live_cards(&used);
        let mut eq = Equity::default();
        for i in 0..live.len() {
            for j in i + 1..live.len() {
                let opp = [live[i], live[j]];
                let e = brute_vs_hand(hole, opp, board);
                eq.wins += e.wins;
                eq.ties += e.ties;
                eq.total += e.total;
            }
        }
        eq
    }

    #[test]
    fn vs_random_matches_brute_force_on_turn_and_river() {
        let hole = cards("QdJd");
        for b_str in ["Td9s2d3c", "Td9s2d3cKh", "AcAdAhAs2c"] {
            let bd = board(b_str);
            assert_eq!(vs_random(hole, &bd), brute_vs_random(hole, &bd), "{b_str}");
        }
    }

    #[test]
    fn vs_random_on_the_flop_matches_brute_force() {
        let hole = cards("7c2d");
        let bd = board("AsKsQh");
        let eq = vs_random(hole, &bd);
        assert_eq!(eq.total, 1081 * 990);
        assert_eq!(eq, brute_vs_random(hole, &bd));
        let eq = vs_random(cards("AsKs"), &board("QsJsTs"));
        assert_eq!(eq.wins + eq.ties, eq.total, "royal flush never loses");
    }

    #[test]
    fn preflop_classes_cover_the_grid() {
        let mut seen = [0u32; 169];
        let all: Vec<Card> = Card::all().collect();
        for i in 0..52 {
            for j in i + 1..52 {
                seen[preflop_class([all[i], all[j]])] += 1;
            }
        }
        for (k, n) in seen.iter().enumerate() {
            let (r, c) = (k / 13, k % 13);
            let want = if r == c {
                6
            } else if r > c {
                4
            } else {
                12
            };
            assert_eq!(*n, want, "class {k}");
        }
        assert_eq!(preflop_class(cards("AsKs")), preflop_class(cards("KhAh")));
    }

    #[test]
    fn preflop_table_is_ordered_sensibly() {
        let share = |s: &str| vs_random(cards(s), &[]).share();
        // Well-known heads-up values vs a random hand: AA 85.2%, 72o 34.6%, 32o 32.3%.
        assert!((share("AsAh") - 0.852).abs() < 0.002, "{}", share("AsAh"));
        assert!((share("7s2h") - 0.346).abs() < 0.002, "{}", share("7s2h"));
        assert!((share("3s2h") - 0.323).abs() < 0.002, "{}", share("3s2h"));
        let worst = (0..169)
            .min_by(|&a, &b| {
                let s =
                    |k: usize| 2 * PREFLOP_VS_RANDOM[k][0] as u64 + PREFLOP_VS_RANDOM[k][1] as u64;
                s(a).cmp(&s(b))
            })
            .unwrap();
        assert_eq!(worst, preflop_class(cards("3s2h")));
        assert!(share("AsKs") > share("AsKh"));
        assert!(share("KsKh") > share("AsKs"));
        // AA is the best class; 32o the worst.
        let best = (0..169).max_by_key(|&k| PREFLOP_VS_RANDOM[k][0]).unwrap();
        assert_eq!(best, preflop_class(cards("AsAh")));
    }

    /// Recomputes all 169 classes (about 2 billion showdowns each; ~3.5 minutes on 16
    /// threads). `UPDATE_PREFLOP=1` prints the table to paste into
    /// `preflop_vs_random.in`.
    #[test]
    #[ignore]
    fn preflop_table_is_exact() {
        crate::eval::init();
        let reps: Vec<[Card; 2]> = (0..169)
            .map(|k| {
                let (r, c) = ((k / 13) as u8, (k % 13) as u8);
                if r == c {
                    [Card::new(r, 0), Card::new(r, 1)]
                } else if r > c {
                    [Card::new(r, 0), Card::new(c, 0)]
                } else {
                    [Card::new(c, 0), Card::new(r, 1)]
                }
            })
            .collect();
        for (k, h) in reps.iter().enumerate() {
            assert_eq!(preflop_class(*h), k);
        }
        let next = std::sync::atomic::AtomicUsize::new(0);
        let mut out = vec![[0u32; 2]; 169];
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
        let results: Vec<Vec<(usize, Equity)>> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..threads)
                .map(|_| {
                    s.spawn(|| {
                        let mut mine = Vec::new();
                        loop {
                            let k = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            if k >= 169 {
                                return mine;
                            }
                            mine.push((k, enumerate_vs_random(reps[k], &[])));
                        }
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        for (k, eq) in results.into_iter().flatten() {
            assert_eq!(eq.total, PREFLOP_TOTAL);
            out[k] = [eq.wins as u32, eq.ties as u32];
        }
        if std::env::var_os("UPDATE_PREFLOP").is_some() {
            let mut s = String::from("[\n");
            for row in &out {
                s.push_str(&format!("    [{}, {}],\n", row[0], row[1]));
            }
            s.push(']');
            std::fs::write(
                concat!(env!("CARGO_MANIFEST_DIR"), "/src/eval/preflop_vs_random.in"),
                s + "\n",
            )
            .unwrap();
        }
        assert_eq!(out.as_slice(), PREFLOP_VS_RANDOM.as_slice());
    }
}
