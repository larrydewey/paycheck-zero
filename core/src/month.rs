//! The [`Month`] aggregate: every piece of a month's budget and the single
//! place where the spec's invariants (§2.6, §10) are enforced.
//!
//! Storage loads a `Month`, the service mutates it through these methods and
//! re-checks [`Month::check_invariants`], then persists it. Nothing here
//! depends on a database or web framework (spec §7.1).

use crate::error::DomainError;
use crate::id::Id;
use crate::money::Cents;
use crate::models::*;
use crate::recurrence::{in_month, Recurrence};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// Starter categories seeded on first use of a month (spec §2.4).
pub const STARTER_CATEGORIES: &[&str] = &[
    "Giving",
    "Saving",
    "Housing",
    "Transportation",
    "Food",
    "Personal",
    "Lifestyle",
    "Health",
    "Insurance",
    "Debt",
    "Other",
];

/// Maximum income line name length (spec §2.2).
pub const INCOME_NAME_MAX: usize = 100;
/// Maximum category / expense line name length.
pub const NAME_MAX: usize = 100;

/// One part of a split payment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SplitPart {
    /// Signed cents, like a transaction amount.
    pub amount: Cents,
    pub expense_line_id: Option<Id>,
    pub paycheck_id: Option<Id>,
}

/// Side effects of a cascading change that the user must be told about
/// (invariants 6 and 7).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Impact {
    /// Expense lines whose derived planned amount went down.
    pub reduced_lines: Vec<Id>,
    /// Dates of paychecks that were removed.
    pub removed_paychecks: Vec<NaiveDate>,
}

impl Impact {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.reduced_lines.is_empty() && self.removed_paychecks.is_empty()
    }

    fn touch_line(&mut self, line: &Id) {
        if !self.reduced_lines.contains(line) {
            self.reduced_lines.push(line.clone());
        }
    }

    fn merge(&mut self, other: Impact) {
        for l in &other.reduced_lines {
            self.touch_line(l);
        }
        self.removed_paychecks.extend(other.removed_paychecks);
    }
}

/// A month budget: the paycheck-first zero-based aggregate (spec §2.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Month {
    pub id: Id,
    /// First day of the budgeted month, e.g. `2026-09-01`.
    pub year_month: NaiveDate,
    pub status: MonthStatus,
    /// Locked month temporarily opened for variance re-assignment (spec §2.9).
    pub reassigning: bool,
    pub income_lines: Vec<IncomeLine>,
    pub paychecks: Vec<Paycheck>,
    pub categories: Vec<ExpenseCategory>,
    pub expense_lines: Vec<ExpenseLine>,
    pub allocations: Vec<Allocation>,
    pub transactions: Vec<Transaction>,
}

fn valid_name(name: &str, max: usize) -> Result<String, DomainError> {
    let trimmed = name.trim();
    let len = trimmed.chars().count();
    if len == 0 || len > max {
        return Err(DomainError::InvalidName { max });
    }
    Ok(trimmed.to_string())
}

