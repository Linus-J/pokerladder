//! Chip-dumping heuristics over the audit log, as a report for a person to review
//! before any payout. Nothing here bans or changes anything: every flag is a lead, and
//! each comes with hand ids to read in `hands.history_audit`.
//!
//! Signals, for each ordered pair "P may be dumping chips to O" in one season:
//!
//! - **Strong folds**: of the hands where P faced a bet holding a strong hand (see
//!   [`hand::Strength`]), how many P folded. Compared with P's own rate against
//!   everyone else.
//! - **Weak big commitments**: of the hands where P put half their stack or more in,
//!   how many with a weak hand. Compared with P's own rate against everyone else, so a
//!   player who shoves light against everybody is not flagged for it.
//! - **Results**: P's rated wins against O versus what the ratings before each match
//!   predicted.
//!
//! And for each pair: how often its matches were very short, and how often it met.
//!
//! Baselines are P's rate against others, shrunk towards the population rate (without
//! the pair under test; a Jeffreys estimate, never exactly 0) by [`PRIOR_WEIGHT`]
//! pseudo-observations, so a player with few hands against others, as a second account
//! would have, is compared with the population. The baseline is a Beta distribution
//! ([`stats::Baseline`]) and counts are tested with an exact beta-binomial tail, so a
//! baseline resting on few hands isn't treated as exact. A flag needs [`MIN_EVENTS`]
//! events and a tail below [`P_FLAG`]. Hands are treated as independent, which they aren't quite (the same
//! style repeats within a match), so p-values rank leads; they aren't proof.

pub mod hand;
pub mod load;
pub mod stats;

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;

use pokerladder_engine::Card;
use pokerladder_engine::eval::equity;

use crate::protocol::{MatchId, UserId, WireAction, WireEvent};
use hand::{HandFacts, Spot};

/// A binomial tail below this flags a signal.
pub const P_FLAG: f64 = 0.001;
/// Fewest suspicious events (hands or matches) for a flag.
pub const MIN_EVENTS: u32 = 3;
/// Pseudo-observations of the population rate added to each player's baseline.
pub const PRIOR_WEIGHT: f64 = 10.0;
/// A finished match of at most this many hands is "short".
pub const SHORT_MATCH_HANDS: i32 = 5;
/// Rated results this far below the ratings' prediction (in standard deviations) flag.
pub const RESULT_Z_FLAG: f64 = -3.0;
/// A pair that met at least this many times, making up at least
/// [`FREQUENT_SHARE`] of either player's matches, gets a note in the report.
pub const FREQUENT_MATCHES: u32 = 10;
pub const FREQUENT_SHARE: f64 = 0.5;
/// Example hands listed per signal.
pub const EVIDENCE_PER_SIGNAL: usize = 5;

/// One match of the season, as stored.
#[derive(Clone, Debug)]
pub struct MatchInfo {
    pub id: MatchId,
    pub players: [UserId; 2],
    /// `finished`, `abandoned` or `in_progress`.
    pub status: String,
    pub winner_seat: Option<i16>,
    pub hands_played: Option<i32>,
    pub rated: bool,
    /// Season ratings when the match started (rated, finished matches only).
    pub ratings_before: Option<[f64; 2]>,
}

impl MatchInfo {
    /// For a finished match, whether it was short.
    fn short(&self) -> Option<bool> {
        match (self.status.as_str(), self.hands_played) {
            ("finished", Some(h)) if self.winner_seat.is_some() => Some(h <= SHORT_MATCH_HANDS),
            _ => None,
        }
    }
}

/// A rate of events out of opportunities.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Count {
    pub k: u32,
    pub n: u32,
}

impl Count {
    fn add(&mut self, event: bool) {
        self.n += 1;
        self.k += event as u32;
    }

    fn plus(self, o: Count) -> Count {
        Count {
            k: self.k + o.k,
            n: self.n + o.n,
        }
    }

