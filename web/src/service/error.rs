//! Service-layer error types.

use paycheckzero_core::DomainError;

/// Errors that occur in the service layer.
#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("domain error: {0}")]
    Domain(#[from] DomainError),

    #[error("storage error: {0}")]
    Storage(#[from] paycheckzero_storage::StorageError),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("conflict: {0}")]
    Conflict(String),
}

impl axum::response::IntoResponse for ServiceError {
    fn into_response(self) -> axum::response::Response {
        let (status, error_code, message) = match &self {
            ServiceError::Domain(d) => {
                let code = d.error_code();
                let msg = d.to_string();
                if code.starts_with("INVARIANT_") {
                    (axum::http::StatusCode::CONFLICT, code.to_string(), msg)
                } else {
                    (axum::http::StatusCode::BAD_REQUEST, code.to_string(), msg)
                }
            }
            ServiceError::Storage(_) => {
                (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "STORAGE_ERROR".into(), self.to_string())
            }
            ServiceError::NotFound(msg) => {
                (axum::http::StatusCode::NOT_FOUND, "NOT_FOUND".into(), msg.clone())
            }
            ServiceError::Conflict(msg) => {
                (axum::http::StatusCode::CONFLICT, "CONFLICT".into(), msg.clone())
            }
        };

        let body = serde_json::json!({
            "error": {
                "code": error_code,
                "message": message,
            }
        });

        (status, axum::Json(body)).into_response()
    }
}
