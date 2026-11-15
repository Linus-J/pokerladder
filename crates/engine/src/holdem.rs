//! Heads-up No-Limit Hold'em: one hand as a state machine.
//!
//! The server owns a [`Hand`], feeds it player [`Action`]s, and forwards the returned
//! [`Event`]s to clients, filtered with [`Event::visibility`] ([`Event::HoleCards`] is
//! private). All rule enforcement lives here; nothing a client sends is trusted.
//!
//! Rules implemented:
//! - The button posts the small blind and acts first preflop; the big blind acts first
//!   on every later street. The big blind gets its option after a limp.
//! - A bet or raise must be at least the size of the last full bet/raise on the street
//!   (initially the big blind). An all-in for less is always allowed.
//! - Bets are capped at the effective stack (what the opponent can still match);
//!   [`Action::AllIn`] shoves exactly that. Any uncalled excess — e.g. a big blind posted
//!   against a shorter all-in stack — is returned before the pot is awarded. Heads-up
//!   there is therefore only ever one pot.
//! - Once betting is closed with a player all-in, both hands are turned face up
//!   ([`Event::HandsTabled`]) and the board is run out.
//! - Both hands are shown at showdown (no mucking — a deliberate anti-collusion choice
//!   for a ladder); the last aggressor on the final betting street shows first,
//!   otherwise the big blind.
//! - Split pots: heads-up the pot is always even at showdown (both contributions are
//!   equal once uncalled chips are returned); any odd chip would go to the big blind.
//!
//! **Secrecy.** The shuffled deck — and so the entire future board — is held inside the
//! `Hand` from the start; cards are only *revealed* street by street. `Hand` is
//! therefore neither `Debug` nor `Clone`, and must never be serialised, logged, or
//! exposed to clients. Use [`Hand::public_state`] for anything client-facing.

use std::fmt;

use crate::card::{Card, Deck};
use crate::eval::{self, HandRank};

pub type Chips = u64;

/// Upper bound on the chips in one hand (2^53, the largest integer a JavaScript client
/// can represent exactly). Keeps every internal sum far from overflow.
pub const MAX_TOTAL_CHIPS: Chips = 1 << 53;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Seat {
    Zero = 0,
    One = 1,
}

impl Seat {
    pub const BOTH: [Seat; 2] = [Seat::Zero, Seat::One];

    #[inline]
    pub const fn other(self) -> Seat {
        match self {
            Seat::Zero => Seat::One,
            Seat::One => Seat::Zero,
        }
    }

    #[inline]
    pub const fn index(self) -> usize {
        self as usize
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Street {
    Preflop,
    Flop,
    Turn,
    River,
}

impl Street {
    fn next(self) -> Street {
        match self {
            Street::Preflop => Street::Flop,
            Street::Flop => Street::Turn,
            Street::Turn | Street::River => Street::River,
        }
    }

    fn new_cards(self) -> usize {
        match self {
            Street::Preflop => 0,
            Street::Flop => 3,
            Street::Turn | Street::River => 1,
        }
    }
}

/// A player decision. Bets and raises are both `RaiseTo(total)`: the player's total
/// contribution on this street after the action. `AllIn` resolves to the largest legal
/// raise, or a call if raising isn't possible; events record the resolved action.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    Fold,
    Check,
    Call,
    RaiseTo(Chips),
    AllIn,
}

/// What the player to act may do.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Legal {
    pub seat: Seat,
    /// Folding is only offered when facing a bet.
    pub can_fold: bool,
    pub can_check: bool,
    /// Chips a call adds (already capped at the stack); `None` if there is nothing to call.
    pub call: Option<Chips>,
    /// Inclusive range for [`Action::RaiseTo`]; `None` if raising isn't possible.
    pub raise: Option<RaiseBounds>,
}

