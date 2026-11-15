//! Cards and the deck.
//!
//! A [`Card`] is a single byte `rank * 4 + suit` in `0..52`. That index is what the
//! hand evaluator's lookup table is keyed on, so it is part of the public contract.

use std::fmt;
use std::str::FromStr;

use rand::CryptoRng;
use rand::seq::SliceRandom;

pub const RANK_CHARS: [char; 13] = [
    '2', '3', '4', '5', '6', '7', '8', '9', 'T', 'J', 'Q', 'K', 'A',
];
pub const SUIT_CHARS: [char; 4] = ['c', 'd', 'h', 's'];

/// Rank index: `0` = deuce … `12` = ace.
pub type Rank = u8;
/// Suit index: `0` = clubs, `1` = diamonds, `2` = hearts, `3` = spades.
pub type Suit = u8;

#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Card(u8);

impl Card {
    /// Panics if `rank > 12` or `suit > 3`.
    pub const fn new(rank: Rank, suit: Suit) -> Self {
        assert!(rank < 13 && suit < 4, "card out of range");
        Card(rank * 4 + suit)
    }

    /// Builds a card from its index in `0..52`. Panics if out of range.
    pub const fn from_index(index: u8) -> Self {
        assert!(index < 52, "card index out of range");
        Card(index)
    }

    #[inline]
    pub const fn index(self) -> u8 {
        self.0
    }

    #[inline]
    pub const fn rank(self) -> Rank {
        self.0 >> 2
    }

    #[inline]
    pub const fn suit(self) -> Suit {
        self.0 & 3
    }

    /// All 52 cards in index order.
    pub fn all() -> impl Iterator<Item = Card> {
        (0..52).map(Card)
    }
}

impl fmt::Display for Card {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}{}",
            RANK_CHARS[self.rank() as usize],
            SUIT_CHARS[self.suit() as usize]
        )
    }
}

impl fmt::Debug for Card {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl TryFrom<u8> for Card {
    type Error = ParseCardError;

    /// Fallible [`Card::from_index`], for indices from untrusted sources.
    fn try_from(index: u8) -> Result<Self, Self::Error> {
        if index < 52 {
            Ok(Card(index))
        } else {
            Err(ParseCardError(format!("index {index}")))
        }
    }
}

/// A set of distinct cards as a 52-bit mask.
#[derive(Copy, Clone, Default, PartialEq, Eq, Hash)]
pub struct CardSet(u64);

impl CardSet {
    pub const EMPTY: CardSet = CardSet(0);

    /// `None` if `cards` contains a duplicate.
    pub fn from_cards(cards: &[Card]) -> Option<CardSet> {
        let mut set = CardSet::EMPTY;
        for &c in cards {
            if !set.insert(c) {
                return None;
            }
        }
        Some(set)
    }

    #[inline]
    pub fn contains(self, c: Card) -> bool {
        self.0 & (1 << c.0) != 0
    }

    /// Adds `c`; returns `false` if it was already present.
    #[inline]
    pub fn insert(&mut self, c: Card) -> bool {
        let fresh = !self.contains(c);
        self.0 |= 1 << c.0;
        fresh
    }

    #[inline]
    pub fn len(self) -> usize {
        self.0.count_ones() as usize
    }

    #[inline]
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn iter(self) -> impl Iterator<Item = Card> {
        Card::all().filter(move |&c| self.contains(c))
    }
}

impl fmt::Debug for CardSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.iter()).finish()
    }
}

/// Panics with a clear message if `cards` repeats a card. Duplicates can only come from
/// an engine bug (cards are dealt from one [`Deck`]), so this is an invariant check.
#[inline]
pub(crate) fn assert_distinct(cards: &[Card]) {
    // Branch-free and popcount-free: the sum of distinct powers of two equals their OR;
    // any repeat makes the sum strictly larger (no overflow: at most 52 * 2^51 < 2^64).
    let (mut or, mut sum) = (0u64, 0u64);
    for c in cards {
        or |= 1 << c.0;
        sum += 1 << c.0;
    }
    if or != sum {
        duplicate_card(cards);
    }
}

#[cold]
#[inline(never)]
pub(crate) fn duplicate_card(cards: &[Card]) -> ! {
    panic!("duplicate card in {cards:?}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseCardError(pub String);

impl fmt::Display for ParseCardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid card: {:?}", self.0)
    }
}

