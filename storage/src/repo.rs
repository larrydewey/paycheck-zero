//! Repository traits for persisting the `Month` aggregate.

use paycheckzero_core::{Id, Month};
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("sqlite error: {0}")]
    Sqlite(rusqlite::Error),

    #[error("unique constraint violation: {0}")]
    UniqueViolation(String),
}

impl From<rusqlite::Error> for StorageError {
    fn from(e: rusqlite::Error) -> Self {
        if let Some(code) = e.sqlite_error_code() {
            if code == rusqlite::ErrorCode::ConstraintViolation {
                return StorageError::UniqueViolation(e.to_string());
            }
        }
        StorageError::Sqlite(e)
    }
}

pub type StorageResult<T> = Result<T, StorageError>;

/// Persistent storage backend for the PaycheckZero domain.
pub trait Repository: Send {
    /// Load a month by id. Returns `None` if not found.
    fn load_month(&self, id: &Id) -> StorageResult<Option<Month>>;

    /// Upsert a complete month aggregate. Idempotent.
    fn save_month(&mut self, month: &Month) -> StorageResult<()>;

    /// List all month identifiers with metadata.
    fn list_months(&self) -> StorageResult<Vec<MonthListItem>>;
}

/// Lightweight month listing item returned by [`Repository::list_months`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonthListItem {
    pub id: Id,
    pub year_month_str: String,
    pub status_str: String,
    pub archived: bool,
}

/// A registered user account.
#[derive(Debug, Clone)]
pub struct UserRecord {
    pub id: Id,
    pub email: String,
    pub password_hash: String,
    pub created_at: String,
}

/// A stored (hashed) refresh token for revocation checks.
#[derive(Debug, Clone)]
pub struct RefreshTokenRecord {
    pub token_hash: String,
    pub user_id: Id,
    pub device: String,
    pub expires_at: String,
    pub created_at: String,
}

/// Persistent backend for authentication data (users + refresh-token stores).
pub trait AuthRepository: Send {
    /// Create a new user. Fails with [`StorageError::UniqueViolation`] on a
    /// duplicate email.
    fn register_user(&mut self, id: &Id, email: &str, password_hash: &str) -> StorageResult<()>;

    fn user_by_email(&self, email: &str) -> StorageResult<Option<UserRecord>>;

    fn user_by_id(&self, id: &Id) -> StorageResult<Option<UserRecord>>;

    fn save_refresh_token(
        &mut self,
        token_hash: &str,
        user_id: &Id,
        device: &str,
        expires_at: &str,
        created_at: &str,
    ) -> StorageResult<()>;

    fn refresh_token(&self, token_hash: &str) -> StorageResult<Option<RefreshTokenRecord>>;

    fn delete_refresh_token(&mut self, token_hash: &str) -> StorageResult<()>;

    fn delete_all_refresh_tokens(&mut self, user_id: &Id) -> StorageResult<()>;
}
