//! SQLite-backed implementation of [`Repository`].

use paycheckzero_core::{
    Allocation, Cents, ExpenseCategory, ExpenseLine, Id, IncomeLine, Month,
    MonthStatus, Paycheck, PaycheckStatus, ScheduleType, Transaction,
};
use rusqlite::{params, Connection, OptionalExtension, Result as SqlResult};

use super::repo::{MonthListItem, Repository, StorageResult};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS months (
    id         TEXT PRIMARY KEY,
    year_month TEXT NOT NULL,
    status     TEXT NOT NULL CHECK (status IN ('draft', 'locked')),
    archived   INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0, 1))
);

CREATE TABLE IF NOT EXISTS income_lines (
    id              TEXT PRIMARY KEY,
    month_id        TEXT NOT NULL REFERENCES months(id),
    name            TEXT NOT NULL,
    planned_amount  INTEGER NOT NULL,
    schedule_type   TEXT NOT NULL CHECK (schedule_type IN ('one_off', 'recurring')),
    recurrence_rule TEXT
);

CREATE TABLE IF NOT EXISTS paychecks (
    id               TEXT PRIMARY KEY,
    month_id         TEXT NOT NULL REFERENCES months(id),
    income_line_id   TEXT NOT NULL,
    date             TEXT NOT NULL,
    planned_amount   INTEGER NOT NULL,
    actual_amount    INTEGER,
    status           TEXT NOT NULL CHECK (status IN ('planned', 'received', 'skipped'))
);

CREATE TABLE IF NOT EXISTS categories (
    id         TEXT PRIMARY KEY,
    month_id   TEXT NOT NULL REFERENCES months(id),
    name       TEXT NOT NULL,
    sort_order INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS expense_lines (
    id                TEXT PRIMARY KEY,
    month_id          TEXT NOT NULL REFERENCES months(id),
    category_id       TEXT NOT NULL REFERENCES categories(id),
    name              TEXT NOT NULL,
    current_balance   INTEGER,
    minimum_payment   INTEGER
);

CREATE TABLE IF NOT EXISTS allocations (
    id                TEXT PRIMARY KEY,
    paycheck_id       TEXT NOT NULL REFERENCES paychecks(id),
    expense_line_id   TEXT NOT NULL REFERENCES expense_lines(id),
    amount            INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS transactions (
    id                TEXT PRIMARY KEY,
    month_id          TEXT NOT NULL REFERENCES months(id),
    date              TEXT NOT NULL,
    amount            INTEGER NOT NULL,
    payee             TEXT,
    notes             TEXT,
    expense_line_id   TEXT REFERENCES expense_lines(id),
    paycheck_id       TEXT REFERENCES paychecks(id)
);
";

/// SQLite storage backend.
pub struct SqliteRepository {
    conn: Connection,
}

impl SqliteRepository {
    /// Open a new SQLite connection at the given path and initialise schema.
    pub fn new(path: &str) -> StorageResult<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(SCHEMA)?;
        Ok(SqliteRepository { conn })
    }
}

fn to_date_str(d: chrono::NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

fn parse_date(s: &str) -> chrono::NaiveDate {
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").expect("valid date string from DB")
}

fn parse_cents_opt(v: Option<i64>) -> Option<Cents> {
    v.map(Cents::from_cents)
}

fn status_to_str(s: MonthStatus) -> &'static str {
    match s {
        MonthStatus::Draft => "draft",
        MonthStatus::Locked => "locked",
    }
}

fn str_to_status(s: &str) -> MonthStatus {
    match s {
        "draft" => MonthStatus::Draft,
        "locked" => MonthStatus::Locked,
        _ => panic!("unknown month status: {s}"),
    }
}

fn paycheck_status_to_str(s: PaycheckStatus) -> &'static str {
    match s {
        PaycheckStatus::Planned => "planned",
        PaycheckStatus::Received => "received",
        PaycheckStatus::Skipped => "skipped",
    }
}

fn str_to_paycheck_status(s: &str) -> PaycheckStatus {
    match s {
        "planned" => PaycheckStatus::Planned,
        "received" => PaycheckStatus::Received,
        "skipped" => PaycheckStatus::Skipped,
        _ => panic!("unknown paycheck status: {s}"),
    }
}

fn schedule_to_str(s: ScheduleType) -> &'static str {
    match s {
        ScheduleType::OneOff => "one_off",
        ScheduleType::Recurring => "recurring",
    }
}

fn str_to_schedule(s: &str) -> ScheduleType {
    match s {
        "one_off" => ScheduleType::OneOff,
        "recurring" => ScheduleType::Recurring,
        _ => panic!("unknown schedule_type: {s}"),
    }
}

