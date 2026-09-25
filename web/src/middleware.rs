//! Request middleware: gateway for the `/api/v1` surface.

use axum::{
    Json,
    extract::{Request, State},
    http::StatusCode,
    http::header::{AUTHORIZATION, COOKIE},
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::AppState;

fn unauthorized(message: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({
            "error": { "code": "UNAUTHORIZED", "message": message }
        })),
    )
        .into_response()
}

/// Require a valid access JWT on every `/api/v1` route except the auth group.
/// The auth handlers themselves are open (they issue the tokens).
#[allow(clippy::result_large_err)]
pub async fn require_auth(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Result<Response, Response> {
    let path = req.uri().path();
    let open = path == "/api/v1"
        || path == "/health"
        || path.ends_with("/auth/register")
        || path.ends_with("/auth/login")
        || path.ends_with("/auth/refresh")
        || path.ends_with("/auth/logout")
        || path.ends_with("/ui/session");
    if open {
        return Ok(next.run(req).await);
    }

    let token = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .or_else(|| {
            req.headers()
                .get(COOKIE)
                .and_then(|v| v.to_str().ok())
                .and_then(|c| {
                    c.split(';')
                        .find_map(|p| p.trim().strip_prefix("pz_access="))
                })
                .filter(|t| !t.is_empty())
        })
        .ok_or_else(|| unauthorized("missing bearer token"))?;

    state
        .config
        .decode_access(token)
        .map_err(|e| unauthorized(&format!("invalid token: {e}")))?;

    Ok(next.run(req).await)
}