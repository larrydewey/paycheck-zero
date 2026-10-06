//! Accounts, credit cards and goals: the parts of a budget that carry on
//! from month to month.
//!
//! Balances are never typed over. An account's balance is the sum of its
//! adjustments (the opening balance and any reconciliations) plus every
//! transaction on it, across all months. Reconciling records the gap between
//! what the bank says and what PaycheckZero knows as an adjustment, so the
//! history always adds up.

use crate::error::DomainError;
use crate::id::Id;
use crate::models::Transaction;
use crate::money::Cents;
use crate::month::Month;
use chrono::{Datelike, Months, NaiveDate};
use serde::{Deserialize, Serialize};

pub const MAX_ACCOUNT_NAME: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountKind {
    Checking,
    Savings,
    Cash,
    CreditCard,
    /// 401(k), 403(b), IRA, HSA investments…
    Retirement,
    /// A brokerage account.
    Investment,
}

/// How accounts are grouped on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AccountGroup {
    Cash,
    Invested,
    Card,
}

impl AccountKind {
    pub const ALL: [AccountKind; 6] = [
        AccountKind::Checking,
        AccountKind::Savings,
        AccountKind::Cash,
        AccountKind::CreditCard,
        AccountKind::Retirement,
        AccountKind::Investment,
    ];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            AccountKind::Checking => "checking",
            AccountKind::Savings => "savings",
            AccountKind::Cash => "cash",
            AccountKind::CreditCard => "credit_card",
            AccountKind::Retirement => "retirement",
            AccountKind::Investment => "investment",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == s)
    }

    #[must_use]
    pub fn is_card(self) -> bool {
        self == AccountKind::CreditCard
    }

    /// Retirement and investment accounts: their value moves with the
    /// market, and they aren't used for everyday spending.
    #[must_use]
    pub fn is_invested(self) -> bool {
        matches!(self, AccountKind::Retirement | AccountKind::Investment)
    }

    /// Can pay for things directly (bank, cash, cards).
    #[must_use]
    pub fn is_spendable(self) -> bool {
        !self.is_invested()
    }

    #[must_use]
    pub fn group(self) -> AccountGroup {
        if self.is_card() {
            AccountGroup::Card
        } else if self.is_invested() {
            AccountGroup::Invested
        } else {
            AccountGroup::Cash
        }
    }
}

/// A bank account, cash envelope or credit card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub id: Id,
    pub name: String,
    pub kind: AccountKind,
    pub sort_order: i32,
    pub archived: bool,
    /// Credit cards only.
    pub credit_limit: Option<Cents>,
    /// Credit cards only: APR in basis points (19.99% = 1999).
    pub apr_bp: Option<i64>,
    /// Credit cards only.
    pub minimum_payment: Option<Cents>,
    /// The last day the balance was checked against the bank.
    pub reconciled_on: Option<NaiveDate>,
    /// Bank sync: the bank's id for this account and the connection it
    /// comes through.
    #[serde(default)]
    pub external_id: Option<String>,
    #[serde(default)]
    pub link_id: Option<Id>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdjustmentKind {
    /// The balance when the account was added.
    Opening,
    /// A correction made while reconciling with the bank.
    Reconcile,
    /// Money added straight from payroll (e.g. a pre-tax 401(k)
    /// contribution): it never passed through the budget.
    Contribution,
    /// Market gains or losses on an investment account.
    Growth,
}

impl AdjustmentKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            AdjustmentKind::Opening => "opening",
            AdjustmentKind::Reconcile => "reconcile",
            AdjustmentKind::Contribution => "contribution",
            AdjustmentKind::Growth => "growth",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "opening" => Some(AdjustmentKind::Opening),
            "reconcile" => Some(AdjustmentKind::Reconcile),
            "contribution" => Some(AdjustmentKind::Contribution),
            "growth" => Some(AdjustmentKind::Growth),
            _ => None,
        }
    }
}

