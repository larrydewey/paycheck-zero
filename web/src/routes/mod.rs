//! API route modules.

pub mod auth;
pub mod export;
pub mod months;

use axum::Router;
use axum_extra::extract::CookieJar;

use crate::AppState;

/// Build the API router mounted under `/api/v1`.
pub fn api_router(state: AppState) -> Router {
    Router::new()
        .merge(auth::router())
        .merge(months::router())
        .merge(export::router())
        .layer(axum::extract::DefaultBodyLimit::max(10 * 1024 * 1024)) // 10MB
}

/// Build the full application router.
pub fn app_router(state: AppState) -> Router {
    Router::new()
        .route("/health", axum::routing::get(|| async { "ok" }))
        .merge(api_router(state))
}
