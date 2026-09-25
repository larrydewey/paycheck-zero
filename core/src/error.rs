//! Domain error type.

use crate::{Cents, Id};
use chrono::NaiveDate;
use thiserror::Error;

/// Errors produced when a rule or invariant would be broken.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum DomainError {
    #[error("amount must be greater than zero")]
    NonPositiveAmount,
    #[error("amount cannot be negative")]
    NegativeAmount,
    #[error("transaction amount cannot be zero")]
    ZeroTransaction,
    #[error("a split needs at least two parts")]
    SplitTooFew,
    #[error("split parts must all be expenses or all be income")]
    SplitMixedSigns,
    #[error("paycheck {paycheck} would be over-allocated by {over} cents")]
    OverAllocated { paycheck: Id, over: Cents },
    #[error("not enough unallocated paycheck money: short by {short} cents")]
    InsufficientUnallocated { short: Cents },
    #[error("cannot move {requested} cents; only {available} cents are allocated there")]
    TransferExceedsAllocation { requested: Cents, available: Cents },
    #[error("month is not at zero; difference is {diff} cents")]
    NotZero { diff: Cents },
    #[error("month is locked; planned amounts and allocations are frozen")]
    Locked,
    #[error("month is already locked")]
    AlreadyLocked,
    #[error("month is not locked")]
    NotLocked,
    #[error("variance re-assignment is not open for this month")]
    NotReassigning,
    #[error("there is no paycheck variance to re-assign")]
    NoVariance,
    #[error("paycheck {0} still has a variance that has not been applied")]
    UnresolvedVariance(Id),
    #[error("paycheck {0} has no recorded actual amount")]
    NoActual(Id),
    #[error("paycheck {0} is skipped and cannot be funded")]
    PaycheckSkipped(Id),
    #[error("a paycheck already exists for this income line on {0}")]
    DuplicatePaycheck(NaiveDate),
    #[error("date {0} is outside this month")]
    DateOutsideMonth(NaiveDate),
    #[error("the schedule produces no paycheck dates in this month")]
    NoDatesInMonth,
    #[error("invalid recurrence rule")]
    InvalidRecurrence,
    #[error("name must be between 1 and {max} characters")]
    InvalidName { max: usize },
    #[error("only lines in a Debt category have a balance and minimum payment")]
    NotDebtLine,
    #[error("{kind} {id} not found")]
    NotFound { kind: &'static str, id: Id },
    #[error("invariant violated: {0}")]
    Invariant(String),
}

impl DomainError {
    /// Stable machine-readable code (spec §9 error format).
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            DomainError::NonPositiveAmount => "NON_POSITIVE_AMOUNT",
            DomainError::NegativeAmount => "NEGATIVE_AMOUNT",
            DomainError::ZeroTransaction => "ZERO_TRANSACTION",
            DomainError::SplitTooFew => "SPLIT_TOO_FEW",
            DomainError::SplitMixedSigns => "SPLIT_MIXED_SIGNS",
            DomainError::OverAllocated { .. } => "INVARIANT_VIOLATION",
            DomainError::InsufficientUnallocated { .. } => "INVARIANT_VIOLATION",
            DomainError::TransferExceedsAllocation { .. } => "INVARIANT_VIOLATION",
            DomainError::NotZero { .. } => "MONTH_NOT_ZERO",
            DomainError::Locked => "MONTH_LOCKED",
            DomainError::AlreadyLocked => "MONTH_ALREADY_LOCKED",
            DomainError::NotLocked => "MONTH_NOT_LOCKED",
            DomainError::NotReassigning => "NOT_REASSIGNING",
            DomainError::NoVariance => "NO_VARIANCE",
            DomainError::UnresolvedVariance(_) => "UNRESOLVED_VARIANCE",
            DomainError::NoActual(_) => "NO_ACTUAL",
            DomainError::PaycheckSkipped(_) => "PAYCHECK_SKIPPED",
            DomainError::DuplicatePaycheck(_) => "DUPLICATE_PAYCHECK",
            DomainError::DateOutsideMonth(_) => "DATE_OUTSIDE_MONTH",
            DomainError::NoDatesInMonth => "NO_DATES_IN_MONTH",
            DomainError::InvalidRecurrence => "INVALID_RECURRENCE",
            DomainError::InvalidName { .. } => "INVALID_NAME",
            DomainError::NotDebtLine => "NOT_DEBT_LINE",
            DomainError::NotFound { .. } => "NOT_FOUND",
            DomainError::Invariant(_) => "INVARIANT_VIOLATION",
        }
    }

    /// True for errors that mean "this would break a budget rule" (HTTP 409).
    #[must_use]
    pub fn is_conflict(&self) -> bool {
        matches!(
            self,
            DomainError::OverAllocated { .. }
                | DomainError::InsufficientUnallocated { .. }
                | DomainError::TransferExceedsAllocation { .. }
                | DomainError::NotZero { .. }
                | DomainError::Locked
                | DomainError::AlreadyLocked
                | DomainError::NotLocked
                | DomainError::NotReassigning
                | DomainError::NoVariance
                | DomainError::UnresolvedVariance(_)
                | DomainError::PaycheckSkipped(_)
                | DomainError::DuplicatePaycheck(_)
                | DomainError::Invariant(_)
        )
    }

    pub(crate) fn not_found(kind: &'static str, id: &Id) -> Self {
        DomainError::NotFound { kind, id: id.clone() }
    }
}
