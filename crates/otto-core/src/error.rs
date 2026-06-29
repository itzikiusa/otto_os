//! Unified error type. otto-server maps each variant to an HTTP status.

/// Workspace-wide error enum; crates converge their failures into this type.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Entity does not exist (HTTP 404).
    #[error("not found: {0}")]
    NotFound(String),
    /// Missing or invalid credentials (HTTP 401).
    #[error("unauthorized")]
    Unauthorized,
    /// Authenticated but not allowed (HTTP 403).
    #[error("forbidden: {0}")]
    Forbidden(String),
    /// State conflict, e.g. duplicate username (HTTP 409).
    #[error("conflict: {0}")]
    Conflict(String),
    /// Invalid input (HTTP 400).
    #[error("invalid: {0}")]
    Invalid(String),
    /// Payload exceeds an allowed size cap (HTTP 413). e.g. a proof media blob
    /// over `proof::MEDIA_CAP`.
    #[error("payload too large: {0}")]
    PayloadTooLarge(String),
    /// Unsupported media / content type (HTTP 415). e.g. a proof media blob whose
    /// MIME is not in the allow-list.
    #[error("unsupported media type: {0}")]
    UnsupportedMedia(String),
    /// Upstream dependency failed: git provider, CLI, network (HTTP 502).
    #[error("upstream: {0}")]
    Upstream(String),
    /// Unexpected internal failure (HTTP 500).
    #[error("internal: {0}")]
    Internal(String),
}

/// Convenience result alias used across all crates.
pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// Stable machine-readable code used in API problem responses.
    pub fn code(&self) -> &'static str {
        match self {
            Error::NotFound(_) => "not_found",
            Error::Unauthorized => "unauthorized",
            Error::Forbidden(_) => "forbidden",
            Error::Conflict(_) => "conflict",
            Error::Invalid(_) => "invalid",
            Error::PayloadTooLarge(_) => "payload_too_large",
            Error::UnsupportedMedia(_) => "unsupported_media_type",
            Error::Upstream(_) => "upstream",
            Error::Internal(_) => "internal",
        }
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Invalid(format!("json: {e}"))
    }
}