impl Legal {
    /// The action to take when a player times out or disconnects: check if possible,
    /// otherwise fold. Never puts chips in.
    pub fn passive_default(&self) -> Action {
        if self.can_check {
            Action::Check
        } else {
            Action::Fold
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct RaiseBounds {
    /// The minimum legal raise, or `max_to` if that is smaller.
    pub min_to: Chips,
    /// The largest raise: the effective stack (puts one player all-in).
    pub max_to: Chips,
}

/// A value that must not show up in logs: its `Debug` output is redacted.
#[derive(Copy, Clone, PartialEq, Eq)]
pub struct Hidden<T>(T);

impl<T> Hidden<T> {
    pub fn reveal(&self) -> &T {
        &self.0
    }
}

impl<T> fmt::Debug for Hidden<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<hidden>")
    }
}

/// Who may see an event.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Visibility {
    Public,
    Private(Seat),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    BlindPosted {
        seat: Seat,
        amount: Chips,
        big: bool,
        all_in: bool,
    },
    /// Private to `seat`; redacted in `Debug` output.
    HoleCards {
        seat: Seat,
        cards: Hidden<[Card; 2]>,
    },
    Acted {
        seat: Seat,
        action: Action,
        /// The highest street total before this action (0 = unopened; lets a hand
        /// history say "bets" rather than "raises").
        facing: Chips,
        /// Chips this action moved from the stack into the pot.
        added: Chips,
        /// The player's street total afterwards.
        street_total: Chips,
        all_in: bool,
    },
    /// Betting closed with a player all-in before the river: both hands go face up
    /// before the rest of the board is dealt. Indexed by seat.
    HandsTabled {
        cards: [[Card; 2]; 2],
    },
    /// Newly revealed board cards.
    Board {
        street: Street,
        cards: Vec<Card>,
    },
    UncalledReturned {
        seat: Seat,
        amount: Chips,
    },
    Showdown {
        seat: Seat,
        cards: [Card; 2],
        rank: HandRank,
    },
    Won {
        seat: Seat,
        amount: Chips,
    },
    Ended {
        stacks: [Chips; 2],
    },
}

impl Event {
    pub fn visibility(&self) -> Visibility {
        match self {
            Event::HoleCards { seat, .. } => Visibility::Private(*seat),
            _ => Visibility::Public,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ConfigError {
    /// A stack is zero.
    EmptyStack,
    /// Blinds must satisfy `0 < small < big`.
    BadBlinds,
    /// Total chips exceed [`MAX_TOTAL_CHIPS`].
    TooManyChips,
    /// Fewer than 9 cards left in the deck.
    ShortDeck,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum HandError {
    InvalidConfig(ConfigError),
    HandComplete,
    NotYourTurn { expected: Seat },
    CannotFold,
    CannotCheck,
    NothingToCall,
    RaiseNotAllowed,
    RaiseOutOfRange { min_to: Chips, max_to: Chips },
}

impl fmt::Display for HandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HandError::InvalidConfig(why) => write!(f, "invalid hand config: {why:?}"),
            HandError::HandComplete => f.write_str("the hand is over"),
            HandError::NotYourTurn { expected } => write!(f, "not your turn ({expected:?} to act)"),
            HandError::CannotFold => f.write_str("nothing to fold to; check instead"),
            HandError::CannotCheck => f.write_str("cannot check facing a bet"),
            HandError::NothingToCall => f.write_str("nothing to call"),
            HandError::RaiseNotAllowed => f.write_str("raising is not allowed here"),
            HandError::RaiseOutOfRange { min_to, max_to } => {
                write!(f, "raise must be to between {min_to} and {max_to}")
            }
        }
    }
}

impl std::error::Error for HandError {}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct HandConfig {
    pub button: Seat,
    pub stacks: [Chips; 2],
    pub small_blind: Chips,
    pub big_blind: Chips,
}

/// Everything about a hand that both players (and spectators) may see — e.g. to
/// rebuild a reconnecting client's table. Add the viewer's own hole cards separately.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublicState {
    pub button: Seat,
    pub street: Street,
    pub board: Vec<Card>,
    /// Chips behind.
    pub stacks: [Chips; 2],
    pub street_bets: [Chips; 2],
    /// Chips each player has put in this hand.
    pub committed: [Chips; 2],
    pub pot: Chips,
    pub legal: Option<Legal>,
    pub complete: bool,
}

