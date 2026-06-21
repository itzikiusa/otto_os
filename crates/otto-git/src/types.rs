//! Local extended types for the otto-git crate: CI check-run aggregation and
//! merge-readiness assembly. These are kept in otto-git (not otto-core) because
//! they are specific to provider integration details and are not part of the
//! shared cross-crate API surface.

use serde::{Deserialize, Serialize};

/// Aggregated CI / check-run status for a pull request.
/// Populated by each provider's `fetch_ci_status` call and serialised into
/// `PrSummary.ci_status` (as the `state` string for backward compat) and
/// returned verbatim from the merge-readiness endpoint.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CiStatus {
    /// Aggregated state: "success" | "failure" | "pending" | "none".
    pub state: String,
    /// Total number of CI checks / pipelines / commit statuses seen.
    pub total: u32,
    /// Number of checks in a passing/success terminal state.
    pub passed: u32,
    /// Number of checks in a failing terminal state.
    pub failed: u32,
    /// Best-effort URL to the CI summary page (provider dashboard / run page).
    pub url: Option<String>,
}

impl CiStatus {
    /// Construct a "none" sentinel (no CI data available).
    pub fn none() -> Self {
        Self {
            state: "none".to_string(),
            ..Default::default()
        }
    }
}
