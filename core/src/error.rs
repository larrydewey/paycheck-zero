//! Domain error type. `core` is a library, so this uses `thiserror`
//! (spec §11 / rust-skills `err-thiserror-lib`).

use thiserror::Error;

/// Errors produced by the pure domain when a rule or invariant would be broken.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum DomainError {
    #[error("amount must be greater than zero")]
    NonPositiveAmount,

    #[error("amount cannot be negative")]
    NegativeAmount,

    #[error("paycheck {paycheck} would be over-allocated by {over} cents")]
    OverAllocated { paycheck: crate::Id, over: crate::Cents },

    #[error("month is not at zero; difference is {diff} cents")]
    NotZero { diff: crate::Cents },

    #[error("month is locked and cannot be edited directly")]
    Locked,

    #[error("paycheck already exists for this income line and date")]
    DuplicatePaycheck,

    #[error("expense line {0} not found")]
    ExpenseLineNotFound(crate::Id),

    #[error("category {0} not found")]
    CategoryNotFound(crate::Id),

    #[error("income line {0} not found")]
    IncomeLineNotFound(crate::Id),

    #[error("paycheck {0} not found")]
    PaycheckNotFound(crate::Id),

    #[error("month already has a category named {0}")]
    DuplicateCategory(String),

    #[error("month already has an income line named {0}")]
    DuplicateIncomeLine(String),

    #[error("allocation {0} not found")]
    AllocationNotFound(crate::Id),

    #[error("transaction {0} not found")]
    TransactionNotFound(crate::Id),

    #[error("name cannot be empty")]
    EmptyName,

    #[error("invalid date: {0}")]
    InvalidDate(String),
}

impl DomainError {
    /// A short machine-readable code, matching the API error contract (spec §9).
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            DomainError::NonPositiveAmount => "NON_POSITIVE_AMOUNT",
            DomainError::NegativeAmount => "NEGATIVE_AMOUNT",
            DomainError::OverAllocated { .. } => "INVARIANT_OVER_ALLOCATED",
            DomainError::NotZero { .. } => "INVARIANT_NOT_ZERO",
            DomainError::Locked => "MONTH_LOCKED",
            DomainError::DuplicatePaycheck => "DUPLICATE_PAYCHECK",
            DomainError::ExpenseLineNotFound(_) => "NOT_FOUND",
            DomainError::CategoryNotFound(_) => "NOT_FOUND",
            DomainError::IncomeLineNotFound(_) => "NOT_FOUND",
            DomainError::PaycheckNotFound(_) => "NOT_FOUND",
            DomainError::DuplicateCategory(_) => "DUPLICATE_CATEGORY",
            DomainError::DuplicateIncomeLine(_) => "DUPLICATE_INCOME_LINE",
            DomainError::AllocationNotFound(_) => "NOT_FOUND",
            DomainError::TransactionNotFound(_) => "NOT_FOUND",
            DomainError::EmptyName => "EMPTY_NAME",
            DomainError::InvalidDate(_) => "INVALID_DATE",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Cents, Id};

    #[test]
    fn codes_are_stable() {
        assert_eq!(DomainError::NonPositiveAmount.code(), "NON_POSITIVE_AMOUNT");
        assert_eq!(
            DomainError::OverAllocated { paycheck: Id::new("p"), over: Cents::from_cents(1) }.code(),
            "INVARIANT_OVER_ALLOCATED"
        );
        assert_eq!(
            DomainError::NotZero { diff: Cents::from_cents(2) }.code(),
            "INVARIANT_NOT_ZERO"
        );
    }
}
