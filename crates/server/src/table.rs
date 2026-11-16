//! The match actor: one tokio task per match owns the [`Match`] and is the only thing
//! that touches it. Connections send it [`MatchCmd`]s; it sends each player only the
//! events they may see, runs the decision clock, persists every hand and finally the
//! result.
//!
//! Clock rules (see `CLAUDE.md`): think time is measured from when a decision is
//! offered and never reset by rejected actions; the deadline is
//! [`Match::time_allowed`]. A player who is disconnected when their deadline passes is
//! *sitting out*: until they reconnect, their later decisions time out (check/fold)
//! after a short delay instead of the full clock, so an abandoned match finishes
//! quickly — and the absent player never wins by being absent.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use pokerladder_engine::history::{HistoryMeta, Viewer, format_hand};
use pokerladder_engine::holdem::{Action, Seat, Visibility};
use pokerladder_engine::{Deck, Match, MatchConfig, MatchEvent};
use time::OffsetDateTime;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::protocol::{
    ErrorCode, HandSnapshot, MatchId, MatchSnapshot, RatingChange, ServerMsg, TurnInfo, WireEvent,
    millis, seat_u8,
};

/// Site name in hand-history headers.
pub const SITE: &str = "Pokerladder";

/// A player's live connection, as seen by a match.
#[derive(Clone)]
pub struct Conn {
    /// Increases with every new connection, so a stale `Connect` can't displace a
    /// newer one whatever order messages arrive in.
    pub id: u64,
    pub tx: mpsc::Sender<ServerMsg>,
    /// Cancelled to close the socket (e.g. it can't keep up and missed messages; the
    /// client reconnects and gets a fresh snapshot).
    pub kill: CancellationToken,
}

impl Conn {
    /// Queues `msg`; a full or closed queue closes the connection.
    pub fn send(&self, msg: ServerMsg) -> bool {
        if self.tx.try_send(msg).is_ok() {
            true
        } else {
            self.kill.cancel();
            false
        }
    }
}

pub enum MatchCmd {
    Connect { seat: Seat, conn: Conn },
    Disconnect { seat: Seat, conn_id: u64 },
    Act { seat: Seat, action: Action },
    SetAutoMuck { seat: Seat, on: bool },
}

#[derive(Copy, Clone, Debug)]
pub struct Timing {
    /// Pause after a hand (to show the result) and before the first hand.
    pub between_hands: Duration,
    /// How long a sitting-out player's decisions take to time out.
    pub sit_out_delay: Duration,
}

impl Timing {
    pub const STANDARD: Timing = Timing {
        between_hands: Duration::from_secs(3),
        sit_out_delay: Duration::from_secs(1),
    };
}

pub struct MatchSetup {
    pub match_id: MatchId,
    /// Indexed by seat.
    pub names: [String; 2],
    pub ratings: [f64; 2],
    pub config: MatchConfig,
    pub first_button: Seat,
    pub timing: Timing,
}

/// A finished hand, with its complete unfiltered events (`HandStarted` .. `Ended`, plus
/// `MatchOver` if it was the last).
pub struct HandRecord {
    pub match_id: MatchId,
    pub number: u32,
    pub started_at: OffsetDateTime,
    pub names: [String; 2],
    pub events: Vec<MatchEvent>,
}

pub struct MatchFinish {
    pub match_id: MatchId,
    pub winner: Seat,
    pub hands_played: u32,
}

/// Where a match writes its results. Implemented by the Postgres store; tests use an
/// in-memory one.
pub trait MatchStore: Send + Sync + 'static {
    fn record_hand(&self, hand: &HandRecord) -> impl Future<Output = anyhow::Result<()>> + Send;
    /// Marks the match finished and updates both ratings in one transaction.
    fn finish_match(
        &self,
        result: &MatchFinish,
    ) -> impl Future<Output = anyhow::Result<[RatingChange; 2]>> + Send;
}

/// Renders a hand's histories for seat 0, seat 1 and the audit view.
pub fn render_histories(hand_id: u64, rec: &HandRecord) -> anyhow::Result<[String; 3]> {
    let started = rec.started_at.to_offset(time::UtcOffset::UTC).format(
        time::macros::format_description!("[year]/[month]/[day] [hour]:[minute]:[second]"),
    )?;
    let meta = HistoryMeta {
        site: SITE,
        hand_id,
        match_id: rec.match_id as u64,
        started_utc: &started,
        names: [&rec.names[0], &rec.names[1]],
    };
    let render = |v| format_hand(&meta, &rec.events, v);
    Ok([
        render(Viewer::Player(Seat::Zero))?,
        render(Viewer::Player(Seat::One))?,
        render(Viewer::Audit)?,
    ])
}

