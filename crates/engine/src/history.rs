//! PokerStars-style text hand histories, rendered from a hand's [`MatchEvent`]s.
//!
//! What a history reveals depends on the [`Viewer`]: a player sees only their own hole
//! cards, a spectator none, and the audit view everything — including mucked hands.

use crate::card::Card;
use crate::eval::HandRank;
use crate::holdem::{Action, Chips, Event, Seat, Street};
use crate::sng::MatchEvent;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Viewer {
    Player(Seat),
    Spectator,
    /// Server-side integrity review: every card, including mucked hands.
    Audit,
}

impl Viewer {
    fn sees_hole_cards(self, seat: Seat) -> bool {
        match self {
            Viewer::Player(s) => s == seat,
            Viewer::Spectator => false,
            Viewer::Audit => true,
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub struct HistoryMeta<'a> {
    /// Site name for the header line, e.g. `"Pokerladder"`.
    pub site: &'a str,
    pub hand_id: u64,
    pub match_id: u64,
    /// UTC start time, formatted `YYYY/MM/DD HH:MM:SS`.
    pub started_utc: &'a str,
    /// Display names, indexed by seat.
    pub names: [&'a str; 2],
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum HistoryError {
    /// The events must start with [`MatchEvent::HandStarted`].
    MissingHandStart,
    /// The hand has no [`Event::Ended`] yet.
    Incomplete,
    /// A name, site or timestamp contains control characters (e.g. a newline that
    /// would inject fake lines). The server should also restrict display names to a
    /// safe character set such as `[A-Za-z0-9_-]{1,20}`.
    UnsafeText,
}

impl std::fmt::Display for HistoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            HistoryError::MissingHandStart => "events must start with HandStarted",
            HistoryError::Incomplete => "the hand has not ended",
            HistoryError::UnsafeText => "names, site and timestamp must be plain text",
        })
    }
}

impl std::error::Error for HistoryError {}

