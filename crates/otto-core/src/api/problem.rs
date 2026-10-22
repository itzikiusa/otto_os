//! Problem response.
//!
//! Split out of `api.rs` by area (re-exported from there, so every path is
//! still `otto_core::api::*`). Mirrored by `ui/src/lib/api/types/core.ts`.
use super::*;

// ---------------------------------------------------------------------------
// Problem response
// ---------------------------------------------------------------------------

/// Error body returned by every endpoint on failure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Problem {
    pub code: String,
    pub message: String,
}
