//! Custom extractors for auth.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::response::Response;
use axum_extra::headers;
use axum_extra::headers::Authorization;
use axum_extra::TypedHeader;

use crate::UserId;

/// Extract the current user from the Bearer token in the Authorization header.
pub struct CurrentUser(pub UserId);

#[cfg(not(test))]
impl<S> FromRequestParts<S> for CurrentUser
where
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let TypedHeader(Authorization(bearer)) =
            TypedHeader::<Authorization<headers::Bearer>>::from_request_parts(parts, _state)
                .await
                .map_err(|_| {
                    let mut resp = Response::new(axum::http::StatusCode::UNAUTHORIZED.into());
                    *resp.body_mut() = axum::body::Body::from("missing or invalid bearer token");
                    resp
                })?;

        // TODO: validate JWT and extract user id
        // For now, return a placeholder
        Ok(CurrentUser(UserId(bearer.token().to_string())))
    }
}
