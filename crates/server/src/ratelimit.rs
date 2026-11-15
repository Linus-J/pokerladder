//! In-memory token-bucket rate limiting (single server process).

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Mutex;
use std::time::Duration;

use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::request::Parts;
use tokio::time::Instant;

use crate::AppState;

/// `burst` requests at once, refilling at one per `every`.
#[derive(Copy, Clone, Debug)]
pub struct Rate {
    pub burst: u32,
    pub every: Duration,
}

pub struct Limiter {
    rate: Rate,
    buckets: Mutex<HashMap<String, (f64, Instant)>>,
}

/// Above this many keys, full buckets are dropped (they carry no information).
const MAX_KEYS: usize = 50_000;

impl Limiter {
    pub fn new(rate: Rate) -> Limiter {
        Limiter {
            rate,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Takes a token for `key`; false if there is none left.
    pub fn check(&self, key: &str) -> bool {
        let now = Instant::now();
        let Rate { burst, every } = self.rate;
        let refill = |tokens: f64, since: Instant| {
            let gained = now.saturating_duration_since(since).as_secs_f64() / every.as_secs_f64();
            (tokens + gained).min(burst as f64)
        };
        let mut buckets = self.buckets.lock().unwrap();
        if buckets.len() >= MAX_KEYS {
            buckets.retain(|_, (t, at)| refill(*t, *at) < burst as f64);
        }
        let (tokens, at) = buckets.entry(key.to_owned()).or_insert((burst as f64, now));
        let available = refill(*tokens, *at);
        *at = now;
        if available >= 1.0 {
            *tokens = available - 1.0;
            true
        } else {
            *tokens = available;
            false
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub struct RateLimits {
    pub login_per_ip: Rate,
    pub login_per_account: Rate,
    pub register_per_ip: Rate,
    pub verify_per_ip: Rate,
}

impl RateLimits {
    pub const STANDARD: RateLimits = RateLimits {
        login_per_ip: Rate {
            burst: 10,
            every: Duration::from_secs(6),
        },
        login_per_account: Rate {
            burst: 10,
            every: Duration::from_secs(30),
        },
        register_per_ip: Rate {
            burst: 10,
            every: Duration::from_secs(60),
        },
        verify_per_ip: Rate {
            burst: 20,
            every: Duration::from_secs(3),
        },
    };
}

pub struct Limiters {
    pub login_ip: Limiter,
    pub login_account: Limiter,
    pub register_ip: Limiter,
    pub verify_ip: Limiter,
}

impl Limiters {
    pub fn new(r: RateLimits) -> Limiters {
        Limiters {
            login_ip: Limiter::new(r.login_per_ip),
            login_account: Limiter::new(r.login_per_account),
            register_ip: Limiter::new(r.register_per_ip),
            verify_ip: Limiter::new(r.verify_per_ip),
        }
    }
}

/// The client's IP: the peer address, or — behind a reverse proxy configured with
/// `TRUSTED_PROXY_HEADER` — the last address that header lists (the one the proxy
/// itself appended, which the client can't forge).
pub struct ClientIp(pub String);

impl FromRequestParts<AppState> for ClientIp {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        if let Some(header) = &state.config.trusted_proxy_header
            && let Some(ip) = parts
                .headers
                .get_all(header.as_str())
                .iter()
                .filter_map(|v| v.to_str().ok())
                .flat_map(|v| v.split(','))
                .map(str::trim)
                .rfind(|s| !s.is_empty())
        {
            return Ok(ClientIp(ip.to_owned()));
        }
        let peer = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map_or_else(|| "unknown".to_owned(), |c| c.0.ip().to_string());
        Ok(ClientIp(peer))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn bucket_allows_burst_then_refills() {
        let l = Limiter::new(Rate {
            burst: 3,
            every: Duration::from_secs(10),
        });
        assert!((0..3).all(|_| l.check("a")));
        assert!(!l.check("a"));
        assert!(l.check("b"), "keys are independent");
        tokio::time::advance(Duration::from_secs(9)).await;
        assert!(!l.check("a"));
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(l.check("a"));
        assert!(!l.check("a"));
        // Refills cap at the burst.
        tokio::time::advance(Duration::from_secs(1000)).await;
        assert!((0..3).all(|_| l.check("a")));
        assert!(!l.check("a"));
    }
}
