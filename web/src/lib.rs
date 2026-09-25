//! PaycheckZero web server.
//!
//! Axum-based HTTP server providing a REST API and Datastar-powered frontend.
//! JWT auth, invariant enforcement, CSV export, and SSE live updates.

pub mod extractors;
pub mod middleware;
pub mod report;
pub mod routes;
pub mod service;
pub mod session;
pub mod ui;

pub use routes::{api_router, app_router};
pub use session::JwtConfig;

use std::sync::{Arc, Mutex};

use paycheckzero_storage::SqliteRepository;

/// Application state shared across all handlers.
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Mutex<SqliteRepository>>,
    pub config: Arc<JwtConfig>,
}

#[derive(Debug, Clone, Default)]
pub struct UserId(pub String);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_clones() {
        let repo = SqliteRepository::new(":memory:").unwrap();
        let state = AppState {
            db: Arc::new(Mutex::new(repo)),
            config: Arc::new(JwtConfig::from_secret(
                "test-secret-that-is-long-enough-123456".into(),
            )),
        };
        let _ = state.clone();
    }
}
