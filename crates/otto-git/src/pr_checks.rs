//! Per-check PR CI status (git batch WP4) — routes are merged into `crate::http::router`.
//!
//! The PR header already shows the aggregate (`CiStatus`); the merge modal
//! needs the rows behind it, so this endpoint returns both from one round trip.

use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Extension, Json, Router};
use otto_core::auth::AuthUser;
use otto_core::domain::WorkspaceRole;
use otto_core::Id;
use serde::Serialize;

use crate::http::{provider_ctx, repo_ctx, ApiResult, GitCtx};
use crate::providers::PrCheck;

/// Response of `GET /repos/{id}/prs/{number}/checks`.
#[derive(Debug, Serialize)]
pub struct PrChecksResp {
    /// Aggregate state/counts — the same value `PrDetail` carries.
    pub ci: crate::types::CiStatus,
    /// One row per check-run / job / commit status (empty when there is no CI).
    pub checks: Vec<PrCheck>,
}

/// Routes owned by this module.
pub fn router<S: GitCtx>() -> Router<S> {
    Router::new().route("/repos/{id}/prs/{number}/checks", get(pr_checks::<S>))
}

async fn pr_checks<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path((id, number)): Path<(Id, u64)>,
) -> ApiResult<Json<PrChecksResp>> {
    let (repo, _) = repo_ctx(&s, &user, &id, WorkspaceRole::Viewer).await?;
    let (provider, remote) = provider_ctx(&s, &user, &repo).await?;
    // `ci_status` is best-effort by contract (never errors); only the per-row
    // list can fail, and a failure there is a real provider error worth showing.
    Ok(Json(PrChecksResp {
        ci: provider.ci_status(&remote, number).await,
        checks: provider.list_checks(&remote, number).await?,
    }))
}
