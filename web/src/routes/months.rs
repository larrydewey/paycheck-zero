//! Month CRUD routes with invariant enforcement.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, patch, post, Router},
    Json,
};

use chrono::NaiveDate;

use paycheckzero_core::{
    Allocation, Cents, DomainError, ExpenseCategory, ExpenseLine, Id, IncomeLine, Month,
    Paycheck, PaycheckStatus, ScheduleType, Transaction,
};
use paycheckzero_core::views::MonthSummary;
use paycheckzero_storage::Repository;

use crate::service::error::ServiceError;
use crate::AppState;

#[derive(serde::Deserialize)]
pub struct CreateMonthRequest {
    pub year_month: String,
}

#[derive(serde::Deserialize)]
pub struct CreateCategoryRequest {
    pub name: String,
    pub sort_order: Option<i32>,
}

#[derive(serde::Deserialize)]
pub struct UpdateCategoryRequest {
    pub name: Option<String>,
    pub sort_order: Option<i32>,
}

#[derive(serde::Deserialize)]
pub struct CreateExpenseLineRequest {
    pub category_id: String,
    pub name: String,
    pub current_balance: Option<i64>,
    pub minimum_payment: Option<i64>,
}

#[derive(serde::Deserialize)]
pub struct UpdateExpenseLineRequest {
    pub name: Option<String>,
    /// `null` clears; absent leaves unchanged.
    pub current_balance: Option<Option<i64>>,
    /// `null` clears; absent leaves unchanged.
    pub minimum_payment: Option<Option<i64>>,
}

