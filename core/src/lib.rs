//! PaycheckZero pure domain crate.
//!
//! Holds the authoritative models, invariant enforcement, Safe-to-Spend,
//! zero-based validation, reports and suggestions. It has **no** web or
//! database dependencies (spec §7.1). All money is integer [`Cents`].

pub mod copy;
pub mod error;
pub mod id;
pub mod models;
pub mod money;
pub mod month;
pub mod recurrence;
pub mod report;
pub mod suggest;
pub mod views;
pub mod wallet;

pub use copy::CopyMode;
pub use error::DomainError;
pub use id::Id;
pub use models::*;
pub use money::{Cents, Rate};
pub use month::{Impact, Month, SplitPart, STARTER_CATEGORIES};
pub use recurrence::Recurrence;
pub use views::*;
pub use wallet::*;
