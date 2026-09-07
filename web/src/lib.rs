//! PaycheckZero web server.
//!
//! Axum-based HTTP server providing a REST API and Datastar-powered frontend.
//! JWT auth, invariant enforcement, CSV export, and SSE live updates.

pub mod extractors;
pub mod routes;
pub mod service;

use paycheckzero_storage::SqliteRepository;

/// Application state shared across all handlers.
pub struct AppState {
    pub db: SqliteRepository,
}

#[derive(Debug, Clone)]
pub struct UserId(pub String);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_clones() {
        let state = AppState {
            db: SqliteRepository::new(":memory:").unwrap(),
        };
        let _ = state.clone();
    }
}
