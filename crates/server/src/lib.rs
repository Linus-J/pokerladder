//! The Pokerladder game server: accounts, matchmaking, one actor task per match, and
//! persistence. All game rules live in `pokerladder-engine`; nothing a client sends is
//! trusted.

pub mod auth;
pub mod config;
pub mod db;
pub mod history;
pub mod hub;
pub mod integrity;
pub mod mail;
pub mod matchmaking;
pub mod protocol;
pub mod ratelimit;
pub mod rating;
pub mod table;
pub mod ws;

use std::ops::Deref;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderName, HeaderValue, StatusCode, header};
use axum::routing::{any, get, post};
use pokerladder_engine::MatchConfig;
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tower::ServiceBuilder;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;

use crate::protocol::ServerMsg;

use crate::config::Config;
use crate::db::PgStore;
use crate::hub::Hub;
use crate::mail::Mailer;
use crate::matchmaking::Matchmaker;
use crate::ratelimit::Limiters;
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
    pub limiters: Limiters,
    pub match_config: MatchConfig,
    pub timing: Timing,
    /// Cancelled when a shutdown begins (see [`AppState::drain`]).
    pub draining: CancellationToken,
    /// Every match in progress.
    pub matches: TaskTracker,
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
            limiters: Limiters::new(config.rate_limits),
            config,
            store: Arc::new(PgStore { db: db.clone() }),
            db,
            hub: Hub::default(),
            matchmaker: Matchmaker::spawn(state_rx),
            mailer,
            match_config,
            timing,
            draining: CancellationToken::new(),
            matches: TaskTracker::new(),
        }));
        let _ = state_tx.send(state.clone());
        state
    }

    /// Starts shutting down without abandoning anyone: the queue is emptied and
    /// closed, everyone online is told, and matches in progress play to the end while
    /// the server keeps accepting (re)connections. Returns once every match has
    /// finished (true) or `timeout` has passed (false; the rest are marked abandoned
    /// at the next startup).
    pub async fn drain(&self, timeout: Duration) -> bool {
        self.draining.cancel();
        for conn in self.hub.conns() {
            conn.send(ServerMsg::Restarting);
        }
        tokio::time::timeout(timeout, async {
            // Only once the queue task has exited can no further match be spawned;
            // closing the tracker before that could let `wait` return early.
            self.matchmaker.stopped().await;
            self.matches.close();
            self.matches.wait().await;
        })
        .await
        .is_ok()
    }

    /// The whole shutdown, for `axum::serve(..).with_graceful_shutdown`: drain (up to
    /// `timeout`, or until `abort` resolves), then close every WebSocket. The server
    /// keeps accepting connections until this returns, so players can reconnect to
    /// their match meanwhile.
    pub async fn shutdown(&self, timeout: Duration, abort: impl Future<Output = ()>) {
        tracing::info!(
            ?timeout,
            "shutting down: no new matches; waiting for matches in progress \
             (signal again to stop now)"
        );
        tokio::select! {
            finished = self.drain(timeout) => {
                if finished {
                    tracing::info!("all matches finished");
                } else {
                    tracing::warn!("drain timed out; unfinished matches will be abandoned");
                }
            }
            () = abort => {
                tracing::warn!("stopping now; unfinished matches will be abandoned");
            }
        }
        self.close_connections();
    }

    /// Closes every WebSocket (the last step of a shutdown, after [`drain`](Self::drain)).
    pub fn close_connections(&self) {
        for conn in self.hub.conns() {
            conn.kill.cancel();
        }
    }
}

/// Content Security Policy for the web client: everything from our own origin, plus
/// our WebSocket origin spelled out (older browsers don't count `ws(s):` as `'self'`),
/// inline styles for Svelte transitions, no framing.
fn csp(public_url: &str) -> HeaderValue {
    let ws = public_url
        .replacen("https://", "wss://", 1)
        .replacen("http://", "ws://", 1);
    let csp = format!(
        "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
         img-src 'self' data:; connect-src 'self' {ws}; frame-ancestors 'none'; \
         base-uri 'none'; form-action 'self'"
    );
    HeaderValue::from_str(&csp).expect("PUBLIC_URL was validated as an origin")
}

/// Vite's build puts content-hashed files under `assets/`: cache them for good.
const IMMUTABLE: &str = "public, max-age=31536000, immutable";

pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/api/register", post(auth::register))
        .route("/api/verify-email", post(auth::verify_email))
        .route("/api/resend-verification", post(auth::resend_verification))
        .route(
            "/api/password-reset/request",
            post(auth::request_password_reset),
        )
        .route(
            "/api/password-reset/confirm",
            post(auth::confirm_password_reset),
        )
        .route("/api/login", post(auth::login))
        .route("/api/logout", post(auth::logout))
        .route("/api/me", get(auth::me))
        .route("/api/leaderboard", get(leaderboard))
        .route("/api/seasons", get(seasons))
        .route("/api/matches", get(history::matches))
        .route("/api/matches/{id}/hands", get(history::hands))
        .route("/api/{*rest}", any(|| async { StatusCode::NOT_FOUND }))
        .route("/ws", get(ws::handler));
    let app = match &state.config.static_dir {
        // The client routes in the browser: unknown paths get index.html.
        Some(dir) => {
            // A missing asset is a 404, not index.html (which would be cached forever).
            let assets = ServiceBuilder::new()
                .layer(SetResponseHeaderLayer::overriding(
                    header::CACHE_CONTROL,
                    |r: &axum::response::Response<_>| {
                        r.status()
                            .is_success()
                            .then(|| HeaderValue::from_static(IMMUTABLE))
                    },
                ))
                .service(ServeDir::new(dir.join("assets")));
            let spa = ServeDir::new(dir).fallback(ServeFile::new(dir.join("index.html")));
            api.nest_service("/assets", assets).fallback_service(spa)
        }
        None => api,
    };
    let headers: [(HeaderName, HeaderValue); 5] = [
        (
            header::CONTENT_SECURITY_POLICY,
            csp(&state.config.public_url),
        ),
        (
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ),
        // Verification links carry a token in the URL: never leak it as a referrer.
        (
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        ),
        (header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY")),
        (header::CACHE_CONTROL, HeaderValue::from_static("no-cache")),
    ];
    let mut app = app.with_state(state);
    for (name, value) in headers {
        app = app.layer(SetResponseHeaderLayer::if_not_present(name, value));
    }
    app
}

#[derive(serde::Deserialize)]
struct SeasonQuery {
    season: Option<String>,
}

/// A season's standings: `?season=YYYY-MM`, or the current season.
async fn leaderboard(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<SeasonQuery>,
) -> Result<axum::Json<db::Standings>, auth::ApiError> {
    if let Some(s) = &q.season
        && !db::is_season(s)
    {
        return Err(auth::ApiError::new(
            StatusCode::BAD_REQUEST,
            "bad_request",
            "seasons look like 2026-11",
        ));
    }
    Ok(axum::Json(
        db::standings(&state.db, q.season.as_deref(), 100).await?,
    ))
}

/// Seasons with any rated match, newest first (always including the current one).
async fn seasons(State(state): State<AppState>) -> Result<axum::Json<Vec<String>>, auth::ApiError> {
    Ok(axum::Json(db::seasons(&state.db).await?))
}
