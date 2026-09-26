//! Lightweight reports (spec §15), derived purely from planned amounts,
//! allocations and transactions.
//!
//! Definitions used throughout:
//! - planned income  = non-skipped paychecks' planned amounts
//! - actual income   = recorded paycheck actuals + positive transactions not
//!   linked to any line and not tagged to a paycheck (tagged deposits are
//!   already reflected in that paycheck's actual)
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
    /// Line-level drill-down (matched across months by name).
    #[serde(default)]
    pub lines: Vec<LineFigures>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineFigures {
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
                    for l in &c.lines {
                        match x.lines.iter_mut().find(|y| y.name == l.name) {
                            Some(y) => {
                                y.planned += l.planned;
                                y.actual += l.actual;
                            }
                            None => x.lines.push(l.clone()),
                        }
                    }
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
                lines: lines
                    .iter()
                    .map(|l| LineFigures { name: l.name.clone(), planned: m.line_planned(&l.id), actual: m.line_spent(&l.id) })
                    .collect(),
            }
        })
        .collect();
    let unlinked_expense: Cents = m
        .transactions
        .iter()
        .filter(|t| t.needs_line())
        .map(|t| t.amount.abs())
        .sum();
    if unlinked_expense.is_positive() {
        categories.push(CategoryFigures { name: UNCATEGORIZED.into(), planned: Cents::ZERO, actual: unlinked_expense, lines: Vec::new() });
    }
    let actual_income = m
        .paychecks
        .iter()
        .filter(|p| p.status != PaycheckStatus::Skipped)
        .filter_map(|p| p.actual_amount)
        .sum::<Cents>()
        + m.transactions
            .iter()
            .filter(|t| t.expense_line_id.is_none() && t.paycheck_id.is_none() && t.amount.is_positive())
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
    /// actual − planned, over paychecks whose actual has been recorded
    /// (plus unlinked positive transactions). Paychecks not yet received
    /// don't count as a shortfall.
    pub income_variance: Cents,
    pub paychecks_received: usize,
    pub paychecks_total: usize,
    /// actual − planned.
    pub expense_variance: Cents,
    pub spending: SpendingStatus,
}

#[must_use]
pub fn summary_cards(m: &Month) -> SummaryCards {
    let f = month_figures(m);
    let expense_variance = f.actual_expense - f.planned_expense;
    let counted: Vec<&crate::models::Paycheck> =
        m.paychecks.iter().filter(|p| p.status != PaycheckStatus::Skipped).collect();
    let received: Vec<&&crate::models::Paycheck> = counted.iter().filter(|p| p.actual_amount.is_some()).collect();
    let unlinked_income: Cents = m
        .transactions
        .iter()
        .filter(|t| t.expense_line_id.is_none() && t.paycheck_id.is_none() && t.amount.is_positive())
        .map(|t| t.amount)
        .sum();
    let income_variance =
        received.iter().filter_map(|p| p.variance()).sum::<Cents>() + unlinked_income;
    SummaryCards {
        planned_income: f.planned_income,
        actual_income: f.actual_income,
        planned_expense: f.planned_expense,
        actual_expense: f.actual_expense,
        remaining_to_zero: m.zero_difference(),
        income_variance,
        paychecks_received: received.len(),
        paychecks_total: counted.len(),
        expense_variance,
        spending: match expense_variance.get() {
            v if v > 0 => SpendingStatus::Over,
            0 => SpendingStatus::OnPlan,
            _ => SpendingStatus::Under,
        },
    }
}

/// The `n` months ending at `through` (oldest first), with figures for the
/// months that have a budget.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trend {
    pub months: Vec<NaiveDate>,
    pub figures: Vec<Option<Figures>>,
    /// Category names in first-seen order across the window.
    pub category_names: Vec<String>,
}

#[must_use]
pub fn trend(through: NaiveDate, all: &[Month], n: usize) -> Trend {
    let mut months = Vec::with_capacity(n);
    let mut ym = crate::recurrence::first_of_month(through);
    for _ in 0..n.max(1) {
        months.push(ym);
        ym = previous_month(ym);
    }
    months.reverse();
    let figures: Vec<Option<Figures>> =
        months.iter().map(|ym| all.iter().find(|m| m.year_month == *ym).map(month_figures)).collect();
    let mut names: Vec<String> = Vec::new();
    for f in figures.iter().flatten() {
        for c in &f.categories {
            if !names.contains(&c.name) {
                names.push(c.name.clone());
            }
        }
    }
    Trend { months, figures, category_names: names }
}

/// Spending and income grouped by payee over a date range (inclusive).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayeeRow {
    pub payee: String,
    pub count: usize,
    pub spent: Cents,
    pub received: Cents,
}

/// Payee name used for transactions without one.
pub const NO_PAYEE: &str = "(no payee)";

#[must_use]
pub fn payees(all: &[Month], from: NaiveDate, to: NaiveDate) -> Vec<PayeeRow> {
    let mut rows: Vec<PayeeRow> = Vec::new();
    let mut seen_groups: Vec<crate::Id> = Vec::new();
    for m in all {
        // Moving money between your own accounts isn't a payee.
        for t in m.transactions.iter().filter(|t| t.date >= from && t.date <= to && (!t.is_transfer() || t.expense_line_id.is_some())) {
            let name = t.payee.clone().unwrap_or_else(|| NO_PAYEE.to_string());
            let key = name.to_lowercase();
            let idx = match rows.iter().position(|r| r.payee.to_lowercase() == key) {
                Some(i) => i,
                None => {
                    rows.push(PayeeRow { payee: name, count: 0, spent: Cents::ZERO, received: Cents::ZERO });
                    rows.len() - 1
                }
            };
            let r = &mut rows[idx];
            // A split payment counts once.
            let new_payment = match &t.split_group {
                Some(g) if seen_groups.contains(g) => false,
                Some(g) => {
                    seen_groups.push(g.clone());
                    true
                }
                None => true,
            };
            if new_payment {
                r.count += 1;
            }
            if t.amount.is_negative() {
                r.spent += t.amount.abs();
            } else {
                r.received += t.amount;
            }
        }
    }
    rows.sort_by(|a, b| b.spent.cmp(&a.spent).then(b.received.cmp(&a.received)).then(a.payee.cmp(&b.payee)));
    rows
}

/// Every transaction in a date range across months, oldest first.
#[must_use]
pub fn transactions_between(all: &[Month], from: NaiveDate, to: NaiveDate) -> Vec<(&Month, &crate::models::Transaction)> {
    let mut out: Vec<(&Month, &crate::models::Transaction)> = all
        .iter()
        .flat_map(|m| m.transactions.iter().filter(move |t| t.date >= from && t.date <= to).map(move |t| (m, t)))
        .collect();
    out.sort_by_key(|(m, t)| (t.date, m.year_month));
    out
}
