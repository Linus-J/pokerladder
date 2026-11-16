use std::sync::Arc;

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
        .connect(&config.database_url)
        .await
        .context("connecting to Postgres")?;
    db::MIGRATOR.run(&db).await.context("running migrations")?;
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

    let bind = config.bind;
    let state = AppState::new(config, db, mailer, match_config, Timing::STANDARD);
    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!(%bind, "listening");
    let app = router(state).into_make_service_with_connect_info::<std::net::SocketAddr>();
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
