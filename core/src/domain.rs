//! The [`Month`] aggregate. It owns every piece of a month's data and is the
//! single place the spec's invariants (§2.6, §10) are enforced. Storage loads a
//! [`Month`], the service mutates it through these methods, then persists it —
//! keeping this crate free of any web/db dependency (spec §7.1).

use crate::error::DomainError;
use crate::id::Id;
use crate::money::Cents;
use crate::models::*;
use crate::views::*;
use chrono::NaiveDate;

/// Starter categories seeded on first use of a month (spec §2.4).
pub const STARTER_CATEGORIES: &[&str] = &[
    "Giving", "Saving", "Housing", "Transportation", "Food", "Personal", "Lifestyle", "Health",
    "Insurance", "Debt", "Other",
];

/// A full month budget: the paycheck-first zero-based aggregate (spec §2.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Month {
    pub id: Id,
    /// First day of the budgeted month, e.g. `2026-09-01`.
    pub year_month: NaiveDate,
    pub status: MonthStatus,
    /// Soft-deleted / archived (spec §2.12).
    pub archived: bool,
    pub income_lines: Vec<IncomeLine>,
    pub paychecks: Vec<Paycheck>,
    pub categories: Vec<ExpenseCategory>,
    pub expense_lines: Vec<ExpenseLine>,
    /// Source of truth for planned spending (spec §2.6).
    pub allocations: Vec<Allocation>,
    pub transactions: Vec<Transaction>,
}

impl Month {
    #[must_use]
    pub fn new(id: Id, year_month: NaiveDate) -> Self {
        Self {
            id,
            year_month,
            status: MonthStatus::Draft,
            archived: false,
            income_lines: Vec::new(),
            paychecks: Vec::new(),
            categories: Vec::new(),
            expense_lines: Vec::new(),
            allocations: Vec::new(),
            transactions: Vec::new(),
        }
    }

    // ------------------------------------------------------------------
    // Getters
    // ------------------------------------------------------------------

    #[must_use]
    pub fn is_draft(&self) -> bool {
        self.status == MonthStatus::Draft
    }

    #[must_use]
    pub fn is_locked(&self) -> bool {
        self.status == MonthStatus::Locked
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

    fn paycheck_index(&self, id: &Id) -> Option<usize> {
        self.paychecks.iter().position(|x| &x.id == id)
    }

    fn allocation_index_for(&self, paycheck: &Id, expense_line: &Id) -> Option<usize> {
        self.allocations
            .iter()
            .position(|a| &a.paycheck_id == paycheck && &a.expense_line_id == expense_line)
    }

    // ------------------------------------------------------------------
    // Derived values (never stored)
    // ------------------------------------------------------------------

    /// Invariant 1: a line's planned amount is the sum of its allocations.
    #[must_use]
    pub fn line_planned(&self, line: &Id) -> Cents {
        Cents::from_cents(
            self.allocations
                .iter()
                .filter(|a| &a.expense_line_id == line)
                .map(|a| a.amount.as_cents())
                .sum(),
        )
    }

    /// Spec §2.10: Spent = sum of linked expense transactions (absolute value).
    #[must_use]
    pub fn line_spent(&self, line: &Id) -> Cents {
        Cents::from_cents(
            self.transactions
                .iter()
                .filter(|t| t.expense_line_id.as_ref() == Some(line) && t.amount.is_negative())
                .map(|t| t.amount.as_cents().abs())
                .sum(),
        )
    }

    #[must_use]
    pub fn line_remaining(&self, line: &Id) -> Cents {
        self.line_planned(line) - self.line_spent(line)
    }

    /// Sum of a paycheck's allocations (invariant 2 bound).
    #[must_use]
    pub fn paycheck_allocated(&self, paycheck: &Id) -> Cents {
        Cents::from_cents(
            self.allocations
                .iter()
                .filter(|a| &a.paycheck_id == paycheck)
                .map(|a| a.amount.as_cents())
                .sum(),
        )
    }

    /// Sum of expense transactions explicitly tagged to a paycheck.
    #[must_use]
    pub fn paycheck_tagged_expense(&self, paycheck: &Id) -> Cents {
        Cents::from_cents(
            self.transactions
                .iter()
                .filter(|t| t.paycheck_id.as_ref() == Some(paycheck) && t.amount.is_negative())
                .map(|t| t.amount.as_cents().abs())
                .sum(),
        )
    }

    /// Per-paycheck safe-to-spend (spec §2.7, the primary daily number).
    #[must_use]
    pub fn safe_to_spend(&self, paycheck: &Id) -> Cents {
        let Some(p) = self.paycheck(paycheck) else {
            return Cents::ZERO;
        };
        p.planned_amount - self.paycheck_allocated(paycheck) - self.paycheck_tagged_expense(paycheck)
    }

    /// Rolling / look-ahead safe-to-spend (spec §2.7, secondary).
    #[must_use]
    pub fn rolling_available(&self, today: NaiveDate) -> Cents {
        Cents::from_cents(
            self.paychecks
                .iter()
                .filter(|p| p.date >= today)
                .map(|p| self.safe_to_spend(&p.id).as_cents())
                .sum(),
        )
    }

    /// Skipped paychecks bring in no money, so they do not count as income.
    #[must_use]
    pub fn total_planned_income(&self) -> Cents {
        Cents::from_cents(
            self.paychecks
                .iter()
                .filter(|p| p.status != PaycheckStatus::Skipped)
                .map(|p| p.planned_amount.as_cents())
                .sum(),
        )
    }

    /// Sum of derived line planned amounts == sum of all allocations.
    #[must_use]
    pub fn total_planned_expense(&self) -> Cents {
        Cents::from_cents(self.allocations.iter().map(|a| a.amount.as_cents()).sum())
    }

    /// Invariant 4: monthly zero difference. Must be zero before locking.
    #[must_use]
    pub fn zero_difference(&self) -> Cents {
        self.total_planned_income() - self.total_planned_expense()
    }

    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.zero_difference().is_zero()
    }