pub struct MatchOutcome {
    pub winner: Seat,
    pub ratings: Option<[RatingChange; 2]>,
}

struct Table<S> {
    setup: MatchSetup,
    m: Match,
    store: Arc<S>,
    conns: [Option<Conn>; 2],
    sitting_out: [bool; 2],
    /// When the current decision was offered, and its deadline.
    offered_at: Option<Instant>,
    deadline: Option<Instant>,
    next_hand_at: Option<Instant>,
    /// The current hand's unfiltered events.
    hand_log: Vec<MatchEvent>,
    hand_started_at: OffsetDateTime,
}

/// Plays the match to the end, then records the result. Returns once the result is
/// recorded (or recording has failed for good).
pub async fn run<S: MatchStore>(
    setup: MatchSetup,
    store: Arc<S>,
    mut rx: mpsc::Receiver<MatchCmd>,
) -> MatchOutcome {
    let m = Match::new(setup.config.clone(), setup.first_button)
        .expect("match config was validated when the server started");
    let mut t = Table {
        next_hand_at: Some(Instant::now() + setup.timing.between_hands),
        setup,
        m,
        store,
        conns: [None, None],
        sitting_out: [false; 2],
        offered_at: None,
        deadline: None,
        hand_log: Vec::new(),
        hand_started_at: OffsetDateTime::now_utc(),
    };
    let mut rx_open = true;
    while t.m.winner().is_none() {
        tokio::select! {
            cmd = rx.recv(), if rx_open => match cmd {
                Some(cmd) => t.handle(cmd).await,
                // Nobody can reach the match any more; it plays out on its timers.
                None => rx_open = false,
            },
            () = sleep_until(t.deadline) => t.on_deadline().await,
            () = sleep_until(t.next_hand_at) => t.start_hand().await,
        }
    }
    t.finish().await
}

