//! PaycheckZero web server.

use std::net::SocketAddr;

use paycheckzero_storage::SqliteRepository;
use tokio::net::TcpListener;

use paycheckzero_web::{app_router, AppState};

#[tokio::main]
async fn main() {
    let db = SqliteRepository::new("paycheckzero.db").expect("failed to open database");
    let state = AppState { db };

    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
    let listener = TcpListener::bind(&addr).await.expect("failed to bind");

    eprintln!("paycheckzero-web listening on http://{}", addr);

    axum::serve(
        listener,
        app_router(state),
    )
    .await
    .expect("server error");
}
