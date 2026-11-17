//! The matchmaking queue. Players can't choose opponents: the server pairs them by
//! rating, widening each player's acceptable rating gap the longer they wait.

use std::sync::Arc;
use std::time::Duration;

use pokerladder_engine::holdem::Seat;
use rand::RngExt;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::protocol::{ErrorCode, ServerMsg, UserId};
use crate::table::{self, MatchCmd, MatchSetup};
use crate::{AppState, db};

/// A waiting player's acceptable rating gap: 100 at first, +20 per second waited.
pub fn window(waited: Duration) -> f64 {
    100.0 + 20.0 * waited.as_secs_f64()
}

#[derive(Clone, Debug)]
pub struct Waiting {
    pub user: UserId,
    pub rating: f64,
    pub since: Instant,
}

/// Picks pairs (indices into `queue`). The longest-waiting players choose first, each
/// taking the closest-rated player whose gap both of them accept.
pub fn pair(queue: &[Waiting], now: Instant) -> Vec<(usize, usize)> {
    let mut order: Vec<usize> = (0..queue.len()).collect();
    order.sort_by_key(|&i| queue[i].since);
    let win = |i: usize| window(now.saturating_duration_since(queue[i].since));
    let mut taken = vec![false; queue.len()];
    let mut pairs = Vec::new();
    for &i in &order {
        if taken[i] {
            continue;
        }
        let best = (0..queue.len())
            .filter(|&j| j != i && !taken[j])
            .map(|j| (j, (queue[i].rating - queue[j].rating).abs()))
            .filter(|&(j, gap)| gap <= win(i).min(win(j)))
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((j, _)) = best {
            taken[i] = true;
            taken[j] = true;
            pairs.push((i, j));
        }
    }
    pairs
}

enum QueueCmd {
    Join { user: UserId, rating: f64 },
    Leave { user: UserId },
}

#[derive(Clone)]
pub struct Matchmaker {
    tx: mpsc::UnboundedSender<QueueCmd>,
    /// Cancelled once the queue task has exited: after that no match can start.
    stopped: CancellationToken,
}

impl Matchmaker {
    /// Starts the queue task; it begins work once `state_rx` delivers the app state
    /// (which itself contains this `Matchmaker`).
    pub fn spawn(state_rx: tokio::sync::oneshot::Receiver<AppState>) -> Matchmaker {
        let (tx, rx) = mpsc::unbounded_channel();
        let stopped = CancellationToken::new();
        let guard = stopped.clone().drop_guard();
        tokio::spawn(async move {
            let _guard = guard;
            let Ok(state) = state_rx.await else { return };
            run(state, rx).await;
        });
        Matchmaker { tx, stopped }
    }

    /// Adds (or re-adds, keeping their place) a player. False if the queue has closed
    /// for a shutdown (the player isn't queued).
    pub fn join(&self, user: UserId, rating: f64) -> bool {
        self.tx.send(QueueCmd::Join { user, rating }).is_ok()
    }

    /// Resolves once the queue task has exited (after a shutdown began).
    pub async fn stopped(&self) {
        self.stopped.cancelled().await
    }

    pub fn leave(&self, user: UserId) {
        let _ = self.tx.send(QueueCmd::Leave { user });
    }
}

async fn run(state: AppState, mut rx: mpsc::UnboundedReceiver<QueueCmd>) {
    let mut queue: Vec<Waiting> = Vec::new();
    let mut tick = tokio::time::interval(Duration::from_millis(500));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        // Pairing runs on the tick and on joins, not on every command (cheap `Leave`
        // spam can't make the queue do O(n^2) work per message).
        tokio::select! {
            // Checked first: once a shutdown begins, nobody else gets paired.
            biased;
            () = state.draining.cancelled() => {
                // Close the queue (later joins fail) and tell everyone waiting,
                // including joins still in the channel.
                rx.close();
                let mut waiting: Vec<UserId> = queue.drain(..).map(|w| w.user).collect();
                while let Ok(cmd) = rx.try_recv() {
                    if let QueueCmd::Join { user, .. } = cmd {
                        waiting.push(user);
                    }
                }
                waiting.sort_unstable();
                waiting.dedup();
                for user in waiting {
                    if let Some(c) = state.hub.conn(user) {
                        c.send(ServerMsg::QueueLeft);
                    }
                }
                return;
            }
            cmd = rx.recv() => match cmd {
                Some(QueueCmd::Join { user, rating }) => {
                    if let Some(w) = queue.iter_mut().find(|w| w.user == user) {
                        w.rating = rating;
                    } else {
                        queue.push(Waiting { user, rating, since: Instant::now() });
                    }
                }
                Some(QueueCmd::Leave { user }) => {
                    queue.retain(|w| w.user != user);
                    continue;
                }
                None => return,
            },
            _ = tick.tick() => {}
        }
        if state.draining.is_cancelled() {
            continue; // the drain arm above runs next
        }
        // Players who got seated some other way (or disconnected) drop out.
        queue.retain(|w| state.hub.seat(w.user).is_none() && state.hub.conn(w.user).is_some());
        let pairs = pair(&queue, Instant::now());
        if pairs.is_empty() {
            continue;
        }
        let mut paired = vec![false; queue.len()];
        let mut starts = Vec::new();
        for (i, j) in pairs {
            paired[i] = true;
            paired[j] = true;
            starts.push([queue[i].user, queue[j].user]);
        }
        let mut idx = 0;
        queue.retain(|_| {
            idx += 1;
            !paired[idx - 1]
        });
        for users in starts {
            let state = state.clone();
            // Tracked, so a shutdown can wait for the match to finish.
            state.clone().matches.spawn(async move {
                if let Err(e) = start_match(state.clone(), users).await {
                    tracing::error!("failed to start match: {e:#}");
                    // Don't leave them believing they're still queued.
                    for u in users {
                        if state.hub.seat(u).is_none()
                            && let Some(c) = state.hub.conn(u)
                        {
                            c.send(ServerMsg::QueueLeft);
                        }
                    }
                }
            });
        }
    }
}