    #[must_use]
    pub fn can_lock(&self) -> bool {
        self.is_zero()
    }

    /// Invariant 3: is a paycheck fully allocated?
    #[must_use]
    pub fn is_paycheck_fully_allocated(&self, paycheck: &Id) -> bool {
        self.paycheck(paycheck)
            .is_some_and(|p| self.paycheck_allocated(paycheck) == p.planned_amount)
    }

    // ------------------------------------------------------------------
    // Seed / structural helpers
    // ------------------------------------------------------------------

    /// Add any missing starter categories (idempotent, spec §2.4).
    pub fn seed_starter_categories(&mut self) {
        let mut order = self.categories.iter().map(|c| c.sort_order).max().unwrap_or(0) + 10;
        for name in STARTER_CATEGORIES {
            if !self.categories.iter().any(|c| c.name == *name) {
                self.categories.push(ExpenseCategory {
                    id: Id::generate(),
                    name: (*name).to_string(),
                    sort_order: order,
                });
                order += 10;
            }
        }
    }

    pub fn add_category(&mut self, name: &str) -> Result<&ExpenseCategory, DomainError> {
        self.require_draft()?;
        if name.trim().is_empty() {
            return Err(DomainError::EmptyName);
        }
        if self.categories.iter().any(|c| c.name == name) {
            return Err(DomainError::DuplicateCategory(name.to_string()));
        }
        let order = self.categories.iter().map(|c| c.sort_order).max().unwrap_or(0) + 10;
        let id = Id::generate();
        self.categories
            .push(ExpenseCategory { id: id.clone(), name: name.to_string(), sort_order: order });
        self.category(&id).ok_or(DomainError::CategoryNotFound(id))
    }

    pub fn add_expense_line(
        &mut self,
        category: &Id,
        name: &str,
        current_balance: Option<Cents>,
        minimum_payment: Option<Cents>,
    ) -> Result<&ExpenseLine, DomainError> {
        self.require_draft()?;
        if name.trim().is_empty() {
            return Err(DomainError::EmptyName);
        }
        if self.category(category).is_none() {
            return Err(DomainError::CategoryNotFound(category.clone()));
        }
        let id = Id::generate();
        self.expense_lines.push(ExpenseLine {
            id: id.clone(),
            category_id: category.clone(),
            name: name.to_string(),
            current_balance,
            minimum_payment,
        });
        self.expense_line(&id).ok_or(DomainError::ExpenseLineNotFound(id))
    }

