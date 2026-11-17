//! Match actor tests: an in-memory store and tokio's paused clock, so timers are exact
//! and whole matches run in milliseconds.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pokerladder_engine::MatchConfig;
use pokerladder_engine::holdem::{Action, Seat};
use pokerladder_server::protocol::{RatingChange, ServerMsg, WireAction, WireEvent};
use pokerladder_server::rating;
use pokerladder_server::table::{
    self, Conn, HandRecord, MatchCmd, MatchFinish, MatchOutcome, MatchSetup, MatchStore, Timing,
    render_histories,
};
use rand::RngExt;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

/// `(number, audit wire events, [seat0, seat1, audit] histories)`.
type StoredHand = (u32, Vec<WireEvent>, [String; 3]);

#[derive(Default)]
struct MemStore {
    hands: Mutex<Vec<StoredHand>>,
    finished: Mutex<Vec<(Seat, u32)>>,
    /// While set, every write fails (a database outage).
    down: AtomicBool,
    /// While set, finishing fails with an error retrying can't fix.
    broken: AtomicBool,
    /// While set, recording a hand fails with an error retrying can't fix.
    hands_broken: AtomicBool,
}

impl MemStore {
    fn check(&self) -> anyhow::Result<()> {
        if self.down.load(Ordering::SeqCst) {
            return Err(sqlx::Error::PoolTimedOut.into());
        }
        Ok(())
    }
}

impl MatchStore for MemStore {
    async fn record_hand(&self, hand: &HandRecord) -> anyhow::Result<()> {
        anyhow::ensure!(!self.hands_broken.load(Ordering::SeqCst), "bad hand");
        self.check()?;
        let histories = render_histories(hand.number as u64, hand)?;
        let events = hand.events.iter().map(WireEvent::from).collect();
        self.hands
            .lock()
            .unwrap()
            .push((hand.number, events, histories));
        Ok(())
    }

    async fn finish_match(&self, r: &MatchFinish) -> anyhow::Result<Option<[RatingChange; 2]>> {
        self.check()?;
        anyhow::ensure!(
            !self.broken.load(Ordering::SeqCst),
            "match is not in progress"
        );
        self.finished
            .lock()
            .unwrap()
            .push((r.winner, r.hands_played));
        let after = rating::match_result([rating::Glicko::NEW; 2], [0.0; 2], r.winner.index());
        Ok(Some([0, 1].map(|i| RatingChange {
            before: rating::INITIAL_RATING,
            after: after[i].rating,
        })))
    }
}

const NAMES: [&str; 2] = ["alice", "bob"];

fn setup() -> MatchSetup {
    pokerladder_engine::eval::init();
    MatchSetup {
        match_id: 7,
        names: NAMES.map(String::from),
        ratings: [1500.0; 2],
        rated: true,
        config: MatchConfig::standard(),
        first_button: Seat::Zero,
        timing: Timing::STANDARD,
    }
}

fn conn(id: u64) -> (Conn, mpsc::Receiver<ServerMsg>) {
    let (tx, rx) = mpsc::channel(10_000);
    let kill = CancellationToken::new();
    (Conn { id, tx, kill }, rx)
}

fn spawn_match(
    store: &Arc<MemStore>,
) -> (
    mpsc::Sender<MatchCmd>,
    tokio::task::JoinHandle<MatchOutcome>,
) {
    let (tx, rx) = mpsc::channel(64);
    let task = tokio::spawn(table::run(setup(), Arc::clone(store), rx));
    (tx, task)
}

/// Checks a message received by `seat` doesn't leak the opponent's hole cards.
fn assert_private(seat: Seat, msg: &ServerMsg) {
    let check = |e: &WireEvent| {
        if let WireEvent::HoleCards { seat: s, .. } = e {
            assert_eq!(
                *s as usize,
                seat.index(),
                "{seat:?} got the opponent's hole cards"
            );
        }
    };
    match msg {
        ServerMsg::Event { event, .. } => check(event),
        ServerMsg::MatchState(snap) => {
            for e in snap.hand.iter().flat_map(|h| &h.events) {
                check(e);
            }
        }
        _ => {}
    }
}

