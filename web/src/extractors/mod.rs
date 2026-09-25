//! Custom extractors for auth.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::session::SessionError;
use crate::{AppState, UserId};

fn unauthorized(message: &str) -> Response {
    let body = axum::Json(serde_json::json!({
        "error": { "code": "UNAUTHORIZED", "message": message }
    }));
    (StatusCode::UNAUTHORIZED, body).into_response()
}

/// Extract a Bearer token from the `Authorization` header or `pz_access` cookie.
fn extract_token(parts: &mut Parts) -> Option<String> {
    use axum::http::header::{AUTHORIZATION, COOKIE};
    parts
        .headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|s| s.to_string())
        .or_else(|| {
            parts
                .headers
                .get(COOKIE)
                .and_then(|v| v.to_str().ok())
                .and_then(|c| {
                    c.split(';')
                        .find_map(|p| p.trim().strip_prefix("pz_access="))
                })
                .filter(|t| !t.is_empty())
                .map(|s| s.to_string())
        })
}

/// Extract the current user from a valid access JWT (Bearer header or cookie).
pub struct CurrentUser(pub UserId);

impl<S> FromRequestParts<S> for CurrentUser
where
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let token = extract_token(parts).ok_or_else(|| unauthorized("missing or invalid bearer token"))?;

        let state = extract_state(parts).ok_or_else(|| {
            unauthorized("state not available in test context")
        })?;
        match state.config.decode_access(&token) {
            Ok(claims) => Ok(CurrentUser(UserId(claims.sub))),
            Err(SessionError(e)) => Err(unauthorized(&format!("invalid token: {e}"))),
        }
    }
}

fn extract_state(parts: &mut Parts) -> Option<AppState> {
    use axum::extract::State;
    parts.extensions.get::<State<AppState>>().map(|s| s.0.clone())
}