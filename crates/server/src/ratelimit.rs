//! In-memory token-bucket rate limiting (single server process).

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::HeaderMap;
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
    inner: Mutex<Buckets>,
}

struct Buckets {
    map: HashMap<String, (f64, Instant)>,
    pruned: Instant,
}

/// Full buckets carry no information; drop them this often.
const PRUNE_EVERY: Duration = Duration::from_secs(60);
/// Keys are truncated to this many bytes (callers should pass short keys anyway).
const MAX_KEY: usize = 128;

impl Limiter {
    pub fn new(rate: Rate) -> Limiter {
        Limiter {
            rate,
            inner: Mutex::new(Buckets {
                map: HashMap::new(),
                pruned: Instant::now(),
            }),
        }
    }

    fn tokens_now(&self, entry: Option<&(f64, Instant)>, now: Instant) -> f64 {
        let Rate { burst, every } = self.rate;
        match entry {
            None => burst as f64,
            Some(&(tokens, at)) => {
                let gained = now.saturating_duration_since(at).as_secs_f64() / every.as_secs_f64();
                (tokens + gained).min(burst as f64)
            }
        }
    }

    fn key(key: &str) -> &str {
        let mut end = key.len().min(MAX_KEY);
        while !key.is_char_boundary(end) {
            end -= 1;
        }
        &key[..end]
    }

    /// Takes a token for `key`; false (taking nothing) if there is none left.
    pub fn check(&self, key: &str) -> bool {
        let key = Self::key(key);
        let now = Instant::now();
        let mut b = self.inner.lock().unwrap();
        if now.saturating_duration_since(b.pruned) >= PRUNE_EVERY {
            let full = self.rate.burst as f64;
            b.map.retain(|_, e| self.tokens_now(Some(e), now) < full);
            b.pruned = now;
        }
        let available = self.tokens_now(b.map.get(key), now);
        let ok = available >= 1.0;
        let left = if ok { available - 1.0 } else { available };
        b.map.insert(key.to_owned(), (left, now));
        ok
    }

    /// Whether `key` has a token, without taking it.
    pub fn allowed(&self, key: &str) -> bool {
        let key = Self::key(key);
        let b = self.inner.lock().unwrap();
        self.tokens_now(b.map.get(key), Instant::now()) >= 1.0
    }

    /// Gives back a token taken by [`check`](Self::check) (capped at the burst).
    pub fn refund(&self, key: &str) {
        let key = Self::key(key);
        let now = Instant::now();
        let mut b = self.inner.lock().unwrap();
        let tokens = (self.tokens_now(b.map.get(key), now) + 1.0).min(self.rate.burst as f64);
        b.map.insert(key.to_owned(), (tokens, now));
    }
}

#[derive(Copy, Clone, Debug)]
pub struct RateLimits {
    pub login_per_ip: Rate,
    /// Failed logins per account.
    pub login_failures_per_account: Rate,
    pub register_per_ip: Rate,
    pub verify_per_ip: Rate,
    /// Emails the address owner asks for (verification, password reset).
    pub mail_per_recipient: Rate,
    /// "Someone tried to sign up with your email" notices, which anyone can trigger.
    pub unsolicited_mail_per_recipient: Rate,
}

impl RateLimits {
    pub const STANDARD: RateLimits = RateLimits {
        login_per_ip: Rate {
            burst: 10,
            every: Duration::from_secs(6),
        },
        login_failures_per_account: Rate {
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
        mail_per_recipient: Rate {
            burst: 3,
            every: Duration::from_secs(2 * 3600),
        },
        unsolicited_mail_per_recipient: Rate {
            burst: 1,
            every: Duration::from_secs(6 * 3600),
        },
    };
}

pub struct Limiters {
    pub login_ip: Limiter,
    pub login_account: Limiter,
    pub register_ip: Limiter,
    pub verify_ip: Limiter,
    pub mail_recipient: Limiter,
    pub mail_unsolicited: Limiter,
}

impl Limiters {
    pub fn new(r: RateLimits) -> Limiters {
        Limiters {
            login_ip: Limiter::new(r.login_per_ip),
            login_account: Limiter::new(r.login_failures_per_account),
            register_ip: Limiter::new(r.register_per_ip),
            verify_ip: Limiter::new(r.verify_per_ip),
            mail_recipient: Limiter::new(r.mail_per_recipient),
            mail_unsolicited: Limiter::new(r.unsolicited_mail_per_recipient),
        }
    }
}

/// The rate-limit key for an address: IPv4 as is; IPv6 by its /64 prefix (one
/// subscriber usually controls a whole /64, so per-address keys would be free to
/// rotate).
pub fn ip_key(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => v4.to_string(),
            None => {
                let s = v6.segments();
                format!("{:x}:{:x}:{:x}:{:x}::/64", s[0], s[1], s[2], s[3])
            }
        },
    }
}

