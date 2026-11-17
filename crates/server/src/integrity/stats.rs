//! Small statistics helpers for the integrity report.

use super::Count;

/// `P(X >= k)` for `X ~ Binomial(n, p)`, computed exactly in log space (stable for
/// thousands of trials and tiny tails).
pub fn binomial_upper_tail(k: u32, n: u32, p: f64) -> f64 {
    assert!(k <= n, "k = {k} > n = {n}");
    let p = p.clamp(0.0, 1.0);
    if k == 0 {
        return 1.0;
    }
    if p == 0.0 {
        return 0.0;
    }
    if p == 1.0 {
        return 1.0;
    }
    let (ln_p, ln_q) = (p.ln(), (1.0 - p).ln());
    // ln C(n, i) built up incrementally from ln C(n, 0) = 0.
    let mut ln_choose = 0.0;
    let mut terms = Vec::with_capacity((n - k + 1) as usize);
    for i in 0..=n {
        if i > 0 {
            ln_choose += ((n - i + 1) as f64).ln() - (i as f64).ln();
        }
        if i >= k {
            terms.push(ln_choose + i as f64 * ln_p + (n - i) as f64 * ln_q);
        }
    }
    let max = terms.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let sum: f64 = terms.iter().map(|t| (t - max).exp()).sum();
    (max + sum.ln()).exp().min(1.0)
}

/// A population rate that is never exactly 0 or 1 (Jeffreys estimate): with few or
/// no events, a plain rate of 0 would make any event look impossible (p = 0).
pub fn prior_rate(c: Count) -> f64 {
    (c.k as f64 + 0.5) / (c.n as f64 + 1.0)
}

/// What a player's rate is believed to be, from their play against others plus
/// `weight` pseudo-observations at `prior`: a Beta(alpha, beta) distribution. Its
/// mean is the smoothed rate; its spread says how little data stands behind it, so a
/// baseline from 30 hands isn't treated as if it were exact.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Baseline {
    pub alpha: f64,
    pub beta: f64,
}

impl Baseline {
    pub fn new(others: Count, prior: f64, weight: f64) -> Baseline {
        Baseline {
            alpha: others.k as f64 + weight * prior,
            beta: (others.n - others.k) as f64 + weight * (1.0 - prior),
        }
    }

    pub fn mean(self) -> f64 {
        self.alpha / (self.alpha + self.beta)
    }

    /// `P(X >= k)` for `X` out of `n` at a rate drawn from this baseline
    /// (beta-binomial).
    pub fn upper_tail(self, k: u32, n: u32) -> f64 {
        beta_binomial_upper_tail(k, n, self.alpha, self.beta)
    }
}

/// `P(X >= k)` for `X ~ BetaBinomial(n, a, b)`, in log space. `a`, `b` > 0.
pub fn beta_binomial_upper_tail(k: u32, n: u32, a: f64, b: f64) -> f64 {
    assert!(k <= n, "k = {k} > n = {n}");
    assert!(
        a > 0.0 && b > 0.0,
        "beta parameters must be positive: {a}, {b}"
    );
    if k == 0 {
        return 1.0;
    }
    let ln_beta = |x: f64, y: f64| ln_gamma(x) + ln_gamma(y) - ln_gamma(x + y);
    let base = ln_beta(a, b);
    let mut ln_choose = 0.0;
    let mut terms = Vec::with_capacity((n - k + 1) as usize);
    for i in 0..=n {
        if i > 0 {
            ln_choose += ((n - i + 1) as f64).ln() - (i as f64).ln();
        }
        if i >= k {
            let (i, m) = (i as f64, (n - i) as f64);
            terms.push(ln_choose + ln_beta(i + a, m + b) - base);
        }
    }
    let max = terms.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let sum: f64 = terms.iter().map(|t| (t - max).exp()).sum();
    (max + sum.ln()).exp().min(1.0)
}

/// ln Γ(x) for x > 0 (Lanczos, g = 7, 9 terms; ~15 significant digits).
pub fn ln_gamma(x: f64) -> f64 {
    const G: f64 = 7.0;
    const C: [f64; 9] = [
        0.999_999_999_999_809_9,
        676.520_368_121_885_1,
        -1_259.139_216_722_402_8,
        771.323_428_777_653_1,
        -176.615_029_162_140_6,
        12.507_343_278_686_905,
        -0.138_571_095_265_720_12,
        9.984_369_578_019_572e-6,
        1.505_632_735_149_311_6e-7,
    ];
    assert!(x > 0.0, "ln_gamma({x})");
    if x < 0.5 {
        // Reflection: Γ(x) Γ(1 - x) = π / sin(πx).
        let pi = std::f64::consts::PI;
        return (pi / (pi * x).sin()).ln() - ln_gamma(1.0 - x);
    }
    let x = x - 1.0;
    let mut sum = C[0];
    for (i, c) in C.iter().enumerate().skip(1) {
        sum += c / (x + i as f64);
    }
    let t = x + G + 0.5;
    0.5 * (2.0 * std::f64::consts::PI).ln() + (x + 0.5) * t.ln() - t + sum.ln()
}

/// Elo-style expected score of a player rated `me` against one rated `them`. (The
/// stored ratings don't include the deviations Glicko-2 would weight this by.)
pub fn expected_score(me: f64, them: f64) -> f64 {
    1.0 / (1.0 + 10f64.powf((them - me) / 400.0))
}

