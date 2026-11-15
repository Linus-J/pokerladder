//! Accounts and sessions: registration, email verification, login/logout.
//!
//! - Passwords: Argon2id (default parameters), hashed on the blocking pool.
//! - Tokens (sessions, email verification): 32 bytes from the thread CSPRNG, sent to
//!   the client base64url-encoded; only their SHA-256 is stored.
//! - Sessions travel in an `HttpOnly; SameSite=Strict` cookie. Strict means browsers
//!   never attach it to cross-site requests (including WebSocket handshakes), and the
//!   JSON endpoints require `Content-Type: application/json`, which a cross-site form
//!   can't send — together that covers CSRF.

use std::sync::LazyLock;
use std::time::Duration;

use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use axum::Json;
use axum::extract::{FromRequestParts, State};
use axum::http::header::{COOKIE, SET_COOKIE};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;

use crate::AppState;
use crate::db::{self, Conflict, CreateUser, User};
use crate::mail;
use crate::protocol::UserId;
use crate::ratelimit::ClientIp;
use ts_rs::TS;

pub const SESSION_COOKIE: &str = "pl_session";
const SESSION_TTL: Duration = Duration::from_secs(30 * 24 * 3600);
const VERIFICATION_TTL: Duration = Duration::from_secs(48 * 3600);
const PASSWORD_LEN: std::ops::RangeInclusive<usize> = 8..=128;

/// API error: a status, a stable machine-readable code and a human message.
#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        ApiError {
            status,
            code,
            message: message.into(),
        }
    }

    fn unauthorized() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized", "not logged in")
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        tracing::error!("internal error: {e:#}");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "internal error",
        )
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        #[derive(Serialize)]
        struct Body<'a> {
            error: &'a str,
            message: &'a str,
        }
        let body = Json(Body {
            error: self.code,
            message: &self.message,
        });
        (self.status, body).into_response()
    }
}

pub fn valid_name(name: &str) -> bool {
    (1..=20).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// A deliberately loose check; the verification email is the real test.
pub fn valid_email(email: &str) -> bool {
    let Some((local, domain)) = email.rsplit_once('@') else {
        return false;
    };
    email.len() <= 254
        && !local.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !email.chars().any(|c| c.is_whitespace() || c.is_control())
}

pub fn new_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn token_hash(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

/// Argon2 uses ~19 MiB per hash: cap concurrent hashes at the CPU count so a burst
/// of logins queues up instead of exhausting memory.
static HASHING: LazyLock<tokio::sync::Semaphore> = LazyLock::new(|| {
    let n = std::thread::available_parallelism().map_or(2, |n| n.get());
    tokio::sync::Semaphore::new(n)
});

pub async fn hash_password(password: String) -> anyhow::Result<String> {
    let _permit = HASHING.acquire().await?;
    tokio::task::spawn_blocking(move || {
        let mut salt = [0u8; 16];
        rand::rng().fill_bytes(&mut salt);
        let salt = SaltString::encode_b64(&salt).map_err(|e| anyhow::anyhow!("salt: {e}"))?;
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map(|h| h.to_string())
            .map_err(|e| anyhow::anyhow!("hash: {e}"))
    })
    .await?
}

/// Verified against when the login name doesn't exist, so the response time doesn't
/// reveal which accounts exist.
static DUMMY_HASH: LazyLock<String> = LazyLock::new(|| {
    let salt = SaltString::encode_b64(b"pokerladder-salt").unwrap();
    Argon2::default()
        .hash_password(b"not a password", &salt)
        .unwrap()
        .to_string()
});

pub async fn verify_password(password: String, hash: Option<String>) -> anyhow::Result<bool> {
    let _permit = HASHING.acquire().await?;
    tokio::task::spawn_blocking(move || {
        let (hash, real) = match &hash {
            Some(h) => (h.as_str(), true),
            None => (DUMMY_HASH.as_str(), false),
        };
        let parsed = PasswordHash::new(hash).map_err(|e| anyhow::anyhow!("stored hash: {e}"))?;
        let ok = Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok();
        Ok(ok && real)
    })
    .await?
}

fn expires_in(ttl: Duration) -> OffsetDateTime {
    OffsetDateTime::now_utc() + ttl
}

/// The session token from the request's cookies.
pub fn session_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, _)| *k == SESSION_COOKIE)
        .map(|(_, v)| v)
}

pub async fn user_from_headers(
    state: &AppState,
    headers: &HeaderMap,
) -> anyhow::Result<Option<User>> {
    match session_token(headers) {
        Some(token) => db::session_user(&state.db, &token_hash(token)).await,
        None => Ok(None),
    }
}

