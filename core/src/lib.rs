//! PaycheckZero pure domain crate.
//!
//! Holds the authoritative models, invariant enforcement, safe-to-spend and
//! zero-based validation logic. This crate has **no** web or database
//! dependencies (spec §7.1). All money is integer [`Cents`]; floating point is
//! forbidden inside the domain.

pub mod domain;
pub mod error;
pub mod id;
pub mod money;
pub mod models;
pub mod views;

pub use domain::Month;
pub use error::DomainError;
pub use id::Id;
pub use money::Cents;
pub use models::*;
pub use views::*;
