//! Elo ratings, updated once per finished match. A placeholder until seasons bring a
//! rating with uncertainty (Glicko-2, Phase 4).

/// New players' rating.
pub const INITIAL: f64 = 1500.0;
/// Maximum change from one match.
pub const K: f64 = 32.0;

/// Probability that a player rated `a` beats one rated `b`.
pub fn expected(a: f64, b: f64) -> f64 {
    1.0 / (1.0 + 10f64.powf((b - a) / 400.0))
}

/// Ratings after `winner` (0 or 1) beat the other player, indexed like `before`.
pub fn update(before: [f64; 2], winner: usize) -> [f64; 2] {
    assert!(winner < 2);
    let gain = K * (1.0 - expected(before[winner], before[1 - winner]));
    let mut after = before;
    after[winner] += gain;
    after[1 - winner] -= gain;
    after
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_players_move_by_half_k() {
        assert_eq!(update([1500.0, 1500.0], 0), [1516.0, 1484.0]);
        assert_eq!(update([1500.0, 1500.0], 1), [1484.0, 1516.0]);
    }

    #[test]
    fn upsets_move_more_and_rating_is_conserved() {
        let fav = update([1800.0, 1400.0], 0);
        let upset = update([1800.0, 1400.0], 1);
        assert!(fav[0] - 1800.0 < 4.0);
        assert!(upset[1] - 1400.0 > 28.0);
        for after in [fav, upset] {
            assert!((after[0] + after[1] - 3200.0).abs() < 1e-9);
        }
        // 400 points = 10:1 odds.
        assert!((expected(1800.0, 1400.0) - 10.0 / 11.0).abs() < 1e-12);
    }
}
