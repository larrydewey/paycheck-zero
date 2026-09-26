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
    Router::new()
        .route("/reset", post(reset))
        .route("/bank/state", post(fake_bank::set_state))
        .route("/simplefin/claim/{code}", post(fake_simplefin::claim))
        .route("/simplefin/accounts", axum::routing::get(fake_simplefin::accounts))
        .route("/plaid/link.js", axum::routing::get(fake_plaid::link_js))
        .route("/plaid/link/token/create", post(fake_plaid::link_token))
        .route("/plaid/item/public_token/exchange", post(fake_plaid::exchange))
        .route("/plaid/accounts/get", post(fake_plaid::accounts))
        .route("/plaid/transactions/sync", post(fake_plaid::sync))
}

/// A stand-in for SimpleFIN Bridge (claim + accounts).
mod fake_simplefin {
    use super::fake_bank::DISCONNECTED;
    use axum::http::{header, HeaderMap, StatusCode};
    use axum::response::{IntoResponse, Response};
    use axum::Json;
    use serde_json::json;
    use std::sync::atomic::Ordering;

    pub async fn claim(h: HeaderMap) -> Response {
        let host = h.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("127.0.0.1").to_string();
        format!("http://sfuser:sfpass@{host}/__test/simplefin").into_response()
    }

    fn ts(d: &str) -> i64 {
        chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok().and_then(|x| x.and_hms_opt(12, 0, 0)).map_or(0, |x| x.and_utc().timestamp())
    }

    pub async fn accounts(h: HeaderMap) -> Response {
        use base64::Engine;
        let ok = format!("Basic {}", base64::engine::general_purpose::STANDARD.encode("sfuser:sfpass"));
        if DISCONNECTED.load(Ordering::Relaxed) || h.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) != Some(ok.as_str()) {
            return (StatusCode::FORBIDDEN, "revoked").into_response();
        }
        Json(json!({ "errors": [], "accounts": [
            { "org": { "name": "SF Credit Union", "domain": "sfcu.test" }, "id": "sf_chk", "name": "SF Checking", "currency": "USD",
              "balance": "980.25", "available-balance": "980.25", "balance-date": ts("2026-09-10"),
              "transactions": [
                { "id": "sft_1", "posted": ts("2026-09-05"), "amount": "-18.75", "description": "CHIPOTLE 123", "payee": "Chipotle" },
                { "id": "sft_p", "posted": 0, "amount": "-5.00", "description": "PENDING", "pending": true }
              ] },
            { "org": { "name": "SF Credit Union", "domain": "sfcu.test" }, "id": "sf_card", "name": "SF Visa Card", "currency": "USD",
              "balance": "-310.40", "available-balance": "0", "balance-date": ts("2026-09-10"),
              "transactions": [
                { "id": "sft_2", "posted": ts("2026-09-06"), "amount": "-42.00", "description": "TARGET", "payee": "Target" }
              ] }
        ] }))
        .into_response()
    }
}

/// A stand-in for Plaid (Link script and the endpoints the app calls).
#[allow(clippy::result_large_err)]
mod fake_plaid {
    use super::fake_bank::{DISCONNECTED, EXTRA};
    use axum::http::{header, StatusCode};
    use axum::response::{IntoResponse, Response};
    use axum::Json;
    use serde_json::{json, Value};
    use std::sync::atomic::Ordering;

