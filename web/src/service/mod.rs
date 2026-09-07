//! Service layer: bridges core domain logic with storage persistence.

use paycheckzero_core::Month;
use paycheckzero_storage::Repository;

use crate::service::error::ServiceError;
use crate::service::result::ServiceResult;

pub mod error;
pub mod result;

/// High-level service handling invariant enforcement and persistence.
pub struct Service {
    pub repo: paycheckzero_storage::SqliteRepository,
}

impl Service {
    pub fn new(repo: paycheckzero_storage::SqliteRepository) -> Self {
        Self { repo }
    }

    /// Load a month by id.
    pub fn load_month(&self, id: &paycheckzero_core::Id) -> ServiceResult<Option<Month>> {
        self.repo.load_month(id)
    }

    /// List all months.
    pub fn list_months(&self) -> ServiceResult<Vec<paycheckzero_storage::MonthListItem>> {
        self.repo.list_months()
    }

    /// Save a month (full upsert).
    pub fn save_month(&mut self, month: &Month) -> ServiceResult<()> {
        self.repo.save_month(month)
    }
}
