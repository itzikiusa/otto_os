//! Per-check PR CI status (git batch WP4) — routes are merged into `crate::http::router`.

use axum::Router;

use crate::http::GitCtx;

/// Routes owned by this module (empty until the batch lands).
pub fn router<S: GitCtx>() -> Router<S> {
    Router::new()
}
