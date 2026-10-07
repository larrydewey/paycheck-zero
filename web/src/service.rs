//! Service layer: loads the month aggregate, applies a domain operation,
//! re-checks every invariant (spec §10) and persists. Both the REST API and
//! the Datastar UI go through here, so rules are never enforced only in a
//! client.

use crate::auth::{hash_password, verify_password, AuthUser};
use crate::error::{AppError, AppResult};
use crate::i18n::t;
use crate::AppState;
use chrono::NaiveDate;
use paycheckzero_core::recurrence::first_of_month;
use paycheckzero_core::suggest::IncomeHistory;
use paycheckzero_core::*;
use paycheckzero_storage::{Loaded, Owner, UserRecord};
use serde::{Deserialize, Serialize};

pub const MIN_PASSWORD: usize = 8;

/// Full-month snapshot for backup/restore (spec §13.6).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub format: String,
    pub version: u32,
    pub currency: String,
    pub archived: bool,
    pub month: Month,
}

pub const SNAPSHOT_FORMAT: &str = "paycheckzero-month-snapshot";

#[must_use]
pub fn valid_timezone(tz: &str) -> bool {
    tz.parse::<chrono_tz::Tz>().is_ok()
}

impl AppState {
    // ------------------------------------------------------------------
    // Accounts
    // ------------------------------------------------------------------

    /// Single-user v1: registration is open only until the first account exists.
    pub async fn registration_open(&self) -> AppResult<bool> {
        Ok(self.store.count_users().await? == 0)
    }

    pub async fn register(&self, email: &str, password: &str, timezone: &str) -> AppResult<UserRecord> {
        if !self.registration_open().await? {
            return Err(AppError::RegistrationClosed);
        }
        let email = email.trim().to_lowercase();
        if !email.contains('@') || email.len() > 255 {
            return Err(AppError::bad(t("err.email")));
        }
        if password.chars().count() < MIN_PASSWORD {
            return Err(AppError::bad(t("err.password_short")));
        }
        let tz = if valid_timezone(timezone) { timezone } else { "UTC" };
        let hash = hash_password(password)?;
        Ok(self.store.create_user(&email, &hash, tz, "USD").await?)
    }

    pub async fn login(&self, email: &str, password: &str) -> AppResult<UserRecord> {
        let user = self.store.user_by_email(&email.trim().to_lowercase()).await?;
        match user {
            Some(u) if verify_password(password, &u.password_hash) => Ok(u),
            _ => Err(AppError::InvalidCredentials),
        }
    }

    /// Adds a login to `owner`'s budget. Only the owner may.
    pub async fn add_member(&self, user: &AuthUser, email: &str, password: &str) -> AppResult<UserRecord> {
        if !user.is_owner() {
            return Err(AppError::bad(t("err.owner_only")));
        }
        let email = email.trim().to_lowercase();
        if !email.contains('@') || email.len() > 255 {
            return Err(AppError::bad(t("err.email")));
        }
        if password.chars().count() < MIN_PASSWORD {
            return Err(AppError::bad(t("err.password_short")));
        }
        let hash = hash_password(password)?;
        match self.store.create_member(&user.0, &email, &hash).await {
            Err(paycheckzero_storage::StorageError::Duplicate) => Err(AppError::bad(t("err.email_taken"))),
            r => Ok(r?),
        }
    }

    pub async fn remove_member(&self, user: &AuthUser, member: &Id) -> AppResult<()> {
        if !user.is_owner() {
            return Err(AppError::bad(t("err.owner_only")));
        }
        if self.store.delete_member(user.id(), member).await? { Ok(()) } else { Err(AppError::NotFound) }
    }

    /// Changes the signed-in login's password.
    pub async fn change_password(&self, user: &AuthUser, current: &str, new: &str) -> AppResult<()> {
        if !verify_password(current, &user.login().password_hash) {
            return Err(AppError::bad(t("err.password_wrong")));
        }
        if new.chars().count() < MIN_PASSWORD {
            return Err(AppError::bad(t("err.password_short")));
        }
        Ok(self.store.set_password(&user.login().id, &hash_password(new)?).await?)
    }

    /// "Today" in the user's timezone (spec §13.9).
    #[must_use]
    pub fn today_for(&self, user: &UserRecord) -> NaiveDate {
        self.clock.today(&user.timezone)
    }

    // ------------------------------------------------------------------
    // Months
    // ------------------------------------------------------------------

    pub async fn load(&self, user: &UserRecord, month: &Id) -> AppResult<Loaded> {
        self.store.load_month(&user.id, month).await?.ok_or(AppError::NotFound)
    }