/// A change to an account's balance that isn't a transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Adjustment {
    pub id: Id,
    pub account_id: Id,
    pub date: NaiveDate,
    /// Signed: for a credit card, negative means more owed.
    pub amount: Cents,
    pub kind: AdjustmentKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalKind {
    /// Build up to a target amount.
    Save,
    /// Bring a debt down to zero.
    Payoff,
}

impl GoalKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            GoalKind::Save => "save",
            GoalKind::Payoff => "payoff",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "save" => Some(GoalKind::Save),
            "payoff" => Some(GoalKind::Payoff),
            _ => None,
        }
    }
}

/// What a goal's progress is measured from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "track", rename_all = "snake_case")]
pub enum GoalTrack {
    /// A budget line, matched by name in every month (lines are copied from
    /// month to month). Saving: what's planned on it. Payoff: the balance on
    /// that debt line.
    Line { name: String },
    /// An account balance. Saving: the balance. Payoff: what's owed on it.
    Account { id: Id },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Goal {
    pub id: Id,
    pub name: String,
    pub kind: GoalKind,
    /// Saving: the amount to reach. Payoff: the debt when the goal started.
    pub target_amount: Cents,
    /// First day of the month the goal should be reached by.
    pub target_month: Option<NaiveDate>,
    pub track: GoalTrack,
    /// First day of the month progress is counted from.
    pub start_month: NaiveDate,
    /// Saving tracked by a line: what was already saved before the goal.
    pub starting_amount: Cents,
    pub sort_order: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    Done,
    OnTrack,
    Behind,
    /// No target date, so no pace to keep.
    NoDate,
}

impl GoalStatus {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            GoalStatus::Done => "done",
            GoalStatus::OnTrack => "on_track",
            GoalStatus::Behind => "behind",
            GoalStatus::NoDate => "no_date",
        }
    }
}

/// One month of a goal's history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalMonth {
    pub month: NaiveDate,
    /// Progress at the end of the month (saved so far, or debt paid down).
    pub value: Cents,
    /// Change during the month.
    pub change: Cents,
}

/// Where a goal stands as of a month.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalProgress {
    /// Saved so far, or debt paid down so far.
    pub current: Cents,
    /// Saving: the target. Payoff: the starting debt.
    pub target: Cents,
    /// Left to save, or debt still owed.
    pub remaining: Cents,
    pub percent: i64,
    /// Progress made this month (planned for line goals).
    pub this_month: Cents,
    /// What this month needs to stay on pace for the target date.
    pub needed_this_month: Option<Cents>,
    /// Months left including this one.
    pub months_left: Option<i64>,
    pub status: GoalStatus,
    /// Oldest first, up to and including this month.
    pub history: Vec<GoalMonth>,
}

/// A credit card as it stands for a month.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardSummary {
    /// What's owed now (never negative; a credit balance shows as zero).
    pub owed: Cents,
    /// Card purchases this month that were on budget lines: the money for
    /// them is already set aside in those lines.
    pub budgeted_spending: Cents,
    /// Card purchases this month with no line yet.
    pub unbudgeted_spending: Cents,
    /// Payments to the card this month.
    pub paid: Cents,
    /// Budgeted spending not paid off yet: pay this to keep the card at
    /// what you carried in.
    pub ready_to_pay: Cents,
    /// Debt beyond this month's budgeted spending (carried in, or spent
    /// without a line). Plan a payment for it on a debt line.
    pub carried: Cents,
    /// Percent of the credit limit in use.
    pub utilization: Option<i64>,
}

/// A retirement or investment account for a calendar year.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestedSummary {
    pub balance: Cents,
    /// Payroll contributions plus transfers in, this year.
    pub contributed: Cents,
    /// Transfers out (withdrawals), this year.
    pub withdrawn: Cents,
    /// Market gains (or losses, negative) this year.
    pub growth: Cents,
}

