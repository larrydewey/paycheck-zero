//! Service-layer result type alias.

pub type ServiceResult<T> = Result<T, super::error::ServiceError>;
