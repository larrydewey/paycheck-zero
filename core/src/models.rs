//! Authoritative domain entities (spec §2). All money is integer [`Cents`].

use crate::id::Id;
use crate::money::Cents;
use crate::recurrence::Recurrence;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MonthStatus {
    Draft,
    Locked,
}

impl MonthStatus {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            MonthStatus::Draft => "draft",
            MonthStatus::Locked => "locked",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "draft" => Some(MonthStatus::Draft),
            "locked" => Some(MonthStatus::Locked),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaycheckStatus {
    Planned,
    Received,
    Skipped,
}

impl PaycheckStatus {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            PaycheckStatus::Planned => "planned",
            PaycheckStatus::Received => "received",
            PaycheckStatus::Skipped => "skipped",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "planned" => Some(PaycheckStatus::Planned),
            "received" => Some(PaycheckStatus::Received),
            "skipped" => Some(PaycheckStatus::Skipped),
            _ => None,
        }
    }
}

/// How an income line produces paychecks (spec §2.2 `schedule_type`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "schedule_type", rename_all = "snake_case")]
pub enum Schedule {
    /// Explicit dates; the paychecks themselves are the expected dates.
    OneOff { dates: Vec<NaiveDate> },
    Recurring { recurrence_rule: Recurrence },
}

impl Schedule {
    #[must_use]
    pub fn type_str(&self) -> &'static str {
        match self {
            Schedule::OneOff { .. } => "one_off",
            Schedule::Recurring { .. } => "recurring",
        }
    }
}

/// A source of income for a month (spec §2.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IncomeLine {
    pub id: Id,
    pub name: String,
    /// Default amount of each generated paycheck.
    pub planned_amount: Cents,
    /// `None` for one-off lines (their dates live on the paychecks).
    pub recurrence_rule: Option<Recurrence>,
}

impl IncomeLine {
    #[must_use]
    pub fn schedule_type(&self) -> &'static str {
        if self.recurrence_rule.is_some() {
            "recurring"
        } else {
            "one_off"
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

impl Paycheck {
    /// `actual − planned` once an actual has been recorded (spec §2.9).
    #[must_use]
    pub fn variance(&self) -> Option<Cents> {
        match (self.status, self.actual_amount) {
            (PaycheckStatus::Skipped, _) | (_, None) => None,
            (_, Some(a)) => Some(a - self.planned_amount),
        }
    }

    #[must_use]
    pub fn has_variance(&self) -> bool {
        self.variance().is_some_and(|v| !v.is_zero())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CategoryKind {
    Standard,
    /// Lines carry `current_balance` / `minimum_payment` (spec §2.5).
    Debt,
}

impl CategoryKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            CategoryKind::Standard => "standard",
            CategoryKind::Debt => "debt",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "standard" => Some(CategoryKind::Standard),
            "debt" => Some(CategoryKind::Debt),
            _ => None,
        }
    }
}

/// A grouping of expense lines (spec §2.4, §14.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpenseCategory {
    pub id: Id,
    pub name: String,
    pub sort_order: i32,
    pub kind: CategoryKind,
}

/// A spending line under a category (spec §2.5).
///
/// There is deliberately no `planned_amount` field: it is always derived from
/// allocations (invariant 1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpenseLine {
    pub id: Id,
    pub category_id: Id,
    pub name: String,
    pub sort_order: i32,
    /// Debt lines only.
    pub current_balance: Option<Cents>,
    /// Debt lines only.
    pub minimum_payment: Option<Cents>,
    /// Non-binding hint copied from a source month's planned amount
    /// (spec §2.11 option 3). Never counts toward any total.
    pub target_amount: Option<Cents>,
}

/// The source of truth for planned spending (spec §2.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Allocation {
    pub id: Id,
    pub expense_line_id: Id,
    pub paycheck_id: Id,
    /// Always > 0 (invariant 5).
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
    pub expense_line_id: Option<Id>,
    pub paycheck_id: Option<Id>,
    /// Parts of one split payment share a group id (user request #9); each
    /// part has its own amount, line and paycheck.
    #[serde(default)]
    pub split_group: Option<Id>,
}