/// Creates the match, seats the players and runs the actor until the match is over.
pub async fn start_match(state: AppState, mut users: [UserId; 2]) -> anyhow::Result<()> {
    let (first_button, swap) = {
        let mut rng = rand::rng();
        (
            if rng.random_bool(0.5) {
                Seat::Zero
            } else {
                Seat::One
            },
            rng.random_bool(0.5),
        )
    };
    if swap {
        users.swap(0, 1);
    }
    let db::NewMatch {
        id: match_id,
        names,
        ratings,
        rated,
    } = db::create_match(&state.db, users).await?;
    let (cmd_tx, cmd_rx) = mpsc::channel(64);
    let Some(conns) = state.hub.seat_players(users, match_id, &cmd_tx) else {
        db::abandon_match(&state.db, match_id).await?;
        anyhow::bail!("a player was seated elsewhere in the meantime");
    };
    tracing::info!(match_id, ?users, rated, "match started");
    for (seat, conn) in Seat::BOTH.into_iter().zip(conns) {
        if let Some(conn) = conn {
            let _ = cmd_tx.send(MatchCmd::Connect { seat, conn }).await;
        }
    }
    drop(cmd_tx); // the hub holds the senders now
    let setup = MatchSetup {
        match_id,
        names,
        ratings,
        rated,
        config: state.match_config.clone(),
        first_button,
        timing: state.timing,
    };
    // A separate task, so that even a panic (the engine asserts its invariants in
    // release) frees the players and closes the match.
    let actor = tokio::spawn(table::run(setup, Arc::clone(&state.store), cmd_rx));
    let outcome = actor.await;
    state.hub.unseat(users, match_id);
    match outcome {
        Ok(o) => tracing::info!(match_id, winner = ?o.winner, "match finished"),
        Err(e) => {
            tracing::error!(match_id, "match actor failed: {e}");
            db::abandon_match(&state.db, match_id).await?;
            for u in users {
                if let Some(c) = state.hub.conn(u) {
                    c.send(ServerMsg::Error {
                        code: ErrorCode::Unavailable,
                        message: "the match was cancelled by a server error (unrated)".into(),
                    });
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(user: UserId, rating: f64, waited_s: u64, now: Instant) -> Waiting {
        Waiting {
            user,
            rating,
            since: now - Duration::from_secs(waited_s),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn pairs_closest_within_both_windows() {
        let now = Instant::now() + Duration::from_secs(1000);
        // 0 and 1 are 150 apart: too far for fresh players (window 100).
        let q = [w(1, 1500.0, 0, now), w(2, 1650.0, 0, now)];
        assert!(pair(&q, now).is_empty());
        // After 3 s both windows are 160.
        let q = [w(1, 1500.0, 3, now), w(2, 1650.0, 3, now)];
        assert_eq!(pair(&q, now), vec![(0, 1)]);
        // One patient player isn't enough: the newcomer's window still says no.
        let q = [w(1, 1500.0, 60, now), w(2, 1650.0, 0, now)];
        assert!(pair(&q, now).is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn longest_waiting_chooses_first_and_nobody_is_paired_twice() {
        let now = Instant::now() + Duration::from_secs(1000);
        let q = [
            w(1, 1500.0, 1, now),
            w(2, 1540.0, 10, now), // waited longest: takes 1530 (closest)
            w(3, 1530.0, 2, now),
            w(4, 1510.0, 1, now),
        ];
        let pairs = pair(&q, now);
        assert_eq!(pairs, vec![(1, 2), (0, 3)]);
        let mut seen: Vec<usize> = pairs.iter().flat_map(|&(a, b)| [a, b]).collect();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), 4);
    }

    #[tokio::test(start_paused = true)]
    async fn odd_one_out_waits() {
        let now = Instant::now() + Duration::from_secs(1000);
        let q = [
            w(1, 1500.0, 5, now),
            w(2, 1500.0, 5, now),
            w(3, 1500.0, 5, now),
        ];
        assert_eq!(pair(&q, now).len(), 1);
        assert!(pair(&q[..1], now).is_empty());
    }
}
