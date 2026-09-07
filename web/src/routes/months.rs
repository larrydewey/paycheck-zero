//! Month CRUD routes with invariant enforcement.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post, Router},
    Json,
};

use paycheckzero_core::{Cents, Id, Month, MonthStatus, ScheduleType, ExpenseCategory, ExpenseLine, IncomeLine, Paycheck, Transaction};

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
    pub current_balance: Option<i64>,
    pub minimum_payment: Option<i64>,
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
    pub recurrence_rule: Option<String>,
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
        .route("/months/{month_id}/paychecks", get(list_paychecks))
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
        .route("/months/{month_id}/expense-lines", get(list_expense_lines))
        .route("/months/{month_id}/expense-lines", post(create_expense_line))
        .route("/months/{month_id}/transactions", get(list_transactions))
        .route("/months/{month_id}/transactions", post(create_transaction))
}

// --- Month routes ---

pub async fn list_months(
    State(state): State<AppState>,
) -> Result<Json<Vec<paycheckzero_storage::MonthListItem>>, crate::service::error::ServiceError> {
    let months = state.db.list_months()?;
    Ok(Json(months))
}

pub async fn create_month(
    State(state): State<AppState>,
    Json(req): Json<CreateMonthRequest>,
) -> Result<(StatusCode, Json<paycheckzero_core::views::MonthSummary>), crate::service::error::ServiceError> {
    let mut m = Month::new(
        paycheckzero_core::Id::generate(),
        paycheckzero_core::parse_date(&req.year_month)?,
        MonthStatus::Draft,
        false,
    );

    // Seed starter categories
    m.seed_categories();

    state.db.save_month(&m)?;

    let summary = m.summary();
    Ok((StatusCode::CREATED, Json(summary)))
}

pub async fn get_month(
    State(state): State<AppState>,
    Path(month_id): Path<String>,
) -> Result<Json<Month>, crate::service::error::ServiceError> {
    let id = Id::new(month_id);
    let month = state
        .db
        .load_month(&id)?
        .ok_or_else(|| crate::service::error::ServiceError::NotFound(format!("month not found: {id}")))?;
    Ok(Json(month))
}

pub async fn lock_month(
    State(state): State<AppState>,
    Path(month_id): Path<String>,
) -> Result<Json<Month>, crate::service::error::ServiceError> {
    let mut m = state
        .db
        .load_month(&Id::new(month_id))?
        .ok_or_else(|| crate::service::error::ServiceError::NotFound(format!("month not found: {month_id}")))?;

    m.lock()?;
    state.db.save_month(&m)?;

    Ok(Json(m))
}

// --- Category routes ---

pub async fn list_categories(
    State(state): State<AppState>,
    Path(month_id): Path<String>,
) -> Result<Json<Vec<ExpenseCategory>>, crate::service::error::ServiceError> {
    let m = state
        .db
        .load_month(&Id::new(month_id))?
        .ok_or_else(|| crate::service::error::ServiceError::NotFound(format!("month not found: {month_id}")))?;
    Ok(Json(m.categories))
}

pub async fn create_category(
    State(state): State<AppState>,
    (Path(month_id), Json(req)): (Path<String>, Json<CreateCategoryRequest>),
) -> Result<(StatusCode, Json<ExpenseCategory>), crate::service::error::ServiceError> {
    let mut m = state
        .db
        .load_month(&Id::new(month_id))?
        .ok_or_else(|| crate::service::error::ServiceError::NotFound(format!("month not found: {month_id}")))?;

    let mut cat = ExpenseCategory {
        id: Id::generate(),
        name: req.name,
        sort_order: req.sort_order.unwrap_or(0),
    };
    m.add_category(&mut cat);
    state.db.save_month(&m)?;

    Ok((StatusCode::CREATED, Json(cat)))
}

// --- Expense line routes ---

pub async fn list_expense_lines(
    State(state): State<AppState>,
    Path(month_id): Path<String>,
) -> Result<Json<Vec<ExpenseLine>>, crate::service::error::ServiceError> {
    let m = state
        .db
        .load_month(&Id::new(month_id))?
        .ok_or_else(|| crate::service::error::ServiceError::NotFound(format!("month not found: {month_id}")))?;
    Ok(Json(m.expense_lines))
}

pub async fn create_expense_line(
    State(state): State<AppState>,
    (Path(month_id), Json(req)): (Path<String>, Json<CreateExpenseLineRequest>),
) -> Result<(StatusCode, Json<ExpenseLine>), crate::service::error::ServiceError> {
    let mut m = state
        .db
        .load_month(&Id::new(month_id))?
        .ok_or_else(|| crate::service::error::ServiceError::NotFound(format!("month not found: {month_id}")))?;

    let mut line = ExpenseLine {
        id: Id::generate(),
        category_id: Id::new(&req.category_id),
        name: req.name,
        current_balance: req.current_balance.map(Cents::from_cents),
        minimum_payment: req.minimum_payment.map(Cents::from_cents),
    };
    m.add_expense_line(&mut line);
    state.db.save_month(&m)?;

    Ok((StatusCode::CREATED, Json(line)))
}

