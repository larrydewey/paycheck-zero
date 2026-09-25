//! Derived, read-only projections of a [`Month`] for the API and UI
//! (spec §2.7, §14). All money stays integer [`Cents`].

use crate::id::Id;
use crate::models::*;
use crate::money::Cents;
use crate::month::Month;
use crate::recurrence::in_month;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// One paycheck's contribution to a line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Funder {
    pub allocation_id: Id,
    pub paycheck_id: Id,
    pub date: NaiveDate,
    pub amount: Cents,
}

/// An expense line with its derived Planned / Spent / Remaining triad.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineView {
    pub id: Id,
    pub category_id: Id,
    pub name: String,
    pub planned: Cents,
    pub spent: Cents,
    pub remaining: Cents,
    pub is_debt: bool,
    pub current_balance: Option<Cents>,
    pub minimum_payment: Option<Cents>,
    pub target: Option<Cents>,
    pub unfunded_target: Cents,
    pub funders: Vec<Funder>,
    /// Amount funded by the paycheck being viewed (paycheck view only).
    pub this_paycheck: Cents,
}

/// A category with live totals of its lines (spec §14.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CategoryView {
    pub id: Id,
    pub name: String,
    pub kind: CategoryKind,
    pub planned: Cents,
    pub spent: Cents,
    pub remaining: Cents,
    /// Paycheck view only: total this paycheck puts into the category.
    pub this_paycheck: Cents,
    pub lines: Vec<LineView>,
}

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
    pub unallocated: Cents,
    pub fully_allocated: bool,
    pub tagged_expense: Cents,
    pub safe_to_spend: Cents,
    pub variance: Option<Cents>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonthView {
    pub id: Id,
    pub year_month: NaiveDate,
    pub status: MonthStatus,
    pub reassigning: bool,
    pub total_planned_income: Cents,
    pub total_planned_expense: Cents,
    pub zero_difference: Cents,
    pub is_zero: bool,
    pub total_spent: Cents,
    pub rolling_available: Cents,
    pub has_variance: bool,
    pub paychecks: Vec<PaycheckView>,
    pub categories: Vec<CategoryView>,
}

impl Month {
    fn line_view(&self, l: &ExpenseLine, paycheck: Option<&Id>) -> LineView {
        let planned = self.line_planned(&l.id);
        let spent = self.line_spent(&l.id);
        let mut funders: Vec<Funder> = self
            .allocations
            .iter()
            .filter(|a| a.expense_line_id == l.id)
            .filter_map(|a| {
                self.paycheck(&a.paycheck_id).map(|p| Funder {
                    allocation_id: a.id.clone(),
                    paycheck_id: p.id.clone(),
                    date: p.date,
                    amount: a.amount,
                })
            })
            .collect();
        funders.sort_by_key(|f| f.date);
        let this_paycheck = paycheck
            .and_then(|p| self.allocation_for(p, &l.id))
            .map_or(Cents::ZERO, |a| a.amount);
        LineView {
            id: l.id.clone(),
            category_id: l.category_id.clone(),
            name: l.name.clone(),
            planned,
            spent,
            remaining: planned - spent,
            is_debt: self.is_debt_line(&l.id),
            current_balance: l.current_balance,
            minimum_payment: l.minimum_payment,
            target: l.target_amount,
            unfunded_target: self.line_unfunded_target(&l.id),
            funders,
            this_paycheck,
        }
    }

    fn category_view(&self, c: &ExpenseCategory, lines: Vec<LineView>) -> CategoryView {
        let planned = lines.iter().map(|l| l.planned).sum();
        let spent = lines.iter().map(|l| l.spent).sum();
        CategoryView {
            id: c.id.clone(),
            name: c.name.clone(),
            kind: c.kind,
            planned,
            spent,
            remaining: planned - spent,
            this_paycheck: lines.iter().map(|l| l.this_paycheck).sum(),
            lines,
        }
    }

    /// Every category with every line (monthly overview).
    #[must_use]
    pub fn category_views(&self) -> Vec<CategoryView> {
        self.categories_sorted()
            .into_iter()
            .map(|c| {
                let lines = self.lines_of(&c.id).into_iter().map(|l| self.line_view(l, None)).collect();
                self.category_view(c, lines)
            })
            .collect()
    }

    /// Only the lines a paycheck funds, grouped by category (paycheck view).
    #[must_use]
    pub fn funding_views(&self, paycheck: &Id) -> Vec<CategoryView> {
        self.categories_sorted()
            .into_iter()
            .filter_map(|c| {
                let lines: Vec<LineView> = self
                    .lines_of(&c.id)
                    .into_iter()
                    .filter(|l| self.allocation_for(paycheck, &l.id).is_some())
                    .map(|l| self.line_view(l, Some(paycheck)))
                    .collect();
                (!lines.is_empty()).then(|| self.category_view(c, lines))
            })
            .collect()
    }

    #[must_use]
    pub fn paycheck_view(&self, p: &Paycheck) -> PaycheckView {
        let allocated = self.paycheck_allocated(&p.id);
        PaycheckView {
            id: p.id.clone(),
            income_line_id: p.income_line_id.clone(),
            income_line_name: self.income_line(&p.income_line_id).map(|l| l.name.clone()).unwrap_or_default(),
            date: p.date,
            planned_amount: p.planned_amount,
            actual_amount: p.actual_amount,
            status: p.status,
            allocated,
            unallocated: self.paycheck_unallocated(&p.id),
            fully_allocated: self.is_paycheck_fully_allocated(&p.id),
            tagged_expense: self.paycheck_tagged_expense(&p.id),
            safe_to_spend: self.safe_to_spend(&p.id),
            variance: p.variance(),
        }
    }

    #[must_use]
    pub fn view(&self, today: NaiveDate) -> MonthView {
        MonthView {
            id: self.id.clone(),
            year_month: self.year_month,
            status: self.status,
            reassigning: self.reassigning,
            total_planned_income: self.total_planned_income(),
            total_planned_expense: self.total_planned_expense(),
            zero_difference: self.zero_difference(),
            is_zero: self.is_zero(),
            total_spent: self.expense_lines.iter().map(|l| self.line_spent(&l.id)).sum(),
            rolling_available: self.rolling_available(today),
            has_variance: self.has_variance(),
            paychecks: self.paychecks_by_date().into_iter().map(|p| self.paycheck_view(p)).collect(),
            categories: self.category_views(),
        }
    }

    /// The paycheck to land on (spec §3): when today falls inside the month,
    /// the current paycheck (latest dated on or before today) or else the next
    /// upcoming one; for a month entirely in the past or future, the first.
    #[must_use]
    pub fn default_paycheck(&self, today: NaiveDate) -> Option<Id> {
        let pcs = self.paychecks_by_date();
        if pcs.is_empty() {
            return None;
        }
        if in_month(today, self.year_month) {
            if let Some(p) = pcs.iter().rev().find(|p| p.date <= today) {
                return Some(p.id.clone());
            }
            if let Some(p) = pcs.iter().find(|p| p.date >= today) {
                return Some(p.id.clone());
            }
        }
        pcs.first().map(|p| p.id.clone())
    }
}
