//! Evaluator benchmarks: Two Plus Two table vs direct bitmask evaluator vs `rs_poker`.
//!
//! - `random_7`: independent random 7-card hands (cache-hostile for the 124 MiB table).
//! - `showdown`: what the server actually does — one board, two players.
//! - `enumerate_river`: all 990 two-card opponent holdings against a fixed hand and
//!   turn board, i.e. the inner loop of an equity calculator, where 2+2 reuses the
//!   board prefix.

use std::hint::black_box;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use pokerladder_engine::eval::direct;
use pokerladder_engine::eval::twoplustwo::{self, Partial};
use pokerladder_engine::{Card, Deck};
use rand::SeedableRng;
use rand::rngs::StdRng;
use rs_poker::core::{Rankable, Suit, Value};

const N: usize = 4096;

fn random_hands() -> Vec<[Card; 7]> {
    let mut rng = StdRng::seed_from_u64(42);
    (0..N)
        .map(|_| {
            let mut d = Deck::shuffled(&mut rng);
            std::array::from_fn(|_| d.deal())
        })
        .collect()
}

fn to_rs(c: Card) -> rs_poker::core::Card {
    let value = Value::from_u8(c.rank());
    let suit = [Suit::Club, Suit::Diamond, Suit::Heart, Suit::Spade][c.suit() as usize];
    rs_poker::core::Card::new(value, suit)
}

fn bench_random(c: &mut Criterion) {
    let t = twoplustwo::global();
    let hands = random_hands();
    let rs_hands: Vec<Vec<rs_poker::core::Card>> = hands
        .iter()
        .map(|h| h.iter().map(|&c| to_rs(c)).collect())
        .collect();

    let mut g = c.benchmark_group("random_7");
    g.throughput(Throughput::Elements(N as u64));
    g.bench_function("two_plus_two", |b| {
        b.iter(|| {
            hands
                .iter()
                .map(|h| t.eval7(black_box(h)).value() as u32)
                .sum::<u32>()
        })
    });
    g.bench_function("two_plus_two_trusted", |b| {
        b.iter(|| {
            hands
                .iter()
                .map(|h| t.eval7_trusted(black_box(h)).value() as u32)
                .sum::<u32>()
        })
    });
    g.bench_function("direct", |b| {
        b.iter(|| {
            hands
                .iter()
                .map(|h| direct::evaluate(black_box(h)).value() as u32)
                .sum::<u32>()
        })
    });
    g.bench_function("rs_poker", |b| {
        b.iter(|| {
            rs_hands
                .iter()
                .map(|h| black_box(h.as_slice()).rank())
                .max()
        })
    });
    g.finish();
}

fn bench_showdown(c: &mut Criterion) {
    let t = twoplustwo::global();
    let mut rng = StdRng::seed_from_u64(7);
    let deals: Vec<([Card; 5], [Card; 2], [Card; 2])> = (0..N)
        .map(|_| {
            let mut d = Deck::shuffled(&mut rng);
            let board = std::array::from_fn(|_| d.deal());
            let a = [d.deal(), d.deal()];
            let b = [d.deal(), d.deal()];
            (board, a, b)
        })
        .collect();

    let mut g = c.benchmark_group("showdown");
    g.throughput(Throughput::Elements(N as u64));
    g.bench_function("two_plus_two_shared_board", |b| {
        b.iter(|| {
            deals
                .iter()
                .map(|(board, a, h)| {
                    let p = board.iter().fold(Partial::EMPTY, |p, &c| t.add(p, c));
                    let ra = t.finish_with(t.add(p, a[0]), a[1]);
                    let rb = t.finish_with(t.add(p, h[0]), h[1]);
                    ra.cmp(&rb) as i32
                })
                .sum::<i32>()
        })
    });
    g.finish();
}

fn bench_enumerate(c: &mut Criterion) {
    let t = twoplustwo::global();
    let board: [Card; 4] = ["2c", "7d", "Th", "Ks"].map(|s| s.parse().unwrap());
    let hero: [Card; 2] = ["Ah", "Qh"].map(|s| s.parse().unwrap());
    let used: Vec<Card> = board.iter().chain(&hero).copied().collect();
    let live: Vec<Card> = Card::all().filter(|c| !used.contains(c)).collect();

    let mut g = c.benchmark_group("enumerate_river");
    // Every river card × every opponent holding from the remaining cards.
    let pairs = live.len() * (live.len() - 1) * (live.len() - 2) / 2;
    g.throughput(Throughput::Elements(2 * pairs as u64));
    g.bench_function("two_plus_two_incremental", |b| {
        b.iter(|| {
            let pb = board.iter().fold(Partial::EMPTY, |p, &c| t.add(p, c));
            let ph = t.add(t.add(pb, hero[0]), hero[1]);
            let mut wins = 0u32;
            for &river in &live {
                let hero_rank = t.finish_with(ph, river);
                let pr = t.add(pb, river);
                for (i, &x) in live.iter().enumerate() {
                    if x == river {
                        continue;
                    }
                    let px = t.add(pr, x);
                    for &y in &live[i + 1..] {
                        if y != river && hero_rank > t.finish_with(px, y) {
                            wins += 1;
                        }
                    }
                }
            }
            black_box(wins)
        })
    });
    g.bench_function("two_plus_two_incremental_trusted", |b| {
        b.iter(|| {
            let pb = board
                .iter()
                .fold(Partial::EMPTY, |p, &c| t.add_trusted(p, c));
            let ph = t.add_trusted(t.add_trusted(pb, hero[0]), hero[1]);
            let mut wins = 0u32;
            for &river in &live {
                let hero_rank = t.finish_with_trusted(ph, river);
                let pr = t.add_trusted(pb, river);
                for (i, &x) in live.iter().enumerate() {
                    if x == river {
                        continue;
                    }
                    let px = t.add_trusted(pr, x);
                    for &y in &live[i + 1..] {
                        if y != river && hero_rank > t.finish_with_trusted(px, y) {
                            wins += 1;
                        }
                    }
                }
            }
            black_box(wins)
        })
    });
    g.bench_function("direct", |b| {
        b.iter(|| {
            let mut wins = 0u32;
            for &river in &live {
                let mut h = [
                    board[0], board[1], board[2], board[3], river, hero[0], hero[1],
                ];
                let hero_rank = direct::evaluate(&h);
                for (i, &x) in live.iter().enumerate() {
                    if x == river {
                        continue;
                    }
                    for &y in &live[i + 1..] {
                        if y != river {
                            h[5] = x;
                            h[6] = y;
                            if hero_rank > direct::evaluate(&h) {
                                wins += 1;
                            }
                        }
                    }
                }
            }
            black_box(wins)
        })
    });
    g.finish();
}

criterion_group!(benches, bench_random, bench_showdown, bench_enumerate);
criterion_main!(benches);
