//! Postgres access. Queries are checked at runtime (not with `query!`), so building
//! doesn't need a database; the integration tests exercise every query.

use sqlx::PgPool;
use sqlx::types::Json;
use time::OffsetDateTime;

use crate::protocol::{MatchId, RatingChange, UserId, WireEvent};
use crate::rating;
use crate::table::{HandRecord, MatchFinish, MatchStore, render_histories};

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

#[derive(Clone, Debug, PartialEq)]
pub struct User {
    pub id: UserId,
    pub name: String,
    pub rating: f64,
    pub email_verified: bool,
}

type UserRow = (UserId, String, f64, bool);

fn user((id, name, rating, email_verified): UserRow) -> User {
    User {
        id,
        name,
        rating,
        email_verified,
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Conflict {
    Name,
    Email,
}

pub enum CreateUser {
    Created(UserId),
    Taken(Conflict),
}

pub async fn create_user(
    db: &PgPool,
    name: &str,
    email: &str,
    password_hash: &str,
) -> anyhow::Result<CreateUser> {
    let r = sqlx::query_scalar(
        "INSERT INTO users (name, email, password_hash) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(name)
    .bind(email)
    .bind(password_hash)
    .fetch_one(db)
    .await;
    match r {
        Ok(id) => Ok(CreateUser::Created(id)),
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            Ok(CreateUser::Taken(match e.constraint() {
                Some("users_name_key") => Conflict::Name,
                _ => Conflict::Email,
            }))
        }
        Err(e) => Err(e.into()),
    }
}

/// `(id, password_hash)` for a name or email (case-insensitive).
pub async fn login_lookup(db: &PgPool, login: &str) -> anyhow::Result<Option<(UserId, String)>> {
    Ok(sqlx::query_as(
        "SELECT id, password_hash FROM users WHERE lower(name) = lower($1) OR lower(email) = lower($1)",
    )
    .bind(login)
    .fetch_optional(db)
    .await?)
}

pub async fn name_taken(db: &PgPool, name: &str) -> anyhow::Result<bool> {
    Ok(
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE lower(name) = lower($1))")
            .bind(name)
            .fetch_one(db)
            .await?,
    )
}

pub async fn user_email(db: &PgPool, id: UserId) -> anyhow::Result<String> {
    Ok(sqlx::query_scalar("SELECT email FROM users WHERE id = $1")
        .bind(id)
        .fetch_one(db)
        .await?)
}

pub async fn user_by_id(db: &PgPool, id: UserId) -> anyhow::Result<Option<User>> {
    let row: Option<UserRow> = sqlx::query_as(
        "SELECT id, name, rating, email_verified_at IS NOT NULL FROM users WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(db)
    .await?;
    Ok(row.map(user))
}

pub async fn create_session(
    db: &PgPool,
    user: UserId,
    token_hash: &[u8],
    expires_at: OffsetDateTime,
) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO sessions (token_hash, user_id, expires_at) VALUES ($1, $2, $3)")
        .bind(token_hash)
        .bind(user)
        .bind(expires_at)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn session_user(db: &PgPool, token_hash: &[u8]) -> anyhow::Result<Option<User>> {
    let row: Option<UserRow> = sqlx::query_as(
        "SELECT u.id, u.name, u.rating, u.email_verified_at IS NOT NULL
         FROM sessions s JOIN users u ON u.id = s.user_id
         WHERE s.token_hash = $1 AND s.expires_at > now()",
    )
    .bind(token_hash)
    .fetch_optional(db)
    .await?;
    Ok(row.map(user))
}

pub async fn delete_session(db: &PgPool, token_hash: &[u8]) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
        .bind(token_hash)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn create_verification(
    db: &PgPool,
    user: UserId,
    token_hash: &[u8],
    expires_at: OffsetDateTime,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO email_verifications (token_hash, user_id, expires_at) VALUES ($1, $2, $3)",
    )
    .bind(token_hash)
    .bind(user)
    .bind(expires_at)
    .execute(db)
    .await?;
    Ok(())
}

/// Consumes a verification token. Returns whether it was valid.
pub async fn verify_email(db: &PgPool, token_hash: &[u8]) -> anyhow::Result<bool> {
    let mut tx = db.begin().await?;
    let user: Option<UserId> = sqlx::query_scalar(
        "DELETE FROM email_verifications WHERE token_hash = $1 AND expires_at > now()
         RETURNING user_id",
    )
    .bind(token_hash)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(user) = user else { return Ok(false) };
    sqlx::query(
        "UPDATE users SET email_verified_at = coalesce(email_verified_at, now()) WHERE id = $1",
    )
    .bind(user)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM email_verifications WHERE user_id = $1")
        .bind(user)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(true)
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, ts_rs::TS)]
pub struct LeaderboardRow {
    pub name: String,
    pub rating: f64,
    pub rated_matches: i32,
}

pub async fn leaderboard(db: &PgPool, limit: i64) -> anyhow::Result<Vec<LeaderboardRow>> {
    let rows: Vec<(String, f64, i32)> = sqlx::query_as(
        "SELECT name, rating, rated_matches FROM users WHERE rated_matches > 0
         ORDER BY rating DESC, id LIMIT $1",
    )
    .bind(limit)
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(name, rating, rated_matches)| LeaderboardRow {
            name,
            rating,
            rated_matches,
        })
        .collect())
}

