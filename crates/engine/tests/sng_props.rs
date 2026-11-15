//! Property tests: whole sit-and-go matches with random actions and timeouts.

use std::time::Duration;

use pokerladder_engine::{Action, Chips, Deck, Match, MatchConfig, MatchEvent, Seat};
use proptest::prelude::*;
use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    #[test]
    fn random_matches_end_with_one_winner(seed in any::<u64>(), first_zero in any::<bool>()) {
        let mut rng = StdRng::seed_from_u64(seed);
        let first = if first_zero { Seat::Zero } else { Seat::One };
        let mut m = Match::new(MatchConfig::standard(), first).unwrap();
        let mut overs = 0;
        let mut hands = 0;
        while m.winner().is_none() {
            prop_assert!(hands < 500, "match did not end");
            hands += 1;
            let deck = Deck::shuffled(&mut StdRng::seed_from_u64(rng.random()));
            let mut events = m.start_hand(deck).unwrap();
            while let Some(legal) = m.hand().and_then(|h| h.legal()) {
                let seat = legal.seat;
                let roll: u8 = rng.random_range(0..10);
                let ev = if roll == 0 {
                    m.timeout(seat).unwrap()
                } else {
                    let action = match (roll, legal.raise, legal.call) {
                        (1..=2, Some(_), _) => Action::AllIn,
                        (3..=4, Some(b), _) => Action::RaiseTo(b.min_to),
                        (_, _, Some(_)) => Action::Call,
                        _ => Action::Check,
                    };
                    let elapsed = Duration::from_secs(rng.random_range(0..30));
                    m.act(seat, action, elapsed).unwrap()
                };
                events.extend(ev);
            }
            overs += events.iter().filter(|e| matches!(e, MatchEvent::MatchOver { .. })).count();
            prop_assert_eq!(m.stacks().iter().sum::<Chips>(), 3000);
            prop_assert_eq!(m.hands_played(), hands);
        }
        prop_assert_eq!(overs, 1);
        let w = m.winner().unwrap();
        prop_assert_eq!(m.stacks(), if w == Seat::Zero { [3000, 0] } else { [0, 3000] });
    }
}
