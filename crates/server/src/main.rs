use std::sync::Arc;

use anyhow::Context;
use pokerladder_engine::MatchConfig;
use pokerladder_server::config::Config;
use pokerladder_server::mail::LogMailer;
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

    let bind = config.bind;
    let state = AppState::new(
        config,
        db,
        Arc::new(LogMailer),
        match_config,
        Timing::STANDARD,
    );
    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!(%bind, "listening");
    axum::serve(listener, router(state))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