#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct Results {
    pub matches: u32,
    pub wins: u32,
    /// Sum of expected scores.
    pub expected: f64,
    /// `(wins - expected) / sd`; 0 when there's nothing to compare.
    pub z: f64,
}

/// Wins against expectation over `(expected score, won)` games.
pub fn results(games: &[(f64, bool)]) -> Results {
    let matches = games.len() as u32;
    let wins = games.iter().filter(|g| g.1).count() as u32;
    let expected: f64 = games.iter().map(|g| g.0).sum();
    let var: f64 = games.iter().map(|g| g.0 * (1.0 - g.0)).sum();
    let z = if var > 0.0 {
        (wins as f64 - expected) / var.sqrt()
    } else {
        0.0
    };
    Results {
        matches,
        wins,
        expected,
        z,
    }
}

pub fn fmt_p(p: f64) -> String {
    if p >= 0.001 {
        format!("{p:.3}")
    } else if p == 0.0 {
        // Underflowed: smaller than f64 can hold.
        "<1e-300".into()
    } else {
        format!("{p:.0e}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Direct sum with exact binomial coefficients, for small n.
    fn naive(k: u32, n: u32, p: f64) -> f64 {
        let choose = |n: u32, i: u32| -> f64 {
            (0..i).fold(1.0, |acc, j| acc * (n - j) as f64 / (j + 1) as f64)
        };
        (k..=n)
            .map(|i| choose(n, i) * p.powi(i as i32) * (1.0 - p).powi((n - i) as i32))
            .sum()
    }

    #[test]
    fn binomial_tail_matches_direct_sum() {
        for n in [1, 5, 20, 60] {
            for k in 0..=n {
                for p in [0.01, 0.1, 0.5, 0.9] {
                    let (a, b) = (binomial_upper_tail(k, n, p), naive(k, n, p));
                    assert!(
                        (a - b).abs() <= 1e-12 + 1e-9 * b,
                        "k={k} n={n} p={p}: {a} vs {b}"
                    );
                }
            }
        }
        assert_eq!(binomial_upper_tail(0, 10, 0.0), 1.0);
        assert_eq!(binomial_upper_tail(1, 10, 0.0), 0.0);
        // Tiny tails stay positive (no catastrophic cancellation from 1 - P(X < k)).
        let t = binomial_upper_tail(200, 1000, 0.05);
        assert!(t > 0.0 && t < 1e-40, "{t}");
        // Tails beyond f64's range underflow to 0 rather than misbehaving.
        assert_eq!(binomial_upper_tail(900, 1000, 0.05), 0.0);
    }

    #[test]
    fn ln_gamma_known_values() {
        let close = |a: f64, b: f64| (a - b).abs() < 1e-12 * b.abs().max(1.0);
        assert!(close(ln_gamma(1.0), 0.0));
        assert!(close(ln_gamma(2.0), 0.0));
        assert!(close(ln_gamma(5.0), 24f64.ln()));
        assert!(close(ln_gamma(0.5), std::f64::consts::PI.sqrt().ln()));
        assert!(close(ln_gamma(0.1), 9.513_507_698_668_732f64.ln()));
        // ln 170! (Γ(171)), against a direct sum of logs.
        let direct: f64 = (1..=170).map(|i| (i as f64).ln()).sum();
        assert!(close(ln_gamma(171.0), direct), "{}", ln_gamma(171.0));
    }

    #[test]
    fn beta_binomial_tail() {
        // Beta(1, 1) is uniform, so every count 0..=n is equally likely.
        for n in [1, 7, 30] {
            for k in 0..=n {
                let t = beta_binomial_upper_tail(k, n, 1.0, 1.0);
                let want = (n - k + 1) as f64 / (n + 1) as f64;
                assert!((t - want).abs() < 1e-9, "k={k} n={n}: {t} vs {want}");
            }
        }
        // A very concentrated Beta behaves like the binomial at its mean.
        for (k, n, p) in [(3, 20, 0.05), (10, 60, 0.1), (1, 5, 0.5)] {
            let w = 1e8;
            let t = beta_binomial_upper_tail(k, n, w * p, w * (1.0 - p));
            let b = binomial_upper_tail(k, n, p);
            assert!((t - b).abs() < 1e-5 * b.max(1e-12), "{t} vs {b}");
        }
        // Less data behind a baseline means a larger tail for the same mean.
        let thin = Baseline::new(Count { k: 0, n: 0 }, 0.02, 10.0);
        let thick = Baseline::new(Count { k: 20, n: 1000 }, 0.02, 10.0);
        assert!((thin.mean() - thick.mean()).abs() < 1e-12);
        assert!(thin.upper_tail(3, 10) > 10.0 * thick.upper_tail(3, 10));
    }

    #[test]
    fn results_z() {
        let r = results(&[(0.5, false); 9]);
        assert_eq!((r.matches, r.wins), (9, 0));
        assert!((r.z - -3.0).abs() < 1e-12);
        assert_eq!(results(&[]).z, 0.0);
        assert!((expected_score(1500.0, 1500.0) - 0.5).abs() < 1e-12);
        assert!((expected_score(1900.0, 1500.0) - 10.0 / 11.0).abs() < 1e-12);
    }
}
