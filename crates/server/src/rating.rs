//! Glicko-2 ratings (Glickman, "Example of the Glicko-2 system", 2013).
//!
//! A rating is a value, a deviation (how unsure we are of it, like a standard error)
//! and a volatility (how erratic the player's results are). Each match is its own
//! rating period, applied as soon as the match ends; before it, the deviation grows
//! for the idle time since the player's last rated match ([`IDLE_PERIODS_PER_DAY`]),
//! so a rating left alone becomes less certain.
//!
//! Glickman suggests 10 to 15 games per rating period; with one, the volatility
//! reacts to single results more than intended. In practice it barely moves at
//! these settings (TAU 0.5), and rating every match at once is what players expect.

/// Converts between the displayed scale (1500 ± 350) and Glicko-2's internal one.
const SCALE: f64 = 173.7178;
/// New players' rating.
pub const INITIAL_RATING: f64 = 1500.0;
/// New players' (and the maximum) deviation.
pub const MAX_RD: f64 = 350.0;
/// New players' volatility.
pub const INITIAL_VOLATILITY: f64 = 0.06;
/// Constrains how fast volatility can change (Glickman suggests 0.3 to 1.2).
pub const TAU: f64 = 0.5;
/// Rated matches a player needs in a season to appear in its ranked standings (and so
/// to be eligible for its prizes).
pub const QUALIFYING_MATCHES: i32 = 20;
/// Rated matches the same two players may start in any 24 hours; later ones are
/// played unrated, so friends can't farm rating off each other.
pub const PAIR_DAILY_LIMIT: i64 = 3;

/// How many empty rating periods one idle day counts as. With one, a day off would
/// add no more uncertainty than a single match does, and a leader who stops playing
/// would lose only ~20 points of score in a month. At 6, a settled player (RD 60)
/// idle for 20 days drifts to RD ~135, about 150 points of score.
pub const IDLE_PERIODS_PER_DAY: f64 = 6.0;

/// Convergence tolerance for the volatility iteration.
const EPSILON: f64 = 1e-6;

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Glicko {
    pub rating: f64,
    pub rd: f64,
    pub volatility: f64,
}

impl Glicko {
    pub const NEW: Glicko = Glicko {
        rating: INITIAL_RATING,
        rd: MAX_RD,
        volatility: INITIAL_VOLATILITY,
    };

    /// The rating minus twice its deviation: a value the player is very probably at
    /// least worth. Season standings are ranked by it, so a lucky streak followed by
    /// inactivity doesn't hold a place (the deviation grows while idle).
    pub fn cautious(&self) -> f64 {
        self.rating - 2.0 * self.rd
    }

    /// The rating after `days` idle days (each [`IDLE_PERIODS_PER_DAY`] empty rating
    /// periods): the deviation grows, up to [`MAX_RD`]. Negative or non-finite `days`
    /// count as none.
    pub fn idle(self, days: f64) -> Glicko {
        if !(days.is_finite() && days > 0.0) {
            return self;
        }
        let phi = self.rd / SCALE;
        let periods = days * IDLE_PERIODS_PER_DAY;
        let phi = (phi * phi + periods * self.volatility * self.volatility).sqrt();
        Glicko {
            rd: (phi * SCALE).min(MAX_RD),
            ..self
        }
    }
}

/// One rating period for `player`, against each `(opponent, score)` with score 1 for a
/// win, 0 for a loss and 0.5 for a draw. Opponents' ratings are their values before
/// the period. With no games only the deviation grows (one idle day).
pub fn update(player: Glicko, games: &[(Glicko, f64)]) -> Glicko {
    if games.is_empty() {
        return player.idle(1.0);
    }
    let mu = (player.rating - INITIAL_RATING) / SCALE;
    let phi = player.rd / SCALE;
    let sigma = player.volatility;

    // Step 3 and 4: estimated variance `v` and improvement `delta`.
    let mut inv_v = 0.0;
    let mut sum = 0.0;
    for &(opp, score) in games {
        let mu_j = (opp.rating - INITIAL_RATING) / SCALE;
        let g = g(opp.rd / SCALE);
        let e = 1.0 / (1.0 + (-g * (mu - mu_j)).exp());
        inv_v += g * g * e * (1.0 - e);
        sum += g * (score - e);
    }
    let v = 1.0 / inv_v;
    let delta = v * sum;

    // Step 5: the new volatility (Illinois algorithm).
    let a = (sigma * sigma).ln();
    let f = |x: f64| {
        let ex = x.exp();
        let d = phi * phi + v + ex;
        ex * (delta * delta - phi * phi - v - ex) / (2.0 * d * d) - (x - a) / (TAU * TAU)
    };
    let mut big_a = a;
    let mut big_b = if delta * delta > phi * phi + v {
        (delta * delta - phi * phi - v).ln()
    } else {
        let mut k = 1.0;
        while f(a - k * TAU) < 0.0 {
            k += 1.0;
        }
        a - k * TAU
    };
    let mut f_a = f(big_a);
    let mut f_b = f(big_b);
    while (big_b - big_a).abs() > EPSILON {
        let c = big_a + (big_a - big_b) * f_a / (f_b - f_a);
        let f_c = f(c);
        if f_c * f_b <= 0.0 {
            big_a = big_b;
            f_a = f_b;
        } else {
            f_a /= 2.0;
        }
        big_b = c;
        f_b = f_c;
    }
    let sigma_new = (big_a / 2.0).exp();

    // Steps 6 to 8: new deviation and rating.
    let phi_star = (phi * phi + sigma_new * sigma_new).sqrt();
    let phi_new = 1.0 / (1.0 / (phi_star * phi_star) + 1.0 / v).sqrt();
    let mu_new = mu + phi_new * phi_new * sum;
    Glicko {
        rating: mu_new * SCALE + INITIAL_RATING,
        rd: (phi_new * SCALE).min(MAX_RD),
        volatility: sigma_new,
    }
}