    pub fn add_income_line(
        &mut self,
        name: &str,
        planned_amount: Cents,
        schedule_type: ScheduleType,
        recurrence_rule: Option<String>,
    ) -> Result<&IncomeLine, DomainError> {
        self.require_draft()?;
        if name.trim().is_empty() {
            return Err(DomainError::EmptyName);
        }
        if planned_amount.is_negative() {
            return Err(DomainError::NegativeAmount);
        }
        if self.income_lines.iter().any(|l| l.name == name) {
            return Err(DomainError::DuplicateIncomeLine(name.to_string()));
        }
        let id = Id::generate();
        self.income_lines.push(IncomeLine {
            id: id.clone(),
            name: name.to_string(),
            planned_amount,
            schedule_type,
            recurrence_rule,
        });
        self.income_line(&id).ok_or(DomainError::IncomeLineNotFound(id))
    }

    pub fn add_paycheck(
        &mut self,
        income_line: &Id,
        date: NaiveDate,
        planned_amount: Cents,
    ) -> Result<&Paycheck, DomainError> {
        self.require_draft()?;
        if planned_amount.is_negative() {
            return Err(DomainError::NegativeAmount);
        }
        if self.income_line(income_line).is_none() {
            return Err(DomainError::IncomeLineNotFound(income_line.clone()));
        }
        if self.paychecks
            .iter()
            .any(|p| &p.income_line_id == income_line && p.date == date)
        {
            return Err(DomainError::DuplicatePaycheck);
        }
        let id = Id::generate();
        self.paychecks.push(Paycheck {
            id: id.clone(),
            income_line_id: income_line.clone(),
            date,
            planned_amount,
            actual_amount: None,
            status: PaycheckStatus::Planned,
        });
        self.paycheck(&id).ok_or(DomainError::PaycheckNotFound(id))
    }

    pub fn delete_category(&mut self, id: &Id) -> Result<(), DomainError> {
        self.require_draft()?;
        if self.category(id).is_none() {
            return Err(DomainError::CategoryNotFound(id.clone()));
        }
        let line_ids: Vec<Id> = self
            .expense_lines
            .iter()
            .filter(|l| &l.category_id == id)
            .map(|l| l.id.clone())
            .collect();
        for line in &line_ids {
            self.remove_allocations_for_line(line);
        }
        self.expense_lines.retain(|l| &l.category_id != id);
        self.categories.retain(|c| &c.id != id);
        Ok(())
    }

    pub fn delete_expense_line(&mut self, id: &Id) -> Result<(), DomainError> {
        self.require_draft()?;
        if self.expense_line(id).is_none() {
            return Err(DomainError::ExpenseLineNotFound(id.clone()));
        }
        self.remove_allocations_for_line(id);
        self.expense_lines.retain(|l| &l.id != id);
        // Mirrors DB `ON DELETE SET NULL` for transactions.
        for t in self.transactions.iter_mut() {
            if t.expense_line_id.as_ref() == Some(id) {
                t.expense_line_id = None;
            }
        }
        Ok(())
    }

    pub fn delete_income_line(&mut self, id: &Id) -> Result<(), DomainError> {
        self.require_draft()?;
        if self.income_line(id).is_none() {
            return Err(DomainError::IncomeLineNotFound(id.clone()));
        }
        let pc_ids: Vec<Id> = self
            .paychecks
            .iter()
            .filter(|p| &p.income_line_id == id)
            .map(|p| p.id.clone())
            .collect();
        for pc in &pc_ids {
            self.remove_allocations_for(pc);
        }
        self.paychecks.retain(|p| &p.income_line_id != id);
        self.income_lines.retain(|l| &l.id != id);
        Ok(())
    }

    // ------------------------------------------------------------------
    // Allocations (the critical surface, spec §2.6)
    // ------------------------------------------------------------------

    /// Create or update the unique `(paycheck, expense_line)` allocation.
    /// Rejects anything that would over-allocate the paycheck (invariant 2).
    pub fn allocate(&mut self, paycheck: &Id, expense_line: &Id, amount: Cents) -> Result<&Allocation, DomainError> {
        self.require_draft()?;
        if !amount.is_positive() {
            return Err(DomainError::NonPositiveAmount);
        }
        if self.paycheck(paycheck).is_none() {
            return Err(DomainError::PaycheckNotFound(paycheck.clone()));
        }
        if self.expense_line(expense_line).is_none() {
            return Err(DomainError::ExpenseLineNotFound(expense_line.clone()));
        }
        let sts = self.safe_to_spend(paycheck);
        if amount > sts {
            return Err(DomainError::OverAllocated {
                paycheck: paycheck.clone(),
                over: amount - sts,
            });
        }
        let idx = self.allocation_index_for(paycheck, expense_line);
        match idx {
            Some(i) => self.allocations[i].amount = amount,
            None => self.allocations.push(Allocation {
                id: Id::generate(),
                expense_line_id: expense_line.clone(),
                paycheck_id: paycheck.clone(),
                amount,
            }),
        }
        let final_idx = idx.unwrap_or_else(|| self.allocations.len() - 1);
        Ok(&self.allocations[final_idx])
    }

