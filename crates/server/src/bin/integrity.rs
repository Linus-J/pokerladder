//! Chip-dumping report for one season, for a person to review before any payout.
//!
//! ```text
//! DATABASE_URL=postgres://... cargo run --release -p pokerladder-server --bin integrity -- \
//!     [--season YYYY-MM] [--all]
//! ```
//!
//! `--season` defaults to the current (UTC) month; `--all` also lists pairs without
//! flags. Read-only: it never writes to the database. See
//! `pokerladder_server::integrity` for what each signal means.

use std::str::FromStr;
use std::time::Instant;

use sqlx::postgres::PgConnectOptions;

use anyhow::{Context, bail};
use pokerladder_server::{db, integrity};

const USAGE: &str = "usage: integrity [--season YYYY-MM] [--all]";

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let mut season = None;
    let mut all = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--season" => season = Some(args.next().context(USAGE)?),
            "--all" => all = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            _ => bail!("unknown argument {arg:?}\n{USAGE}"),
        }
    }
    if let Some(s) = &season
        && !db::is_season(s)
    {
        bail!("--season must look like 2026-11, got {s:?}");
    }

    let started = Instant::now();
    let table = tokio::task::spawn_blocking(pokerladder_engine::eval::init);
    let url = std::env::var("DATABASE_URL").context("DATABASE_URL is not set")?;
    // Read-only enforced by the database, not just by this code's queries.
    let options = PgConnectOptions::from_str(&url)
        .context("parsing DATABASE_URL")?
        .options([("default_transaction_read_only", "on")]);
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect_with(options)
        .await
        .context("connecting to Postgres")?;
    let season = match season {
        Some(s) => s,
        None => {
            sqlx::query_scalar("SELECT current_season()")
                .fetch_one(&pool)
                .await?
        }
    };
    table.await?;
    let report = integrity::load::season_report(&pool, &season).await?;
    print!("{}", report.render(all));
    eprintln!("({:.1} s)", started.elapsed().as_secs_f64());
    Ok(())
}
