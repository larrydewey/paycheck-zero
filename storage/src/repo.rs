//! Repository traits for persisting the `Month` aggregate.

use paycheckzero_core::{Id, Month};

#[derive(Debug, thiserror::Error)]
#[error("storage error: {0}")]
pub struct StorageError(#[from] pub rusqlite::Error);

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
#[derive(Debug, Clone)]
pub struct MonthListItem {
    pub id: Id,
    pub year_month_str: String,
    pub status_str: String,
    pub archived: bool,
}