/// A bot that plays random legal actions; occasionally sends an illegal one first.
async fn random_bot(
    seat: Seat,
    mut rx: mpsc::Receiver<ServerMsg>,
    cmd: mpsc::Sender<MatchCmd>,
) -> (Vec<ServerMsg>, Option<u8>) {
    let mut log = Vec::new();
    while let Some(msg) = rx.recv().await {
        assert_private(seat, &msg);
        if let ServerMsg::Turn { turn, .. } = &msg
            && turn.seat as usize == seat.index()
        {
            let l = turn.legal;
            let action = {
                let mut rng = rand::rng();
                if rng.random_bool(0.05) {
                    // Illegal: a raise of 0 is never in range.
                    Action::RaiseTo(0)
                } else {
                    let mut options = vec![];
                    if l.can_fold {
                        options.push(Action::Fold);
                    }
                    if l.can_check {
                        options.push(Action::Check);
                    }
                    if l.call.is_some() {
                        options.extend([Action::Call, Action::Call]);
                    }
                    if let (Some(lo), Some(hi)) = (l.raise_min_to, l.raise_max_to) {
                        options.push(Action::RaiseTo(rng.random_range(lo..=hi)));
                        options.push(Action::AllIn);
                    }
                    options[rng.random_range(0..options.len())]
                }
            };
            let _ = cmd.send(MatchCmd::Act { seat, action }).await;
            if action == Action::RaiseTo(0) {
                // Follow up with something legal.
                let fallback = if l.can_check {
                    Action::Check
                } else {
                    Action::Call
                };
                let _ = cmd
                    .send(MatchCmd::Act {
                        seat,
                        action: fallback,
                    })
                    .await;
            }
        }
        let ended = match &msg {
            ServerMsg::MatchEnded { winner, .. } => Some(*winner),
            _ => None,
        };
        log.push(msg);
        if ended.is_some() {
            return (log, ended);
        }
    }
    (log, None)
}

#[tokio::test(start_paused = true)]
async fn random_bots_play_whole_matches_privately() {
    for _ in 0..20 {
        let store = Arc::new(MemStore::default());
        let (cmd, task) = spawn_match(&store);
        let mut bots = Vec::new();
        for (i, seat) in Seat::BOTH.into_iter().enumerate() {
            let (c, rx) = conn(i as u64 + 1);
            cmd.send(MatchCmd::Connect { seat, conn: c }).await.unwrap();
            bots.push(tokio::spawn(random_bot(seat, rx, cmd.clone())));
        }
        let outcome = task.await.unwrap();
        let mut winners = Vec::new();
        for bot in bots {
            let (log, winner) = bot.await.unwrap();
            winners.push(winner);
            assert!(matches!(log[0], ServerMsg::MatchState(_)));
            // Every hand's chips add up.
            for m in &log {
                if let ServerMsg::Event {
                    event: WireEvent::Ended { stacks },
                    ..
                } = m
                {
                    assert_eq!(stacks[0] + stacks[1], 3000);
                }
            }
        }
        let w = outcome.winner.index() as u8;
        assert_eq!(winners, vec![Some(w), Some(w)]);
        assert!(outcome.ratings.is_some());

        let hands = store.hands.lock().unwrap();
        let finished = store.finished.lock().unwrap();
        assert_eq!(finished.len(), 1);
        assert_eq!(finished[0].1 as usize, hands.len());
        for (i, (number, audit, [h0, h1, ha])) in hands.iter().enumerate() {
            assert_eq!(*number as usize, i + 1);
            // The audit log keeps both players' hole cards; each history only its own.
            let holes = audit
                .iter()
                .filter(|e| matches!(e, WireEvent::HoleCards { .. }))
                .count();
            assert_eq!(holes, 2);
            assert_eq!(ha.matches("Dealt to").count(), 2, "{ha}");
            assert_eq!(h0.matches("Dealt to").count(), 1);
            assert_eq!(h1.matches("Dealt to").count(), 1);
            assert!(h0.contains("Dealt to alice"));
            assert!(h1.contains("Dealt to bob"));
        }
        assert!(matches!(
            hands.last().unwrap().1.last(),
            Some(WireEvent::MatchOver { .. })
        ));
    }
}

/// Waits for the next `Turn` and returns when it arrived.
async fn next_turn(rx: &mut mpsc::Receiver<ServerMsg>) -> (Instant, u8) {
    loop {
        match rx.recv().await.expect("channel open") {
            ServerMsg::Turn { turn, .. } => return (Instant::now(), turn.seat),
            _ => continue,
        }
    }
}

