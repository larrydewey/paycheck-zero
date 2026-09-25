//! Service layer: bridges core domain logic with storage persistence.

use std::sync::{Arc, Mutex};

use paycheckzero_core::Month;
use paycheckzero_storage::Repository;

pub mod error;
pub mod result;
use result::ServiceResult;

/// High-level service handling invariant enforcement and persistence.
pub struct Service {
    pub repo: Arc<Mutex<paycheckzero_storage::SqliteRepository>>,
}

impl Service {
    pub fn new(repo: Arc<Mutex<paycheckzero_storage::SqliteRepository>>) -> Self {
        Self { repo }
    }

    /// Load a month by id.
    pub fn load_month(&self, id: &paycheckzero_core::Id) -> ServiceResult<Option<Month>> {
        let db = self.repo.lock().unwrap();
        Ok(db.load_month(id)?)
    }

    /// List all months.
    pub fn list_months(&self) -> ServiceResult<Vec<paycheckzero_storage::MonthListItem>> {
        let db = self.repo.lock().unwrap();
        Ok(db.list_months()?)
    }

    /// Save a month (full upsert).
    pub fn save_month(&self, month: &Month) -> ServiceResult<()> {
        let mut db = self.repo.lock().unwrap();
        Ok(db.save_month(month)?)
    }
}
