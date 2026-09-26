//! Configuration from environment variables (spec §7.6).
//!
//! | Variable              | Default                               |
//! |-----------------------|---------------------------------------|
//! | `PZ_DATABASE_URL`     | `sqlite://paycheckzero.db?mode=rwc`   |
//! | `PZ_BIND`             | `127.0.0.1:8080`                      |
//! | `PZ_JWT_SECRET`       | random per process (sessions reset on restart) |
//! | `PZ_SECURE_COOKIES`   | `false` (set `true` behind HTTPS)     |
//! | `PZ_TEST_MODE`        | `false` (enables `/__test/*` endpoints; never in production) |
//! | `PZ_DATA_KEY`         | 64 hex chars; encrypts stored bank tokens (else `PZ_DATA_KEY_FILE`) |
//! | `PZ_DATA_KEY_FILE`    | `paycheckzero.key` (created on first start, mode 0600) |
//! | `PZ_TELLER_APP_ID`    | unset: bank sync is off. Your Teller application id |
//! | `PZ_TELLER_ENV`       | `development` (`sandbox`, `development` or `production`) |
//! | `PZ_TELLER_CERT`      | path to the Teller client certificate (PEM), needed outside sandbox |
//! | `PZ_TELLER_KEY`       | path to its private key (PEM) |
//! | `PZ_BANK_SYNC_HOURS`  | `6` (background sync of every connection; `0` turns it off; `PZ_TELLER_SYNC_HOURS` also works) |
//! | `PZ_PLAID_CLIENT_ID`  | unset: Plaid is off |
//! | `PZ_PLAID_SECRET`     | Plaid secret for the chosen environment |
//! | `PZ_PLAID_ENV`        | `sandbox` (`sandbox` or `production`) |
//! | `PZ_PLAID_COUNTRIES`  | `US` (comma separated, e.g. `US,CA`) |
//!
//! SimpleFIN needs no server settings: each person pastes a setup token.

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub bind: String,
    pub jwt_secret: Vec<u8>,
    pub jwt_secret_generated: bool,
    pub secure_cookies: bool,
    pub test_mode: bool,
    /// Encrypts bank access tokens at rest.
    pub data_key: [u8; 32],
    /// Bank sync through Teller; `None` when not configured.
    pub teller: Option<TellerConfig>,
    /// Bank sync through Plaid; `None` when not configured.
    pub plaid: Option<PlaidConfig>,
    /// Background sync interval in hours (0 = off).
    pub bank_sync_hours: u64,
    /// SimpleFIN claim URLs may be plain http (tests only).
    pub simplefin_allow_http: bool,
}

#[derive(Debug, Clone)]
pub struct PlaidConfig {
    pub client_id: String,
    pub secret: String,
    pub environment: String,
    pub api: String,
    pub link_js: String,
    pub countries: Vec<String>,
    pub from_env: bool,
}

