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

    #[error("unauthorized: {0}")]
    Unauthorized(String),

    #[error("internal error: {0}")]
    Internal(String),
}

impl axum::response::IntoResponse for ServiceError {
    fn into_response(self) -> axum::response::Response {
        let (status, error_code, message) = match &self {
            ServiceError::Domain(d) => {
                let code = d.code();
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
            ServiceError::Unauthorized(msg) => {
                (axum::http::StatusCode::UNAUTHORIZED, "UNAUTHORIZED".into(), msg.clone())
            }
            ServiceError::Internal(msg) => {
                (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR".into(), msg.clone())
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
