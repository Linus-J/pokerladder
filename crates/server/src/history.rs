//! Players' match and hand histories. A player only ever gets their own seat's view
//! of a hand (their own hole cards; the opponent's only if shown down).

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use ts_rs::TS;

use crate::AppState;
use crate::auth::{ApiError, AuthUser};
use crate::protocol::{MatchId, UserId};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MatchSummary {
    #[ts(type = "number")]
    pub id: MatchId,
    pub opponent: String,
    /// `in_progress`, `finished` or `abandoned`.
    pub status: String,
    /// `None` unless finished.
    pub won: Option<bool>,
    /// False if the match didn't count (the pair's daily quota was used up).
    pub rated: bool,
    /// The season it counted towards (`YYYY-MM`), once finished. `None` for
    /// unrated matches and for beta matches from before seasons began.
    pub season: Option<String>,
    /// Season ratings, for rated matches (beta matches: the old Elo ratings).
    pub rating_before: Option<f64>,
    pub rating_after: Option<f64>,
    pub hands_played: Option<i32>,
    #[serde(with = "time::serde::rfc3339")]
    #[ts(type = "string")]
    pub started_at: OffsetDateTime,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct HandHistory {
    pub number: i32,
    #[serde(with = "time::serde::rfc3339")]
    #[ts(type = "string")]
    pub started_at: OffsetDateTime,
    /// PokerStars-style text from this player's seat.
    pub text: String,
}

#[derive(Deserialize)]
pub struct ListParams {
    pub limit: Option<i64>,
}

type SummaryRow = (
    MatchId,
    String,
    String,
    Option<bool>,
    bool,
    Option<String>,
    Option<f64>,
    Option<f64>,
    Option<i32>,
    OffsetDateTime,
);

pub async fn list_matches(
    db: &sqlx::PgPool,
    user: UserId,
    limit: i64,
) -> anyhow::Result<Vec<MatchSummary>> {
    let rows: Vec<SummaryRow> = sqlx::query_as(
        "SELECT m.id, u.name, m.status,
                CASE WHEN m.winner_seat IS NULL THEN NULL
                     ELSE m.winner_seat = (CASE WHEN m.player0 = $1 THEN 0 ELSE 1 END) END,
                m.rated, m.season,
                CASE WHEN m.player0 = $1 THEN m.rating_before0 ELSE m.rating_before1 END,
                CASE WHEN m.player0 = $1 THEN m.rating_after0 ELSE m.rating_after1 END,
                m.hands_played, m.started_at
         FROM matches m
         JOIN users u ON u.id = CASE WHEN m.player0 = $1 THEN m.player1 ELSE m.player0 END
         WHERE $1 IN (m.player0, m.player1)
         ORDER BY m.started_at DESC, m.id DESC
         LIMIT $2",
    )
    .bind(user)
    .bind(limit)
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(
                id,
                opponent,
                status,
                won,
                rated,
                season,
                rating_before,
                rating_after,
                hands_played,
                started_at,
            )| {
                MatchSummary {
                    id,
                    opponent,
                    status,
                    won,
                    rated,
                    season,
                    rating_before,
                    rating_after,
                    hands_played,
                    started_at,
                }
            },
        )
        .collect())
}

pub async fn matches(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Query(params): Query<ListParams>,
) -> Result<Json<Vec<MatchSummary>>, ApiError> {
    let limit = params.limit.unwrap_or(50).clamp(1, 200);
    Ok(Json(list_matches(&state.db, user.id, limit).await?))
}

/// The hands of one of the user's matches, from their seat. 404 for other matches.
pub async fn hands(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(match_id): Path<MatchId>,
) -> Result<Json<Vec<HandHistory>>, ApiError> {
    let seat: Option<i32> = sqlx::query_scalar(
        "SELECT CASE WHEN player0 = $1 THEN 0 ELSE 1 END FROM matches
         WHERE id = $2 AND $1 IN (player0, player1)",
    )
    .bind(user.id)
    .bind(match_id)
    .fetch_optional(&state.db)
    .await
    .map_err(anyhow::Error::from)?;
    let Some(seat) = seat else {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "no such match",
        ));
    };
    let column = if seat == 0 {
        "history_seat0"
    } else {
        "history_seat1"
    };
    let rows: Vec<(i32, OffsetDateTime, String)> = sqlx::query_as(&format!(
        "SELECT number, started_at, {column} FROM hands WHERE match_id = $1 ORDER BY number"
    ))
    .bind(match_id)
    .fetch_all(&state.db)
    .await
    .map_err(anyhow::Error::from)?;
    Ok(Json(
        rows.into_iter()
            .map(|(number, started_at, text)| HandHistory {
                number,
                started_at,
                text,
            })
            .collect(),
    ))
}