    pub async fn link_js() -> Response {
        let js = r#"window.Plaid = { create: function (o) { return { open: function () {
  setTimeout(function () { o.onSuccess("public-sandbox-1", { institution: { name: "Test Bank" } }); }, 30);
} }; } };"#;
        ([(header::CONTENT_TYPE, "text/javascript")], js).into_response()
    }

    fn check(body: &Value) -> Result<(), Response> {
        if body["client_id"] != "cid" || body["secret"] != "sec" {
            return Err((StatusCode::BAD_REQUEST, Json(json!({ "error_code": "INVALID_API_KEYS", "error_message": "bad keys" }))).into_response());
        }
        Ok(())
    }

    fn item_ok(body: &Value) -> Result<(), Response> {
        check(body)?;
        if DISCONNECTED.load(Ordering::Relaxed) || body["access_token"] != "access-sandbox-1" {
            return Err((StatusCode::BAD_REQUEST, Json(json!({ "error_code": "ITEM_LOGIN_REQUIRED", "error_message": "login required" }))).into_response());
        }
        Ok(())
    }

    pub async fn link_token(Json(b): Json<Value>) -> Response {
        if let Err(r) = check(&b) {
            return r;
        }
        Json(json!({ "link_token": "link-sandbox-1" })).into_response()
    }

    pub async fn exchange(Json(b): Json<Value>) -> Response {
        if let Err(r) = check(&b) {
            return r;
        }
        Json(json!({ "access_token": "access-sandbox-1", "item_id": "item_1" })).into_response()
    }

    pub async fn accounts(Json(b): Json<Value>) -> Response {
        if let Err(r) = item_ok(&b) {
            return r;
        }
        let checking = if EXTRA.load(Ordering::Relaxed) { 2395.0 } else { 2410.0 };
        Json(json!({ "accounts": [
            { "account_id": "p_chk", "name": "Checking", "mask": "1234", "type": "depository", "subtype": "checking",
              "balances": { "current": checking, "available": checking } },
            { "account_id": "p_card", "name": "Sapphire", "mask": "9876", "type": "credit", "subtype": "credit card",
              "balances": { "current": 250.0 } },
            { "account_id": "p_401k", "name": "401k", "mask": "1111", "type": "investment", "subtype": "401k",
              "balances": { "current": 52000.0 } }
        ] }))
        .into_response()
    }

    /// Plaid amounts are positive for money leaving the account.
    fn tx(id: &str, acct: &str, amount: f64, date: &str, merchant: &str, pending: bool) -> Value {
        json!({ "transaction_id": id, "account_id": acct, "amount": amount, "date": date,
                "name": merchant.to_uppercase(), "merchant_name": merchant, "pending": pending })
    }

    pub async fn sync(Json(b): Json<Value>) -> Response {
        if let Err(r) = item_ok(&b) {
            return r;
        }
        let page = |added: Vec<Value>, next: &str| Json(json!({ "added": added, "modified": [], "removed": [], "next_cursor": next, "has_more": false })).into_response();
        match b["cursor"].as_str() {
            Some("c1") if EXTRA.load(Ordering::Relaxed) => page(vec![tx("pt_chk6", "p_chk", 15.0, "2026-09-10", "Chipotle", false)], "c2"),
            Some(c) => page(Vec::new(), c),
            None => page(
                vec![
                    tx("pt_pend", "p_chk", 9.99, "2026-09-10", "Netflix", true),
                    tx("pt_chk4", "p_chk", 23.50, "2026-09-09", "Trader Joe's", false),
                    tx("pt_chk3", "p_chk", 120.0, "2026-09-08", "Payment to Sapphire", false),
                    tx("pt_chk2", "p_chk", 40.0, "2026-09-06", "Shell", false),
                    tx("pt_chk1", "p_chk", -1950.0, "2026-09-04", "Acme Payroll", false),
                    tx("pt_old", "p_chk", 5.0, "2026-08-20", "Old", false),
                    tx("pt_card3", "p_card", 64.10, "2026-09-09", "Target", false),
                    tx("pt_card2", "p_card", -120.0, "2026-09-08", "Payment received", false),
                    tx("pt_card1", "p_card", 12.0, "2026-09-07", "Coffee", false),
                ],
                "c1",
            ),
        }
    }
}

/// State shared by the fake providers: a "disconnected" bank, and one
/// extra transaction for testing incremental syncs.
mod fake_bank {
    use axum::http::StatusCode;
    use axum::Json;
    use serde::Deserialize;
    use std::sync::atomic::{AtomicBool, Ordering};