impl std::error::Error for ParseCardError {}

impl FromStr for Card {
    type Err = ParseCardError;

    /// Parses `"As"`, `"Td"`, `"2c"`, … (rank case-insensitive, suit lowercase or upper).
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || ParseCardError(s.to_owned());
        let mut chars = s.chars();
        let (Some(r), Some(su), None) = (chars.next(), chars.next(), chars.next()) else {
            return Err(err());
        };
        let rank = RANK_CHARS
            .iter()
            .position(|&c| c == r.to_ascii_uppercase())
            .ok_or_else(err)?;
        let suit = SUIT_CHARS
            .iter()
            .position(|&c| c == su.to_ascii_lowercase())
            .ok_or_else(err)?;
        Ok(Card::new(rank as u8, suit as u8))
    }
}

/// Parses a whitespace-separated or concatenated list such as `"As Kd"` or `"AsKd"`.
pub fn parse_cards(s: &str) -> Result<Vec<Card>, ParseCardError> {
    let compact: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if !compact.len().is_multiple_of(2) {
        return Err(ParseCardError(s.to_owned()));
    }
    compact
        .as_bytes()
        .chunks(2)
        .map(|pair| {
            std::str::from_utf8(pair)
                .map_err(|_| ParseCardError(s.to_owned()))?
                .parse()
        })
        .collect()
}

/// A shuffled 52-card deck dealt from the top.
#[derive(Clone)]
pub struct Deck {
    cards: [Card; 52],
    next: usize,
}

impl Deck {
    /// Shuffles a fresh deck. The RNG must be cryptographically secure: a predictable
    /// shuffle is a predictable game. In production pass `rand::rng()` (OS-seeded
    /// ChaCha); in tests a seeded `StdRng` gives reproducible deals.
    pub fn shuffled<R: CryptoRng + ?Sized>(rng: &mut R) -> Self {
        let mut cards = [Card(0); 52];
        for (i, c) in cards.iter_mut().enumerate() {
            *c = Card(i as u8);
        }
        cards.shuffle(rng);
        Deck { cards, next: 0 }
    }

    /// Deals the next card. Panics if the deck is exhausted (heads-up uses at most 9).
    pub fn deal(&mut self) -> Card {
        let c = self.cards[self.next];
        self.next += 1;
        c
    }

    pub fn remaining(&self) -> usize {
        52 - self.next
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    #[test]
    fn index_rank_suit_roundtrip() {
        for c in Card::all() {
            assert_eq!(Card::new(c.rank(), c.suit()), c);
            assert_eq!(c.to_string().parse::<Card>().unwrap(), c);
        }
    }

    #[test]
    fn parse_examples() {
        assert_eq!("As".parse::<Card>().unwrap(), Card::new(12, 3));
        assert_eq!("2c".parse::<Card>().unwrap(), Card::new(0, 0));
        assert_eq!("td".parse::<Card>().unwrap(), Card::new(8, 1));
        assert!("1s".parse::<Card>().is_err());
        assert!("Asx".parse::<Card>().is_err());
        assert_eq!(parse_cards("As Kd").unwrap(), parse_cards("AsKd").unwrap());
    }

    #[test]
    fn card_set_detects_duplicates() {
        let cards = parse_cards("As Kd 2c").unwrap();
        let set = CardSet::from_cards(&cards).unwrap();
        assert_eq!(set.len(), 3);
        assert!(set.contains("Kd".parse().unwrap()));
        assert_eq!(set.iter().count(), 3);
        assert!(CardSet::from_cards(&parse_cards("As Kd As").unwrap()).is_none());
        assert!(Card::try_from(52).is_err());
        assert!(parse_cards("Aé1").is_err());
    }

    #[test]
    fn deck_is_a_permutation() {
        let mut deck = Deck::shuffled(&mut StdRng::seed_from_u64(7));
        let mut seen = [false; 52];
        for _ in 0..52 {
            let c = deck.deal();
            assert!(!seen[c.index() as usize], "duplicate {c}");
            seen[c.index() as usize] = true;
        }
        assert_eq!(deck.remaining(), 0);
    }
}
