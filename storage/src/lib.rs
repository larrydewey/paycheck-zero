//! Persistent storage adapters for PaycheckZero.
//!
//! Currently provides a SQLite backend via [`sqlite::SqliteRepository`].

mod repo;
mod sqlite;

pub use repo::{
    AuthRepository, MonthListItem, RefreshTokenRecord, Repository, StorageError, StorageResult,
    UserRecord,
};
pub use sqlite::SqliteRepository;
