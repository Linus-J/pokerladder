//! Property tests: random hands played with random legal actions.

use pokerladder_engine::holdem::RaiseBounds;
use pokerladder_engine::{Action, Chips, Deck, Event, Hand, HandConfig, HandError, Seat};
use proptest::prelude::*;
use rand::SeedableRng;
use rand::rngs::StdRng;

fn snapshot(h: &Hand) -> (Option<Seat>, [Chips; 2], [Chips; 2], Chips, usize) {
    (
        h.to_act(),
        h.stacks(),
        h.street_bets(),
        h.pot(),
        h.board().len(),
    )
}

fn config() -> impl Strategy<Value = HandConfig> {
    (
        1..=20_000u64,
        1..=20_000u64,
        1..=200u64,
        1..=200u64,
        any::<bool>(),
    )
        .prop_map(|(a, b, sb, extra, button_zero)| HandConfig {
            button: if button_zero { Seat::Zero } else { Seat::One },
            stacks: [a, b],
            small_blind: sb,
            big_blind: sb + extra,
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(4000))]

    #[test]
    fn random_hands_follow_the_rules(
        config in config(),
        seed in any::<u64>(),
        choices in prop::collection::vec((any::<u8>(), any::<u64>()), 1..64),
    ) {
        let total: Chips = config.stacks.iter().sum();
        let deck = Deck::shuffled(&mut StdRng::seed_from_u64(seed));
        let (mut h, mut events) = Hand::new(config, deck).unwrap();
        prop_assert_eq!(h.stacks().iter().sum::<Chips>() + h.pot(), total);

        let button = config.button;
        // Independent model of the min-raise rule: last full bet/raise on this street.
        let mut last_full = config.big_blind;
        // Who must make the first decision of the current street.
        let mut expect_first = Some(button);
        let mut step = 0usize;
        while let Some(legal) = h.legal() {
            prop_assert!(step < 100_000, "hand did not terminate");
            let seat = legal.seat;
            if let Some(first) = expect_first.take() {
                prop_assert_eq!(seat, first, "wrong first actor on {:?}", h.street());
            }
            let (pick, amount) = choices[step % choices.len()];
            step += 1;

            // Legal-set sanity.
            if let Some(c) = legal.call {
                prop_assert!(c > 0 && c <= h.stacks()[seat.index()]);
            }
            if let Some(RaiseBounds { min_to, max_to }) = legal.raise {
                let s = seat.index();
                let max_bet = h.street_bets()[0].max(h.street_bets()[1]);
                prop_assert!(min_to <= max_to);
                prop_assert!(max_to <= h.street_bets()[s] + h.stacks()[s]);
                prop_assert!(min_to > max_bet);
                prop_assert_eq!(min_to, (max_bet + last_full).min(max_to));
            }
            prop_assert!(legal.can_check != legal.can_fold);

            // An illegal action must be rejected without touching the state.
            let before = snapshot(&h);
            let illegal = match pick % 3 {
                0 => h.act(seat.other(), Action::Call).map(|_| ()),
                1 if !legal.can_check => h.act(seat, Action::Check).map(|_| ()),
                _ => match legal.raise {
                    Some(b) => h.act(seat, Action::RaiseTo(b.max_to + 1)).map(|_| ()),
                    None => h.act(seat, Action::RaiseTo(Chips::MAX)).map(|_| ()),
                },
            };
            prop_assert!(illegal.is_err());
            prop_assert_eq!(snapshot(&h), before);

            let mut options = vec![];
            if legal.can_fold { options.push(Action::Fold); }
            if legal.can_check { options.push(Action::Check); }
            if legal.call.is_some() { options.push(Action::Call); }
            if let Some(b) = legal.raise {
                let span = b.max_to - b.min_to + 1;
                options.push(Action::RaiseTo(b.min_to + amount % span));
                options.push(Action::RaiseTo(b.max_to));
                options.push(Action::AllIn);
            }
            let action = options[pick as usize % options.len()];
            let new_events = h.act(seat, action).unwrap();
            prop_assert_eq!(h.stacks().iter().sum::<Chips>() + h.pot(), total);

            let mut new_street = false;
            for e in &new_events {
                match *e {
                    Event::Acted { action: Action::RaiseTo(to), facing, all_in, .. } => {
                        // Every raise is full, or leaves someone all-in: the raiser, or
                        // the opponent, who can now put in at most `to` in total.
                        let o = seat.other().index();
                        let full = to - facing >= last_full;
                        let opp_capped = h.street_bets()[o] + h.stacks()[o] == to;
                        prop_assert!(full || all_in || opp_capped, "illegal short raise");
                        if full {
                            last_full = to - facing;
                        }
                    }
                    Event::Acted { action: Action::AllIn, .. } => {
                        prop_assert!(false, "AllIn must be resolved in events");
                    }
                    Event::Board { .. } => {
                        last_full = config.big_blind;
                        new_street = true;
                    }
                    _ => {}
                }
            }
            if new_street && !h.is_complete() {
                expect_first = Some(button.other());
            }
            events.extend(new_events);
        }

        prop_assert!(h.is_complete());
        prop_assert_eq!(h.pot(), 0);
        prop_assert_eq!(h.stacks().iter().sum::<Chips>(), total);
        prop_assert_eq!(h.act(Seat::Zero, Action::Check), Err(HandError::HandComplete));

        // Event log is self-consistent: chips in = chips out.
        let mut put_in: Chips = 0;
        let mut paid_out: Chips = 0;
        let mut ended = None;
        for e in &events {
            match e {
                Event::BlindPosted { amount, .. } => put_in += amount,
                Event::Acted { added, .. } => put_in += added,
                Event::UncalledReturned { amount, .. } | Event::Won { amount, .. } => {
                    paid_out += amount
                }
                Event::Ended { stacks } => ended = Some(*stacks),
                _ => {}
            }
        }
        prop_assert_eq!(put_in, paid_out);
        prop_assert_eq!(ended, Some(h.stacks()));
        let ends_with_ended = matches!(events.last(), Some(Event::Ended { .. }));
        prop_assert!(ends_with_ended);
        let showdowns = events.iter().filter(|e| matches!(e, Event::Showdown { .. })).count();
        prop_assert!(showdowns == 0 || (showdowns == 2 && h.board().len() == 5));
        // Hands are tabled at most once, and only ahead of a showdown.
        let tabled = events.iter().filter(|e| matches!(e, Event::HandsTabled { .. })).count();
        prop_assert!(tabled <= 1 && (tabled == 0 || showdowns == 2));
    }
}