    pub static DISCONNECTED: AtomicBool = AtomicBool::new(false);
    pub static EXTRA: AtomicBool = AtomicBool::new(false);

    pub fn reset() {
        DISCONNECTED.store(false, Ordering::Relaxed);
        EXTRA.store(false, Ordering::Relaxed);
    }

    #[derive(Deserialize)]
    pub struct StateReq {
        #[serde(default)]
        disconnected: bool,
        #[serde(default)]
        extra: bool,
    }

    pub async fn set_state(Json(r): Json<StateReq>) -> StatusCode {
        DISCONNECTED.store(r.disconnected, Ordering::Relaxed);
        EXTRA.store(r.extra, Ordering::Relaxed);
        StatusCode::NO_CONTENT
    }
}

#[derive(Deserialize)]
struct ResetReq {
    #[serde(default)]
    seed: Option<String>,
    #[serde(default)]
    today: Option<NaiveDate>,
    #[serde(default)]
    access_ttl: Option<i64>,
    /// Ignore provider environment variables (to test Settings → Bank providers).
    #[serde(default)]
    no_env_providers: bool,
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
        split_group: None,
        account_id: None,
        transfer_account_id: None,
        external_id: None,
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
    fake_bank::reset();
    st.set_ignore_env_providers(r.no_env_providers);
    st.load_providers().await;
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
    let mut wallet: Option<Wallet> = None;
    match seed.as_str() {
        "user" => {}
        "wallet" => {
            // Basic month plus accounts, a credit card and two goals.
            let (mut m, pcs) = basic_month(d(2026, 9, 1), d(2026, 9, 4))?;
            let mut w = Wallet::default();
            let opened = d(2026, 9, 1);
            let checking = w.add_account("Checking", AccountKind::Checking, usd(2_450), opened, None)?;
            w.add_account("Savings", AccountKind::Savings, usd(5_000), opened, None)?;
            let visa = w.add_account("Visa", AccountKind::CreditCard, usd(820), opened, Some(usd(5_000)))?;
            let k401 = w.add_account("401(k)", AccountKind::Retirement, usd(42_000), opened, None)?;
            w.add_contribution(&k401, usd(300), d(2026, 9, 4))?;
            w.set_card_details(&visa, Some(usd(5_000)), Some(2_199), Some(usd(35)))?;
            for t in &mut m.transactions {
                t.account_id = Some(if t.payee.as_deref() == Some("Shell") { checking.clone() } else { visa.clone() });
            }
            w.add_goal(Goal {
                id: Id::generate(),
                name: "Emergency fund".into(),
                kind: GoalKind::Save,
                target_amount: usd(5_000),
                target_month: Some(d(2027, 6, 1)),
                track: GoalTrack::Line { name: "Emergency Fund".into() },
                start_month: d(2026, 9, 1),
                starting_amount: usd(1_000),
                sort_order: 0,
            })?;
            let debt = w.debt_now(&GoalTrack::Account { id: visa.clone() }, std::slice::from_ref(&m), d(2026, 9, 1));
            w.add_goal(Goal {
                id: Id::generate(),
                name: "Visa paid off".into(),
                kind: GoalKind::Payoff,
                target_amount: debt,
                target_month: Some(d(2027, 3, 1)),
                track: GoalTrack::Account { id: visa },
                start_month: d(2026, 9, 1),
                starting_amount: Cents::ZERO,
                sort_order: 0,
            })?;
            months.push((m, pcs));
            out["accounts"] = json!(w.accounts.iter().map(|a| (a.name.clone(), a.id.clone())).collect::<std::collections::BTreeMap<_, _>>());
            wallet = Some(w);
        }
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
    if let Some(w) = &wallet {
        st.store.save_wallet(&user.id, w).await?;
    }
    if let Some((m, _)) = months.first() {
        st.store.set_last_month(&user.id, Some(&m.id)).await?;
    }
    Ok(Json(out))
}