/// Creates the match row with `players[i]` in seat `i`; returns its id and the
/// players' names and current ratings, by seat.
pub async fn create_match(
    db: &PgPool,
    players: [UserId; 2],
) -> anyhow::Result<(MatchId, [String; 2], [f64; 2])> {
    let mut tx = db.begin().await?;
    let id: MatchId =
        sqlx::query_scalar("INSERT INTO matches (player0, player1) VALUES ($1, $2) RETURNING id")
            .bind(players[0])
            .bind(players[1])
            .fetch_one(&mut *tx)
            .await?;
    let mut names = [String::new(), String::new()];
    let mut ratings = [0.0; 2];
    for (i, p) in players.into_iter().enumerate() {
        let (name, rating): (String, f64) =
            sqlx::query_as("SELECT name, rating FROM users WHERE id = $1")
                .bind(p)
                .fetch_one(&mut *tx)
                .await?;
        names[i] = name;
        ratings[i] = rating;
    }
    tx.commit().await?;
    Ok((id, names, ratings))
}

pub async fn abandon_match(db: &PgPool, id: MatchId) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE matches SET status = 'abandoned', finished_at = now()
         WHERE id = $1 AND status = 'in_progress'",
    )
    .bind(id)
    .execute(db)
    .await?;
    Ok(())
}

/// Marks matches left running by a previous server process as abandoned (unrated).
pub async fn abandon_unfinished_matches(db: &PgPool) -> anyhow::Result<u64> {
    Ok(sqlx::query(
        "UPDATE matches SET status = 'abandoned', finished_at = now() WHERE status = 'in_progress'",
    )
    .execute(db)
    .await?
    .rows_affected())
}

pub struct PgStore {
    pub db: PgPool,
}

impl MatchStore for PgStore {
    async fn record_hand(&self, hand: &HandRecord) -> anyhow::Result<()> {
        let id: i64 = sqlx::query_scalar("SELECT nextval(pg_get_serial_sequence('hands', 'id'))")
            .fetch_one(&self.db)
            .await?;
        let [seat0, seat1, audit] = render_histories(id as u64, hand)?;
        let events: Vec<WireEvent> = hand.events.iter().map(WireEvent::from).collect();
        sqlx::query(
            "INSERT INTO hands (id, match_id, number, started_at, audit_events,
                                history_seat0, history_seat1, history_audit)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT (match_id, number) DO NOTHING",
        )
        .bind(id)
        .bind(hand.match_id)
        .bind(hand.number as i32)
        .bind(hand.started_at)
        .bind(Json(events))
        .bind(seat0)
        .bind(seat1)
        .bind(audit)
        .execute(&self.db)
        .await?;
        Ok(())
    }

    /// Idempotent: finishing an already-finished match returns the recorded ratings.
    async fn finish_match(&self, result: &MatchFinish) -> anyhow::Result<[RatingChange; 2]> {
        let mut tx = self.db.begin().await?;
        type Row = (
            UserId,
            UserId,
            String,
            Option<f64>,
            Option<f64>,
            Option<f64>,
            Option<f64>,
        );
        let (p0, p1, status, b0, b1, a0, a1): Row = sqlx::query_as(
            "SELECT player0, player1, status, rating_before0, rating_before1,
                    rating_after0, rating_after1
             FROM matches WHERE id = $1 FOR UPDATE",
        )
        .bind(result.match_id)
        .fetch_one(&mut *tx)
        .await?;
        if status == "finished" {
            let (Some(b0), Some(b1), Some(a0), Some(a1)) = (b0, b1, a0, a1) else {
                anyhow::bail!("finished match {} has no ratings", result.match_id);
            };
            return Ok([
                RatingChange {
                    before: b0,
                    after: a0,
                },
                RatingChange {
                    before: b1,
                    after: a1,
                },
            ]);
        }
        anyhow::ensure!(
            status == "in_progress",
            "match {} is {status}",
            result.match_id
        );
        // Lock both rating rows in id order, so concurrent finishes can't deadlock.
        let rows: Vec<(UserId, f64)> = sqlx::query_as(
            "SELECT id, rating FROM users WHERE id = ANY($1) ORDER BY id FOR UPDATE",
        )
        .bind(vec![p0, p1])
        .fetch_all(&mut *tx)
        .await?;
        let rating_of = |p: UserId| {
            rows.iter()
                .find(|r| r.0 == p)
                .map(|r| r.1)
                .ok_or_else(|| anyhow::anyhow!("user {p} vanished"))
        };
        let before = [rating_of(p0)?, rating_of(p1)?];
        let after = rating::update(before, result.winner.index());
        for (p, r) in [(p0, after[0]), (p1, after[1])] {
            sqlx::query(
                "UPDATE users SET rating = $2, rated_matches = rated_matches + 1 WHERE id = $1",
            )
            .bind(p)
            .bind(r)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query(
            "UPDATE matches SET status = 'finished', winner_seat = $2, hands_played = $3,
                    rating_before0 = $4, rating_before1 = $5,
                    rating_after0 = $6, rating_after1 = $7, finished_at = now()
             WHERE id = $1",
        )
        .bind(result.match_id)
        .bind(result.winner.index() as i16)
        .bind(result.hands_played as i32)
        .bind(before[0])
        .bind(before[1])
        .bind(after[0])
        .bind(after[1])
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok([0, 1].map(|i| RatingChange {
            before: before[i],
            after: after[i],
        }))
    }
}
