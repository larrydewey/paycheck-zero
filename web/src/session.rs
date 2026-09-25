//! Session tokens: JWT access tokens + stateful (hashed) refresh tokens.

use std::time::Duration;

use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// JWT claims for both access and refresh tokens.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    /// User id.
    pub sub: String,
    /// Token kind: `access` or `refresh`.
    pub typ: String,
    /// Issued-at (Unix seconds).
    pub iat: usize,
    /// Expiry (Unix seconds).
    pub exp: usize,
    /// Random token id (refresh tokens, for revocation).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jti: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SessionError(pub String);

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SessionError {}

/// Token signing configuration.
#[derive(Debug, Clone)]
pub struct JwtConfig {
    secret: String,
    pub access_ttl: Duration,
    pub refresh_ttl: Duration,
}

impl JwtConfig {
    /// Build from the `PAYCHECKZERO_JWT_SECRET` env var, falling back to a
    /// dev-only secret (logged, never for production).
    pub fn from_env() -> Self {
        let secret = std::env::var("PAYCHECKZERO_JWT_SECRET")
            .ok()
            .filter(|s| s.len() >= 32)
            .unwrap_or_else(|| {
                eprintln!("warning: PAYCHECKZERO_JWT_SECRET not set; using insecure dev secret");
                "dev-only-insecure-secret-change-me-0123456789".to_string()
            });
        JwtConfig {
            secret,
            access_ttl: Duration::from_secs(15 * 60),
            refresh_ttl: Duration::from_secs(30 * 24 * 60 * 60),
        }
    }

    pub fn from_secret(secret: String) -> Self {
        JwtConfig {
            secret,
            access_ttl: Duration::from_secs(15 * 60),
            refresh_ttl: Duration::from_secs(30 * 24 * 60 * 60),
        }
    }

    fn encode(&self, claims: &Claims) -> Result<String, SessionError> {
        encode(
            &Header::default(),
            claims,
            &EncodingKey::from_secret(self.secret.as_bytes()),
        )
        .map_err(|e| SessionError(e.to_string()))
    }

    fn decode(&self, token: &str, expected_typ: &str) -> Result<Claims, SessionError> {
        let data = decode::<Claims>(
            token,
            &DecodingKey::from_secret(self.secret.as_bytes()),
            &Validation::new(Algorithm::HS256),
        )
        .map_err(|e| SessionError(e.to_string()))?;
        if data.claims.typ != expected_typ {
            return Err(SessionError("wrong token type".into()));
        }
        Ok(data.claims)
    }

    fn now() -> usize {
        chrono::Utc::now().timestamp().max(0) as usize
    }

    pub fn access_token(&self, user_id: &str) -> Result<String, SessionError> {
        let now = Self::now();
        self.encode(&Claims {
            sub: user_id.to_string(),
            typ: "access".into(),
            iat: now,
            exp: now + self.access_ttl.as_secs() as usize,
            jti: None,
        })
    }

    /// Issue a refresh token and return `(token, sha256_hash)`.
    pub fn refresh_token(&self, user_id: &str) -> Result<(String, String), SessionError> {
        let now = Self::now();
        let jti = uuid::Uuid::new_v4().to_string();
        let token = self.encode(&Claims {
            sub: user_id.to_string(),
            typ: "refresh".into(),
            iat: now,
            exp: now + self.refresh_ttl.as_secs() as usize,
            jti: Some(jti.clone()),
        })?;
        Ok((token.clone(), Self::hash_refresh(&token)))
    }

    pub fn decode_access(&self, token: &str) -> Result<Claims, SessionError> {
        self.decode(token, "access")
    }

    pub fn decode_refresh(&self, token: &str) -> Result<Claims, SessionError> {
        self.decode(token, "refresh")
    }

    pub fn hash_refresh(token: &str) -> String {
        format!("{:x}", Sha256::digest(token.as_bytes()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> JwtConfig {
        JwtConfig::from_secret("test-secret-that-is-long-enough-123456".into())
    }

    #[test]
    fn access_token_round_trips() {
        let c = cfg();
        let tok = c.access_token("user-1").unwrap();
        let claims = c.decode_access(&tok).unwrap();
        assert_eq!(claims.sub, "user-1");
        assert_eq!(claims.typ, "access");
    }

    #[test]
    fn refresh_token_hashes_deterministically() {
        let c = cfg();
        let (tok, h1) = c.refresh_token("user-1").unwrap();
        let h2 = JwtConfig::hash_refresh(&tok);
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64);
        let claims = c.decode_refresh(&tok).unwrap();
        assert_eq!(claims.typ, "refresh");
        assert!(claims.jti.is_some());
    }

    #[test]
    fn access_rejects_refresh_type() {
        let c = cfg();
        let (tok, _) = c.refresh_token("user-1").unwrap();
        assert!(c.decode_access(&tok).is_err());
    }

    #[test]
    fn tampered_token_rejected() {
        let c = cfg();
        let tok = c.access_token("user-1").unwrap();
        let mut bytes = tok.into_bytes();
        let n = bytes.len();
        bytes[n - 1] = if bytes[n - 1] == b'a' { b'b' } else { b'a' };
        let tampered = String::from_utf8(bytes).unwrap();
        assert!(c.decode_access(&tampered).is_err());
    }
}