fn g(phi: f64) -> f64 {
    1.0 / (1.0 + 3.0 * phi * phi / (std::f64::consts::PI * std::f64::consts::PI)).sqrt()
}

/// Both players' ratings after one match won by `winner` (0 or 1), each first aged by
/// its own idle days. Indexed like `before`.
pub fn match_result(before: [Glicko; 2], idle_days: [f64; 2], winner: usize) -> [Glicko; 2] {
    assert!(winner < 2);
    let aged = [before[0].idle(idle_days[0]), before[1].idle(idle_days[1])];
    [0, 1].map(|i| {
        let score = if i == winner { 1.0 } else { 0.0 };
        update(aged[i], &[(aged[1 - i], score)])
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    /// The worked example in Glickman's paper.
    #[test]
    fn glickman_example() {
        let player = Glicko {
            rating: 1500.0,
            rd: 200.0,
            volatility: 0.06,
        };
        let opp = |rating, rd| Glicko {
            rating,
            rd,
            volatility: 0.06,
        };
        let after = update(
            player,
            &[
                (opp(1400.0, 30.0), 1.0),
                (opp(1550.0, 100.0), 0.0),
                (opp(1700.0, 300.0), 0.0),
            ],
        );
        assert!(close(after.rating, 1464.06, 0.01), "{after:?}");
        assert!(close(after.rd, 151.52, 0.01), "{after:?}");
        assert!(close(after.volatility, 0.05999, 0.00001), "{after:?}");
    }

    #[test]
    fn equal_new_players() {
        let [w, l] = match_result([Glicko::NEW; 2], [0.0; 2], 0);
        // Symmetric, a big first step (both are very uncertain), less uncertain after.
        assert!(close(w.rating - 1500.0, 1500.0 - l.rating, 1e-9));
        assert!(w.rating > 1600.0 && w.rating < 1700.0, "{w:?}");
        assert!(w.rd < MAX_RD && close(w.rd, l.rd, 1e-9));
    }

    #[test]
    fn idleness_widens_the_deviation_up_to_the_maximum() {
        let g = Glicko {
            rating: 1800.0,
            rd: 60.0,
            volatility: 0.06,
        };
        assert_eq!(g.idle(0.0), g);
        assert_eq!(g.idle(-5.0), g);
        assert_eq!(g.idle(f64::NAN), g);
        let month = g.idle(30.0);
        assert!(month.rd > g.rd && month.rating == g.rating);
        assert!(month.cautious() < g.cautious());
        assert_eq!(g.idle(1e9).rd, MAX_RD);
        // Strong enough that sitting on a lead doesn't pay: 20 idle days cost a
        // settled player well over 100 points of score.
        assert!(
            g.cautious() - g.idle(20.0).cautious() > 100.0,
            "{:?}",
            g.idle(20.0)
        );
        // But a day or two off is cheap.
        assert!(g.cautious() - g.idle(1.0).cautious() < 20.0);
    }

    fn glicko() -> impl Strategy<Value = Glicko> {
        (800.0..2600.0f64, 30.0..350.0f64, 0.03..0.12f64).prop_map(|(rating, rd, volatility)| {
            Glicko {
                rating,
                rd,
                volatility,
            }
        })
    }

    proptest! {
        /// Winning never lowers a rating and losing never raises it; a game always
        /// makes the rating more certain than it was going in (after idle ageing);
        /// everything stays finite and in range.
        #[test]
        fn results_move_ratings_the_right_way(
            a in glicko(), b in glicko(), idle in (0.0..400.0f64, 0.0..400.0f64), winner in 0usize..2,
        ) {
            let idle = [idle.0, idle.1];
            let before = [a, b];
            let after = match_result(before, idle, winner);
            for i in 0..2 {
                let aged = before[i].idle(idle[i]);
                let x = after[i];
                prop_assert!(x.rating.is_finite() && x.rd.is_finite() && x.volatility.is_finite());
                prop_assert!(x.rd > 0.0 && x.rd <= MAX_RD);
                prop_assert!(x.volatility > 0.0 && x.volatility < 1.0);
                prop_assert!(x.rd <= aged.rd.hypot(x.volatility * SCALE) + 1e-9);
                if i == winner {
                    prop_assert!(x.rating >= aged.rating);
                } else {
                    prop_assert!(x.rating <= aged.rating);
                }
            }
        }

        /// An upset moves the winner more than an expected win does.
        #[test]
        fn upsets_are_worth_more(low in 1000.0..1500.0f64, gap in 100.0..800.0f64) {
            let weak = Glicko { rating: low, rd: 80.0, volatility: 0.06 };
            let strong = Glicko { rating: low + gap, rd: 80.0, volatility: 0.06 };
            let upset = match_result([weak, strong], [0.0; 2], 0)[0].rating - weak.rating;
            let expected = match_result([weak, strong], [0.0; 2], 1)[1].rating - strong.rating;
            prop_assert!(upset > expected);
        }
    }
}
