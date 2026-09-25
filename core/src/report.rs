//! Lightweight reports (spec §15), derived purely from planned amounts,
//! allocations and transactions.
//!
//! Definitions used throughout:
//! - planned income  = non-skipped paychecks' planned amounts
//! - actual income   = recorded paycheck actuals + positive transactions not
//!   linked to any line
//! - planned expense = sum of allocations
//! - actual expense  = Spent of every line (spec §2.10) + absolute value of
//!   expense transactions not linked to any line ("Uncategorized")
//!
//! Categories are matched across months by name.

use crate::models::PaycheckStatus;
use crate::money::Cents;
use crate::month::Month;
use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};

/// Name of the pseudo-category for unlinked expense transactions.
pub const UNCATEGORIZED: &str = "Uncategorized";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CategoryFigures {
    pub name: String,
    pub planned: Cents,
    pub actual: Cents,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Figures {
    pub planned_income: Cents,
    pub actual_income: Cents,
    pub planned_expense: Cents,
    pub actual_expense: Cents,
    pub categories: Vec<CategoryFigures>,
}

impl Figures {
    fn merge(&mut self, other: &Figures) {
        self.planned_income += other.planned_income;
        self.actual_income += other.actual_income;
        self.planned_expense += other.planned_expense;
        self.actual_expense += other.actual_expense;
        for c in &other.categories {
            match self.categories.iter_mut().find(|x| x.name == c.name) {
                Some(x) => {
                    x.planned += c.planned;
                    x.actual += c.actual;
                }
                None => self.categories.push(c.clone()),
            }
        }
    }

    #[must_use]
    pub fn category(&self, name: &str) -> Option<&CategoryFigures> {
        self.categories.iter().find(|c| c.name == name)
    }
}

/// Figures for one month.
#[must_use]
pub fn month_figures(m: &Month) -> Figures {
    let mut categories: Vec<CategoryFigures> = m
        .categories_sorted()
        .into_iter()
        .map(|c| {
            let lines = m.lines_of(&c.id);
            CategoryFigures {
                name: c.name.clone(),
                planned: lines.iter().map(|l| m.line_planned(&l.id)).sum(),
                actual: lines.iter().map(|l| m.line_spent(&l.id)).sum(),
            }
        })
        .collect();
    let unlinked_expense: Cents = m
        .transactions
        .iter()
        .filter(|t| t.expense_line_id.is_none() && t.amount.is_negative())
        .map(|t| t.amount.abs())
        .sum();
    if unlinked_expense.is_positive() {
        categories.push(CategoryFigures { name: UNCATEGORIZED.into(), planned: Cents::ZERO, actual: unlinked_expense });
    }
    let actual_income = m
        .paychecks
        .iter()
        .filter(|p| p.status != PaycheckStatus::Skipped)
        .filter_map(|p| p.actual_amount)
        .sum::<Cents>()
        + m.transactions
            .iter()
            .filter(|t| t.expense_line_id.is_none() && t.amount.is_positive())
            .map(|t| t.amount)
            .sum::<Cents>();
    Figures {
        planned_income: m.total_planned_income(),
        actual_income,
        planned_expense: m.total_planned_expense(),
        actual_expense: categories.iter().map(|c| c.actual).sum(),
        categories,
    }
}

/// A selected month next to a comparison period (MoM or YoY).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comparison {
    pub current_month: NaiveDate,
    pub current: Figures,
    pub other_month: NaiveDate,
    /// `None` when no data exists for the comparison month.
    pub other: Option<Figures>,
    /// Union of category names, current month's order first.
    pub category_names: Vec<String>,
}

fn compare(current: &Month, other_month: NaiveDate, other: Option<&Month>) -> Comparison {
    let cur = month_figures(current);
    let oth = other.map(month_figures);
    let mut names: Vec<String> = cur.categories.iter().map(|c| c.name.clone()).collect();
    if let Some(o) = &oth {
        for c in &o.categories {
            if !names.contains(&c.name) {
                names.push(c.name.clone());
            }
        }
    }
    Comparison { current_month: current.year_month, current: cur, other_month, other: oth, category_names: names }
}

#[must_use]
pub fn previous_month(ym: NaiveDate) -> NaiveDate {
    if ym.month() == 1 {
        NaiveDate::from_ymd_opt(ym.year() - 1, 12, 1).unwrap_or(ym)
    } else {
        NaiveDate::from_ymd_opt(ym.year(), ym.month() - 1, 1).unwrap_or(ym)
    }
}

#[must_use]
pub fn same_month_last_year(ym: NaiveDate) -> NaiveDate {
    NaiveDate::from_ymd_opt(ym.year() - 1, ym.month(), 1).unwrap_or(ym)
}

/// Month-over-month: `all` is every month the user has.
#[must_use]
pub fn month_over_month(current: &Month, all: &[Month]) -> Comparison {
    let prev = previous_month(current.year_month);
    compare(current, prev, all.iter().find(|m| m.year_month == prev))
}

/// Year-over-year for the selected month.
#[must_use]
pub fn year_over_year(current: &Month, all: &[Month]) -> Comparison {
    let prev = same_month_last_year(current.year_month);
    compare(current, prev, all.iter().find(|m| m.year_month == prev))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct YearToDate {
    pub from: NaiveDate,
    pub through: NaiveDate,
    pub months_included: usize,
    pub figures: Figures,
}

/// January 1 of the selected month's year through the end of that month.
#[must_use]
pub fn year_to_date(current: &Month, all: &[Month]) -> YearToDate {
    let from = NaiveDate::from_ymd_opt(current.year_month.year(), 1, 1).unwrap_or(current.year_month);
    let mut months: Vec<&Month> =
        all.iter().filter(|m| m.year_month >= from && m.year_month <= current.year_month).collect();
    if !months.iter().any(|m| m.id == current.id) {
        months.push(current);
    }
    months.sort_by_key(|m| m.year_month);
    let mut figures = Figures::default();
    for m in &months {
        figures.merge(&month_figures(m));
    }
    YearToDate { from, through: current.year_month, months_included: months.len(), figures }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpendingStatus {
    Under,
    OnPlan,
    Over,
}

/// Quick-win summary cards (spec §15.1 item 4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SummaryCards {
    pub planned_income: Cents,
    pub actual_income: Cents,
    pub planned_expense: Cents,
    pub actual_expense: Cents,
    /// Remaining-to-zero (planned income − planned expense).
    pub remaining_to_zero: Cents,
    /// actual − planned.
    pub income_variance: Cents,
    /// actual − planned.
    pub expense_variance: Cents,
    pub spending: SpendingStatus,
}

#[must_use]
pub fn summary_cards(m: &Month) -> SummaryCards {
    let f = month_figures(m);
    let expense_variance = f.actual_expense - f.planned_expense;
    SummaryCards {
        planned_income: f.planned_income,
        actual_income: f.actual_income,
        planned_expense: f.planned_expense,
        actual_expense: f.actual_expense,
        remaining_to_zero: m.zero_difference(),
        income_variance: f.actual_income - f.planned_income,
        expense_variance,
        spending: match expense_variance.get() {
            v if v > 0 => SpendingStatus::Over,
            0 => SpendingStatus::OnPlan,
            _ => SpendingStatus::Under,
        },
    }
}
