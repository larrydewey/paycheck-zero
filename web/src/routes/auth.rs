//! Authentication routes: register, login, refresh, logout.

use axum::{
    extract::State,
    http::StatusCode,
    routing::{post, Router},
    Json,
};

use crate::AppState;

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

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/auth/register", post(register))
        .route("/auth/login", post(login))
        .route("/auth/refresh", post(refresh))
        .route("/auth/logout", post(logout))
}

pub async fn register(
    State(_): State<AppState>,
    Json(_req): Json<RegisterRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), axum::response::Response> {
    // TODO: hash password with argon2, create user, generate tokens
    Err((
        StatusCode::NOT_IMPLEMENTED,
        axum::Json(serde_json::json!({"error": {"code": "NOT_IMPLEMENTED", "message": "auth not yet implemented"}})),
    )
        .into_response())
}

pub async fn login(
    State(_): State<AppState>,
    Json(_req): Json<LoginRequest>,
) -> Result<(StatusCode, Json<AuthResponse>), axum::response::Response> {
    // TODO: verify credentials, generate JWT tokens
    Err((
        StatusCode::NOT_IMPLEMENTED,
        axum::Json(serde_json::json!({"error": {"code": "NOT_IMPLEMENTED", "message": "auth not yet implemented"}})),
    )
        .into_response())
}

pub async fn refresh(
    State(_): State<AppState>,
    Json(_req): Json<RefreshRequest>,
) -> Result<(StatusCode, Json<AuthResponse>), axum::response::Response> {
    // TODO: validate refresh token, issue new access token
    Err((
        StatusCode::NOT_IMPLEMENTED,
        axum::Json(serde_json::json!({"error": {"code": "NOT_IMPLEMENTED", "message": "auth not yet implemented"}})),
    )
        .into_response())
}

pub async fn logout(
    State(_): State<AppState>,
) -> Result<(StatusCode, Json<serde_json::Value>), axum::response::Response> {
    // TODO: revoke refresh token
    Ok((
        StatusCode::OK,
        axum::Json(serde_json::json!({"message": "logged out"})),
    ))
}
