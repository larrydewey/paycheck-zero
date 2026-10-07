//! Application errors and their human-readable presentation (spec §13.3):
//! a clear, actionable primary message plus technical details.

use crate::i18n::{t, tf};
use crate::money;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use paycheckzero_core::{DomainError, Month};
use paycheckzero_storage::StorageError;
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error(transparent)]
    Domain(#[from] DomainError),
    #[error("not found")]
    NotFound,
    #[error("unauthorized")]
    Unauthorized,
    #[error("registration closed")]
    RegistrationClosed,
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("month archived")]
    Archived,
    #[error("month exists")]
    MonthExists,
    #[error("stale")]
    Stale,
    #[error("invalid credentials")]
    InvalidCredentials,
    #[error("internal: {0}")]
    Internal(String),
}

impl From<StorageError> for AppError {
    fn from(e: StorageError) -> Self {
        match e {
            StorageError::Conflict => AppError::Stale,
            StorageError::Duplicate => AppError::MonthExists,
            other => AppError::Internal(other.to_string()),
        }
    }
}

pub type AppResult<T> = Result<T, AppError>;

impl AppError {
    #[must_use]
    pub fn bad(msg: impl Into<String>) -> Self {
        AppError::BadRequest(msg.into())
    }

    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            AppError::Domain(d) => d.code(),
            AppError::NotFound => "NOT_FOUND",
            AppError::Unauthorized => "UNAUTHORIZED",
            AppError::RegistrationClosed => "REGISTRATION_CLOSED",
            AppError::BadRequest(_) => "BAD_REQUEST",
            AppError::Archived => "MONTH_ARCHIVED",
            AppError::MonthExists => "MONTH_EXISTS",
            AppError::Stale => "STALE_WRITE",
            AppError::InvalidCredentials => "INVALID_CREDENTIALS",
            AppError::Internal(_) => "INTERNAL",
        }
    }

    #[must_use]
    pub fn status(&self) -> StatusCode {
        match self {
            AppError::Domain(DomainError::NotFound { .. }) | AppError::NotFound => StatusCode::NOT_FOUND,
            AppError::Domain(d) if d.is_conflict() || matches!(d, DomainError::NonPositiveAmount) => StatusCode::CONFLICT,
            AppError::Domain(_) => StatusCode::UNPROCESSABLE_ENTITY,
            AppError::Unauthorized | AppError::InvalidCredentials => StatusCode::UNAUTHORIZED,
            AppError::RegistrationClosed => StatusCode::FORBIDDEN,
            AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
            AppError::Archived | AppError::MonthExists | AppError::Stale => StatusCode::CONFLICT,
            AppError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Primary message: human, specific and actionable.
    #[must_use]
    pub fn human(&self, currency: &str, month: Option<&Month>) -> String {
        self.human_with(&|c| money::format(c, currency), month)
    }

    /// Primary message with a custom money formatter (the API uses cents).
    #[must_use]
    pub fn human_with(&self, m: &dyn Fn(paycheckzero_core::Cents) -> String, month: Option<&Month>) -> String {
        let pc_date = |id: &paycheckzero_core::Id| {
            month
                .and_then(|mo| mo.paycheck(id))
                .map(|p| p.date.format("%b %-d").to_string())
                .unwrap_or_default()
        };
        match self {
            AppError::Domain(d) => match d {
                DomainError::InsufficientUnallocated { short } => tf("err.insufficient", &[("amount", &m(*short))]),
                DomainError::TransferExceedsAllocation { available, .. } => {
                    tf("err.transfer_exceeds", &[("amount", &m(*available))])
                }
                DomainError::NotZero { diff } => {
                    if diff.is_positive() {
                        tf("err.not_zero_left", &[("amount", &m(*diff))])
                    } else {
                        tf("err.not_zero_over", &[("amount", &m(diff.abs()))])
                    }
                }
                DomainError::NonPositiveAmount => t("err.non_positive"),
                DomainError::NegativeAmount => t("err.negative"),
                DomainError::ZeroTransaction => t("err.zero_transaction"),
                DomainError::InvalidTransfer => t("err.invalid_transfer"),
                DomainError::SplitTooFew => t("err.split_too_few"),
                DomainError::SplitMixedSigns => t("err.split_mixed"),
                DomainError::Locked => t("err.locked"),
                DomainError::AlreadyLocked => t("err.already_locked"),
                DomainError::NotLocked => t("err.not_locked"),
                DomainError::NotReassigning => t("err.not_reassigning"),
                DomainError::NoVariance => t("err.no_variance"),
                DomainError::UnresolvedVariance(p) => tf("err.unresolved_variance", &[("date", &pc_date(p))]),
                DomainError::NoActual(_) => t("err.no_actual"),
                DomainError::PaycheckSkipped(_) => t("err.skipped"),
                DomainError::DuplicatePaycheck(d) => tf("err.duplicate_paycheck", &[("date", &d.format("%b %-d").to_string())]),
                DomainError::DateOutsideMonth(d) => tf("err.date_outside", &[("date", &d.format("%b %-d, %Y").to_string())]),
                DomainError::NoDatesInMonth => t("err.no_dates"),
                DomainError::InvalidRecurrence => t("err.invalid_recurrence"),
                DomainError::InvalidName { max } => tf("err.invalid_name", &[("max", &max.to_string())]),
                DomainError::NotDebtLine => t("err.not_debt"),
                DomainError::NotFound { .. } => t("err.not_found"),
                DomainError::Invariant(_) => t("err.invariant"),
            },
            AppError::NotFound => t("err.not_found"),
            AppError::Unauthorized => t("err.unauthorized"),
            AppError::RegistrationClosed => t("err.registration_closed"),
            AppError::BadRequest(msg) => msg.clone(),
            AppError::Archived => t("err.archived"),
            AppError::MonthExists => t("err.month_exists"),
            AppError::Stale => t("err.stale"),
            AppError::InvalidCredentials => t("err.invalid_credentials"),
            AppError::Internal(_) => t("err.internal"),
        }
    }

    /// Secondary, technical details (collapsible in the UI).
    #[must_use]
    pub fn technical(&self) -> String {
        match self {
            AppError::Internal(_) => format!("{} (see server log)", self.code()),
            other => format!("{}: {other}", other.code()),
        }
    }

    fn details_json(&self) -> serde_json::Value {
        match self {
            AppError::Domain(DomainError::NotZero { diff }) => json!({ "difference_cents": diff }),
            AppError::Domain(DomainError::InsufficientUnallocated { short }) => json!({ "short_cents": short }),
            AppError::Domain(DomainError::TransferExceedsAllocation { requested, available }) => {
                json!({ "requested_cents": requested, "available_cents": available })
            }
            _ => json!({}),
        }
    }
}

/// JSON error body for the REST API (spec §9 error format).
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        if let AppError::Internal(msg) = &self {
            tracing::error!("internal error: {msg}");
        }
        let body = json!({
            "error": {
                "code": self.code(),
                "message": self.human_with(&|c| format!("{} cents", c.get()), None),
                "technical": self.technical(),
                "details": self.details_json(),
            }
        });
        (self.status(), Json(body)).into_response()
    }
}
