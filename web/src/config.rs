//! Configuration from environment variables (spec §7.6).
//!
//! | Variable              | Default                               |
//! |-----------------------|---------------------------------------|
//! | `PZ_DATABASE_URL`     | `sqlite://paycheckzero.db?mode=rwc`   |
//! | `PZ_BIND`             | `127.0.0.1:8080`                      |
//! | `PZ_JWT_SECRET`       | random per process (sessions reset on restart) |
//! | `PZ_SECURE_COOKIES`   | `false` (set `true` behind HTTPS)     |
//! | `PZ_TEST_MODE`        | `false` (enables `/__test/*` endpoints; never in production) |

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub bind: String,
    pub jwt_secret: Vec<u8>,
    pub jwt_secret_generated: bool,
    pub secure_cookies: bool,
    pub test_mode: bool,
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
        }
    }
}

fn random_secret() -> Vec<u8> {
    use rand::RngCore;
    let mut b = vec![0u8; 48];
    rand::thread_rng().fill_bytes(&mut b);
    b
}
