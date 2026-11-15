//! Tests against a real Postgres. Each `#[sqlx::test]` gets a fresh, migrated database.
//! Needs `DATABASE_URL` (see `compose.yaml` at the repository root).

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use futures_util::{SinkExt, StreamExt};
use http_body_util::BodyExt;
use pokerladder_engine::MatchConfig;
use pokerladder_engine::holdem::Seat;
use pokerladder_server::config::Config;
use pokerladder_server::db::{self, PgStore};
use pokerladder_server::mail::CaptureMailer;
use pokerladder_server::protocol::{ClientMsg, ErrorCode, ServerMsg, WireAction, WireEvent};
use pokerladder_server::ratelimit::RateLimits;
use pokerladder_server::table::{MatchFinish, MatchStore, Timing};
use pokerladder_server::{AppState, router};
use rand::RngExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio_tungstenite::tungstenite::{self, client::IntoClientRequest};
use tower::ServiceExt;

const ORIGIN: &str = "http://localhost:8080";

struct App {
    state: AppState,
    router: Router,
    mail: Arc<CaptureMailer>,
}

fn app(db: PgPool, require_verification: bool) -> App {
    pokerladder_engine::eval::init();
    let mail = Arc::new(CaptureMailer::default());
    let config = Config {
        database_url: String::new(),
        bind: "127.0.0.1:0".parse().unwrap(),
        public_url: ORIGIN.into(),
        secure_cookies: false,
        require_email_verification: require_verification,
        smtp_url: None,
        mail_from: "test <t@localhost>".into(),
        static_dir: None,
        trusted_proxy_header: None,
        rate_limits: RateLimits::STANDARD,
    };
    let timing = Timing {
        between_hands: Duration::from_millis(5),
        sit_out_delay: Duration::from_millis(5),
    };
    let state = AppState::new(config, db, mail.clone(), MatchConfig::standard(), timing);
    App {
        router: router(state.clone()),
        state,
        mail,
    }
}

struct Res {
    status: StatusCode,
    cookie: Option<String>,
    body: Value,
}

