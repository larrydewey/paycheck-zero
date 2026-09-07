//! Export routes (CSV).

use axum::{
    extract::Path,
    http::StatusCode,
    routing::get,
    Router,
    response::IntoResponse,
};

use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/months/{id}/export", get(csv_export))
}

pub async fn csv_export(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let m = match state.db.load_month(&paycheckzero_core::Id::new(&id)) {
        Ok(Some(m)) => m,
        Ok(None) | Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                axum::Json(serde_json::json!({"error": {"code": "NOT_FOUND", "message": "month not found"}})),
            )
                .into_response();
        }
    };

    let summary = m.summary();

    let mut csv = String::from("Category,Line,Planned,Spent,Remaining\n");
    for cat in &summary.categories {
        for line in &cat.lines {
            csv.push_str(&format!(
                "{},{},{},{},{}\n",
                cat.name, line.name, line.planned, line.spent, line.remaining
            ));
        }
    }

    (
        StatusCode::OK,
        [
            (axum::http::header::CONTENT_TYPE, "text/csv"),
            (axum::http::header::CONTENT_DISPOSITION, "attachment; filename=month.csv"),
        ],
        csv,
    )
        .into_response()
}
