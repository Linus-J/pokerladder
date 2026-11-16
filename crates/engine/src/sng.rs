//! A heads-up sit-and-go match: equal starting stacks, blinds rising by hand count, an
//! alternating button and a per-player time bank. Played until one player has all the
//! chips.
//!
//! Like [`crate::holdem`], this is pure state. The server supplies shuffled decks and
//! measures wall-clock think time; the match does the accounting. Blind levels go up
//! by hand count, not by clock time, so a player can't hurry the blinds along (or hold
//! them back) by stalling.

use std::time::Duration;

use crate::card::Deck;
use crate::holdem::{
    Action, Chips, ConfigError, Event, Hand, HandConfig, HandError, MAX_TOTAL_CHIPS, Seat,
    Visibility,
};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Level {
    pub small_blind: Chips,
    pub big_blind: Chips,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct TimeControl {
    /// Free thinking time for every decision.
    pub per_action: Duration,
    /// Each player's bank at the start of the match, drawn on once `per_action` runs out.
    pub initial_bank: Duration,
    /// Added to both banks every `topup_every_hands` hands.
    pub bank_topup: Duration,
    pub topup_every_hands: u32,
}

impl TimeControl {
    /// 15 s per action, 60 s bank, +10 s every 10 hands.
    pub const STANDARD: TimeControl = TimeControl {
        per_action: Duration::from_secs(15),
        initial_bank: Duration::from_secs(60),
        bank_topup: Duration::from_secs(10),
        topup_every_hands: 10,
    };
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MatchConfig {
    pub starting_stack: Chips,
    pub hands_per_level: u32,
    /// Blind levels in order; the last one repeats until the match ends.
    pub levels: Vec<Level>,
    pub time: TimeControl,
}

impl MatchConfig {
    /// The ladder format: 1500 chips (75 big blinds), blinds from 10/20 rising about
    /// 50% every 10 hands. Typically 60–90 hands, 15–20 minutes. The last level
    /// (1500/3000) puts everyone all-in, so a match always ends.
    pub fn standard() -> Self {
        let blinds = [
            (10, 20),
            (15, 30),
            (25, 50),
            (40, 80),
            (60, 120),
            (100, 200),
            (150, 300),
            (250, 500),
            (400, 800),
            (600, 1200),
            (1000, 2000),
            (1500, 3000),
        ];
        MatchConfig {
            starting_stack: 1500,
            hands_per_level: 10,
            levels: blinds
                .iter()
                .map(|&(small_blind, big_blind)| Level {
                    small_blind,
                    big_blind,
                })
                .collect(),
            time: TimeControl::STANDARD,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MatchEvent {
    HandStarted {
        /// 1-based.
        number: u32,
        /// 1-based blind level.
        level: u32,
        button: Seat,
        small_blind: Chips,
        big_blind: Chips,
        stacks: [Chips; 2],
        time_banks: [Duration; 2],
    },
    Hand(Event),
    /// `seat` ran out of time; the hand events that follow are its automatic
    /// check or fold.
    TimedOut {
        seat: Seat,
    },
    MatchOver {
        winner: Seat,
    },
}

impl MatchEvent {
    /// Who may see this event. Always filter with this (never forward `MatchEvent`s
    /// wholesale): `Hand(HoleCards)` is private.
    pub fn visibility(&self) -> Visibility {
        match self {
            MatchEvent::Hand(e) => e.visibility(),
            _ => Visibility::Public,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MatchError {
    HandInProgress,
    NoHandInProgress,
    MatchOver,
    Hand(HandError),
}

impl From<HandError> for MatchError {
    fn from(e: HandError) -> Self {
        MatchError::Hand(e)
    }
}

impl std::fmt::Display for MatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MatchError::HandInProgress => f.write_str("a hand is already in progress"),
            MatchError::NoHandInProgress => f.write_str("no hand in progress"),
            MatchError::MatchOver => f.write_str("the match is over"),
            MatchError::Hand(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for MatchError {}

/// Neither `Debug` nor `Clone`, for the same reason as [`Hand`].
pub struct Match {
    config: MatchConfig,
    stacks: [Chips; 2],
    hands_played: u32,
    next_button: Seat,
    hand: Option<Hand>,
    auto_muck: [bool; 2],
    banks: [Duration; 2],
    winner: Option<Seat>,
}

impl Match {
    /// `first_button` should be chosen at random by the server.
    pub fn new(config: MatchConfig, first_button: Seat) -> Result<Match, ConfigError> {
        if config.starting_stack == 0 {
            return Err(ConfigError::EmptyStack);
        }
        let Some(total) = config
            .starting_stack
            .checked_mul(2)
            .filter(|&t| t <= MAX_TOTAL_CHIPS)
        else {
            return Err(ConfigError::TooManyChips);
        };
        // Same per-level rules `Hand::new` enforces, so a match can't get stuck on a
        // level it can never deal.
        for l in &config.levels {
            if l.small_blind == 0 || l.small_blind >= l.big_blind {
                return Err(ConfigError::BadBlinds);
            }
            if l.big_blind > MAX_TOTAL_CHIPS {
                return Err(ConfigError::TooManyChips);
            }
        }
        // The last level must force everyone all-in, so every match ends.
        let ends = config.levels.last().is_some_and(|l| l.big_blind >= total);
        if !ends || config.hands_per_level == 0 || config.time.topup_every_hands == 0 {
            return Err(ConfigError::BadSchedule);
        }
        Ok(Match {
            stacks: [config.starting_stack; 2],
            banks: [config.time.initial_bank; 2],
            config,
            hands_played: 0,
            next_button: first_button,
            hand: None,
            auto_muck: [true; 2],
            winner: None,
        })
    }

    pub fn stacks(&self) -> [Chips; 2] {
        self.stacks
    }

    pub fn hands_played(&self) -> u32 {
        self.hands_played
    }

    pub fn winner(&self) -> Option<Seat> {
        self.winner
    }

    /// The hand in progress, if any (for `legal()` and `public_state()`).
    pub fn hand(&self) -> Option<&Hand> {
        self.hand.as_ref()
    }

    pub fn time_banks(&self) -> [Duration; 2] {
        self.banks
    }

    /// How long `seat` may take over its current decision: the server's deadline.
    pub fn time_allowed(&self, seat: Seat) -> Duration {
        self.config.time.per_action + self.banks[seat.index()]
    }

    /// Takes effect from the next hand.
    pub fn set_auto_muck(&mut self, seat: Seat, on: bool) {
        self.auto_muck[seat.index()] = on;
    }

    pub fn auto_muck(&self, seat: Seat) -> bool {
        self.auto_muck[seat.index()]
    }

    /// 0-based index into the schedule for the hand about to start (or in progress).
    fn level_index(&self) -> usize {
        let i = (self.hands_played / self.config.hands_per_level) as usize;
        i.min(self.config.levels.len() - 1)
    }

    /// Deals the next hand from `deck` (shuffled by the server with a CSPRNG).
    pub fn start_hand(&mut self, deck: Deck) -> Result<Vec<MatchEvent>, MatchError> {
        if self.winner.is_some() {
            return Err(MatchError::MatchOver);
        }
        if self.hand.is_some() {
            return Err(MatchError::HandInProgress);
        }
        let level_index = self.level_index();
        let Level {
            small_blind,
            big_blind,
        } = self.config.levels[level_index];
        let button = self.next_button;
        let (hand, events) = Hand::new(
            HandConfig {
                button,
                stacks: self.stacks,
                small_blind,
                big_blind,
                auto_muck: self.auto_muck,
            },
            deck,
        )?;
        // Only now that the hand exists: a failed start must not top up the banks.
        self.next_button = button.other();
        let tc = self.config.time;
        if self.hands_played > 0 && self.hands_played.is_multiple_of(tc.topup_every_hands) {
            for bank in &mut self.banks {
                *bank = bank.saturating_add(tc.bank_topup);
            }
        }

        let mut out = vec![MatchEvent::HandStarted {
            number: self.hands_played + 1,
            level: level_index as u32 + 1,
            button,
            small_blind,
            big_blind,
            stacks: self.stacks,
            time_banks: self.banks,
        }];
        out.extend(events.into_iter().map(MatchEvent::Hand));
        self.hand = Some(hand);
        self.after_hand_events(&mut out);
        Ok(out)
    }

    /// Applies a player's action. Anything beyond `per_action` is drawn from their bank.
    ///
    /// `elapsed` must be measured by the server from the moment this decision was
    /// offered, **not** reset by rejected actions — otherwise a player could stall
    /// forever by sending illegal actions. An action arriving after
    /// [`time_allowed`](Self::time_allowed) is treated as a [`timeout`](Self::timeout).
    /// On error nothing changes (including the bank).
    pub fn act(
        &mut self,
        seat: Seat,
        action: Action,
        elapsed: Duration,
    ) -> Result<Vec<MatchEvent>, MatchError> {
        if self.winner.is_some() {
            return Err(MatchError::MatchOver);
        }
        let late = elapsed > self.time_allowed(seat);
        let hand = self.hand.as_mut().ok_or(MatchError::NoHandInProgress)?;
        if late && hand.to_act() == Some(seat) {
            return self.timeout(seat);
        }
        let events = hand.act(seat, action)?;
        let over = elapsed.saturating_sub(self.config.time.per_action);
        let bank = &mut self.banks[seat.index()];
        *bank = bank.saturating_sub(over);

        let mut out: Vec<MatchEvent> = events.into_iter().map(MatchEvent::Hand).collect();
        self.after_hand_events(&mut out);
        Ok(out)
    }

    /// The server's deadline for `seat` passed: empty its bank and check or fold.
    pub fn timeout(&mut self, seat: Seat) -> Result<Vec<MatchEvent>, MatchError> {
        if self.winner.is_some() {
            return Err(MatchError::MatchOver);
        }
        let hand = self.hand.as_mut().ok_or(MatchError::NoHandInProgress)?;
        let legal = hand.legal().ok_or(MatchError::NoHandInProgress)?;
        if legal.seat != seat {
            return Err(HandError::NotYourTurn {
                expected: legal.seat,
            }
            .into());
        }
        let events = hand.act(seat, legal.passive_default())?;
        self.banks[seat.index()] = Duration::ZERO;

        let mut out = vec![MatchEvent::TimedOut { seat }];
        out.extend(events.into_iter().map(MatchEvent::Hand));
        self.after_hand_events(&mut out);
        Ok(out)
    }

    fn after_hand_events(&mut self, out: &mut Vec<MatchEvent>) {
        let Some(hand) = &self.hand else { return };
        if !hand.is_complete() {
            return;
        }
        self.stacks = hand.stacks();
        self.hand = None;
        self.hands_played += 1;
        if let Some(loser) = Seat::BOTH.into_iter().find(|s| self.stacks[s.index()] == 0) {
            let winner = loser.other();
            self.winner = Some(winner);
            out.push(MatchEvent::MatchOver { winner });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Seat::*;
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    fn deck(seed: u64) -> Deck {
        Deck::shuffled(&mut StdRng::seed_from_u64(seed))
    }

    fn started(ev: &[MatchEvent]) -> (u32, u32, Seat, Chips, Chips) {
        match ev[0] {
            MatchEvent::HandStarted {
                number,
                level,
                button,
                small_blind,
                big_blind,
                ..
            } => (number, level, button, small_blind, big_blind),
            _ => panic!("expected HandStarted"),
        }
    }

    /// Plays a hand to the end with the button folding preflop.
    fn fold_hand(m: &mut Match, seed: u64) -> Vec<MatchEvent> {
        let mut ev = m.start_hand(deck(seed)).unwrap();
        if let Some(seat) = m.hand().and_then(|h| h.to_act()) {
            ev.extend(m.act(seat, Action::Fold, Duration::ZERO).unwrap());
        }
        ev
    }

    #[test]
    fn standard_config_matches_the_agreed_format() {
        let c = MatchConfig::standard();
        assert_eq!(c.starting_stack, 1500);
        assert_eq!(c.hands_per_level, 10);
        assert_eq!(
            c.levels[0],
            Level {
                small_blind: 10,
                big_blind: 20
            }
        );
        assert!(c.levels.last().unwrap().big_blind >= 2 * c.starting_stack);
        assert_eq!(c.time, TimeControl::STANDARD);
    }

    #[test]
    fn blinds_rise_every_ten_hands_and_button_alternates() {
        let mut m = Match::new(MatchConfig::standard(), Zero).unwrap();
        for n in 1..=21 {
            let ev = fold_hand(&mut m, n as u64);
            let (number, level, button, sb, bb) = started(&ev);
            assert_eq!(number, n);
            assert_eq!(button, if n % 2 == 1 { Zero } else { One });
            let expected = match n {
                1..=10 => (1, 10, 20),
                11..=20 => (2, 15, 30),
                _ => (3, 25, 50),
            };
            assert_eq!((level, sb, bb), expected, "hand {n}");
        }
        assert_eq!(m.stacks().iter().sum::<Chips>(), 3000);
    }

    #[test]
    fn time_bank_is_charged_only_beyond_per_action_time() {
        let mut m = Match::new(MatchConfig::standard(), Zero).unwrap();
        m.start_hand(deck(1)).unwrap();
        assert_eq!(m.time_allowed(Zero), Duration::from_secs(75));
        // Errors don't charge the bank.
        assert!(m.act(One, Action::Call, Duration::from_secs(40)).is_err());
        assert_eq!(m.time_banks(), [Duration::from_secs(60); 2]);
        m.act(Zero, Action::Call, Duration::from_secs(20)).unwrap();
        assert_eq!(m.time_banks()[0], Duration::from_secs(55));
        m.act(One, Action::Check, Duration::from_secs(3)).unwrap();
        assert_eq!(m.time_banks()[1], Duration::from_secs(60));
    }

    #[test]
    fn timeout_drains_bank_and_checks_or_folds() {
        let mut m = Match::new(MatchConfig::standard(), Zero).unwrap();
        m.start_hand(deck(1)).unwrap();
        assert!(m.timeout(One).is_err(), "not One's turn");
        let ev = m.timeout(Zero).unwrap();
        assert_eq!(ev[0], MatchEvent::TimedOut { seat: Zero });
        assert!(ev.iter().any(|e| matches!(
            e,
            MatchEvent::Hand(Event::Acted {
                action: Action::Fold,
                ..
            })
        )));
        assert_eq!(m.time_banks()[0], Duration::ZERO);
        assert!(m.hand().is_none(), "fold ended the hand");
        // The bank comes back in 10 s steps every 10 hands.
        for n in 2..=11 {
            fold_hand(&mut m, n);
        }
        assert_eq!(m.time_banks()[0], Duration::from_secs(10));
    }

    #[test]
    fn match_ends_when_a_player_busts() {
        let mut m = Match::new(MatchConfig::standard(), Zero).unwrap();
        let mut seed = 0;
        while m.winner().is_none() {
            seed += 1;
            m.start_hand(deck(seed)).unwrap();
            while let Some(seat) = m.hand().and_then(|h| h.to_act()) {
                m.act(seat, Action::AllIn, Duration::ZERO).unwrap();
            }
        }
        let w = m.winner().unwrap();
        assert_eq!(m.stacks()[w.index()], 3000);
        assert_eq!(m.start_hand(deck(0)).unwrap_err(), MatchError::MatchOver);
    }

    #[test]
    fn misuse_is_rejected() {
        let mut m = Match::new(MatchConfig::standard(), One).unwrap();
        assert_eq!(
            m.act(Zero, Action::Call, Duration::ZERO).unwrap_err(),
            MatchError::NoHandInProgress
        );
        m.start_hand(deck(1)).unwrap();
        assert_eq!(
            m.start_hand(deck(2)).unwrap_err(),
            MatchError::HandInProgress
        );
        let mut bad = MatchConfig::standard();
        bad.levels.clear();
        assert_eq!(Match::new(bad, Zero).err(), Some(ConfigError::BadSchedule));
    }

    #[test]
    fn failed_start_does_not_top_up_banks() {
        let mut m = Match::new(MatchConfig::standard(), Zero).unwrap();
        for n in 1..=10 {
            fold_hand(&mut m, n);
        }
        let mut short = deck(99);
        for _ in 0..45 {
            short.deal();
        }
        assert!(m.start_hand(short).is_err());
        assert_eq!(m.time_banks(), [Duration::from_secs(60); 2]);
        m.start_hand(deck(100)).unwrap();
        assert_eq!(m.time_banks(), [Duration::from_secs(70); 2]);
    }

    #[test]
    fn schedules_that_could_stall_are_rejected() {
        let base = MatchConfig::standard;
        let mut c = base();
        c.levels.pop(); // last level 1000/2000 < 3000 chips: may never end
        assert_eq!(Match::new(c, Zero).err(), Some(ConfigError::BadSchedule));
        let mut c = base();
        c.levels[1].big_blind = MAX_TOTAL_CHIPS + 1;
        assert_eq!(Match::new(c, Zero).err(), Some(ConfigError::TooManyChips));
        let mut c = base();
        c.time.topup_every_hands = 0;
        assert_eq!(Match::new(c, Zero).err(), Some(ConfigError::BadSchedule));
    }

    #[test]
    fn late_action_is_a_timeout() {
        let mut m = Match::new(MatchConfig::standard(), Zero).unwrap();
        m.start_hand(deck(1)).unwrap();
        let ev = m.act(Zero, Action::AllIn, Duration::from_secs(76)).unwrap();
        assert_eq!(ev[0], MatchEvent::TimedOut { seat: Zero });
        assert!(ev.iter().any(|e| matches!(
            e,
            MatchEvent::Hand(Event::Acted {
                action: Action::Fold,
                ..
            })
        )));
        assert_eq!(m.time_banks()[0], Duration::ZERO);
    }

    #[test]
    fn hole_cards_are_private_match_events() {
        let mut m = Match::new(MatchConfig::standard(), Zero).unwrap();
        let ev = m.start_hand(deck(1)).unwrap();
        let private: Vec<_> = ev
            .iter()
            .filter(|e| e.visibility() != Visibility::Public)
            .collect();
        assert_eq!(private.len(), 2);
        assert!(
            private
                .iter()
                .all(|e| matches!(e, MatchEvent::Hand(Event::HoleCards { .. })))
        );
    }

    #[test]
    fn calls_after_the_match_report_match_over() {
        let mut m = Match::new(MatchConfig::standard(), Zero).unwrap();
        let mut seed = 0;
        while m.winner().is_none() {
            seed += 1;
            m.start_hand(deck(seed)).unwrap();
            while let Some(seat) = m.hand().and_then(|h| h.to_act()) {
                m.act(seat, Action::AllIn, Duration::ZERO).unwrap();
            }
        }
        assert_eq!(
            m.act(Zero, Action::Check, Duration::ZERO),
            Err(MatchError::MatchOver)
        );
        assert_eq!(m.timeout(Zero), Err(MatchError::MatchOver));
    }
}