async fn call(app: &App, method: &str, path: &str, body: Value, cookie: Option<&str>) -> Res {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(c) = cookie {
        req = req.header(header::COOKIE, c);
    }
    let res = app
        .router
        .clone()
        .oneshot(req.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let cookie = res
        .headers()
        .get(header::SET_COOKIE)
        .map(|v| v.to_str().unwrap().to_owned());
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Res {
        status,
        cookie,
        body,
    }
}

/// `pl_session=...` from a Set-Cookie header.
fn cookie_pair(set_cookie: &str) -> String {
    set_cookie.split(';').next().unwrap().to_owned()
}

/// Registers, verifies and logs in; returns the session cookie.
async fn signup(app: &App, name: &str) -> String {
    let email = format!("{name}@example.com");
    let r = call(
        app,
        "POST",
        "/api/register",
        json!({"name": name, "email": email, "password": "hunter2hunter2"}),
        None,
    )
    .await;
    assert_eq!(r.status, StatusCode::ACCEPTED, "{:?}", r.body);
    let token = app.mail.token_for(&email).unwrap();
    let r = call(
        app,
        "POST",
        "/api/verify-email",
        json!({ "token": token }),
        None,
    )
    .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    login(app, name, "hunter2hunter2").await.unwrap()
}

async fn login(app: &App, login: &str, password: &str) -> Option<String> {
    let r = call(
        app,
        "POST",
        "/api/login",
        json!({"login": login, "password": password}),
        None,
    )
    .await;
    (r.status == StatusCode::OK).then(|| cookie_pair(&r.cookie.unwrap()))
}

#[sqlx::test(migrator = "pokerladder_server::db::MIGRATOR")]
async fn account_lifecycle(db: PgPool) {
    let app = app(db, true);
    let reg =
        |name: &str, email: &str, pw: &str| json!({"name": name, "email": email, "password": pw});
    let r = call(
        &app,
        "POST",
        "/api/register",
        reg("Alice", "a@x.io", "longenough"),
        None,
    )
    .await;
    assert_eq!(r.status, StatusCode::ACCEPTED);
    let accepted = r.body.clone();
    assert!(
        accepted["message"]
            .as_str()
            .unwrap()
            .contains("Check your email")
    );

    // Names and emails are unique ignoring case.
    let r = call(
        &app,
        "POST",
        "/api/register",
        reg("alice", "b@x.io", "longenough"),
        None,
    )
    .await;
    assert_eq!(
        (r.status, r.body["error"].as_str()),
        (StatusCode::CONFLICT, Some("name_taken"))
    );
    // A registered email gets exactly the same response as a new one (no account
    // enumeration); its owner is told by email instead, and no account is created.
    let r = call(
        &app,
        "POST",
        "/api/register",
        reg("Bob", "A@X.io", "longenough"),
        None,
    )
    .await;
    assert_eq!((r.status, &r.body), (StatusCode::ACCEPTED, &accepted));
    let notice = app.mail.last_to("A@X.io").unwrap();
    assert!(notice.body.contains("already has one"), "{}", notice.body);
    assert!(!notice.body.contains("token="));
    let users: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    assert_eq!(users, 1);
    // Validation.
    for (body, code) in [
        (reg("bad name", "c@x.io", "longenough"), "bad_name"),
        (reg("Bob\n", "c@x.io", "longenough"), "bad_name"),
        (reg("Bob", "nope", "longenough"), "bad_email"),
        (reg("Bob", "c@x.io", "short"), "bad_password"),
    ] {
        let r = call(&app, "POST", "/api/register", body, None).await;
        assert_eq!(
            (r.status, r.body["error"].as_str()),
            (StatusCode::BAD_REQUEST, Some(code))
        );
    }
    // A JSON content type is required (a cross-site form can't set one).
    let res = app
        .router
        .clone()
        .oneshot(
            Request::post("/api/login")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("login=Alice&password=longenough"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);

    // Login by name or email, any case; wrong passwords and unknown users fail alike.
    assert!(login(&app, "nobody", "longenough").await.is_none());
    assert!(login(&app, "Alice", "wrongpassword").await.is_none());
    assert!(login(&app, "a@X.IO", "longenough").await.is_some());
    let r = call(
        &app,
        "POST",
        "/api/login",
        json!({"login": "ALICE", "password": "longenough"}),
        None,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    let set = r.cookie.unwrap();
    for attr in ["HttpOnly", "SameSite=Strict", "Path=/", "Max-Age=2592000"] {
        assert!(set.contains(attr), "{set}");
    }
    assert!(!set.contains("Secure"), "http PUBLIC_URL: no Secure flag");
    let cookie = cookie_pair(&set);

    let r = call(&app, "GET", "/api/me", Value::Null, Some(&cookie)).await;
    assert_eq!(
        (r.status, r.body["name"].as_str()),
        (StatusCode::OK, Some("Alice"))
    );
    let r = call(
        &app,
        "GET",
        "/api/me",
        Value::Null,
        Some("pl_session=forged"),
    )
    .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);

    // Verification: a bad token fails; the real one works once.
    let r = call(
        &app,
        "POST",
        "/api/verify-email",
        json!({"token": "x"}),
        None,
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    let mail = app.mail.last_to("a@x.io").unwrap();
    assert!(
        mail.body
            .contains("http://localhost:8080/verify-email?token=")
    );
    let token = app.mail.token_for("a@x.io").unwrap();
    let token = token.as_str();
    let r = call(
        &app,
        "POST",
        "/api/verify-email",
        json!({ "token": token }),
        None,
    )
    .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    let r = call(
        &app,
        "POST",
        "/api/verify-email",
        json!({ "token": token }),
        None,
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    let r = call(&app, "GET", "/api/me", Value::Null, Some(&cookie)).await;
    assert_eq!(r.body["email_verified"], true);

    // The database holds only hashes.
    let stored: String = sqlx::query_scalar("SELECT password_hash FROM users")
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    assert!(stored.starts_with("$argon2id$"));
    let token = cookie.split('=').nth(1).unwrap();
    let raw: i64 = sqlx::query_scalar("SELECT count(*) FROM sessions WHERE token_hash = $1")
        .bind(token.as_bytes())
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    assert_eq!(raw, 0);

    // Logout kills the session server-side.
    let r = call(&app, "POST", "/api/logout", Value::Null, Some(&cookie)).await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    assert!(r.cookie.unwrap().contains("Max-Age=0"));
    let r = call(&app, "GET", "/api/me", Value::Null, Some(&cookie)).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrator = "pokerladder_server::db::MIGRATOR")]
async fn finishing_a_match_updates_ratings_once(db: PgPool) {
    let mut ids = vec![];
    for n in ["p0", "p1"] {
        let id = match db::create_user(&db, n, &format!("{n}@x.io"), "h")
            .await
            .unwrap()
        {
            db::CreateUser::Created(id) => id,
            db::CreateUser::Taken(_) => panic!(),
        };
        ids.push(id);
    }
    let (match_id, names, ratings) = db::create_match(&db, [ids[0], ids[1]]).await.unwrap();
    assert_eq!(names, ["p0", "p1"]);
    assert_eq!(ratings, [1500.0, 1500.0]);
    let store = PgStore { db: db.clone() };
    let finish = MatchFinish {
        match_id,
        winner: Seat::One,
        hands_played: 42,
    };
    let first = store.finish_match(&finish).await.unwrap();
    assert_eq!((first[0].after, first[1].after), (1484.0, 1516.0));
    // Retrying (e.g. after a lost commit acknowledgement) doesn't apply it twice.
    let again = store.finish_match(&finish).await.unwrap();
    assert_eq!(first, again);
    let rows: Vec<(f64, i32)> =
        sqlx::query_as("SELECT rating, rated_matches FROM users ORDER BY id")
            .fetch_all(&db)
            .await
            .unwrap();
    assert_eq!(rows, vec![(1484.0, 1), (1516.0, 1)]);
    let board = db::leaderboard(&db, 10).await.unwrap();
    assert_eq!(board[0].name, "p1");

    // Abandoned matches can't be finished (no rating change).
    let (m2, ..) = db::create_match(&db, [ids[0], ids[1]]).await.unwrap();
    assert_eq!(db::abandon_unfinished_matches(&db).await.unwrap(), 1);
    let finish2 = MatchFinish {
        match_id: m2,
        ..finish
    };
    assert!(store.finish_match(&finish2).await.is_err());
}

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn serve(app: &App) -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = app.router.clone();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    addr
}

async fn ws_connect(
    addr: std::net::SocketAddr,
    cookie: Option<&str>,
    origin: Option<&str>,
) -> Result<Ws, tungstenite::Error> {
    let mut req = format!("ws://{addr}/ws").into_client_request().unwrap();
    if let Some(c) = cookie {
        req.headers_mut().insert("cookie", c.parse().unwrap());
    }
    if let Some(o) = origin {
        req.headers_mut().insert("origin", o.parse().unwrap());
    }
    tokio_tungstenite::connect_async(req)
        .await
        .map(|(ws, _)| ws)
}

async fn send(ws: &mut Ws, msg: &ClientMsg) {
    let text = serde_json::to_string(msg).unwrap();
    ws.send(tungstenite::Message::text(text)).await.unwrap();
}

async fn recv(ws: &mut Ws) -> Option<ServerMsg> {
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(30), ws.next())
            .await
            .expect("timed out waiting for the server")?;
        match frame.ok()? {
            tungstenite::Message::Text(t) => return Some(serde_json::from_str(&t).unwrap()),
            tungstenite::Message::Close(_) => return None,
            _ => continue,
        }
    }
}

