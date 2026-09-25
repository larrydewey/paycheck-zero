//! PaycheckZero web server.

use std::net::SocketAddr;

use std::sync::{Arc, Mutex};

use paycheckzero_storage::SqliteRepository;
use tokio::net::TcpListener;

use paycheckzero_web::{app_router, AppState, JwtConfig};

#[tokio::main]
async fn main() {
    let db = SqliteRepository::new("paycheckzero.db").expect("failed to open database");
    let state = AppState {
        db: Arc::new(Mutex::new(db)),
        config: Arc::new(JwtConfig::from_env()),
    };

    let addr = SocketAddr::from((
        [0, 0, 0, 0],
        std::env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(3000),
    ));
    let listener = TcpListener::bind(&addr).await.expect("failed to bind");

    eprintln!("paycheckzero-web listening on http://{}", addr);

    axum::serve(
        listener,
        app_router(state),
    )
    .await
    .expect("server error");
}
