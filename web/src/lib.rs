//! PaycheckZero web server: Datastar UI (HTML + SSE), REST API, auth,
//! exports and offline sync on top of the pure domain crate.

pub mod api;
pub mod bank;
pub mod auth;
pub mod config;
pub mod error;
pub mod export;
pub mod i18n;
pub mod money;
pub mod service;
pub mod sse;
pub mod sync;
pub mod testing;
pub mod ui;

use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::HeaderValue;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{middleware, Router};
use chrono::NaiveDate;
use config::Config;
use paycheckzero_core::Id;
use paycheckzero_storage::Store;
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Instant;

/// Supplies "today". Test mode can pin the date for deterministic runs.
#[derive(Default)]
pub struct Clock {
    fixed: RwLock<Option<NaiveDate>>,
}

impl Clock {
    /// Today's date in an IANA timezone (falls back to UTC).
    #[must_use]
    pub fn today(&self, timezone: &str) -> NaiveDate {
        if let Some(d) = self.fixed.read().ok().and_then(|g| *g) {
            return d;
        }
        let tz: chrono_tz::Tz = timezone.parse().unwrap_or(chrono_tz::UTC);
        chrono::Utc::now().with_timezone(&tz).date_naive()
    }

    pub fn set_fixed(&self, date: Option<NaiveDate>) {
        if let Ok(mut g) = self.fixed.write() {
            *g = date;
        }
    }
}

pub struct AppState {
    pub store: Store,
    pub cfg: Config,
    pub clock: Clock,
    access_ttl: AtomicI64,
    pub refresh_grace: tokio::sync::Mutex<HashMap<String, (Instant, auth::Tokens, Id)>>,
}

impl AppState {
    #[must_use]
    pub fn new(store: Store, cfg: Config) -> Self {
        Self {
            store,
            cfg,
            clock: Clock::default(),
            access_ttl: AtomicI64::new(auth::ACCESS_TTL_SECS),
            refresh_grace: tokio::sync::Mutex::new(HashMap::new()),
        }
    }

    #[must_use]
    pub fn access_ttl(&self) -> i64 {
        self.access_ttl.load(Ordering::Relaxed)
    }

    pub fn set_access_ttl(&self, secs: i64) {
        self.access_ttl.store(secs.max(1), Ordering::Relaxed);
    }
}

pub type Shared = Arc<AppState>;

pub async fn build_state(cfg: Config) -> Result<Shared, paycheckzero_storage::StorageError> {
    let store = Store::connect(&cfg.database_url).await?;
    Ok(Arc::new(AppState::new(store, cfg)))
}

fn asset(body: &'static [u8], content_type: &'static str, cache: &'static str) -> Response {
    let mut r = body.into_response();
    r.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
    r.headers_mut().insert(CACHE_CONTROL, HeaderValue::from_static(cache));
    r
}

macro_rules! static_route {
    ($path:literal, $file:literal, $ct:literal) => {
        get(|| async { asset(include_bytes!(concat!("../static/", $file)), $ct, "public, max-age=3600") })
    };
}

fn static_routes() -> Router<Shared> {
    Router::new()
        .route("/static/datastar.js", static_route!("/static/datastar.js", "datastar.js", "text/javascript"))
        .route("/static/app.js", static_route!("/static/app.js", "app.js", "text/javascript"))
        .route("/static/app.css", static_route!("/static/app.css", "app.css", "text/css"))
        .route("/static/icon.svg", static_route!("/static/icon.svg", "icon.svg", "image/svg+xml"))
        .route("/static/icon-192.png", static_route!("/static/icon-192.png", "icon-192.png", "image/png"))
        .route("/static/icon-512.png", static_route!("/static/icon-512.png", "icon-512.png", "image/png"))
        .route(
            "/manifest.webmanifest",
            static_route!("/manifest.webmanifest", "manifest.webmanifest", "application/manifest+json"),
        )
        .route("/sw.js", get(service_worker))
}

/// A short hash of the bundled static assets; it changes with every release.
fn asset_version() -> &'static str {
    static V: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        // FNV-1a over the assets the service worker caches.
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for part in [
            &include_bytes!("../static/app.js")[..],
            &include_bytes!("../static/app.css")[..],
            &include_bytes!("../static/datastar.js")[..],
            &include_bytes!("../static/icon.svg")[..],
            &include_bytes!("../static/manifest.webmanifest")[..],
        ] {
            for b in part {
                h ^= u64::from(*b);
                h = h.wrapping_mul(0x0100_0000_01b3);
            }
        }
        format!("{h:016x}")
    });
    &V
}

async fn service_worker() -> Response {
    static SW: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        include_str!("../static/sw.js").replace("__PZ_ASSET_VERSION__", asset_version())
    });
    let mut r = SW.as_str().into_response();
    r.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("text/javascript"));
    r.headers_mut().insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    r
}

/// The whole application router.
pub fn app(state: Shared) -> Router {
    let mut router = Router::new()
        .merge(static_routes())
        .merge(ui::routes(state.clone()))
        .nest("/api/v1", api::routes())
        .route("/healthz", get(|| async { "ok" }));
    if state.cfg.test_mode {
        router = router.nest("/__test", testing::routes());
    }
    router
        .layer(middleware::from_fn(security_headers))
        .layer(tower_http::compression::CompressionLayer::new())
        .with_state(state)
}

async fn security_headers(req: axum::extract::Request, next: middleware::Next) -> Response {
    let mut resp = next.run(req).await;
    let h = resp.headers_mut();
    h.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    h.insert("referrer-policy", HeaderValue::from_static("same-origin"));
    h.insert("x-frame-options", HeaderValue::from_static("DENY"));
    resp
}
