//! Authentication (spec §7.2, §13.5): argon2 password hashes, short-lived
//! JWT access tokens, rotating refresh tokens (stored hashed), HttpOnly
//! cookies for the web UI with automatic silent refresh, Bearer tokens for
//! the REST API, and "log out of all devices" via a per-user token version.

use crate::error::{AppError, AppResult};
use crate::AppState;
use argon2::password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use axum::extract::{FromRequestParts, Request, State};
use axum::http::header::{AUTHORIZATION, COOKIE, SET_COOKIE};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use chrono::{Duration, SecondsFormat, Utc};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use paycheckzero_core::Id;
use paycheckzero_storage::UserRecord;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::Instant;

pub const ACCESS_COOKIE: &str = "pz_at";
pub const REFRESH_COOKIE: &str = "pz_rt";
pub const REFRESH_TTL_DAYS: i64 = 30;
/// Default access token lifetime (seconds). Tests may shorten it.
pub const ACCESS_TTL_SECS: i64 = 15 * 60;

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    sub: String,
    tv: i64,
    exp: i64,
    iat: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: &'static str,
    pub expires_in: i64,
}

/// The authenticated user: `.0` owns the budget the data comes from, `.1`
/// is who signed in (the same user unless a member of a shared budget).
#[derive(Debug, Clone)]
pub struct AuthUser(pub UserRecord, pub UserRecord);

impl AuthUser {
    /// Budget owner's id; all data is keyed by it.
    #[must_use]
    pub fn id(&self) -> &Id {
        &self.0.id
    }

    #[must_use]
    pub fn login(&self) -> &UserRecord {
        &self.1
    }

    #[must_use]
    pub fn is_owner(&self) -> bool {
        self.1.owner_id.is_none()
    }

    /// Maps a login to the budget it uses.
    pub async fn resolve(state: &AppState, login: UserRecord) -> Option<Self> {
        match &login.owner_id {
            None => Some(Self(login.clone(), login)),
            Some(owner) => {
                let owner = state.store.user_by_id(owner).await.ok()??;
                Some(Self(owner, login))
            }
        }
    }
}

pub fn hash_password(password: &str) -> AppResult<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| AppError::Internal(e.to_string()))
}

#[must_use]
pub fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash).is_ok_and(|h| Argon2::default().verify_password(password.as_bytes(), &h).is_ok())
}

fn sha256_hex(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}

fn random_token() -> String {
    let mut b = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut b);
    hex::encode(b)
}

/// Issues a fresh access + refresh pair.
pub async fn issue(state: &AppState, user: &UserRecord) -> AppResult<Tokens> {
    let ttl = state.access_ttl();
    let now = Utc::now().timestamp();
    let claims = Claims { sub: user.id.to_string(), tv: user.token_version, iat: now, exp: now + ttl };
    let access_token = encode(&Header::default(), &claims, &EncodingKey::from_secret(&state.cfg.jwt_secret))
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let refresh_token = random_token();
    let expires = (Utc::now() + Duration::days(REFRESH_TTL_DAYS)).to_rfc3339_opts(SecondsFormat::Secs, true);
    state.store.insert_refresh(&sha256_hex(&refresh_token), &user.id, &expires).await?;
    Ok(Tokens { access_token, refresh_token, token_type: "Bearer", expires_in: ttl })
}

/// Validates an access token and its token version.
pub async fn user_from_access(state: &AppState, token: &str) -> Option<UserRecord> {
    let mut validation = Validation::default();
    validation.leeway = 0;
    let data = decode::<Claims>(token, &DecodingKey::from_secret(&state.cfg.jwt_secret), &validation).ok()?;
    let user = state.store.user_by_id(&Id::new(data.claims.sub)).await.ok()??;
    (user.token_version == data.claims.tv).then_some(user)
}

/// Rotates a refresh token: the old one stops working and a new pair is
/// issued. A token presented again within a few seconds (parallel requests
/// from one page) receives the same replacement instead of failing.
pub async fn rotate(state: &AppState, refresh_token: &str) -> AppResult<(UserRecord, Tokens)> {
    let hash = sha256_hex(refresh_token);
    if let Some((at, tokens, uid)) = state.refresh_grace.lock().await.get(&hash).cloned() {
        if at.elapsed().as_secs() < 10 {
            let user = state.store.user_by_id(&uid).await?.ok_or(AppError::Unauthorized)?;
            return Ok((user, tokens));
        }
    }
    let (uid, expires) = state.store.take_refresh(&hash).await?.ok_or(AppError::Unauthorized)?;
    let expired = chrono::DateTime::parse_from_rfc3339(&expires).map_or(true, |e| e < Utc::now());
    if expired {
        return Err(AppError::Unauthorized);
    }
    let user = state.store.user_by_id(&uid).await?.ok_or(AppError::Unauthorized)?;
    let tokens = issue(state, &user).await?;
    let mut grace = state.refresh_grace.lock().await;
    grace.retain(|_, (at, _, _)| at.elapsed().as_secs() < 10);
    grace.insert(hash, (Instant::now(), tokens.clone(), uid));
    Ok((user, tokens))
}

pub async fn revoke(state: &AppState, refresh_token: &str) -> AppResult<()> {
    state.store.delete_refresh(&sha256_hex(refresh_token)).await?;
    Ok(())
}

/// Log out of all devices: every refresh token is deleted and every access
/// token is invalidated by bumping the token version.
pub async fn revoke_all(state: &AppState, user: &Id) -> AppResult<()> {
    state.store.delete_all_refresh(user).await?;
    state.store.bump_token_version(user).await?;
    Ok(())
}

// ----------------------------------------------------------------------
// Cookies
// ----------------------------------------------------------------------