/// The client address from `header` as set by a trusted reverse proxy: the rightmost
/// entry (the one the proxy itself appended; anything to its left is client-supplied).
/// Use a single-value header such as `X-Real-IP` or `CF-Connecting-IP` when there is
/// more than one proxy hop.
pub fn ip_from_header(headers: &HeaderMap, header: &str) -> Option<IpAddr> {
    headers
        .get_all(header)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(str::trim)
        .rfind(|s| !s.is_empty())?
        .parse()
        .ok()
}

/// The client's rate-limit key (see [`ip_key`]): from `TRUSTED_PROXY_HEADER` if
/// configured, else the peer address.
pub struct ClientIp(pub String);

static WARNED: AtomicBool = AtomicBool::new(false);

fn warn_once(what: &str) {
    if !WARNED.swap(true, Ordering::Relaxed) {
        tracing::warn!("{what}: rate limits fall back to one shared bucket");
    }
}

impl FromRequestParts<AppState> for ClientIp {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        if let Some(header) = &state.config.trusted_proxy_header {
            match ip_from_header(&parts.headers, header) {
                Some(ip) => return Ok(ClientIp(ip_key(ip))),
                None => warn_once("TRUSTED_PROXY_HEADER is set but missing or invalid"),
            }
        }
        match parts.extensions.get::<ConnectInfo<SocketAddr>>() {
            Some(c) => Ok(ClientIp(ip_key(c.0.ip()))),
            None => {
                warn_once("no peer address (serve with into_make_service_with_connect_info)");
                Ok(ClientIp("unknown".into()))
            }
        }
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
        assert!(!l.allowed("a"));
        assert!(!l.check("a"));
        assert!(l.check("b"), "keys are independent");
        tokio::time::advance(Duration::from_secs(9)).await;
        assert!(!l.check("a"));
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(l.allowed("a"));
        assert!(l.allowed("a"), "peeking takes nothing");
        assert!(l.check("a"));
        assert!(!l.check("a"));
        l.refund("a");
        assert!(l.check("a"), "a refunded token can be used again");
        assert!(!l.check("a"));
        // Refills cap at the burst.
        tokio::time::advance(Duration::from_secs(1000)).await;
        assert!((0..3).all(|_| l.check("a")));
        assert!(!l.check("a"));
    }

    #[tokio::test(start_paused = true)]
    async fn full_buckets_are_pruned_and_keys_are_bounded() {
        let l = Limiter::new(Rate {
            burst: 2,
            every: Duration::from_secs(1),
        });
        for i in 0..1000 {
            l.check(&format!("k{i}"));
        }
        let huge = "x".repeat(1 << 20);
        l.check(&huge);
        assert!(
            l.inner
                .lock()
                .unwrap()
                .map
                .keys()
                .all(|k| k.len() <= MAX_KEY)
        );
        tokio::time::advance(PRUNE_EVERY).await;
        l.check("fresh");
        assert_eq!(l.inner.lock().unwrap().map.len(), 1);
    }

    #[test]
    fn ipv6_is_keyed_by_slash_64() {
        let a: IpAddr = "2001:db8:1:2:aaaa::1".parse().unwrap();
        let b: IpAddr = "2001:db8:1:2:ffff::9".parse().unwrap();
        let c: IpAddr = "2001:db8:1:3::1".parse().unwrap();
        assert_eq!(ip_key(a), ip_key(b));
        assert_ne!(ip_key(a), ip_key(c));
        assert_eq!(ip_key("::ffff:192.0.2.7".parse().unwrap()), "192.0.2.7");
        assert_eq!(ip_key("192.0.2.7".parse().unwrap()), "192.0.2.7");
    }

    #[test]
    fn proxy_header_takes_the_rightmost_address() {
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-for", "1.1.1.1, 2.2.2.2".parse().unwrap());
        h.append("x-forwarded-for", "3.3.3.3".parse().unwrap());
        assert_eq!(
            ip_from_header(&h, "x-forwarded-for"),
            "3.3.3.3".parse().ok()
        );
        h.insert("x-real-ip", "garbage".parse().unwrap());
        assert_eq!(ip_from_header(&h, "x-real-ip"), None);
        assert_eq!(ip_from_header(&h, "missing"), None);
    }
}
