//! Authentication routes: register, login, refresh, logout.

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use axum::{
    Json,
    extract::State,
    http::{HeaderValue, StatusCode},
    routing::post,
};
use axum::extract::DefaultBodyLimit;
use axum::Router;

use paycheckzero_core::Id;
use paycheckzero_storage::AuthRepository;

use crate::AppState;
use crate::service::error::ServiceError;
use crate::JwtConfig;

/// Access-token cookie for browser/BSS flows (Datastar sends it same-origin).
const ACCESS_COOKIE: &str = "pz_access";

fn access_cookie(token: &str) -> HeaderValue {
    HeaderValue::from_str(&format!(
        "{ACCESS_COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict"
    ))
    .expect("cookie value is valid header")
}

fn clear_access_cookie() -> HeaderValue {
    HeaderValue::from_static("pz_access=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0")
}

#[derive(serde::Deserialize)]
pub struct RegisterRequest {
    pub email: String,
    pub password: String,
    pub full_name: Option<String>,
}

#[derive(serde::Serialize)]
pub struct AuthResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: &'static str,
    pub expires_in: u64,
}

#[derive(serde::Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(serde::Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

#[derive(serde::Deserialize)]
pub struct LogoutRequest {
    pub refresh_token: Option<String>,
    /// When true, revoke every refresh token for the user (all devices).
    #[serde(default)]
    pub all_devices: bool,
}

fn router_with_limit() -> Router<AppState> {
    Router::new()
        .route("/auth/register", post(register))
        .route("/auth/login", post(login))
        .route("/auth/refresh", post(refresh))
        .route("/auth/logout", post(logout))
        .layer(DefaultBodyLimit::max(16 * 1024))
}

/// Router for the auth API. Mounted at `/api/v1`.
pub fn router() -> Router<AppState> {
    router_with_limit()
}

fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}

fn validate_credentials(email: &str, password: &str) -> Result<(), ServiceError> {
    if !email.contains('@') || email.len() < 3 || email.len() > 254 {
        return Err(ServiceError::Conflict(
            "invalid email address".to_string(),
        ));
    }
    if password.len() < 8 {
        return Err(ServiceError::Conflict(
            "password must be at least 8 characters".to_string(),
        ));
    }
    if password.len() > 100 {
        return Err(ServiceError::Conflict(
            "password must be at most 100 characters".to_string(),
        ));
    }
    Ok(())
}

fn hash_password(password: &str) -> Result<String, ServiceError> {
    let salt = SaltString::encode_b64(&uuid::Uuid::new_v4().into_bytes())
        .map_err(|e| ServiceError::Internal(format!("salt generation failed: {e}")))?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|p| p.to_string())
        .map_err(|e| ServiceError::Internal(format!("password hashing failed: {e}")))
}

fn verify_password(password: &str, stored: &str) -> Result<(), ServiceError> {
    let parsed = PasswordHash::new(stored)
        .map_err(|_| ServiceError::Unauthorized("invalid email or password".into()))?;
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .map_err(|_| ServiceError::Unauthorized("invalid email or password".into()))
}

fn issue_tokens(state: &AppState, user_id: &str) -> Result<AuthResponse, ServiceError> {
    let access_token = state
        .config
        .access_token(user_id)
        .map_err(|e| ServiceError::Internal(e.to_string()))?;
    let (refresh_token, hash) = state
        .config
        .refresh_token(user_id)
        .map_err(|e| ServiceError::Internal(e.to_string()))?;
    let created_at = chrono::Utc::now().to_rfc3339();
    let expires_at = (chrono::Utc::now()
        + chrono::Duration::from_std(state.config.refresh_ttl).unwrap_or_else(|_| {
            chrono::Duration::days(30)
        }))
    .to_rfc3339();
    state
        .db
        .lock()
        .unwrap()
        .save_refresh_token(&hash, &Id::new(user_id), "web", expires_at.as_str(), created_at.as_str())
        .map_err(ServiceError::Storage)?;
    Ok(AuthResponse {
        access_token,
        refresh_token,
        token_type: "bearer",
        expires_in: state.config.access_ttl.as_secs(),
    })
}