async fn next_timeout(rx: &mut mpsc::Receiver<ServerMsg>) -> (Instant, u8) {
    loop {
        if let ServerMsg::Event {
            event: WireEvent::TimedOut { seat },
            ..
        } = rx.recv().await.expect("channel open")
        {
            return (Instant::now(), seat);
        }
    }
}

#[tokio::test(start_paused = true)]
async fn rejected_actions_do_not_reset_the_clock() {
    let store = Arc::new(MemStore::default());
    let (cmd, _task) = spawn_match(&store);
    let (c0, mut rx0) = conn(1);
    let (c1, _rx1) = conn(2);
    cmd.send(MatchCmd::Connect {
        seat: Seat::Zero,
        conn: c0,
    })
    .await
    .unwrap();
    cmd.send(MatchCmd::Connect {
        seat: Seat::One,
        conn: c1,
    })
    .await
    .unwrap();

    // Seat 0 has the button and acts first preflop.
    let (offered, seat) = next_turn(&mut rx0).await;
    assert_eq!(seat, 0);
    for _ in 0..5 {
        tokio::time::sleep(Duration::from_secs(10)).await;
        let illegal = Action::RaiseTo(1);
        cmd.send(MatchCmd::Act {
            seat: Seat::Zero,
            action: illegal,
        })
        .await
        .unwrap();
    }
    let (at, seat) = next_timeout(&mut rx0).await;
    assert_eq!(seat, 0);
    // 15 s per action + 60 s bank, from when the decision was offered.
    assert_eq!(at - offered, Duration::from_secs(75));
}

#[tokio::test(start_paused = true)]
async fn late_actions_become_timeouts_and_bank_is_spent() {
    let store = Arc::new(MemStore::default());
    let (cmd, _task) = spawn_match(&store);
    let (c0, mut rx0) = conn(1);
    let (c1, _rx1) = conn(2);
    cmd.send(MatchCmd::Connect {
        seat: Seat::Zero,
        conn: c0,
    })
    .await
    .unwrap();
    cmd.send(MatchCmd::Connect {
        seat: Seat::One,
        conn: c1,
    })
    .await
    .unwrap();

    let (offered, _) = next_turn(&mut rx0).await;
    // Use 40 s: 25 s comes out of the bank.
    tokio::time::sleep(Duration::from_secs(40)).await;
    cmd.send(MatchCmd::Act {
        seat: Seat::Zero,
        action: Action::Call,
    })
    .await
    .unwrap();
    let (_, seat) = next_turn(&mut rx0).await;
    assert_eq!(seat, 1);
    cmd.send(MatchCmd::Act {
        seat: Seat::One,
        action: Action::Check,
    })
    .await
    .unwrap();
    // Flop: seat 1 (big blind) acts first, then seat 0 again.
    let (_, seat) = next_turn(&mut rx0).await;
    assert_eq!(seat, 1);
    cmd.send(MatchCmd::Act {
        seat: Seat::One,
        action: Action::Check,
    })
    .await
    .unwrap();
    let (offered2, seat) = next_turn(&mut rx0).await;
    assert_eq!(seat, 0);
    assert!(offered2 - offered >= Duration::from_secs(40));
    // Seat 0 now has 15 + 35 = 50 s.
    let (at, _) = next_timeout(&mut rx0).await;
    assert_eq!(at - offered2, Duration::from_secs(50));
}