impl PlaidConfig {
    fn from_env() -> Option<PlaidConfig> {
        let client_id = std::env::var("PZ_PLAID_CLIENT_ID").ok().filter(|s| !s.trim().is_empty())?;
        let secret = std::env::var("PZ_PLAID_SECRET").ok().filter(|s| !s.trim().is_empty())?;
        let environment = std::env::var("PZ_PLAID_ENV").unwrap_or_else(|_| "sandbox".into());
        let api = plaid_api(&environment);
        Some(PlaidConfig {
            client_id: client_id.trim().into(),
            secret: secret.trim().into(),
            environment,
            api,
            from_env: true,
            link_js: plaid_link_js(),
            countries: std::env::var("PZ_PLAID_COUNTRIES").unwrap_or_else(|_| "US".into()).split(',').map(|c| c.trim().to_uppercase()).filter(|c| !c.is_empty()).collect(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct TellerConfig {
    pub app_id: String,
    pub environment: String,
    /// API base URL (tests point this at the built-in fake).
    pub api: String,
    /// Teller Connect script URL.
    pub connect_js: String,
    pub cert: Option<String>,
    pub key: Option<String>,
    /// Certificate and key contents (set in the app) instead of file paths.
    pub cert_pem: Option<String>,
    pub key_pem: Option<String>,
    pub sync_hours: u64,
    /// Where it came from: environment variables, or saved in the app.
    pub from_env: bool,
}

/// Teller API base (tests point this at the built-in fake).
#[must_use]
pub fn teller_api() -> String {
    std::env::var("PZ_TELLER_API").unwrap_or_else(|_| "https://api.teller.io".into())
}

#[must_use]
pub fn teller_connect_js() -> String {
    std::env::var("PZ_TELLER_CONNECT_JS").unwrap_or_else(|_| "https://cdn.teller.io/connect/connect.js".into())
}

#[must_use]
pub fn plaid_api(environment: &str) -> String {
    std::env::var("PZ_PLAID_API").unwrap_or_else(|_| format!("https://{environment}.plaid.com"))
}

#[must_use]
pub fn plaid_link_js() -> String {
    std::env::var("PZ_PLAID_LINK_JS").unwrap_or_else(|_| "https://cdn.plaid.com/link/v2/stable/link-initialize.js".into())
}

impl TellerConfig {
    fn from_env() -> Option<TellerConfig> {
        let app_id = std::env::var("PZ_TELLER_APP_ID").ok().filter(|s| !s.trim().is_empty())?;
        Some(TellerConfig {
            app_id: app_id.trim().to_string(),
            environment: std::env::var("PZ_TELLER_ENV").unwrap_or_else(|_| "development".into()),
            api: teller_api(),
            connect_js: teller_connect_js(),
            cert: std::env::var("PZ_TELLER_CERT").ok().filter(|s| !s.is_empty()),
            key: std::env::var("PZ_TELLER_KEY").ok().filter(|s| !s.is_empty()),
            cert_pem: None,
            key_pem: None,
            sync_hours: sync_hours(),
            from_env: true,
        })
    }
}

fn sync_hours() -> u64 {
    std::env::var("PZ_BANK_SYNC_HOURS").or_else(|_| std::env::var("PZ_TELLER_SYNC_HOURS")).ok().and_then(|v| v.parse().ok()).unwrap_or(6)
}

/// The key that encrypts bank tokens: `PZ_DATA_KEY`, else a key file that
/// is created on first start (keep it with your database backups).
fn data_key() -> [u8; 32] {
    if let Some(k) = std::env::var("PZ_DATA_KEY").ok().and_then(|h| hex::decode(h.trim()).ok()).and_then(|b| <[u8; 32]>::try_from(b).ok()) {
        return k;
    }
    let path = std::env::var("PZ_DATA_KEY_FILE").unwrap_or_else(|_| "paycheckzero.key".into());
    if let Some(k) = std::fs::read_to_string(&path).ok().and_then(|h| hex::decode(h.trim()).ok()).and_then(|b| <[u8; 32]>::try_from(b).ok()) {
        return k;
    }
    let mut k = [0u8; 32];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut k);
    let written = std::fs::write(&path, hex::encode(k));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    if let Err(e) = written {
        tracing::warn!("could not save the data key to {path}: {e}; connected banks will need reconnecting after a restart");
    }
    k
}

fn flag(name: &str) -> bool {
    std::env::var(name).is_ok_and(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
}

impl Config {
    #[must_use]
    pub fn from_env() -> Config {
        let (jwt_secret, generated) = match std::env::var("PZ_JWT_SECRET") {
            Ok(s) if s.len() >= 32 => (s.into_bytes(), false),
            _ => (random_secret(), true),
        };
        Config {
            database_url: std::env::var("PZ_DATABASE_URL").unwrap_or_else(|_| "sqlite://paycheckzero.db?mode=rwc".into()),
            bind: std::env::var("PZ_BIND").unwrap_or_else(|_| "127.0.0.1:8080".into()),
            jwt_secret,
            jwt_secret_generated: generated,
            secure_cookies: flag("PZ_SECURE_COOKIES"),
            test_mode: flag("PZ_TEST_MODE"),
            data_key: data_key(),
            teller: TellerConfig::from_env(),
            plaid: PlaidConfig::from_env(),
            bank_sync_hours: sync_hours(),
            simplefin_allow_http: flag("PZ_TEST_MODE"),
        }
    }

    /// A configuration for in-process tests.
    #[must_use]
    pub fn for_tests() -> Config {
        Config {
            database_url: "sqlite::memory:".into(),
            bind: "127.0.0.1:0".into(),
            jwt_secret: b"test-secret-test-secret-test-secret!".to_vec(),
            jwt_secret_generated: false,
            secure_cookies: false,
            test_mode: true,
            data_key: [7u8; 32],
            teller: None,
            plaid: None,
            bank_sync_hours: 0,
            simplefin_allow_http: true,
        }
    }
}

fn random_secret() -> Vec<u8> {
    use rand::RngCore;
    let mut b = vec![0u8; 48];
    rand::thread_rng().fill_bytes(&mut b);
    b
}
