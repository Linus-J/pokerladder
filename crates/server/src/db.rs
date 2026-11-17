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
    /// The long-running rating, used for matchmaking only.
    pub rating: f64,
    pub email_verified: bool,
    /// This season's rating (what players see) and rated matches so far.
    pub season_rating: f64,
    pub season_matches: i32,
}

type UserRow = (UserId, String, f64, bool, f64, i32);

fn user((id, name, rating, email_verified, season_rating, season_matches): UserRow) -> User {
    User {
        id,
        name,
        rating,
        email_verified,
        season_rating,
        season_matches,
    }
}

/// The columns of a [`UserRow`], for a query over `users u`.
const USER_COLUMNS: &str = "u.id, u.name, u.rating, u.email_verified_at IS NOT NULL,
    COALESCE(sr.rating, 1500), COALESCE(sr.matches, 0)";
/// Joins the current season's rating as `sr`.
const SEASON_JOIN: &str =
    "LEFT JOIN season_ratings sr ON sr.user_id = u.id AND sr.season = current_season()";

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

/// Whether a name belongs to an account or is reserved (see [`reserve_name`]).
pub async fn name_taken(db: &PgPool, name: &str) -> anyhow::Result<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM users WHERE lower(name) = lower($1))
             OR EXISTS (SELECT 1 FROM name_reservations
                        WHERE name_lower = lower($1) AND expires_at > now())",
    )
    .bind(name)
    .fetch_one(db)
    .await?)
}

/// Holds `name` until `until` (or later, if it is already held longer).
pub async fn reserve_name(db: &PgPool, name: &str, until: OffsetDateTime) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM name_reservations WHERE expires_at <= now()")
        .execute(db)
        .await?;
    sqlx::query(
        "INSERT INTO name_reservations (name_lower, expires_at) VALUES (lower($1), $2)
         ON CONFLICT (name_lower) DO UPDATE
         SET expires_at = greatest(name_reservations.expires_at, EXCLUDED.expires_at)",
    )
    .bind(name)
    .bind(until)
    .execute(db)
    .await?;
    Ok(())
}

/// Deletes never-verified accounts older than `age` that have played no matches and
/// hold `name` or `email`, so an abandoned (or squatted) sign-up doesn't block them
/// forever. Returns how many were deleted.
pub async fn purge_stale_unverified(
    db: &PgPool,
    name: &str,
    email: &str,
    age: std::time::Duration,
) -> anyhow::Result<u64> {
    let cutoff = OffsetDateTime::now_utc() - age;
    let r = sqlx::query(
        "DELETE FROM users u
         WHERE u.email_verified_at IS NULL AND u.created_at < $3
           AND (lower(u.name) = lower($1) OR lower(u.email) = lower($2))
           AND NOT EXISTS (SELECT 1 FROM matches m WHERE m.player0 = u.id OR m.player1 = u.id)",
    )
    .bind(name)
    .bind(email)
    .bind(cutoff)
    .execute(db)
    .await?;
    Ok(r.rows_affected())
}

/// `(id, name, email as stored)` of the account with this email (case-insensitive).
pub async fn user_by_email(
    db: &PgPool,
    email: &str,
) -> anyhow::Result<Option<(UserId, String, String)>> {
    Ok(
        sqlx::query_as("SELECT id, name, email FROM users WHERE lower(email) = lower($1)")
            .bind(email)
            .fetch_optional(db)
            .await?,
    )
}

pub async fn create_password_reset(
    db: &PgPool,
    user: UserId,
    token_hash: &[u8],
    expires_at: OffsetDateTime,
) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM password_resets WHERE expires_at <= now()")
        .execute(db)
        .await?;
    sqlx::query(
        "INSERT INTO password_resets (token_hash, user_id, expires_at) VALUES ($1, $2, $3)",
    )
    .bind(token_hash)
    .bind(user)
    .bind(expires_at)
    .execute(db)
    .await?;
    Ok(())
}

