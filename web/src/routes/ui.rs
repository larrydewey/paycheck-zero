//! Datastar fragment routes for the web UI (§14).
//!
//! Fragments are returned as Datastar `datastar-patch-elements` SSE events so
//! `data-on-load @get(...)` and action chains can patch them in place.

use axum::{
    extract::{Path, State},
    http::HeaderMap,
    http::header::{AUTHORIZATION, CACHE_CONTROL, CONTENT_TYPE},
    response::{IntoResponse, Response},
};

use paycheckzero_core::Month;
use paycheckzero_storage::Repository;

use crate::AppState;
use crate::ui;

/// Wrap a fragment as a Datastar SSE patch-elements event.
fn sse(fragment: String) -> Response {
    let single = fragment.replace(['\n', '\r'], " ");
    let body = format!("event: datastar-patch-elements\ndata: elements {single}\n\n");
    (
        [(CONTENT_TYPE, "text/event-stream"), (CACHE_CONTROL, "no-cache")],
        body,
    )
        .into_response()
}

fn dashboard_fragment(state: &AppState) -> Result<String, String> {
    let listing = state
        .db
        .lock()
        .unwrap()
        .list_months()
        .map_err(|e| e.to_string())?;
    let today = chrono::Local::now().date_naive();
    let mut summaries = Vec::with_capacity(listing.len());
    let mut months: Vec<Month> = Vec::with_capacity(listing.len());
    for item in &listing {
        let m = state
            .db
            .lock()
            .unwrap()
            .load_month(&item.id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("month not found: {}", item.id))?;
        summaries.push(m.summary(today));
        months.push(m);
    }
    Ok(crate::ui::dashboard(&summaries, &months))
}

/// Initial fragment: login page, or the dashboard when a valid access token is
/// present. This route is public (see middleware) so the shell can bootstrap.
pub async fn session(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let bearer = headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    let cookie = headers
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(|c| c.split(';').find_map(|p| p.trim().strip_prefix("pz_access=")))
        .filter(|t| !t.is_empty());
    let authed = bearer
        .or(cookie)
        .and_then(|t| state.config.decode_access(t).ok())
        .is_some();
    if !authed {
        return sse(ui::login_page());
    }
    match dashboard_fragment(&state) {
        Ok(f) => sse(f),
        Err(_) => sse(ui::login_page()),
    }
}

/// Month view fragment: summary cards, paycheck-first view, editable lines.
pub async fn month(State(state): State<AppState>, Path(month_id): Path<String>) -> Response {
    let loaded = state
        .db
        .lock()
        .unwrap()
        .load_month(&paycheckzero_core::Id::new(&month_id));
    let (s, m) = match loaded.ok().flatten() {
        Some(m) => {
            let today = chrono::Local::now().date_naive();
            let s = m.summary(today);
            (s, m)
        }
        None => return sse("not found".to_string()),
    };
    sse(ui::month_view(&s, &m))
}

pub fn router() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/ui/session", axum::routing::get(session))
        .route("/ui/month/{month_id}", axum::routing::get(month))
}