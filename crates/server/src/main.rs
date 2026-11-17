use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use pokerladder_engine::MatchConfig;
use pokerladder_server::config::Config;
use pokerladder_server::mail::{LogMailer, Mailer, SmtpMailer};
use pokerladder_server::table::Timing;
use pokerladder_server::{AppState, db, router};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,sqlx=warn")),
        )
        .init();
    let config = Config::from_env()?;

    // Build the 124 MiB evaluator table before accepting games, off the async threads.
    let table = tokio::task::spawn_blocking(|| {
        let t = std::time::Instant::now();
        pokerladder_engine::eval::init();
        t.elapsed()
    });

    let db = sqlx::postgres::PgPoolOptions::new()
        .max_connections(20)
        // Fail fast while the database is down (the default waits 30 s per query);
        // matches retry what they couldn't store.
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect(&config.database_url)
        .await
        .context("connecting to Postgres")?;
    db::MIGRATOR.run(&db).await.context("running migrations")?;
    // Matches that were decided but whose result was never stored are finished from
    // their audit log; only the rest are abandoned.
    match db::recover_decided_matches(&db).await {
        Ok(r) if r.is_empty() => {}
        Ok(r) => tracing::warn!(recovered = ?r, "finished decided matches from their audit log"),
        // A passing database hiccup: fail startup (systemd restarts us) rather than
        // abandon matches that could have been recovered.
        Err(e) if pokerladder_server::table::is_transient(&e) => {
            return Err(e.context("recovering decided matches"));
        }
        Err(e) => tracing::error!("couldn't recover decided matches: {e:#}"),
    }
    let abandoned = db::abandon_unfinished_matches(&db).await?;
    if abandoned > 0 {
        tracing::warn!(abandoned, "marked matches from a previous run as abandoned");
    }
    tracing::info!(elapsed = ?table.await?, "evaluator table ready");

    let match_config = MatchConfig::standard();
    // Fail fast on a bad schedule rather than in the first match.
    pokerladder_engine::Match::new(match_config.clone(), pokerladder_engine::holdem::Seat::Zero)
        .map_err(|e| anyhow::anyhow!("invalid match config: {e:?}"))?;

    let mailer: Arc<dyn Mailer> = match &config.smtp_url {
        Some(url) => {
            let smtp = SmtpMailer::new(url, &config.mail_from).context("SMTP_URL / MAIL_FROM")?;
            // A mail outage shouldn't stop the game: start anyway, loudly.
            match smtp.test_connection().await {
                Ok(true) => tracing::info!("email via SMTP"),
                Ok(false) => tracing::error!("the SMTP server refused the connection"),
                Err(e) => tracing::error!("connecting to the SMTP server failed: {e:#}"),
            }
            Arc::new(smtp)
        }
        None => {
            tracing::warn!("SMTP_URL not set: emails are only logged (development)");
            Arc::new(LogMailer)
        }
    };
    match &config.static_dir {
        Some(dir) => tracing::info!(dir = %dir.display(), "serving the web client"),
        None => tracing::warn!("no web client build found (run `npm run build` in web/)"),
    }

    let drain_timeout = match std::env::var("DRAIN_TIMEOUT_SECS") {
        Ok(s) => Duration::from_secs(s.parse().context("DRAIN_TIMEOUT_SECS")?),
        Err(_) => DEFAULT_DRAIN_TIMEOUT,
    };

    let bind = config.bind;
    let state = AppState::new(config, db, mailer, match_config, Timing::STANDARD);
    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!(%bind, "listening");
    let app = router(state.clone()).into_make_service_with_connect_info::<std::net::SocketAddr>();
    // Listening from now on, so no signal is missed (the first starts the drain, a
    // second stops at once).
    let mut signals = Signals::new()?;
    let shutdown = async move {
        signals.next().await;
        state.shutdown(drain_timeout, signals.next()).await;
    };
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await?;
    Ok(())
}

/// How long a shutdown waits for matches in progress (`DRAIN_TIMEOUT_SECS`). The
/// blinds keep rising, so every match ends.
const DEFAULT_DRAIN_TIMEOUT: Duration = Duration::from_secs(45 * 60);

/// SIGINT (Ctrl-C, and our systemd unit's KillSignal) or SIGTERM (systemd's and
/// Docker's default).
struct Signals {
    int: tokio::signal::unix::Signal,
    term: tokio::signal::unix::Signal,
}

impl Signals {
    fn new() -> anyhow::Result<Signals> {
        use tokio::signal::unix::{SignalKind, signal};
        Ok(Signals {
            int: signal(SignalKind::interrupt()).context("installing the SIGINT handler")?,
            term: signal(SignalKind::terminate()).context("installing the SIGTERM handler")?,
        })
    }

    async fn next(&mut self) {
        tokio::select! {
            _ = self.int.recv() => {}
            _ = self.term.recv() => {}
        }
    }
}
