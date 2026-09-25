//! Report endpoints (§15): MoM, YTD, YoY, quick-win summary cards.

use axum::{
    Json,
    extract::{Path, State},
    routing::get,
    Router,
};

use chrono::Datelike;
use paycheckzero_core::{Id, Month};
use paycheckzero_storage::Repository;

use crate::report::{self, MoMReport, SummaryCards, YoYReport, YtdReport};
use crate::service::error::ServiceError;
use crate::AppState;

/// Router for the reports API. Mounted at `/api/v1`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/months/{month_id}/reports/mom", get(mom))
        .route("/months/{month_id}/reports/ytd", get(ytd))
        .route("/months/{month_id}/reports/yoy", get(yoy))
        .route("/months/{month_id}/reports/summary", get(summary))
}

fn load_all_months(state: &AppState) -> Result<Vec<Month>, ServiceError> {
    let db = state.db.lock().unwrap();
    let ids: Vec<Id> = db.list_months()?.into_iter().map(|i| i.id).collect();
    let mut months = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(m) = db.load_month(&id)? {
            months.push(m);
        }
    }
    Ok(months)
}

fn load_month(state: &AppState, id: &str) -> Result<Month, ServiceError> {
    let db = state.db.lock().unwrap();
    db.load_month(&Id::new(id))?
        .ok_or_else(|| ServiceError::NotFound(format!("month not found: {id}")))
}

pub async fn mom(
    State(state): State<AppState>,
    Path(month_id): Path<String>,
) -> Result<Json<MoMReport>, ServiceError> {
    let selected = load_month(&state, &month_id)?;
    let months = load_all_months(&state)?;
    let prev = report::find_month(
        &months,
        report::previous_month_start(selected.year_month),
    );
    Ok(Json(report::mom_report(&selected, prev)))
}

pub async fn ytd(
    State(state): State<AppState>,
    Path(month_id): Path<String>,
) -> Result<Json<YtdReport>, ServiceError> {
    let selected = load_month(&state, &month_id)?;
    let months = load_all_months(&state)?;
    let in_year: Vec<&Month> = months
        .iter()
        .filter(|m| report::in_year_through_month(m.year_month, selected.year_month))
        .collect();
    Ok(Json(report::ytd_report(selected.year_month.year(), &in_year)))
}

pub async fn yoy(
    State(state): State<AppState>,
    Path(month_id): Path<String>,
) -> Result<Json<YoYReport>, ServiceError> {
    let selected = load_month(&state, &month_id)?;
    let months = load_all_months(&state)?;
    let prev = report::previous_year_month_start(selected.year_month)
        .and_then(|start| report::find_month(&months, start));
    Ok(Json(report::yoy_report(&selected, prev)))
}

pub async fn summary(
    State(state): State<AppState>,
    Path(month_id): Path<String>,
) -> Result<Json<SummaryCards>, ServiceError> {
    let selected = load_month(&state, &month_id)?;
    Ok(Json(report::summary_cards(&selected)))
}