// --- Income line routes ---

pub async fn list_income_lines(
    State(state): State<AppState>,
    Path(month_id): Path<String>,
) -> Result<Json<Vec<IncomeLine>>, crate::service::error::ServiceError> {
    let m = state
        .db
        .load_month(&Id::new(month_id))?
        .ok_or_else(|| crate::service::error::ServiceError::NotFound(format!("month not found: {month_id}")))?;
    Ok(Json(m.income_lines))
}

pub async fn create_income_line(
    State(state): State<AppState>,
    (Path(month_id), Json(req)): (Path<String>, Json<CreateIncomeLineRequest>),
) -> Result<(StatusCode, Json<IncomeLine>), crate::service::error::ServiceError> {
    let mut m = state
        .db
        .load_month(&Id::new(month_id))?
        .ok_or_else(|| crate::service::error::ServiceError::NotFound(format!("month not found: {month_id}")))?;

    let schedule = match req.schedule_type.as_str() {
        "one_off" => ScheduleType::OneOff,
        "recurring" => ScheduleType::Recurring,
        _ => {
            return Err(crate::service::error::ServiceError::Conflict(format!(
                "invalid schedule_type: {}",
                req.schedule_type
            )))
        }
    };

    let mut il = IncomeLine {
        id: Id::generate(),
        name: req.name,
        planned_amount: Cents::from_cents(req.planned_amount),
        schedule_type: schedule,
        recurrence_rule: req.recurrence_rule,
    };
    m.add_income_line(&mut il);
    state.db.save_month(&m)?;

    Ok((StatusCode::CREATED, Json(il)))
}

// --- Paycheck routes ---

pub async fn list_paychecks(
    State(state): State<AppState>,
    Path(month_id): Path<String>,
) -> Result<Json<Vec<Paycheck>>, crate::service::error::ServiceError> {
    let m = state
        .db
        .load_month(&Id::new(month_id))?
        .ok_or_else(|| crate::service::error::ServiceError::NotFound(format!("month not found: {month_id}")))?;
    Ok(Json(m.paychecks))
}

pub async fn update_paycheck(
    State(state): State<AppState>,
    (Path(id), Json(req)): (Path<String>, Json<UpdatePaycheckRequest>),
) -> Result<Json<Paycheck>, crate::service::error::ServiceError> {
    let mut m = state
        .db
        .load_month(&Id::new(&id))? // use paycheck id to find month
        .ok_or_else(|| crate::service::error::ServiceError::NotFound(format!("paycheck not found: {id}")))?;

    let pc = m
        .paychecks
        .iter_mut()
        .find(|p| p.id == Id::new(&id))
        .ok_or_else(|| crate::service::error::ServiceError::NotFound(format!("paycheck not found: {id}")))?;

    if let Some(pa) = req.planned_amount {
        pc.planned_amount = Cents::from_cents(pa);
    }
    if let Some(aa) = req.actual_amount {
        pc.actual_amount = Some(Cents::from_cents(aa));
    }
    if let Some(status) = req.status {
        pc.status = match status.as_str() {
            "planned" => paycheckzero_core::PaycheckStatus::Planned,
            "received" => paycheckzero_core::PaycheckStatus::Received,
            "skipped" => paycheckzero_core::PaycheckStatus::Skipped,
            _ => {
                return Err(crate::service::error::ServiceError::Conflict(format!(
                    "invalid status: {}",
                    status
                )))
            }
        };
    }

    // Re-validate invariants after update
    if let Err(e) = m.validate() {
        return Err(e.into());
    }

    state.db.save_month(&m)?;

    Ok(Json(pc.clone()))
}

// --- Allocation routes ---

pub async fn list_allocations(
    State(state): State<AppState>,
    Path(paycheck_id): Path<String>,
) -> Result<Json<Vec<paycheckzero_core::Allocation>>, crate::service::error::ServiceError> {
    let month = find_month_by_paycheck(&state.db, &paycheck_id)?;
    Ok(Json(month.allocations))
}

pub async fn create_allocation(
    State(state): State<AppState>,
    (Path(paycheck_id), Json(req)): (Path<String>, Json<CreateAllocationRequest>),
) -> Result<(StatusCode, Json<paycheckzero_core::Allocation>), crate::service::error::ServiceError> {
    let mut m = find_month_by_paycheck_mut(&state.db, &paycheck_id)?;

    m.allocate(
        &paycheckzero_core::Id::new(&paycheck_id),
        &paycheckzero_core::Id::new(&req.expense_line_id),
        paycheckzero_core::Cents::from_cents(req.amount),
    )?;

    state.db.save_month(&m)?;

    // Return the created allocation
    let alloc = m
        .allocations
        .iter()
        .find(|a| a.paycheck_id == Id::new(&paycheck_id) && a.expense_line_id == Id::new(&req.expense_line_id))
        .ok_or_else(|| crate::service::error::ServiceError::NotFound("allocation not found".into()))?;

    Ok((StatusCode::CREATED, Json(alloc.clone())))
}