    pub fn update_allocation(&mut self, id: &Id, amount: Cents) -> Result<&Allocation, DomainError> {
        self.require_draft()?;
        if !amount.is_positive() {
            return Err(DomainError::NonPositiveAmount);
        }
        let idx = self
            .allocations
            .iter()
            .position(|a| &a.id == id)
            .ok_or(DomainError::AllocationNotFound(id.clone()))?;
        let pc = self.allocations[idx].paycheck_id.clone();
        let planned = self.paycheck(&pc).map(|p| p.planned_amount).unwrap_or(Cents::ZERO);
        let old = self.allocations[idx].amount;
        let new_sum = self.paycheck_allocated(&pc) - old + amount;
        if new_sum > planned {
            return Err(DomainError::OverAllocated { paycheck: pc, over: new_sum - planned });
        }
        self.allocations[idx].amount = amount;
        Ok(&self.allocations[idx])
    }

    pub fn delete_allocation(&mut self, id: &Id) -> Result<(), DomainError> {
        self.require_draft()?;
        if !self.allocations.iter().any(|a| &a.id == id) {
            return Err(DomainError::AllocationNotFound(id.clone()));
        }
        self.allocations.retain(|a| &a.id != id);
        Ok(())
    }

    /// Move an amount of a line from one paycheck to another (spec §9 transfer).
    pub fn transfer(
        &mut self,
        from: &Id,
        to: &Id,
        expense_line: &Id,
        amount: Cents,
    ) -> Result<(), DomainError> {
        if !amount.is_positive() {
            return Err(DomainError::NonPositiveAmount);
        }
        let from_alloc = self
            .allocations
            .iter()
            .find(|a| &a.paycheck_id == from && &a.expense_line_id == expense_line)
            .map(|a| a.amount);
        let moved = from_alloc.unwrap_or(Cents::ZERO).min(amount);
        if moved.is_zero() {
            return Err(DomainError::NonPositiveAmount);
        }
        let remaining = from_alloc.unwrap() - moved;
        if remaining.is_zero() {
            self.allocation_index_for(from, expense_line)
                .map(|i| self.allocations.remove(i));
        } else {
            let idx = self.allocation_index_for(from, expense_line).unwrap();
            self.allocations[idx].amount = remaining;
        }
        // Add to destination (creates/updates), which re-validates the bound.
        self.allocate(to, expense_line, moved)?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Paycheck planned / skip / delete cascades (invariants 6 & 7)
    // ------------------------------------------------------------------

    /// Set a paycheck's planned amount. If reduced below current allocations,
    /// excess allocations are reduced/deleted in deterministic order and the
    /// affected lines are returned for UI notification (invariant 6).
    pub fn set_paycheck_planned(&mut self, paycheck: &Id, new_amount: Cents) -> Result<Vec<Id>, DomainError> {
        self.require_draft()?;
        if new_amount.is_negative() {
            return Err(DomainError::NegativeAmount);
        }
        let idx = self.paycheck_index(paycheck).ok_or(DomainError::PaycheckNotFound(paycheck.clone()))?;
        let current = self.paycheck_allocated(paycheck);
        let mut affected: Vec<Id> = Vec::new();
        if new_amount < current {
            let mut excess = current - new_amount;
            while excess.is_positive() {
                let Some(ai) = self
                    .allocations
                    .iter()
                    .position(|a| &a.paycheck_id == paycheck)
                else {
                    break;
                };
                let line_id = self.allocations[ai].expense_line_id.clone();
                let amt = self.allocations[ai].amount;
                if !affected.contains(&line_id) {
                    affected.push(line_id.clone());
                }
                let reduce = amt.min(excess);
                if reduce == amt {
                    self.allocations.remove(ai);
                } else {
                    self.allocations[ai].amount = amt - reduce;
                }
                excess -= reduce;
            }
        }
        self.paychecks[idx].planned_amount = new_amount;
        Ok(affected)
    }

    /// Mark a paycheck skipped and delete all its allocations (invariant 7).
    /// Returns the affected expense line ids.
    pub fn skip_paycheck(&mut self, paycheck: &Id) -> Result<Vec<Id>, DomainError> {
        self.require_draft()?;
        let idx = self.paycheck_index(paycheck).ok_or(DomainError::PaycheckNotFound(paycheck.clone()))?;
        self.paychecks[idx].status = PaycheckStatus::Skipped;
        Ok(self.remove_allocations_for(paycheck))
    }

    /// Delete a paycheck and all its allocations (invariant 7).
    /// Returns the affected expense line ids.
    pub fn delete_paycheck(&mut self, paycheck: &Id) -> Result<Vec<Id>, DomainError> {
        self.require_draft()?;
        if self.paycheck_index(paycheck).is_none() {
            return Err(DomainError::PaycheckNotFound(paycheck.clone()));
        }
        let affected = self.remove_allocations_for(paycheck);
        self.paychecks.retain(|p| &p.id != paycheck);
        Ok(affected)
    }

    /// Update a paycheck's recorded actual (allowed even while locked, spec §2.9).
    pub fn set_paycheck_actual(&mut self, paycheck: &Id, actual: Option<Cents>) -> Result<(), DomainError> {
        if actual.is_some_and(|a| a.is_negative()) {
            return Err(DomainError::NegativeAmount);
        }
        let idx = self
            .paycheck_index(paycheck)
            .ok_or(DomainError::PaycheckNotFound(paycheck.clone()))?;
        self.paychecks[idx].actual_amount = actual;
        if actual.is_some() && self.paychecks[idx].status == PaycheckStatus::Planned {
            self.paychecks[idx].status = PaycheckStatus::Received;
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Transactions (manual, spec §2.8) — allowed even while locked
    // ------------------------------------------------------------------

    pub fn add_transaction(&mut self, t: Transaction) -> &Transaction {
        self.transactions.push(t);
        let len = self.transactions.len();
        &self.transactions[len - 1]
    }

    pub fn update_transaction(&mut self, id: &Id, t: Transaction) -> Result<&Transaction, DomainError> {
        let idx = self
            .transactions
            .iter()
            .position(|x| &x.id == id)
            .ok_or(DomainError::TransactionNotFound(id.clone()))?;
        self.transactions[idx] = t;
        Ok(&self.transactions[idx])
    }

    pub fn delete_transaction(&mut self, id: &Id) -> Result<(), DomainError> {
        if !self.transactions.iter().any(|x| &x.id == id) {
            return Err(DomainError::TransactionNotFound(id.clone()));
        }
        self.transactions.retain(|x| &x.id != id);
        Ok(())
    }

    // ------------------------------------------------------------------
    // Locking (spec §2.1, invariant 4)
    // ------------------------------------------------------------------

    pub fn lock(&mut self) -> Result<(), DomainError> {
        if !self.is_zero() {
            return Err(DomainError::NotZero { diff: self.zero_difference() });
        }
        self.status = MonthStatus::Locked;
        Ok(())
    }

    /// Re-open a locked month for variance re-assignment (spec §2.9).
    pub fn unlock(&mut self) {
        self.status = MonthStatus::Draft;
    }

    pub fn archive(&mut self) {
        self.archived = true;
    }

    pub fn restore(&mut self) {
        self.archived = false;
    }

    // ------------------------------------------------------------------
    // Derived views (spec §14, §15)
    // ------------------------------------------------------------------

    pub fn summary(&self, today: NaiveDate) -> MonthSummary {
        let mut cat_ids: Vec<&ExpenseCategory> = self.categories.iter().collect();
        cat_ids.sort_by_key(|c| c.sort_order);

        let mut categories = Vec::with_capacity(cat_ids.len());
        for c in cat_ids {
            let mut lines = Vec::new();
            let mut c_planned = Cents::ZERO;
            let mut c_spent = Cents::ZERO;
            for l in self.expense_lines.iter().filter(|l| l.category_id == c.id) {
                let planned = self.line_planned(&l.id);
                let spent = self.line_spent(&l.id);
                c_planned += planned;
                c_spent += spent;
                lines.push(LineView {
                    id: l.id.clone(),
                    category_id: c.id.clone(),
                    name: l.name.clone(),
                    planned,
                    spent,
                    remaining: planned - spent,
                    is_debt: l.current_balance.is_some() || l.minimum_payment.is_some(),
                    current_balance: l.current_balance,
                    minimum_payment: l.minimum_payment,
                });
            }
            categories.push(CategoryView {
                id: c.id.clone(),
                name: c.name.clone(),
                sort_order: c.sort_order,
                planned: c_planned,
                spent: c_spent,
                remaining: c_planned - c_spent,
                lines,
            });
        }

        let total_spent = Cents::from_cents(
            self.expense_lines
                .iter()
                .map(|l| self.line_spent(&l.id).as_cents())
                .sum(),
        );

        let paychecks: Vec<PaycheckView> = self
            .paychecks
            .iter()
            .map(|p| {
                let allocated = self.paycheck_allocated(&p.id);
                let iname = self
                    .income_line(&p.income_line_id)
                    .map(|l| l.name.clone())
                    .unwrap_or_default();
                PaycheckView {
                    id: p.id.clone(),
                    income_line_id: p.income_line_id.clone(),
                    income_line_name: iname,
                    date: p.date,
                    planned_amount: p.planned_amount,
                    actual_amount: p.actual_amount,
                    status: p.status,
                    allocated,
                    remaining_to_allocate: p.planned_amount - allocated,
                    is_fully_allocated: allocated == p.planned_amount,
                    safe_to_spend: self.safe_to_spend(&p.id),
                    variance: p.actual_amount.map(|a| a - p.planned_amount),
                }
            })
            .collect();

        MonthSummary {
            id: self.id.clone(),
            year_month: self.year_month,
            status: self.status,
            archived: self.archived,
            total_planned_income: self.total_planned_income(),
            total_planned_expense: self.total_planned_expense(),
            zero_difference: self.zero_difference(),
            is_zero: self.is_zero(),
            total_spent,
            rolling_available: self.rolling_available(today),
            paychecks,
            categories,
        }
    }

    // ------------------------------------------------------------------
    // Internals
    // ------------------------------------------------------------------

    fn require_draft(&self) -> Result<(), DomainError> {
        if self.is_locked() {
            Err(DomainError::Locked)
        } else {
            Ok(())
        }
    }

    fn remove_allocations_for(&mut self, paycheck: &Id) -> Vec<Id> {
        let mut affected: Vec<Id> = Vec::new();
        for a in self.allocations.iter().filter(|a| &a.paycheck_id == paycheck) {
            if !affected.contains(&a.expense_line_id) {
                affected.push(a.expense_line_id.clone());
            }
        }
        self.allocations.retain(|a| &a.paycheck_id != paycheck);
        affected
    }

    fn remove_allocations_for_line(&mut self, line: &Id) -> Vec<Id> {
        let mut affected: Vec<Id> = Vec::new();
        for a in self.allocations.iter().filter(|a| &a.expense_line_id == line) {
            if !affected.contains(&a.paycheck_id) {
                affected.push(a.paycheck_id.clone());
            }
        }
        self.allocations.retain(|a| &a.expense_line_id != line);
        affected
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Cents, Id};
    use chrono::NaiveDate;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }
    fn c(n: i64) -> Cents {
        Cents::from_cents(n)
    }

    struct Fix {
        m: Month,
        il: Id,
        pc: Id,
        cat: Id,
        line: Id,
    }

    /// Month with one income line, one paycheck (planned `planned`), one
    /// Housing category and one "Rent" expense line.
    fn fix(planned: i64) -> Fix {
        let mut m = Month::new(Id::generate(), d("2026-09-01"));
        m.seed_starter_categories();
        let il = m
            .add_income_line("Job", c(planned), ScheduleType::OneOff, None)
            .unwrap()
            .id
            .clone();
        let pc = m.add_paycheck(&il, d("2026-09-01"), c(planned)).unwrap().id.clone();
        let housing = m.categories.iter().find(|c| c.name == "Housing").unwrap().id.clone();
        let line = m.add_expense_line(&housing, "Rent", None, None).unwrap().id.clone();
        Fix { m, il, pc, cat: housing, line }
    }

    #[test]
    fn seed_seeds_all_starter_categories() {
        let mut m = Month::new(Id::generate(), d("2026-09-01"));
        m.seed_starter_categories();
        assert_eq!(m.categories.len(), STARTER_CATEGORIES.len());
        assert!(m.categories.iter().any(|c| c.name == "Debt"));
        // idempotent
        m.seed_starter_categories();
        assert_eq!(m.categories.len(), STARTER_CATEGORIES.len());
    }

    #[test]
    fn allocated_amount_is_derived() {
        let f = fix(1000);
        let mut m = f.m;
        m.allocate(&f.pc, &f.line, c(400)).unwrap();
        assert_eq!(m.line_planned(&f.line), c(400));
        assert_eq!(m.paycheck_allocated(&f.pc), c(400));
    }

    #[test]
    fn over_allocation_is_rejected() {
        let f = fix(1000);
        let mut m = f.m;
        m.allocate(&f.pc, &f.line, c(600)).unwrap();
        let err = m.allocate(&f.pc, &f.line, c(900)).unwrap_err();
        assert_eq!(
            err,
            DomainError::OverAllocated { paycheck: f.pc.clone(), over: c(500) }
        );
        // unchanged
        assert_eq!(m.line_planned(&f.line), c(600));
    }

    #[test]
    fn non_positive_amount_rejected() {
        let f = fix(1000);
        let mut m = f.m;
        assert!(matches!(m.allocate(&f.pc, &f.line, c(0)), Err(DomainError::NonPositiveAmount)));
        assert!(matches!(m.allocate(&f.pc, &f.line, c(-5)), Err(DomainError::NonPositiveAmount)));
    }

    #[test]
    fn fully_allocated_month_is_zero_and_locks() {
        let f = fix(1000);
        let mut m = f.m;
        m.allocate(&f.pc, &f.line, c(1000)).unwrap();
        assert!(m.is_zero());
        assert_eq!(m.zero_difference(), c(0));
        m.lock().unwrap();
        assert!(m.is_locked());
    }

    #[test]
    fn cannot_lock_when_not_zero() {
        let f = fix(1000);
        let mut m = f.m;
        m.allocate(&f.pc, &f.line, c(400)).unwrap();
        assert_eq!(m.zero_difference(), c(600));
        let err = m.lock().unwrap_err();
        assert_eq!(err, DomainError::NotZero { diff: c(600) });
        assert!(m.is_draft());
    }

    #[test]
    fn locked_month_blocks_edits_but_allows_actuals_and_txns() {
        let f = fix(1000);
        let mut m = f.m;
        m.allocate(&f.pc, &f.line, c(1000)).unwrap();
        m.lock().unwrap();
        assert!(matches!(m.allocate(&f.pc, &f.line, c(10)), Err(DomainError::Locked)));
        assert!(matches!(
            m.set_paycheck_planned(&f.pc, c(500)),
            Err(DomainError::Locked)
        ));
        // actuals + transactions still allowed
        m.set_paycheck_actual(&f.pc, Some(c(950))).unwrap();
        m.add_transaction(Transaction {
            id: Id::generate(),
            date: d("2026-09-02"),
            amount: c(-50),
            payee: None,
            notes: None,
            expense_line_id: Some(f.line.clone()),
            paycheck_id: None,
        });
        assert_eq!(m.line_spent(&f.line), c(50));
    }

    #[test]
    fn reduce_planned_cascades_allocations() {
        let f = fix(1000);
        let mut m = f.m;
        m.allocate(&f.pc, &f.line, c(1000)).unwrap();
        let affected = m.set_paycheck_planned(&f.pc, c(600)).unwrap();
        assert_eq!(m.line_planned(&f.line), c(600));
        assert_eq!(m.paycheck_allocated(&f.pc), c(600));
        assert_eq!(affected, vec![f.line.clone()]);
        // now not fully allocated -> not zero
        assert_eq!(m.zero_difference(), c(600) - c(600)); // still balanced because income dropped too
    }

    #[test]
    fn skip_paycheck_removes_allocations_and_income() {
        let f = fix(1000);
        let mut m = f.m;
        m.allocate(&f.pc, &f.line, c(1000)).unwrap();
        m.skip_paycheck(&f.pc).unwrap();
        assert_eq!(m.line_planned(&f.line), c(0));
        assert_eq!(m.paycheck_allocated(&f.pc), c(0));
        // skipped paycheck contributes no income
        assert_eq!(m.total_planned_income(), c(0));
        assert!(m.is_zero());
    }

    #[test]
    fn delete_paycheck_removes_allocations() {
        let f = fix(1000);
        let mut m = f.m;
        m.allocate(&f.pc, &f.line, c(1000)).unwrap();
        m.delete_paycheck(&f.pc).unwrap();
        assert!(m.paycheck(&f.pc).is_none());
        assert_eq!(m.line_planned(&f.line), c(0));
        assert_eq!(m.paychecks.len(), 0);
    }

    #[test]
    fn splitting_a_line_across_paychecks() {
        let f = fix(0);
        let mut m = f.m;
        // two paychecks of 500 each
        let pc2 = m.add_paycheck(&f.il, d("2026-09-15"), c(500)).unwrap().id.clone();
        // first paycheck already has planned 0; set it to 500
        m.set_paycheck_planned(&f.pc, c(500)).unwrap();
        m.allocate(&f.pc, &f.line, c(300)).unwrap();
        m.allocate(&pc2, &f.line, c(200)).unwrap();
        assert_eq!(m.line_planned(&f.line), c(500));
        assert_eq!(m.line_planned(&f.line), c(300) + c(200));
    }

    #[test]
    fn safe_to_spend_accounts_for_allocations_and_tagged_txns() {
        let f = fix(1000);
        let mut m = f.m;
        m.allocate(&f.pc, &f.line, c(400)).unwrap();
        assert_eq!(m.safe_to_spend(&f.pc), c(600));
        // expense txn tagged to this paycheck
        m.add_transaction(Transaction {
            id: Id::generate(),
            date: d("2026-09-02"),
            amount: c(-150),
            payee: Some("Store".into()),
            notes: None,
            expense_line_id: Some(f.line.clone()),
            paycheck_id: Some(f.pc.clone()),
        });
        assert_eq!(m.safe_to_spend(&f.pc), c(450));
    }

    #[test]
    fn txn_without_paycheck_tag_does_not_affect_safe_to_spend() {
        let f = fix(1000);
        let mut m = f.m;
        m.allocate(&f.pc, &f.line, c(400)).unwrap();
        m.add_transaction(Transaction {
            id: Id::generate(),
            date: d("2026-09-02"),
            amount: c(-150),
            payee: None,
            notes: None,
            expense_line_id: Some(f.line.clone()),
            paycheck_id: None,
        });
        assert_eq!(m.safe_to_spend(&f.pc), c(600));
        // but line spent does increase
        assert_eq!(m.line_spent(&f.line), c(150));
    }

    #[test]
    fn rolling_available_only_future_or_today() {
        let mut m = fix(1000).m;
        let il = m.income_lines[0].id.clone();
        let past = m.add_paycheck(&il, d("2026-08-01"), c(300)).unwrap().id.clone();
        let line = fix(0).line;
        // past paycheck fully allocated so it's self-consistent
        let _ = m.allocate(&past, &line, c(300));
        // today = 2026-09-01; pc (09-01) counts, past (08-01) does not
        assert_eq!(m.rolling_available(d("2026-09-01")), c(1000));
    }

    #[test]
    fn duplicate_paycheck_rejected() {
        let f = fix(1000);
        let mut m = f.m;
        assert!(matches!(
            m.add_paycheck(&f.il, d("2026-09-01"), c(100)),
            Err(DomainError::DuplicatePaycheck)
        ));
    }

    #[test]
    fn delete_line_nulls_transaction_link() {
        let f = fix(1000);
        let mut m = f.m;
        m.add_transaction(Transaction {
            id: Id::generate(),
            date: d("2026-09-02"),
            amount: c(-100),
            payee: None,
            notes: None,
            expense_line_id: Some(f.line.clone()),
            paycheck_id: None,
        });
        m.delete_expense_line(&f.line).unwrap();
        assert!(m.expense_line(&f.line).is_none());
        assert!(m.transactions.iter().all(|t| t.expense_line_id.is_none()));
    }

    #[test]
    fn transfer_moves_money_between_paychecks() {
        let f = fix(0);
        let mut m = f.m;
        let pc2 = m.add_paycheck(&f.il, d("2026-09-15"), c(1000)).unwrap().id.clone();
        m.set_paycheck_planned(&f.pc, c(1000)).unwrap();
        m.allocate(&f.pc, &f.line, c(500)).unwrap();
        m.transfer(&f.pc, &pc2, &f.line, c(200)).unwrap();
        assert_eq!(m.line_planned(&f.line), c(500));
        let from: i64 = m.allocations.iter().filter(|a| a.paycheck_id == f.pc).map(|a| a.amount.as_cents()).sum();
        let to: i64 = m.allocations.iter().filter(|a| a.paycheck_id == pc2).map(|a| a.amount.as_cents()).sum();
        assert_eq!(from, 300);
        assert_eq!(to, 200);
    }
}