async fn recv_until(ws: &mut Ws, pred: impl Fn(&ServerMsg) -> bool) -> ServerMsg {
    loop {
        let m = recv(ws).await.expect("connection closed");
        if pred(&m) {
            return m;
        }
    }
}

fn status_of(e: tungstenite::Error) -> u16 {
    match e {
        tungstenite::Error::Http(res) => res.status().as_u16(),
        e => panic!("expected an HTTP rejection, got {e}"),
    }
}

#[sqlx::test(migrator = "pokerladder_server::db::MIGRATOR")]
async fn websocket_requires_session_and_same_origin(db: PgPool) {
    let app = app(db, true);
    let addr = serve(&app).await;
    let e = ws_connect(addr, None, Some(ORIGIN)).await.unwrap_err();
    assert_eq!(status_of(e), 401);
    let cookie = signup(&app, "carol").await;
    let e = ws_connect(addr, Some(&cookie), Some("https://evil.example"))
        .await
        .unwrap_err();
    assert_eq!(status_of(e), 403);
    let mut ws = ws_connect(addr, Some(&cookie), Some(ORIGIN)).await.unwrap();
    let ServerMsg::Hello { name, .. } = recv(&mut ws).await.unwrap() else {
        panic!("expected hello")
    };
    assert_eq!(name, "carol");

    // Garbage and out-of-match actions are refused politely.
    ws.send(tungstenite::Message::text("{nope")).await.unwrap();
    assert!(matches!(
        recv(&mut ws).await,
        Some(ServerMsg::Error {
            code: ErrorCode::BadMessage,
            ..
        })
    ));
    send(
        &mut ws,
        &ClientMsg::Act {
            action: WireAction::Fold,
        },
    )
    .await;
    assert!(matches!(
        recv(&mut ws).await,
        Some(ServerMsg::Error {
            code: ErrorCode::NotInMatch,
            ..
        })
    ));

    // A second connection replaces the first.
    let mut ws2 = ws_connect(addr, Some(&cookie), Some(ORIGIN)).await.unwrap();
    assert!(matches!(
        recv(&mut ws2).await,
        Some(ServerMsg::Hello { .. })
    ));
    assert!(matches!(recv(&mut ws).await, Some(ServerMsg::Replaced)));
    assert!(
        recv(&mut ws).await.is_none(),
        "replaced connection should close"
    );

    // Logout needs a JSON request (no cross-site form), then closes the live socket.
    let res = app
        .router
        .clone()
        .oneshot(
            Request::post("/api/logout")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    let r = call(&app, "POST", "/api/logout", Value::Null, Some(&cookie)).await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    assert!(
        recv(&mut ws2).await.is_none(),
        "logout should close the socket"
    );
}

#[sqlx::test(migrator = "pokerladder_server::db::MIGRATOR")]
async fn a_player_cannot_be_seated_in_two_matches(db: PgPool) {
    let app = app(db.clone(), true);
    for n in ["g1", "g2", "g3"] {
        signup(&app, n).await;
    }
    let (tx, _rx) = tokio::sync::mpsc::channel(1);
    assert!(app.state.hub.seat_players([1, 2], 999, &tx).is_some());
    let err = pokerladder_server::matchmaking::start_match(app.state.clone(), [2, 3])
        .await
        .unwrap_err();
    assert!(err.to_string().contains("seated elsewhere"), "{err}");
    let status: String = sqlx::query_scalar("SELECT status FROM matches")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(status, "abandoned");
    assert!(app.state.hub.seat(3).is_none());
    assert_eq!(app.state.hub.seat(2).unwrap().match_id, 999);
}

#[sqlx::test(migrator = "pokerladder_server::db::MIGRATOR")]
async fn unverified_players_cannot_queue(db: PgPool) {
    let app = app(db, true);
    let addr = serve(&app).await;
    let r = call(
        &app,
        "POST",
        "/api/register",
        json!({"name": "dave", "email": "d@x.io", "password": "longenough"}),
        None,
    )
    .await;
    assert_eq!(r.status, StatusCode::ACCEPTED);
    let cookie = login(&app, "dave", "longenough").await.unwrap();
    let mut ws = ws_connect(addr, Some(&cookie), None).await.unwrap();
    recv(&mut ws).await; // hello
    send(&mut ws, &ClientMsg::QueueJoin).await;
    assert!(matches!(
        recv(&mut ws).await,
        Some(ServerMsg::Error {
            code: ErrorCode::EmailNotVerified,
            ..
        })
    ));
}

/// Plays random legal actions until the match ends. Returns the winner, the rating
/// changes and every event this player received.
async fn play(mut ws: Ws) -> (u8, u8, Vec<WireEvent>, ServerMsg) {
    let first = recv_until(&mut ws, |m| matches!(m, ServerMsg::MatchState(_))).await;
    let ServerMsg::MatchState(snap) = &first else {
        unreachable!()
    };
    let me = snap.seat;
    let mut events = vec![];
    loop {
        let msg = recv(&mut ws).await.expect("connection closed mid-match");
        match msg {
            ServerMsg::Event { event, .. } => events.push(event),
            ServerMsg::Turn { turn, .. } if turn.seat == me => {
                let l = turn.legal;
                let action = {
                    let mut rng = rand::rng();
                    let mut options = vec![];
                    if l.can_check {
                        options.push(WireAction::Check);
                    }
                    if l.call.is_some() {
                        options.extend([WireAction::Call, WireAction::Fold]);
                    }
                    if let (Some(lo), Some(hi)) = (l.raise_min_to, l.raise_max_to) {
                        options.push(WireAction::RaiseTo {
                            amount: rng.random_range(lo..=hi),
                        });
                    }
                    options[rng.random_range(0..options.len())]
                };
                send(&mut ws, &ClientMsg::Act { action }).await;
            }
            ServerMsg::MatchEnded { winner, .. } => return (me, winner, events, msg),
            ServerMsg::Error { message, .. } => panic!("server refused a legal action: {message}"),
            _ => {}
        }
    }
}

#[sqlx::test(migrator = "pokerladder_server::db::MIGRATOR")]
async fn two_players_queue_play_a_match_and_get_rated(db: PgPool) {
    let app = app(db.clone(), true);
    let addr = serve(&app).await;
    let mut sockets = vec![];
    for name in ["erin", "frank"] {
        let cookie = signup(&app, name).await;
        let mut ws = ws_connect(addr, Some(&cookie), Some(ORIGIN)).await.unwrap();
        assert!(matches!(recv(&mut ws).await, Some(ServerMsg::Hello { .. })));
        send(&mut ws, &ClientMsg::QueueJoin).await;
        assert!(matches!(recv(&mut ws).await, Some(ServerMsg::Queued)));
        sockets.push(ws);
    }
    let players: Vec<_> = sockets
        .into_iter()
        .map(|ws| tokio::spawn(play(ws)))
        .collect();
    let mut results = vec![];
    for p in players {
        results.push(
            tokio::time::timeout(Duration::from_secs(120), p)
                .await
                .unwrap()
                .unwrap(),
        );
    }

    let winner = &results[0].1;
    assert_ne!(results[0].0, results[1].0, "players got different seats");
    assert_eq!(results[1].1, *winner);
    for (me, _, events, _) in &results {
        // Never the opponent's hole cards, and exactly one HoleCards per hand.
        let mut hands = 0;
        for e in events {
            match e {
                WireEvent::HoleCards { seat, .. } => assert_eq!(seat, me),
                WireEvent::HandStarted { .. } => hands += 1,
                _ => {}
            }
        }
        let holes = events
            .iter()
            .filter(|e| matches!(e, WireEvent::HoleCards { .. }))
            .count();
        assert_eq!(holes, hands);
    }

    // The database agrees: finished, rated, every hand stored with a full audit log.
    let (status, winner_seat, hands_played, b0, b1, a0, a1): (
        String,
        i16,
        i32,
        f64,
        f64,
        f64,
        f64,
    ) = sqlx::query_as(
        "SELECT status, winner_seat, hands_played, rating_before0, rating_before1,
                rating_after0, rating_after1 FROM matches",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(status, "finished");
    assert_eq!(winner_seat as u8, *winner);
    assert_eq!((b0, b1), (1500.0, 1500.0));
    assert!((a0 + a1 - 3000.0).abs() < 1e-9);
    assert_eq!(if *winner == 0 { a0 } else { a1 }, 1516.0);
    let ServerMsg::MatchEnded {
        ratings: Some(r), ..
    } = &results[0].3
    else {
        panic!("ratings should be reported")
    };
    assert_eq!((r[0].after, r[1].after), (a0, a1));

    let hands: Vec<(i32, serde_json::Value, String, String, String)> = sqlx::query_as(
        "SELECT number, audit_events, history_seat0, history_seat1, history_audit
         FROM hands ORDER BY number",
    )
    .fetch_all(&db)
    .await
    .unwrap();
    assert_eq!(hands.len(), hands_played as usize);
    let names: Vec<String> = sqlx::query_scalar(
        "SELECT u.name FROM matches m JOIN users u ON u.id IN (m.player0, m.player1)
         ORDER BY u.id = m.player1",
    )
    .fetch_all(&db)
    .await
    .unwrap();
    for (i, (number, audit, h0, h1, ha)) in hands.iter().enumerate() {
        assert_eq!(*number as usize, i + 1);
        let audit: Vec<WireEvent> = serde_json::from_value(audit.clone()).unwrap();
        let holes = audit
            .iter()
            .filter(|e| matches!(e, WireEvent::HoleCards { .. }))
            .count();
        assert_eq!(holes, 2, "audit log keeps both hands");
        assert!(ha.contains(&format!("Dealt to {}", names[0])));
        assert!(ha.contains(&format!("Dealt to {}", names[1])));
        assert!(h0.contains(&format!("Dealt to {}", names[0])));
        assert!(!h0.contains(&format!("Dealt to {}", names[1])));
        assert!(!h1.contains(&format!("Dealt to {}", names[0])));
    }
    // The players are free again (the actor unseats them just after `MatchEnded`).
    for _ in 0..100 {
        if app.state.hub.seat(1).is_none() && app.state.hub.seat(2).is_none() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("players still seated after the match");
}

#[sqlx::test(migrator = "pokerladder_server::db::MIGRATOR")]
async fn auth_endpoints_are_rate_limited(db: PgPool) {
    let app = app(db, true);
    // Per account: 10 attempts at once, whatever the password.
    for _ in 0..10 {
        let r = call(
            &app,
            "POST",
            "/api/login",
            json!({"login": "victim", "password": "guess1234"}),
            None,
        )
        .await;
        assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    }
    let r = call(
        &app,
        "POST",
        "/api/login",
        json!({"login": "VICTIM ", "password": "guess1234"}),
        None,
    )
    .await;
    assert_eq!(
        (r.status, r.body["error"].as_str()),
        (StatusCode::TOO_MANY_REQUESTS, Some("rate_limited"))
    );
    // Per IP (all requests here come from the same "unknown" address).
    let r = call(
        &app,
        "POST",
        "/api/login",
        json!({"login": "other", "password": "guess1234"}),
        None,
    )
    .await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS);
    for i in 0..10 {
        let body = json!({"name": format!("u{i}"), "email": format!("u{i}@x.io"), "password": "longenough"});
        let r = call(&app, "POST", "/api/register", body, None).await;
        assert_eq!(r.status, StatusCode::ACCEPTED);
    }
    let body = json!({"name": "u10", "email": "u10@x.io", "password": "longenough"});
    let r = call(&app, "POST", "/api/register", body, None).await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS);
}

#[sqlx::test(migrator = "pokerladder_server::db::MIGRATOR")]
async fn players_see_only_their_own_histories(db: PgPool) {
    let app = app(db.clone(), true);
    let alice = signup(&app, "alice").await;
    let bob = signup(&app, "bob").await;
    let eve = signup(&app, "eve").await;
    // Play a real match between alice (1) and bob (2).
    let addr = serve(&app).await;
    let mut sockets = vec![];
    for cookie in [&alice, &bob] {
        let mut ws = ws_connect(addr, Some(cookie), Some(ORIGIN)).await.unwrap();
        recv(&mut ws).await;
        send(&mut ws, &ClientMsg::QueueJoin).await;
        sockets.push(ws);
    }
    let players: Vec<_> = sockets
        .into_iter()
        .map(|ws| tokio::spawn(play(ws)))
        .collect();
    for p in players {
        p.await.unwrap();
    }
    let r = call(&app, "GET", "/api/matches", Value::Null, Some(&alice)).await;
    assert_eq!(r.status, StatusCode::OK);
    let list = r.body.as_array().unwrap();
    assert_eq!(list.len(), 1);
    let m = &list[0];
    assert_eq!(m["opponent"], "bob");
    assert_eq!(m["status"], "finished");
    let won = m["won"].as_bool().unwrap();
    assert_eq!(
        m["rating_after"].as_f64().unwrap(),
        if won { 1516.0 } else { 1484.0 }
    );
    let id = m["id"].as_i64().unwrap();

    let r = call(
        &app,
        "GET",
        &format!("/api/matches/{id}/hands"),
        Value::Null,
        Some(&alice),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    let hands = r.body.as_array().unwrap();
    assert_eq!(hands.len() as i64, m["hands_played"].as_i64().unwrap());
    for h in hands {
        let text = h["text"].as_str().unwrap();
        assert!(text.contains("Dealt to alice"));
        assert!(!text.contains("Dealt to bob"));
    }
    // Not a participant: indistinguishable from a match that doesn't exist.
    let r = call(
        &app,
        "GET",
        &format!("/api/matches/{id}/hands"),
        Value::Null,
        Some(&eve),
    )
    .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    let r = call(
        &app,
        "GET",
        "/api/matches/999999/hands",
        Value::Null,
        Some(&eve),
    )
    .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    let r = call(&app, "GET", "/api/matches", Value::Null, None).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    let r = call(&app, "GET", "/api/leaderboard", Value::Null, None).await;
    assert_eq!(r.body.as_array().unwrap().len(), 2);
}

#[sqlx::test(migrator = "pokerladder_server::db::MIGRATOR")]
async fn serves_the_web_client_with_security_headers(db: PgPool) {
    let dir = std::env::temp_dir().join(format!("pl-static-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    std::fs::write(dir.join("index.html"), "<!doctype html>app").unwrap();
    std::fs::write(dir.join("assets/x.js"), "1").unwrap();
    let mut a = app(db, true);
    let mut config = a.state.config.clone();
    config.static_dir = Some(dir.clone());
    let state = AppState::new(
        config,
        a.state.db.clone(),
        a.mail.clone(),
        MatchConfig::standard(),
        Timing::STANDARD,
    );
    a.router = router(state);
    let get = |path: &'static str| {
        let router = a.router.clone();
        async move {
            router
                .oneshot(Request::get(path).body(Body::empty()).unwrap())
                .await
                .unwrap()
        }
    };
    let res = get("/assets/x.js").await;
    assert_eq!(res.status(), StatusCode::OK);
    // Client-side routes fall back to index.html.
    let res = get("/verify-email?token=abc").await;
    assert_eq!(res.status(), StatusCode::OK);
    let h = res.headers().clone();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body[..], b"<!doctype html>app");
    assert!(
        h["content-security-policy"]
            .to_str()
            .unwrap()
            .contains("default-src 'self'")
    );
    assert_eq!(h["referrer-policy"], "no-referrer");
    assert_eq!(h["x-frame-options"], "DENY");
    assert_eq!(h["x-content-type-options"], "nosniff");
    // Unknown API paths are a 404, not the app.
    assert_eq!(get("/api/nope").await.status(), StatusCode::NOT_FOUND);
    std::fs::remove_dir_all(dir).unwrap();
}
