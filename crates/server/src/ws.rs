//! The `/ws` endpoint: one WebSocket per logged-in user carries the queue and the
//! match. A new connection replaces the previous one (which is told `Replaced`).

use std::time::Duration;

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::http::header::ORIGIN;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::AppState;
use crate::auth::{ApiError, user_from_headers};
use crate::db::{self, User};
use crate::protocol::{ClientMsg, ErrorCode, ServerMsg};
use crate::table::{Conn, MatchCmd};

/// Outgoing messages a connection may have queued before it counts as too slow.
const OUTBOX: usize = 256;
const PING_EVERY: Duration = Duration::from_secs(20);
/// A connection that sends nothing (not even a pong) for this long is dead.
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_MESSAGE: usize = 4096;
/// Client messages: a burst of 20, refilling at 10 per second. Far above what a
/// person clicking buttons sends; beyond it messages are refused, and a client that
/// keeps going is disconnected.
const MSG_BURST: f64 = 20.0;
const MSG_PER_SEC: f64 = 10.0;
const MAX_REFUSED: u32 = 100;
/// A send that takes longer means the client has stopped reading: drop it.
const SEND_TIMEOUT: Duration = Duration::from_secs(10);

pub async fn handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    // Browsers always send Origin on WebSocket handshakes; reject other sites. (Their
    // requests carry no session cookie anyway, thanks to SameSite=Strict.)
    if let Some(origin) = headers.get(ORIGIN)
        && origin.as_bytes() != state.config.public_url.as_bytes()
    {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "bad_origin",
            "cross-site request",
        ));
    }
    let Some(user) = user_from_headers(&state, &headers).await? else {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "not logged in",
        ));
    };
    Ok(ws
        .max_message_size(MAX_MESSAGE)
        .on_upgrade(move |socket| run(state, user, socket))
        .into_response())
}

async fn run(state: AppState, user: User, socket: WebSocket) {
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::channel::<ServerMsg>(OUTBOX);
    let conn = Conn {
        id: state.hub.next_conn_id(),
        tx,
        kill: CancellationToken::new(),
    };

    let kill = conn.kill.clone();
    let writer = tokio::spawn(async move {
        let mut ping = tokio::time::interval(PING_EVERY);
        loop {
            let msg = tokio::select! {
                biased;
                m = rx.recv() => match m {
                    Some(m) => m,
                    None => break,
                },
                () = kill.cancelled() => {
                    // Flush what's queued (e.g. `Replaced`), then close.
                    while let Ok(m) = rx.try_recv() {
                        if send(&mut sink, &m).await.is_err() {
                            break;
                        }
                    }
                    break;
                }
                _ = ping.tick() => {
                    let ping = sink.send(Message::Ping(Default::default()));
                    if !matches!(tokio::time::timeout(SEND_TIMEOUT, ping).await, Ok(Ok(()))) {
                        break;
                    }
                    continue;
                }
            };
            if send(&mut sink, &msg).await.is_err() || matches!(msg, ServerMsg::Replaced) {
                break;
            }
        }
        kill.cancel();
        let _ = tokio::time::timeout(SEND_TIMEOUT, sink.close()).await;
    });

    let (old, seat) = state.hub.connect(user.id, conn.clone());
    // Queued before the actor's MatchState (which only follows the Connect below).
    conn.send(ServerMsg::Hello {
        user_id: user.id,
        name: user.name.clone(),
        rating: user.season_rating,
        season_matches: user.season_matches,
        email_verified: user.email_verified,
        in_match: seat.is_some(),
    });
    if state.draining.is_cancelled() {
        conn.send(ServerMsg::Restarting);
    }
    if let Some(old) = old {
        old.send(ServerMsg::Replaced);
        old.kill.cancel();
    }
    if let Some(s) = seat {
        let conn = conn.clone();
        let _ = s.cmd.send(MatchCmd::Connect { seat: s.seat, conn }).await;
    }

    let (mut tokens, mut refilled, mut refused) = (MSG_BURST, tokio::time::Instant::now(), 0u32);
    loop {
        let frame = tokio::select! {
            () = conn.kill.cancelled() => break,
            f = tokio::time::timeout(IDLE_TIMEOUT, stream.next()) => f,
        };
        match frame {
            Ok(Some(Ok(Message::Text(text)))) => {
                let now = tokio::time::Instant::now();
                tokens = (tokens + (now - refilled).as_secs_f64() * MSG_PER_SEC).min(MSG_BURST);
                refilled = now;
                if tokens >= 1.0 {
                    tokens -= 1.0;
                    refused = 0;
                    on_message(&state, &user, &conn, &text).await;
                } else {
                    refused += 1;
                    if refused > MAX_REFUSED {
                        break;
                    }
                    conn.send(error(ErrorCode::RateLimited, "slow down"));
                }
            }
            Ok(Some(Ok(Message::Binary(_)))) => {
                conn.send(error(ErrorCode::BadMessage, "text frames only"));
            }
            Ok(Some(Ok(Message::Ping(_) | Message::Pong(_)))) => {}
            Ok(Some(Ok(Message::Close(_)) | Err(_)) | None) | Err(_) => break,
        }
    }

    conn.kill.cancel();
    let (current, seat) = state.hub.disconnect(user.id, conn.id);
    if current {
        state.matchmaker.leave(user.id);
    }
    if let Some(s) = seat {
        let msg = MatchCmd::Disconnect {
            seat: s.seat,
            conn_id: conn.id,
        };
        let _ = s.cmd.send(msg).await;
    }
    // Every send in the writer is bounded, so this wait is too; abort as a backstop.
    let mut writer = writer;
    if tokio::time::timeout(SEND_TIMEOUT * 3, &mut writer)
        .await
        .is_err()
    {
        writer.abort();
    }
}