/// One hand in progress. Deliberately neither `Debug` nor `Clone`: see the module docs.
pub struct Hand {
    button: Seat,
    big_blind: Chips,
    stacks: [Chips; 2],
    street_bet: [Chips; 2],
    committed: [Chips; 2],
    hole: [[Card; 2]; 2],
    board: Vec<Card>,
    deck: Deck,
    street: Street,
    to_act: Option<Seat>,
    acted: [bool; 2],
    /// Size of the last full bet or raise this street; the minimum raise increment.
    last_raise: Chips,
    /// Last player to bet or raise on the current street.
    aggressor: Option<Seat>,
    complete: bool,
}

impl Hand {
    /// Posts blinds, deals hole cards, and returns the hand with its opening events.
    pub fn new(config: HandConfig, mut deck: Deck) -> Result<(Hand, Vec<Event>), HandError> {
        let HandConfig {
            button,
            stacks,
            small_blind,
            big_blind,
        } = config;
        let invalid = |e| Err(HandError::InvalidConfig(e));
        if stacks.contains(&0) {
            return invalid(ConfigError::EmptyStack);
        }
        if small_blind == 0 || small_blind >= big_blind {
            return invalid(ConfigError::BadBlinds);
        }
        if stacks[0]
            .checked_add(stacks[1])
            .is_none_or(|t| t > MAX_TOTAL_CHIPS)
            || big_blind > MAX_TOTAL_CHIPS
        {
            return invalid(ConfigError::TooManyChips);
        }
        if deck.remaining() < 9 {
            return invalid(ConfigError::ShortDeck);
        }

        // Deal one at a time starting left of the button (the big blind).
        let bb = button.other();
        let dealt: [Card; 4] = std::array::from_fn(|_| deck.deal());
        let mut hole = [[dealt[0]; 2]; 2];
        hole[bb.index()] = [dealt[0], dealt[2]];
        hole[button.index()] = [dealt[1], dealt[3]];

        let mut hand = Hand {
            button,
            big_blind,
            stacks,
            street_bet: [0; 2],
            committed: [0; 2],
            hole,
            board: Vec::with_capacity(5),
            deck,
            street: Street::Preflop,
            to_act: None,
            acted: [false; 2],
            last_raise: big_blind,
            aggressor: None,
            complete: false,
        };
        let mut ev = Vec::new();
        for (seat, blind, big) in [(button, small_blind, false), (bb, big_blind, true)] {
            let amount = hand.commit(seat, blind);
            ev.push(Event::BlindPosted {
                seat,
                amount,
                big,
                all_in: hand.stacks[seat.index()] == 0,
            });
        }
        for seat in Seat::BOTH {
            ev.push(Event::HoleCards {
                seat,
                cards: Hidden(hand.hole[seat.index()]),
            });
        }
        hand.to_act = hand.next_to_act([button, bb]);
        if hand.to_act.is_none() {
            hand.close_street(&mut ev);
        }
        Ok((hand, ev))
    }

    pub fn button(&self) -> Seat {
        self.button
    }

    pub fn street(&self) -> Street {
        self.street
    }

    pub fn board(&self) -> &[Card] {
        &self.board
    }

    /// Chips behind (not yet in the pot).
    pub fn stacks(&self) -> [Chips; 2] {
        self.stacks
    }

    pub fn street_bets(&self) -> [Chips; 2] {
        self.street_bet
    }

    /// Chips each player has put in this hand (net of returned uncalled bets).
    pub fn committed(&self) -> [Chips; 2] {
        self.committed
    }

    /// Chips in the middle; 0 once the hand is over and the pot has been awarded.
    pub fn pot(&self) -> Chips {
        if self.complete {
            0
        } else {
            self.committed[0] + self.committed[1]
        }
    }

    pub fn hole_cards(&self, seat: Seat) -> [Card; 2] {
        self.hole[seat.index()]
    }

    pub fn is_complete(&self) -> bool {
        self.complete
    }

    pub fn to_act(&self) -> Option<Seat> {
        self.to_act
    }

    pub fn public_state(&self) -> PublicState {
        PublicState {
            button: self.button,
            street: self.street,
            board: self.board.clone(),
            stacks: self.stacks,
            street_bets: self.street_bet,
            committed: self.committed,
            pot: self.pot(),
            legal: self.legal(),
            complete: self.complete,
        }
    }