pub async fn register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> Result<(StatusCode, [(&'static str, HeaderValue); 1], Json<AuthResponse>), ServiceError> {
    let email = normalize_email(&req.email);
    validate_credentials(&email, &req.password)?;
    let id = Id::new(uuid::Uuid::new_v4().to_string());
    let hash = hash_password(&req.password)?;
    state
        .db
        .lock()
        .unwrap()
        .register_user(&id, &email, &hash)
        .map_err(|e| match e {
            paycheckzero_storage::StorageError::UniqueViolation(_) => {
                ServiceError::Conflict("email already registered".into())
            }
            other => ServiceError::Storage(other),
        })?;
    let tokens = issue_tokens(&state, id.as_str())?;
    let cookie = access_cookie(&tokens.access_token);
    Ok((
        StatusCode::CREATED,
        [("set-cookie", cookie)],
        Json(tokens),
    ))
}

pub async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<(StatusCode, [(&'static str, HeaderValue); 1], Json<AuthResponse>), ServiceError> {
    let email = normalize_email(&req.email);
    let user = state
        .db
        .lock()
        .unwrap()
        .user_by_email(&email)
        .map_err(ServiceError::Storage)?
        .ok_or_else(|| ServiceError::Unauthorized("invalid email or password".into()))?;
    verify_password(&req.password, &user.password_hash)?;
    let tokens = issue_tokens(&state, user.id.as_str())?;
    let cookie = access_cookie(&tokens.access_token);
    Ok((StatusCode::OK, [("set-cookie", cookie)], Json(tokens)))
}

pub async fn refresh(
    State(state): State<AppState>,
    Json(req): Json<RefreshRequest>,
) -> Result<(StatusCode, [(&'static str, HeaderValue); 1], Json<AuthResponse>), ServiceError> {
    let claims = state
        .config
        .decode_refresh(&req.refresh_token)
        .map_err(|_| ServiceError::Unauthorized("invalid or expired refresh token".into()))?;
    let hash = JwtConfig::hash_refresh(&req.refresh_token);
    let record = state
        .db
        .lock()
        .unwrap()
        .refresh_token(&hash)
        .map_err(ServiceError::Storage)?
        .ok_or_else(|| ServiceError::Unauthorized("refresh token revoked".into()))?;
    let expires_at = chrono::DateTime::parse_from_rfc3339(&record.expires_at)
        .map(|d| d.with_timezone(&chrono::Utc))
        .unwrap_or_else(|_| chrono::Utc::now());
    if expires_at < chrono::Utc::now() {
        state
            .db
            .lock()
            .unwrap()
            .delete_refresh_token(&hash)
            .map_err(ServiceError::Storage)?;
        return Err(ServiceError::Unauthorized("refresh token expired".into()));
    }
    let mut db = state.db.lock().unwrap();
    db.delete_refresh_token(&hash)
        .map_err(ServiceError::Storage)?;
    drop(db);
    let tokens = issue_tokens(&state, &claims.sub)?;
    let cookie = access_cookie(&tokens.access_token);
    Ok((StatusCode::OK, [("set-cookie", cookie)], Json(tokens)))
}

pub async fn logout(
    State(state): State<AppState>,
    body: Option<Json<LogoutRequest>>,
) -> Result<(StatusCode, [(&'static str, HeaderValue); 1]), ServiceError> {
    let req = body
        .map(|b| b.0)
        .unwrap_or(LogoutRequest { refresh_token: None, all_devices: false });
    let clearcookie = clear_access_cookie();
    if req.all_devices {
        let token = req.refresh_token.as_ref().ok_or_else(|| {
            ServiceError::Conflict("refresh_token required to identify user".into())
        })?;
        let claims = state
            .config
            .decode_refresh(token)
            .map_err(|_| ServiceError::Unauthorized("invalid refresh token".into()))?;
        state
            .db
            .lock()
            .unwrap()
            .delete_all_refresh_tokens(&Id::new(&claims.sub))
            .map_err(ServiceError::Storage)?;
        return Ok((StatusCode::NO_CONTENT, [("set-cookie", clearcookie)]));
    }
    let token = req.refresh_token.as_ref().ok_or_else(|| {
        ServiceError::Conflict("refresh_token required".into())
    })?;
    let hash = JwtConfig::hash_refresh(token);
    state
        .db
        .lock()
        .unwrap()
        .delete_refresh_token(&hash)
        .map_err(ServiceError::Storage)?;
    Ok((StatusCode::NO_CONTENT, [("set-cookie", clearcookie)]))
}