#[tokio::test(start_paused = true)]
async fn reconnect_gets_a_private_snapshot_and_stale_connects_are_ignored() {
    let store = Arc::new(MemStore::default());
    let (cmd, _task) = spawn_match(&store);
    let (c0, mut rx0) = conn(5);
    let (c1, _rx1) = conn(6);
    cmd.send(MatchCmd::Connect {
        seat: Seat::Zero,
        conn: c0,
    })
    .await
    .unwrap();
    cmd.send(MatchCmd::Connect {
        seat: Seat::One,
        conn: c1,
    })
    .await
    .unwrap();
    next_turn(&mut rx0).await;

    // Seat 0 reconnects on a newer connection mid-hand.
    let (c0b, mut rx0b) = conn(9);
    cmd.send(MatchCmd::Connect {
        seat: Seat::Zero,
        conn: c0b,
    })
    .await
    .unwrap();
    let ServerMsg::MatchState(snap) = rx0b.recv().await.unwrap() else {
        panic!("expected a snapshot first");
    };
    assert_eq!(snap.seat, 0);
    assert_eq!(snap.names, NAMES.map(String::from));
    let hand = snap.hand.as_ref().expect("a hand is in progress");
    let holes: Vec<_> = hand
        .events
        .iter()
        .filter_map(|e| match e {
            WireEvent::HoleCards { seat, .. } => Some(*seat),
            _ => None,
        })
        .collect();
    assert_eq!(holes, vec![0]);
    let turn = snap.turn.expect("someone is to act");
    assert_eq!(turn.seat, 0);
    assert!(turn.remaining_ms <= 75_000);
    assert_private(Seat::Zero, &ServerMsg::MatchState(snap));

    // A stale connect (lower id) arriving late doesn't steal the seat back.
    let (stale, mut rx_stale) = conn(7);
    cmd.send(MatchCmd::Connect {
        seat: Seat::Zero,
        conn: stale,
    })
    .await
    .unwrap();
    // Disconnecting the old id is a no-op too.
    cmd.send(MatchCmd::Disconnect {
        seat: Seat::Zero,
        conn_id: 5,
    })
    .await
    .unwrap();
    cmd.send(MatchCmd::Act {
        seat: Seat::Zero,
        action: Action::Call,
    })
    .await
    .unwrap();
    // Wait for the call to be processed and delivered.
    let (_, seat) = next_turn(&mut rx0b).await;
    assert_eq!(seat, 1);
    assert!(
        rx_stale.try_recv().is_err(),
        "stale connection got messages"
    );
    let _ = rx0.try_recv(); // the replaced connection may hold older messages
}

#[tokio::test(start_paused = true)]
async fn an_absent_player_sits_out_and_never_wins() {
    let store = Arc::new(MemStore::default());
    let (cmd, task) = spawn_match(&store);
    let (c1, mut rx1) = conn(1);
    cmd.send(MatchCmd::Connect {
        seat: Seat::One,
        conn: c1,
    })
    .await
    .unwrap();
    let start = Instant::now();

    // Seat 1 shoves every decision; seat 0 never shows up.
    let shover = tokio::spawn({
        let cmd = cmd.clone();
        async move {
            while let Some(msg) = rx1.recv().await {
                match msg {
                    ServerMsg::Turn { turn, .. } if turn.seat == 1 => {
                        let action = Action::AllIn;
                        let _ = cmd
                            .send(MatchCmd::Act {
                                seat: Seat::One,
                                action,
                            })
                            .await;
                    }
                    ServerMsg::MatchEnded { winner, .. } => return winner,
                    _ => {}
                }
            }
            panic!("channel closed before the match ended");
        }
    });
    let outcome = task.await.unwrap();
    assert_eq!(outcome.winner, Seat::One);
    assert_eq!(shover.await.unwrap(), 1);
    // Only the first decision used the absent player's full clock; after that they
    // were sitting out, so the match finished promptly.
    let hands = store.hands.lock().unwrap().len() as u32;
    let elapsed = start.elapsed();
    let bound = Duration::from_secs(75 + 3 + 5 * hands as u64);
    assert!(elapsed < bound, "took {elapsed:?} for {hands} hands");
}

/// The database is down for a whole match: every hand and the result miss their quick
/// retries and come back as pending; once the database returns, `settle` stores every
/// hand once, in order, and the result once.
#[tokio::test(start_paused = true)]
async fn a_database_outage_loses_nothing() {
    let store = Arc::new(MemStore::default());
    store.down.store(true, Ordering::SeqCst);
    let (cmd, task) = spawn_match(&store);
    let outcome = task.await.unwrap();
    drop(cmd);
    assert!(outcome.ratings.is_none(), "nothing could be recorded");
    let pending = outcome.pending;
    let hands = pending.hands.len();
    assert!(hands > 0);
    let numbers: Vec<u32> = pending.hands.iter().map(|h| h.number).collect();
    assert_eq!(numbers, (1..=hands as u32).collect::<Vec<_>>());
    let f = pending.finish.as_ref().expect("the result is pending");
    assert_eq!((f.winner, f.hands_played), (outcome.winner, hands as u32));
    assert!(store.hands.lock().unwrap().is_empty());

    let settling = {
        let store = Arc::clone(&store);
        tokio::spawn(async move { table::settle(&*store, pending).await })
    };
    tokio::time::sleep(Duration::from_secs(300)).await;
    assert!(
        !settling.is_finished(),
        "keeps trying while the database is down"
    );
    store.down.store(false, Ordering::SeqCst);
    // Back-off is capped at a minute.
    tokio::time::timeout(Duration::from_secs(61), settling)
        .await
        .expect("settles within a minute of the database returning")
        .unwrap();
    let stored: Vec<u32> = store.hands.lock().unwrap().iter().map(|h| h.0).collect();
    assert_eq!(stored, (1..=hands as u32).collect::<Vec<_>>());
    assert_eq!(
        *store.finished.lock().unwrap(),
        vec![(outcome.winner, hands as u32)]
    );
}