    fn max_bet(&self) -> Chips {
        self.street_bet[0].max(self.street_bet[1])
    }

    /// The options of the player to act, or `None` if the hand is over.
    pub fn legal(&self) -> Option<Legal> {
        let seat = self.to_act?;
        let (s, o) = (seat.index(), seat.other().index());
        let max = self.max_bet();
        let to_call = max - self.street_bet[s];
        let call = (to_call > 0).then(|| to_call.min(self.stacks[s]));

        let my_total = self.street_bet[s] + self.stacks[s];
        let their_total = self.street_bet[o] + self.stacks[o];
        let max_to = my_total.min(their_total);
        let raise = (self.stacks[o] > 0 && max_to > max).then(|| RaiseBounds {
            min_to: (max + self.last_raise).min(max_to),
            max_to,
        });

        Some(Legal {
            seat,
            can_fold: to_call > 0,
            can_check: to_call == 0,
            call,
            raise,
        })
    }

    /// Applies `action` for `seat`. On error the hand is unchanged.
    pub fn act(&mut self, seat: Seat, action: Action) -> Result<Vec<Event>, HandError> {
        let legal = self.legal().ok_or(HandError::HandComplete)?;
        if seat != legal.seat {
            return Err(HandError::NotYourTurn {
                expected: legal.seat,
            });
        }
        let action = match action {
            Action::AllIn => match (legal.raise, legal.call) {
                (Some(b), _) => Action::RaiseTo(b.max_to),
                (None, Some(_)) => Action::Call,
                (None, None) => return Err(HandError::RaiseNotAllowed),
            },
            other => other,
        };
        let s = seat.index();
        let facing = self.max_bet();
        let added = match action {
            Action::Fold if !legal.can_fold => return Err(HandError::CannotFold),
            Action::Check if !legal.can_check => return Err(HandError::CannotCheck),
            Action::Fold | Action::Check => 0,
            Action::Call => {
                let amount = legal.call.ok_or(HandError::NothingToCall)?;
                self.commit(seat, amount)
            }
            Action::RaiseTo(to) => {
                let bounds = legal.raise.ok_or(HandError::RaiseNotAllowed)?;
                if to < bounds.min_to || to > bounds.max_to {
                    return Err(HandError::RaiseOutOfRange {
                        min_to: bounds.min_to,
                        max_to: bounds.max_to,
                    });
                }
                // Only a full raise sets the new minimum; a short all-in doesn't.
                self.last_raise = self.last_raise.max(to - facing);
                self.acted[seat.other().index()] = false;
                self.aggressor = Some(seat);
                self.commit(seat, to - self.street_bet[s])
            }
            Action::AllIn => unreachable!("resolved above"),
        };
        self.acted[s] = true;

        let mut ev = vec![Event::Acted {
            seat,
            action,
            facing,
            added,
            street_total: self.street_bet[s],
            all_in: self.stacks[s] == 0,
        }];
        if action == Action::Fold {
            self.award_fold(seat.other(), &mut ev);
            return Ok(ev);
        }
        self.to_act = self.next_to_act([seat.other(), seat]);
        if self.to_act.is_none() {
            self.close_street(&mut ev);
        }
        Ok(ev)
    }

    /// Moves up to `amount` from `seat`'s stack into the pot; returns what moved.
    fn commit(&mut self, seat: Seat, amount: Chips) -> Chips {
        let s = seat.index();
        let amount = amount.min(self.stacks[s]);
        self.stacks[s] -= amount;
        self.street_bet[s] += amount;
        self.committed[s] += amount;
        amount
    }

    fn needs_action(&self, seat: Seat) -> bool {
        let (s, o) = (seat.index(), seat.other().index());
        if self.stacks[s] == 0 {
            return false;
        }
        if self.street_bet[s] < self.max_bet() {
            return true;
        }
        // Matched: still owed a decision only if it hasn't acted and the opponent can
        // still respond to a raise.
        !self.acted[s] && self.stacks[o] > 0
    }