/// Extractor for handlers that need a logged-in user.
pub struct AuthUser(pub User);

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        user_from_headers(state, &parts.headers)
            .await?
            .map(AuthUser)
            .ok_or_else(ApiError::unauthorized)
    }
}

fn session_cookie(state: &AppState, token: &str, max_age: Duration) -> HeaderValue {
    let secure = if state.config.secure_cookies {
        "; Secure"
    } else {
        ""
    };
    let cookie = format!(
        "{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={}{secure}",
        max_age.as_secs()
    );
    HeaderValue::from_str(&cookie).expect("cookie is ASCII")
}

#[derive(Serialize, Deserialize, TS)]
pub struct MeResponse {
    #[ts(type = "number")]
    pub id: i64,
    pub name: String,
    pub rating: f64,
    pub email_verified: bool,
    /// Whether this server only lets verified accounts queue.
    pub verification_required: bool,
}

fn me_response(state: &AppState, u: User) -> MeResponse {
    MeResponse {
        id: u.id,
        name: u.name,
        rating: u.rating,
        email_verified: u.email_verified,
        verification_required: state.config.require_email_verification,
    }
}

#[derive(Deserialize)]
pub struct RegisterRequest {
    pub name: String,
    pub email: String,
    pub password: String,
}

#[derive(Serialize, Deserialize, TS)]
pub struct RegisterResponse {
    pub message: String,
}

fn too_many() -> ApiError {
    ApiError::new(
        StatusCode::TOO_MANY_REQUESTS,
        "rate_limited",
        "too many attempts; wait a minute and try again",
    )
}

/// Creates an account and emails a verification link. An email that already has an
/// account gets the same response (and its owner a heads-up email), so this endpoint
/// doesn't reveal which emails are registered. Names are public, so a taken name is
/// reported.
pub async fn register(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    Json(req): Json<RegisterRequest>,
) -> Result<(StatusCode, Json<RegisterResponse>), ApiError> {
    if !state.limiters.register_ip.check(&ip) {
        return Err(too_many());
    }
    let bad = |code, msg: &str| ApiError::new(StatusCode::BAD_REQUEST, code, msg);
    if !valid_name(&req.name) {
        return Err(bad("bad_name", "names are 1-20 letters, digits, _ or -"));
    }
    let email = req.email.trim().to_owned();
    if !valid_email(&email) {
        return Err(bad("bad_email", "that doesn't look like an email address"));
    }
    if !PASSWORD_LEN.contains(&req.password.chars().count()) {
        return Err(bad("bad_password", "passwords are 8-128 characters"));
    }
    let name_taken = || ApiError::new(StatusCode::CONFLICT, "name_taken", "that name is taken");
    if db::name_taken(&state.db, &req.name).await? {
        return Err(name_taken());
    }
    let hash = hash_password(req.password).await?;
    match db::create_user(&state.db, &req.name, &email, &hash).await? {
        CreateUser::Created(id) => send_verification(&state, id, &email, &req.name).await?,
        CreateUser::Taken(Conflict::Name) => return Err(name_taken()),
        CreateUser::Taken(Conflict::Email) => {
            let login = format!("{}/login", state.config.public_url);
            state.mailer.send(mail::already_registered(&email, &login));
        }
    }
    let message = "Check your email for a link to confirm your account.".to_owned();
    Ok((StatusCode::ACCEPTED, Json(RegisterResponse { message })))
}

async fn send_verification(
    state: &AppState,
    user: UserId,
    email: &str,
    name: &str,
) -> anyhow::Result<()> {
    let token = new_token();
    db::create_verification(
        &state.db,
        user,
        &token_hash(&token),
        expires_in(VERIFICATION_TTL),
    )
    .await?;
    let link = format!("{}/verify-email?token={token}", state.config.public_url);
    state.mailer.send(mail::verification(email, name, &link));
    Ok(())
}

/// Emails a fresh verification link to the logged-in user (e.g. the first expired).
pub async fn resend_verification(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<StatusCode, ApiError> {
    if user.email_verified {
        return Ok(StatusCode::NO_CONTENT);
    }
    if !state
        .limiters
        .register_ip
        .check(&format!("resend:{}", user.id))
    {
        return Err(too_many());
    }
    let email = db::user_email(&state.db, user.id).await?;
    send_verification(&state, user.id, &email, &user.name).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct VerifyRequest {
    pub token: String,
}

pub async fn verify_email(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    Json(req): Json<VerifyRequest>,
) -> Result<StatusCode, ApiError> {
    if !state.limiters.verify_ip.check(&ip) {
        return Err(too_many());
    }
    if db::verify_email(&state.db, &token_hash(&req.token)).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "bad_token",
            "this link is invalid or has expired",
        ))
    }
}