/// Consumes a reset token: sets the password, marks the email verified (the link
/// proved ownership), and ends every session and outstanding reset. Returns the user,
/// or `None` if the token is invalid or expired.
pub async fn reset_password(
    db: &PgPool,
    token_hash: &[u8],
    password_hash: &str,
) -> anyhow::Result<Option<UserId>> {
    let mut tx = db.begin().await?;
    let user: Option<UserId> = sqlx::query_scalar(
        "DELETE FROM password_resets WHERE token_hash = $1 AND expires_at > now()
         RETURNING user_id",
    )
    .bind(token_hash)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(user) = user else { return Ok(None) };
    sqlx::query(
        "UPDATE users SET password_hash = $2,
                          email_verified_at = coalesce(email_verified_at, now())
         WHERE id = $1",
    )
    .bind(user)
    .bind(password_hash)
    .execute(&mut *tx)
    .await?;
    for q in [
        "DELETE FROM password_resets WHERE user_id = $1",
        "DELETE FROM sessions WHERE user_id = $1",
        "DELETE FROM email_verifications WHERE user_id = $1",
    ] {
        sqlx::query(q).bind(user).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(Some(user))
}

pub async fn user_email(db: &PgPool, id: UserId) -> anyhow::Result<String> {
    Ok(sqlx::query_scalar("SELECT email FROM users WHERE id = $1")
        .bind(id)
        .fetch_one(db)
        .await?)
}

pub async fn user_by_id(db: &PgPool, id: UserId) -> anyhow::Result<Option<User>> {
    let row: Option<UserRow> = sqlx::query_as(&format!(
        "SELECT {USER_COLUMNS} FROM users u {SEASON_JOIN} WHERE u.id = $1"
    ))
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
    let row: Option<UserRow> = sqlx::query_as(&format!(
        "SELECT {USER_COLUMNS}
         FROM sessions s JOIN users u ON u.id = s.user_id {SEASON_JOIN}
         WHERE s.token_hash = $1 AND s.expires_at > now()"
    ))
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

/// One player's line in a season's standings.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, ts_rs::TS)]
pub struct StandingRow {
    pub name: String,
    pub rating: f64,
    /// The rating's deviation, aged for idle days up to now (or the season's end).
    pub rd: f64,
    /// `rating - 2 * rd`: what qualified players are ranked by.
    pub score: f64,
    pub matches: i32,
    pub wins: i32,
    /// Has played at least [`rating::QUALIFYING_MATCHES`] rated matches this season.
    pub qualified: bool,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, ts_rs::TS)]
pub struct Standings {
    /// `YYYY-MM` (UTC calendar month).
    pub season: String,
    /// Whether this is the season in progress.
    pub current: bool,
    pub qualifying_matches: i32,
    /// Qualified players by score (best first), then everyone still qualifying by
    /// matches played.
    pub rows: Vec<StandingRow>,
}

/// Whether `s` looks like a season key (`YYYY-MM`).
pub fn is_season(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 7
        && b[4] == b'-'
        && b[..4].iter().chain(&b[5..]).all(u8::is_ascii_digit)
        && matches!(
            &s[5..],
            "01" | "02" | "03" | "04" | "05" | "06" | "07" | "08" | "09" | "10" | "11" | "12"
        )
}

/// A season's standings (the current season if `season` is `None`), at most `limit`
/// rows of each kind. `season` must satisfy [`is_season`].
pub async fn standings(
    db: &PgPool,
    season: Option<&str>,
    limit: usize,
) -> anyhow::Result<Standings> {
    let (season, current): (String, bool) = sqlx::query_as(
        "SELECT COALESCE($1, current_season()), COALESCE($1, current_season()) = current_season()",
    )
    .bind(season)
    .fetch_one(db)
    .await?;
    // Deviations age until now, or until the end of a past season.
    let rows: Vec<(String, f64, f64, f64, i32, i32, f64)> = sqlx::query_as(
        "SELECT u.name, s.rating, s.rd, s.volatility, s.matches, s.wins,
                EXTRACT(EPOCH FROM LEAST(now(),
                    ((s.season || '-01')::date + interval '1 month') AT TIME ZONE 'UTC')
                    - s.rated_at)::float8 / 86400
         FROM season_ratings s JOIN users u ON u.id = s.user_id
         WHERE s.season = $1",
    )
    .bind(&season)
    .fetch_all(db)
    .await?;
    let mut rows: Vec<StandingRow> = rows
        .into_iter()
        .map(|(name, r, rd, volatility, matches, wins, idle)| {
            let g = rating::Glicko {
                rating: r,
                rd,
                volatility,
            }
            .idle(idle);
            StandingRow {
                name,
                rating: g.rating,
                rd: g.rd,
                score: g.cautious(),
                matches,
                wins,
                qualified: matches >= rating::QUALIFYING_MATCHES,
            }
        })
        .collect();
    rows.sort_by(|a, b| {
        b.qualified
            .cmp(&a.qualified)
            .then_with(|| {
                if a.qualified {
                    b.score.total_cmp(&a.score)
                } else {
                    b.matches.cmp(&a.matches).then(b.score.total_cmp(&a.score))
                }
            })
            .then_with(|| a.name.cmp(&b.name))
    });
    let qualified = rows.iter().filter(|r| r.qualified).count();
    let mut rows: Vec<StandingRow> = rows
        .into_iter()
        .enumerate()
        .filter(|&(i, _)| i < limit.min(qualified) || (i >= qualified && i < qualified + limit))
        .map(|(_, r)| r)
        .collect();
    rows.shrink_to_fit();
    Ok(Standings {
        season,
        current,
        qualifying_matches: rating::QUALIFYING_MATCHES,
        rows,
    })
}

/// Seasons that have any rated match, newest first, always including the current one.
pub async fn seasons(db: &PgPool) -> anyhow::Result<Vec<String>> {
    Ok(sqlx::query_scalar(
        "SELECT season FROM (
             SELECT DISTINCT season FROM season_ratings
             UNION SELECT current_season()
         ) s ORDER BY season DESC",
    )
    .fetch_all(db)
    .await?)
}

