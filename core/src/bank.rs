//! Importing transactions from a bank connection into a month.
//!
//! The rules aim for "it just sorts itself out":
//! - a bank transaction is imported once (by its bank id);
//! - one you already typed in by hand (same amount, account unset or the
//!   same, within three days) is linked instead of duplicated;
//! - spending gets the line the same payee had last time, and is tagged to
//!   the paycheck that was current on its date;
//! - a deposit close to a paycheck's date and amount is tagged to that
//!   paycheck (which records what it actually paid);
//! - money leaving one connected account and arriving in another becomes a
//!   single transfer (e.g. a card payment seen from both sides).

use crate::id::Id;
use crate::models::{PaycheckStatus, Transaction};
use crate::money::Cents;
use crate::month::Month;
use chrono::NaiveDate;

/// How close (in days) two sightings of the same money must be.
pub const MATCH_DAYS: i64 = 3;

/// A posted transaction from the bank, already mapped to a local account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BankTx {
    pub external_id: String,
    pub account: Id,
    pub date: NaiveDate,
    /// Signed: negative is money leaving the account.
    pub amount: Cents,
    pub payee: Option<String>,
    /// Deposits into this account can be paychecks (checking, savings).
    pub depository: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportStats {
    pub added: usize,
    /// Linked to a transaction you had already entered.
    pub matched: usize,
    /// Given a line learned from an earlier transaction.
    pub categorized: usize,
    /// Pairs merged into one transfer.
    pub transfers: usize,
    /// Deposits recognised as a paycheck.
    pub paychecks: usize,
}

fn norm(s: &str) -> String {
    s.chars().filter(char::is_ascii_alphanumeric).collect::<String>().to_ascii_lowercase()
}

fn days_apart(a: NaiveDate, b: NaiveDate) -> i64 {
    (a - b).num_days().abs()
}

/// The line name the same payee was given most recently, in `m` first,
/// then in earlier months.
fn learned_line(m: &Month, history: &[Month], payee: &str) -> Option<String> {
    let key = norm(payee);
    if key.is_empty() {
        return None;
    }
    let find = |month: &Month| -> Option<String> {
        let mut txs: Vec<&Transaction> = month
            .transactions
            .iter()
            .filter(|t| t.expense_line_id.is_some() && !t.is_transfer() && t.payee.as_deref().is_some_and(|p| norm(p) == key))
            .collect();
        txs.sort_by_key(|t| std::cmp::Reverse(t.date));
        txs.first().and_then(|t| t.expense_line_id.as_ref()).and_then(|l| month.expense_line(l)).map(|l| l.name.clone())
    };
    if let Some(n) = find(m) {
        return Some(n);
    }
    let mut older: Vec<&Month> = history.iter().filter(|h| h.year_month < m.year_month).collect();
    older.sort_by_key(|h| std::cmp::Reverse(h.year_month));
    older.into_iter().find_map(find)
}

/// The latest paycheck on or before `date` (spending belongs to it).
fn current_paycheck(m: &Month, date: NaiveDate) -> Option<Id> {
    m.paychecks_by_date().into_iter().rev().find(|p| p.status != PaycheckStatus::Skipped && p.date <= date).map(|p| p.id.clone())
}

/// A paycheck this deposit looks like: within three days and 25% of the
/// planned amount, with no deposit recorded yet.
fn paycheck_for_deposit(m: &Month, date: NaiveDate, amount: Cents) -> Option<Id> {
    m.paychecks_by_date()
        .into_iter()
        .filter(|p| p.status != PaycheckStatus::Skipped && days_apart(p.date, date) <= MATCH_DAYS)
        .filter(|p| {
            let diff = (amount - p.planned_amount).abs().get();
            p.planned_amount.is_positive() && diff.saturating_mul(4) <= p.planned_amount.get()
        })
        .find(|p| m.paycheck_deposits(&p.id).is_zero())
        .map(|p| p.id.clone())
}

