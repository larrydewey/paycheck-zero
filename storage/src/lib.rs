//! PaycheckZero persistence adapter.
//!
//! Loads and saves the [`Month`] aggregate and user/auth records through
//! sqlx's `Any` driver so one code path serves SQLite (primary), PostgreSQL
//! and MariaDB (spec §7.3). Saving diffs the aggregate against the version
//! that was loaded and applies inserts/updates/deletes inside one database
//! transaction, guarded by an optimistic `version` check.

mod schema;

use chrono::{NaiveDate, SecondsFormat, Utc};
use paycheckzero_core::*;
use sqlx::any::{AnyConnectOptions, AnyPoolOptions, AnyRow};
use sqlx::{AnyConnection, AnyPool, Row};
use std::str::FromStr;
use std::time::Duration;

pub use schema::MIGRATIONS;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("the record was changed by someone else; reload and try again")]
    Conflict,
    #[error("a record with the same unique key already exists")]
    Duplicate,
    #[error("stored data is invalid: {0}")]
    Corrupt(String),
    #[error("unsupported database url: {0}")]
    UnsupportedUrl(String),
}

pub type Result<T> = std::result::Result<T, StorageError>;

fn map_db(e: sqlx::Error) -> StorageError {
    if let sqlx::Error::Database(db) = &e {
        if db.is_unique_violation() {
            return StorageError::Duplicate;
        }
    }
    StorageError::Db(e)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Sqlite,
    Postgres,
    MySql,
}

/// A login. Owners hold a budget; members (`owner_id` set) share one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserRecord {
    pub id: Id,
    pub email: String,
    pub password_hash: String,
    pub timezone: String,
    pub currency: String,
    pub token_version: i64,
    pub last_month_id: Option<Id>,
    pub owner_id: Option<Id>,
}

/// Listing entry for a month.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonthMeta {
    pub id: Id,
    pub year_month: NaiveDate,
    pub status: MonthStatus,
    pub reassigning: bool,
    pub archived: bool,
}

/// A month as loaded, with its concurrency version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    pub month: Month,
    pub version: i64,
    pub archived: bool,
}

/// Entity kinds that can be resolved to their owning month.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    IncomeLine,
    Paycheck,
    Category,
    ExpenseLine,
    Allocation,
    Transaction,
}

#[derive(Clone)]
pub struct Store {
    pool: AnyPool,
    dialect: Dialect,
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn date_str(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

fn parse_date(s: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| StorageError::Corrupt(format!("bad date {s}")))
}

fn opt_cents(v: Option<i64>) -> Option<Cents> {
    v.map(Cents::new)
}

fn opt_i64(v: Option<Cents>) -> Option<i64> {
    v.map(Cents::get)
}

fn opt_id(v: Option<String>) -> Option<Id> {
    v.map(Id::new)
}

fn opt_str(v: &Option<Id>) -> Option<String> {
    v.as_ref().map(|i| i.as_str().to_string())
}

impl Store {
    /// Connects and runs migrations. Accepts `sqlite:`, `postgres://` and
    /// `mysql://` / `mariadb://` URLs.
    pub async fn connect(url: &str) -> Result<Store> {
        sqlx::any::install_default_drivers();
        let url = url.replacen("mariadb://", "mysql://", 1);
        let dialect = if url.starts_with("sqlite:") {
            Dialect::Sqlite
        } else if url.starts_with("postgres://") || url.starts_with("postgresql://") {
            Dialect::Postgres
        } else if url.starts_with("mysql://") {
            Dialect::MySql
        } else {
            return Err(StorageError::UnsupportedUrl(url));
        };
        let memory = url.contains(":memory:") || url.contains("mode=memory");
        let options = AnyConnectOptions::from_str(&url)?;
        let pool = AnyPoolOptions::new()
            .max_connections(if dialect == Dialect::Sqlite { if memory { 1 } else { 4 } } else { 8 })
            .idle_timeout(if memory { None } else { Some(Duration::from_secs(600)) })
            .max_lifetime(if memory { None } else { Some(Duration::from_secs(1800)) })
            .after_connect(move |conn, _| {
                Box::pin(async move {
                    match dialect {
                        Dialect::Sqlite => {
                            sqlx::query("PRAGMA foreign_keys = ON").execute(&mut *conn).await?;
                            sqlx::query("PRAGMA busy_timeout = 5000").execute(&mut *conn).await?;
                            if !memory {
                                sqlx::query("PRAGMA journal_mode = WAL").execute(&mut *conn).await?;
                            }
                        }
                        Dialect::MySql => {
                            sqlx::query("SET SESSION sql_mode = CONCAT(@@sql_mode, ',ANSI_QUOTES')")
                                .execute(&mut *conn)
                                .await?;
                        }
                        Dialect::Postgres => {}
                    }
                    Ok(())
                })
            })
            .connect_with(options)
            .await?;
        let store = Store { pool, dialect };
        store.migrate().await?;
        Ok(store)
    }

    #[must_use]
    pub fn dialect(&self) -> Dialect {
        self.dialect
    }

    /// Rewrites `?` placeholders to `$n` for PostgreSQL.
    fn sql(&self, s: &str) -> String {
        if self.dialect != Dialect::Postgres {
            return s.to_string();
        }
        let mut out = String::with_capacity(s.len() + 8);
        let mut n = 0;
        for ch in s.chars() {
            if ch == '?' {
                n += 1;
                out.push('$');
                out.push_str(&n.to_string());
            } else {
                out.push(ch);
            }
        }
        out
    }

    async fn migrate(&self) -> Result<()> {
        let mut conn = self.pool.acquire().await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS schema_migrations (version BIGINT NOT NULL PRIMARY KEY, applied_at VARCHAR(40) NOT NULL)")
            .execute(&mut *conn)
            .await?;
        for (version, statements) in MIGRATIONS {
            let done: Option<i64> = sqlx::query(&self.sql("SELECT version FROM schema_migrations WHERE version = ?"))
                .bind(*version)
                .fetch_optional(&mut *conn)
                .await?
                .map(|r| r.get::<i64, _>(0));
            if done.is_some() {
                continue;
            }
            for stmt in *statements {
                sqlx::query(stmt).execute(&mut *conn).await?;
            }
            sqlx::query(&self.sql("INSERT INTO schema_migrations (version, applied_at) VALUES (?, ?)"))
                .bind(*version)
                .bind(now())
                .execute(&mut *conn)
                .await?;
        }
        Ok(())
    }