fn clean_opt(s: Option<String>) -> Option<String> {
    s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

impl Month {
    #[must_use]
    pub fn new(id: Id, year_month: NaiveDate) -> Self {
        Self {
            id,
            year_month: crate::recurrence::first_of_month(year_month),
            status: MonthStatus::Draft,
            reassigning: false,
            income_lines: Vec::new(),
            paychecks: Vec::new(),
            categories: Vec::new(),
            expense_lines: Vec::new(),
            allocations: Vec::new(),
            transactions: Vec::new(),
        }
    }

    // ------------------------------------------------------------------
    // Lookups
    // ------------------------------------------------------------------

    #[must_use]
    pub fn is_locked(&self) -> bool {
        self.status == MonthStatus::Locked
    }

    /// Can allocations be changed right now (Draft, or re-assigning a variance)?
    #[must_use]
    pub fn allocations_editable(&self) -> bool {
        !self.is_locked() || self.reassigning
    }

    #[must_use]
    pub fn income_line(&self, id: &Id) -> Option<&IncomeLine> {
        self.income_lines.iter().find(|x| &x.id == id)
    }

    #[must_use]
    pub fn paycheck(&self, id: &Id) -> Option<&Paycheck> {
        self.paychecks.iter().find(|x| &x.id == id)
    }

    #[must_use]
    pub fn category(&self, id: &Id) -> Option<&ExpenseCategory> {
        self.categories.iter().find(|x| &x.id == id)
    }

    #[must_use]
    pub fn expense_line(&self, id: &Id) -> Option<&ExpenseLine> {
        self.expense_lines.iter().find(|x| &x.id == id)
    }

    #[must_use]
    pub fn allocation(&self, id: &Id) -> Option<&Allocation> {
        self.allocations.iter().find(|x| &x.id == id)
    }

    #[must_use]
    pub fn allocation_for(&self, paycheck: &Id, line: &Id) -> Option<&Allocation> {
        self.allocations
            .iter()
            .find(|a| &a.paycheck_id == paycheck && &a.expense_line_id == line)
    }

    #[must_use]
    pub fn transaction(&self, id: &Id) -> Option<&Transaction> {
        self.transactions.iter().find(|x| &x.id == id)
    }

    #[must_use]
    pub fn is_debt_line(&self, line: &Id) -> bool {
        self.expense_line(line)
            .and_then(|l| self.category(&l.category_id))
            .is_some_and(|c| c.kind == CategoryKind::Debt)
    }

    /// Paychecks in date order.
    #[must_use]
    pub fn paychecks_by_date(&self) -> Vec<&Paycheck> {
        let mut v: Vec<&Paycheck> = self.paychecks.iter().collect();
        v.sort_by_key(|p| p.date);
        v
    }

    /// Categories in display order.
    #[must_use]
    pub fn categories_sorted(&self) -> Vec<&ExpenseCategory> {
        let mut v: Vec<&ExpenseCategory> = self.categories.iter().collect();
        v.sort_by_key(|c| c.sort_order);
        v
    }

    /// Lines of a category in display order.
    #[must_use]
    pub fn lines_of(&self, category: &Id) -> Vec<&ExpenseLine> {
        let mut v: Vec<&ExpenseLine> = self.expense_lines.iter().filter(|l| &l.category_id == category).collect();
        v.sort_by_key(|l| l.sort_order);
        v
    }

    fn paycheck_mut(&mut self, id: &Id) -> Result<&mut Paycheck, DomainError> {
        self.paychecks
            .iter_mut()
            .find(|x| &x.id == id)
            .ok_or_else(|| DomainError::not_found("paycheck", id))
    }

    fn line_mut(&mut self, id: &Id) -> Result<&mut ExpenseLine, DomainError> {
        self.expense_lines
            .iter_mut()
            .find(|x| &x.id == id)
            .ok_or_else(|| DomainError::not_found("expense line", id))
    }

    fn category_mut(&mut self, id: &Id) -> Result<&mut ExpenseCategory, DomainError> {
        self.categories
            .iter_mut()
            .find(|x| &x.id == id)
            .ok_or_else(|| DomainError::not_found("category", id))
    }

    // ------------------------------------------------------------------
    // Derived values (never stored)
    // ------------------------------------------------------------------

    /// Invariant 1: a line's planned amount is the sum of its allocations.
    #[must_use]
    pub fn line_planned(&self, line: &Id) -> Cents {
        self.allocations.iter().filter(|a| &a.expense_line_id == line).map(|a| a.amount).sum()
    }

    /// Spec §2.10: money out on the line, less refunds linked to it.
    #[must_use]
    pub fn line_spent(&self, line: &Id) -> Cents {
        self.transactions.iter().filter(|t| t.expense_line_id.as_ref() == Some(line)).map(Transaction::spending).sum()
    }

    #[must_use]
    pub fn line_remaining(&self, line: &Id) -> Cents {
        self.line_planned(line) - self.line_spent(line)
    }

    /// Target hint not yet funded (spec §2.11 option 3), never negative.
    #[must_use]
    pub fn line_unfunded_target(&self, line: &Id) -> Cents {
        let target = self.expense_line(line).and_then(|l| l.target_amount).unwrap_or(Cents::ZERO);
        let short = target - self.line_planned(line);
        if short.is_positive() {
            short
        } else {
            Cents::ZERO
        }
    }

    /// Sum of a paycheck's allocations.
    #[must_use]
    pub fn paycheck_allocated(&self, paycheck: &Id) -> Cents {
        self.allocations.iter().filter(|a| &a.paycheck_id == paycheck).map(|a| a.amount).sum()
    }

    /// Money on a paycheck that has not been assigned yet.
    #[must_use]
    pub fn paycheck_unallocated(&self, paycheck: &Id) -> Cents {
        match self.paycheck(paycheck) {
            Some(p) if p.status != PaycheckStatus::Skipped => p.planned_amount - self.paycheck_allocated(paycheck),
            _ => Cents::ZERO,
        }
    }

    /// Spending explicitly tagged to a paycheck, less tagged refunds.
    #[must_use]
    pub fn paycheck_tagged_expense(&self, paycheck: &Id) -> Cents {
        self.transactions.iter().filter(|t| t.paycheck_id.as_ref() == Some(paycheck)).map(Transaction::spending).sum()
    }

    /// Spending on a line tagged to a paycheck.
    #[must_use]
    pub fn paycheck_line_spent(&self, paycheck: &Id, line: &Id) -> Cents {
        self.transactions
            .iter()
            .filter(|t| t.paycheck_id.as_ref() == Some(paycheck) && t.expense_line_id.as_ref() == Some(line))
            .map(Transaction::spending)
            .sum()
    }

    /// Tagged spending per line for a paycheck: (line or None, amount spent).
    fn tagged_by_line(&self, paycheck: &Id) -> Vec<(Option<Id>, Cents)> {
        let mut out: Vec<(Option<Id>, Cents)> = Vec::new();
        for t in self.transactions.iter().filter(|t| t.paycheck_id.as_ref() == Some(paycheck) && (t.is_spending() || t.is_refund())) {
            match out.iter_mut().find(|(l, _)| *l == t.expense_line_id) {
                Some((_, v)) => *v += t.spending(),
                None => out.push((t.expense_line_id.clone(), t.spending())),
            }
        }
        out
    }

    /// Spending tagged to a paycheck that its plan didn't cover: spending on a
    /// line beyond what this paycheck put into that line, plus spending on
    /// lines it doesn't fund (or on no line at all).
    #[must_use]
    pub fn paycheck_unplanned_spending(&self, paycheck: &Id) -> Cents {
        self.tagged_by_line(paycheck)
            .into_iter()
            .map(|(line, spent)| {
                let alloc = line.as_ref().and_then(|l| self.allocation_for(paycheck, l)).map_or(Cents::ZERO, |a| a.amount);
                let over = spent - alloc;
                if over.is_positive() { over } else { Cents::ZERO }
            })
            .sum()
    }

    /// What this paycheck still has budgeted in the lines it funds, after
    /// spending tagged to it on those lines.
    #[must_use]
    pub fn paycheck_budget_left(&self, paycheck: &Id) -> Cents {
        let tagged = self.tagged_by_line(paycheck);
        self.allocations
            .iter()
            .filter(|a| &a.paycheck_id == paycheck)
            .map(|a| {
                let spent = tagged.iter().find(|(l, _)| l.as_ref() == Some(&a.expense_line_id)).map_or(Cents::ZERO, |(_, v)| *v);
                let left = a.amount - spent;
                if left.is_positive() { left } else { Cents::ZERO }
            })
            .sum()
    }

    /// Per-paycheck Safe-to-Spend (spec §2.7): money on the paycheck that has
    /// no job yet, minus spending tagged to it that the plan didn't cover.
    ///
    /// Owner decision (2026-09-25): the spec's literal formula subtracted all
    /// tagged spending on top of allocations, which double-counted spending
    /// on lines the paycheck already funds and made every fully assigned
    /// paycheck negative. Spending on a funded line now draws from that
    /// line's allocation first. Skipped paychecks have nothing to spend.
    #[must_use]
    pub fn safe_to_spend(&self, paycheck: &Id) -> Cents {
        match self.paycheck(paycheck) {
            Some(p) if p.status != PaycheckStatus::Skipped => {
                p.planned_amount - self.paycheck_allocated(paycheck) - self.paycheck_unplanned_spending(paycheck)
            }
            _ => Cents::ZERO,
        }
    }

    /// Rolling look-ahead (spec §2.7): Safe-to-Spend of paychecks dated today or later.
    #[must_use]
    pub fn rolling_available(&self, today: NaiveDate) -> Cents {
        self.paychecks.iter().filter(|p| p.date >= today).map(|p| self.safe_to_spend(&p.id)).sum()
    }

    /// Planned income; skipped paychecks do not count.
    #[must_use]
    pub fn total_planned_income(&self) -> Cents {
        self.paychecks
            .iter()
            .filter(|p| p.status != PaycheckStatus::Skipped)
            .map(|p| p.planned_amount)
            .sum()
    }

    /// Sum of derived line planned amounts (== sum of all allocations).
    #[must_use]
    pub fn total_planned_expense(&self) -> Cents {
        self.expense_lines.iter().map(|l| self.line_planned(&l.id)).sum()
    }

    /// Invariant 4: must be exactly zero to lock.
    #[must_use]
    pub fn zero_difference(&self) -> Cents {
        self.total_planned_income() - self.total_planned_expense()
    }

    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.zero_difference().is_zero()
    }

    /// Invariant 3: a paycheck is valid only when completely allocated.
    #[must_use]
    pub fn is_paycheck_fully_allocated(&self, paycheck: &Id) -> bool {
        self.paycheck(paycheck).is_some_and(|p| {
            p.status == PaycheckStatus::Skipped || self.paycheck_allocated(paycheck) == p.planned_amount
        })
    }

    #[must_use]
    pub fn has_variance(&self) -> bool {
        self.paychecks.iter().any(Paycheck::has_variance)
    }

    // ------------------------------------------------------------------
    // Guards
    // ------------------------------------------------------------------

    fn require_draft(&self) -> Result<(), DomainError> {
        if self.is_locked() {
            Err(DomainError::Locked)
        } else {
            Ok(())
        }
    }

    fn require_allocations_editable(&self) -> Result<(), DomainError> {
        if self.allocations_editable() {
            Ok(())
        } else {
            Err(DomainError::Locked)
        }
    }

    fn require_in_month(&self, date: NaiveDate) -> Result<(), DomainError> {
        if in_month(date, self.year_month) {
            Ok(())
        } else {
            Err(DomainError::DateOutsideMonth(date))
        }
    }

    // ------------------------------------------------------------------
    // Categories and expense lines (Draft only, spec §2.4)
    // ------------------------------------------------------------------

    /// Adds any missing starter categories (idempotent, spec §2.4).
    pub fn seed_starter_categories(&mut self) {
        for name in STARTER_CATEGORIES {
            if !self.categories.iter().any(|c| c.name == *name) {
                let kind = if *name == "Debt" { CategoryKind::Debt } else { CategoryKind::Standard };
                let order = self.next_category_order();
                self.categories.push(ExpenseCategory { id: Id::generate(), name: (*name).to_string(), sort_order: order, kind });
            }
        }
    }

    fn next_category_order(&self) -> i32 {
        self.categories.iter().map(|c| c.sort_order).max().unwrap_or(0) + 10
    }

    pub fn add_category(&mut self, name: &str, kind: CategoryKind) -> Result<Id, DomainError> {
        self.require_draft()?;
        let name = valid_name(name, NAME_MAX)?;
        let id = Id::generate();
        let order = self.next_category_order();
        self.categories.push(ExpenseCategory { id: id.clone(), name, sort_order: order, kind });
        Ok(id)
    }

    pub fn rename_category(&mut self, id: &Id, name: &str) -> Result<(), DomainError> {
        self.require_draft()?;
        let name = valid_name(name, NAME_MAX)?;
        self.category_mut(id)?.name = name;
        Ok(())
    }

    /// Swaps a category with its neighbour (`up` = earlier in the list).
    pub fn move_category(&mut self, id: &Id, up: bool) -> Result<(), DomainError> {
        self.require_draft()?;
        // Categories with lines and empty ones are listed separately, so a
        // move steps past the other kind until the visible order changes.
        let has_lines = |m: &Self, c: &Id| m.expense_lines.iter().any(|l| &l.category_id == c);
        let mine = has_lines(self, id);
        loop {
            let order: Vec<Id> = self.categories_sorted().iter().map(|c| c.id.clone()).collect();
            let pos = order.iter().position(|x| x == id).ok_or_else(|| DomainError::not_found("category", id))?;
            let other = if up { pos.checked_sub(1) } else { Some(pos + 1).filter(|p| *p < order.len()) };
            let Some(o) = other else { break };
            let (a, b) = (order[pos].clone(), order[o].clone());
            let sa = self.category_mut(&a)?.sort_order;
            let sb = self.category_mut(&b)?.sort_order;
            self.category_mut(&a)?.sort_order = sb;
            self.category_mut(&b)?.sort_order = sa;
            if has_lines(self, &b) == mine {
                break;
            }
        }
        Ok(())
    }

    /// Deletes a category, its lines and their allocations.
    pub fn delete_category(&mut self, id: &Id) -> Result<Impact, DomainError> {
        self.require_draft()?;
        if self.category(id).is_none() {
            return Err(DomainError::not_found("category", id));
        }
        let lines: Vec<Id> = self.expense_lines.iter().filter(|l| &l.category_id == id).map(|l| l.id.clone()).collect();
        let mut impact = Impact::default();
        for l in &lines {
            impact.merge(self.delete_expense_line(l)?);
        }
        self.categories.retain(|c| &c.id != id);
        Ok(impact)
    }

    pub fn add_expense_line(&mut self, category: &Id, name: &str) -> Result<Id, DomainError> {
        self.require_draft()?;
        let name = valid_name(name, NAME_MAX)?;
        if self.category(category).is_none() {
            return Err(DomainError::not_found("category", category));
        }
        let order = self
            .expense_lines
            .iter()
            .filter(|l| &l.category_id == category)
            .map(|l| l.sort_order)
            .max()
            .unwrap_or(0)
            + 10;
        let id = Id::generate();
        self.expense_lines.push(ExpenseLine {
            id: id.clone(),
            category_id: category.clone(),
            name,
            sort_order: order,
            current_balance: None,
            minimum_payment: None,
            target_amount: None,
        });
        Ok(id)
    }

    pub fn rename_expense_line(&mut self, id: &Id, name: &str) -> Result<(), DomainError> {
        self.require_draft()?;
        let name = valid_name(name, NAME_MAX)?;
        self.line_mut(id)?.name = name;
        Ok(())
    }

    /// Moves a line to another category (appended at the end).
    pub fn set_line_category(&mut self, id: &Id, category: &Id) -> Result<(), DomainError> {
        self.require_draft()?;
        if self.category(category).is_none() {
            return Err(DomainError::not_found("category", category));
        }
        let order = self
            .expense_lines
            .iter()
            .filter(|l| &l.category_id == category)
            .map(|l| l.sort_order)
            .max()
            .unwrap_or(0)
            + 10;
        let line = self.line_mut(id)?;
        if &line.category_id != category {
            line.category_id = category.clone();
            line.sort_order = order;
        }
        if !self.is_debt_line(id) {
            let line = self.line_mut(id)?;
            line.current_balance = None;
            line.minimum_payment = None;
        }
        Ok(())
    }

    /// Swaps a line with its neighbour inside its category.
    pub fn move_expense_line(&mut self, id: &Id, up: bool) -> Result<(), DomainError> {
        self.require_draft()?;
        let cat = self.expense_line(id).ok_or_else(|| DomainError::not_found("expense line", id))?.category_id.clone();
        let order: Vec<Id> = self.lines_of(&cat).iter().map(|l| l.id.clone()).collect();
        let pos = order.iter().position(|x| x == id).unwrap_or(0);
        let other = if up { pos.checked_sub(1) } else { Some(pos + 1).filter(|p| *p < order.len()) };
        if let Some(o) = other {
            let (a, b) = (order[pos].clone(), order[o].clone());
            let sa = self.line_mut(&a)?.sort_order;
            let sb = self.line_mut(&b)?.sort_order;
            self.line_mut(&a)?.sort_order = sb;
            self.line_mut(&b)?.sort_order = sa;
        }
        Ok(())
    }

    /// Drag-and-drop: puts a category at `index` in display order.
    pub fn place_category(&mut self, id: &Id, index: usize) -> Result<(), DomainError> {
        self.require_draft()?;
        let mut order: Vec<Id> = self.categories_sorted().iter().map(|c| c.id.clone()).collect();
        let pos = order.iter().position(|x| x == id).ok_or_else(|| DomainError::not_found("category", id))?;
        let item = order.remove(pos);
        order.insert(index.min(order.len()), item);
        for (i, cid) in order.iter().enumerate() {
            self.category_mut(cid)?.sort_order = (i32::try_from(i).unwrap_or(i32::MAX / 10) + 1) * 10;
        }
        Ok(())
    }

    /// Drag-and-drop: puts a line at `index` within `category` (which may be
    /// a different category than its current one).
    pub fn place_line(&mut self, id: &Id, category: &Id, index: usize) -> Result<(), DomainError> {
        self.require_draft()?;
        if self.category(category).is_none() {
            return Err(DomainError::not_found("category", category));
        }
        if self.expense_line(id).is_none() {
            return Err(DomainError::not_found("expense line", id));
        }
        self.set_line_category(id, category)?;
        let mut order: Vec<Id> = self.lines_of(category).iter().map(|l| l.id.clone()).filter(|l| l != id).collect();
        order.insert(index.min(order.len()), id.clone());
        for (i, lid) in order.iter().enumerate() {
            self.line_mut(lid)?.sort_order = (i32::try_from(i).unwrap_or(i32::MAX / 10) + 1) * 10;
        }
        Ok(())
    }

    /// Sets the Debt-only fields (spec §2.5).
    pub fn set_debt_fields(
        &mut self,
        id: &Id,
        current_balance: Option<Cents>,
        minimum_payment: Option<Cents>,
    ) -> Result<(), DomainError> {
        self.require_draft()?;
        if self.expense_line(id).is_none() {
            return Err(DomainError::not_found("expense line", id));
        }
        if !self.is_debt_line(id) {
            return Err(DomainError::NotDebtLine);
        }
        if current_balance.is_some_and(Cents::is_negative) || minimum_payment.is_some_and(Cents::is_negative) {
            return Err(DomainError::NegativeAmount);
        }
        let line = self.line_mut(id)?;
        line.current_balance = current_balance;
        line.minimum_payment = minimum_payment;
        Ok(())
    }

    /// Deletes a line and its allocations; linked transactions are unlinked.
    pub fn delete_expense_line(&mut self, id: &Id) -> Result<Impact, DomainError> {
        self.require_draft()?;
        if self.expense_line(id).is_none() {
            return Err(DomainError::not_found("expense line", id));
        }
        self.allocations.retain(|a| &a.expense_line_id != id);
        self.expense_lines.retain(|l| &l.id != id);
        for t in &mut self.transactions {
            if t.expense_line_id.as_ref() == Some(id) {
                t.expense_line_id = None;
            }
        }
        Ok(Impact::default())
    }

    // ------------------------------------------------------------------
    // Income lines and paychecks (spec §2.2, §2.3)
    // ------------------------------------------------------------------

    fn schedule_dates(&self, schedule: &Schedule) -> Result<Vec<NaiveDate>, DomainError> {
        let mut dates = match schedule {
            Schedule::OneOff { dates } => {
                for d in dates {
                    self.require_in_month(*d)?;
                }
                dates.clone()
            }
            Schedule::Recurring { recurrence_rule } => {
                if !recurrence_rule.is_valid() {
                    return Err(DomainError::InvalidRecurrence);
                }
                recurrence_rule.dates_in_month(self.year_month)
            }
        };
        dates.sort();
        dates.dedup();
        if dates.is_empty() {
            return Err(DomainError::NoDatesInMonth);
        }
        Ok(dates)
    }

    /// Adds an income line and generates its paychecks.
    pub fn add_income_line(&mut self, name: &str, planned_amount: Cents, schedule: Schedule) -> Result<Id, DomainError> {
        self.require_draft()?;
        let name = valid_name(name, INCOME_NAME_MAX)?;
        if planned_amount.is_negative() {
            return Err(DomainError::NegativeAmount);
        }
        let dates = self.schedule_dates(&schedule)?;
        let id = Id::generate();
        let recurrence_rule = match schedule {
            Schedule::Recurring { recurrence_rule } => Some(recurrence_rule),
            Schedule::OneOff { .. } => None,
        };
        self.income_lines.push(IncomeLine { id: id.clone(), name, planned_amount, recurrence_rule });
        for date in dates {
            self.push_paycheck(&id, date, planned_amount);
        }
        Ok(id)
    }

    fn push_paycheck(&mut self, line: &Id, date: NaiveDate, amount: Cents) -> Id {
        let id = Id::generate();
        self.paychecks.push(Paycheck {
            id: id.clone(),
            income_line_id: line.clone(),
            date,
            planned_amount: amount,
            actual_amount: None,
            status: PaycheckStatus::Planned,
        });
        id
    }

    /// Updates an income line. A new amount flows to every paycheck still at
    /// the old default (overridden paychecks keep their amount). A new
    /// schedule keeps paychecks whose dates still match, adds new dates, and
    /// removes the rest together with their allocations.
    pub fn update_income_line(
        &mut self,
        id: &Id,
        name: Option<&str>,
        planned_amount: Option<Cents>,
        schedule: Option<Schedule>,
    ) -> Result<Impact, DomainError> {
        self.require_draft()?;
        let line = self.income_line(id).ok_or_else(|| DomainError::not_found("income line", id))?.clone();
        let name = name.map(|n| valid_name(n, INCOME_NAME_MAX)).transpose()?;
        if planned_amount.is_some_and(Cents::is_negative) {
            return Err(DomainError::NegativeAmount);
        }
        let new_dates = schedule.as_ref().map(|s| self.schedule_dates(s)).transpose()?;
        let mut impact = Impact::default();

        if let Some(amount) = planned_amount {
            let targets: Vec<Id> = self
                .paychecks
                .iter()
                .filter(|p| &p.income_line_id == id && p.planned_amount == line.planned_amount)
                .map(|p| p.id.clone())
                .collect();
            for p in targets {
                impact.merge(self.set_paycheck_planned(&p, amount)?);
            }
        }
        let amount = planned_amount.unwrap_or(line.planned_amount);

        if let (Some(schedule), Some(dates)) = (schedule, new_dates) {
            let existing: Vec<(Id, NaiveDate)> =
                self.paychecks.iter().filter(|p| &p.income_line_id == id).map(|p| (p.id.clone(), p.date)).collect();
            for (pid, date) in &existing {
                if !dates.contains(date) {
                    impact.merge(self.delete_paycheck(pid)?);
                }
            }
            for date in dates {
                if !existing.iter().any(|(_, d)| *d == date) {
                    self.push_paycheck(id, date, amount);
                }
            }
            if let Some(l) = self.income_lines.iter_mut().find(|l| &l.id == id) {
                l.recurrence_rule = match schedule {
                    Schedule::Recurring { recurrence_rule } => Some(recurrence_rule),
                    Schedule::OneOff { .. } => None,
                };
            }
        }

        if let Some(l) = self.income_lines.iter_mut().find(|l| &l.id == id) {
            if let Some(n) = name {
                l.name = n;
            }
            l.planned_amount = amount;
        }
        Ok(impact)
    }

    /// Deletes an income line, its paychecks and their allocations.
    pub fn delete_income_line(&mut self, id: &Id) -> Result<Impact, DomainError> {
        self.require_draft()?;
        if self.income_line(id).is_none() {
            return Err(DomainError::not_found("income line", id));
        }
        let pcs: Vec<Id> = self.paychecks.iter().filter(|p| &p.income_line_id == id).map(|p| p.id.clone()).collect();
        let mut impact = Impact::default();
        for p in &pcs {
            impact.merge(self.delete_paycheck(p)?);
        }
        self.income_lines.retain(|l| &l.id != id);
        Ok(impact)
    }

    /// Adds a paycheck on a specific date (mid-month additions, spec §5).
    pub fn add_paycheck(&mut self, income_line: &Id, date: NaiveDate, amount: Option<Cents>) -> Result<Id, DomainError> {
        self.require_draft()?;
        self.require_in_month(date)?;
        let line = self.income_line(income_line).ok_or_else(|| DomainError::not_found("income line", income_line))?;
        let amount = amount.unwrap_or(line.planned_amount);
        if amount.is_negative() {
            return Err(DomainError::NegativeAmount);
        }
        if self.paychecks.iter().any(|p| &p.income_line_id == income_line && p.date == date) {
            return Err(DomainError::DuplicatePaycheck(date));
        }
        Ok(self.push_paycheck(income_line, date, amount))
    }

    fn remove_allocations_of(&mut self, paycheck: &Id) -> Impact {
        let mut impact = Impact::default();
        for a in self.allocations.iter().filter(|a| &a.paycheck_id == paycheck) {
            impact.touch_line(&a.expense_line_id);
        }
        self.allocations.retain(|a| &a.paycheck_id != paycheck);
        impact
    }

    /// Invariant 7: deleting a paycheck deletes its allocations.
    pub fn delete_paycheck(&mut self, id: &Id) -> Result<Impact, DomainError> {
        self.require_draft()?;
        let date = self.paycheck(id).ok_or_else(|| DomainError::not_found("paycheck", id))?.date;
        let mut impact = self.remove_allocations_of(id);
        self.paychecks.retain(|p| &p.id != id);
        for t in &mut self.transactions {
            if t.paycheck_id.as_ref() == Some(id) {
                t.paycheck_id = None;
            }
        }
        impact.removed_paychecks.push(date);
        Ok(impact)
    }

    /// Invariant 6: reducing a paycheck below its allocations reduces or
    /// deletes the excess, latest-created allocations first.
    pub fn set_paycheck_planned(&mut self, id: &Id, amount: Cents) -> Result<Impact, DomainError> {
        self.require_draft()?;
        self.set_planned_unchecked(id, amount)
    }

    fn set_planned_unchecked(&mut self, id: &Id, amount: Cents) -> Result<Impact, DomainError> {
        if amount.is_negative() {
            return Err(DomainError::NegativeAmount);
        }
        self.paycheck_mut(id)?.planned_amount = amount;
        let mut impact = Impact::default();
        let mut excess = self.paycheck_allocated(id) - amount;
        while excess.is_positive() {
            let Some(idx) = self.allocations.iter().rposition(|a| &a.paycheck_id == id) else {
                break;
            };
            let a = &mut self.allocations[idx];
            impact.touch_line(&a.expense_line_id.clone());
            if a.amount <= excess {
                excess -= a.amount;
                self.allocations.remove(idx);
            } else {
                a.amount -= excess;
                excess = Cents::ZERO;
            }
        }
        Ok(impact)
    }

    /// Changes a paycheck's status. Skipping deletes its allocations
    /// (invariant 7) and is a planning change, so it needs a Draft month.
    pub fn set_paycheck_status(&mut self, id: &Id, status: PaycheckStatus) -> Result<Impact, DomainError> {
        let current = self.paycheck(id).ok_or_else(|| DomainError::not_found("paycheck", id))?.status;
        if current == status {
            return Ok(Impact::default());
        }
        let mut impact = Impact::default();
        if status == PaycheckStatus::Skipped || current == PaycheckStatus::Skipped {
            self.require_draft()?;
        }
        if status == PaycheckStatus::Skipped {
            impact = self.remove_allocations_of(id);
            let p = self.paycheck_mut(id)?;
            p.actual_amount = None;
        }
        self.paycheck_mut(id)?.status = status;
        Ok(impact)
    }

    /// Records the amount actually received (allowed while locked, spec §2.9).
    pub fn set_paycheck_actual(&mut self, id: &Id, actual: Option<Cents>) -> Result<(), DomainError> {
        if actual.is_some_and(Cents::is_negative) {
            return Err(DomainError::NegativeAmount);
        }
        let p = self.paycheck_mut(id)?;
        if p.status == PaycheckStatus::Skipped {
            return Err(DomainError::PaycheckSkipped(id.clone()));
        }
        p.actual_amount = actual;
        p.status = if actual.is_some() { PaycheckStatus::Received } else { PaycheckStatus::Planned };
        Ok(())
    }

    // ------------------------------------------------------------------
    // Allocations (spec §2.6)
    // ------------------------------------------------------------------

    fn check_capacity(&self, paycheck: &Id, line: &Id, new_amount: Cents) -> Result<(), DomainError> {
        let p = self.paycheck(paycheck).ok_or_else(|| DomainError::not_found("paycheck", paycheck))?;
        if p.status == PaycheckStatus::Skipped {
            return Err(DomainError::PaycheckSkipped(paycheck.clone()));
        }
        if self.expense_line(line).is_none() {
            return Err(DomainError::not_found("expense line", line));
        }
        let existing = self.allocation_for(paycheck, line).map_or(Cents::ZERO, |a| a.amount);
        let total = self.paycheck_allocated(paycheck) - existing + new_amount;
        if total > p.planned_amount {
            return Err(DomainError::OverAllocated { paycheck: paycheck.clone(), over: total - p.planned_amount });
        }
        Ok(())
    }

    /// Sets the unique (paycheck, line) allocation to `amount`. Zero deletes
    /// it (invariant 5). Returns the allocation id when one remains.
    pub fn set_allocation(&mut self, paycheck: &Id, line: &Id, amount: Cents) -> Result<Option<Id>, DomainError> {
        self.require_allocations_editable()?;
        if amount.is_negative() {
            return Err(DomainError::NonPositiveAmount);
        }
        self.check_capacity(paycheck, line, amount)?;
        let idx = self.allocations.iter().position(|a| &a.paycheck_id == paycheck && &a.expense_line_id == line);
        match (idx, amount.is_zero()) {
            (Some(i), true) => {
                self.allocations.remove(i);
                Ok(None)
            }
            (None, true) => Ok(None),
            (Some(i), false) => {
                self.allocations[i].amount = amount;
                Ok(Some(self.allocations[i].id.clone()))
            }
            (None, false) => {
                let id = Id::generate();
                self.allocations.push(Allocation {
                    id: id.clone(),
                    expense_line_id: line.clone(),
                    paycheck_id: paycheck.clone(),
                    amount,
                });
                Ok(Some(id))
            }
        }
    }

    /// Strict create-or-update used by the REST API: amount must be > 0.
    pub fn allocate(&mut self, paycheck: &Id, line: &Id, amount: Cents) -> Result<Id, DomainError> {
        if !amount.is_positive() {
            return Err(DomainError::NonPositiveAmount);
        }
        self.set_allocation(paycheck, line, amount)?.ok_or(DomainError::NonPositiveAmount)
    }

    pub fn update_allocation(&mut self, id: &Id, amount: Cents) -> Result<(), DomainError> {
        if !amount.is_positive() {
            return Err(DomainError::NonPositiveAmount);
        }
        let a = self.allocation(id).ok_or_else(|| DomainError::not_found("allocation", id))?.clone();
        self.set_allocation(&a.paycheck_id, &a.expense_line_id, amount).map(|_| ())
    }

    pub fn delete_allocation(&mut self, id: &Id) -> Result<(), DomainError> {
        self.require_allocations_editable()?;
        if self.allocation(id).is_none() {
            return Err(DomainError::not_found("allocation", id));
        }
        self.allocations.retain(|a| &a.id != id);
        Ok(())
    }

    /// Moves part of a line's funding from one paycheck to another (spec §9).
    pub fn transfer(&mut self, from: &Id, to: &Id, line: &Id, amount: Cents) -> Result<(), DomainError> {
        self.require_allocations_editable()?;
        if !amount.is_positive() {
            return Err(DomainError::NonPositiveAmount);
        }
        if from == to {
            return Ok(());
        }
        let available = self.allocation_for(from, line).map_or(Cents::ZERO, |a| a.amount);
        if amount > available {
            return Err(DomainError::TransferExceedsAllocation { requested: amount, available });
        }
        let dest = self.allocation_for(to, line).map_or(Cents::ZERO, |a| a.amount);
        self.check_capacity(to, line, dest + amount)?;
        self.set_allocation(from, line, available - amount)?;
        self.set_allocation(to, line, dest + amount)?;
        Ok(())
    }

    /// Monthly-overview edit (spec §3, §14.6): sets a line's total planned
    /// amount by translating the change into allocation operations.
    /// Increases draw on paychecks with unallocated money, earliest first;
    /// decreases come off the line's latest-dated allocations first.
    pub fn set_line_planned(&mut self, line: &Id, total: Cents) -> Result<(), DomainError> {
        self.require_allocations_editable()?;
        if total.is_negative() {
            return Err(DomainError::NegativeAmount);
        }
        if self.expense_line(line).is_none() {
            return Err(DomainError::not_found("expense line", line));
        }
        let current = self.line_planned(line);
        let pcs: Vec<(Id, NaiveDate)> = self.paychecks_by_date().iter().map(|p| (p.id.clone(), p.date)).collect();
        if total > current {
            let mut need = total - current;
            let free: Cents = pcs.iter().map(|(p, _)| self.paycheck_unallocated(p)).filter(|c| c.is_positive()).sum();
            if free < need {
                return Err(DomainError::InsufficientUnallocated { short: need - free });
            }
            for (p, _) in &pcs {
                if need.is_zero() {
                    break;
                }
                let free = self.paycheck_unallocated(p);
                if !free.is_positive() {
                    continue;
                }
                let take = free.min(need);
                let existing = self.allocation_for(p, line).map_or(Cents::ZERO, |a| a.amount);
                self.set_allocation(p, line, existing + take)?;
                need -= take;
            }
        } else if total < current {
            let mut cut = current - total;
            for (p, _) in pcs.iter().rev() {
                if cut.is_zero() {
                    break;
                }
                let existing = self.allocation_for(p, line).map_or(Cents::ZERO, |a| a.amount);
                if existing.is_zero() {
                    continue;
                }
                let take = existing.min(cut);
                self.set_allocation(p, line, existing - take)?;
                cut -= take;
            }
        }
        Ok(())
    }

    /// The tithe amount for a paycheck: `percent` of its planned amount,
    /// rounded half-to-even to the cent.
    #[must_use]
    pub fn tithe_amount(&self, paycheck: &Id, percent: u32) -> Cents {
        self.paycheck(paycheck)
            .and_then(|p| crate::money::Rate::parse(&format!("0.{percent:02}")).map(|r| r.convert(p.planned_amount)))
            .unwrap_or(Cents::ZERO)
    }

    /// The line one-click giving funds: "Tithe" in the Giving category
    /// (either is created when missing).
    pub fn tithe_line(&mut self) -> Result<Id, DomainError> {
        let giving = match self.categories.iter().find(|c| c.name.eq_ignore_ascii_case("Giving")) {
            Some(c) => c.id.clone(),
            None => self.add_category("Giving", CategoryKind::Standard)?,
        };
        match self.lines_of(&giving).iter().find(|l| l.name.eq_ignore_ascii_case("Tithe")) {
            Some(l) => Ok(l.id.clone()),
            None => self.add_expense_line(&giving, "Tithe"),
        }
    }

    /// One-click giving: sets this paycheck's Tithe allocation to `percent`
    /// of the paycheck (creating Giving → Tithe if needed).
    pub fn give_percent(&mut self, paycheck: &Id, percent: u32) -> Result<Id, DomainError> {
        self.require_allocations_editable()?;
        if percent == 0 || percent > 99 {
            return Err(DomainError::NonPositiveAmount);
        }
        let amount = self.tithe_amount(paycheck, percent);
        if !amount.is_positive() {
            return Err(DomainError::NonPositiveAmount);
        }
        let line = match self.categories.iter().find(|c| c.name.eq_ignore_ascii_case("Giving")).and_then(|c| {
            self.lines_of(&c.id).iter().find(|l| l.name.eq_ignore_ascii_case("Tithe")).map(|l| l.id.clone())
        }) {
            Some(l) => l,
            None => self.tithe_line()?,
        };
        self.set_allocation(paycheck, &line, amount)?;
        Ok(line)
    }

    // ------------------------------------------------------------------
    // Locking and variance re-assignment (spec §2.1, §2.9)
    // ------------------------------------------------------------------

    pub fn lock(&mut self) -> Result<(), DomainError> {
        if self.is_locked() {
            return Err(DomainError::AlreadyLocked);
        }
        if !self.is_zero() {
            return Err(DomainError::NotZero { diff: self.zero_difference() });
        }
        self.status = MonthStatus::Locked;
        Ok(())
    }

    /// Temporarily re-opens allocation editing on a locked month so a
    /// paycheck variance can be re-assigned.
    pub fn begin_reassignment(&mut self) -> Result<(), DomainError> {
        if !self.is_locked() {
            return Err(DomainError::NotLocked);
        }
        if !self.has_variance() && self.is_zero() {
            return Err(DomainError::NoVariance);
        }
        self.reassigning = true;
        Ok(())
    }

    /// Makes a paycheck's actual its planned amount (Draft, or while
    /// re-assigning). Excess allocations cascade per invariant 6.
    pub fn apply_actual(&mut self, id: &Id) -> Result<Impact, DomainError> {
        self.require_allocations_editable()?;
        let p = self.paycheck(id).ok_or_else(|| DomainError::not_found("paycheck", id))?;
        let actual = p.actual_amount.ok_or_else(|| DomainError::NoActual(id.clone()))?;
        self.set_planned_unchecked(id, actual)
    }

    /// Returns a re-assigned month to the locked state once every variance is
    /// applied and the month is back at exact zero.
    pub fn finish_reassignment(&mut self) -> Result<(), DomainError> {
        if !self.reassigning {
            return Err(DomainError::NotReassigning);
        }
        if let Some(p) = self.paychecks.iter().find(|p| p.has_variance()) {
            return Err(DomainError::UnresolvedVariance(p.id.clone()));
        }
        if !self.is_zero() {
            return Err(DomainError::NotZero { diff: self.zero_difference() });
        }
        self.reassigning = false;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Transactions (spec §2.8) — allowed in every state
    // ------------------------------------------------------------------

    fn validate_transaction(&self, t: &Transaction) -> Result<(), DomainError> {
        if t.amount.is_zero() {
            return Err(DomainError::ZeroTransaction);
        }
        if let Some(to) = &t.transfer_account_id {
            if t.account_id.as_ref().is_none_or(|from| from == to) || !t.amount.is_negative() || t.split_group.is_some() {
                return Err(DomainError::InvalidTransfer);
            }
        }
        if let Some(l) = &t.expense_line_id {
            if self.expense_line(l).is_none() {
                return Err(DomainError::not_found("expense line", l));
            }
        }
        if let Some(p) = &t.paycheck_id {
            if self.paycheck(p).is_none() {
                return Err(DomainError::not_found("paycheck", p));
            }
        }
        Ok(())
    }

    /// Income transactions explicitly tagged to a paycheck (deposits). A
    /// refund tagged to it is money back on spending, not pay.
    #[must_use]
    pub fn paycheck_deposits(&self, paycheck: &Id) -> Cents {
        self.transactions
            .iter()
            .filter(|t| t.paycheck_id.as_ref() == Some(paycheck) && t.amount.is_positive() && !t.is_refund())
            .map(|t| t.amount)
            .sum()
    }

    /// Reconciles a paycheck with its deposits: when income transactions are
    /// tagged to it, their sum becomes the amount actually received. If the
    /// last deposit goes away, an actual that came from deposits is cleared
    /// (a manually entered actual is left alone).
    fn reconcile(&mut self, paycheck: &Id, deposits_before: Cents) {
        let after = self.paycheck_deposits(paycheck);
        let Ok(p) = self.paycheck_mut(paycheck) else { return };
        if p.status == PaycheckStatus::Skipped {
            return;
        }
        if after.is_positive() {
            p.actual_amount = Some(after);
            p.status = PaycheckStatus::Received;
        } else if deposits_before.is_positive() && p.actual_amount == Some(deposits_before) {
            p.actual_amount = None;
            p.status = PaycheckStatus::Planned;
        }
    }

    fn with_reconcile<T>(&mut self, touched: &[Option<Id>], f: impl FnOnce(&mut Self) -> Result<T, DomainError>) -> Result<T, DomainError> {
        let before: Vec<(Id, Cents)> = touched.iter().flatten().map(|p| (p.clone(), self.paycheck_deposits(p))).collect();
        let out = f(self)?;
        for (p, b) in before {
            self.reconcile(&p, b);
        }
        Ok(out)
    }

    pub fn add_transaction(&mut self, mut t: Transaction) -> Result<Id, DomainError> {
        t.payee = clean_opt(t.payee);
        t.notes = clean_opt(t.notes);
        self.validate_transaction(&t)?;
        if self.transaction(&t.id).is_some() {
            return Err(DomainError::Invariant(format!("transaction {} already exists", t.id)));
        }
        let touched = [t.paycheck_id.clone()];
        self.with_reconcile(&touched, |m| {
            let id = t.id.clone();
            m.transactions.push(t);
            Ok(id)
        })
    }

    pub fn update_transaction(&mut self, mut t: Transaction) -> Result<(), DomainError> {
        t.payee = clean_opt(t.payee);
        t.notes = clean_opt(t.notes);
        self.validate_transaction(&t)?;
        let old = self.transaction(&t.id).ok_or_else(|| DomainError::not_found("transaction", &t.id))?.paycheck_id.clone();
        let touched = [old, t.paycheck_id.clone()];
        self.with_reconcile(&touched, |m| {
            if let Some(slot) = m.transactions.iter_mut().find(|x| x.id == t.id) {
                *slot = t;
            }
            Ok(())
        })
    }

    pub fn delete_transaction(&mut self, id: &Id) -> Result<(), DomainError> {
        let old = self.transaction(id).ok_or_else(|| DomainError::not_found("transaction", id))?.paycheck_id.clone();
        self.with_reconcile(&[old], |m| {
            m.transactions.retain(|x| &x.id != id);
            Ok(())
        })
    }

    /// Parts of a split payment, in their stored order.
    #[must_use]
    pub fn split_parts(&self, group: &Id) -> Vec<&Transaction> {
        self.transactions.iter().filter(|t| t.split_group.as_ref() == Some(group)).collect()
    }

    /// Creates (or replaces, when `group` is given) a split payment: one
    /// payment divided into parts, each with its own amount, line and
    /// paycheck. Needs at least two parts, all expenses or all income.
    pub fn save_split(
        &mut self,
        group: Option<&Id>,
        date: NaiveDate,
        payee: Option<String>,
        notes: Option<String>,
        account: Option<Id>,
        parts: Vec<SplitPart>,
    ) -> Result<Id, DomainError> {
        if parts.len() < 2 {
            return Err(DomainError::SplitTooFew);
        }
        if parts.iter().any(|p| p.amount.is_zero()) {
            return Err(DomainError::ZeroTransaction);
        }
        if !(parts.iter().all(|p| p.amount.is_negative()) || parts.iter().all(|p| p.amount.is_positive())) {
            return Err(DomainError::SplitMixedSigns);
        }
        let payee = clean_opt(payee);
        let notes = clean_opt(notes);
        let group_id = match group {
            Some(g) => {
                if self.split_parts(g).is_empty() {
                    return Err(DomainError::not_found("split transaction", g));
                }
                g.clone()
            }
            None => Id::generate(),
        };
        let new: Vec<Transaction> = parts
            .into_iter()
            .map(|p| Transaction {
                id: Id::generate(),
                date,
                amount: p.amount,
                payee: payee.clone(),
                notes: notes.clone(),
                expense_line_id: p.expense_line_id,
                paycheck_id: p.paycheck_id,
                split_group: Some(group_id.clone()),
                account_id: account.clone(),
                transfer_account_id: None,
                external_id: None,
            })
            .collect();
        for t in &new {
            self.validate_transaction(t)?;
        }
        let mut touched: Vec<Option<Id>> = self.split_parts(&group_id).iter().map(|t| t.paycheck_id.clone()).collect();
        touched.extend(new.iter().map(|t| t.paycheck_id.clone()));
        touched.sort();
        touched.dedup();
        let gid = group_id.clone();
        self.with_reconcile(&touched, move |m| {
            // Keep the split where it was in the list.
            let at = m.transactions.iter().position(|t| t.split_group.as_ref() == Some(&gid)).unwrap_or(m.transactions.len());
            m.transactions.retain(|t| t.split_group.as_ref() != Some(&gid));
            let at = at.min(m.transactions.len());
            for (i, t) in new.into_iter().enumerate() {
                m.transactions.insert(at + i, t);
            }
            Ok(gid)
        })
    }

    /// Takes a transaction (every part, for a split payment) out of this
    /// month, so it can be counted in another one.
    pub fn take_transaction(&mut self, id: &Id) -> Result<Vec<Transaction>, DomainError> {
        let x = self.transaction(id).ok_or_else(|| DomainError::not_found("transaction", id))?;
        let parts: Vec<Transaction> = match &x.split_group {
            Some(g) => self.split_parts(g).into_iter().cloned().collect(),
            None => vec![x.clone()],
        };
        let ids: Vec<Id> = parts.iter().map(|t| t.id.clone()).collect();
        let touched: Vec<Option<Id>> = parts.iter().map(|t| t.paycheck_id.clone()).collect();
        self.with_reconcile(&touched, |m| {
            m.transactions.retain(|t| !ids.contains(&t.id));
            Ok(())
        })?;
        Ok(parts)
    }

    /// Adds transactions taken out of `from`. Lines carry over when this
    /// month has one with the same category and name; paychecks don't.
    pub fn receive_transactions(&mut self, from: &Month, parts: Vec<Transaction>) -> Result<(), DomainError> {
        for mut t in parts {
            t.expense_line_id = t.expense_line_id.as_ref().and_then(|l| self.same_line(from, l));
            t.paycheck_id = None;
            self.add_transaction(t)?;
        }
        Ok(())
    }

    /// This month's line matching `line` of `other` by category and name.
    fn same_line(&self, other: &Month, line: &Id) -> Option<Id> {
        let l = other.expense_line(line)?;
        let cat = other.category(&l.category_id)?;
        self.expense_lines
            .iter()
            .find(|x| x.name == l.name && self.category(&x.category_id).is_some_and(|c| c.name == cat.name))
            .map(|x| x.id.clone())
    }

    pub fn delete_split(&mut self, group: &Id) -> Result<(), DomainError> {
        let touched: Vec<Option<Id>> = self.split_parts(group).iter().map(|t| t.paycheck_id.clone()).collect();
        if touched.is_empty() {
            return Err(DomainError::not_found("split transaction", group));
        }
        self.with_reconcile(&touched, |m| {
            m.transactions.retain(|t| t.split_group.as_ref() != Some(group));
            Ok(())
        })
    }

    // ------------------------------------------------------------------
    // Full re-validation (spec §10)
    // ------------------------------------------------------------------

    /// Re-checks every structural invariant. The service calls this after
    /// every mutation and refuses to persist a month that fails it.
    pub fn check_invariants(&self) -> Result<(), DomainError> {
        let fail = |m: String| Err(DomainError::Invariant(m));
        let mut seen: Vec<(&Id, &Id)> = Vec::new();
        for a in &self.allocations {
            if !a.amount.is_positive() {
                return fail(format!("allocation {} is not positive", a.id));
            }
            let Some(p) = self.paycheck(&a.paycheck_id) else {
                return fail(format!("allocation {} references a missing paycheck", a.id));
            };
            if p.status == PaycheckStatus::Skipped {
                return fail(format!("skipped paycheck {} has allocations", p.id));
            }
            if self.expense_line(&a.expense_line_id).is_none() {
                return fail(format!("allocation {} references a missing line", a.id));
            }
            let key = (&a.paycheck_id, &a.expense_line_id);
            if seen.contains(&key) {
                return fail(format!("duplicate allocation for paycheck {} and line {}", key.0, key.1));
            }
            seen.push(key);
        }
        for p in &self.paychecks {
            if p.planned_amount.is_negative() {
                return fail(format!("paycheck {} has a negative planned amount", p.id));
            }
            let allocated = self.paycheck_allocated(&p.id);
            if allocated > p.planned_amount {
                return Err(DomainError::OverAllocated { paycheck: p.id.clone(), over: allocated - p.planned_amount });
            }
            if self.income_line(&p.income_line_id).is_none() {
                return fail(format!("paycheck {} has no income line", p.id));
            }
            if !in_month(p.date, self.year_month) {
                return fail(format!("paycheck {} is dated outside the month", p.id));
            }
        }
        for l in &self.expense_lines {
            if self.category(&l.category_id).is_none() {
                return fail(format!("line {} has no category", l.id));
            }
        }
        if self.is_locked() && !self.reassigning && !self.is_zero() {
            return fail("a locked month must stay at zero".into());
        }
        Ok(())
    }
}

/// Parses a recurrence rule from stored JSON; a helper for adapters.
#[must_use]
pub fn parse_rule(json: Option<&str>) -> Option<Recurrence> {
    json.and_then(Recurrence::from_json)
}