/// An error retrying can't fix: `settle` stops at once instead of retrying for hours
/// (and holding a graceful shutdown for its whole drain timeout).
#[tokio::test(start_paused = true)]
async fn settle_stops_on_a_permanent_error() {
    let store = Arc::new(MemStore::default());
    store.down.store(true, Ordering::SeqCst);
    let (cmd, task) = spawn_match(&store);
    let outcome = task.await.unwrap();
    drop(cmd);
    store.down.store(false, Ordering::SeqCst);
    store.broken.store(true, Ordering::SeqCst);
    let started = Instant::now();
    table::settle(&*store, outcome.pending).await;
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(store.finished.lock().unwrap().is_empty());
    // The hands still got stored in that one round.
    assert!(!store.hands.lock().unwrap().is_empty());
}

/// Hands that can never be stored are dropped, but the result, which is only waiting
/// for the database, is still kept and stored once it returns.
#[tokio::test(start_paused = true)]
async fn a_bad_hand_does_not_cost_the_result() {
    let store = Arc::new(MemStore::default());
    store.down.store(true, Ordering::SeqCst);
    let (cmd, task) = spawn_match(&store);
    let outcome = task.await.unwrap();
    drop(cmd);
    store.hands_broken.store(true, Ordering::SeqCst);
    let settling = {
        let store = Arc::clone(&store);
        tokio::spawn(async move { table::settle(&*store, outcome.pending).await })
    };
    tokio::time::sleep(Duration::from_secs(120)).await;
    assert!(!settling.is_finished());
    store.down.store(false, Ordering::SeqCst);
    tokio::time::timeout(Duration::from_secs(61), settling)
        .await
        .expect("settles once the database returns")
        .unwrap();
    assert_eq!(store.finished.lock().unwrap().len(), 1);
    assert!(store.hands.lock().unwrap().is_empty());
}

#[test]
fn transient_errors_are_told_apart() {
    assert!(table::is_transient(&sqlx::Error::PoolTimedOut.into()));
    let wrapped = anyhow::Error::from(sqlx::Error::PoolClosed).context("finishing match");
    assert!(table::is_transient(&wrapped));
    assert!(!table::is_transient(&sqlx::Error::RowNotFound.into()));
    assert!(!table::is_transient(&anyhow::anyhow!("rating not finite")));
}

/// An outage that never ends: `settle` gives up after `SETTLE_FOR` instead of
/// retrying forever.
#[tokio::test(start_paused = true)]
async fn settle_gives_up_eventually() {
    let store = Arc::new(MemStore::default());
    store.down.store(true, Ordering::SeqCst);
    let (cmd, task) = spawn_match(&store);
    let outcome = task.await.unwrap();
    drop(cmd);
    let started = Instant::now();
    table::settle(&*store, outcome.pending).await;
    let took = started.elapsed();
    assert!(took >= table::SETTLE_FOR && took < table::SETTLE_FOR + Duration::from_secs(61));
    assert!(store.finished.lock().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn an_unattended_match_still_finishes() {
    let store = Arc::new(MemStore::default());
    let (cmd, task) = spawn_match(&store);
    let outcome = task.await.unwrap();
    drop(cmd);
    let finished = store.finished.lock().unwrap();
    assert_eq!(finished.len(), 1);
    assert_eq!(finished[0].0, outcome.winner);
}

#[tokio::test(start_paused = true)]
async fn wire_actions_convert_losslessly() {
    for a in [
        Action::Fold,
        Action::Check,
        Action::Call,
        Action::RaiseTo(1234),
        Action::AllIn,
    ] {
        assert_eq!(Action::from(WireAction::from(a)), a);
    }
}
