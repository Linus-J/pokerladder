//! Who is connected, and who is seated in which match. Each user has at most one live
//! connection (a new one replaces the old) and at most one match.
//!
//! Messages to match actors are sent *outside* the lock. That is safe because
//! [`MatchCmd::Connect`] carries a connection id that only ever increases, so the actor
//! ignores a stale connect whatever order they arrive in.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use pokerladder_engine::holdem::Seat;
use tokio::sync::mpsc;

use crate::protocol::{MatchId, UserId};
use crate::table::{Conn, MatchCmd};

#[derive(Clone)]
pub struct SeatHandle {
    pub match_id: MatchId,
    pub seat: Seat,
    pub cmd: mpsc::Sender<MatchCmd>,
}

#[derive(Default)]
pub struct Hub {
    next_conn_id: AtomicU64,
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    conns: HashMap<UserId, Conn>,
    seats: HashMap<UserId, SeatHandle>,
}

impl Hub {
    pub fn next_conn_id(&self) -> u64 {
        self.next_conn_id.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Registers `conn` as the user's connection. Returns the connection it replaced
    /// and the user's seat, if any (the caller sends the match a `Connect`).
    pub fn connect(&self, user: UserId, conn: Conn) -> (Option<Conn>, Option<SeatHandle>) {
        let mut inner = self.inner.lock().unwrap();
        let old = inner.conns.insert(user, conn);
        (old, inner.seats.get(&user).cloned())
    }

    /// Unregisters `conn_id` if it is still the user's connection. Returns whether it
    /// was, and the user's seat (the caller sends the match a `Disconnect`).
    pub fn disconnect(&self, user: UserId, conn_id: u64) -> (bool, Option<SeatHandle>) {
        let mut inner = self.inner.lock().unwrap();
        let current = inner.conns.get(&user).is_some_and(|c| c.id == conn_id);
        if current {
            inner.conns.remove(&user);
        }
        (current, inner.seats.get(&user).cloned())
    }

    pub fn seat(&self, user: UserId) -> Option<SeatHandle> {
        self.inner.lock().unwrap().seats.get(&user).cloned()
    }

    pub fn conn(&self, user: UserId) -> Option<Conn> {
        self.inner.lock().unwrap().conns.get(&user).cloned()
    }

    /// Seats `users[i]` in seat `i`. Fails (changing nothing) if either is already
    /// seated. Returns their current connections.
    pub fn seat_players(
        &self,
        users: [UserId; 2],
        match_id: MatchId,
        cmd: &mpsc::Sender<MatchCmd>,
    ) -> Option<[Option<Conn>; 2]> {
        let mut inner = self.inner.lock().unwrap();
        if users.iter().any(|u| inner.seats.contains_key(u)) {
            return None;
        }
        for (u, seat) in users.into_iter().zip(Seat::BOTH) {
            let handle = SeatHandle {
                match_id,
                seat,
                cmd: cmd.clone(),
            };
            inner.seats.insert(u, handle);
        }
        Some(users.map(|u| inner.conns.get(&u).cloned()))
    }

    /// Frees both players' seats in `match_id`.
    pub fn unseat(&self, users: [UserId; 2], match_id: MatchId) {
        let mut inner = self.inner.lock().unwrap();
        for u in users {
            if inner.seats.get(&u).is_some_and(|s| s.match_id == match_id) {
                inner.seats.remove(&u);
            }
        }
    }
}