    /// Deletes every row (test isolation, spec §13.1).
    pub async fn reset(&self) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        for t in schema::TABLES_CHILD_FIRST {
            sqlx::query(&format!("DELETE FROM {t}")).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Users
    // ------------------------------------------------------------------

    pub async fn count_users(&self) -> Result<i64> {
        let row = sqlx::query("SELECT COUNT(*) FROM users").fetch_one(&self.pool).await?;
        Ok(row.try_get::<i64, _>(0)?)
    }

    pub async fn create_user(&self, email: &str, password_hash: &str, timezone: &str, currency: &str) -> Result<UserRecord> {
        let id = Id::generate();
        let ts = now();
        sqlx::query(&self.sql(
            "INSERT INTO users (id, email, password_hash, timezone, currency, token_version, created_at, updated_at) VALUES (?, ?, ?, ?, ?, 0, ?, ?)",
        ))
        .bind(id.as_str())
        .bind(email)
        .bind(password_hash)
        .bind(timezone)
        .bind(currency)
        .bind(&ts)
        .bind(&ts)
        .execute(&self.pool)
        .await
        .map_err(map_db)?;
        self.user_by_id(&id).await?.ok_or_else(|| StorageError::Corrupt("user vanished".into()))
    }

    fn user_from_row(row: &AnyRow) -> Result<UserRecord> {
        Ok(UserRecord {
            id: Id::new(row.try_get::<String, _>("id")?),
            email: row.try_get("email")?,
            password_hash: row.try_get("password_hash")?,
            timezone: row.try_get("timezone")?,
            currency: row.try_get("currency")?,
            token_version: row.try_get("token_version")?,
            last_month_id: opt_id(row.try_get("last_month_id")?),
            owner_id: opt_id(row.try_get("owner_id")?),
        })
    }

    const USER_COLS: &'static str = "id, email, password_hash, timezone, currency, token_version, last_month_id, owner_id";

    pub async fn user_by_id(&self, id: &Id) -> Result<Option<UserRecord>> {
        let row = sqlx::query(&self.sql(&format!("SELECT {} FROM users WHERE id = ?", Self::USER_COLS)))
            .bind(id.as_str())
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(Self::user_from_row).transpose()
    }

    pub async fn user_by_email(&self, email: &str) -> Result<Option<UserRecord>> {
        let row = sqlx::query(&self.sql(&format!("SELECT {} FROM users WHERE email = ?", Self::USER_COLS)))
            .bind(email)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(Self::user_from_row).transpose()
    }

    /// Adds a login that shares `owner`'s budget.
    pub async fn create_member(&self, owner: &UserRecord, email: &str, password_hash: &str) -> Result<UserRecord> {
        let id = Id::generate();
        let ts = now();
        sqlx::query(&self.sql(
            "INSERT INTO users (id, email, password_hash, timezone, currency, token_version, owner_id, created_at, updated_at) VALUES (?, ?, ?, ?, ?, 0, ?, ?, ?)",
        ))
        .bind(id.as_str())
        .bind(email)
        .bind(password_hash)
        .bind(&owner.timezone)
        .bind(&owner.currency)
        .bind(owner.id.as_str())
        .bind(&ts)
        .bind(&ts)
        .execute(&self.pool)
        .await
        .map_err(map_db)?;
        self.user_by_id(&id).await?.ok_or_else(|| StorageError::Corrupt("user vanished".into()))
    }

    /// Logins sharing `owner`'s budget, oldest first.
    pub async fn members(&self, owner: &Id) -> Result<Vec<UserRecord>> {
        let rows = sqlx::query(&self.sql(&format!("SELECT {} FROM users WHERE owner_id = ? ORDER BY created_at", Self::USER_COLS)))
            .bind(owner.as_str())
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(Self::user_from_row).collect()
    }

    /// Removes a member login; false when it isn't `owner`'s.
    pub async fn delete_member(&self, owner: &Id, member: &Id) -> Result<bool> {
        let mut tx = self.pool.begin().await?;
        let owned = sqlx::query(&self.sql("SELECT id FROM users WHERE id = ? AND owner_id = ?"))
            .bind(member.as_str())
            .bind(owner.as_str())
            .fetch_optional(&mut *tx)
            .await?
            .is_some();
        if owned {
            sqlx::query(&self.sql("DELETE FROM refresh_tokens WHERE user_id = ?")).bind(member.as_str()).execute(&mut *tx).await?;
            sqlx::query(&self.sql("DELETE FROM users WHERE id = ?")).bind(member.as_str()).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(owned)
    }

    pub async fn set_password(&self, user: &Id, password_hash: &str) -> Result<()> {
        sqlx::query(&self.sql("UPDATE users SET password_hash = ?, updated_at = ? WHERE id = ?"))
            .bind(password_hash)
            .bind(now())
            .bind(user.as_str())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_timezone(&self, user: &Id, timezone: &str) -> Result<()> {
        sqlx::query(&self.sql("UPDATE users SET timezone = ?, updated_at = ? WHERE id = ?"))
            .bind(timezone)
            .bind(now())
            .bind(user.as_str())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_last_month(&self, user: &Id, month: Option<&Id>) -> Result<()> {
        sqlx::query(&self.sql("UPDATE users SET last_month_id = ? WHERE id = ?"))
            .bind(month.map(|m| m.as_str().to_string()))
            .bind(user.as_str())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Invalidates every outstanding access token ("log out of all devices").
    pub async fn bump_token_version(&self, user: &Id) -> Result<()> {
        sqlx::query(&self.sql("UPDATE users SET token_version = token_version + 1 WHERE id = ?"))
            .bind(user.as_str())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Refresh tokens
    // ------------------------------------------------------------------

    pub async fn insert_refresh(&self, token_hash: &str, user: &Id, expires_at: &str) -> Result<()> {
        sqlx::query(&self.sql("INSERT INTO refresh_tokens (token_hash, user_id, expires_at, created_at) VALUES (?, ?, ?, ?)"))
            .bind(token_hash)
            .bind(user.as_str())
            .bind(expires_at)
            .bind(now())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Removes and returns a refresh token (rotation: each token works once).
    pub async fn take_refresh(&self, token_hash: &str) -> Result<Option<(Id, String)>> {
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(&self.sql("SELECT user_id, expires_at FROM refresh_tokens WHERE token_hash = ?"))
            .bind(token_hash)
            .fetch_optional(&mut *tx)
            .await?;
        let out = match row {
            Some(r) => {
                let affected = sqlx::query(&self.sql("DELETE FROM refresh_tokens WHERE token_hash = ?"))
                    .bind(token_hash)
                    .execute(&mut *tx)
                    .await?
                    .rows_affected();
                (affected == 1).then(|| -> Result<(Id, String)> {
                    Ok((Id::new(r.try_get::<String, _>(0)?), r.try_get::<String, _>(1)?))
                })
            }
            None => None,
        };
        tx.commit().await?;
        out.transpose()
    }

    pub async fn delete_refresh(&self, token_hash: &str) -> Result<()> {
        sqlx::query(&self.sql("DELETE FROM refresh_tokens WHERE token_hash = ?"))
            .bind(token_hash)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn delete_all_refresh(&self, user: &Id) -> Result<()> {
        sqlx::query(&self.sql("DELETE FROM refresh_tokens WHERE user_id = ?"))
            .bind(user.as_str())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Sync idempotency
    // ------------------------------------------------------------------

    pub async fn sync_result(&self, user: &Id, op_id: &str) -> Result<Option<String>> {
        let row = sqlx::query(&self.sql("SELECT result FROM sync_ops WHERE op_id = ? AND user_id = ?"))
            .bind(op_id)
            .bind(user.as_str())
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| r.try_get::<String, _>(0)).transpose()?)
    }

    pub async fn record_sync(&self, user: &Id, op_id: &str, result: &str) -> Result<()> {
        sqlx::query(&self.sql("INSERT INTO sync_ops (op_id, user_id, result, created_at) VALUES (?, ?, ?, ?)"))
            .bind(op_id)
            .bind(user.as_str())
            .bind(result)
            .bind(now())
            .execute(&self.pool)
            .await
            .map_err(map_db)?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Months
    // ------------------------------------------------------------------

    pub async fn list_months(&self, user: &Id, include_archived: bool) -> Result<Vec<MonthMeta>> {
        let filter = if include_archived { "" } else { " AND archived_at IS NULL" };
        let rows = sqlx::query(&self.sql(&format!(
            r#"SELECT id, "year_month", status, reassigning, archived_at FROM months WHERE user_id = ?{filter} ORDER BY "year_month" DESC"#
        )))
        .bind(user.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|r| {
                Ok(MonthMeta {
                    id: Id::new(r.try_get::<String, _>(0)?),
                    year_month: parse_date(&r.try_get::<String, _>(1)?)?,
                    status: MonthStatus::parse(&r.try_get::<String, _>(2)?)
                        .ok_or_else(|| StorageError::Corrupt("status".into()))?,
                    reassigning: r.try_get::<i64, _>(3)? == 1,
                    archived: r.try_get::<Option<String>, _>(4)?.is_some(),
                })
            })
            .collect()
    }

    pub async fn month_id_by_year_month(&self, user: &Id, ym: NaiveDate) -> Result<Option<Id>> {
        let row = sqlx::query(&self.sql(r#"SELECT id FROM months WHERE user_id = ? AND "year_month" = ?"#))
            .bind(user.as_str())
            .bind(date_str(ym))
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| r.try_get::<String, _>(0).map(Id::new)).transpose()?)
    }

    /// Resolves the month that owns a child entity, scoped to the user.
    pub async fn owner_month(&self, user: &Id, kind: Owner, id: &Id) -> Result<Option<Id>> {
        let sql = match kind {
            Owner::IncomeLine => "SELECT m.id FROM income_lines x JOIN months m ON m.id = x.month_id WHERE x.id = ? AND m.user_id = ?",
            Owner::Paycheck => "SELECT m.id FROM paychecks x JOIN income_lines i ON i.id = x.income_line_id JOIN months m ON m.id = i.month_id WHERE x.id = ? AND m.user_id = ?",
            Owner::Category => "SELECT m.id FROM expense_categories x JOIN months m ON m.id = x.month_id WHERE x.id = ? AND m.user_id = ?",
            Owner::ExpenseLine => "SELECT m.id FROM expense_lines x JOIN expense_categories c ON c.id = x.category_id JOIN months m ON m.id = c.month_id WHERE x.id = ? AND m.user_id = ?",
            Owner::Allocation => "SELECT m.id FROM allocations x JOIN paychecks p ON p.id = x.paycheck_id JOIN income_lines i ON i.id = p.income_line_id JOIN months m ON m.id = i.month_id WHERE x.id = ? AND m.user_id = ?",
            Owner::Transaction => "SELECT m.id FROM transactions x JOIN months m ON m.id = x.month_id WHERE x.id = ? AND m.user_id = ?",
        };
        let row = sqlx::query(&self.sql(sql)).bind(id.as_str()).bind(user.as_str()).fetch_optional(&self.pool).await?;
        Ok(row.map(|r| r.try_get::<String, _>(0).map(Id::new)).transpose()?)
    }

    pub async fn load_month(&self, user: &Id, id: &Id) -> Result<Option<Loaded>> {
        let mut conn = self.pool.acquire().await?;
        self.load_with(&mut conn, user, id).await
    }

    /// Every month of the user, including archived ones, oldest first.
    pub async fn load_all_months(&self, user: &Id) -> Result<Vec<Loaded>> {
        let metas = self.list_months(user, true).await?;
        let mut conn = self.pool.acquire().await?;
        let mut out = Vec::with_capacity(metas.len());
        for m in metas.iter().rev() {
            if let Some(l) = self.load_with(&mut conn, user, &m.id).await? {
                out.push(l);
            }
        }
        Ok(out)
    }

    async fn load_with(&self, conn: &mut AnyConnection, user: &Id, id: &Id) -> Result<Option<Loaded>> {
        let Some(row) = sqlx::query(&self.sql(
            r#"SELECT id, "year_month", status, reassigning, archived_at, version FROM months WHERE id = ? AND user_id = ?"#,
        ))
        .bind(id.as_str())
        .bind(user.as_str())
        .fetch_optional(&mut *conn)
        .await?
        else {
            return Ok(None);
        };
        let mut month = Month::new(id.clone(), parse_date(&row.try_get::<String, _>(1)?)?);
        month.status =
            MonthStatus::parse(&row.try_get::<String, _>(2)?).ok_or_else(|| StorageError::Corrupt("status".into()))?;
        month.reassigning = row.try_get::<i64, _>(3)? == 1;
        let archived = row.try_get::<Option<String>, _>(4)?.is_some();
        let version: i64 = row.try_get(5)?;
        let mid = id.as_str();

        for r in sqlx::query(&self.sql(
            "SELECT id, name, planned_amount, recurrence_rule FROM income_lines WHERE month_id = ? ORDER BY position, id",
        ))
        .bind(mid)
        .fetch_all(&mut *conn)
        .await?
        {
            let rule: Option<String> = r.try_get(3)?;
            let recurrence_rule = match rule {
                Some(s) => Some(Recurrence::from_json(&s).ok_or_else(|| StorageError::Corrupt(format!("rule {s}")))?),
                None => None,
            };
            month.income_lines.push(IncomeLine {
                id: Id::new(r.try_get::<String, _>(0)?),
                name: r.try_get(1)?,
                planned_amount: Cents::new(r.try_get(2)?),
                recurrence_rule,
            });
        }

        for r in sqlx::query(&self.sql(
            "SELECT p.id, p.income_line_id, p.date, p.planned_amount, p.actual_amount, p.status FROM paychecks p JOIN income_lines i ON i.id = p.income_line_id WHERE i.month_id = ? ORDER BY p.position, p.id",
        ))
        .bind(mid)
        .fetch_all(&mut *conn)
        .await?
        {
            month.paychecks.push(Paycheck {
                id: Id::new(r.try_get::<String, _>(0)?),
                income_line_id: Id::new(r.try_get::<String, _>(1)?),
                date: parse_date(&r.try_get::<String, _>(2)?)?,
                planned_amount: Cents::new(r.try_get(3)?),
                actual_amount: opt_cents(r.try_get(4)?),
                status: PaycheckStatus::parse(&r.try_get::<String, _>(5)?)
                    .ok_or_else(|| StorageError::Corrupt("paycheck status".into()))?,
            });
        }

        for r in sqlx::query(&self.sql(
            "SELECT id, name, sort_order, kind FROM expense_categories WHERE month_id = ? ORDER BY position, id",
        ))
        .bind(mid)
        .fetch_all(&mut *conn)
        .await?
        {
            month.categories.push(ExpenseCategory {
                id: Id::new(r.try_get::<String, _>(0)?),
                name: r.try_get(1)?,
                sort_order: i32::try_from(r.try_get::<i64, _>(2)?).unwrap_or(0),
                kind: CategoryKind::parse(&r.try_get::<String, _>(3)?).ok_or_else(|| StorageError::Corrupt("kind".into()))?,
            });
        }

        for r in sqlx::query(&self.sql(
            "SELECT l.id, l.category_id, l.name, l.sort_order, l.current_balance, l.minimum_payment, l.target_amount FROM expense_lines l JOIN expense_categories c ON c.id = l.category_id WHERE c.month_id = ? ORDER BY l.position, l.id",
        ))
        .bind(mid)
        .fetch_all(&mut *conn)
        .await?
        {
            month.expense_lines.push(ExpenseLine {
                id: Id::new(r.try_get::<String, _>(0)?),
                category_id: Id::new(r.try_get::<String, _>(1)?),
                name: r.try_get(2)?,
                sort_order: i32::try_from(r.try_get::<i64, _>(3)?).unwrap_or(0),
                current_balance: opt_cents(r.try_get(4)?),
                minimum_payment: opt_cents(r.try_get(5)?),
                target_amount: opt_cents(r.try_get(6)?),
            });
        }

        for r in sqlx::query(&self.sql(
            "SELECT a.id, a.expense_line_id, a.paycheck_id, a.amount FROM allocations a JOIN paychecks p ON p.id = a.paycheck_id JOIN income_lines i ON i.id = p.income_line_id WHERE i.month_id = ? ORDER BY a.position, a.id",
        ))
        .bind(mid)
        .fetch_all(&mut *conn)
        .await?
        {
            month.allocations.push(Allocation {
                id: Id::new(r.try_get::<String, _>(0)?),
                expense_line_id: Id::new(r.try_get::<String, _>(1)?),
                paycheck_id: Id::new(r.try_get::<String, _>(2)?),
                amount: Cents::new(r.try_get(3)?),
            });
        }

        for r in sqlx::query(&self.sql(
            "SELECT id, date, amount, payee, notes, expense_line_id, paycheck_id, split_group, account_id, transfer_account_id, external_id FROM transactions WHERE month_id = ? ORDER BY position, id",
        ))
        .bind(mid)
        .fetch_all(&mut *conn)
        .await?
        {
            month.transactions.push(Transaction {
                id: Id::new(r.try_get::<String, _>(0)?),
                date: parse_date(&r.try_get::<String, _>(1)?)?,
                amount: Cents::new(r.try_get(2)?),
                payee: r.try_get(3)?,
                notes: r.try_get(4)?,
                expense_line_id: opt_id(r.try_get(5)?),
                paycheck_id: opt_id(r.try_get(6)?),
                split_group: opt_id(r.try_get(7)?),
                account_id: opt_id(r.try_get(8)?),
                transfer_account_id: opt_id(r.try_get(9)?),
                external_id: r.try_get(10)?,
            });
        }

        Ok(Some(Loaded { month, version, archived }))
    }

    /// Inserts a brand-new month aggregate. Fails with
    /// [`StorageError::Duplicate`] if the user already has that year-month.
    pub async fn insert_month(&self, user: &Id, month: &Month) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let ts = now();
        sqlx::query(&self.sql(
            r#"INSERT INTO months (id, user_id, "year_month", status, reassigning, version, created_at, updated_at) VALUES (?, ?, ?, ?, ?, 0, ?, ?)"#,
        ))
        .bind(month.id.as_str())
        .bind(user.as_str())
        .bind(date_str(month.year_month))
        .bind(month.status.as_str())
        .bind(i64::from(month.reassigning))
        .bind(&ts)
        .bind(&ts)
        .execute(&mut *tx)
        .await
        .map_err(map_db)?;
        let empty = Month::new(month.id.clone(), month.year_month);
        self.apply_diff(&mut tx, &empty, month).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Persists `after` over the `before` snapshot that was loaded.
    /// Returns the new version, or [`StorageError::Conflict`] when the month
    /// changed since it was loaded.
    pub async fn save_month(&self, user: &Id, before: &Loaded, after: &Month) -> Result<i64> {
        let mut tx = self.pool.begin().await?;
        let v = self.save_in(&mut tx, user, before, after).await?;
        tx.commit().await?;
        Ok(v)
    }

    /// Saves several months atomically, in the given order (so a row that
    /// moves leaves its old month before it arrives in the new one).
    pub async fn save_months(&self, user: &Id, changes: &[(&Loaded, &Month)]) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        for (before, after) in changes {
            self.save_in(&mut tx, user, before, after).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Saves several months and changes the user's currency atomically.
    pub async fn save_months_with_currency(&self, user: &Id, changes: &[(Loaded, Month)], wallet: &Wallet, currency: &str) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        for (before, after) in changes {
            self.save_in(&mut tx, user, before, after).await?;
        }
        self.write_wallet(&mut tx, user, wallet).await?;
        sqlx::query(&self.sql("UPDATE users SET currency = ?, updated_at = ? WHERE id = ?"))
            .bind(currency)
            .bind(now())
            .bind(user.as_str())
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn save_in(&self, conn: &mut AnyConnection, user: &Id, before: &Loaded, after: &Month) -> Result<i64> {
        let res = sqlx::query(&self.sql(
            "UPDATE months SET status = ?, reassigning = ?, version = version + 1, updated_at = ? WHERE id = ? AND user_id = ? AND version = ?",
        ))
        .bind(after.status.as_str())
        .bind(i64::from(after.reassigning))
        .bind(now())
        .bind(after.id.as_str())
        .bind(user.as_str())
        .bind(before.version)
        .execute(&mut *conn)
        .await?;
        if res.rows_affected() != 1 {
            return Err(StorageError::Conflict);
        }
        self.apply_diff(conn, &before.month, after).await?;
        Ok(before.version + 1)
    }

    pub async fn set_archived(&self, user: &Id, id: &Id, archived: bool) -> Result<bool> {
        let res = sqlx::query(&self.sql("UPDATE months SET archived_at = ?, version = version + 1 WHERE id = ? AND user_id = ?"))
            .bind(archived.then(now))
            .bind(id.as_str())
            .bind(user.as_str())
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() == 1)
    }

    /// Permanently deletes a month and everything in it.
    pub async fn delete_month(&self, user: &Id, id: &Id) -> Result<bool> {
        let Some(loaded) = self.load_month(user, id).await? else {
            return Ok(false);
        };
        let mut tx = self.pool.begin().await?;
        let empty = Month::new(id.clone(), loaded.month.year_month);
        self.apply_diff(&mut tx, &loaded.month, &empty).await?;
        sqlx::query(&self.sql("UPDATE users SET last_month_id = NULL WHERE id = ? AND last_month_id = ?"))
            .bind(user.as_str())
            .bind(id.as_str())
            .execute(&mut *tx)
            .await?;
        sqlx::query(&self.sql("DELETE FROM months WHERE id = ? AND user_id = ?"))
            .bind(id.as_str())
            .bind(user.as_str())
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(true)
    }

    // ------------------------------------------------------------------
    // Wallet: accounts, adjustments, goals
    // ------------------------------------------------------------------

    pub async fn load_wallet(&self, user: &Id) -> Result<Wallet> {
        let mut w = Wallet::default();
        for r in sqlx::query(&self.sql(
            "SELECT id, name, kind, sort_order, archived, credit_limit, apr_bp, minimum_payment, reconciled_on, external_id, link_id FROM accounts WHERE user_id = ? ORDER BY sort_order, id",
        ))
        .bind(user.as_str())
        .fetch_all(&self.pool)
        .await?
        {
            w.accounts.push(Account {
                id: Id::new(r.try_get::<String, _>(0)?),
                name: r.try_get(1)?,
                kind: AccountKind::parse(&r.try_get::<String, _>(2)?).ok_or_else(|| StorageError::Corrupt("account kind".into()))?,
                sort_order: i32::try_from(r.try_get::<i64, _>(3)?).unwrap_or(0),
                archived: r.try_get::<i64, _>(4)? == 1,
                credit_limit: opt_cents(r.try_get(5)?),
                apr_bp: r.try_get(6)?,
                minimum_payment: opt_cents(r.try_get(7)?),
                reconciled_on: r.try_get::<Option<String>, _>(8)?.map(|s| parse_date(&s)).transpose()?,
                external_id: r.try_get(9)?,
                link_id: opt_id(r.try_get(10)?),
            });
        }
        for r in sqlx::query(&self.sql(
            "SELECT x.id, x.account_id, x.date, x.amount, x.kind FROM account_adjustments x JOIN accounts a ON a.id = x.account_id WHERE a.user_id = ? ORDER BY x.created_at, x.id",
        ))
        .bind(user.as_str())
        .fetch_all(&self.pool)
        .await?
        {
            w.adjustments.push(Adjustment {
                id: Id::new(r.try_get::<String, _>(0)?),
                account_id: Id::new(r.try_get::<String, _>(1)?),
                date: parse_date(&r.try_get::<String, _>(2)?)?,
                amount: Cents::new(r.try_get(3)?),
                kind: AdjustmentKind::parse(&r.try_get::<String, _>(4)?).ok_or_else(|| StorageError::Corrupt("adjustment kind".into()))?,
            });
        }
        for r in sqlx::query(&self.sql(
            "SELECT id, name, kind, target_amount, target_month, line_name, account_id, start_month, starting_amount, sort_order FROM goals WHERE user_id = ? ORDER BY sort_order, id",
        ))
        .bind(user.as_str())
        .fetch_all(&self.pool)
        .await?
        {
            let line: Option<String> = r.try_get(5)?;
            let account: Option<String> = r.try_get(6)?;
            let track = match (account, line) {
                (Some(a), _) => GoalTrack::Account { id: Id::new(a) },
                (None, Some(name)) => GoalTrack::Line { name },
                (None, None) => return Err(StorageError::Corrupt("goal without a line or account".into())),
            };
            w.goals.push(Goal {
                id: Id::new(r.try_get::<String, _>(0)?),
                name: r.try_get(1)?,
                kind: GoalKind::parse(&r.try_get::<String, _>(2)?).ok_or_else(|| StorageError::Corrupt("goal kind".into()))?,
                target_amount: Cents::new(r.try_get(3)?),
                target_month: r.try_get::<Option<String>, _>(4)?.map(|s| parse_date(&s)).transpose()?,
                track,
                start_month: parse_date(&r.try_get::<String, _>(7)?)?,
                starting_amount: Cents::new(r.try_get(8)?),
                sort_order: i32::try_from(r.try_get::<i64, _>(9)?).unwrap_or(0),
            });
        }
        for r in sqlx::query(&self.sql(
            "SELECT id, provider, enrollment_id, institution, access_token, status, last_error, import_from, last_sync, cursor FROM bank_links WHERE user_id = ? ORDER BY created_at, id",
        ))
        .bind(user.as_str())
        .fetch_all(&self.pool)
        .await?
        {
            w.links.push(BankLink {
                id: Id::new(r.try_get::<String, _>(0)?),
                provider: r.try_get(1)?,
                enrollment_id: r.try_get(2)?,
                institution: r.try_get(3)?,
                access_token: r.try_get(4)?,
                status: LinkStatus::parse(&r.try_get::<String, _>(5)?).ok_or_else(|| StorageError::Corrupt("link status".into()))?,
                last_error: r.try_get(6)?,
                import_from: parse_date(&r.try_get::<String, _>(7)?)?,
                last_sync: r.try_get(8)?,
                cursor: r.try_get(9)?,
            });
        }
        Ok(w)
    }

    /// Bank transaction ids already handled for a user.
    pub async fn bank_seen(&self, user: &Id) -> Result<std::collections::HashSet<String>> {
        let rows = sqlx::query(&self.sql("SELECT external_id FROM bank_seen WHERE user_id = ?")).bind(user.as_str()).fetch_all(&self.pool).await?;
        rows.iter().map(|r| r.try_get::<String, _>(0).map_err(StorageError::from)).collect()
    }

    pub async fn mark_bank_seen(&self, user: &Id, ids: &[String]) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let ts = now();
        for id in ids {
            let done: Option<i64> = sqlx::query(&self.sql("SELECT 1 FROM bank_seen WHERE user_id = ? AND external_id = ?"))
                .bind(user.as_str())
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?
                .map(|_| 1);
            if done.is_none() {
                self.exec(&mut tx, "INSERT INTO bank_seen (user_id, external_id, created_at) VALUES (?, ?, ?)", vec![Bind::s(user), Bind::S(id.clone()), Bind::S(ts.clone())]).await?;
            }
        }
        tx.commit().await?;
        Ok(())
    }

    /// A server-wide setting.
    pub async fn setting(&self, name: &str) -> Result<Option<String>> {
        let row = sqlx::query(&self.sql("SELECT value FROM app_settings WHERE name = ?")).bind(name).fetch_optional(&self.pool).await?;
        Ok(row.map(|r| r.try_get::<String, _>(0)).transpose()?)
    }

    /// Saves (or with `None` removes) a server-wide setting.
    pub async fn set_setting(&self, name: &str, value: Option<&str>) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        self.exec(&mut tx, "DELETE FROM app_settings WHERE name = ?", vec![Bind::S(name.to_string())]).await?;
        if let Some(v) = value {
            self.exec(&mut tx, "INSERT INTO app_settings (name, value, updated_at) VALUES (?, ?, ?)", vec![Bind::S(name.to_string()), Bind::S(v.to_string()), Bind::S(now())]).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Every budget owner's id (for the background bank sync).
    pub async fn user_ids(&self) -> Result<Vec<Id>> {
        let rows = sqlx::query("SELECT id FROM users WHERE owner_id IS NULL").fetch_all(&self.pool).await?;
        rows.iter().map(|r| r.try_get::<String, _>(0).map(Id::new).map_err(StorageError::from)).collect()
    }

    /// Replaces the user's wallet (it is small; one transaction).
    pub async fn save_wallet(&self, user: &Id, w: &Wallet) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        self.write_wallet(&mut tx, user, w).await?;
        tx.commit().await?;
        Ok(())
    }

    async fn write_wallet(&self, conn: &mut AnyConnection, user: &Id, w: &Wallet) -> Result<()> {
        let ts = now();
        let uid = user.as_str().to_string();
        self.exec(conn, "DELETE FROM account_adjustments WHERE account_id IN (SELECT id FROM accounts WHERE user_id = ?)", vec![Bind::S(uid.clone())]).await?;
        self.exec(conn, "DELETE FROM goals WHERE user_id = ?", vec![Bind::S(uid.clone())]).await?;
        self.exec(conn, "DELETE FROM accounts WHERE user_id = ?", vec![Bind::S(uid.clone())]).await?;
        self.exec(conn, "DELETE FROM bank_links WHERE user_id = ?", vec![Bind::S(uid.clone())]).await?;
        for a in &w.accounts {
            self.exec(conn, "INSERT INTO accounts (id, user_id, name, kind, sort_order, archived, credit_limit, apr_bp, minimum_payment, reconciled_on, external_id, link_id, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                vec![Bind::s(&a.id), Bind::S(uid.clone()), Bind::S(a.name.clone()), Bind::S(a.kind.as_str().into()), Bind::I(i64::from(a.sort_order)), Bind::I(i64::from(a.archived)),
                     Bind::OI(opt_i64(a.credit_limit)), Bind::OI(a.apr_bp), Bind::OI(opt_i64(a.minimum_payment)), Bind::OS(a.reconciled_on.map(date_str)),
                     Bind::OS(a.external_id.clone()), Bind::OS(opt_str(&a.link_id)), Bind::S(ts.clone()), Bind::S(ts.clone())]).await?;
        }
        for (i, l) in w.links.iter().enumerate() {
            self.exec(conn, "INSERT INTO bank_links (id, user_id, provider, enrollment_id, institution, access_token, status, last_error, import_from, last_sync, cursor, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                vec![Bind::s(&l.id), Bind::S(uid.clone()), Bind::S(l.provider.clone()), Bind::S(l.enrollment_id.clone()), Bind::S(l.institution.clone()), Bind::S(l.access_token.clone()),
                     Bind::S(l.status.as_str().into()), Bind::OS(l.last_error.clone()), Bind::S(date_str(l.import_from)), Bind::OS(l.last_sync.clone()), Bind::OS(l.cursor.clone()), Bind::S(format!("{i:08}"))]).await?;
        }
        for (i, x) in w.adjustments.iter().enumerate() {
            // created_at keeps same-day adjustments in order.
            self.exec(conn, "INSERT INTO account_adjustments (id, account_id, date, amount, kind, created_at) VALUES (?, ?, ?, ?, ?, ?)",
                vec![Bind::s(&x.id), Bind::s(&x.account_id), Bind::S(date_str(x.date)), Bind::I(x.amount.get()), Bind::S(x.kind.as_str().into()), Bind::S(format!("{i:08}"))]).await?;
        }
        for g in &w.goals {
            let (line, account) = match &g.track {
                GoalTrack::Line { name } => (Some(name.clone()), None),
                GoalTrack::Account { id } => (None, Some(id.as_str().to_string())),
            };
            self.exec(conn, "INSERT INTO goals (id, user_id, name, kind, target_amount, target_month, line_name, account_id, start_month, starting_amount, sort_order, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                vec![Bind::s(&g.id), Bind::S(uid.clone()), Bind::S(g.name.clone()), Bind::S(g.kind.as_str().into()), Bind::I(g.target_amount.get()), Bind::OS(g.target_month.map(date_str)),
                     Bind::OS(line), Bind::OS(account), Bind::S(date_str(g.start_month)), Bind::I(g.starting_amount.get()), Bind::I(i64::from(g.sort_order)), Bind::S(ts.clone()), Bind::S(ts.clone())]).await?;
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Diff application
    // ------------------------------------------------------------------

    async fn exec(&self, conn: &mut AnyConnection, sql: &str, binds: Vec<Bind>) -> Result<()> {
        let sql = self.sql(sql);
        let mut q = sqlx::query(&sql);
        for b in binds {
            q = match b {
                Bind::S(s) => q.bind(s),
                Bind::OS(s) => q.bind(s),
                Bind::I(i) => q.bind(i),
                Bind::OI(i) => q.bind(i),
            };
        }
        q.execute(&mut *conn).await.map_err(map_db)?;
        Ok(())
    }

    async fn apply_diff(&self, conn: &mut AnyConnection, before: &Month, after: &Month) -> Result<()> {
        let ts = now();
        let mid = after.id.as_str().to_string();

        // 1. Allocations that disappear go first (they reference everything).
        for id in removed(&before.allocations, &after.allocations, |a| &a.id) {
            self.exec(conn, "DELETE FROM allocations WHERE id = ?", vec![Bind::s(id)]).await?;
        }

        // 2. Parents: categories, lines, income lines, paychecks.
        for (pos, c, new) in changed(&before.categories, &after.categories, |c| &c.id) {
            if new {
                self.exec(conn, "INSERT INTO expense_categories (id, month_id, name, sort_order, kind, position, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                    vec![Bind::s(&c.id), Bind::S(mid.clone()), Bind::S(c.name.clone()), Bind::I(i64::from(c.sort_order)), Bind::S(c.kind.as_str().into()), Bind::I(pos), Bind::S(ts.clone()), Bind::S(ts.clone())]).await?;
            } else {
                self.exec(conn, "UPDATE expense_categories SET name = ?, sort_order = ?, kind = ?, position = ?, updated_at = ? WHERE id = ?",
                    vec![Bind::S(c.name.clone()), Bind::I(i64::from(c.sort_order)), Bind::S(c.kind.as_str().into()), Bind::I(pos), Bind::S(ts.clone()), Bind::s(&c.id)]).await?;
            }
        }
        for (pos, l, new) in changed(&before.expense_lines, &after.expense_lines, |l| &l.id) {
            if new {
                self.exec(conn, "INSERT INTO expense_lines (id, category_id, name, sort_order, current_balance, minimum_payment, target_amount, position, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    vec![Bind::s(&l.id), Bind::s(&l.category_id), Bind::S(l.name.clone()), Bind::I(i64::from(l.sort_order)), Bind::OI(opt_i64(l.current_balance)), Bind::OI(opt_i64(l.minimum_payment)), Bind::OI(opt_i64(l.target_amount)), Bind::I(pos), Bind::S(ts.clone()), Bind::S(ts.clone())]).await?;
            } else {
                self.exec(conn, "UPDATE expense_lines SET category_id = ?, name = ?, sort_order = ?, current_balance = ?, minimum_payment = ?, target_amount = ?, position = ?, updated_at = ? WHERE id = ?",
                    vec![Bind::s(&l.category_id), Bind::S(l.name.clone()), Bind::I(i64::from(l.sort_order)), Bind::OI(opt_i64(l.current_balance)), Bind::OI(opt_i64(l.minimum_payment)), Bind::OI(opt_i64(l.target_amount)), Bind::I(pos), Bind::S(ts.clone()), Bind::s(&l.id)]).await?;
            }
        }
        for (pos, l, new) in changed(&before.income_lines, &after.income_lines, |l| &l.id) {
            let rule = l.recurrence_rule.as_ref().map(Recurrence::to_json);
            if new {
                self.exec(conn, "INSERT INTO income_lines (id, month_id, name, planned_amount, schedule_type, recurrence_rule, position, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    vec![Bind::s(&l.id), Bind::S(mid.clone()), Bind::S(l.name.clone()), Bind::I(l.planned_amount.get()), Bind::S(l.schedule_type().into()), Bind::OS(rule), Bind::I(pos), Bind::S(ts.clone()), Bind::S(ts.clone())]).await?;
            } else {
                self.exec(conn, "UPDATE income_lines SET name = ?, planned_amount = ?, schedule_type = ?, recurrence_rule = ?, position = ?, updated_at = ? WHERE id = ?",
                    vec![Bind::S(l.name.clone()), Bind::I(l.planned_amount.get()), Bind::S(l.schedule_type().into()), Bind::OS(rule), Bind::I(pos), Bind::S(ts.clone()), Bind::s(&l.id)]).await?;
            }
        }
        for (pos, p, new) in changed(&before.paychecks, &after.paychecks, |p| &p.id) {
            if new {
                self.exec(conn, "INSERT INTO paychecks (id, income_line_id, date, planned_amount, actual_amount, status, position, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    vec![Bind::s(&p.id), Bind::s(&p.income_line_id), Bind::S(date_str(p.date)), Bind::I(p.planned_amount.get()), Bind::OI(opt_i64(p.actual_amount)), Bind::S(p.status.as_str().into()), Bind::I(pos), Bind::S(ts.clone()), Bind::S(ts.clone())]).await?;
            } else {
                self.exec(conn, "UPDATE paychecks SET date = ?, planned_amount = ?, actual_amount = ?, status = ?, position = ?, updated_at = ? WHERE id = ?",
                    vec![Bind::S(date_str(p.date)), Bind::I(p.planned_amount.get()), Bind::OI(opt_i64(p.actual_amount)), Bind::S(p.status.as_str().into()), Bind::I(pos), Bind::S(ts.clone()), Bind::s(&p.id)]).await?;
            }
        }

        // 3. Children: allocations and transactions.
        for (pos, a, new) in changed(&before.allocations, &after.allocations, |a| &a.id) {
            if new {
                self.exec(conn, "INSERT INTO allocations (id, expense_line_id, paycheck_id, amount, position, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
                    vec![Bind::s(&a.id), Bind::s(&a.expense_line_id), Bind::s(&a.paycheck_id), Bind::I(a.amount.get()), Bind::I(pos), Bind::S(ts.clone()), Bind::S(ts.clone())]).await?;
            } else {
                self.exec(conn, "UPDATE allocations SET amount = ?, position = ?, updated_at = ? WHERE id = ?",
                    vec![Bind::I(a.amount.get()), Bind::I(pos), Bind::S(ts.clone()), Bind::s(&a.id)]).await?;
            }
        }
        for (pos, t, new) in changed(&before.transactions, &after.transactions, |t| &t.id) {
            if new {
                self.exec(conn, "INSERT INTO transactions (id, month_id, date, amount, payee, notes, expense_line_id, paycheck_id, split_group, account_id, transfer_account_id, external_id, position, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    vec![Bind::s(&t.id), Bind::S(mid.clone()), Bind::S(date_str(t.date)), Bind::I(t.amount.get()), Bind::OS(t.payee.clone()), Bind::OS(t.notes.clone()), Bind::OS(opt_str(&t.expense_line_id)), Bind::OS(opt_str(&t.paycheck_id)), Bind::OS(opt_str(&t.split_group)), Bind::OS(opt_str(&t.account_id)), Bind::OS(opt_str(&t.transfer_account_id)), Bind::OS(t.external_id.clone()), Bind::I(pos), Bind::S(ts.clone()), Bind::S(ts.clone())]).await?;
            } else {
                self.exec(conn, "UPDATE transactions SET date = ?, amount = ?, payee = ?, notes = ?, expense_line_id = ?, paycheck_id = ?, split_group = ?, account_id = ?, transfer_account_id = ?, external_id = ?, position = ?, updated_at = ? WHERE id = ?",
                    vec![Bind::S(date_str(t.date)), Bind::I(t.amount.get()), Bind::OS(t.payee.clone()), Bind::OS(t.notes.clone()), Bind::OS(opt_str(&t.expense_line_id)), Bind::OS(opt_str(&t.paycheck_id)), Bind::OS(opt_str(&t.split_group)), Bind::OS(opt_str(&t.account_id)), Bind::OS(opt_str(&t.transfer_account_id)), Bind::OS(t.external_id.clone()), Bind::I(pos), Bind::S(ts.clone()), Bind::s(&t.id)]).await?;
            }
        }

        // 4. Removed rows, children before parents.
        for id in removed(&before.transactions, &after.transactions, |t| &t.id) {
            self.exec(conn, "DELETE FROM transactions WHERE id = ?", vec![Bind::s(id)]).await?;
        }
        for id in removed(&before.paychecks, &after.paychecks, |p| &p.id) {
            self.exec(conn, "DELETE FROM paychecks WHERE id = ?", vec![Bind::s(id)]).await?;
        }
        for id in removed(&before.expense_lines, &after.expense_lines, |l| &l.id) {
            self.exec(conn, "DELETE FROM expense_lines WHERE id = ?", vec![Bind::s(id)]).await?;
        }
        for id in removed(&before.income_lines, &after.income_lines, |l| &l.id) {
            self.exec(conn, "DELETE FROM income_lines WHERE id = ?", vec![Bind::s(id)]).await?;
        }
        for id in removed(&before.categories, &after.categories, |c| &c.id) {
            self.exec(conn, "DELETE FROM expense_categories WHERE id = ?", vec![Bind::s(id)]).await?;
        }
        Ok(())
    }
}

enum Bind {
    S(String),
    OS(Option<String>),
    I(i64),
    OI(Option<i64>),
}

impl Bind {
    fn s(id: &Id) -> Bind {
        Bind::S(id.as_str().to_string())
    }
}

/// Ids present in `before` but not in `after`.
fn removed<'a, T>(before: &'a [T], after: &[T], id: fn(&T) -> &Id) -> Vec<&'a Id> {
    before.iter().map(id).filter(|i| !after.iter().any(|a| id(a) == *i)).collect()
}

/// Items of `after` that are new or differ (value or position) from `before`.
/// Yields (position, item, is_new).
fn changed<'a, T: PartialEq>(before: &[T], after: &'a [T], id: fn(&T) -> &Id) -> Vec<(i64, &'a T, bool)> {
    after
        .iter()
        .enumerate()
        .filter_map(|(pos, a)| {
            let pos_i = i64::try_from(pos).unwrap_or(i64::MAX);
            match before.iter().position(|b| id(b) == id(a)) {
                None => Some((pos_i, a, true)),
                Some(bp) if bp != pos || before[bp] != *a => Some((pos_i, a, false)),
                Some(_) => None,
            }
        })
        .collect()
}