    pub async fn resolve(&self, user: &UserRecord, kind: Owner, id: &Id) -> AppResult<Id> {
        self.store.owner_month(&user.id, kind, id).await?.ok_or(AppError::NotFound)
    }

    /// Applies `f` to the month and saves it. Archived months are read-only.
    pub async fn mutate<T>(
        &self,
        user: &UserRecord,
        month: &Id,
        f: impl FnOnce(&mut Month) -> Result<T, DomainError>,
    ) -> AppResult<(T, Month)> {
        let loaded = self.load(user, month).await?;
        if loaded.archived {
            return Err(AppError::Archived);
        }
        let mut next = loaded.month.clone();
        let out = f(&mut next)?;
        next.check_invariants()?;
        if next != loaded.month {
            self.store.save_month(&user.id, &loaded, &next).await?;
        }
        Ok((out, next))
    }

    /// Like [`Self::mutate`] for several months at once (e.g. moving a
    /// transaction), saved together. Months are saved in the given order.
    pub async fn mutate_months<T>(
        &self,
        user: &UserRecord,
        months: &[Id],
        f: impl FnOnce(&mut [Month]) -> Result<T, DomainError>,
    ) -> AppResult<(T, Vec<Month>)> {
        let mut loaded = Vec::with_capacity(months.len());
        for id in months {
            let l = self.load(user, id).await?;
            if l.archived {
                return Err(AppError::Archived);
            }
            loaded.push(l);
        }
        let mut next: Vec<Month> = loaded.iter().map(|l| l.month.clone()).collect();
        let out = f(&mut next)?;
        for m in &next {
            m.check_invariants()?;
        }
        let changes: Vec<(&Loaded, &Month)> = loaded.iter().zip(&next).filter(|(l, m)| &l.month != *m).collect();
        if !changes.is_empty() {
            self.store.save_months(&user.id, &changes).await?;
        }
        Ok((out, next))
    }

    pub async fn create_month(
        &self,
        user: &UserRecord,
        year_month: NaiveDate,
        mode: CopyMode,
        source: Option<&Id>,
    ) -> AppResult<Month> {
        let ym = first_of_month(year_month);
        if self.store.month_id_by_year_month(&user.id, ym).await?.is_some() {
            return Err(AppError::MonthExists);
        }
        let source = match (mode, source) {
            (CopyMode::Blank, _) => None,
            (_, Some(id)) => Some(self.load(user, id).await?.month),
            (_, None) => return Err(AppError::bad(t("err.source_required"))),
        };
        let month = Month::create(Id::generate(), ym, mode, source.as_ref());
        month.check_invariants()?;
        self.store.insert_month(&user.id, &month).await?;
        Ok(month)
    }

    pub async fn archive(&self, user: &UserRecord, month: &Id, archived: bool) -> AppResult<()> {
        if !self.store.set_archived(&user.id, month, archived).await? {
            return Err(AppError::NotFound);
        }
        Ok(())
    }

