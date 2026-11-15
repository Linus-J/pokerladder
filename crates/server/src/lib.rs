//! The Pokerladder game server: accounts, matchmaking, one actor task per match, and
//! persistence. All game rules live in `pokerladder-engine`; nothing a client sends is
//! trusted.

pub mod auth;
pub mod config;
pub mod db;
pub mod hub;
pub mod mail;
pub mod matchmaking;
pub mod protocol;
pub mod rating;
pub mod table;
pub mod ws;

use std::ops::Deref;
use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::routing::{get, post};
use pokerladder_engine::MatchConfig;
use sqlx::PgPool;

use crate::config::Config;
use crate::db::PgStore;
use crate::hub::Hub;
use crate::mail::Mailer;
use crate::matchmaking::Matchmaker;
use crate::table::Timing;

#[derive(Clone)]
pub struct AppState(Arc<Inner>);

pub struct Inner {
    pub config: Config,
    pub db: PgPool,
    pub store: Arc<PgStore>,
    pub hub: Hub,
    pub matchmaker: Matchmaker,
    pub mailer: Arc<dyn Mailer>,
    pub match_config: MatchConfig,
    pub timing: Timing,
}

impl Deref for AppState {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

impl AppState {
    /// Builds the state and starts the matchmaking task. Call
    /// `pokerladder_engine::eval::init()` (on a blocking thread) before accepting games.
    pub fn new(
        config: Config,
        db: PgPool,
        mailer: Arc<dyn Mailer>,
        match_config: MatchConfig,
        timing: Timing,
    ) -> AppState {
        let (state_tx, state_rx) = tokio::sync::oneshot::channel();
        let state = AppState(Arc::new(Inner {
            config,
            store: Arc::new(PgStore { db: db.clone() }),
            db,
            hub: Hub::default(),
            matchmaker: Matchmaker::spawn(state_rx),
            mailer,
            match_config,
            timing,
        }));
        let _ = state_tx.send(state.clone());
        state
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/register", post(auth::register))
        .route("/api/verify-email", post(auth::verify_email))
        .route("/api/login", post(auth::login))
        .route("/api/logout", post(auth::logout))
        .route("/api/me", get(auth::me))
        .route("/api/leaderboard", get(leaderboard))
        .route("/ws", get(ws::handler))
        .with_state(state)
}

async fn leaderboard(
    State(state): State<AppState>,
) -> Result<axum::Json<Vec<db::LeaderboardRow>>, auth::ApiError> {
    Ok(axum::Json(db::leaderboard(&state.db, 100).await?))
}