async fn sleep_until(at: Option<Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

fn may_see(vis: Visibility, seat: Seat) -> bool {
    match vis {
        Visibility::Public => true,
        Visibility::Private(s) => s == seat,
    }
}

impl<S: MatchStore> Table<S> {
    fn match_id(&self) -> MatchId {
        self.setup.match_id
    }

    fn send(&mut self, seat: Seat, msg: ServerMsg) {
        if let Some(conn) = &self.conns[seat.index()]
            && !conn.send(msg)
        {
            self.conns[seat.index()] = None;
        }
    }

    async fn handle(&mut self, cmd: MatchCmd) {
        match cmd {
            MatchCmd::Connect { seat, conn } => {
                let slot = &mut self.conns[seat.index()];
                if conn.kill.is_cancelled() || slot.as_ref().is_some_and(|c| c.id > conn.id) {
                    return; // stale or already closed
                }
                *slot = Some(conn);
                let was_sitting_out = std::mem::take(&mut self.sitting_out[seat.index()]);
                if was_sitting_out {
                    self.reset_deadline();
                }
                let snapshot = self.snapshot(seat);
                self.send(seat, ServerMsg::MatchState(Box::new(snapshot)));
                // The opponent's clock display changes if this player's deadline did.
                if was_sitting_out && let Some(turn) = self.turn_info() {
                    let match_id = self.match_id();
                    self.send(seat.other(), ServerMsg::Turn { match_id, turn });
                }
            }
            MatchCmd::Disconnect { seat, conn_id } => {
                let slot = &mut self.conns[seat.index()];
                if slot.as_ref().is_some_and(|c| c.id == conn_id) {
                    *slot = None;
                }
            }
            MatchCmd::SetAutoMuck { seat, on } => self.m.set_auto_muck(seat, on),
            MatchCmd::Act { seat, action } => {
                let now = Instant::now();
                let elapsed = now - self.offered_at.unwrap_or(now);
                match self.m.act(seat, action, elapsed) {
                    Ok(events) => self.process(events).await,
                    Err(e) => self.send(
                        seat,
                        ServerMsg::Error {
                            code: ErrorCode::IllegalAction,
                            message: e.to_string(),
                        },
                    ),
                }
            }
        }
    }

    async fn on_deadline(&mut self) {
        let seat = self
            .m
            .hand()
            .and_then(|h| h.to_act())
            .expect("a deadline is only set while someone is to act");
        if self.conns[seat.index()].is_none() {
            self.sitting_out[seat.index()] = true;
        }
        let events = self
            .m
            .timeout(seat)
            .expect("timing out the player to act always succeeds");
        self.process(events).await;
    }

    async fn start_hand(&mut self) {
        self.next_hand_at = None;
        let deck = Deck::shuffled(&mut rand::rng());
        let events = self
            .m
            .start_hand(deck)
            .expect("a hand is only started between hands of an unfinished match");
        self.hand_started_at = OffsetDateTime::now_utc();
        self.process(events).await;
    }

    /// Forwards `events` (filtered per seat), logs them, then either offers the next
    /// decision or wraps up the hand.
    async fn process(&mut self, events: Vec<MatchEvent>) {
        let match_id = self.match_id();
        for e in events {
            let vis = e.visibility();
            let wire = WireEvent::from(&e);
            for seat in Seat::BOTH {
                if may_see(vis, seat) {
                    let event = wire.clone();
                    self.send(seat, ServerMsg::Event { match_id, event });
                }
            }
            self.hand_log.push(e);
        }

        if self.m.hand().is_some() {
            self.offered_at = Some(Instant::now());
            self.reset_deadline();
            if let Some(turn) = self.turn_info() {
                for seat in Seat::BOTH {
                    self.send(seat, ServerMsg::Turn { match_id, turn });
                }
            }
            return;
        }

        // The hand is over.
        self.offered_at = None;
        self.deadline = None;
        let record = HandRecord {
            match_id,
            number: self.m.hands_played(),
            started_at: self.hand_started_at,
            names: self.setup.names.clone(),
            events: std::mem::take(&mut self.hand_log),
        };
        // The audit log is integrity evidence: retry (no clock runs between hands).
        for attempt in 0..5u32 {
            match self.store.record_hand(&record).await {
                Ok(()) => break,
                Err(e) => {
                    let hand = record.number;
                    tracing::error!(match_id, hand, attempt, "failed to record hand: {e:#}");
                    tokio::time::sleep(Duration::from_millis(200 << attempt)).await;
                }
            }
        }
        if self.m.winner().is_none() {
            self.next_hand_at = Some(Instant::now() + self.setup.timing.between_hands);
        }
    }

    /// Sets the deadline for the decision offered at `offered_at`.
    fn reset_deadline(&mut self) {
        let (Some(offered), Some(seat)) = (self.offered_at, self.m.hand().and_then(|h| h.to_act()))
        else {
            return;
        };
        let full = offered + self.m.time_allowed(seat);
        self.deadline = Some(if self.sitting_out[seat.index()] {
            full.min(offered + self.setup.timing.sit_out_delay)
        } else {
            full
        });
    }

    fn turn_info(&self) -> Option<TurnInfo> {
        let legal = self.m.hand()?.legal()?;
        let remaining = self.deadline.map_or(Duration::ZERO, |d| {
            d.saturating_duration_since(Instant::now())
        });
        Some(TurnInfo {
            seat: seat_u8(legal.seat),
            legal: legal.into(),
            remaining_ms: millis(remaining),
        })
    }

    fn snapshot(&self, seat: Seat) -> MatchSnapshot {
        let hand = self.m.hand().map(|h| HandSnapshot {
            state: (&h.public_state()).into(),
            events: self
                .hand_log
                .iter()
                .filter(|e| may_see(e.visibility(), seat))
                .map(WireEvent::from)
                .collect(),
        });
        MatchSnapshot {
            match_id: self.match_id(),
            seat: seat_u8(seat),
            names: self.setup.names.clone(),
            ratings: self.setup.ratings,
            hands_played: self.m.hands_played(),
            stacks: self.m.stacks(),
            time_banks_ms: self.m.time_banks().map(millis),
            per_action_ms: millis(self.setup.config.time.per_action),
            hand,
            turn: self.turn_info(),
            winner: self.m.winner().map(seat_u8),
            auto_muck: self.m.auto_muck(seat),
        }
    }

    async fn finish(mut self) -> MatchOutcome {
        let winner = self
            .m
            .winner()
            .expect("finish is only called once there is a winner");
        let result = MatchFinish {
            match_id: self.match_id(),
            winner,
            hands_played: self.m.hands_played(),
        };
        let mut ratings = None;
        for attempt in 0..5u32 {
            match self.store.finish_match(&result).await {
                Ok(r) => {
                    ratings = Some(r);
                    break;
                }
                Err(e) => {
                    tracing::error!(
                        match_id = result.match_id,
                        attempt,
                        "failed to finish match: {e:#}"
                    );
                    tokio::time::sleep(Duration::from_millis(500 << attempt)).await;
                }
            }
        }
        for seat in Seat::BOTH {
            self.send(
                seat,
                ServerMsg::MatchEnded {
                    match_id: result.match_id,
                    winner: seat_u8(winner),
                    ratings,
                },
            );
        }
        MatchOutcome { winner, ratings }
    }
}
