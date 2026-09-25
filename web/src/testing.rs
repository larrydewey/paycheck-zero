//! Test-mode endpoints (`PZ_TEST_MODE=1` only): full database reset plus a
//! deterministic seed for every Playwright suite (spec §13.1), and a pinned
//! clock so "today" is stable.

use crate::auth::hash_password;
use crate::error::{AppError, AppResult};
use crate::Shared;
use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use chrono::NaiveDate;
use paycheckzero_core::*;
use paycheckzero_storage::UserRecord;
use serde::Deserialize;
use serde_json::{json, Value};

pub const EMAIL: &str = "demo@paycheckzero.test";
pub const PASSWORD: &str = "correct-horse-battery";

pub fn routes() -> Router<Shared> {
    Router::new().route("/reset", post(reset))
}

#[derive(Deserialize)]
struct ResetReq {
    #[serde(default)]
    seed: Option<String>,
    #[serde(default)]
    today: Option<NaiveDate>,
    #[serde(default)]
    access_ttl: Option<i64>,
}

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap_or_default()
}

fn usd(dollars: i64) -> Cents {
    Cents::new(dollars * 100)
}

fn cat(m: &Month, name: &str) -> Id {
    m.categories.iter().find(|c| c.name == name).map(|c| c.id.clone()).unwrap_or_default()
}

fn tx(date: NaiveDate, cents: i64, payee: &str, line: Option<&Id>, paycheck: Option<&Id>) -> Transaction {
    Transaction {
        id: Id::generate(),
        date,
        amount: Cents::new(cents),
        payee: Some(payee.into()),
        notes: None,
        expense_line_id: line.cloned(),
        paycheck_id: paycheck.cloned(),
    }
}

/// The standard seeded month: two $2,000 paychecks, six lines, partly funded.
fn basic_month(ym: NaiveDate, anchor: NaiveDate) -> Result<(Month, Vec<Id>), DomainError> {
    let mut m = Month::create(Id::generate(), ym, CopyMode::Blank, None);
    m.add_income_line("Acme Payroll", usd(2_000), Schedule::Recurring { recurrence_rule: Recurrence::Biweekly { anchor } })?;
    let pcs: Vec<Id> = m.paychecks_by_date().iter().map(|p| p.id.clone()).collect();
    let rent = m.add_expense_line(&cat(&m, "Housing"), "Rent")?;
    let electric = m.add_expense_line(&cat(&m, "Housing"), "Electric")?;
    let groceries = m.add_expense_line(&cat(&m, "Food"), "Groceries")?;
    let gas = m.add_expense_line(&cat(&m, "Transportation"), "Gas")?;
    let visa = m.add_expense_line(&cat(&m, "Debt"), "Visa")?;
    m.set_debt_fields(&visa, Some(usd(3_200)), Some(usd(75)))?;
    m.add_expense_line(&cat(&m, "Saving"), "Emergency Fund")?;
    m.set_allocation(&pcs[0], &rent, usd(1_200))?;
    m.set_allocation(&pcs[0], &groceries, usd(300))?;
    m.set_allocation(&pcs[0], &gas, usd(100))?;
    m.set_allocation(&pcs[1], &groceries, usd(300))?;
    m.set_allocation(&pcs[1], &electric, usd(150))?;
    m.set_allocation(&pcs[1], &visa, usd(75))?;
    let first = pcs[0].date_hint(&m);
    m.add_transaction(tx(first + chrono::Duration::days(1), -8_520, "Trader Joe's", Some(&groceries), Some(&pcs[0])))?;
    m.add_transaction(tx(first + chrono::Duration::days(2), -4_000, "Shell", Some(&gas), None))?;
    m.add_transaction(tx(first + chrono::Duration::days(3), -1_200, "Coffee", None, Some(&pcs[0])))?;
    Ok((m, pcs))
}

trait DateHint {
    fn date_hint(&self, m: &Month) -> NaiveDate;
}

impl DateHint for Id {
    fn date_hint(&self, m: &Month) -> NaiveDate {
        m.paycheck(self).map_or(m.year_month, |p| p.date)
    }
}

