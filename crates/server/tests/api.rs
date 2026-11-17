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
use pokerladder_server::rating;
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
    app_with(db, require_verification, RateLimits::STANDARD)
}

fn app_with(db: PgPool, require_verification: bool, rate_limits: RateLimits) -> App {
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
        rate_limits,
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
    let m = db::create_match(&db, [ids[0], ids[1]]).await.unwrap();
    assert_eq!(m.names, ["p0", "p1"]);
    assert_eq!(m.ratings, [1500.0, 1500.0]);
    assert!(m.rated);
    let store = PgStore { db: db.clone() };
    let finish = |match_id| MatchFinish {
        match_id,
        winner: Seat::One,
        hands_played: 42,
    };
    let expected = rating::match_result([rating::Glicko::NEW; 2], [0.0; 2], 1);
    let first = store.finish_match(&finish(m.id)).await.unwrap().unwrap();
    assert_eq!(
        (first[0].after, first[1].after),
        (expected[0].rating, expected[1].rating)
    );
    assert!(first[1].after > 1500.0 && first[0].after < 1500.0);
    // Retrying (e.g. after a lost commit acknowledgement) doesn't apply it twice.
    let again = store.finish_match(&finish(m.id)).await.unwrap().unwrap();
    assert_eq!(first, again);
    // Both the matchmaking rating and the season rating moved, once.
    let lifetime: Vec<(f64, f64, i32)> =
        sqlx::query_as("SELECT rating, rd, rated_matches FROM users ORDER BY id")
            .fetch_all(&db)
            .await
            .unwrap();
    assert_eq!(
        lifetime,
        vec![
            (expected[0].rating, expected[0].rd, 1),
            (expected[1].rating, expected[1].rd, 1)
        ]
    );
    let season: Vec<(String, f64, i32, i32, bool)> = sqlx::query_as(
        "SELECT s.season, s.rating, s.matches, s.wins, s.season = current_season()
         FROM season_ratings s ORDER BY user_id",
    )
    .fetch_all(&db)
    .await
    .unwrap();
    assert_eq!(season.len(), 2);
    assert_eq!(
        (season[0].1, season[0].2, season[0].3),
        (expected[0].rating, 1, 0)
    );
    assert_eq!(
        (season[1].1, season[1].2, season[1].3),
        (expected[1].rating, 1, 1)
    );
    assert!(season[0].4 && db::is_season(&season[0].0));
    let standings = db::standings(&db, None, 10).await.unwrap();
    assert!(standings.current);
    assert_eq!(standings.qualifying_matches, rating::QUALIFYING_MATCHES);
    // Nobody has qualified yet: everyone is listed as still qualifying.
    assert_eq!(standings.rows.len(), 2);
    assert!(
        standings
            .rows
            .iter()
            .all(|r| !r.qualified && r.matches == 1)
    );

    // Abandoned matches can't be finished, and don't use up the pair's daily quota.
    let m2 = db::create_match(&db, [ids[0], ids[1]]).await.unwrap();
    assert!(m2.rated);
    assert_eq!(db::abandon_unfinished_matches(&db).await.unwrap(), 1);
    assert!(store.finish_match(&finish(m2.id)).await.is_err());

    // The same pair: matches 2 and 3 in a day are rated, the 4th is not (in either
    // seating order), and finishing it changes no rating.
    for _ in 0..2 {
        let m = db::create_match(&db, [ids[1], ids[0]]).await.unwrap();
        assert!(m.rated);
        assert!(store.finish_match(&finish(m.id)).await.unwrap().is_some());
    }
    let before: Vec<(f64, i32)> =
        sqlx::query_as("SELECT rating, matches FROM season_ratings ORDER BY user_id")
            .fetch_all(&db)
            .await
            .unwrap();
    let m4 = db::create_match(&db, [ids[0], ids[1]]).await.unwrap();
    assert!(!m4.rated, "a pair's 4th match in 24 hours must be unrated");
    assert_eq!(store.finish_match(&finish(m4.id)).await.unwrap(), None);
    assert_eq!(store.finish_match(&finish(m4.id)).await.unwrap(), None);
    let after: Vec<(f64, i32)> =
        sqlx::query_as("SELECT rating, matches FROM season_ratings ORDER BY user_id")
            .fetch_all(&db)
            .await
            .unwrap();
    assert_eq!(before, after);
    let status: (String, bool, Option<String>) =
        sqlx::query_as("SELECT status, rated, season FROM matches WHERE id = $1")
            .bind(m4.id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(status, ("finished".into(), false, None));

    // A different pair still gets rated matches.
    let id2 = match db::create_user(&db, "p2", "p2@x.io", "h").await.unwrap() {
        db::CreateUser::Created(id) => id,
        db::CreateUser::Taken(_) => panic!(),
    };
    assert!(db::create_match(&db, [ids[0], id2]).await.unwrap().rated);

    // Matches older than a day no longer count towards the quota.
    sqlx::query("UPDATE matches SET started_at = now() - interval '25 hours'")
        .execute(&db)
        .await
        .unwrap();
    assert!(db::create_match(&db, [ids[1], ids[0]]).await.unwrap().rated);
}

#[sqlx::test(migrator = "pokerladder_server::db::MIGRATOR")]
async fn standings_rank_qualified_players_by_cautious_rating(db: PgPool) {
    let mut ids = vec![];
    for n in ["ann", "bea", "cal", "dot"] {
        let db::CreateUser::Created(id) = db::create_user(&db, n, &format!("{n}@x.io"), "h")
            .await
            .unwrap()
        else {
            panic!()
        };
        ids.push(id);
    }
    // Season rows written directly: ann and bea have qualified (bea has the higher
    // rating but is far less certain, so ann ranks first); cal and dot haven't.
    let q = rating::QUALIFYING_MATCHES;
    for (id, r, rd, matches) in [
        (ids[0], 1700.0, 50.0, q),
        (ids[1], 1800.0, 120.0, q + 5),
        (ids[2], 2000.0, 60.0, q - 1),
        (ids[3], 1600.0, 60.0, 3),
    ] {
        sqlx::query(
            "INSERT INTO season_ratings (season, user_id, rating, rd, volatility, matches, wins, rated_at)
             VALUES (current_season(), $1, $2, $3, 0.06, $4, 0, now())",
        )
        .bind(id)
        .bind(r)
        .bind(rd)
        .bind(matches)
        .execute(&db)
        .await
        .unwrap();
    }
    let s = db::standings(&db, None, 100).await.unwrap();
    let order: Vec<(&str, bool)> = s
        .rows
        .iter()
        .map(|r| (r.name.as_str(), r.qualified))
        .collect();
    assert_eq!(
        order,
        [("ann", true), ("bea", true), ("cal", false), ("dot", false)]
    );
    assert!((s.rows[0].score - (1700.0 - 2.0 * s.rows[0].rd)).abs() < 1e-9);

    // Idle time widens the deviation, so a player who stops playing slips.
    sqlx::query(
        "UPDATE season_ratings SET rated_at = now() - interval '20 days' WHERE user_id = $1",
    )
    .bind(ids[0])
    .execute(&db)
    .await
    .unwrap();
    let s = db::standings(&db, None, 100).await.unwrap();
    assert!(s.rows.iter().find(|r| r.name == "ann").unwrap().rd > 100.0);
    // ...and loses first place to bea.
    assert_eq!(s.rows[0].name, "bea");

    // A past season stops ageing at its end (midnight UTC, 1 February 2020): a row
    // last rated 12 days before has aged exactly 12 days, however long ago that was.
    sqlx::query(
        "INSERT INTO season_ratings (season, user_id, rating, rd, volatility, matches, wins, rated_at)
         VALUES ('2020-01', $1, 1600, 50, 0.06, $2, 10, '2020-01-20T00:00:00Z')",
    )
    .bind(ids[0])
    .bind(q)
    .execute(&db)
    .await
    .unwrap();
    let past = db::standings(&db, Some("2020-01"), 100).await.unwrap();
    assert!(!past.current && past.season == "2020-01");
    assert_eq!(past.rows.len(), 1);
    let expected = rating::Glicko {
        rating: 1600.0,
        rd: 50.0,
        volatility: 0.06,
    }
    .idle(12.0);
    assert!(
        (past.rows[0].rd - expected.rd).abs() < 1e-6,
        "{:?}",
        past.rows[0]
    );
    assert!(
        db::standings(&db, Some("2019-12"), 100)
            .await
            .unwrap()
            .rows
            .is_empty()
    );
    let seasons = db::seasons(&db).await.unwrap();
    assert_eq!(seasons.len(), 2);
    assert!(db::is_season(&seasons[0]) && seasons[1] == "2020-01");

    // `limit` applies to each list separately.
    let s = db::standings(&db, None, 1).await.unwrap();
    let order: Vec<(&str, bool)> = s
        .rows
        .iter()
        .map(|r| (r.name.as_str(), r.qualified))
        .collect();
    assert_eq!(order, [("bea", true), ("cal", false)]);

    // The API rejects malformed seasons and serves the standings.
    let app = app(db, true);
    let r = call(
        &app,
        "GET",
        "/api/leaderboard?season=2026-13",
        Value::Null,
        None,
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    let r = call(&app, "GET", "/api/leaderboard", Value::Null, None).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.body["rows"][0]["name"], "bea");
    assert_eq!(r.body["qualifying_matches"], q);
}

#[test]
fn season_keys() {
    for ok in ["2026-11", "2027-01", "1999-12"] {
        assert!(db::is_season(ok), "{ok}");
    }
    for bad in [
        "2026-13", "2026-00", "2026-1", "26-11", "2026/11", "2026-11x", "abcd-11",
    ] {
        assert!(!db::is_season(bad), "{bad}");
    }
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
    let mut last: Option<WireAction> = None;
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
                last = Some(action);
                send(&mut ws, &ClientMsg::Act { action }).await;
            }
            ServerMsg::MatchEnded { winner, .. } => return (me, winner, events, msg),
            // A bot acts far faster than a person and can hit the message limit:
            // back off and resend, as a client should.
            ServerMsg::Error {
                code: ErrorCode::RateLimited,
                ..
            } => {
                tokio::time::sleep(Duration::from_millis(200)).await;
                let action = last.expect("rate limited before acting");
                send(&mut ws, &ClientMsg::Act { action }).await;
            }
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
    let won = rating::match_result([rating::Glicko::NEW; 2], [0.0; 2], 0)[0].rating;
    assert_eq!(if *winner == 0 { a0 } else { a1 }, won);
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

async fn login_status(app: &App, login: &str, password: &str) -> StatusCode {
    call(
        app,
        "POST",
        "/api/login",
        json!({"login": login, "password": password}),
        None,
    )
    .await
    .status
}

#[sqlx::test(migrator = "pokerladder_server::db::MIGRATOR")]
async fn failed_logins_are_limited_per_account(db: PgPool) {
    // A generous per-IP limit, so only the per-account one is in play.
    let mut limits = RateLimits::STANDARD;
    limits.login_per_ip.burst = 1000;
    let app = app_with(db, true, limits);
    signup(&app, "victim").await;
    for _ in 0..9 {
        assert_eq!(
            login_status(&app, "victim", "guess1234").await,
            StatusCode::UNAUTHORIZED
        );
    }
    // Successful logins don't use up the allowance.
    for _ in 0..5 {
        assert_eq!(
            login_status(&app, "victim", "hunter2hunter2").await,
            StatusCode::OK
        );
    }
    assert_eq!(
        login_status(&app, "victim", "guess1234").await,
        StatusCode::UNAUTHORIZED
    );
    for (login, password) in [("VICTIM ", "guess1234"), ("victim", "hunter2hunter2")] {
        let r = call(
            &app,
            "POST",
            "/api/login",
            json!({"login": login, "password": password}),
            None,
        )
        .await;
        assert_eq!(
            (r.status, r.body["error"].as_str()),
            (StatusCode::TOO_MANY_REQUESTS, Some("rate_limited"))
        );
    }
    // Buckets follow what was typed, not the account it resolves to: otherwise, with
    // "victim" locked, a 429 on a guessed email would confirm it is victim's.
    for login in ["victim@example.com", "someone@example.com", "nobody"] {
        assert_eq!(
            login_status(&app, login, "guess1234").await,
            StatusCode::UNAUTHORIZED,
            "{login}"
        );
    }
    assert!(
        login(&app, "victim@example.com", "hunter2hunter2")
            .await
            .is_some()
    );
    // Oversized input is refused before it reaches a limiter or argon2.
    let huge = "x".repeat(10_000);
    for _ in 0..20 {
        assert_eq!(
            login_status(&app, &huge, "guess1234").await,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            login_status(&app, "nobody", &huge).await,
            StatusCode::BAD_REQUEST
        );
    }
}

#[sqlx::test(migrator = "pokerladder_server::db::MIGRATOR")]
async fn auth_endpoints_are_rate_limited(db: PgPool) {
    // Debug-build argon2 is slow enough for the standard refill to matter.
    let mut limits = RateLimits::STANDARD;
    limits.login_per_ip.every = Duration::from_secs(3600);
    limits.register_per_ip.every = Duration::from_secs(3600);
    let app = app_with(db, true, limits);
    // Per IP (all requests here come from the same "unknown" address): 10 at once,
    // whoever they are for.
    for i in 0..10 {
        assert_eq!(
            login_status(&app, &format!("n{i}"), "guess1234").await,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        login_status(&app, "other", "guess1234").await,
        StatusCode::TOO_MANY_REQUESTS
    );
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
    let [w, l] = rating::match_result([rating::Glicko::NEW; 2], [0.0; 2], 0);
    assert_eq!(
        m["rating_after"].as_f64().unwrap(),
        if won { w.rating } else { l.rating }
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
    assert_eq!(r.body["rows"].as_array().unwrap().len(), 2);
    assert_eq!(m["rated"], true);
    assert!(db::is_season(m["season"].as_str().unwrap()));
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
    assert_eq!(
        res.headers()["cache-control"],
        "public, max-age=31536000, immutable"
    );
    // A missing asset is a 404 (not index.html, which would then be cached forever).
    let res = get("/assets/gone.js").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    assert_ne!(
        res.headers().get("cache-control").unwrap(),
        "public, max-age=31536000, immutable"
    );
    // Client-side routes fall back to index.html.
    let res = get("/verify-email?token=abc").await;
    assert_eq!(res.status(), StatusCode::OK);
    let h = res.headers().clone();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body[..], b"<!doctype html>app");
    let csp = h["content-security-policy"].to_str().unwrap();
    assert!(csp.contains("default-src 'self'"));
    assert!(
        csp.contains("connect-src 'self' ws://localhost:8080;"),
        "{csp}"
    );
    assert_eq!(h["cache-control"], "no-cache");
    assert_eq!(h["referrer-policy"], "no-referrer");
    assert_eq!(h["x-frame-options"], "DENY");
    assert_eq!(h["x-content-type-options"], "nosniff");
    // Unknown API paths are a 404, not the app.
    assert_eq!(get("/api/nope").await.status(), StatusCode::NOT_FOUND);
    std::fs::remove_dir_all(dir).unwrap();
}

async fn register(app: &App, name: &str, email: &str) -> Res {
    call(
        app,
        "POST",
        "/api/register",
        json!({"name": name, "email": email, "password": "longenough"}),
        None,
    )
    .await
}

fn reset_mails(app: &App, to: &str) -> usize {
    let sent = app.mail.sent.lock().unwrap();
    sent.iter()
        .filter(|e| e.to == to && e.subject.contains("Reset"))
        .count()
}

/// Requests a reset for `email` and (since the request is handled in the
/// background) waits until `to` has received `expect` reset emails in total.
async fn request_reset(app: &App, email: &str, to: &str, expect: usize) {
    let r = call(
        app,
        "POST",
        "/api/password-reset/request",
        json!({ "email": email }),
        None,
    )
    .await;
    assert_eq!(r.status, StatusCode::ACCEPTED);
    for _ in 0..200 {
        if reset_mails(app, to) >= expect {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(reset_mails(app, to), expect, "reset emails to {to}");
}

#[sqlx::test(migrator = "pokerladder_server::db::MIGRATOR")]
async fn password_reset(db: PgPool) {
    let app = app(db, true);
    let old = signup(&app, "alice").await;
    // Unknown addresses get the same answer and no email.
    let sent = app.mail.sent.lock().unwrap().len();
    request_reset(&app, "nobody@example.com", "nobody@example.com", 0).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(app.mail.sent.lock().unwrap().len(), sent);
    request_reset(&app, " ALICE@example.com ", "alice@example.com", 1).await;
    let token = app.mail.token_for("alice@example.com").unwrap();

    let confirm = |token: String, password: &'static str| {
        let app = &app;
        async move {
            call(
                app,
                "POST",
                "/api/password-reset/confirm",
                json!({"token": token, "password": password}),
                None,
            )
            .await
            .status
        }
    };
    assert_eq!(
        confirm(token.clone(), "short").await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        confirm("bogus".into(), "newpassword").await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        confirm(token.clone(), "newpassword").await,
        StatusCode::NO_CONTENT
    );
    // Single use; old sessions and the old password are gone.
    assert_eq!(confirm(token, "another-one").await, StatusCode::BAD_REQUEST);
    let r = call(&app, "GET", "/api/me", Value::Null, Some(&old)).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert!(login(&app, "alice", "hunter2hunter2").await.is_none());
    assert!(login(&app, "alice", "newpassword").await.is_some());

    // A reset proves the address, so it verifies an unverified account (this is how
    // someone recovers an email that was registered by somebody else).
    assert_eq!(
        register(&app, "squatter", "bob@example.com").await.status,
        StatusCode::ACCEPTED
    );
    request_reset(&app, "bob@example.com", "bob@example.com", 1).await;
    let token = app.mail.token_for("bob@example.com").unwrap();
    assert_eq!(
        confirm(token, "bobs-password").await,
        StatusCode::NO_CONTENT
    );
    let cookie = login(&app, "bob@example.com", "bobs-password")
        .await
        .unwrap();
    let r = call(&app, "GET", "/api/me", Value::Null, Some(&cookie)).await;
    assert_eq!(r.body["email_verified"], true);
}

#[sqlx::test(migrator = "pokerladder_server::db::MIGRATOR")]
async fn registration_does_not_reveal_emails(db: PgPool) {
    let app = app(db.clone(), true);
    signup(&app, "alice").await;
    // A sign-up with a registered email and a new name: same answer as a fresh one...
    let taken = register(&app, "probe", "alice@example.com").await;
    let fresh = register(&app, "probe2", "new@example.com").await;
    assert_eq!((taken.status, &taken.body), (fresh.status, &fresh.body));
    // ...and the name then looks taken either way.
    for name in ["probe", "PROBE2"] {
        let r = register(&app, name, "other@example.com").await;
        assert_eq!(
            (r.status, r.body["error"].as_str()),
            (StatusCode::CONFLICT, Some("name_taken")),
            "{name}"
        );
    }
    assert!(
        app.mail
            .last_to("alice@example.com")
            .unwrap()
            .subject
            .contains("sign-up attempt")
    );
    // Still the same later on: past a day (the reservation must outlast the
    // unverified account, which can't be purged for 48 hours)...
    let age = |hours: i32| {
        let db = db.clone();
        async move {
            for q in [
                "UPDATE users SET created_at = created_at - make_interval(hours => $1)",
                "UPDATE name_reservations SET expires_at = expires_at - make_interval(hours => $1)",
            ] {
                sqlx::query(q).bind(hours).execute(&db).await.unwrap();
            }
        }
    };
    age(30).await;
    for (name, email) in [("probe", "o2@example.com"), ("probe2", "o3@example.com")] {
        assert_eq!(
            register(&app, name, email).await.status,
            StatusCode::CONFLICT,
            "{name} after 30 h"
        );
    }
    // ...and once both have lapsed, both names are free again.
    age(24).await;
    for (name, email) in [("probe", "o4@example.com"), ("probe2", "o5@example.com")] {
        assert_eq!(
            register(&app, name, email).await.status,
            StatusCode::ACCEPTED,
            "{name} after 54 h"
        );
    }
}

#[sqlx::test(migrator = "pokerladder_server::db::MIGRATOR")]
async fn stale_unverified_accounts_give_up_their_name_and_email(db: PgPool) {
    let app = app(db.clone(), true);
    assert_eq!(
        register(&app, "squat", "victim@example.com").await.status,
        StatusCode::ACCEPTED
    );
    // Fresh: still holds both.
    assert_eq!(
        register(&app, "squat", "x@example.com").await.status,
        StatusCode::CONFLICT
    );
    // Real accounts are never purged: verified ones, and unverified ones that have
    // played (matches reference them).
    signup(&app, "verified").await;
    register(&app, "player", "player@example.com").await;
    sqlx::query(
        "INSERT INTO matches (player0, player1)
         SELECT p.id, v.id FROM users p, users v WHERE p.name = 'player' AND v.name = 'verified'",
    )
    .execute(&db)
    .await
    .unwrap();
    sqlx::query("UPDATE users SET created_at = now() - interval '3 days'")
        .execute(&db)
        .await
        .unwrap();
    // A taken name is a 409; a taken email looks like success (202) whether or not it
    // freed the address, so the users table is checked below.
    for (name, email, status) in [
        ("verified", "a@example.com", StatusCode::CONFLICT),
        ("b", "verified@example.com", StatusCode::ACCEPTED),
        ("player", "c@example.com", StatusCode::CONFLICT),
        ("d", "player@example.com", StatusCode::ACCEPTED),
    ] {
        assert_eq!(
            register(&app, name, email).await.status,
            status,
            "{name} / {email}"
        );
    }
    let r = register(&app, "victim", "victim@example.com").await;
    assert_eq!(r.status, StatusCode::ACCEPTED);
    let mail = app.mail.last_to("victim@example.com").unwrap();
    assert!(mail.subject.contains("Confirm"), "{}", mail.subject);
    let mut names: Vec<String> = sqlx::query_scalar("SELECT name FROM users")
        .fetch_all(&db)
        .await
        .unwrap();
    names.sort();
    assert_eq!(names, ["player", "verified", "victim"]);
}

#[sqlx::test(migrator = "pokerladder_server::db::MIGRATOR")]
async fn emails_per_recipient_are_capped(db: PgPool) {
    let app = app(db, true);
    signup(&app, "alice").await; // 1 email
    let count = || {
        app.mail
            .sent
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.to == "alice@example.com")
            .count()
    };
    // Anyone can trigger "already registered" notices: one per 6 hours gets through.
    for i in 0..5 {
        register(&app, &format!("p{i}"), "alice@example.com").await;
    }
    assert_eq!(count(), 2);
    // ...and they don't use up what the owner asks for: 2 reset emails left of 3.
    for n in 1..=2 {
        request_reset(&app, "alice@example.com", "alice@example.com", n).await;
    }
    let r = call(
        &app,
        "POST",
        "/api/password-reset/request",
        json!({"email": "alice@example.com"}),
        None,
    )
    .await;
    assert_eq!(r.status, StatusCode::ACCEPTED);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        reset_mails(&app, "alice@example.com"),
        2,
        "capped at 3 in all"
    );
}

#[sqlx::test(migrator = "pokerladder_server::db::MIGRATOR")]
async fn websocket_floods_are_refused_then_disconnected(db: PgPool) {
    let app = app(db, true);
    let cookie = signup(&app, "flood").await;
    let addr = serve(&app).await;
    let mut ws = ws_connect(addr, Some(&cookie), Some(ORIGIN)).await.unwrap();
    assert!(matches!(
        recv(&mut ws).await,
        Some(ServerMsg::Hello {
            email_verified: true,
            in_match: false,
            ..
        })
    ));
    // A little over the burst: the excess is refused, the connection stays up.
    for _ in 0..30 {
        ws.send(tungstenite::Message::text("{}")).await.unwrap();
    }
    recv_until(&mut ws, |m| {
        matches!(
            m,
            ServerMsg::Error {
                code: ErrorCode::RateLimited,
                ..
            }
        )
    })
    .await;
    // Keep going and the server hangs up (queued replies may be cut off, so only
    // the close is checked).
    for _ in 0..300 {
        if ws.send(tungstenite::Message::text("{}")).await.is_err() {
            break;
        }
    }
    while recv(&mut ws).await.is_some() {}
}

/// Connects, returning the socket and the user id from `Hello`.
async fn connect_user(
    addr: std::net::SocketAddr,
    cookie: &str,
) -> (Ws, pokerladder_server::protocol::UserId) {
    let mut ws = ws_connect(addr, Some(cookie), Some(ORIGIN)).await.unwrap();
    let Some(ServerMsg::Hello { user_id, .. }) = recv(&mut ws).await else {
        panic!("expected hello")
    };
    (ws, user_id)
}

#[sqlx::test(migrator = "pokerladder_server::db::MIGRATOR")]
async fn shutdown_lets_matches_finish_and_starts_no_new_ones(db: PgPool) {
    let app = app(db, true);
    // Served the way main.rs does it: the shutdown runs inside the graceful-shutdown
    // future, so the server keeps accepting connections while it drains.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn({
        let router = app.router.clone();
        let state = app.state.clone();
        async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async move {
                    let _ = stop_rx.await;
                    state
                        .shutdown(Duration::from_secs(120), std::future::pending())
                        .await;
                })
                .await
                .unwrap()
        }
    });

    let cookies: Vec<String> = {
        let mut v = vec![];
        for name in ["gina", "hank", "ivy", "jack"] {
            v.push(signup(&app, name).await);
        }
        v
    };
    let (mut gina, gina_id) = connect_user(addr, &cookies[0]).await;
    let (mut hank, _) = connect_user(addr, &cookies[1]).await;
    send(&mut gina, &ClientMsg::QueueJoin).await;
    send(&mut hank, &ClientMsg::QueueJoin).await;
    let mut seated = false;
    for _ in 0..500 {
        if app.state.hub.seat(gina_id).is_some() {
            seated = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(seated, "gina and hank weren't paired within 5 s");
    // Nobody acts yet, so the match can't end before the checks below (each
    // decision has a 15 s clock).

    // Ivy is waiting in the queue when the shutdown begins.
    let (mut ivy, ivy_id) = connect_user(addr, &cookies[2]).await;
    send(&mut ivy, &ClientMsg::QueueJoin).await;
    assert!(matches!(recv(&mut ivy).await, Some(ServerMsg::Queued)));

    stop_tx.send(()).unwrap();
    // The queue task exits promptly, and the running match holds up the drain.
    tokio::time::timeout(Duration::from_secs(2), app.state.matchmaker.stopped())
        .await
        .expect("the queue should close when the shutdown begins");
    assert!(
        !app.state.drain(Duration::from_millis(50)).await,
        "a running match must hold up the shutdown"
    );

    // Ivy is told, and taken out of the queue.
    let mut told = (false, false);
    while told != (true, true) {
        match recv(&mut ivy).await.expect("ivy disconnected") {
            ServerMsg::Restarting => told.0 = true,
            ServerMsg::QueueLeft => told.1 = true,
            _ => {}
        }
    }
    // Jack connects during the drain (new connections are still accepted): he hears
    // about the restart straight after Hello, and can't queue.
    let (mut jack, jack_id) = connect_user(addr, &cookies[3]).await;
    assert!(matches!(recv(&mut jack).await, Some(ServerMsg::Restarting)));
    for ws in [&mut ivy, &mut jack] {
        send(ws, &ClientMsg::QueueJoin).await;
        // (`Restarting` may repeat: each drain call broadcasts it.)
        loop {
            match recv(ws).await.expect("disconnected") {
                ServerMsg::Restarting => {}
                ServerMsg::QueueLeft => break,
                m => panic!("expected the join to be refused, got {m:?}"),
            }
        }
    }
    // Ivy and Jack would pair at once if the queue were open: they never do.
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert!(app.state.hub.seat(ivy_id).is_none());
    assert!(app.state.hub.seat(jack_id).is_none());

    // Now the bots play: the match runs to a normal, rated finish...
    for p in [tokio::spawn(play(gina)), tokio::spawn(play(hank))] {
        let (_, _, _, end) = tokio::time::timeout(Duration::from_secs(120), p)
            .await
            .unwrap()
            .unwrap();
        let ServerMsg::MatchEnded { ratings, .. } = end else {
            unreachable!()
        };
        assert!(
            ratings.is_some(),
            "the match should be rated, not abandoned"
        );
    }
    // ...after which the server closes every socket and stops.
    tokio::time::timeout(Duration::from_secs(10), server)
        .await
        .expect("the server should stop once the match is over")
        .unwrap();
    while recv(&mut ivy).await.is_some() {}
    while recv(&mut jack).await.is_some() {}
}