/// Renders one hand. `events` must start with its [`MatchEvent::HandStarted`] and
/// contain that hand's events (a trailing [`MatchEvent::MatchOver`] is included).
pub fn format_hand(
    meta: &HistoryMeta,
    events: &[MatchEvent],
    viewer: Viewer,
) -> Result<String, HistoryError> {
    let Some(MatchEvent::HandStarted {
        level,
        button,
        small_blind,
        big_blind,
        stacks,
        ..
    }) = events.first()
    else {
        return Err(HistoryError::MissingHandStart);
    };
    let texts = [meta.site, meta.started_utc, meta.names[0], meta.names[1]];
    if texts.iter().any(|t| t.chars().any(char::is_control)) {
        return Err(HistoryError::UnsafeText);
    }
    let hand_events = events[1..]
        .iter()
        .take_while(|e| !matches!(e, MatchEvent::HandStarted { .. }));
    if !hand_events
        .clone()
        .any(|e| matches!(e, MatchEvent::Hand(Event::Ended { .. })))
    {
        return Err(HistoryError::Incomplete);
    }
    let name = |s: Seat| meta.names[s.index()];
    let seat_no = |s: Seat| s.index() + 1;
    let mut out = String::new();
    let mut line = |text: String| {
        out.push_str(&text);
        out.push('\n');
    };

    line(format!(
        "{} Hand #{}: Tournament #{}, Freeroll Hold'em No Limit - Level {} ({}/{}) - {} UTC",
        meta.site,
        meta.hand_id,
        meta.match_id,
        roman(*level),
        small_blind,
        big_blind,
        meta.started_utc
    ));
    line(format!(
        "Table '{} 1' 2-max Seat #{} is the button",
        meta.match_id,
        seat_no(*button)
    ));
    for s in Seat::BOTH {
        line(format!(
            "Seat {}: {} ({} in chips)",
            seat_no(s),
            name(s),
            stacks[s.index()]
        ));
    }

    let mut board: Vec<Card> = Vec::new();
    let mut street = Street::Preflop;
    let mut hole: [Option<[Card; 2]>; 2] = [None; 2];
    let mut dealt_header = false;
    let mut showdown_header = false;
    let mut folded: [Option<Street>; 2] = [None; 2];
    let mut showed: [Option<([Card; 2], HandRank)>; 2] = [None; 2];
    let mut mucked = [false; 2];
    let mut won: [Chips; 2] = [0; 2];

    for e in hand_events {
        let ev = match e {
            MatchEvent::Hand(ev) => ev,
            MatchEvent::TimedOut { seat } => {
                line(format!("{} has timed out", name(*seat)));
                continue;
            }
            MatchEvent::MatchOver { winner } => {
                let loser = winner.other();
                line(format!(
                    "{} finished the tournament in 2nd place",
                    name(loser)
                ));
                line(format!("{} wins the tournament", name(*winner)));
                continue;
            }
            MatchEvent::HandStarted { .. } => break,
        };
        match ev {
            Event::BlindPosted {
                seat,
                amount,
                big,
                all_in,
            } => {
                let which = if *big { "big" } else { "small" };
                line(format!(
                    "{}: posts {which} blind {amount}{}",
                    name(*seat),
                    all_in_suffix(*all_in)
                ));
            }
            Event::HoleCards { seat, cards } => {
                hole[seat.index()] = Some(*cards.reveal());
                if !dealt_header {
                    line("*** HOLE CARDS ***".into());
                    dealt_header = true;
                }
                if viewer.sees_hole_cards(*seat) {
                    line(format!(
                        "Dealt to {} {}",
                        name(*seat),
                        cards_str(cards.reveal())
                    ));
                }
            }
            Event::Acted {
                seat,
                action,
                facing,
                added,
                street_total,
                all_in,
            } => {
                let what = match action {
                    Action::Fold => {
                        folded[seat.index()] = Some(street);
                        "folds".to_owned()
                    }
                    Action::Check => "checks".to_owned(),
                    Action::Call => format!("calls {added}"),
                    Action::RaiseTo(_) | Action::AllIn if *facing == 0 => {
                        format!("bets {street_total}")
                    }
                    Action::RaiseTo(_) | Action::AllIn => {
                        format!("raises {} to {street_total}", street_total - facing)
                    }
                };
                line(format!("{}: {what}{}", name(*seat), all_in_suffix(*all_in)));
            }
            Event::HandsTabled { .. } => {}
            Event::Board { street: s, cards } => {
                street = *s;
                let label = match s {
                    Street::Preflop => "PREFLOP",
                    Street::Flop => "FLOP",
                    Street::Turn => "TURN",
                    Street::River => "RIVER",
                };
                let text = if board.is_empty() {
                    format!("*** {label} *** {}", cards_str(cards))
                } else {
                    format!("*** {label} *** {} {}", cards_str(&board), cards_str(cards))
                };
                board.extend_from_slice(cards);
                line(text);
            }
            Event::UncalledReturned { seat, amount } => {
                line(format!(
                    "Uncalled bet ({amount}) returned to {}",
                    name(*seat)
                ));
            }
            Event::Showdown { seat, cards, rank } => {
                if !showdown_header {
                    line("*** SHOW DOWN ***".into());
                    showdown_header = true;
                }
                line(format!(
                    "{}: shows {} ({rank})",
                    name(*seat),
                    cards_str(cards)
                ));
                showed[seat.index()] = Some((*cards, *rank));
            }
            Event::Mucked { seat } => {
                if !showdown_header {
                    line("*** SHOW DOWN ***".into());
                    showdown_header = true;
                }
                mucked[seat.index()] = true;
                let cards = match hole[seat.index()] {
                    Some(c) if viewer.sees_hole_cards(*seat) => format!(" {}", cards_str(&c)),
                    _ => String::new(),
                };
                line(format!("{}: mucks hand{cards}", name(*seat)));
            }
            Event::Won { seat, amount } => {
                won[seat.index()] += amount;
                line(format!("{} collected {amount} from pot", name(*seat)));
            }
            Event::Ended { .. } => {}
        }
    }

    line("*** SUMMARY ***".into());
    line(format!("Total pot {} | Rake 0", won[0] + won[1]));
    if !board.is_empty() {
        line(format!("Board {}", cards_str(&board)));
    }
    for s in Seat::BOTH {
        let i = s.index();
        let roles = if s == *button {
            "(button) (small blind)"
        } else {
            "(big blind)"
        };
        let result = if let Some(at) = folded[i] {
            match at {
                Street::Preflop => "folded before Flop".to_owned(),
                Street::Flop => "folded on the Flop".to_owned(),
                Street::Turn => "folded on the Turn".to_owned(),
                Street::River => "folded on the River".to_owned(),
            }
        } else if let Some((cards, rank)) = showed[i] {
            let cards = cards_str(&cards);
            if won[i] > 0 {
                format!("showed {cards} and won ({}) with {rank}", won[i])
            } else {
                format!("showed {cards} and lost with {rank}")
            }
        } else if mucked[i] {
            match hole[i] {
                Some(c) if viewer.sees_hole_cards(s) => format!("mucked {}", cards_str(&c)),
                _ => "mucked".to_owned(),
            }
        } else {
            format!("collected ({})", won[i])
        };
        line(format!("Seat {}: {} {roles} {result}", seat_no(s), name(s)));
    }
    Ok(out)
}

fn all_in_suffix(all_in: bool) -> &'static str {
    if all_in { " and is all-in" } else { "" }
}