pub async fn update_allocation(
    State(state): State<AppState>,
    (Path(id), Json(req)): (Path<String>, Json<UpdateAllocationRequest>),
) -> Result<Json<paycheckzero_core::Allocation>, crate::service::error::ServiceError> {
    let mut m = find_month_by_allocation(&state.db, &id)?;

    let alloc = m
        .allocations
        .iter_mut()
        .find(|a| a.id == Id::new(&id))
        .ok_or_else(|| crate::service::error::ServiceError::NotFound(format!("allocation not found: {id}")))?;

    alloc.amount = paycheckzero_core::Cents::from_cents(req.amount);

    if let Err(e) = m.validate() {
        return Err(e.into());
    }

    state.db.save_month(&m)?;

    Ok(Json(alloc.clone()))
}

pub async fn delete_allocation(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, crate::service::error::ServiceError> {
    let mut m = find_month_by_allocation(&state.db, &id)?;

    m.delete_allocation(&Id::new(&id))?;
    state.db.save_month(&m)?;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn transfer(
    State(state): State<AppState>,
    Json(req): Json<TransferRequest>,
) -> Result<StatusCode, crate::service::error::ServiceError> {
    let mut m = find_month_by_paycheck(&state.db, &req.from_paycheck_id)?;

    m.transfer(
        &paycheckzero_core::Id::new(&req.from_paycheck_id),
        &paycheckzero_core::Id::new(&req.to_paycheck_id),
        &paycheckzero_core::Id::new(&req.expense_line_id),
        paycheckzero_core::Cents::from_cents(req.amount),
    )?;

    state.db.save_month(&m)?;

    Ok(StatusCode::OK)
}

// --- Transaction routes ---

pub async fn list_transactions(
    State(state): State<AppState>,
    Path(month_id): Path<String>,
) -> Result<Json<Vec<Transaction>>, crate::service::error::ServiceError> {
    let m = state
        .db
        .load_month(&Id::new(month_id))?
        .ok_or_else(|| crate::service::error::ServiceError::NotFound(format!("month not found: {month_id}")))?;
    Ok(Json(m.transactions))
}

pub async fn create_transaction(
    State(state): State<AppState>,
    (Path(month_id), Json(req)): (Path<String>, Json<CreateTransactionRequest>),
) -> Result<(StatusCode, Json<Transaction>), crate::service::error::ServiceError> {
    let mut m = state
        .db
        .load_month(&Id::new(month_id))?
        .ok_or_else(|| crate::service::error::ServiceError::NotFound(format!("month not found: {month_id}")))?;

    let txn = Transaction {
        id: Id::generate(),
        date: paycheckzero_core::parse_date(&req.date)?,
        amount: paycheckzero_core::Cents::from_cents(req.amount),
        payee: req.payee,
        notes: req.notes,
        expense_line_id: req.expense_line_id.map(Id::new),
        paycheck_id: req.paycheck_id.map(Id::new),
    };

    m.add_transaction(txn.clone());
    state.db.save_month(&m)?;

    Ok((StatusCode::CREATED, Json(txn)))
}

// --- Derived views ---

pub async fn safe_to_spend(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<paycheckzero_core::views::PaycheckView>, crate::service::error::ServiceError> {
    let m = find_month_by_paycheck(&state.db, &id)?;
    let pc_view = m.paycheck_view(&Id::new(&id))?;
    Ok(Json(pc_view))
}

pub async fn month_summary(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<paycheckzero_core::views::MonthSummary>, crate::service::error::ServiceError> {
    let m = state
        .db
        .load_month(&Id::new(id))?
        .ok_or_else(|| crate::service::error::ServiceError::NotFound("month not found".into()))?;
    Ok(Json(m.summary()))
}

// --- Helpers ---

fn find_month_by_paycheck(
    repo: &paycheckzero_storage::SqliteRepository,
    paycheck_id: &str,
) -> Result<Month, crate::service::error::ServiceError> {
    // We need to iterate months to find the one containing this paycheck
    // For now, load all months and search
    for item in repo.list_months()? {
        if let Some(m) = repo.load_month(&item.id)? {
            if m.paychecks.iter().any(|p| p.id == Id::new(paycheck_id)) {
                return Ok(m);
            }
        }
    }
    Err(crate::service::error::ServiceError::NotFound(format!(
        "paycheck not found: {paycheck_id}"
    )))
}

fn find_month_by_paycheck_mut(
    repo: &paycheckzero_storage::SqliteRepository,
    paycheck_id: &str,
) -> Result<Month, crate::service::error::ServiceError> {
    find_month_by_paycheck(repo, paycheck_id)
}

fn find_month_by_allocation(
    repo: &paycheckzero_storage::SqliteRepository,
    allocation_id: &str,
) -> Result<Month, crate::service::error::ServiceError> {
    for item in repo.list_months()? {
        if let Some(m) = repo.load_month(&item.id)? {
            if m.allocations.iter().any(|a| a.id == Id::new(allocation_id)) {
                return Ok(m);
            }
        }
    }
    Err(crate::service::error::ServiceError::NotFound(format!(
        "allocation not found: {allocation_id}"
    )))
}
