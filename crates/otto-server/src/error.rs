//! `otto_core::Error` → HTTP response mapping.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use otto_core::api::Problem;

/// Wrapper making `otto_core::Error` an axum response: status per variant +
/// JSON body `{"code","message"}`.
#[derive(Debug)]
pub struct ApiError(pub otto_core::Error);

/// Handler result alias.
pub type ApiResult<T> = std::result::Result<T, ApiError>;

impl From<otto_core::Error> for ApiError {
    fn from(e: otto_core::Error) -> Self {
        ApiError(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        use otto_core::Error as E;
        let status = match &self.0 {
            E::NotFound(_) => StatusCode::NOT_FOUND,
            E::Unauthorized => StatusCode::UNAUTHORIZED,
            E::Forbidden(_) => StatusCode::FORBIDDEN,
            E::Conflict(_) => StatusCode::CONFLICT,
            E::Invalid(_) => StatusCode::BAD_REQUEST,
            E::Upstream(_) => StatusCode::BAD_GATEWAY,
            E::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        if status == StatusCode::INTERNAL_SERVER_ERROR {
            tracing::error!("internal error: {}", self.0);
        }
        let body = Problem {
            code: self.0.code().to_string(),
            message: self.0.to_string(),
        };
        (status, Json(body)).into_response()
    }
}
