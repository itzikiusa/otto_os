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
            E::PayloadTooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
            E::UnsupportedMedia(_) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            E::Upstream(_) => StatusCode::BAD_GATEWAY,
            E::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        if status == StatusCode::INTERNAL_SERVER_ERROR {
            tracing::error!("internal error: {}", self.0);
        }
        let body = Problem {
            code: self.0.code().to_string(),
            message: self.0.public_message(),
        };
        (status, Json(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn internal_error_body_does_not_leak_raw_details() {
        let resp = ApiError(otto_core::Error::Internal(
            "sqlite error on /Users/alice/.otto/otto.db: disk I/O".into(),
        ))
        .into_response();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let bytes = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["code"], "internal");
        assert_eq!(body["message"], "internal server error");
        assert!(!body.to_string().contains("/Users/alice"));
    }

    #[tokio::test]
    async fn client_errors_keep_their_message() {
        let resp = ApiError(otto_core::Error::Invalid("bad field".into())).into_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let bytes = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["code"], "invalid");
        assert_eq!(body["message"], "invalid: bad field");
    }
}
