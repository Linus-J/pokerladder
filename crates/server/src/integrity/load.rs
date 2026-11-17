//! Reads a season's matches and hands from Postgres into an [`Analyzer`].

use futures_util::TryStreamExt;
use sqlx::PgPool;
use sqlx::types::Json;

use super::{Analyzer, MatchInfo, Report};
use crate::protocol::{MatchId, UserId, WireEvent};

/// The season a match belongs to: the one it was rated in, otherwise the UTC month
/// it started in (unrated and abandoned matches have no `season`).
const MATCH_SEASON: &str =
    "COALESCE(m.season, to_char(m.started_at AT TIME ZONE 'UTC', 'YYYY-MM'))";

/// Builds the report for `season` ('YYYY-MM'). Needs the evaluator table
/// (`pokerladder_engine::eval::init`), and runs the analysis on the calling thread:
/// call it from a blocking-friendly context (the `integrity` binary does).
pub async fn season_report(db: &PgPool, season: &str) -> anyhow::Result<Report> {
    let mut a = Analyzer::new();

    type MatchRow = (
        MatchId,
        UserId,
        UserId,
        String,
        Option<i16>,
        Option<i32>,
        bool,
        Option<f64>,
        Option<f64>,
    );
    let rows: Vec<MatchRow> = sqlx::query_as(&format!(
        "SELECT m.id, m.player0, m.player1, m.status, m.winner_seat, m.hands_played,
                m.rated, m.rating_before0, m.rating_before1
         FROM matches m WHERE {MATCH_SEASON} = $1 ORDER BY m.id"
    ))
    .bind(season)
    .fetch_all(db)
    .await?;
    let mut players: Vec<UserId> = Vec::new();
    for (id, p0, p1, status, winner_seat, hands_played, rated, b0, b1) in rows {
        players.extend([p0, p1]);
        a.add_match(MatchInfo {
            id,
            players: [p0, p1],
            status,
            winner_seat,
            hands_played,
            rated,
            ratings_before: b0.zip(b1).map(|(x, y)| [x, y]),
        });
    }
    players.sort_unstable();
    players.dedup();
    let names: Vec<(UserId, String)> =
        sqlx::query_as("SELECT id, name FROM users WHERE id = ANY($1)")
            .bind(&players)
            .fetch_all(db)
            .await?;
    for (id, name) in names {
        a.add_name(id, name);
    }

    // Stream hands so a whole season never sits in memory at once. A hand whose JSON
    // doesn't parse as events is skipped (listed in the report), like any other
    // unreadable hand.
    let sql = format!(
        "SELECT h.id, h.match_id, h.number, h.audit_events
         FROM hands h JOIN matches m ON m.id = h.match_id
         WHERE {MATCH_SEASON} = $1 ORDER BY h.match_id, h.number"
    );
    let mut hands = sqlx::query_as::<_, (i64, MatchId, i32, Json<serde_json::Value>)>(&sql)
        .bind(season)
        .fetch(db);
    while let Some((id, match_id, number, Json(value))) = hands.try_next().await? {
        match serde_json::from_value::<Vec<WireEvent>>(value) {
            Ok(events) => a.add_hand(id, match_id, number, &events),
            Err(e) => a.skip(id, format!("unreadable events: {e}")),
        }
    }
    Ok(a.finish(season))
}
