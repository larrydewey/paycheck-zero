//! Persistent storage adapters for PaycheckZero.
//!
//! Currently provides a SQLite backend via [`sqlite::SqliteRepository`].

mod repo;
mod sqlite;

pub use repo::{Repository, StorageError, StorageResult};
pub use sqlite::SqliteRepository;