    fn next_to_act(&self, order: [Seat; 2]) -> Option<Seat> {
        order.into_iter().find(|&s| self.needs_action(s))
    }

    fn return_uncalled(&mut self, ev: &mut Vec<Event>) {
        let [a, b] = self.street_bet;
        if a == b {
            return;
        }
        let seat = if a > b { Seat::Zero } else { Seat::One };
        let amount = a.abs_diff(b);
        let s = seat.index();
        self.street_bet[s] -= amount;
        self.committed[s] -= amount;
        self.stacks[s] += amount;
        ev.push(Event::UncalledReturned { seat, amount });
    }

    fn close_street(&mut self, ev: &mut Vec<Event>) {
        self.return_uncalled(ev);
        let someone_all_in = self.stacks.contains(&0);
        if self.street == Street::River || someone_all_in {
            if self.street != Street::River {
                ev.push(Event::HandsTabled { cards: self.hole });
            }
            while self.street != Street::River {
                self.deal_next_street(ev);
            }
            self.showdown(ev);
        } else {
            self.deal_next_street(ev);
            self.street_bet = [0; 2];
            self.acted = [false; 2];
            self.last_raise = self.big_blind;
            self.aggressor = None;
            self.to_act = self.next_to_act([self.button.other(), self.button]);
            // Nobody is all-in, so the big blind always has a decision; checked in
            // release too, as a stuck hand would hang a match.
            assert!(self.to_act.is_some(), "nobody to act on a new street");
        }
    }

    fn deal_next_street(&mut self, ev: &mut Vec<Event>) {
        self.street = self.street.next();
        let cards: Vec<Card> = (0..self.street.new_cards())
            .map(|_| self.deck.deal())
            .collect();
        self.board.extend_from_slice(&cards);
        ev.push(Event::Board {
            street: self.street,
            cards,
        });
    }

    fn award_fold(&mut self, winner: Seat, ev: &mut Vec<Event>) {
        self.return_uncalled(ev);
        let pot = self.pot();
        self.stacks[winner.index()] += pot;
        ev.push(Event::Won {
            seat: winner,
            amount: pot,
        });
        self.finish(ev);
    }

    fn showdown(&mut self, ev: &mut Vec<Event>) {
        let rank = |seat: Seat| {
            let h = self.hole[seat.index()];
            let b = &self.board;
            eval::evaluate(&[h[0], h[1], b[0], b[1], b[2], b[3], b[4]])
        };
        let ranks = [rank(Seat::Zero), rank(Seat::One)];
        let bb = self.button.other();
        let first = self.aggressor.unwrap_or(bb);
        for seat in [first, first.other()] {
            ev.push(Event::Showdown {
                seat,
                cards: self.hole[seat.index()],
                rank: ranks[seat.index()],
            });
        }

        assert_eq!(
            self.committed[0], self.committed[1],
            "uneven pot at showdown"
        );
        let pot = self.pot();
        let shares: [(Seat, Chips); 2] = match ranks[0].cmp(&ranks[1]) {
            std::cmp::Ordering::Greater => [(Seat::Zero, pot), (Seat::One, 0)],
            std::cmp::Ordering::Less => [(Seat::One, pot), (Seat::Zero, 0)],
            std::cmp::Ordering::Equal => [(bb, pot - pot / 2), (bb.other(), pot / 2)],
        };
        for (seat, amount) in shares {
            if amount > 0 {
                self.stacks[seat.index()] += amount;
                ev.push(Event::Won { seat, amount });
            }
        }
        self.finish(ev);
    }

