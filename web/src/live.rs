//! Live updates: every open page holds `GET /live`, an SSE stream that says
//! "changed" whenever its budget is written (any login, any device, bank
//! sync). The page then reloads its own content.

use crate::auth::{AuthUser, ACCESS_COOKIE};
use crate::Shared;
use axum::extract::{Request, State};
use axum::http::Method;
use axum::middleware::Next;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Extension;
use futures_util::stream::{self, Stream};
use paycheckzero_core::Id;
use std::convert::Infallible;
use std::time::Duration;
use tokio::sync::{broadcast, watch};
use tokio::time::Instant;

/// Streams end after this long; the browser reconnects, which re-checks the session.
const STREAM_TTL: Duration = Duration::from_secs(10 * 60);

pub struct Hub {
    tx: broadcast::Sender<Id>,
    closing: watch::Sender<bool>,
}

impl Default for Hub {
    fn default() -> Self {
        Self { tx: broadcast::channel(256).0, closing: watch::channel(false).0 }
    }
}

impl Hub {
    /// Tells every stream on `budget` that its data changed.
    pub fn publish(&self, budget: &Id) {
        let _ = self.tx.send(budget.clone());
    }

    /// Ends every stream (server shutdown).
    pub fn close(&self) {
        let _ = self.closing.send(true);
    }
}

/// `GET /live`.
pub async fn stream(State(st): State<Shared>, Extension(user): Extension<AuthUser>) -> impl IntoResponse {
    Sse::new(events(&st, user.id().clone())).keep_alive(KeepAlive::new().interval(Duration::from_secs(20)))
}

fn events(st: &Shared, budget: Id) -> impl Stream<Item = Result<Event, Infallible>> {
    let state = (st.live.tx.subscribe(), st.live.closing.subscribe(), budget, Instant::now() + STREAM_TTL);
    stream::unfold(state, |(mut rx, mut closing, budget, until)| async move {
        if *closing.borrow() {
            return None;
        }
        loop {
            tokio::select! {
                msg = rx.recv() => match msg {
                    Ok(id) if id == budget => break,
                    Ok(_) => {}
                    // Missed some: one of them may be ours.
                    Err(broadcast::error::RecvError::Lagged(_)) => break,
                    Err(broadcast::error::RecvError::Closed) => return None,
                },
                _ = closing.changed() => return None,
                () = tokio::time::sleep_until(until) => return None,
            }
        }
        Some((Ok(Event::default().event("changed").data("1")), (rx, closing, budget, until)))
    })
}

fn writes(method: &Method) -> bool {
    !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
}

/// UI middleware (inside `require_session`): announce successful writes.
pub async fn notify(State(st): State<Shared>, req: Request, next: Next) -> Response {
    let budget = writes(req.method()).then(|| req.extensions().get::<AuthUser>().map(|u| u.id().clone())).flatten();
    let resp = next.run(req).await;
    if let Some(b) = budget.filter(|_| resp.status().is_success()) {
        st.live.publish(&b);
    }
    resp
}

/// REST API middleware: same, resolving the Bearer (or cookie) user itself.
pub async fn notify_api(State(st): State<Shared>, req: Request, next: Next) -> Response {
    let budget = if writes(req.method()) { api_budget(&st, req.headers().clone()).await } else { None };
    let resp = next.run(req).await;
    if let Some(b) = budget.filter(|_| resp.status().is_success()) {
        st.live.publish(&b);
    }
    resp
}

async fn api_budget(st: &Shared, h: axum::http::HeaderMap) -> Option<Id> {
    let token = h
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_string)
        .or_else(|| crate::auth::cookie_value(&h, ACCESS_COOKIE))?;
    let login = crate::auth::user_from_access(st, &token).await?;
    AuthUser::resolve(st, login).await.map(|u| u.id().clone())
}