#[derive(Deserialize)]
pub struct LoginRequest {
    /// Name or email.
    pub login: String,
    pub password: String,
}

pub async fn login(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    Json(req): Json<LoginRequest>,
) -> Result<Response, ApiError> {
    // Both limits are checked (and charged) before the password is.
    let ip_ok = state.limiters.login_ip.check(&ip);
    let account_ok = state
        .limiters
        .login_account
        .check(&req.login.trim().to_lowercase());
    if !(ip_ok && account_ok) {
        return Err(too_many());
    }
    if req.login.len() > 254 || req.password.len() > 4 * *PASSWORD_LEN.end() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "bad_request",
            "too long",
        ));
    }
    let found = db::login_lookup(&state.db, &req.login).await?;
    let (id, hash) = match found {
        Some((id, hash)) => (Some(id), Some(hash)),
        None => (None, None),
    };
    let ok = verify_password(req.password, hash).await?;
    let (true, Some(id)) = (ok, id) else {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "bad_credentials",
            "wrong name/email or password",
        ));
    };
    let token = new_token();
    db::create_session(&state.db, id, &token_hash(&token), expires_in(SESSION_TTL)).await?;
    let user = db::user_by_id(&state.db, id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("user {id} vanished"))?;
    let mut res = Json(me_response(&state, user)).into_response();
    res.headers_mut()
        .insert(SET_COOKIE, session_cookie(&state, &token, SESSION_TTL));
    Ok(res)
}

/// Ends the session and closes the user's live WebSocket. Like the other POST
/// endpoints it requires a JSON content type, so a cross-site form can't trigger it.
pub async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let json = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("application/json"));
    if !json {
        return Err(ApiError::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "bad_content_type",
            "expected application/json",
        ));
    }
    if let Some(token) = session_token(&headers) {
        let hash = token_hash(token);
        if let Some(user) = db::session_user(&state.db, &hash).await? {
            db::delete_session(&state.db, &hash).await?;
            if let Some(conn) = state.hub.conn(user.id) {
                conn.kill.cancel();
            }
        }
    }
    let mut res = StatusCode::NO_CONTENT.into_response();
    res.headers_mut()
        .insert(SET_COOKIE, session_cookie(&state, "", Duration::ZERO));
    Ok(res)
}

pub async fn me(State(state): State<AppState>, AuthUser(user): AuthUser) -> Json<MeResponse> {
    Json(me_response(&state, user))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        for ok in ["a", "Bob_99", "x-y", "ABCDEFGHIJKLMNOPQRST"] {
            assert!(valid_name(ok), "{ok}");
        }
        for bad in [
            "",
            "ABCDEFGHIJKLMNOPQRSTU",
            "a b",
            "a\nb",
            "é",
            "a.b",
            "<b>",
        ] {
            assert!(!valid_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn emails() {
        assert!(valid_email("a@b.co"));
        for bad in [
            "", "a", "a@b", "@b.co", "a@.co", "a@b.", "a b@c.de", "a@b.c\n",
        ] {
            assert!(!valid_email(bad), "{bad:?}");
        }
    }

    #[test]
    fn cookie_parsing() {
        let mut h = HeaderMap::new();
        h.insert(COOKIE, HeaderValue::from_static("a=1; pl_session=tok; b=2"));
        assert_eq!(session_token(&h), Some("tok"));
        h.insert(COOKIE, HeaderValue::from_static("xpl_session=no"));
        assert_eq!(session_token(&h), None);
    }

    #[test]
    fn tokens_are_distinct_and_hash_stably() {
        let (a, b) = (new_token(), new_token());
        assert_ne!(a, b);
        assert_eq!(a.len(), 43);
        assert_eq!(token_hash(&a), token_hash(&a));
        assert_ne!(token_hash(&a), token_hash(&b));
    }

    #[tokio::test]
    async fn passwords_verify_only_against_their_hash() {
        let h = hash_password("correct horse".into()).await.unwrap();
        assert!(h.starts_with("$argon2id$"));
        assert!(
            verify_password("correct horse".into(), Some(h.clone()))
                .await
                .unwrap()
        );
        assert!(
            !verify_password("wrong horse".into(), Some(h))
                .await
                .unwrap()
        );
        // The dummy path never succeeds, even with the dummy password.
        assert!(
            !verify_password("not a password".into(), None)
                .await
                .unwrap()
        );
    }
}
