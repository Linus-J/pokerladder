//! Server configuration, from environment variables.

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, bail};

#[derive(Clone, Debug)]
pub struct Config {
    /// `DATABASE_URL`, e.g. `postgres://user:pass@localhost/pokerladder`.
    pub database_url: String,
    /// `BIND_ADDR`, default `127.0.0.1:8080`.
    pub bind: SocketAddr,
    /// `PUBLIC_URL`: the site's origin as browsers see it (scheme, host, port), e.g.
    /// `https://pokerladder.example`. Used for links in emails and to reject
    /// WebSocket connections opened by other sites. Default `http://localhost:8080`.
    pub public_url: String,
    /// Mark session cookies `Secure`. On whenever `PUBLIC_URL` is https.
    pub secure_cookies: bool,
    /// `REQUIRE_EMAIL_VERIFICATION` (default `true`): only verified accounts may queue.
    pub require_email_verification: bool,
    /// `SMTP_URL`, e.g. `smtps://user:pass@smtp.example.com:465`. Without it, emails
    /// go to the log — allowed only when `PUBLIC_URL` is plain http (development).
    pub smtp_url: Option<String>,
    /// `MAIL_FROM`, e.g. `Pokerladder <noreply@pokerladder.example>`.
    pub mail_from: String,
    /// `STATIC_DIR`: the built web client (default `web/dist`, if it exists).
    pub static_dir: Option<PathBuf>,
    /// `TRUSTED_PROXY_HEADER`, e.g. `X-Forwarded-For`: set only when a reverse proxy
    /// in front of the server always sets it; used to rate-limit by client IP.
    pub trusted_proxy_header: Option<String>,
    pub rate_limits: crate::ratelimit::RateLimits,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Config> {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let database_url = var("DATABASE_URL").context("DATABASE_URL is not set")?;
        let bind = var("BIND_ADDR")
            .unwrap_or_else(|| "127.0.0.1:8080".into())
            .parse()
            .context("BIND_ADDR")?;
        let public_url = var("PUBLIC_URL")
            .unwrap_or_else(|| "http://localhost:8080".into())
            .trim_end_matches('/')
            .to_owned();
        if !is_origin(&public_url) {
            bail!("PUBLIC_URL must be a bare origin like https://example.com[:port] (no path)");
        }
        let require_email_verification = match var("REQUIRE_EMAIL_VERIFICATION").as_deref() {
            None | Some("true" | "1") => true,
            Some("false" | "0") => false,
            Some(v) => bail!("REQUIRE_EMAIL_VERIFICATION must be true or false, not {v:?}"),
        };
        let smtp_url = var("SMTP_URL");
        if smtp_url.is_none() && public_url.starts_with("https://") {
            bail!("SMTP_URL is required in production (PUBLIC_URL is https)");
        }
        let mail_from =
            var("MAIL_FROM").unwrap_or_else(|| "Pokerladder <noreply@localhost>".into());
        let static_dir = match var("STATIC_DIR") {
            Some(d) => Some(PathBuf::from(d)),
            None => Some(PathBuf::from("web/dist")).filter(|d| d.join("index.html").exists()),
        };
        Ok(Config {
            database_url,
            bind,
            secure_cookies: public_url.starts_with("https://"),
            public_url,
            require_email_verification,
            smtp_url,
            mail_from,
            static_dir,
            trusted_proxy_header: var("TRUSTED_PROXY_HEADER"),
            rate_limits: crate::ratelimit::RateLimits::STANDARD,
        })
    }
}

/// `scheme://host[:port]` with nothing after: what browsers send as `Origin`.
fn is_origin(url: &str) -> bool {
    let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
    else {
        return false;
    };
    !rest.is_empty()
        && rest
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':' | b'[' | b']'))
}

#[cfg(test)]
mod tests {
    use super::is_origin;

    #[test]
    fn origins() {
        for ok in [
            "http://localhost:8080",
            "https://poker.example",
            "http://[::1]:80",
        ] {
            assert!(is_origin(ok), "{ok}");
        }
        for bad in [
            "",
            "localhost",
            "https://",
            "https://x.io/app",
            "https://x.io?a",
            "ftp://x",
        ] {
            assert!(!is_origin(bad), "{bad}");
        }
    }
}