/// One line of an account's history: a transaction or an adjustment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Activity<'a> {
    Tx(&'a Transaction),
    Adjustment(&'a Adjustment),
}

impl Activity<'_> {
    #[must_use]
    pub fn date(&self) -> NaiveDate {
        match self {
            Activity::Tx(t) => t.date,
            Activity::Adjustment(a) => a.date,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkStatus {
    Active,
    /// The bank needs you to sign in again.
    NeedsReconnect,
    /// The last sync failed for another reason (see `last_error`).
    Error,
}

impl LinkStatus {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            LinkStatus::Active => "active",
            LinkStatus::NeedsReconnect => "needs_reconnect",
            LinkStatus::Error => "error",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "active" => Some(LinkStatus::Active),
            "needs_reconnect" => Some(LinkStatus::NeedsReconnect),
            "error" => Some(LinkStatus::Error),
            _ => None,
        }
    }
}

/// A connection to a bank through a data provider (SimpleFIN or Plaid).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BankLink {
    pub id: Id,
    pub provider: String,
    pub enrollment_id: String,
    pub institution: String,
    /// The provider's access token, encrypted by the web layer.
    pub access_token: String,
    pub status: LinkStatus,
    pub last_error: Option<String>,
    /// Transactions dated before this aren't imported.
    pub import_from: NaiveDate,
    /// RFC 3339 time of the last successful sync.
    pub last_sync: Option<String>,
    /// Provider paging state (Plaid's transactions cursor).
    #[serde(default)]
    pub cursor: Option<String>,
}

/// Everything that spans months: accounts, their adjustments, goals and
/// bank connections.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Wallet {
    pub accounts: Vec<Account>,
    pub adjustments: Vec<Adjustment>,
    pub goals: Vec<Goal>,
    #[serde(default)]
    pub links: Vec<BankLink>,
}

fn clean_name(name: &str) -> Result<String, DomainError> {
    let n = name.trim();
    if n.is_empty() || n.chars().count() > MAX_ACCOUNT_NAME {
        return Err(DomainError::InvalidName { max: MAX_ACCOUNT_NAME });
    }
    Ok(n.to_string())
}

fn first_of(d: NaiveDate) -> NaiveDate {
    d.with_day(1).unwrap_or(d)
}

fn month_end(ym: NaiveDate) -> NaiveDate {
    first_of(ym).checked_add_months(Months::new(1)).and_then(|d| d.pred_opt()).unwrap_or(ym)
}

/// Months from `a` to `b`, counting both (0 when `b` is before `a`).
fn months_between_inclusive(a: NaiveDate, b: NaiveDate) -> i64 {
    let n = i64::from(b.year() - a.year()) * 12 + i64::from(b.month()) - i64::from(a.month()) + 1;
    n.max(0)
}

fn all_transactions(months: &[Month]) -> impl Iterator<Item = &Transaction> {
    months.iter().flat_map(|m| m.transactions.iter())
}

/// A transaction's effect on an account's balance.
fn effect(t: &Transaction, account: &Id) -> Cents {
    let mut v = Cents::ZERO;
    if t.account_id.as_ref() == Some(account) {
        v += t.amount;
    }
    if t.transfer_account_id.as_ref() == Some(account) {
        v -= t.amount;
    }
    v
}

impl Wallet {
    #[must_use]
    pub fn account(&self, id: &Id) -> Option<&Account> {
        self.accounts.iter().find(|a| &a.id == id)
    }

    fn account_mut(&mut self, id: &Id) -> Result<&mut Account, DomainError> {
        self.accounts.iter_mut().find(|a| &a.id == id).ok_or_else(|| DomainError::not_found("account", id))
    }

    /// Open accounts in display order: bank and cash, then retirement and
    /// investments, then cards.
    #[must_use]
    pub fn accounts_sorted(&self) -> Vec<&Account> {
        let mut v: Vec<&Account> = self.accounts.iter().filter(|a| !a.archived).collect();
        v.sort_by_key(|a| (a.kind.group(), a.sort_order, a.name.to_lowercase()));
        v
    }

    #[must_use]
    pub fn link(&self, id: &Id) -> Option<&BankLink> {
        self.links.iter().find(|l| &l.id == id)
    }

    pub fn link_mut(&mut self, id: &Id) -> Result<&mut BankLink, DomainError> {
        self.links.iter_mut().find(|l| &l.id == id).ok_or_else(|| DomainError::not_found("bank connection", id))
    }

