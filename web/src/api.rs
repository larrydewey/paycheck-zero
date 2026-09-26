//! REST API (spec §9), base `/api/v1`. Money is always integer cents.
//! Mutations re-validate invariants and answer `409 Conflict` when a rule
//! would be broken.

use crate::auth::{self, AuthUser};
use crate::error::{AppError, AppResult};
use crate::export;
use crate::service::Snapshot;
use crate::Shared;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use chrono::NaiveDate;
use paycheckzero_core::report;
use paycheckzero_core::suggest::{suggest_income, variance_suggestions};
use paycheckzero_core::*;
use paycheckzero_storage::Owner;
use serde::{Deserialize, Deserializer};
use serde_json::{json, Value};

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/auth/register", post(register))
        .route("/auth/login", post(login))
        .route("/auth/refresh", post(refresh))
        .route("/auth/logout", post(logout))
        .route("/auth/logout-all", post(logout_all))
        .route("/months", get(list_months).post(create_month))
        .route("/months/import", post(import_month))
        .route("/months/{id}", get(get_month).delete(delete_month))
        .route("/months/{id}/lock", post(lock_month))
        .route("/months/{id}/archive", post(archive_month))
        .route("/months/{id}/restore", post(restore_month))
        .route("/months/{id}/reassignment/begin", post(begin_reassign))
        .route("/months/{id}/reassignment/finish", post(finish_reassign))
        .route("/months/{id}/income-lines", get(list_income).post(create_income))
        .route("/months/{id}/income-suggestions", get(income_suggestions))
        .route("/income-lines/{id}", patch(patch_income).delete(delete_income))
        .route("/months/{id}/paychecks", get(list_paychecks).post(add_paycheck))
        .route("/paychecks/{id}", patch(patch_paycheck).delete(delete_paycheck))
        .route("/paychecks/{id}/apply-actual", post(apply_actual))
        .route("/months/{id}/categories", get(list_categories).post(create_category))
        .route("/categories/{id}", patch(patch_category).delete(delete_category))
        .route("/months/{id}/expense-lines", get(list_lines).post(create_line))
        .route("/expense-lines/{id}", patch(patch_line).delete(delete_line))
        .route("/paychecks/{id}/allocations", get(list_allocations).post(create_allocation))
        .route("/allocations/{id}", patch(patch_allocation).delete(delete_allocation))
        .route("/allocations/transfer", post(transfer))
        .route("/months/{id}/transactions", get(list_transactions).post(create_transaction))
        .route("/transactions/{id}", patch(patch_transaction).delete(delete_transaction))
        .route("/paychecks/{id}/safe-to-spend", get(safe_to_spend))
        .route("/months/{id}/summary", get(summary))
        .route("/months/{id}/variance", get(variance))
        .route("/months/{id}/export", get(export_csv))
        .route("/months/{id}/snapshot", get(snapshot))
        .route("/months/{id}/reports/mom", get(report_mom))
        .route("/months/{id}/reports/ytd", get(report_ytd))
        .route("/months/{id}/reports/yoy", get(report_yoy))
        .route("/months/{id}/reports/cards", get(report_cards))
}

type J = AppResult<Json<Value>>;

fn ok(v: impl serde::Serialize) -> J {
    Ok(Json(serde_json::to_value(v).map_err(|e| AppError::Internal(e.to_string()))?))
}

fn created(v: impl serde::Serialize) -> AppResult<Response> {
    Ok((StatusCode::CREATED, Json(serde_json::to_value(v).map_err(|e| AppError::Internal(e.to_string()))?)).into_response())
}

/// Distinguishes "absent" from explicit `null` for nullable PATCH fields.
fn double_option<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}

fn cents(v: i64) -> Cents {
    Cents::new(v)
}

pub fn parse_year_month(s: &str) -> AppResult<NaiveDate> {
    let s = s.trim();
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .or_else(|_| NaiveDate::parse_from_str(&format!("{s}-01"), "%Y-%m-%d"))
        .map(paycheckzero_core::recurrence::first_of_month)
        .map_err(|_| AppError::bad("year_month must be YYYY-MM or YYYY-MM-DD"))
}

// ---------------------------------------------------------------- auth

#[derive(Deserialize)]
struct RegisterReq {
    email: String,
    password: String,
    #[serde(default)]
    timezone: Option<String>,
}