fn balance(m: &mut Month) -> Result<(), DomainError> {
    let ef = m.expense_lines.iter().find(|l| l.name == "Emergency Fund").map(|l| l.id.clone()).unwrap_or_default();
    for p in m.paychecks_by_date().iter().map(|p| p.id.clone()).collect::<Vec<_>>() {
        let free = m.paycheck_unallocated(&p);
        if free.is_positive() {
            let have = m.allocation_for(&p, &ef).map_or(Cents::ZERO, |a| a.amount);
            m.set_allocation(&p, &ef, have + free)?;
        }
    }
    Ok(())
}

async fn seed_user(st: &Shared) -> AppResult<UserRecord> {
    let hash = hash_password(PASSWORD)?;
    Ok(st.store.create_user(EMAIL, &hash, "America/Chicago", "USD").await?)
}

async fn reset(State(st): State<Shared>, Json(r): Json<ResetReq>) -> AppResult<Json<Value>> {
    st.store.reset().await?;
    st.refresh_grace.lock().await.clear();
    st.clock.set_fixed(Some(r.today.unwrap_or(d(2026, 9, 10))));
    st.set_access_ttl(r.access_ttl.unwrap_or(crate::auth::ACCESS_TTL_SECS));
    let seed = r.seed.unwrap_or_else(|| "empty".into());
    let mut out = json!({ "seed": seed, "email": EMAIL, "password": PASSWORD, "months": {} });
    if seed == "empty" {
        return Ok(Json(out));
    }
    let user = seed_user(&st).await?;
    let mut months: Vec<(Month, Vec<Id>)> = Vec::new();
    match seed.as_str() {
        "user" => {}
        "basic" | "balanced" | "locked" | "history" => {
            let (mut m, pcs) = basic_month(d(2026, 9, 1), d(2026, 9, 4))?;
            if seed != "basic" {
                balance(&mut m)?;
            }
            if seed == "locked" {
                m.lock()?;
            }
            months.push((m, pcs));
            if seed == "history" {
                let (mut aug, apcs) = basic_month(d(2026, 8, 1), d(2026, 8, 7))?;
                balance(&mut aug)?;
                aug.lock()?;
                aug.set_paycheck_actual(&apcs[0], Some(usd(2_000)))?;
                aug.set_paycheck_actual(&apcs[1], Some(usd(2_050)))?;
                let rent = aug.expense_lines.iter().find(|l| l.name == "Rent").map(|l| l.id.clone());
                aug.add_transaction(tx(d(2026, 8, 1), -120_000, "Landlord", rent.as_ref(), None))?;
                months.push((aug, apcs));

                let mut old = Month::create(Id::generate(), d(2025, 9, 1), CopyMode::Blank, None);
                old.add_income_line("Old Job", usd(3_000), Schedule::OneOff { dates: vec![d(2025, 9, 1)] })?;
                let p = old.paychecks[0].id.clone();
                let rent = old.add_expense_line(&cat(&old, "Housing"), "Rent")?;
                let groceries = old.add_expense_line(&cat(&old, "Food"), "Groceries")?;
                let misc = old.add_expense_line(&cat(&old, "Other"), "Misc")?;
                old.set_allocation(&p, &rent, usd(1_000))?;
                old.set_allocation(&p, &groceries, usd(500))?;
                old.set_allocation(&p, &misc, usd(1_500))?;
                old.set_paycheck_actual(&p, Some(usd(3_000)))?;
                old.add_transaction(tx(d(2025, 9, 3), -45_000, "Market", Some(&groceries), None))?;
                old.lock()?;
                months.push((old, vec![p]));
            }
        }
        other => return Err(AppError::bad(format!("unknown seed {other}"))),
    }
    for (m, pcs) in &months {
        m.check_invariants()?;
        st.store.insert_month(&user.id, m).await?;
        out["months"][m.year_month.format("%Y-%m").to_string()] = json!({
            "id": m.id,
            "paychecks": pcs,
            "lines": m.expense_lines.iter().map(|l| (l.name.clone(), l.id.clone())).collect::<std::collections::BTreeMap<_, _>>(),
        });
    }
    if let Some((m, _)) = months.first() {
        st.store.set_last_month(&user.id, Some(&m.id)).await?;
    }
    Ok(Json(out))
}