    /// Accounts that sync through a connection.
    #[must_use]
    pub fn linked_accounts(&self, link: &Id) -> Vec<&Account> {
        self.accounts.iter().filter(|a| a.link_id.as_ref() == Some(link) && !a.archived).collect()
    }

    /// Disconnects a bank. Its accounts stay (with their history); they
    /// just stop syncing.
    pub fn remove_link(&mut self, id: &Id) -> Result<(), DomainError> {
        self.link_mut(id)?;
        self.links.retain(|l| &l.id != id);
        for a in &mut self.accounts {
            if a.link_id.as_ref() == Some(id) {
                a.link_id = None;
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn goal(&self, id: &Id) -> Option<&Goal> {
        self.goals.iter().find(|g| &g.id == id)
    }

    #[must_use]
    pub fn goals_sorted(&self) -> Vec<&Goal> {
        let mut v: Vec<&Goal> = self.goals.iter().collect();
        v.sort_by_key(|g| (g.sort_order, g.name.to_lowercase()));
        v
    }

    /// Adds an account. `balance` is what's in it now; for a credit card,
    /// pass what's owed as a positive amount.
    pub fn add_account(&mut self, name: &str, kind: AccountKind, balance: Cents, today: NaiveDate, credit_limit: Option<Cents>) -> Result<Id, DomainError> {
        let name = clean_name(name)?;
        if credit_limit.is_some_and(|l| l.is_negative()) {
            return Err(DomainError::NegativeAmount);
        }
        let id = Id::generate();
        let sort_order = self.accounts.iter().map(|a| a.sort_order).max().unwrap_or(0) + 1;
        self.accounts.push(Account {
            id: id.clone(),
            name,
            kind,
            sort_order,
            archived: false,
            credit_limit: if kind.is_card() { credit_limit } else { None },
            apr_bp: None,
            minimum_payment: None,
            reconciled_on: Some(today),
            external_id: None,
            link_id: None,
        });
        let signed = if kind.is_card() { -balance } else { balance };
        if !signed.is_zero() {
            self.adjustments.push(Adjustment { id: Id::generate(), account_id: id.clone(), date: today, amount: signed, kind: AdjustmentKind::Opening });
        }
        Ok(id)
    }

    pub fn rename_account(&mut self, id: &Id, name: &str) -> Result<(), DomainError> {
        let name = clean_name(name)?;
        self.account_mut(id)?.name = name;
        Ok(())
    }

    /// Credit card details (ignored for other kinds).
    pub fn set_card_details(&mut self, id: &Id, credit_limit: Option<Cents>, apr_bp: Option<i64>, minimum_payment: Option<Cents>) -> Result<(), DomainError> {
        if credit_limit.is_some_and(|c| c.is_negative()) || minimum_payment.is_some_and(|c| c.is_negative()) || apr_bp.is_some_and(|a| a < 0) {
            return Err(DomainError::NegativeAmount);
        }
        let a = self.account_mut(id)?;
        if a.kind.is_card() {
            a.credit_limit = credit_limit;
            a.apr_bp = apr_bp;
            a.minimum_payment = minimum_payment;
        }
        Ok(())
    }

    pub fn set_archived(&mut self, id: &Id, archived: bool) -> Result<(), DomainError> {
        self.account_mut(id)?.archived = archived;
        Ok(())
    }

    pub fn move_account(&mut self, id: &Id, up: bool) -> Result<(), DomainError> {
        let group = self.account(id).ok_or_else(|| DomainError::not_found("account", id))?.kind.group();
        let order: Vec<Id> = self.accounts_sorted().into_iter().filter(|a| a.kind.group() == group).map(|a| a.id.clone()).collect();
        let pos = order.iter().position(|x| x == id).unwrap_or(0);
        let other = if up { pos.checked_sub(1) } else { Some(pos + 1).filter(|p| *p < order.len()) };
        if let Some(o) = other {
            // Renumber so ties can't make a swap a no-op.
            let mut order = order;
            order.swap(pos, o);
            for (i, aid) in order.iter().enumerate() {
                self.account_mut(aid)?.sort_order = i32::try_from(i).unwrap_or(i32::MAX);
            }
        }
        Ok(())
    }

    /// Removes an account, its adjustments and the goals that track it.
    /// Callers must make sure no transaction uses it (or unlink them).
    pub fn delete_account(&mut self, id: &Id) -> Result<(), DomainError> {
        if self.account(id).is_none() {
            return Err(DomainError::not_found("account", id));
        }
        self.accounts.retain(|a| &a.id != id);
        self.adjustments.retain(|a| &a.account_id != id);
        self.goals.retain(|g| g.track != GoalTrack::Account { id: id.clone() });
        Ok(())
    }

    /// Current balance, signed (negative for a card means money owed).
    #[must_use]
    pub fn balance(&self, id: &Id, months: &[Month]) -> Cents {
        let adj: Cents = self.adjustments.iter().filter(|a| &a.account_id == id).map(|a| a.amount).sum();
        adj + all_transactions(months).map(|t| effect(t, id)).sum::<Cents>()
    }

    /// Balance at the end of `date`.
    #[must_use]
    pub fn balance_on(&self, id: &Id, months: &[Month], date: NaiveDate) -> Cents {
        let adj: Cents = self.adjustments.iter().filter(|a| &a.account_id == id && a.date <= date).map(|a| a.amount).sum();
        adj + all_transactions(months).filter(|t| t.date <= date).map(|t| effect(t, id)).sum::<Cents>()
    }

    /// What's owed on a card now (a credit balance counts as zero).
    #[must_use]
    pub fn owed(&self, id: &Id, months: &[Month]) -> Cents {
        (-self.balance(id, months)).max(Cents::ZERO)
    }

    /// Records that the bank shows `actual`. For a card, `actual` is what's
    /// owed as a positive amount. Returns the adjustment that was needed
    /// (zero when everything already matched).
    pub fn reconcile(&mut self, id: &Id, actual: Cents, months: &[Month], today: NaiveDate) -> Result<Cents, DomainError> {
        let kind = self.account(id).ok_or_else(|| DomainError::not_found("account", id))?.kind;
        let target = if kind.is_card() { -actual } else { actual };
        let delta = target - self.balance(id, months);
        if !delta.is_zero() {
            // For investments the difference is the market, not a mistake.
            let kind = if kind.is_invested() { AdjustmentKind::Growth } else { AdjustmentKind::Reconcile };
            self.adjustments.push(Adjustment { id: Id::generate(), account_id: id.clone(), date: today, amount: delta, kind });
        }
        self.account_mut(id)?.reconciled_on = Some(today);
        Ok(delta)
    }

    /// Records money added to a retirement or investment account straight
    /// from payroll (it never reached the budget).
    pub fn add_contribution(&mut self, id: &Id, amount: Cents, date: NaiveDate) -> Result<(), DomainError> {
        let kind = self.account(id).ok_or_else(|| DomainError::not_found("account", id))?.kind;
        if !kind.is_invested() {
            return Err(DomainError::Invariant("contributions are for retirement and investment accounts".into()));
        }
        if !amount.is_positive() {
            return Err(DomainError::NonPositiveAmount);
        }
        self.adjustments.push(Adjustment { id: Id::generate(), account_id: id.clone(), date, amount, kind: AdjustmentKind::Contribution });
        Ok(())
    }

    /// Removes a contribution or growth entry (e.g. entered by mistake).
    pub fn delete_adjustment(&mut self, id: &Id) -> Result<(), DomainError> {
        let found = self.adjustments.iter().any(|a| &a.id == id && matches!(a.kind, AdjustmentKind::Contribution | AdjustmentKind::Growth));
        if !found {
            return Err(DomainError::not_found("adjustment", id));
        }
        self.adjustments.retain(|a| &a.id != id);
        Ok(())
    }

    /// A retirement or investment account for the calendar year of `year`.
    #[must_use]
    pub fn invested_summary(&self, id: &Id, months: &[Month], year: i32) -> InvestedSummary {
        let in_year = |d: NaiveDate| d.year() == year;
        let adj = |k: AdjustmentKind| -> Cents {
            self.adjustments.iter().filter(|a| &a.account_id == id && a.kind == k && in_year(a.date)).map(|a| a.amount).sum()
        };
        let transfers_in: Cents = all_transactions(months)
            .filter(|t| t.transfer_account_id.as_ref() == Some(id) && in_year(t.date))
            .map(|t| t.amount.abs())
            .sum();
        let transfers_out: Cents = all_transactions(months)
            .filter(|t| t.account_id.as_ref() == Some(id) && t.is_transfer() && in_year(t.date))
            .map(|t| t.amount.abs())
            .sum();
        InvestedSummary {
            balance: self.balance(id, months),
            contributed: adj(AdjustmentKind::Contribution) + transfers_in,
            withdrawn: transfers_out,
            growth: adj(AdjustmentKind::Growth),
        }
    }

    /// Everything that changed an account, newest first.
    #[must_use]
    pub fn account_activity<'a>(&'a self, id: &Id, months: &'a [Month]) -> Vec<Activity<'a>> {
        let mut v: Vec<Activity> = self.account_transactions(id, months).into_iter().map(Activity::Tx).collect();
        v.extend(self.adjustments.iter().filter(|a| &a.account_id == id).map(Activity::Adjustment));
        v.sort_by_key(|a| std::cmp::Reverse(a.date()));
        v
    }

    /// Transactions on an account (either side of a transfer), newest first.
    #[must_use]
    pub fn account_transactions<'a>(&self, id: &Id, months: &'a [Month]) -> Vec<&'a Transaction> {
        let mut v: Vec<&Transaction> = all_transactions(months)
            .filter(|t| t.account_id.as_ref() == Some(id) || t.transfer_account_id.as_ref() == Some(id))
            .collect();
        v.sort_by_key(|t| std::cmp::Reverse(t.date));
        v
    }

    #[must_use]
    pub fn is_used(&self, id: &Id, months: &[Month]) -> bool {
        all_transactions(months).any(|t| t.account_id.as_ref() == Some(id) || t.transfer_account_id.as_ref() == Some(id))
    }

    /// How a credit card stands for `month`.
    #[must_use]
    pub fn card_summary(&self, id: &Id, months: &[Month], month: &Month) -> CardSummary {
        let owed = self.owed(id, months);
        let on_card = || month.transactions.iter().filter(|t| t.account_id.as_ref() == Some(id) && !t.is_transfer() && t.amount.is_negative());
        let budgeted_spending: Cents = on_card().filter(|t| t.expense_line_id.is_some()).map(|t| t.amount.abs()).sum();
        let unbudgeted_spending: Cents = on_card().filter(|t| t.expense_line_id.is_none()).map(|t| t.amount.abs()).sum();
        let refunds: Cents = month.transactions.iter().filter(|t| t.account_id.as_ref() == Some(id) && !t.is_transfer() && t.amount.is_positive()).map(|t| t.amount).sum();
        let paid: Cents = month.transactions.iter().filter(|t| t.transfer_account_id.as_ref() == Some(id)).map(|t| t.amount.abs()).sum();
        // Payments cover this month's budgeted spending first; payments made
        // from a debt line are paying down older debt instead.
        let paid_for_spending: Cents = month
            .transactions
            .iter()
            .filter(|t| t.transfer_account_id.as_ref() == Some(id) && t.expense_line_id.is_none())
            .map(|t| t.amount.abs())
            .sum();
        let ready_to_pay = (budgeted_spending - refunds - paid_for_spending).max(Cents::ZERO).min(owed);
        let carried = (owed - ready_to_pay).max(Cents::ZERO);
        let utilization = self
            .account(id)
            .and_then(|a| a.credit_limit)
            .filter(|l| l.is_positive())
            .map(|l| (owed.get().saturating_mul(100) / l.get()).clamp(0, 999));
        CardSummary { owed, budgeted_spending, unbudgeted_spending, paid, ready_to_pay, carried, utilization }
    }

    // ------------------------------------------------------------------
    // Goals
    // ------------------------------------------------------------------

    /// Checks a goal the way [`Wallet::add_goal`] would, without adding it.
    pub fn validate_goal(&self, g: &Goal) -> Result<(), DomainError> {
        if g.name.trim().is_empty() || g.name.chars().count() > MAX_ACCOUNT_NAME {
            return Err(DomainError::InvalidName { max: MAX_ACCOUNT_NAME });
        }
        if g.kind == GoalKind::Save && !g.target_amount.is_positive() {
            return Err(DomainError::NonPositiveAmount);
        }
        if g.target_amount.is_negative() || g.starting_amount.is_negative() {
            return Err(DomainError::NegativeAmount);
        }
        match &g.track {
            GoalTrack::Account { id } if self.account(id).is_none() => Err(DomainError::not_found("account", id)),
            GoalTrack::Line { name } if name.trim().is_empty() => Err(DomainError::InvalidName { max: MAX_ACCOUNT_NAME }),
            _ => Ok(()),
        }
    }

    /// Adds a goal. For a payoff goal, `target_amount` should be the debt
    /// today (see [`Wallet::debt_now`]).
    pub fn add_goal(&mut self, mut g: Goal) -> Result<Id, DomainError> {
        g.name = g.name.trim().to_string();
        g.start_month = first_of(g.start_month);
        g.target_month = g.target_month.map(first_of);
        self.validate_goal(&g)?;
        g.sort_order = self.goals.iter().map(|x| x.sort_order).max().unwrap_or(0) + 1;
        let id = g.id.clone();
        self.goals.push(g);
        Ok(id)
    }

    pub fn update_goal(&mut self, mut g: Goal) -> Result<(), DomainError> {
        g.name = g.name.trim().to_string();
        g.start_month = first_of(g.start_month);
        g.target_month = g.target_month.map(first_of);
        self.validate_goal(&g)?;
        let slot = self.goals.iter_mut().find(|x| x.id == g.id).ok_or_else(|| DomainError::not_found("goal", &g.id))?;
        g.sort_order = slot.sort_order;
        *slot = g;
        Ok(())
    }

    pub fn delete_goal(&mut self, id: &Id) -> Result<(), DomainError> {
        if self.goal(id).is_none() {
            return Err(DomainError::not_found("goal", id));
        }
        self.goals.retain(|g| &g.id != id);
        Ok(())
    }

    /// The debt a payoff goal would start from today.
    #[must_use]
    pub fn debt_now(&self, track: &GoalTrack, months: &[Month], current: NaiveDate) -> Cents {
        self.debt_at(track, months, first_of(current))
    }

    /// Debt at the end of month `ym`.
    fn debt_at(&self, track: &GoalTrack, months: &[Month], ym: NaiveDate) -> Cents {
        match track {
            GoalTrack::Account { id } => (-self.balance_on(id, months, month_end(ym))).max(Cents::ZERO),
            GoalTrack::Line { name } => {
                // The balance on the debt line in the latest month up to `ym`.
                let mut ms: Vec<&Month> = months.iter().filter(|m| m.year_month <= ym).collect();
                ms.sort_by_key(|m| std::cmp::Reverse(m.year_month));
                ms.into_iter()
                    .find_map(|m| m.expense_lines.iter().find(|l| l.name.eq_ignore_ascii_case(name)).and_then(|l| l.current_balance))
                    .unwrap_or(Cents::ZERO)
            }
        }
    }

    /// Progress value (saved so far, or debt paid down) at the end of `ym`.
    fn value_at(&self, g: &Goal, months: &[Month], ym: NaiveDate) -> Cents {
        match (g.kind, &g.track) {
            (GoalKind::Save, GoalTrack::Line { name }) => {
                g.starting_amount
                    + months
                        .iter()
                        .filter(|m| m.year_month >= g.start_month && m.year_month <= ym)
                        .flat_map(|m| m.expense_lines.iter().filter(|l| l.name.eq_ignore_ascii_case(name)).map(move |l| m.line_planned(&l.id)))
                        .sum::<Cents>()
            }
            (GoalKind::Save, GoalTrack::Account { id }) => self.balance_on(id, months, month_end(ym)).max(Cents::ZERO),
            (GoalKind::Payoff, track) => (g.target_amount - self.debt_at(track, months, ym)).max(Cents::ZERO),
        }
    }

    /// Progress just before month `ym` began.
    fn value_before(&self, g: &Goal, months: &[Month], ym: NaiveDate) -> Cents {
        if ym <= g.start_month {
            return match (g.kind, &g.track) {
                (GoalKind::Save, GoalTrack::Line { .. }) => g.starting_amount,
                (GoalKind::Payoff, _) => Cents::ZERO,
                (GoalKind::Save, GoalTrack::Account { .. }) => {
                    self.value_at(g, months, ym.checked_sub_months(Months::new(1)).unwrap_or(ym))
                }
            };
        }
        self.value_at(g, months, ym.checked_sub_months(Months::new(1)).unwrap_or(ym))
    }

    /// Where a goal stands as of month `current` (any day in it).
    #[must_use]
    pub fn goal_progress(&self, g: &Goal, months: &[Month], current: NaiveDate) -> GoalProgress {
        let current = first_of(current);
        let target = g.target_amount;
        let value = self.value_at(g, months, current);
        let remaining = match g.kind {
            GoalKind::Save => (target - value).max(Cents::ZERO),
            GoalKind::Payoff => self.debt_at(&g.track, months, current),
        };
        let before = self.value_before(g, months, current);
        let this_month = value - before;
        let percent = if target.is_positive() { (value.get().saturating_mul(100) / target.get()).clamp(0, 100) } else { 100 };
        let months_left = g.target_month.map(|t| months_between_inclusive(current, t).max(1));
        // What the month needs: what was left when it began, spread over the
        // months that remain.
        let left_at_start = (remaining + this_month.max(Cents::ZERO)).max(Cents::ZERO);
        let needed_this_month = months_left.map(|n| Cents::new((left_at_start.get() + n - 1) / n));
        let status = if remaining.is_zero() && (g.kind == GoalKind::Payoff || value >= target) {
            GoalStatus::Done
        } else {
            match needed_this_month {
                None => GoalStatus::NoDate,
                Some(need) if this_month >= need => GoalStatus::OnTrack,
                Some(_) => GoalStatus::Behind,
            }
        };
        let first = g.start_month.max(current.checked_sub_months(Months::new(5)).unwrap_or(current));
        let mut history = Vec::new();
        let mut ym = first;
        let mut last = self.value_before(g, months, first);
        while ym <= current {
            let v = self.value_at(g, months, ym);
            history.push(GoalMonth { month: ym, value: v, change: v - last });
            last = v;
            match ym.checked_add_months(Months::new(1)) {
                Some(n) => ym = n,
                None => break,
            }
        }
        GoalProgress { current: value, target, remaining, percent, this_month, needed_this_month, months_left, status, history }
    }

    /// Goals whose progress comes from this budget line.
    #[must_use]
    pub fn goals_for_line(&self, line_name: &str) -> Vec<&Goal> {
        self.goals
            .iter()
            .filter(|g| matches!(&g.track, GoalTrack::Line { name } if name.eq_ignore_ascii_case(line_name)))
            .collect()
    }

    /// Converts every stored amount with `rate` (currency change).
    pub fn convert(&mut self, rate: &crate::money::Rate) {
        for a in &mut self.adjustments {
            a.amount = rate.convert(a.amount);
        }
        for a in &mut self.accounts {
            a.credit_limit = a.credit_limit.map(|c| rate.convert(c));
            a.minimum_payment = a.minimum_payment.map(|c| rate.convert(c));
        }
        for g in &mut self.goals {
            g.target_amount = rate.convert(g.target_amount);
            g.starting_amount = rate.convert(g.starting_amount);
        }
    }
}