/// Imports `txs` into `m`. `history` is every month (for learning lines);
/// `linked` are the accounts that sync (for pairing transfers).
pub fn import_into_month(m: &mut Month, txs: Vec<BankTx>, history: &[Month], linked: &[Id]) -> ImportStats {
    let mut stats = ImportStats::default();
    for b in txs {
        if b.amount.is_zero() || m.transactions.iter().any(|t| t.external_id.as_deref() == Some(b.external_id.as_str())) {
            continue;
        }
        // Already typed in by hand?
        let manual = m
            .transactions
            .iter()
            .filter(|t| {
                t.external_id.is_none()
                    && t.split_group.is_none()
                    && !t.is_transfer()
                    && t.amount == b.amount
                    && t.account_id.as_ref().is_none_or(|a| a == &b.account)
                    && days_apart(t.date, b.date) <= MATCH_DAYS
            })
            .min_by_key(|t| days_apart(t.date, b.date))
            .map(|t| t.id.clone());
        if let Some(id) = manual {
            if let Some(t) = m.transactions.iter_mut().find(|t| t.id == id) {
                t.external_id = Some(b.external_id.clone());
                t.account_id = Some(b.account.clone());
            }
            stats.matched += 1;
            continue;
        }
        let payee = b.payee.as_deref().map(str::trim).filter(|p| !p.is_empty()).map(str::to_string);
        let line = if b.amount.is_negative() {
            payee
                .as_deref()
                .and_then(|p| learned_line(m, history, p))
                .and_then(|name| m.expense_lines.iter().find(|l| l.name.eq_ignore_ascii_case(&name)).map(|l| l.id.clone()))
        } else {
            None
        };
        let paycheck = if b.amount.is_negative() {
            current_paycheck(m, b.date)
        } else if b.depository {
            paycheck_for_deposit(m, b.date, b.amount)
        } else {
            None
        };
        let tx = Transaction {
            id: Id::generate(),
            date: b.date,
            amount: b.amount,
            payee,
            notes: None,
            expense_line_id: line.clone(),
            paycheck_id: paycheck.clone(),
            split_group: None,
            account_id: Some(b.account.clone()),
            transfer_account_id: None,
            external_id: Some(b.external_id.clone()),
        };
        if m.add_transaction(tx).is_ok() {
            stats.added += 1;
            if line.is_some() {
                stats.categorized += 1;
            }
            if paycheck.is_some() && b.amount.is_positive() {
                stats.paychecks += 1;
            }
        }
    }
    stats.transfers = pair_transfers(m, linked);
    stats
}

/// Merges "out of one connected account" + "into another" into a transfer.
fn pair_transfers(m: &mut Month, linked: &[Id]) -> usize {
    let is_candidate = |t: &Transaction| {
        t.external_id.is_some()
            && t.split_group.is_none()
            && !t.is_transfer()
            && t.account_id.as_ref().is_some_and(|a| linked.contains(a))
    };
    let mut merged = 0;
    loop {
        let pair = m.transactions.iter().filter(|t| is_candidate(t) && t.amount.is_negative()).find_map(|out| {
            m.transactions
                .iter()
                .filter(|inn| {
                    is_candidate(inn)
                        && inn.amount == -out.amount
                        && inn.account_id != out.account_id
                        && inn.paycheck_id.is_none()
                        && inn.expense_line_id.is_none()
                        && days_apart(inn.date, out.date) <= MATCH_DAYS
                })
                .min_by_key(|inn| days_apart(inn.date, out.date))
                .map(|inn| (out.clone(), inn.id.clone(), inn.account_id.clone()))
        });
        let Some((mut out, inn_id, to)) = pair else { break };
        if m.delete_transaction(&inn_id).is_err() {
            break;
        }
        out.transfer_account_id = to;
        out.expense_line_id = None;
        out.paycheck_id = None;
        if m.update_transaction(out).is_err() {
            break;
        }
        merged += 1;
    }
    merged
}