fn load_income_lines(conn: &Connection, month_id: &str) -> SqlResult<Vec<IncomeLine>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, planned_amount, schedule_type, recurrence_rule FROM income_lines WHERE month_id = ?1",
    )?;
    let iter = stmt.query_map(params![month_id], |row| -> SqlResult<IncomeLine> {
        Ok(IncomeLine {
            id: Id::new(row.get::<_, String>(0)?),
            name: row.get::<_, String>(1)?,
            planned_amount: Cents::from_cents(row.get::<_, i64>(2)?),
            schedule_type: str_to_schedule(&row.get::<_, String>(3)?),
            recurrence_rule: row.get::<_, Option<String>>(4)?,
        })
    })?;
    iter.collect()
}

fn load_paychecks(conn: &Connection, month_id: &str) -> SqlResult<Vec<Paycheck>> {
    let mut stmt = conn.prepare(
        "SELECT id, income_line_id, date, planned_amount, actual_amount, status FROM paychecks WHERE month_id = ?1",
    )?;
    let iter = stmt.query_map(params![month_id], |row| -> SqlResult<Paycheck> {
        Ok(Paycheck {
            id: Id::new(row.get::<_, String>(0)?),
            income_line_id: Id::new(row.get::<_, String>(1)?),
            date: parse_date(&row.get::<_, String>(2)?),
            planned_amount: Cents::from_cents(row.get::<_, i64>(3)?),
            actual_amount: parse_cents_opt(row.get::<_, Option<i64>>(4)?),
            status: str_to_paycheck_status(&row.get::<_, String>(5)?),
        })
    })?;
    iter.collect()
}

fn load_categories(conn: &Connection, month_id: &str) -> SqlResult<Vec<ExpenseCategory>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, sort_order FROM categories WHERE month_id = ?1",
    )?;
    let iter = stmt.query_map(params![month_id], |row| -> SqlResult<ExpenseCategory> {
        Ok(ExpenseCategory {
            id: Id::new(row.get::<_, String>(0)?),
            name: row.get::<_, String>(1)?,
            sort_order: row.get::<_, i32>(2)?,
        })
    })?;
    iter.collect()
}

fn load_expense_lines(conn: &Connection, month_id: &str) -> SqlResult<Vec<ExpenseLine>> {
    let mut stmt = conn.prepare(
        "SELECT id, category_id, name, current_balance, minimum_payment FROM expense_lines WHERE month_id = ?1",
    )?;
    let iter = stmt.query_map(params![month_id], |row| -> SqlResult<ExpenseLine> {
        Ok(ExpenseLine {
            id: Id::new(row.get::<_, String>(0)?),
            category_id: Id::new(row.get::<_, String>(1)?),
            name: row.get::<_, String>(2)?,
            current_balance: parse_cents_opt(row.get::<_, Option<i64>>(3)?),
            minimum_payment: parse_cents_opt(row.get::<_, Option<i64>>(4)?),
        })
    })?;
    iter.collect()
}

fn load_allocations(conn: &Connection, month_id: &str) -> SqlResult<Vec<Allocation>> {
    let mut stmt = conn.prepare(
        "SELECT id, paycheck_id, expense_line_id, amount FROM allocations WHERE expense_line_id IN (SELECT id FROM expense_lines WHERE month_id = ?1)",
    )?;
    let iter = stmt.query_map(params![month_id], |row| -> SqlResult<Allocation> {
        Ok(Allocation {
            id: Id::new(row.get::<_, String>(0)?),
            paycheck_id: Id::new(row.get::<_, String>(1)?),
            expense_line_id: Id::new(row.get::<_, String>(2)?),
            amount: Cents::from_cents(row.get::<_, i64>(3)?),
        })
    })?;
    iter.collect()
}

fn load_transactions(conn: &Connection, month_id: &str) -> SqlResult<Vec<Transaction>> {
    let mut stmt = conn.prepare(
        "SELECT id, date, amount, payee, notes, expense_line_id, paycheck_id FROM transactions WHERE month_id = ?1",
    )?;
    let iter = stmt.query_map(params![month_id], |row| -> SqlResult<Transaction> {
        Ok(Transaction {
            id: Id::new(row.get::<_, String>(0)?),
            date: parse_date(&row.get::<_, String>(1)?),
            amount: Cents::from_cents(row.get::<_, i64>(2)?),
            payee: row.get::<_, Option<String>>(3)?,
            notes: row.get::<_, Option<String>>(4)?,
            expense_line_id: row.get::<_, Option<String>>(5)?.map(Id::new),
            paycheck_id: row.get::<_, Option<String>>(6)?.map(Id::new),
        })
    })?;
    iter.collect()
}

