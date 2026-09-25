//! API route modules.

pub mod auth;
pub mod export;
pub mod months;
pub mod reports;
pub mod ui;

use axum::Router;

use crate::AppState;

/// Build the API router mounted under `/api/v1`.
pub fn api_router() -> Router<AppState> {
    Router::new()
        .merge(auth::router())
        .merge(months::router())
        .merge(reports::router())
        .merge(export::router())
        .merge(ui::router())
        .layer(axum::extract::DefaultBodyLimit::max(10 * 1024 * 1024)) // 10MB
}

/// Build the full application router with state attached.
pub fn app_router(state: AppState) -> Router {
    Router::new()
        .route("/health", axum::routing::get(|| async { "ok" }))
        .route("/", axum::routing::get(shell))
        .route("/static/datastar.js", axum::routing::get(datastar_js))
        .nest(
            "/api/v1",
            api_router().route_layer(axum::middleware::from_fn_with_state(
                state.clone(),
                crate::middleware::require_auth,
            )),
        )
        .with_state(state)
}

/// Datastar shell page; the UI itself is server-rendered fragments (§14).
async fn shell() -> axum::response::Html<&'static str> {
    axum::response::Html(include_str!("../../templates/index.html"))
}

/// Vendored single-file Datastar bundle (esbuild output of @starfederation/datastar 0.20.1).
async fn datastar_js() -> impl axum::response::IntoResponse {
    (
        axum::http::StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "application/javascript; charset=utf-8")],
        include_str!("../../static/datastar.js"),
    )
}