/// A new match between `players` (seated in that order).
pub struct NewMatch {
    pub id: MatchId,
    pub names: [String; 2],
    /// Season ratings, for display.
    pub ratings: [f64; 2],
    /// False if the pair has already started [`rating::PAIR_DAILY_LIMIT`] rated
    /// matches in the last 24 hours (abandoned ones don't count).
    pub rated: bool,
}

/// Creates a match. Two `create_match` calls for the same pair could in principle
/// both see room in the daily quota, but a player is seated in only one match at a
/// time, so the loser of that race is abandoned at once (and abandoned matches don't
/// count): the quota holds.
pub async fn create_match(db: &PgPool, players: [UserId; 2]) -> anyhow::Result<NewMatch> {
    let mut tx = db.begin().await?;
    let recent: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM matches
         WHERE LEAST(player0, player1) = LEAST($1::bigint, $2::bigint)
           AND GREATEST(player0, player1) = GREATEST($1::bigint, $2::bigint)
           AND rated AND status <> 'abandoned'
           AND started_at > now() - interval '24 hours'",
    )
    .bind(players[0])
    .bind(players[1])
    .fetch_one(&mut *tx)
    .await?;
    let rated = recent < rating::PAIR_DAILY_LIMIT;
    let id: MatchId = sqlx::query_scalar(
        "INSERT INTO matches (player0, player1, rated) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(players[0])
    .bind(players[1])
    .bind(rated)
    .fetch_one(&mut *tx)
    .await?;
    let mut names = [String::new(), String::new()];
    let mut ratings = [0.0; 2];
    for (i, p) in players.into_iter().enumerate() {
        let (name, rating): (String, f64) = sqlx::query_as(&format!(
            "SELECT u.name, COALESCE(sr.rating, 1500) FROM users u {SEASON_JOIN} WHERE u.id = $1"
        ))
        .bind(p)
        .fetch_one(&mut *tx)
        .await?;
        names[i] = name;
        ratings[i] = rating;
    }
    tx.commit().await?;
    Ok(NewMatch {
        id,
        names,
        ratings,
        rated,
    })
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

/// At startup, before [`abandon_unfinished_matches`]: finishes every match still
/// `in_progress` whose last stored hand ends with `MatchOver` (the match was decided
/// but its result never reached the database, e.g. the database was down and the
/// server then restarted). The winner and hand count come from that audit log, so
/// nothing is guessed. Rated matches count towards the season current *now*. Returns
/// the matches finished.
pub async fn recover_decided_matches(db: &PgPool) -> anyhow::Result<Vec<MatchId>> {
    // Decoded per row: one unreadable log (e.g. written by an older version) must not
    // stop the others, or the server, starting.
    let last: Vec<(MatchId, i32, Json<serde_json::Value>)> = sqlx::query_as(
        "SELECT DISTINCT ON (h.match_id) h.match_id, h.number, h.audit_events
         FROM hands h JOIN matches m ON m.id = h.match_id
         WHERE m.status = 'in_progress'
         ORDER BY h.match_id, h.number DESC",
    )
    .fetch_all(db)
    .await?;
    let store = PgStore { db: db.clone() };
    let mut recovered = Vec::new();
    for (match_id, number, Json(events)) in last {
        let events: Vec<WireEvent> = match serde_json::from_value(events) {
            Ok(e) => e,
            Err(e) => {
                tracing::error!(match_id, "unreadable audit log, can't recover: {e}");
                continue;
            }
        };
        let winner = events.iter().find_map(|e| match e {
            WireEvent::MatchOver { winner } => crate::protocol::seat_from_u8(*winner),
            _ => None,
        });
        let Some(winner) = winner else { continue };
        let finish = MatchFinish {
            match_id,
            winner,
            hands_played: number as u32,
        };
        // One bad match shouldn't stop the server starting; it is then abandoned.
        match store.finish_match(&finish).await {
            Ok(_) => recovered.push(match_id),
            Err(e) => tracing::error!(match_id, "couldn't finish decided match: {e:#}"),
        }
    }
    Ok(recovered)
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
    async fn finish_match(
        &self,
        result: &MatchFinish,
    ) -> anyhow::Result<Option<[RatingChange; 2]>> {
        let mut tx = self.db.begin().await?;
        type Row = (
            UserId,
            UserId,
            String,
            bool,
            Option<f64>,
            Option<f64>,
            Option<f64>,
            Option<f64>,
        );
        let (p0, p1, status, rated, b0, b1, a0, a1): Row = sqlx::query_as(
            "SELECT player0, player1, status, rated, rating_before0, rating_before1,
                    rating_after0, rating_after1
             FROM matches WHERE id = $1 FOR UPDATE",
        )
        .bind(result.match_id)
        .fetch_one(&mut *tx)
        .await?;
        if status == "finished" {
            if !rated {
                return Ok(None);
            }
            let (Some(b0), Some(b1), Some(a0), Some(a1)) = (b0, b1, a0, a1) else {
                anyhow::bail!("finished match {} has no ratings", result.match_id);
            };
            return Ok(Some([
                RatingChange {
                    before: b0,
                    after: a0,
                },
                RatingChange {
                    before: b1,
                    after: a1,
                },
            ]));
        }
        anyhow::ensure!(
            status == "in_progress",
            "match {} is {status}",
            result.match_id
        );
        let winner = result.winner.index();
        let players = [p0, p1];
        if !rated {
            sqlx::query(
                "UPDATE matches SET status = 'finished', winner_seat = $2, hands_played = $3,
                        finished_at = now()
                 WHERE id = $1",
            )
            .bind(result.match_id)
            .bind(winner as i16)
            .bind(result.hands_played as i32)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            return Ok(None);
        }

        // The long-running (matchmaking) ratings. Rows are locked in id order so
        // concurrent finishes can't deadlock.
        type Rated = (UserId, f64, f64, f64, Option<f64>);
        let rows: Vec<Rated> = sqlx::query_as(
            "SELECT id, rating, rd, volatility,
                    EXTRACT(EPOCH FROM now() - rated_at)::float8 / 86400
             FROM users WHERE id = ANY($1) ORDER BY id FOR UPDATE",
        )
        .bind(players.to_vec())
        .fetch_all(&mut *tx)
        .await?;
        let lifetime = glickos(&rows, players)?;
        let after = rating::match_result(lifetime.0, lifetime.1, winner);
        ensure_finite(&after)?;
        for (p, g) in players.into_iter().zip(after) {
            sqlx::query(
                "UPDATE users SET rating = $2, rd = $3, volatility = $4, rated_at = now(),
                        rated_matches = rated_matches + 1
                 WHERE id = $1",
            )
            .bind(p)
            .bind(g.rating)
            .bind(g.rd)
            .bind(g.volatility)
            .execute(&mut *tx)
            .await?;
        }

        // The season ratings: a new player in the season starts fresh.
        let season: String = sqlx::query_scalar("SELECT current_season()")
            .fetch_one(&mut *tx)
            .await?;
        let rows: Vec<Rated> = sqlx::query_as(
            "SELECT user_id, rating, rd, volatility,
                    EXTRACT(EPOCH FROM now() - rated_at)::float8 / 86400
             FROM season_ratings WHERE season = $1 AND user_id = ANY($2)
             ORDER BY user_id FOR UPDATE",
        )
        .bind(&season)
        .bind(players.to_vec())
        .fetch_all(&mut *tx)
        .await?;
        let mut before = [rating::Glicko::NEW; 2];
        let mut idle = [0.0; 2];
        for (i, p) in players.into_iter().enumerate() {
            if let Some(&(_, r, rd, volatility, days)) = rows.iter().find(|r| r.0 == p) {
                before[i] = rating::Glicko {
                    rating: r,
                    rd,
                    volatility,
                };
                idle[i] = days.unwrap_or(0.0);
            }
        }
        let after = rating::match_result(before, idle, winner);
        ensure_finite(&after)?;
        for (i, p) in players.into_iter().enumerate() {
            let g = after[i];
            sqlx::query(
                "INSERT INTO season_ratings
                     (season, user_id, rating, rd, volatility, matches, wins, rated_at)
                 VALUES ($1, $2, $3, $4, $5, 1, $6, now())
                 ON CONFLICT (season, user_id) DO UPDATE SET
                     rating = $3, rd = $4, volatility = $5,
                     matches = season_ratings.matches + 1,
                     wins = season_ratings.wins + $6, rated_at = now()",
            )
            .bind(&season)
            .bind(p)
            .bind(g.rating)
            .bind(g.rd)
            .bind(g.volatility)
            .bind((i == winner) as i32)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query(
            "UPDATE matches SET status = 'finished', winner_seat = $2, hands_played = $3,
                    rating_before0 = $4, rating_before1 = $5,
                    rating_after0 = $6, rating_after1 = $7, season = $8, finished_at = now()
             WHERE id = $1",
        )
        .bind(result.match_id)
        .bind(winner as i16)
        .bind(result.hands_played as i32)
        .bind(before[0].rating)
        .bind(before[1].rating)
        .bind(after[0].rating)
        .bind(after[1].rating)
        .bind(&season)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(Some([0, 1].map(|i| RatingChange {
            before: before[i].rating,
            after: after[i].rating,
        })))
    }
}

/// Refuses to store a rating that isn't a number (only possible far outside realistic
/// ranges); the match then stays unfinished rather than corrupting the standings.
fn ensure_finite(g: &[rating::Glicko; 2]) -> anyhow::Result<()> {
    anyhow::ensure!(
        g.iter()
            .all(|g| g.rating.is_finite() && g.rd.is_finite() && g.volatility.is_finite()),
        "rating update produced a non-finite value: {g:?}"
    );
    Ok(())
}

/// Each player's rating and idle days from rows of `(id, rating, rd, volatility,
/// idle days)`, by seat.
fn glickos(
    rows: &[(UserId, f64, f64, f64, Option<f64>)],
    players: [UserId; 2],
) -> anyhow::Result<([rating::Glicko; 2], [f64; 2])> {
    let mut g = [rating::Glicko::NEW; 2];
    let mut idle = [0.0; 2];
    for (i, p) in players.into_iter().enumerate() {
        let &(_, rating, rd, volatility, days) = rows
            .iter()
            .find(|r| r.0 == p)
            .ok_or_else(|| anyhow::anyhow!("user {p} vanished"))?;
        g[i] = rating::Glicko {
            rating,
            rd,
            volatility,
        };
        idle[i] = days.unwrap_or(0.0);
    }
    Ok((g, idle))
}