    fn minus(self, o: Count) -> Count {
        Count {
            k: self.k - o.k,
            n: self.n - o.n,
        }
    }

    pub fn rate(self) -> f64 {
        if self.n == 0 {
            0.0
        } else {
            self.k as f64 / self.n as f64
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SignalKind {
    StrongFolds,
    WeakCommits,
}

/// A hand cited as evidence.
#[derive(Clone, Debug, PartialEq)]
pub struct Evidence {
    pub hand_id: i64,
    pub match_id: MatchId,
    pub number: i32,
    pub spot: Spot,
    /// Chips the player won (positive) or lost in the hand.
    pub net: i64,
}

#[derive(Default)]
struct Directed {
    strong: Count,
    weak: Count,
    /// Hands where the server folded a strong hand for a timed-out player.
    strong_timeouts: u32,
    strong_evidence: Vec<Evidence>,
    weak_evidence: Vec<Evidence>,
}

/// Collects a season's matches and hands, then builds the [`Report`].
#[derive(Default)]
pub struct Analyzer {
    matches: HashMap<MatchId, MatchInfo>,
    names: HashMap<UserId, String>,
    /// Keyed by (player, opponent).
    directed: HashMap<(UserId, UserId), Directed>,
    hands: u64,
    skipped: Vec<(i64, String)>,
}

impl Analyzer {
    pub fn new() -> Analyzer {
        Analyzer::default()
    }

    pub fn add_name(&mut self, id: UserId, name: String) {
        self.names.insert(id, name);
    }

    /// Matches must be added before their hands.
    pub fn add_match(&mut self, m: MatchInfo) {
        self.matches.insert(m.id, m);
    }

    /// Lists a hand that couldn't be read in the report.
    pub fn skip(&mut self, hand_id: i64, why: String) {
        self.skipped.push((hand_id, why));
    }

    /// Adds one stored hand. Malformed or orphaned hands are skipped and listed in the
    /// report rather than failing it.
    pub fn add_hand(&mut self, hand_id: i64, match_id: MatchId, number: i32, events: &[WireEvent]) {
        let Some(m) = self.matches.get(&match_id) else {
            self.skipped
                .push((hand_id, "match not in this season".into()));
            return;
        };
        let players = m.players;
        let facts = match hand::analyse(events) {
            Ok(f) => f,
            Err(e) => {
                self.skipped.push((hand_id, e.0));
                return;
            }
        };
        self.hands += 1;
        self.record(hand_id, match_id, number, players, facts);
    }

    fn record(
        &mut self,
        hand_id: i64,
        match_id: MatchId,
        number: i32,
        players: [UserId; 2],
        facts: HandFacts,
    ) {
        for (s, seat) in facts.seats.into_iter().enumerate() {
            let d = self
                .directed
                .entry((players[s], players[1 - s]))
                .or_default();
            let evidence = |spot: Spot| Evidence {
                hand_id,
                match_id,
                number,
                spot,
                net: seat.net,
            };
            d.strong_timeouts += seat.strong_timeout as u32;
            if seat.strong_faced {
                d.strong.add(seat.strong_fold.is_some());
                if let Some(spot) = seat.strong_fold.clone() {
                    d.strong_evidence.push(evidence(spot));
                }
            }
            if let Some(spot) = seat.big_commit.clone() {
                let weak = spot.strength.is_weak();
                d.weak.add(weak);
                if weak {
                    d.weak_evidence.push(evidence(spot));
                }
            }
        }
    }

    pub fn finish(self, season: &str) -> Report {
        // Per-player totals over all opponents, and population totals.
        let mut player_total: HashMap<UserId, [Count; 2]> = HashMap::new();
        let mut population = [Count::default(); 2];
        for (&(p, _), d) in &self.directed {
            let t = player_total.entry(p).or_default();
            t[0] = t[0].plus(d.strong);
            t[1] = t[1].plus(d.weak);
            population[0] = population[0].plus(d.strong);
            population[1] = population[1].plus(d.weak);
        }

        // Matches per pair and per player.
        let mut pairs: BTreeMap<(UserId, UserId), Vec<&MatchInfo>> = BTreeMap::new();
        let mut per_player: HashMap<UserId, u32> = HashMap::new();
        let mut short_population = Count::default();
        let mut short_player: HashMap<UserId, Count> = HashMap::new();
        for m in self.matches.values() {
            let key = (
                m.players[0].min(m.players[1]),
                m.players[0].max(m.players[1]),
            );
            pairs.entry(key).or_default().push(m);
            for p in m.players {
                *per_player.entry(p).or_default() += 1;
            }
            if let Some(short) = m.short() {
                short_population.add(short);
                for p in m.players {
                    short_player.entry(p).or_default().add(short);
                }
            }
        }

        let name = |id: UserId| {
            self.names
                .get(&id)
                .cloned()
                .unwrap_or_else(|| format!("user#{id}"))
        };
        let mut reports = Vec::new();
        for (&(a, b), ms) in &pairs {
            let mut r = PairReport {
                players: [a, b],
                names: [name(a), name(b)],
                matches: ms.len() as u32,
                rated: ms
                    .iter()
                    .filter(|m| m.rated && m.status == "finished")
                    .count() as u32,
                wins: [0, 0],
                short: Count::default(),
                short_p: 1.0,
                signals: Vec::new(),
                results: Vec::new(),
                flags: Vec::new(),
                frequent: false,
                strong_timeouts: [0, 0],
            };
            for m in ms {
                if let (Some(short), Some(w)) = (m.short(), m.winner_seat) {
                    let winner = m.players[w as usize];
                    r.wins[(winner == b) as usize] += 1;
                    r.short.add(short);
                }
            }
            // Tested against whichever player plays short matches more often against
            // everyone else: someone who shoves every hand explains their own short
            // matches.
            let prior = stats::prior_rate(short_population.minus(r.short));
            r.short_p = [a, b]
                .map(|p| {
                    let others = short_player
                        .get(&p)
                        .copied()
                        .unwrap_or_default()
                        .minus(r.short);
                    stats::Baseline::new(others, prior, PRIOR_WEIGHT)
                        .upper_tail(r.short.k, r.short.n)
                })
                .into_iter()
                .fold(0.0, f64::max);
            if r.short.k >= MIN_EVENTS && r.short_p < P_FLAG {
                r.flags.push(Flag::ShortMatches);
            }
            r.frequent = r.matches >= FREQUENT_MATCHES
                && [a, b]
                    .iter()
                    .any(|p| r.matches as f64 >= FREQUENT_SHARE * per_player[p] as f64);

            for (i, (p, o)) in [(a, b), (b, a)].into_iter().enumerate() {
                let empty = Directed::default();
                let d = self.directed.get(&(p, o)).unwrap_or(&empty);
                let total = player_total.get(&p).copied().unwrap_or_default();
                r.strong_timeouts[i] = d.strong_timeouts;
                for (j, kind) in [SignalKind::StrongFolds, SignalKind::WeakCommits]
                    .into_iter()
                    .enumerate()
                {
                    let (count, evidence) = match kind {
                        SignalKind::StrongFolds => (d.strong, &d.strong_evidence),
                        SignalKind::WeakCommits => (d.weak, &d.weak_evidence),
                    };
                    let others = total[j].minus(count);
                    // The prior leaves out the play under test, so a blatant dumper
                    // in a small ladder can't raise the rate they're compared with.
                    let prior = stats::prior_rate(population[j].minus(count));
                    let base = stats::Baseline::new(others, prior, PRIOR_WEIGHT);
                    let baseline = base.mean();
                    let p_value = base.upper_tail(count.k, count.n);
                    let flagged = count.k >= MIN_EVENTS && p_value < P_FLAG;
                    if flagged {
                        r.flags.push(Flag::Signal(i, kind));
                    }
                    let mut evidence = evidence.clone();
                    // Most telling first: strongest folds, weakest commitments.
                    evidence.sort_by(|x, y| match kind {
                        SignalKind::StrongFolds => {
                            y.spot.strength.share.total_cmp(&x.spot.strength.share)
                        }
                        SignalKind::WeakCommits => {
                            x.spot.strength.share.total_cmp(&y.spot.strength.share)
                        }
                    });
                    evidence.truncate(EVIDENCE_PER_SIGNAL);
                    r.signals.push(Signal {
                        player: i,
                        kind,
                        count,
                        others,
                        baseline,
                        p_value,
                        flagged,
                        evidence,
                    });
                }

                // Rated results from p's side.
                let mut games = Vec::new();
                for m in ms {
                    let (Some(w), Some(before), true) = (m.winner_seat, m.ratings_before, m.rated)
                    else {
                        continue;
                    };
                    if m.status != "finished" {
                        continue;
                    }
                    let seat = (m.players[1] == p) as usize;
                    let expected = stats::expected_score(before[seat], before[1 - seat]);
                    games.push((expected, w as usize == seat));
                }
                let res = stats::results(&games);
                if res.matches >= MIN_EVENTS && res.z <= RESULT_Z_FLAG {
                    r.flags.push(Flag::Results(i));
                }
                r.results.push(res);
            }
            reports.push(r);
        }
        // Most flags first, then the strongest single signal.
        reports.sort_by(|x, y| {
            y.flags
                .len()
                .cmp(&x.flags.len())
                .then(x.min_p().total_cmp(&y.min_p()))
                .then(x.players.cmp(&y.players))
        });

        let mut skipped = self.skipped;
        skipped.sort();
        Report {
            season: season.to_owned(),
            hands: self.hands,
            matches: self.matches.len() as u32,
            players: per_player.len() as u32,
            population_strong: population[0],
            population_weak: population[1],
            population_short: short_population,
            pairs: reports,
            skipped,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Flag {
    /// The player at this index of the pair, for this signal.
    Signal(usize, SignalKind),
    /// The player at this index lost far more than ratings predicted.
    Results(usize),
    ShortMatches,
}

#[derive(Clone, Debug)]
pub struct Signal {
    /// Index into the pair of the player whose behaviour this measures.
    pub player: usize,
    pub kind: SignalKind,
    /// Against the other player of the pair.
    pub count: Count,
    /// Against everyone else.
    pub others: Count,
    /// The rate `count` is tested against.
    pub baseline: f64,
    pub p_value: f64,
    pub flagged: bool,
    pub evidence: Vec<Evidence>,
}

#[derive(Clone, Debug)]
pub struct PairReport {
    /// Ordered by user id.
    pub players: [UserId; 2],
    pub names: [String; 2],
    /// All matches this season, whatever their status.
    pub matches: u32,
    /// Rated, finished matches.
    pub rated: u32,
    /// Finished matches won, by player index.
    pub wins: [u32; 2],
    /// Finished matches of at most [`SHORT_MATCH_HANDS`] hands.
    pub short: Count,
    pub short_p: f64,
    pub signals: Vec<Signal>,
    /// Rated results, by player index.
    pub results: Vec<stats::Results>,
    pub flags: Vec<Flag>,
    /// Met at least [`FREQUENT_MATCHES`] times, making up at least
    /// [`FREQUENT_SHARE`] of either player's matches. Shown, but not a flag on its
    /// own: in a small ladder, matchmaking alone pairs the same players often.
    pub frequent: bool,
    /// By player index: hands where they timed out (or sat out) facing a bet with a
    /// strong hand and the server folded it. Not part of the strong-fold signal; shown
    /// so a reviewer can see whether sitting out was used to lose chips.
    pub strong_timeouts: [u32; 2],
}

impl PairReport {
    pub fn signal(&self, player: usize, kind: SignalKind) -> &Signal {
        self.signals
            .iter()
            .find(|s| s.player == player && s.kind == kind)
            .expect("every pair has both signals for both players")
    }

    fn min_p(&self) -> f64 {
        self.signals
            .iter()
            .map(|s| s.p_value)
            .chain([self.short_p])
            .fold(1.0, f64::min)
    }
}

#[derive(Clone, Debug)]
pub struct Report {
    pub season: String,
    pub hands: u64,
    pub matches: u32,
    pub players: u32,
    pub population_strong: Count,
    pub population_weak: Count,
    pub population_short: Count,
    /// Every pair that met, flagged pairs first.
    pub pairs: Vec<PairReport>,
    /// Hands that couldn't be read: (hand id, reason).
    pub skipped: Vec<(i64, String)>,
}

impl Report {
    pub fn pair(&self, a: UserId, b: UserId) -> Option<&PairReport> {
        let key = [a.min(b), a.max(b)];
        self.pairs.iter().find(|p| p.players == key)
    }

    pub fn flagged(&self) -> impl Iterator<Item = &PairReport> {
        self.pairs.iter().filter(|p| !p.flags.is_empty())
    }

    /// Plain-text report. `all` also lists pairs without flags.
    pub fn render(&self, all: bool) -> String {
        let mut s = String::new();
        let pct = |c: Count| format!("{:.1}% of {}", 100.0 * c.rate(), c.n);
        let _ = writeln!(s, "Integrity report, season {}", self.season);
        let _ = writeln!(
            s,
            "{} hands in {} matches between {} players.",
            self.hands, self.matches, self.players
        );
        let _ = writeln!(s, "Everyone, for comparison:");
        let _ = writeln!(
            s,
            "  folded a strong hand when facing a bet: {}",
            pct(self.population_strong)
        );
        let _ = writeln!(
            s,
            "  big commitments made with a weak hand:  {}",
            pct(self.population_weak)
        );
        let _ = writeln!(
            s,
            "  finished matches of {SHORT_MATCH_HANDS} hands or fewer:   {}",
            pct(self.population_short)
        );
        if !self.skipped.is_empty() {
            let _ = writeln!(s, "Skipped {} unreadable hands:", self.skipped.len());
            for (id, why) in self.skipped.iter().take(10) {
                let _ = writeln!(s, "  hand {id}: {why}");
            }
        }
        let flagged = self.flagged().count();
        let _ = writeln!(s);
        if flagged == 0 {
            let _ = writeln!(s, "No pair was flagged.");
        } else {
            let _ = writeln!(
                s,
                "{flagged} pair(s) flagged. These are leads for a person to review, not proof."
            );
            let _ = writeln!(
                s,
                "Read a hand with: SELECT history_audit FROM hands WHERE id = <hand>;"
            );
        }
        for (i, p) in self.pairs.iter().enumerate() {
            if p.flags.is_empty() && !all {
                continue;
            }
            let _ = writeln!(s);
            p.render(&mut s, i + 1);
        }
        s
    }
}

fn cards(cs: &[Card]) -> String {
    cs.iter().map(Card::to_string).collect::<Vec<_>>().join(" ")
}

fn action_word(a: WireAction) -> String {
    match a {
        WireAction::Fold => "folded".into(),
        WireAction::Check => "checked".into(),
        WireAction::Call => "called".into(),
        WireAction::RaiseTo { amount } => format!("raised to {amount}"),
        WireAction::AllIn => "went all in".into(),
    }
}

impl PairReport {
    fn render(&self, s: &mut String, rank: usize) {
        let [a, b] = &self.names;
        let mark = if self.flags.is_empty() {
            ""
        } else {
            "FLAGGED "
        };
        let _ = writeln!(s, "#{rank} {mark}{a} vs {b}");
        let _ = writeln!(
            s,
            "  {} matches ({} rated); finished: {a} won {}, {b} won {}",
            self.matches, self.rated, self.wins[0], self.wins[1]
        );
        let bang = |f: bool| if f { "!" } else { " " };
        for i in 0..2 {
            let (me, them) = (&self.names[i], &self.names[1 - i]);
            for kind in [SignalKind::StrongFolds, SignalKind::WeakCommits] {
                let sig = self.signal(i, kind);
                if sig.count.n == 0 {
                    continue;
                }
                let what = match kind {
                    SignalKind::StrongFolds => "folded a strong hand facing a bet",
                    SignalKind::WeakCommits => "made big commitments with a weak hand",
                };
                let _ = writeln!(
                    s,
                    "  {} {me} {what} against {them} in {}/{} hands \
                     (expected {:.1}%; {} vs others {}/{}; p={})",
                    bang(sig.flagged),
                    sig.count.k,
                    sig.count.n,
                    100.0 * sig.baseline,
                    me,
                    sig.others.k,
                    sig.others.n,
                    stats::fmt_p(sig.p_value),
                );
            }
            if self.strong_timeouts[i] > 0 {
                let _ = writeln!(
                    s,
                    "    {me} timed out or sat out facing a bet with a strong hand against {them} \
                     in {} hands (folded by the server; not counted above)",
                    self.strong_timeouts[i]
                );
            }
            let r = &self.results[i];
            if r.matches > 0 {
                let flagged = self.flags.contains(&Flag::Results(i));
                let _ = writeln!(
                    s,
                    "  {} {me} won {}/{} rated matches against {them}; ratings predicted {:.1} (z={:.1})",
                    bang(flagged),
                    r.wins,
                    r.matches,
                    r.expected,
                    r.z
                );
            }
        }
        if self.short.n > 0 {
            let _ = writeln!(
                s,
                "  {} {}/{} finished matches lasted {SHORT_MATCH_HANDS} hands or fewer (p={})",
                bang(self.flags.contains(&Flag::ShortMatches)),
                self.short.k,
                self.short.n,
                stats::fmt_p(self.short_p)
            );
        }
        if self.frequent {
            let _ = writeln!(
                s,
                "    note: met {} times, at least half of one player's matches this season",
                self.matches
            );
        }
        for sig in &self.signals {
            if !sig.flagged {
                continue;
            }
            let me = &self.names[sig.player];
            let them = &self.names[1 - sig.player];
            let _ = writeln!(s, "  Example hands ({me}):");
            for e in &sig.evidence {
                let sp = &e.spot;
                let strength = match sp.strength.top {
                    Some(top) => format!(
                        "top {:.0}% of starting hands",
                        (100.0 * top).ceil().max(1.0)
                    ),
                    None => format!("{:.0}% against a random hand", 100.0 * sp.strength.share),
                };
                let vs = equity::vs_hand(sp.hole, sp.opponent_hole, &sp.board).share();
                let board = if sp.board.is_empty() {
                    String::new()
                } else {
                    format!(" on [{}]", cards(&sp.board))
                };
                let result = if e.net < 0 {
                    format!("lost {}", -e.net)
                } else if e.net > 0 {
                    format!("won {}", e.net)
                } else {
                    "broke even".into()
                };
                let _ = writeln!(
                    s,
                    "    hand {} (match {} #{}): {} {} {}{board} facing {} into {}; {strength}, \
                     {:.0}% against {them}'s {}; {result}",
                    e.hand_id,
                    e.match_id,
                    e.number,
                    action_word(sp.action),
                    cards(&sp.hole),
                    sp.street.name(),
                    sp.to_call,
                    sp.pot,
                    100.0 * vs,
                    cards(&sp.opponent_hole),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finished(
        id: MatchId,
        players: [UserId; 2],
        winner: i16,
        hands: i32,
        r: [f64; 2],
    ) -> MatchInfo {
        MatchInfo {
            id,
            players,
            status: "finished".into(),
            winner_seat: Some(winner),
            hands_played: Some(hands),
            rated: true,
            ratings_before: Some(r),
        }
    }

    /// Players 1..=6 play long, even matches; but 6 (rated 1600) loses six short
    /// matches in a row to 5 (rated 1400), from either seat. Both match-level signals
    /// must flag 6, with the expectation computed from 6's own rating.
    #[test]
    fn results_and_short_matches_flag() {
        let mut a = Analyzer::new();
        let mut id = 0;
        for p in 1..=6 {
            for q in p + 1..=6 {
                if (p, q) == (5, 6) {
                    continue;
                }
                for w in 0..2 {
                    id += 1;
                    a.add_match(finished(id, [p, q], w, 40, [1500.0, 1500.0]));
                }
            }
        }
        for k in 0..6 {
            id += 1;
            a.add_match(if k % 2 == 0 {
                finished(id, [5, 6], 0, 3, [1400.0, 1600.0])
            } else {
                finished(id, [6, 5], 1, 3, [1600.0, 1400.0])
            });
        }
        let report = a.finish("2026-11");
        let pair = report.pair(5, 6).unwrap();
        assert_eq!(pair.wins, [6, 0]);
        // Index 1 is user 6.
        let r = pair.results[1];
        let e = stats::expected_score(1600.0, 1400.0);
        assert_eq!((r.matches, r.wins), (6, 0));
        assert!((r.expected - 6.0 * e).abs() < 1e-9, "{}", r.expected);
        assert!(pair.flags.contains(&Flag::Results(1)), "{:?}", pair.flags);
        assert!(!pair.flags.contains(&Flag::Results(0)));
        assert_eq!(pair.short, Count { k: 6, n: 6 });
        assert!(pair.flags.contains(&Flag::ShortMatches), "{:?}", pair.flags);
        assert_eq!(report.flagged().count(), 1, "{}", report.render(false));
    }

    /// A small season where nobody else ever folded a strong hand: three strong
    /// folds out of ten against one opponent must not get p = 0 (and a flag) just
    /// because the rest of the population's rate is 0/n.
    #[test]
    fn an_empty_population_rate_does_not_make_events_impossible() {
        let mut a = Analyzer::new();
        a.add_match(finished(1, [1, 2], 0, 40, [1500.0, 1500.0]));
        a.add_match(finished(2, [1, 3], 0, 40, [1500.0, 1500.0]));
        let with = |k, n| Directed {
            strong: Count { k, n },
            ..Directed::default()
        };
        a.directed.insert((2, 1), with(3, 10));
        a.directed.insert((1, 3), with(0, 30));
        let report = a.finish("2026-11");
        let sig = report
            .pair(1, 2)
            .unwrap()
            .signal(1, SignalKind::StrongFolds);
        assert_eq!(sig.count, Count { k: 3, n: 10 });
        assert!(sig.baseline > 0.0 && sig.p_value > P_FLAG, "{sig:?}");
        assert_eq!(report.flagged().count(), 0);
    }

    /// Short matches are judged against the shorter-playing of the two: a player
    /// whose matches are all short explains a pair's short matches.
    #[test]
    fn a_player_who_always_plays_short_matches_explains_them() {
        let mut a = Analyzer::new();
        let mut id = 0;
        for p in 1..=6 {
            for q in p + 1..=6 {
                // Enough meetings per pair (4) for a flag to be possible.
                for w in [0, 1, 0, 1] {
                    id += 1;
                    // Player 6 shoves everything: all their matches are short.
                    let hands = if q == 6 { 3 } else { 40 };
                    a.add_match(finished(id, [p, q], w, hands, [1500.0, 1500.0]));
                }
            }
        }
        let report = a.finish("2026-11");
        assert_eq!(report.flagged().count(), 0, "{}", report.render(false));
        assert_eq!(report.pair(5, 6).unwrap().short, Count { k: 4, n: 4 });
    }
}
