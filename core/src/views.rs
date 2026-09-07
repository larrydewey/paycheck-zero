//! Derived, read-only views surfaced to the API and UI (spec §14, §15).
//! All money is integer [`Cents`]; the web layer converts to dollars for display.

use crate::id::Id;
use crate::money::Cents;
use crate::models::{MonthStatus, PaycheckStatus};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// A single expense line with its derived Planned / Spent / Remaining triad.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineView {
    pub id: Id,
    pub category_id: Id,
    pub name: String,
    /// Derived: sum of the line's allocations (spec §2.5, invariant 1).
    pub planned: Cents,
    /// Derived: sum of linked expense transactions (spec §2.10).
    pub spent: Cents,
    /// Read-only: `planned - spent`.
    pub remaining: Cents,
    pub is_debt: bool,
    pub current_balance: Option<Cents>,
    pub minimum_payment: Option<Cents>,
}

/// A category with per-line detail and live category totals (spec §14.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CategoryView {
    pub id: Id,
    pub name: String,
    pub sort_order: i32,
    pub planned: Cents,
    pub spent: Cents,
    pub remaining: Cents,
    pub lines: Vec<LineView>,
}

/// A paycheck with allocation and safe-to-spend detail (spec §14.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaycheckView {
    pub id: Id,
    pub income_line_id: Id,
    pub income_line_name: String,
    pub date: NaiveDate,
    pub planned_amount: Cents,
    pub actual_amount: Option<Cents>,
    pub status: PaycheckStatus,
    pub allocated: Cents,
    pub remaining_to_allocate: Cents,
    pub is_fully_allocated: bool,
    /// Per-paycheck safe-to-spend (spec §2.7, primary).
    pub safe_to_spend: Cents,
    /// `actual - planned`, when an actual is recorded (spec §2.9).
    pub variance: Option<Cents>,
}

/// Aggregate monthly summary (spec §14.6, §15).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonthSummary {
    pub id: Id,
    pub year_month: NaiveDate,
    pub status: MonthStatus,
    pub archived: bool,
    pub total_planned_income: Cents,
    pub total_planned_expense: Cents,
    /// `total_planned_income - total_planned_expense`; must be zero to lock.
    pub zero_difference: Cents,
    pub is_zero: bool,
    pub total_spent: Cents,
    /// Rolling look-ahead safe-to-spend (spec §2.7, secondary).
    pub rolling_available: Cents,
    pub paychecks: Vec<PaycheckView>,
    pub categories: Vec<CategoryView>,
}