fn cards_str(cards: &[Card]) -> String {
    let inner: Vec<String> = cards.iter().map(Card::to_string).collect();
    format!("[{}]", inner.join(" "))
}

fn roman(mut n: u32) -> String {
    const TABLE: [(u32, &str); 9] = [
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut s = String::new();
    for (value, numeral) in TABLE {
        while n >= value {
            s.push_str(numeral);
            n -= value;
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::{Deck, parse_cards};
    use crate::sng::{Match, MatchConfig};
    use std::time::Duration;

    const META: HistoryMeta = HistoryMeta {
        site: "Pokerladder",
        hand_id: 42,
        match_id: 7,
        started_utc: "2026/11/15 17:20:00",
        names: ["alice", "bob"],
    };

    /// Seat Zero (alice) on the button. Deal order: BB, button, BB, button, board.
    fn play(cards: &str, actions: &[(Seat, Action)]) -> Vec<MatchEvent> {
        let mut m = Match::new(MatchConfig::standard(), Seat::Zero).unwrap();
        let mut ev = m
            .start_hand(Deck::stacked(&parse_cards(cards).unwrap()))
            .unwrap();
        for &(seat, action) in actions {
            ev.extend(m.act(seat, action, Duration::ZERO).unwrap());
        }
        ev
    }

    #[test]
    fn fold_on_the_flop_player_view() {
        use Action::*;
        use Seat::*;
        // bob (BB) 7d 2c, alice (button) As Kd; flop 8h Th 3h.
        let ev = play(
            "7d As 2c Kd 8h Th 3h 4s 5s",
            &[
                (Zero, RaiseTo(60)),
                (One, Call),
                (One, Check),
                (Zero, RaiseTo(60)),
                (One, Fold),
            ],
        );
        let text = format_hand(&META, &ev, Viewer::Player(Zero)).unwrap();
        let expected = "\
Pokerladder Hand #42: Tournament #7, Freeroll Hold'em No Limit - Level I (10/20) - 2026/11/15 17:20:00 UTC
Table '7 1' 2-max Seat #1 is the button
Seat 1: alice (1500 in chips)
Seat 2: bob (1500 in chips)
alice: posts small blind 10
bob: posts big blind 20
*** HOLE CARDS ***
Dealt to alice [As Kd]
alice: raises 40 to 60
bob: calls 40
*** FLOP *** [8h Th 3h]
bob: checks
alice: bets 60
bob: folds
Uncalled bet (60) returned to alice
alice collected 120 from pot
*** SUMMARY ***
Total pot 120 | Rake 0
Board [8h Th 3h]
Seat 1: alice (button) (small blind) collected (120)
Seat 2: bob (big blind) folded on the Flop
";
        assert_eq!(text, expected);
        // Spectators see no hole cards; bob's view shows only bob's.
        let spectator = format_hand(&META, &ev, Viewer::Spectator).unwrap();
        assert!(!spectator.contains("Dealt to"));
        let bob = format_hand(&META, &ev, Viewer::Player(One)).unwrap();
        assert!(bob.contains("Dealt to bob [7d 2c]") && !bob.contains("As Kd"));
    }

    #[test]
    fn mucked_cards_only_in_audit_view() {
        use Action::*;
        use Seat::*;
        // bob (BB) As Ad beats alice (button) 7d 2c; check it down. bob shows first
        // and wins, alice auto-mucks.
        let ev = play(
            "As 7d Ad 2c Kh Qs 3c 8h 9d",
            &[
                (Zero, Call),
                (One, Check),
                (One, Check),
                (Zero, Check),
                (One, Check),
                (Zero, Check),
                (One, Check),
                (Zero, Check),
            ],
        );
        let public = format_hand(&META, &ev, Viewer::Spectator).unwrap();
        assert!(public.contains("*** SHOW DOWN ***"));
        assert!(public.contains("bob: shows [As Ad] (a pair of Aces)"));
        assert!(public.contains("alice: mucks hand\n"));
        assert!(public.contains("Seat 1: alice (button) (small blind) mucked\n"));
        assert!(
            public.contains(
                "Seat 2: bob (big blind) showed [As Ad] and won (40) with a pair of Aces"
            )
        );
        assert!(!public.contains("7d"));

        let audit = format_hand(&META, &ev, Viewer::Audit).unwrap();
        assert!(audit.contains("alice: mucks hand [7d 2c]"));
        assert!(audit.contains("Dealt to alice [7d 2c]") && audit.contains("Dealt to bob [As Ad]"));
    }

    #[test]
    fn all_in_and_timeout_lines() {
        let mut m = Match::new(MatchConfig::standard(), Seat::Zero).unwrap();
        let deck = Deck::stacked(&parse_cards("As 7d Ad 2c Kh Qs 3c 8h 9d").unwrap());
        let mut ev = m.start_hand(deck).unwrap();
        ev.extend(m.act(Seat::Zero, Action::AllIn, Duration::ZERO).unwrap());
        ev.extend(m.timeout(Seat::One).unwrap());
        let text = format_hand(&META, &ev, Viewer::Spectator).unwrap();
        assert!(text.contains("alice: raises 1480 to 1500 and is all-in"));
        assert!(text.contains("bob has timed out\nbob: folds"));
        assert!(text.contains("Uncalled bet (1480) returned to alice"));
    }

    #[test]
    fn roman_numerals() {
        assert_eq!(roman(1), "I");
        assert_eq!(roman(4), "IV");
        assert_eq!(roman(9), "IX");
        assert_eq!(roman(12), "XII");
        assert_eq!(roman(14), "XIV");
    }

    #[test]
    fn requires_hand_start() {
        assert_eq!(
            format_hand(&META, &[], Viewer::Audit),
            Err(HistoryError::MissingHandStart)
        );
    }

    #[test]
    fn player_sees_own_mucked_cards_only() {
        use Action::*;
        use Seat::*;
        let ev = play(
            "As 7d Ad 2c Kh Qs 3c 8h 9d",
            &[
                (Zero, Call),
                (One, Check),
                (One, Check),
                (Zero, Check),
                (One, Check),
                (Zero, Check),
                (One, Check),
                (Zero, Check),
            ],
        );
        let alice = format_hand(&META, &ev, Viewer::Player(Zero)).unwrap();
        assert!(alice.contains("alice: mucks hand [7d 2c]"));
        let bob = format_hand(&META, &ev, Viewer::Player(One)).unwrap();
        assert!(bob.contains("alice: mucks hand\n") && !bob.contains("7d"));
    }

    #[test]
    fn rejects_injection_and_incomplete_hands() {
        let ev = play("As 7d Ad 2c Kh Qs 3c 8h 9d", &[]);
        assert_eq!(
            format_hand(&META, &ev, Viewer::Audit),
            Err(HistoryError::Incomplete)
        );
        let ev = play("As 7d Ad 2c Kh Qs 3c 8h 9d", &[(Seat::Zero, Action::Fold)]);
        let evil = HistoryMeta {
            names: ["alice\n*** SUMMARY ***", "bob"],
            ..META
        };
        assert_eq!(
            format_hand(&evil, &ev, Viewer::Audit),
            Err(HistoryError::UnsafeText)
        );
    }

    /// Later hands, blinds all-in, split pots and the match result.
    #[test]
    fn later_hand_with_all_in_blinds_split_and_match_end() {
        let mut m = Match::new(MatchConfig::standard(), Seat::Zero).unwrap();
        // Hand 1: alice (button) shoves, bob calls; royal flush board splits the pot.
        let deck = Deck::stacked(&parse_cards("2c 3c 4d 4h Ah Kh Qh Jh Th").unwrap());
        let mut ev = m.start_hand(deck).unwrap();
        ev.extend(m.act(Seat::Zero, Action::AllIn, Duration::ZERO).unwrap());
        ev.extend(m.act(Seat::One, Action::Call, Duration::ZERO).unwrap());
        let text = format_hand(&META, &ev, Viewer::Spectator).unwrap();
        assert!(
            text.contains("alice collected 1500 from pot\nbob collected 1500 from pot")
                || text.contains("bob collected 1500 from pot\nalice collected 1500 from pot")
        );
        assert!(text.contains("showed [3c 4h] and won (1500) with a Royal Flush"));

        // Hand 2: bob on the button. Bust alice via a blinds-level all-in.
        let deck = Deck::stacked(&parse_cards("2c As 7d Ad Kh Qs 3c 8h 9d").unwrap());
        let mut ev = m.start_hand(deck).unwrap();
        ev.extend(m.act(Seat::One, Action::AllIn, Duration::ZERO).unwrap());
        ev.extend(m.act(Seat::Zero, Action::Call, Duration::ZERO).unwrap());
        let meta = HistoryMeta {
            hand_id: 43,
            ..META
        };
        let text = format_hand(&meta, &ev, Viewer::Player(Seat::One)).unwrap();
        assert!(text.starts_with("Pokerladder Hand #43:"));
        assert!(text.contains("Seat #2 is the button"));
        assert!(text.contains("bob: posts small blind 10\nalice: posts big blind 20"));
        assert!(
            text.contains("alice finished the tournament in 2nd place\nbob wins the tournament")
        );
        assert!(text.contains("alice: calls 1480 and is all-in"));
    }
}