impl Repository for SqliteRepository {
    fn load_month(&self, id: &Id) -> StorageResult<Option<Month>> {
        let row = self.conn.query_row(
            "SELECT id, year_month, status, archived FROM months WHERE id = ?1",
            params![id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i32>(3)? != 0,
                ))
            },
        );

        let (id_s, ym_s, status_s, archived) = match row.optional()? {
            Some(r) => r,
            None => return Ok(None),
        };

        let status = str_to_status(&status_s);
        let month_id = id_s.clone();
        let id = Id::new(id_s);

        Ok(Some(Month {
            id,
            year_month: parse_date(&ym_s),
            status,
            archived,
            income_lines: load_income_lines(&self.conn, &month_id)?,
            paychecks: load_paychecks(&self.conn, &month_id)?,
            categories: load_categories(&self.conn, &month_id)?,
            expense_lines: load_expense_lines(&self.conn, &month_id)?,
            allocations: load_allocations(&self.conn, &month_id)?,
            transactions: load_transactions(&self.conn, &month_id)?,
        }))
    }

    fn save_month(&mut self, month: &Month) -> StorageResult<()> {
        let mid = month.id.as_str();
        let ym = to_date_str(month.year_month);
        let st = status_to_str(month.status);
        let tx = self.conn.transaction()?;

        tx.execute("DELETE FROM transactions WHERE month_id = ?1", params![mid])?;
        tx.execute(
            "DELETE FROM allocations WHERE expense_line_id IN (SELECT id FROM expense_lines WHERE month_id = ?1)",
            params![mid],
        )?;
        tx.execute("DELETE FROM paychecks WHERE month_id = ?1", params![mid])?;
        tx.execute("DELETE FROM expense_lines WHERE month_id = ?1", params![mid])?;
        tx.execute("DELETE FROM income_lines WHERE month_id = ?1", params![mid])?;
        tx.execute("DELETE FROM categories WHERE month_id = ?1", params![mid])?;

        tx.execute(
            "INSERT OR REPLACE INTO months (id, year_month, status, archived) VALUES (?1, ?2, ?3, ?4)",
            params![mid, &ym, st, month.archived as i32],
        )?;

        for il in &month.income_lines {
            tx.execute(
                "INSERT OR REPLACE INTO income_lines (id, month_id, name, planned_amount, schedule_type, recurrence_rule) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![il.id.as_str(), mid, &il.name, il.planned_amount.as_cents(), schedule_to_str(il.schedule_type), &il.recurrence_rule],
            )?;
        }

        for p in &month.paychecks {
            tx.execute(
                "INSERT OR REPLACE INTO paychecks (id, month_id, income_line_id, date, planned_amount, actual_amount, status) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![p.id.as_str(), mid, p.income_line_id.as_str(), to_date_str(p.date), p.planned_amount.as_cents(), p.actual_amount.map(|a| a.as_cents()), paycheck_status_to_str(p.status)],
            )?;
        }

        for c in &month.categories {
            tx.execute(
                "INSERT OR REPLACE INTO categories (id, month_id, name, sort_order) VALUES (?1, ?2, ?3, ?4)",
                params![c.id.as_str(), mid, &c.name, c.sort_order],
            )?;
        }

        for el in &month.expense_lines {
            tx.execute(
                "INSERT OR REPLACE INTO expense_lines (id, month_id, category_id, name, current_balance, minimum_payment) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![el.id.as_str(), mid, el.category_id.as_str(), &el.name, el.current_balance.map(|b| b.as_cents()), el.minimum_payment.map(|m| m.as_cents())],
            )?;
        }

        for a in &month.allocations {
            tx.execute(
                "INSERT OR REPLACE INTO allocations (id, paycheck_id, expense_line_id, amount) VALUES (?1, ?2, ?3, ?4)",
                params![a.id.as_str(), a.paycheck_id.as_str(), a.expense_line_id.as_str(), a.amount.as_cents()],
            )?;
        }

        for t in &month.transactions {
            tx.execute(
                "INSERT OR REPLACE INTO transactions (id, month_id, date, amount, payee, notes, expense_line_id, paycheck_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![t.id.as_str(), mid, to_date_str(t.date), t.amount.as_cents(), &t.payee, &t.notes, t.expense_line_id.as_ref().map(|i| i.as_str()), t.paycheck_id.as_ref().map(|i| i.as_str())],
            )?;
        }

        tx.commit()?;
        Ok(())
    }

    fn list_months(&self) -> StorageResult<Vec<MonthListItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, year_month, status, archived FROM months ORDER BY year_month DESC",
        )?;
        let rows: Vec<MonthListItem> = stmt
            .query_map(params![], |row| Ok(MonthListItem {
                id: Id::new(row.get::<_, String>(0)?),
                year_month_str: row.get::<_, String>(1)?,
                status_str: row.get::<_, String>(2)?,
                archived: row.get::<_, i32>(3)? != 0,
            }))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}