    /// Permanent delete; `confirm` must equal the month's `YYYY-MM`.
    pub async fn delete_month(&self, user: &UserRecord, month: &Id, confirm: &str) -> AppResult<()> {
        let loaded = self.load(user, month).await?;
        if confirm.trim() != loaded.month.year_month.format("%Y-%m").to_string() {
            return Err(AppError::bad(t("err.delete_confirm")));
        }
        self.store.delete_month(&user.id, month).await?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Wallet: accounts, credit cards, goals
    // ------------------------------------------------------------------

    pub async fn wallet(&self, user: &UserRecord) -> AppResult<Wallet> {
        Ok(self.store.load_wallet(&user.id).await?)
    }

    /// Applies `f` to the wallet (with every month for balances) and saves it.
    pub async fn mutate_wallet<T>(
        &self,
        user: &UserRecord,
        f: impl FnOnce(&mut Wallet, &[Month]) -> Result<T, AppError>,
    ) -> AppResult<(T, Wallet)> {
        let before = self.wallet(user).await?;
        let months = self.all_months(user).await?;
        let mut next = before.clone();
        let out = f(&mut next, &months)?;
        if next != before {
            self.store.save_wallet(&user.id, &next).await?;
        }
        Ok((out, next))
    }

    /// Fails unless every given account exists.
    pub async fn check_accounts(&self, user: &UserRecord, ids: &[Option<&Id>]) -> AppResult<()> {
        if ids.iter().all(Option::is_none) {
            return Ok(());
        }
        let w = self.wallet(user).await?;
        for id in ids.iter().flatten() {
            if w.account(id).is_none() {
                return Err(AppError::Domain(DomainError::NotFound { kind: "account", id: (*id).clone() }));
            }
        }
        Ok(())
    }

    pub async fn all_months(&self, user: &UserRecord) -> AppResult<Vec<Month>> {
        Ok(self.store.load_all_months(&user.id).await?.into_iter().map(|l| l.month).collect())
    }

    pub async fn income_history(&self, user: &UserRecord) -> AppResult<Vec<IncomeHistory>> {
        let mut out = Vec::new();
        for m in self.all_months(user).await? {
            for l in &m.income_lines {
                out.push(IncomeHistory {
                    year_month: m.year_month,
                    name: l.name.clone(),
                    planned_amount: l.planned_amount,
                    recurrence_rule: l.recurrence_rule.clone(),
                    dates: m.paychecks.iter().filter(|p| p.income_line_id == l.id).map(|p| p.date).collect(),
                });
            }
        }
        Ok(out)
    }

    /// Converts every month by a user-supplied rate and switches currency
    /// atomically (spec §13.9).
    pub async fn change_currency(&self, user: &UserRecord, code: &str, rate: &str) -> AppResult<()> {
        if !crate::money::is_supported(code) {
            return Err(AppError::bad(t("err.currency")));
        }
        if code == user.currency {
            return Ok(());
        }
        let rate = Rate::parse(rate).ok_or_else(|| AppError::bad(t("err.rate")))?;
        let mut changes = Vec::new();
        for loaded in self.store.load_all_months(&user.id).await? {
            let mut m = loaded.month.clone();
            m.convert_currency(rate);
            m.check_invariants()?;
            changes.push((loaded, m));
        }
        let mut wallet = self.store.load_wallet(&user.id).await?;
        wallet.convert(&rate);
        self.store.save_months_with_currency(&user.id, &changes, &wallet, code).await?;
        Ok(())
    }

    pub async fn snapshot(&self, user: &UserRecord, month: &Id) -> AppResult<Snapshot> {
        let loaded = self.load(user, month).await?;
        Ok(Snapshot {
            format: SNAPSHOT_FORMAT.into(),
            version: 1,
            currency: user.currency.clone(),
            archived: loaded.archived,
            month: loaded.month,
        })
    }

    /// Restores a snapshot as a month (fresh ids). With `replace`, an existing
    /// month for the same year-month is permanently replaced.
    pub async fn restore(&self, user: &UserRecord, snap: Snapshot, replace: bool) -> AppResult<Month> {
        if snap.format != SNAPSHOT_FORMAT || snap.version != 1 {
            return Err(AppError::bad(t("err.snapshot_format")));
        }
        if snap.currency != user.currency {
            return Err(AppError::bad(t("err.snapshot_currency")));
        }
        let month = remap_ids(snap.month);
        month.check_invariants()?;
        if let Some(existing) = self.store.month_id_by_year_month(&user.id, month.year_month).await? {
            if !replace {
                return Err(AppError::MonthExists);
            }
            self.store.delete_month(&user.id, &existing).await?;
        }
        self.store.insert_month(&user.id, &month).await?;
        if snap.archived {
            self.store.set_archived(&user.id, &month.id, true).await?;
        }
        Ok(month)
    }
}

/// Gives every entity in a month a fresh id, keeping all references intact.
fn remap_ids(mut m: Month) -> Month {
    use std::collections::HashMap;
    fn fresh(old: &Id, map: &mut HashMap<Id, Id>) -> Id {
        map.entry(old.clone()).or_insert_with(Id::generate).clone()
    }
    let mut map: HashMap<Id, Id> = HashMap::new();
    m.id = Id::generate();
    for x in &mut m.income_lines {
        x.id = fresh(&x.id, &mut map);
    }
    for x in &mut m.categories {
        x.id = fresh(&x.id, &mut map);
    }
    for x in &mut m.paychecks {
        x.id = fresh(&x.id, &mut map);
        x.income_line_id = fresh(&x.income_line_id, &mut map);
    }
    for x in &mut m.expense_lines {
        x.id = fresh(&x.id, &mut map);
        x.category_id = fresh(&x.category_id, &mut map);
    }
    for x in &mut m.allocations {
        x.id = Id::generate();
        x.paycheck_id = fresh(&x.paycheck_id, &mut map);
        x.expense_line_id = fresh(&x.expense_line_id, &mut map);
    }
    for x in &mut m.transactions {
        x.id = Id::generate();
        x.paycheck_id = x.paycheck_id.as_ref().map(|p| fresh(p, &mut map));
        x.expense_line_id = x.expense_line_id.as_ref().map(|l| fresh(l, &mut map));
    }
    m
}