async fn register(State(st): State<Shared>, Json(r): Json<RegisterReq>) -> AppResult<Response> {
    let user = st.register(&r.email, &r.password, r.timezone.as_deref().unwrap_or("UTC")).await?;
    let tokens = auth::issue(&st, &user).await?;
    created(tokens)
}

#[derive(Deserialize)]
struct LoginReq {
    email: String,
    password: String,
}

async fn login(State(st): State<Shared>, Json(r): Json<LoginReq>) -> J {
    let user = st.login(&r.email, &r.password).await?;
    ok(auth::issue(&st, &user).await?)
}

#[derive(Deserialize)]
struct RefreshReq {
    refresh_token: String,
}

async fn refresh(State(st): State<Shared>, Json(r): Json<RefreshReq>) -> J {
    let (_, tokens) = auth::rotate(&st, &r.refresh_token).await?;
    ok(tokens)
}

async fn logout(State(st): State<Shared>, Json(r): Json<RefreshReq>) -> AppResult<StatusCode> {
    auth::revoke(&st, &r.refresh_token).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn logout_all(State(st): State<Shared>, user: AuthUser) -> AppResult<StatusCode> {
    auth::revoke_all(&st, user.id()).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------- months

#[derive(Deserialize)]
struct ListQuery {
    #[serde(default)]
    include_archived: bool,
}

async fn list_months(State(st): State<Shared>, user: AuthUser, Query(q): Query<ListQuery>) -> J {
    let months = st.store.list_months(user.id(), q.include_archived).await?;
    ok(months
        .iter()
        .map(|m| {
            json!({
                "id": m.id, "year_month": m.year_month, "status": m.status,
                "reassigning": m.reassigning, "archived": m.archived,
            })
        })
        .collect::<Vec<_>>())
}

#[derive(Deserialize)]
struct CreateMonthReq {
    year_month: String,
    #[serde(default)]
    copy_mode: Option<String>,
    #[serde(default)]
    source_month_id: Option<Id>,
}

async fn create_month(State(st): State<Shared>, user: AuthUser, Json(r): Json<CreateMonthReq>) -> AppResult<Response> {
    let ym = parse_year_month(&r.year_month)?;
    let mode = match r.copy_mode.as_deref() {
        None => CopyMode::Blank,
        Some(s) => CopyMode::parse(s).ok_or_else(|| AppError::bad("copy_mode must be blank, structure or structure_and_planned"))?,
    };
    let m = st.create_month(&user.0, ym, mode, r.source_month_id.as_ref()).await?;
    created(m.view(st.today_for(&user.0)))
}

async fn get_month(State(st): State<Shared>, user: AuthUser, Path(id): Path<Id>) -> J {
    let l = st.load(&user.0, &id).await?;
    let mut v = serde_json::to_value(l.month.view(st.today_for(&user.0))).map_err(|e| AppError::Internal(e.to_string()))?;
    v["archived"] = json!(l.archived);
    Ok(Json(v))
}

#[derive(Deserialize)]
struct DeleteQuery {
    confirm: String,
}

async fn delete_month(State(st): State<Shared>, user: AuthUser, Path(id): Path<Id>, Query(q): Query<DeleteQuery>) -> AppResult<StatusCode> {
    st.delete_month(&user.0, &id, &q.confirm).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn month_op(st: &Shared, user: &AuthUser, id: &Id, f: impl FnOnce(&mut Month) -> Result<(), DomainError>) -> J {
    let (_, m) = st.mutate(&user.0, id, f).await?;
    ok(m.view(st.today_for(&user.0)))
}

async fn lock_month(State(st): State<Shared>, user: AuthUser, Path(id): Path<Id>) -> J {
    month_op(&st, &user, &id, Month::lock).await
}

async fn begin_reassign(State(st): State<Shared>, user: AuthUser, Path(id): Path<Id>) -> J {
    month_op(&st, &user, &id, Month::begin_reassignment).await
}

async fn finish_reassign(State(st): State<Shared>, user: AuthUser, Path(id): Path<Id>) -> J {
    month_op(&st, &user, &id, Month::finish_reassignment).await
}

async fn archive_month(State(st): State<Shared>, user: AuthUser, Path(id): Path<Id>) -> AppResult<StatusCode> {
    st.archive(&user.0, &id, true).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn restore_month(State(st): State<Shared>, user: AuthUser, Path(id): Path<Id>) -> AppResult<StatusCode> {
    st.archive(&user.0, &id, false).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct ImportReq {
    snapshot: Snapshot,
    #[serde(default)]
    replace: bool,
}

async fn import_month(State(st): State<Shared>, user: AuthUser, Json(r): Json<ImportReq>) -> AppResult<Response> {
    let m = st.restore(&user.0, r.snapshot, r.replace).await?;
    created(m.view(st.today_for(&user.0)))
}

// ---------------------------------------------------------------- income & paychecks

#[derive(Deserialize)]
pub struct ScheduleReq {
    pub schedule_type: String,
    #[serde(default)]
    pub recurrence_rule: Option<Recurrence>,
    #[serde(default)]
    pub expected_dates: Option<Vec<NaiveDate>>,
}

impl ScheduleReq {
    pub fn to_schedule(&self) -> AppResult<Schedule> {
        match self.schedule_type.as_str() {
            "recurring" => Ok(Schedule::Recurring {
                recurrence_rule: self.recurrence_rule.clone().ok_or_else(|| AppError::bad("recurrence_rule is required for recurring lines"))?,
            }),
            "one_off" => Ok(Schedule::OneOff {
                dates: self.expected_dates.clone().ok_or_else(|| AppError::bad("expected_dates is required for one_off lines"))?,
            }),
            _ => Err(AppError::bad("schedule_type must be one_off or recurring")),
        }
    }
}

#[derive(Deserialize)]
struct IncomeReq {
    name: String,
    planned_amount: i64,
    #[serde(flatten)]
    schedule: ScheduleReq,
}

fn income_json(m: &Month, id: &Id) -> Value {
    let l = m.income_line(id);
    json!({
        "income_line": l.map(|l| json!({
            "id": l.id, "name": l.name, "planned_amount": l.planned_amount,
            "schedule_type": l.schedule_type(), "recurrence_rule": l.recurrence_rule,
            "expected_dates": m.paychecks_by_date().iter().filter(|p| p.income_line_id == l.id).map(|p| p.date).collect::<Vec<_>>(),
        })),
        "paychecks": m.paychecks_by_date().iter().filter(|p| &p.income_line_id == id).map(|p| m.paycheck_view(p)).collect::<Vec<_>>(),
    })
}

async fn list_income(State(st): State<Shared>, user: AuthUser, Path(id): Path<Id>) -> J {
    let m = st.load(&user.0, &id).await?.month;
    ok(m.income_lines.iter().map(|l| income_json(&m, &l.id)).collect::<Vec<_>>())
}

async fn create_income(State(st): State<Shared>, user: AuthUser, Path(id): Path<Id>, Json(r): Json<IncomeReq>) -> AppResult<Response> {
    let schedule = r.schedule.to_schedule()?;
    let (lid, m) = st.mutate(&user.0, &id, |m| m.add_income_line(&r.name, cents(r.planned_amount), schedule)).await?;
    created(income_json(&m, &lid))
}

#[derive(Deserialize)]
struct SuggestQuery {
    #[serde(default)]
    q: String,
}

async fn income_suggestions(State(st): State<Shared>, user: AuthUser, Path(id): Path<Id>, Query(q): Query<SuggestQuery>) -> J {
    let m = st.load(&user.0, &id).await?.month;
    let history = st.income_history(&user.0).await?;
    ok(suggest_income(&q.q, &history, m.year_month, 5))
}

#[derive(Deserialize)]
struct IncomePatch {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    planned_amount: Option<i64>,
    #[serde(default)]
    schedule_type: Option<String>,
    #[serde(default)]
    recurrence_rule: Option<Recurrence>,
    #[serde(default)]
    expected_dates: Option<Vec<NaiveDate>>,
}

async fn patch_income(State(st): State<Shared>, user: AuthUser, Path(id): Path<Id>, Json(r): Json<IncomePatch>) -> J {
    let mid = st.resolve(&user.0, Owner::IncomeLine, &id).await?;
    let schedule = r
        .schedule_type
        .map(|t| ScheduleReq { schedule_type: t, recurrence_rule: r.recurrence_rule, expected_dates: r.expected_dates }.to_schedule())
        .transpose()?;
    let (impact, m) = st
        .mutate(&user.0, &mid, |m| m.update_income_line(&id, r.name.as_deref(), r.planned_amount.map(cents), schedule))
        .await?;
    let mut v = income_json(&m, &id);
    v["impact"] = json!(impact);
    Ok(Json(v))
}

async fn delete_income(State(st): State<Shared>, user: AuthUser, Path(id): Path<Id>) -> J {
    let mid = st.resolve(&user.0, Owner::IncomeLine, &id).await?;
    let (impact, _) = st.mutate(&user.0, &mid, |m| m.delete_income_line(&id)).await?;
    ok(json!({ "impact": impact }))
}

async fn list_paychecks(State(st): State<Shared>, user: AuthUser, Path(id): Path<Id>) -> J {
    let m = st.load(&user.0, &id).await?.month;
    ok(m.paychecks_by_date().iter().map(|p| m.paycheck_view(p)).collect::<Vec<_>>())
}

#[derive(Deserialize)]
struct AddPaycheckReq {
    income_line_id: Id,
    date: NaiveDate,
    #[serde(default)]
    planned_amount: Option<i64>,
}

async fn add_paycheck(State(st): State<Shared>, user: AuthUser, Path(id): Path<Id>, Json(r): Json<AddPaycheckReq>) -> AppResult<Response> {
    let (pid, m) = st.mutate(&user.0, &id, |m| m.add_paycheck(&r.income_line_id, r.date, r.planned_amount.map(cents))).await?;
    let p = m.paycheck(&pid).ok_or(AppError::NotFound)?;
    created(m.paycheck_view(p))
}

#[derive(Deserialize)]
struct PaycheckPatch {
    #[serde(default)]
    planned_amount: Option<i64>,
    #[serde(default, deserialize_with = "double_option")]
    actual_amount: Option<Option<i64>>,
    #[serde(default)]
    status: Option<String>,
}

async fn patch_paycheck(State(st): State<Shared>, user: AuthUser, Path(id): Path<Id>, Json(r): Json<PaycheckPatch>) -> J {
    let mid = st.resolve(&user.0, Owner::Paycheck, &id).await?;
    let status = r
        .status
        .as_deref()
        .map(|s| PaycheckStatus::parse(s).ok_or_else(|| AppError::bad("status must be planned, received or skipped")))
        .transpose()?;
    let (impact, m) = st
        .mutate(&user.0, &mid, |m| {
            let mut impact = Impact::default();
            if let Some(p) = r.planned_amount {
                impact = m.set_paycheck_planned(&id, cents(p))?;
            }
            if let Some(a) = r.actual_amount {
                m.set_paycheck_actual(&id, a.map(cents))?;
            }
            if let Some(s) = status {
                let i = m.set_paycheck_status(&id, s)?;
                impact.reduced_lines.extend(i.reduced_lines);
            }
            Ok(impact)
        })
        .await?;
    let p = m.paycheck(&id).ok_or(AppError::NotFound)?;
    ok(json!({ "paycheck": m.paycheck_view(p), "impact": impact }))
}

async fn delete_paycheck(State(st): State<Shared>, user: AuthUser, Path(id): Path<Id>) -> J {
    let mid = st.resolve(&user.0, Owner::Paycheck, &id).await?;
    let (impact, _) = st.mutate(&user.0, &mid, |m| m.delete_paycheck(&id)).await?;
    ok(json!({ "impact": impact }))
}

async fn apply_actual(State(st): State<Shared>, user: AuthUser, Path(id): Path<Id>) -> J {
    let mid = st.resolve(&user.0, Owner::Paycheck, &id).await?;
    let (impact, m) = st.mutate(&user.0, &mid, |m| m.apply_actual(&id)).await?;
    let p = m.paycheck(&id).ok_or(AppError::NotFound)?;
    ok(json!({ "paycheck": m.paycheck_view(p), "impact": impact }))
}

// ---------------------------------------------------------------- categories & lines

async fn list_categories(State(st): State<Shared>, user: AuthUser, Path(id): Path<Id>) -> J {
    ok(st.load(&user.0, &id).await?.month.category_views())
}

#[derive(Deserialize)]
struct CategoryReq {
    name: String,
    #[serde(default)]
    kind: Option<String>,
}

async fn create_category(State(st): State<Shared>, user: AuthUser, Path(id): Path<Id>, Json(r): Json<CategoryReq>) -> AppResult<Response> {
    let kind = r.kind.as_deref().map_or(Some(CategoryKind::Standard), CategoryKind::parse).ok_or_else(|| AppError::bad("kind must be standard or debt"))?;
    let (cid, m) = st.mutate(&user.0, &id, |m| m.add_category(&r.name, kind)).await?;
    created(m.category_views().into_iter().find(|c| c.id == cid))
}

#[derive(Deserialize)]
struct CategoryPatch {
    #[serde(default)]
    name: Option<String>,
    #[serde(default, rename = "move")]
    direction: Option<String>,
}

async fn patch_category(State(st): State<Shared>, user: AuthUser, Path(id): Path<Id>, Json(r): Json<CategoryPatch>) -> J {
    let mid = st.resolve(&user.0, Owner::Category, &id).await?;
    let (_, m) = st
        .mutate(&user.0, &mid, |m| {
            if let Some(n) = &r.name {
                m.rename_category(&id, n)?;
            }
            if let Some(d) = &r.direction {
                m.move_category(&id, d == "up")?;
            }
            Ok(())
        })
        .await?;
    ok(m.category_views().into_iter().find(|c| c.id == id))
}

async fn delete_category(State(st): State<Shared>, user: AuthUser, Path(id): Path<Id>) -> J {
    let mid = st.resolve(&user.0, Owner::Category, &id).await?;
    let (impact, _) = st.mutate(&user.0, &mid, |m| m.delete_category(&id)).await?;
    ok(json!({ "impact": impact }))
}

async fn list_lines(State(st): State<Shared>, user: AuthUser, Path(id): Path<Id>) -> J {
    let m = st.load(&user.0, &id).await?.month;
    ok(m.category_views().into_iter().flat_map(|c| c.lines).collect::<Vec<_>>())
}

fn line_json(m: &Month, id: &Id) -> Option<LineView> {
    m.category_views().into_iter().flat_map(|c| c.lines).find(|l| &l.id == id)
}

#[derive(Deserialize)]
struct LineReq {
    category_id: Id,
    name: String,
}

async fn create_line(State(st): State<Shared>, user: AuthUser, Path(id): Path<Id>, Json(r): Json<LineReq>) -> AppResult<Response> {
    let (lid, m) = st.mutate(&user.0, &id, |m| m.add_expense_line(&r.category_id, &r.name)).await?;
    created(line_json(&m, &lid))
}

#[derive(Deserialize)]
struct LinePatch {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    category_id: Option<Id>,
    /// Monthly-overview edit: translated into allocation changes.
    #[serde(default)]
    planned_amount: Option<i64>,
    #[serde(default, deserialize_with = "double_option")]
    current_balance: Option<Option<i64>>,
    #[serde(default, deserialize_with = "double_option")]
    minimum_payment: Option<Option<i64>>,
    #[serde(default, rename = "move")]
    direction: Option<String>,
}

async fn patch_line(State(st): State<Shared>, user: AuthUser, Path(id): Path<Id>, Json(r): Json<LinePatch>) -> J {
    let mid = st.resolve(&user.0, Owner::ExpenseLine, &id).await?;
    let (_, m) = st
        .mutate(&user.0, &mid, |m| {
            if let Some(n) = &r.name {
                m.rename_expense_line(&id, n)?;
            }
            if let Some(c) = &r.category_id {
                m.set_line_category(&id, c)?;
            }
            if r.current_balance.is_some() || r.minimum_payment.is_some() {
                let line = m.expense_line(&id).cloned().ok_or(DomainError::NotFound { kind: "expense line", id: id.clone() })?;
                let bal = r.current_balance.map_or(line.current_balance, |v| v.map(cents));
                let min = r.minimum_payment.map_or(line.minimum_payment, |v| v.map(cents));
                m.set_debt_fields(&id, bal, min)?;
            }
            if let Some(d) = &r.direction {
                m.move_expense_line(&id, d == "up")?;
            }
            if let Some(p) = r.planned_amount {
                m.set_line_planned(&id, cents(p))?;
            }
            Ok(())
        })
        .await?;
    ok(line_json(&m, &id))
}

async fn delete_line(State(st): State<Shared>, user: AuthUser, Path(id): Path<Id>) -> AppResult<StatusCode> {
    let mid = st.resolve(&user.0, Owner::ExpenseLine, &id).await?;
    st.mutate(&user.0, &mid, |m| m.delete_expense_line(&id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------- allocations

async fn list_allocations(State(st): State<Shared>, user: AuthUser, Path(pid): Path<Id>) -> J {
    let mid = st.resolve(&user.0, Owner::Paycheck, &pid).await?;
    let m = st.load(&user.0, &mid).await?.month;
    ok(m.allocations.iter().filter(|a| a.paycheck_id == pid).collect::<Vec<_>>())
}

#[derive(Deserialize)]
struct AllocReq {
    expense_line_id: Id,
    amount: i64,
}

fn alloc_json(m: &Month, aid: &Id) -> Value {
    let a = m.allocation(aid);
    json!({
        "allocation": a,
        "safe_to_spend": a.map(|a| m.safe_to_spend(&a.paycheck_id)),
        "line_planned": a.map(|a| m.line_planned(&a.expense_line_id)),
    })
}

async fn create_allocation(State(st): State<Shared>, user: AuthUser, Path(pid): Path<Id>, Json(r): Json<AllocReq>) -> AppResult<Response> {
    let mid = st.resolve(&user.0, Owner::Paycheck, &pid).await?;
    let (aid, m) = st.mutate(&user.0, &mid, |m| m.allocate(&pid, &r.expense_line_id, cents(r.amount))).await?;
    created(alloc_json(&m, &aid))
}

#[derive(Deserialize)]
struct AllocPatch {
    amount: i64,
}

async fn patch_allocation(State(st): State<Shared>, user: AuthUser, Path(id): Path<Id>, Json(r): Json<AllocPatch>) -> J {
    let mid = st.resolve(&user.0, Owner::Allocation, &id).await?;
    let (_, m) = st.mutate(&user.0, &mid, |m| m.update_allocation(&id, cents(r.amount))).await?;
    Ok(Json(alloc_json(&m, &id)))
}

async fn delete_allocation(State(st): State<Shared>, user: AuthUser, Path(id): Path<Id>) -> AppResult<StatusCode> {
    let mid = st.resolve(&user.0, Owner::Allocation, &id).await?;
    st.mutate(&user.0, &mid, |m| m.delete_allocation(&id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct TransferReq {
    from_paycheck_id: Id,
    to_paycheck_id: Id,
    expense_line_id: Id,
    amount: i64,
}

async fn transfer(State(st): State<Shared>, user: AuthUser, Json(r): Json<TransferReq>) -> J {
    let mid = st.resolve(&user.0, Owner::Paycheck, &r.from_paycheck_id).await?;
    let (_, m) = st
        .mutate(&user.0, &mid, |m| m.transfer(&r.from_paycheck_id, &r.to_paycheck_id, &r.expense_line_id, cents(r.amount)))
        .await?;
    ok(json!({
        "from": m.allocation_for(&r.from_paycheck_id, &r.expense_line_id),
        "to": m.allocation_for(&r.to_paycheck_id, &r.expense_line_id),
    }))
}

// ---------------------------------------------------------------- transactions

async fn list_transactions(State(st): State<Shared>, user: AuthUser, Path(id): Path<Id>) -> J {
    ok(st.load(&user.0, &id).await?.month.transactions)
}

#[derive(Deserialize)]
pub struct TxReq {
    #[serde(default)]
    pub id: Option<Id>,
    pub date: NaiveDate,
    pub amount: i64,
    #[serde(default)]
    pub payee: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub expense_line_id: Option<Id>,
    #[serde(default)]
    pub paycheck_id: Option<Id>,
    /// Parts of one split payment share a group id.
    #[serde(default)]
    pub split_group: Option<Id>,
}

impl TxReq {
    #[must_use]
    pub fn into_tx(self) -> Transaction {
        Transaction {
            id: self.id.unwrap_or_else(Id::generate),
            date: self.date,
            amount: cents(self.amount),
            payee: self.payee,
            notes: self.notes,
            expense_line_id: self.expense_line_id,
            paycheck_id: self.paycheck_id,
            split_group: self.split_group,
            account_id: None,
            transfer_account_id: None,
        }
    }
}

async fn create_transaction(State(st): State<Shared>, user: AuthUser, Path(id): Path<Id>, Json(r): Json<TxReq>) -> AppResult<Response> {
    let (tid, m) = st.mutate(&user.0, &id, |m| m.add_transaction(r.into_tx())).await?;
    created(m.transaction(&tid))
}

#[derive(Deserialize)]
struct TxPatch {
    #[serde(default)]
    date: Option<NaiveDate>,
    #[serde(default)]
    amount: Option<i64>,
    #[serde(default, deserialize_with = "double_option")]
    payee: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    notes: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    expense_line_id: Option<Option<Id>>,
    #[serde(default, deserialize_with = "double_option")]
    paycheck_id: Option<Option<Id>>,
}

async fn patch_transaction(State(st): State<Shared>, user: AuthUser, Path(id): Path<Id>, Json(r): Json<TxPatch>) -> J {
    let mid = st.resolve(&user.0, Owner::Transaction, &id).await?;
    let (_, m) = st
        .mutate(&user.0, &mid, |m| {
            let mut t = m.transaction(&id).cloned().ok_or(DomainError::NotFound { kind: "transaction", id: id.clone() })?;
            if let Some(d) = r.date {
                t.date = d;
            }
            if let Some(a) = r.amount {
                t.amount = cents(a);
            }
            if let Some(v) = r.payee {
                t.payee = v;
            }
            if let Some(v) = r.notes {
                t.notes = v;
            }
            if let Some(v) = r.expense_line_id {
                t.expense_line_id = v;
            }
            if let Some(v) = r.paycheck_id {
                t.paycheck_id = v;
            }
            m.update_transaction(t)
        })
        .await?;
    ok(m.transaction(&id))
}

async fn delete_transaction(State(st): State<Shared>, user: AuthUser, Path(id): Path<Id>) -> AppResult<StatusCode> {
    let mid = st.resolve(&user.0, Owner::Transaction, &id).await?;
    st.mutate(&user.0, &mid, |m| m.delete_transaction(&id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------- derived views

async fn safe_to_spend(State(st): State<Shared>, user: AuthUser, Path(pid): Path<Id>) -> J {
    let mid = st.resolve(&user.0, Owner::Paycheck, &pid).await?;
    let m = st.load(&user.0, &mid).await?.month;
    ok(json!({
        "safe_to_spend": m.safe_to_spend(&pid),
        "rolling_available": m.rolling_available(st.today_for(&user.0)),
    }))
}

async fn summary(State(st): State<Shared>, user: AuthUser, Path(id): Path<Id>) -> J {
    let m = st.load(&user.0, &id).await?.month;
    let v = m.view(st.today_for(&user.0));
    ok(json!({
        "zero_difference": v.zero_difference,
        "is_zero": v.is_zero,
        "status": v.status,
        "reassigning": v.reassigning,
        "total_planned_income": v.total_planned_income,
        "total_planned_expense": v.total_planned_expense,
        "total_spent": v.total_spent,
        "rolling_available": v.rolling_available,
        "categories": v.categories.iter().map(|c| json!({
            "id": c.id, "name": c.name, "planned": c.planned, "spent": c.spent, "remaining": c.remaining,
        })).collect::<Vec<_>>(),
        "cards": report::summary_cards(&m),
    }))
}

async fn variance(State(st): State<Shared>, user: AuthUser, Path(id): Path<Id>) -> J {
    ok(variance_suggestions(&st.load(&user.0, &id).await?.month))
}

async fn export_csv(State(st): State<Shared>, user: AuthUser, Path(id): Path<Id>) -> AppResult<Response> {
    let m = st.load(&user.0, &id).await?.month;
    let w = st.wallet(&user.0).await?;
    Ok(export::csv_response(&m, &w))
}

async fn snapshot(State(st): State<Shared>, user: AuthUser, Path(id): Path<Id>) -> J {
    ok(st.snapshot(&user.0, &id).await?)
}

async fn report_mom(State(st): State<Shared>, user: AuthUser, Path(id): Path<Id>) -> J {
    let m = st.load(&user.0, &id).await?.month;
    ok(report::month_over_month(&m, &st.all_months(&user.0).await?))
}

async fn report_ytd(State(st): State<Shared>, user: AuthUser, Path(id): Path<Id>) -> J {
    let m = st.load(&user.0, &id).await?.month;
    ok(report::year_to_date(&m, &st.all_months(&user.0).await?))
}

async fn report_yoy(State(st): State<Shared>, user: AuthUser, Path(id): Path<Id>) -> J {
    let m = st.load(&user.0, &id).await?.month;
    ok(report::year_over_year(&m, &st.all_months(&user.0).await?))
}

async fn report_cards(State(st): State<Shared>, user: AuthUser, Path(id): Path<Id>) -> J {
    ok(report::summary_cards(&st.load(&user.0, &id).await?.month))
}
