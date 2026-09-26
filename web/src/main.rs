//! `paycheckzero` — single binary: web UI, REST API and database.

use paycheckzero_web::config::Config;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();
    let cfg = Config::from_env();
    if cfg.jwt_secret_generated {
        tracing::warn!("PZ_JWT_SECRET not set (or shorter than 32 bytes); using a random secret, sessions end on restart");
    }
    if cfg.test_mode {
        tracing::warn!("PZ_TEST_MODE is on: /__test endpoints can wipe the database. Never enable this in production.");
    }
    let bind = cfg.bind.clone();
    let state = match paycheckzero_web::build_state(cfg).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("cannot open database: {e}");
            std::process::exit(1);
        }
    };
    let listener = match tokio::net::TcpListener::bind(&bind).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("cannot bind {bind}: {e}");
            std::process::exit(1);
        }
    };
    tracing::info!("PaycheckZero listening on http://{bind}");
    if let Some(t) = &state.cfg.teller {
        tracing::info!(environment = %t.environment, "bank sync (Teller) is on");
        paycheckzero_web::bank::spawn_background_sync(state.clone());
    }
    let app = paycheckzero_web::app(state);
    if let Err(e) = axum::serve(listener, app).with_graceful_shutdown(shutdown()).await {
        tracing::error!("server error: {e}");
    }
}

async fn shutdown() {
    let _ = tokio::signal::ctrl_c().await;
}