async fn send(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    msg: &ServerMsg,
) -> Result<(), axum::Error> {
    let text = serde_json::to_string(msg).expect("server messages always serialise");
    match tokio::time::timeout(SEND_TIMEOUT, sink.send(Message::Text(text.into()))).await {
        Ok(r) => r,
        Err(_) => Err(axum::Error::new("send timed out")),
    }
}

fn error(code: ErrorCode, message: &str) -> ServerMsg {
    ServerMsg::Error {
        code,
        message: message.to_owned(),
    }
}

async fn on_message(state: &AppState, user: &User, conn: &Conn, text: &str) {
    let msg = match serde_json::from_str::<ClientMsg>(text) {
        Ok(m) => m,
        Err(_) => {
            conn.send(error(ErrorCode::BadMessage, "unrecognised message"));
            return;
        }
    };
    match msg {
        ClientMsg::QueueJoin => {
            if state.draining.is_cancelled() {
                conn.send(ServerMsg::Restarting);
                conn.send(ServerMsg::QueueLeft);
                return;
            }
            if state.hub.seat(user.id).is_some() {
                conn.send(error(ErrorCode::AlreadyInMatch, "finish your match first"));
                return;
            }
            // Fresh from the database: the user may have verified or been rated since
            // connecting.
            let fresh = match db::user_by_id(&state.db, user.id).await {
                Ok(Some(u)) => u,
                Ok(None) | Err(_) => {
                    conn.send(error(ErrorCode::Unavailable, "try again shortly"));
                    return;
                }
            };
            if state.config.require_email_verification && !fresh.email_verified {
                conn.send(error(
                    ErrorCode::EmailNotVerified,
                    "verify your email first",
                ));
                return;
            }
            // `Queued` goes first so that a `QueueLeft` from a shutdown always follows it.
            conn.send(ServerMsg::Queued);
            if !state.matchmaker.join(user.id, fresh.rating) {
                // A shutdown began while we looked the user up.
                conn.send(ServerMsg::Restarting);
                conn.send(ServerMsg::QueueLeft);
            }
        }
        ClientMsg::QueueLeave => {
            state.matchmaker.leave(user.id);
            conn.send(ServerMsg::QueueLeft);
        }
        ClientMsg::Act { action } => match state.hub.seat(user.id) {
            Some(s) => {
                let action = action.into();
                let _ = s
                    .cmd
                    .send(MatchCmd::Act {
                        seat: s.seat,
                        action,
                    })
                    .await;
            }
            None => {
                conn.send(error(ErrorCode::NotInMatch, "you are not in a match"));
            }
        },
        ClientMsg::SetAutoMuck { on } => match state.hub.seat(user.id) {
            Some(s) => {
                let _ = s.cmd.send(MatchCmd::SetAutoMuck { seat: s.seat, on }).await;
            }
            None => {
                conn.send(error(ErrorCode::NotInMatch, "you are not in a match"));
            }
        },
    }
}
