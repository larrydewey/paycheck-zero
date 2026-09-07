//! Authoritative domain models (spec §2). All money is integer [`Cents`].

use crate::id::Id;
use crate::money::Cents;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleType {
    OneOff,
    Recurring,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MonthStatus {
    Draft,
    Locked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaycheckStatus {
    Planned,
    Received,
    Skipped,
}

/// A source of income for a month (spec §2.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IncomeLine {
    pub id: Id,
    pub name: String,
    /// Per-paycheck default amount in cents (spec §2.2 `planned_amount`).
    pub planned_amount: Cents,
    pub schedule_type: ScheduleType,
    /// iCal RRULE or equivalent JSON; required when `schedule_type` is `Recurring`.
    pub recurrence_rule: Option<String>,
}

impl IncomeLine {
    #[must_use]
    pub fn new(name: impl Into<String>, planned_amount: Cents, schedule_type: ScheduleType, recurrence_rule: Option<String>) -> Self {
        Self {
            id: Id::generate(),
            name: name.into(),
            planned_amount,
            schedule_type,
            recurrence_rule,
        }
    }
}

/// A concrete instance of an [`IncomeLine`] on a specific date (spec §2.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Paycheck {
    pub id: Id,
    pub income_line_id: Id,
    pub date: NaiveDate,
    pub planned_amount: Cents,
    pub actual_amount: Option<Cents>,
    pub status: PaycheckStatus,
}

/// A hierarchical grouping of expense lines (spec §2.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpenseCategory {
    pub id: Id,
    pub name: String,
    pub sort_order: i32,
}

/// A spending line under a category (spec §2.5).
///
/// `planned_amount` is **never stored** here; it is always derived from
/// allocations (invariant 1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpenseLine {
    pub id: Id,
    pub category_id: Id,
    pub name: String,
    /// Debt lines only (spec §2.5).
    pub current_balance: Option<Cents>,
    /// Debt lines only (spec §2.5).
    pub minimum_payment: Option<Cents>,
}

/// The source of truth for planned spending (spec §2.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Allocation {
    pub id: Id,
    pub expense_line_id: Id,
    pub paycheck_id: Id,
    /// Must always be > 0 (invariant 5).
    pub amount: Cents,
}

/// A manual transaction (spec §2.8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transaction {
    pub id: Id,
    pub date: NaiveDate,
    /// Signed cents: positive = income, negative = expense.
    pub amount: Cents,
    pub payee: Option<String>,
    pub notes: Option<String>,
    /// Links the transaction to an expense line (drives that line's Spent).
    pub expense_line_id: Option<Id>,
    /// Optional explicit paycheck tag (drives that paycheck's Safe-to-Spend).
    pub paycheck_id: Option<Id>,
}