#[must_use]
pub fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers.get_all(COOKIE).iter().filter_map(|v| v.to_str().ok()).flat_map(|v| v.split(';')).find_map(|kv| {
        let (k, v) = kv.trim().split_once('=')?;
        (k == name).then(|| v.to_string())
    })
}

fn cookie(name: &str, value: &str, max_age: i64, secure: bool, same_site: &str) -> HeaderValue {
    let secure = if secure { "; Secure" } else { "" };
    HeaderValue::from_str(&format!("{name}={value}; Path=/; HttpOnly; SameSite={same_site}; Max-Age={max_age}{secure}"))
        .unwrap_or_else(|_| HeaderValue::from_static(""))
}

/// `Set-Cookie` headers carrying a token pair.
#[must_use]
pub fn session_cookies(state: &AppState, tokens: &Tokens) -> Vec<HeaderValue> {
    let secure = state.cfg.secure_cookies;
    vec![
        // The access cookie outlives the JWT slightly so an expired token is
        // still presented and triggers a silent refresh.
        cookie(ACCESS_COOKIE, &tokens.access_token, tokens.expires_in + 3600, secure, "Lax"),
        cookie(REFRESH_COOKIE, &tokens.refresh_token, REFRESH_TTL_DAYS * 86_400, secure, "Strict"),
    ]
}

#[must_use]
pub fn clear_cookies(state: &AppState) -> Vec<HeaderValue> {
    let secure = state.cfg.secure_cookies;
    vec![cookie(ACCESS_COOKIE, "", 0, secure, "Lax"), cookie(REFRESH_COOKIE, "", 0, secure, "Strict")]
}

// ----------------------------------------------------------------------
// Extractors / middleware
// ----------------------------------------------------------------------

/// REST API auth: `Authorization: Bearer <access_token>` (spec §9).
impl FromRequestParts<Arc<AppState>> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &Arc<AppState>) -> Result<Self, Self::Rejection> {
        if let Some(user) = parts.extensions.get::<AuthUser>() {
            return Ok(user.clone());
        }
        let token = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or(AppError::Unauthorized)?;
        let login = user_from_access(state, token).await.ok_or(AppError::Unauthorized)?;
        AuthUser::resolve(state, login).await.ok_or(AppError::Unauthorized)
    }
}

/// Resolves the web session from cookies, silently refreshing an expired
/// access token. Returns the user and any cookies to set.
pub async fn session_user(state: &AppState, headers: &HeaderMap) -> Option<(UserRecord, Vec<HeaderValue>)> {
    if let Some(at) = cookie_value(headers, ACCESS_COOKIE) {
        if let Some(user) = user_from_access(state, &at).await {
            return Some((user, Vec::new()));
        }
    }
    let rt = cookie_value(headers, REFRESH_COOKIE)?;
    let (user, tokens) = rotate(state, &rt).await.ok()?;
    Some((user, session_cookies(state, &tokens)))
}

/// Middleware for web UI routes. Unauthenticated page loads go to the login
/// page; Datastar requests receive a script that navigates there.
pub async fn require_session(State(state): State<Arc<AppState>>, mut req: Request, next: Next) -> Response {
    let session = match session_user(&state, req.headers()).await {
        Some((login, cookies)) => AuthUser::resolve(&state, login).await.map(|u| (u, cookies)),
        None => None,
    };
    let Some((user, cookies)) = session else {
        let datastar = req.headers().contains_key("datastar-request");
        // Just signed in but no session came back: the browser refused the cookie.
        let just_signed_in = req.uri().query().is_some_and(|q| q.split('&').any(|kv| kv == "signed_in=1"));
        let target = if just_signed_in { "/login?error=COOKIE_BLOCKED" } else { "/login" };
        if just_signed_in {
            tracing::warn!(ua = ?req.headers().get(axum::http::header::USER_AGENT), "session cookie missing right after sign-in");
        }
        let mut resp = if datastar {
            crate::sse::Sse::new().redirect(target).into_response()
        } else {
            axum::response::Redirect::to(target).into_response()
        };
        for c in clear_cookies(&state) {
            resp.headers_mut().append(SET_COOKIE, c);
        }
        return resp;
    };
    req.extensions_mut().insert(user);
    let mut resp = next.run(req).await;
    for c in cookies {
        resp.headers_mut().append(SET_COOKIE, c);
    }
    resp
}

/// CSRF defence for cookie-authenticated mutations: require the header
/// Datastar adds to every request (a cross-site form cannot set it).
pub async fn require_datastar_header(req: Request, next: Next) -> Response {
    let safe = req.method() == axum::http::Method::GET || req.method() == axum::http::Method::HEAD;
    if !safe && !req.headers().contains_key("datastar-request") && !req.headers().contains_key("x-pz-sync") {
        return (axum::http::StatusCode::FORBIDDEN, "missing request header").into_response();
    }
    next.run(req).await
}

/// CSRF defence for the sign-in forms, which must also work as plain HTML
/// form posts (no JavaScript): accept Datastar requests, or plain posts whose
/// `Origin` (or `Referer`) matches the `Host` they were sent to.
pub async fn same_origin_or_datastar(req: Request, next: Next) -> Response {
    let h = req.headers();
    if h.contains_key("datastar-request") {
        return next.run(req).await;
    }
    let host = h.get(axum::http::header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("");
    let origin = h
        .get(axum::http::header::ORIGIN)
        .or_else(|| h.get(axum::http::header::REFERER))
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let origin_host = origin.split("://").nth(1).and_then(|r| r.split('/').next()).unwrap_or("");
    if !host.is_empty() && origin_host == host {
        return next.run(req).await;
    }
    (axum::http::StatusCode::FORBIDDEN, "cross-site request refused").into_response()
}