    fn finish(&mut self, ev: &mut Vec<Event>) {
        self.complete = true;
        self.to_act = None;
        ev.push(Event::Ended {
            stacks: self.stacks,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::parse_cards;
    use Action::*;
    use Seat::*;

    /// Hand with seat Zero on the button. `cards` (in deal order): BB's first card,
    /// button's first, BB's second, button's second, then flop, turn, river.
    fn hand(stacks: [Chips; 2], cards: &str) -> (Hand, Vec<Event>) {
        let config = HandConfig {
            button: Zero,
            stacks,
            small_blind: 50,
            big_blind: 100,
        };
        Hand::new(config, Deck::stacked(&parse_cards(cards).unwrap())).unwrap()
    }

    const BTN_WINS: &str = "2c As 7d Ad Kh Qs 3c 8h 9d"; // button AA, BB 27o
    const SPLIT: &str = "2c 3c 4d 4h Ah Kh Qh Jh Th"; // royal on board

    fn won(ev: &[Event]) -> Vec<(Seat, Chips)> {
        ev.iter()
            .filter_map(|e| match e {
                Event::Won { seat, amount } => Some((*seat, *amount)),
                _ => None,
            })
            .collect()
    }

    fn total(h: &Hand) -> Chips {
        h.stacks().iter().sum::<Chips>() + h.pot()
    }

    #[test]
    fn blinds_and_preflop_order() {
        let (h, ev) = hand([1000, 1000], BTN_WINS);
        assert_eq!(h.stacks(), [950, 900]);
        assert_eq!(h.pot(), 150);
        assert_eq!(h.to_act(), Some(Zero), "button acts first preflop");
        let l = h.legal().unwrap();
        assert_eq!(l.call, Some(50));
        assert!(!l.can_check && l.can_fold);
        assert_eq!(
            l.raise,
            Some(RaiseBounds {
                min_to: 200,
                max_to: 1000
            })
        );
        assert_eq!(
            ev.iter()
                .filter(|e| matches!(e, Event::HoleCards { .. }))
                .count(),
            2
        );
        assert_eq!(h.hole_cards(Zero), parse_cards("As Ad").unwrap()[..]);
    }

    #[test]
    fn button_folds_preflop() {
        let (mut h, _) = hand([1000, 1000], BTN_WINS);
        let ev = h.act(Zero, Fold).unwrap();
        assert!(h.is_complete());
        assert_eq!(h.stacks(), [950, 1050]);
        // BB's extra 50 was never called.
        assert!(ev.contains(&Event::UncalledReturned {
            seat: One,
            amount: 50
        }));
        assert_eq!(won(&ev), [(One, 100)]);
    }

    #[test]
    fn limp_gives_big_blind_the_option_then_bb_acts_first_postflop() {
        let (mut h, _) = hand([1000, 1000], BTN_WINS);
        h.act(Zero, Call).unwrap();
        assert_eq!(h.to_act(), Some(One), "big blind option");
        assert!(h.legal().unwrap().can_check);
        assert!(!h.legal().unwrap().can_fold);
        assert_eq!(h.act(One, Fold), Err(HandError::CannotFold));
        let ev = h.act(One, Check).unwrap();
        assert!(ev.iter().any(|e| matches!(
            e,
            Event::Board {
                street: Street::Flop,
                ..
            }
        )));
        assert_eq!(h.street(), Street::Flop);
        assert_eq!(h.board().len(), 3);
        assert_eq!(h.to_act(), Some(One), "big blind first postflop");
    }

    #[test]
    fn check_down_to_showdown() {
        let (mut h, _) = hand([1000, 1000], BTN_WINS);
        h.act(Zero, Call).unwrap();
        h.act(One, Check).unwrap();
        let mut last = vec![];
        for _ in 0..3 {
            h.act(One, Check).unwrap();
            last = h.act(Zero, Check).unwrap();
        }
        assert!(h.is_complete());
        assert_eq!(h.board().len(), 5);
        assert_eq!(won(&last), [(Zero, 200)]);
        assert_eq!(h.stacks(), [1100, 900]);
        // No aggressor on the river: big blind shows first.
        let first_show = last.iter().find_map(|e| match e {
            Event::Showdown { seat, .. } => Some(*seat),
            _ => None,
        });
        assert_eq!(first_show, Some(One));
    }

    #[test]
    fn min_raise_tracks_last_full_raise() {
        let (mut h, _) = hand([10_000, 10_000], BTN_WINS);
        assert_eq!(
            h.act(Zero, RaiseTo(150)),
            Err(HandError::RaiseOutOfRange {
                min_to: 200,
                max_to: 10_000
            })
        );
        h.act(Zero, RaiseTo(300)).unwrap(); // raise by 200
        assert_eq!(h.legal().unwrap().raise.unwrap().min_to, 500);
        h.act(One, RaiseTo(1000)).unwrap(); // raise by 700
        assert_eq!(h.legal().unwrap().raise.unwrap().min_to, 1700);
        h.act(Zero, Call).unwrap();
        // New street: minimum bet resets to the big blind.
        assert_eq!(h.street(), Street::Flop);
        assert_eq!(h.legal().unwrap().raise.unwrap().min_to, 100);
    }

    #[test]
    fn errors_leave_state_unchanged() {
        let (mut h, _) = hand([1000, 1000], BTN_WINS);
        let before = (h.stacks(), h.street_bets(), h.to_act());
        assert_eq!(
            h.act(One, Call),
            Err(HandError::NotYourTurn { expected: Zero })
        );
        assert_eq!(h.act(Zero, Check), Err(HandError::CannotCheck));
        assert_eq!(
            h.act(Zero, RaiseTo(5000)),
            Err(HandError::RaiseOutOfRange {
                min_to: 200,
                max_to: 1000
            })
        );
        assert_eq!((h.stacks(), h.street_bets(), h.to_act()), before);
    }

    #[test]
    fn raise_is_capped_at_effective_stack() {
        let (h, _) = hand([5000, 800], BTN_WINS);
        assert_eq!(h.legal().unwrap().raise.unwrap().max_to, 800);
    }

    #[test]
    fn all_in_and_call_runs_out_the_board() {
        let (mut h, _) = hand([1000, 1000], BTN_WINS);
        h.act(Zero, RaiseTo(1000)).unwrap();
        assert_eq!(h.legal().unwrap().raise, None, "can't raise an all-in");
        let ev = h.act(One, Call).unwrap();
        assert!(h.is_complete());
        let boards = ev
            .iter()
            .filter(|e| matches!(e, Event::Board { .. }))
            .count();
        assert_eq!(boards, 3);
        assert_eq!(h.stacks(), [2000, 0]);
    }

    #[test]
    fn short_all_in_call_returns_the_excess() {
        // Button (400) shoves; BB (1000) calls 300 more; pot 800 to the button's aces.
        let (mut h, _) = hand([400, 1000], BTN_WINS);
        h.act(Zero, RaiseTo(400)).unwrap();
        let ev = h.act(One, Call).unwrap();
        assert_eq!(h.stacks(), [800, 600]);
        assert!(won(&ev).contains(&(Zero, 800)));
    }

    #[test]
    fn short_all_in_raise_does_not_set_min_raise() {
        // Button raises to 300; BB has 350 total: all-in to 350 is a short raise.
        let (mut h, _) = hand([10_000, 350], BTN_WINS);
        h.act(Zero, RaiseTo(300)).unwrap();
        let l = h.legal().unwrap();
        assert_eq!(
            l.raise,
            Some(RaiseBounds {
                min_to: 350,
                max_to: 350
            })
        );
        h.act(One, RaiseTo(350)).unwrap();
        // Button may only call the extra 50 or fold.
        let l = h.legal().unwrap();
        assert_eq!((l.call, l.raise), (Some(50), None));
    }

    #[test]
    fn button_all_in_posting_small_blind() {
        let (h, ev) = hand([30, 1000], BTN_WINS);
        assert!(h.is_complete(), "no decisions possible");
        assert!(ev.contains(&Event::UncalledReturned {
            seat: One,
            amount: 70
        }));
        assert_eq!(h.stacks(), [60, 970]);
    }

    #[test]
    fn big_blind_all_in_for_less_than_small_blind() {
        let (h, ev) = hand([1000, 30], BTN_WINS);
        assert!(h.is_complete());
        assert!(ev.contains(&Event::UncalledReturned {
            seat: Zero,
            amount: 20
        }));
        assert_eq!(h.stacks(), [1030, 0]);
    }

    #[test]
    fn big_blind_short_all_in_leaves_button_call_or_fold() {
        let (mut h, _) = hand([1000, 70], BTN_WINS);
        let l = h.legal().unwrap();
        assert_eq!((l.call, l.raise), (Some(20), None));
        h.act(Zero, Call).unwrap();
        assert!(h.is_complete());
        assert_eq!(h.stacks(), [1070, 0]);
    }

    #[test]
    fn board_plays_splits_the_pot() {
        let (mut h, _) = hand([1000, 1000], SPLIT);
        h.act(Zero, Call).unwrap();
        h.act(One, Check).unwrap();
        h.act(One, RaiseTo(100)).unwrap();
        h.act(Zero, Call).unwrap();
        for _ in 0..2 {
            h.act(One, Check).unwrap();
            h.act(Zero, Check).unwrap();
        }
        assert!(h.is_complete());
        assert_eq!(h.stacks(), [1000, 1000]);
        assert_eq!(total(&h), 2000);
    }

    #[test]
    fn invalid_configs_are_rejected() {
        let deck = || Deck::stacked(&[]);
        let cfg = |stacks: [Chips; 2], small_blind, big_blind| HandConfig {
            button: Zero,
            stacks,
            small_blind,
            big_blind,
        };
        let err = |c: HandConfig| Hand::new(c, deck()).err();
        use ConfigError::*;
        assert_eq!(
            err(cfg([0, 100], 1, 2)),
            Some(HandError::InvalidConfig(EmptyStack))
        );
        assert_eq!(
            err(cfg([100, 100], 2, 2)),
            Some(HandError::InvalidConfig(BadBlinds))
        );
        assert_eq!(
            err(cfg([100, 100], 0, 2)),
            Some(HandError::InvalidConfig(BadBlinds))
        );
        let huge = u64::MAX / 2 + 10;
        assert_eq!(
            err(cfg([huge, huge], 1, 2)),
            Some(HandError::InvalidConfig(TooManyChips))
        );
        let max = MAX_TOTAL_CHIPS / 2;
        assert!(err(cfg([max, max], 1, 2)).is_none());
    }

    #[test]
    fn all_in_action_shoves_the_effective_stack() {
        let (mut h, _) = hand([5000, 800], BTN_WINS);
        let ev = h.act(Zero, AllIn).unwrap();
        assert!(matches!(
            ev[0],
            Event::Acted {
                action: RaiseTo(800),
                ..
            }
        ));
        // Facing an all-in, AllIn resolves to a call.
        let ev = h.act(One, AllIn).unwrap();
        assert!(matches!(
            ev[0],
            Event::Acted {
                action: Call,
                all_in: true,
                ..
            }
        ));
        assert!(h.is_complete());
    }

    #[test]
    fn hands_are_tabled_before_an_all_in_runout() {
        let (mut h, _) = hand([1000, 1000], BTN_WINS);
        h.act(Zero, AllIn).unwrap();
        let ev = h.act(One, Call).unwrap();
        let tabled = ev
            .iter()
            .position(|e| matches!(e, Event::HandsTabled { .. }));
        let first_board = ev.iter().position(|e| matches!(e, Event::Board { .. }));
        assert!(tabled.unwrap() < first_board.unwrap());
    }

    #[test]
    fn hole_cards_are_private_and_redacted_in_debug() {
        let (_, ev) = hand([1000, 1000], BTN_WINS);
        let hole: Vec<&Event> = ev
            .iter()
            .filter(|e| matches!(e, Event::HoleCards { .. }))
            .collect();
        assert_eq!(hole[0].visibility(), Visibility::Private(Zero));
        let dbg = format!("{ev:?}");
        assert!(!dbg.contains("As") && dbg.contains("<hidden>"), "{dbg}");
    }

    #[test]
    fn passive_default_checks_or_folds() {
        let (mut h, _) = hand([1000, 1000], BTN_WINS);
        assert_eq!(h.legal().unwrap().passive_default(), Fold);
        h.act(Zero, Call).unwrap();
        assert_eq!(h.legal().unwrap().passive_default(), Check);
    }

    #[test]
    fn public_state_matches_accessors() {
        let (mut h, _) = hand([1000, 1000], BTN_WINS);
        h.act(Zero, RaiseTo(300)).unwrap();
        let p = h.public_state();
        assert_eq!(p.pot, 400);
        assert_eq!(p.committed, [300, 100]);
        assert_eq!(p.legal, h.legal());
        assert!(p.board.is_empty());
    }
}