#[derive(serde::Deserialize)]
pub struct CreateIncomeLineRequest {
    pub name: String,
    pub planned_amount: i64,
    pub schedule_type: String,
    pub recurrence_rule: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct UpdateIncomeLineRequest {
    pub name: Option<String>,
    pub planned_amount: Option<i64>,
    pub schedule_type: Option<String>,
    /// `null` clears; absent leaves unchanged.
    pub recurrence_rule: Option<Option<String>>,
}

#[derive(serde::Deserialize)]
pub struct CreatePaycheckRequest {
    pub income_line_id: String,
    pub date: String,
    pub planned_amount: Option<i64>,
    pub actual_amount: Option<i64>,
    pub status: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct UpdatePaycheckRequest {
    pub planned_amount: Option<i64>,
    pub actual_amount: Option<i64>,
    pub status: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct CreateAllocationRequest {
    pub expense_line_id: String,
    pub amount: i64,
}

#[derive(serde::Deserialize)]
pub struct UpdateAllocationRequest {
    pub amount: i64,
}

#[derive(serde::Deserialize)]
pub struct TransferRequest {
    pub from_paycheck_id: String,
    pub to_paycheck_id: String,
    pub expense_line_id: String,
    pub amount: i64,
}

#[derive(serde::Deserialize)]
pub struct CreateTransactionRequest {
    pub date: String,
    pub amount: i64,
    pub payee: Option<String>,
    pub notes: Option<String>,
    pub expense_line_id: Option<String>,
    pub paycheck_id: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct UpdateTransactionRequest {
    pub date: Option<String>,
    pub amount: Option<i64>,
    pub payee: Option<String>,
    pub notes: Option<String>,
    pub expense_line_id: Option<String>,
    pub paycheck_id: Option<String>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/months", get(list_months))
        .route("/months", post(create_month))
        .route("/months/{month_id}", get(get_month))
        .route("/months/{month_id}/lock", post(lock_month))
        .route("/months/{month_id}/income-lines", get(list_income_lines))
        .route("/months/{month_id}/income-lines", post(create_income_line))
        .route("/income-lines/{id}", patch(update_income_line))
        .route("/income-lines/{id}", delete(delete_income_line))
        .route("/months/{month_id}/paychecks", get(list_paychecks))
        .route("/months/{month_id}/paychecks", post(create_paycheck))
        .route("/paychecks/{id}", patch(update_paycheck))
        .route("/paychecks/{id}/allocations", get(list_allocations))
        .route("/paychecks/{id}/allocations", post(create_allocation))
        .route("/allocations/{id}", patch(update_allocation))
        .route("/allocations/{id}", delete(delete_allocation))
        .route("/allocations/transfer", post(transfer))
        .route("/paychecks/{id}/safe-to-spend", get(safe_to_spend))
        .route("/months/{id}/summary", get(month_summary))
        .route("/months/{month_id}/categories", get(list_categories))
        .route("/months/{month_id}/categories", post(create_category))
        .route("/categories/{id}", patch(update_category))
        .route("/categories/{id}", delete(delete_category))
        .route("/months/{month_id}/expense-lines", get(list_expense_lines))
        .route("/months/{month_id}/expense-lines", post(create_expense_line))
        .route("/expense-lines/{id}", patch(update_expense_line))
        .route("/expense-lines/{id}", delete(delete_expense_line))
        .route("/months/{month_id}/transactions", get(list_transactions))
        .route("/months/{month_id}/transactions", post(create_transaction))
        .route("/transactions/{id}", patch(update_transaction))
        .route("/transactions/{id}", delete(delete_transaction))
}

// --- Month routes ---

pub async fn list_months(
    State(state): State<AppState>,
) -> Result<Json<Vec<paycheckzero_storage::MonthListItem>>, ServiceError> {
    let months = state.db.lock().unwrap().list_months()?;
    Ok(Json(months))
}

pub async fn create_month(
    State(state): State<AppState>,
    Json(req): Json<CreateMonthRequest>,
) -> Result<(StatusCode, Json<MonthSummary>), ServiceError> {
    let ym = parse_date(&req.year_month)?;
    let mut m = Month::new(Id::generate(), ym);
    m.seed_starter_categories();
    state.db.lock().unwrap().save_month(&m)?;
    let summary = m.summary(chrono::Utc::now().date_naive());
    Ok((StatusCode::CREATED, Json(summary)))
}

pub async fn get_month(
    State(state): State<AppState>,
    Path(month_id): Path<String>,
) -> Result<Json<Month>, ServiceError> {
    let month = load_month(&state, &month_id)?;
    Ok(Json(month))
}

pub async fn lock_month(
    State(state): State<AppState>,
    Path(month_id): Path<String>,
) -> Result<Json<Month>, ServiceError> {
    let mut m = load_month(&state, &month_id)?;
    m.lock()?;
    state.db.lock().unwrap().save_month(&m)?;
    Ok(Json(m))
}

// --- Category routes ---

pub async fn list_categories(
    State(state): State<AppState>,
    Path(month_id): Path<String>,
) -> Result<Json<Vec<ExpenseCategory>>, ServiceError> {
    let m = load_month(&state, &month_id)?;
    Ok(Json(m.categories))
}

pub async fn create_category(
    State(state): State<AppState>,
    (Path(month_id), Json(req)): (Path<String>, Json<CreateCategoryRequest>),
) -> Result<(StatusCode, Json<ExpenseCategory>), ServiceError> {
    let mut m = load_month(&state, &month_id)?;
    let cat = m.add_category(&req.name)?.clone();
    if let Some(order) = req.sort_order {
        if let Some(c) = m.categories.iter_mut().find(|c| c.id == cat.id) {
            c.sort_order = order;
        }
    }
    state.db.lock().unwrap().save_month(&m)?;
    Ok((StatusCode::CREATED, Json(cat)))
}

pub async fn update_category(
    State(state): State<AppState>,
    (Path(id), Json(req)): (Path<String>, Json<UpdateCategoryRequest>),
) -> Result<Json<ExpenseCategory>, ServiceError> {
    let mut m = find_month_by_category(&state, &id)?;
    if let Some(name) = req.name {
        m.rename_category(&Id::new(&id), &name)?;
    }
    if let Some(order) = req.sort_order {
        m.set_category_order(&Id::new(&id), order)?;
    }
    state.db.lock().unwrap().save_month(&m)?;
    let cat = m
        .category(&Id::new(&id))
        .ok_or_else(|| ServiceError::NotFound(format!("category not found: {id}")))?
        .clone();
    Ok(Json(cat))
}

pub async fn delete_category(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ServiceError> {
    let mut m = find_month_by_category(&state, &id)?;
    m.delete_category(&Id::new(&id))?;
    state.db.lock().unwrap().save_month(&m)?;
    Ok(StatusCode::NO_CONTENT)
}

// --- Expense line routes ---

pub async fn list_expense_lines(
    State(state): State<AppState>,
    Path(month_id): Path<String>,
) -> Result<Json<Vec<ExpenseLine>>, ServiceError> {
    let m = load_month(&state, &month_id)?;
    Ok(Json(m.expense_lines))
}

pub async fn create_expense_line(
    State(state): State<AppState>,
    (Path(month_id), Json(req)): (Path<String>, Json<CreateExpenseLineRequest>),
) -> Result<(StatusCode, Json<ExpenseLine>), ServiceError> {
    let mut m = load_month(&state, &month_id)?;
    let line = m
        .add_expense_line(
            &Id::new(&req.category_id),
            &req.name,
            req.current_balance.map(Cents::from_cents),
            req.minimum_payment.map(Cents::from_cents),
        )?
        .clone();
    state.db.lock().unwrap().save_month(&m)?;
    Ok((StatusCode::CREATED, Json(line)))
}

pub async fn update_expense_line(
    State(state): State<AppState>,
    (Path(id), Json(req)): (Path<String>, Json<UpdateExpenseLineRequest>),
) -> Result<Json<ExpenseLine>, ServiceError> {
    let mut m = find_month_by_expense_line(&state, &id)?;
    let name = req.name.as_deref();
    let balance = req.current_balance.map(|v| v.map(Cents::from_cents));
    let min = req.minimum_payment.map(|v| v.map(Cents::from_cents));
    m.update_expense_line(&Id::new(&id), name, balance, min)?;
    state.db.lock().unwrap().save_month(&m)?;
    let line = m
        .expense_line(&Id::new(&id))
        .ok_or_else(|| ServiceError::NotFound(format!("expense line not found: {id}")))?
        .clone();
    Ok(Json(line))
}

pub async fn delete_expense_line(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ServiceError> {
    let mut m = find_month_by_expense_line(&state, &id)?;
    m.delete_expense_line(&Id::new(&id))?;
    state.db.lock().unwrap().save_month(&m)?;
    Ok(StatusCode::NO_CONTENT)
}

// --- Income line routes ---

pub async fn list_income_lines(
    State(state): State<AppState>,
    Path(month_id): Path<String>,
) -> Result<Json<Vec<IncomeLine>>, ServiceError> {
    let m = load_month(&state, &month_id)?;
    Ok(Json(m.income_lines))
}

pub async fn create_income_line(
    State(state): State<AppState>,
    (Path(month_id), Json(req)): (Path<String>, Json<CreateIncomeLineRequest>),
) -> Result<(StatusCode, Json<IncomeLine>), ServiceError> {
    let schedule = match req.schedule_type.as_str() {
        "one_off" => ScheduleType::OneOff,
        "recurring" => ScheduleType::Recurring,
        _ => {
            return Err(ServiceError::Conflict(format!(
                "invalid schedule_type: {}",
                req.schedule_type
            )))
        }
    };
    let mut m = load_month(&state, &month_id)?;
    let il = m
        .add_income_line(
            &req.name,
            Cents::from_cents(req.planned_amount),
            schedule,
            req.recurrence_rule,
        )?
        .clone();
    state.db.lock().unwrap().save_month(&m)?;
    Ok((StatusCode::CREATED, Json(il)))
}

pub async fn update_income_line(
    State(state): State<AppState>,
    (Path(id), Json(req)): (Path<String>, Json<UpdateIncomeLineRequest>),
) -> Result<Json<IncomeLine>, ServiceError> {
    let mut m = find_month_by_income_line(&state, &id)?;
    let schedule = match req.schedule_type.as_deref() {
        None => None,
        Some("one_off") => Some(ScheduleType::OneOff),
        Some("recurring") => Some(ScheduleType::Recurring),
        Some(other) => {
            return Err(ServiceError::Conflict(format!(
                "invalid schedule_type: {}",
                other
            )))
        }
    };
    let amount = req.planned_amount.map(Cents::from_cents);
    m.update_income_line(&Id::new(&id), req.name.as_deref(), amount, schedule, req.recurrence_rule)?;
    state.db.lock().unwrap().save_month(&m)?;
    let il = m
        .income_line(&Id::new(&id))
        .ok_or_else(|| ServiceError::NotFound(format!("income line not found: {id}")))?
        .clone();
    Ok(Json(il))
}

pub async fn delete_income_line(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ServiceError> {
    let mut m = find_month_by_income_line(&state, &id)?;
    m.delete_income_line(&Id::new(&id))?;
    state.db.lock().unwrap().save_month(&m)?;
    Ok(StatusCode::NO_CONTENT)
}

// --- Paycheck routes ---

pub async fn list_paychecks(
    State(state): State<AppState>,
    Path(month_id): Path<String>,
) -> Result<Json<Vec<Paycheck>>, ServiceError> {
    let m = load_month(&state, &month_id)?;
    Ok(Json(m.paychecks))
}

pub async fn create_paycheck(
    State(state): State<AppState>,
    (Path(month_id), Json(req)): (Path<String>, Json<CreatePaycheckRequest>),
) -> Result<(StatusCode, Json<Paycheck>), ServiceError> {
    let mut m = load_month(&state, &month_id)?;
    let date = parse_date(&req.date)?;
    let planned = req.planned_amount.unwrap_or(0);
    let pc = m
        .add_paycheck(&Id::new(&req.income_line_id), date, Cents::from_cents(planned))?
        .clone();
    if let Some(aa) = req.actual_amount {
        m.set_paycheck_actual(&pc.id, Some(Cents::from_cents(aa)))?;
    }
    if let Some(status) = req.status {
        match status.as_str() {
            "skipped" => {
                m.skip_paycheck(&pc.id)?;
            }
            "received" => {
                if let Some(p) = m.paychecks.iter_mut().find(|p| p.id == pc.id) {
                    p.status = PaycheckStatus::Received;
                }
            }
            "planned" => {}
            _ => {
                return Err(ServiceError::Conflict(format!("invalid status: {}", status)))
            }
        }
    }
    state.db.lock().unwrap().save_month(&m)?;
    let saved = m
        .paycheck(&pc.id)
        .ok_or_else(|| ServiceError::NotFound("paycheck not found".into()))?
        .clone();
    Ok((StatusCode::CREATED, Json(saved)))
}

pub async fn update_paycheck(
    State(state): State<AppState>,
    (Path(id), Json(req)): (Path<String>, Json<UpdatePaycheckRequest>),
) -> Result<Json<Paycheck>, ServiceError> {
    let mut m = find_month_by_paycheck(&state, &id)?;
    let pc_id = Id::new(&id);

    if let Some(pa) = req.planned_amount {
        m.set_paycheck_planned(&pc_id, Cents::from_cents(pa))?;
    }
    if let Some(aa) = req.actual_amount {
        m.set_paycheck_actual(&pc_id, Some(Cents::from_cents(aa)))?;
    }
    if let Some(status) = req.status {
        match status.as_str() {
            "planned" => {
                if let Some(p) = m.paychecks.iter_mut().find(|p| p.id.as_str() == pc_id.as_str()) {
                    p.status = PaycheckStatus::Planned;
                }
            }
            "received" => {
                if let Some(p) = m.paychecks.iter_mut().find(|p| p.id.as_str() == pc_id.as_str()) {
                    p.status = PaycheckStatus::Received;
                }
            }
            "skipped" => {
                m.skip_paycheck(&pc_id)?;
            }
            _ => {
                return Err(ServiceError::Conflict(format!("invalid status: {}", status)))
            }
        }
    }

    state.db.lock().unwrap().save_month(&m)?;

    let pc = m
        .paycheck(&pc_id)
        .ok_or_else(|| ServiceError::NotFound(format!("paycheck not found: {id}")))?
        .clone();
    Ok(Json(pc))
}

// --- Allocation routes ---

pub async fn list_allocations(
    State(state): State<AppState>,
    Path(paycheck_id): Path<String>,
) -> Result<Json<Vec<Allocation>>, ServiceError> {
    let m = find_month_by_paycheck(&state, &paycheck_id)?;
    let allocs = m
        .allocations
        .iter()
        .filter(|a| a.paycheck_id == Id::new(&paycheck_id))
        .cloned()
        .collect();
    Ok(Json(allocs))
}

pub async fn create_allocation(
    State(state): State<AppState>,
    (Path(paycheck_id), Json(req)): (Path<String>, Json<CreateAllocationRequest>),
) -> Result<(StatusCode, Json<Allocation>), ServiceError> {
    let mut m = find_month_by_paycheck(&state, &paycheck_id)?;
    let alloc = m
        .allocate(
            &Id::new(&paycheck_id),
            &Id::new(&req.expense_line_id),
            Cents::from_cents(req.amount),
        )?
        .clone();
    state.db.lock().unwrap().save_month(&m)?;
    Ok((StatusCode::CREATED, Json(alloc)))
}

pub async fn update_allocation(
    State(state): State<AppState>,
    (Path(id), Json(req)): (Path<String>, Json<UpdateAllocationRequest>),
) -> Result<Json<Allocation>, ServiceError> {
    let mut m = find_month_by_allocation(&state, &id)?;
    let alloc = m
        .update_allocation(&Id::new(&id), Cents::from_cents(req.amount))?
        .clone();
    state.db.lock().unwrap().save_month(&m)?;
    Ok(Json(alloc))
}

pub async fn delete_allocation(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ServiceError> {
    let mut m = find_month_by_allocation(&state, &id)?;
    m.delete_allocation(&Id::new(&id))?;
    state.db.lock().unwrap().save_month(&m)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn transfer(
    State(state): State<AppState>,
    Json(req): Json<TransferRequest>,
) -> Result<StatusCode, ServiceError> {
    let mut m = find_month_by_paycheck(&state, &req.from_paycheck_id)?;
    m.transfer(
        &Id::new(&req.from_paycheck_id),
        &Id::new(&req.to_paycheck_id),
        &Id::new(&req.expense_line_id),
        Cents::from_cents(req.amount),
    )?;
    state.db.lock().unwrap().save_month(&m)?;
    Ok(StatusCode::OK)
}

// --- Transaction routes ---

pub async fn list_transactions(
    State(state): State<AppState>,
    Path(month_id): Path<String>,
) -> Result<Json<Vec<Transaction>>, ServiceError> {
    let m = load_month(&state, &month_id)?;
    Ok(Json(m.transactions))
}

pub async fn create_transaction(
    State(state): State<AppState>,
    (Path(month_id), Json(req)): (Path<String>, Json<CreateTransactionRequest>),
) -> Result<(StatusCode, Json<Transaction>), ServiceError> {
    let txn = Transaction {
        id: Id::generate(),
        date: parse_date(&req.date)?,
        amount: Cents::from_cents(req.amount),
        payee: req.payee,
        notes: req.notes,
        expense_line_id: req.expense_line_id.map(Id::new),
        paycheck_id: req.paycheck_id.map(Id::new),
    };
    let mut m = load_month(&state, &month_id)?;
    m.add_transaction(txn.clone());
    state.db.lock().unwrap().save_month(&m)?;
    Ok((StatusCode::CREATED, Json(txn)))
}

pub async fn update_transaction(
    State(state): State<AppState>,
    (Path(id), Json(req)): (Path<String>, Json<UpdateTransactionRequest>),
) -> Result<Json<Transaction>, ServiceError> {
    let mut m = find_month_by_transaction(&state, &id)?;
    let txn_id = Id::new(&id);
    let mut txn = m
        .transactions
        .iter()
        .find(|t| t.id == txn_id)
        .ok_or_else(|| ServiceError::NotFound(format!("transaction not found: {id}")))?
        .clone();
    if let Some(date) = req.date {
        txn.date = parse_date(&date)?;
    }
    if let Some(amount) = req.amount {
        txn.amount = Cents::from_cents(amount);
    }
    if req.payee.is_some() {
        txn.payee = req.payee;
    }
    if req.notes.is_some() {
        txn.notes = req.notes;
    }
    if let Some(el) = req.expense_line_id {
        txn.expense_line_id = if el.is_empty() { None } else { Some(Id::new(&el)) };
    }
    if let Some(pc) = req.paycheck_id {
        txn.paycheck_id = if pc.is_empty() { None } else { Some(Id::new(&pc)) };
    }
    let updated = m.update_transaction(&txn_id, txn)?.clone();
    state.db.lock().unwrap().save_month(&m)?;
    Ok(Json(updated))
}

pub async fn delete_transaction(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ServiceError> {
    let mut m = find_month_by_transaction(&state, &id)?;
    m.delete_transaction(&Id::new(&id))?;
    state.db.lock().unwrap().save_month(&m)?;
    Ok(StatusCode::NO_CONTENT)
}

// --- Derived views ---

pub async fn safe_to_spend(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<paycheckzero_core::views::PaycheckView>, ServiceError> {
    let m = find_month_by_paycheck(&state, &id)?;
    let pc = m
        .summary(chrono::Utc::now().date_naive())
        .paychecks
        .into_iter()
        .find(|p| p.id == Id::new(&id))
        .ok_or_else(|| ServiceError::NotFound(format!("paycheck not found: {id}")))?;
    Ok(Json(pc))
}

pub async fn month_summary(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<MonthSummary>, ServiceError> {
    let m = load_month(&state, &id)?;
    Ok(Json(m.summary(chrono::Utc::now().date_naive())))
}

// --- Helpers ---

fn parse_date(s: &str) -> Result<NaiveDate, ServiceError> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|_| ServiceError::Domain(DomainError::InvalidDate(s.to_string())))
}

fn load_month(state: &AppState, id: &str) -> Result<Month, ServiceError> {
    let m = state.db.lock().unwrap().load_month(&Id::new(id))?;
    m.ok_or_else(|| ServiceError::NotFound(format!("month not found: {id}")))
}

fn find_month_by_paycheck(state: &AppState, paycheck_id: &str) -> Result<Month, ServiceError> {
    let listing = state.db.lock().unwrap().list_months()?;
    for item in listing {
        let loaded = state.db.lock().unwrap().load_month(&item.id)?;
        if let Some(m) = loaded {
            if m.paychecks.iter().any(|p| p.id == Id::new(paycheck_id)) {
                return Ok(m);
            }
        }
    }
    Err(ServiceError::NotFound(format!(
        "paycheck not found: {paycheck_id}"
    )))
}

fn find_month_by_allocation(state: &AppState, allocation_id: &str) -> Result<Month, ServiceError> {
    let listing = state.db.lock().unwrap().list_months()?;
    for item in listing {
        let loaded = state.db.lock().unwrap().load_month(&item.id)?;
        if let Some(m) = loaded {
            if m.allocations.iter().any(|a| a.id == Id::new(allocation_id)) {
                return Ok(m);
            }
        }
    }
    Err(ServiceError::NotFound(format!(
        "allocation not found: {allocation_id}"
    )))
}

fn find_month_by_income_line(state: &AppState, id: &str) -> Result<Month, ServiceError> {
    let listing = state.db.lock().unwrap().list_months()?;
    for item in listing {
        let loaded = state.db.lock().unwrap().load_month(&item.id)?;
        if let Some(m) = loaded {
            if m.income_lines.iter().any(|l| l.id == Id::new(id)) {
                return Ok(m);
            }
        }
    }
    Err(ServiceError::NotFound(format!("income line not found: {id}")))
}

fn find_month_by_category(state: &AppState, id: &str) -> Result<Month, ServiceError> {
    let listing = state.db.lock().unwrap().list_months()?;
    for item in listing {
        let loaded = state.db.lock().unwrap().load_month(&item.id)?;
        if let Some(m) = loaded {
            if m.categories.iter().any(|c| c.id == Id::new(id)) {
                return Ok(m);
            }
        }
    }
    Err(ServiceError::NotFound(format!("category not found: {id}")))
}

fn find_month_by_expense_line(state: &AppState, id: &str) -> Result<Month, ServiceError> {
    let listing = state.db.lock().unwrap().list_months()?;
    for item in listing {
        let loaded = state.db.lock().unwrap().load_month(&item.id)?;
        if let Some(m) = loaded {
            if m.expense_lines.iter().any(|l| l.id == Id::new(id)) {
                return Ok(m);
            }
        }
    }
    Err(ServiceError::NotFound(format!("expense line not found: {id}")))
}

fn find_month_by_transaction(state: &AppState, id: &str) -> Result<Month, ServiceError> {
    let listing = state.db.lock().unwrap().list_months()?;
    for item in listing {
        let loaded = state.db.lock().unwrap().load_month(&item.id)?;
        if let Some(m) = loaded {
            if m.transactions.iter().any(|t| t.id == Id::new(id)) {
                return Ok(m);
            }
        }
    }
    Err(ServiceError::NotFound(format!("transaction not found: {id}")))
}