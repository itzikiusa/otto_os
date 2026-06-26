//! Review **findings workflow** endpoints — the id-keyed action surface that
//! turns code-review output into a tracked security/code-quality workflow.
//!
//! Each finding (`crates/otto-state/review_findings`) carries the 11 required
//! fields + a 6-state workflow `status`. Every action here resolves the finding,
//! checks the workspace role, performs the transition / field patch (validated by
//! `FindingStatus::can_transition`), appends an immutable `finding_events` audit
//! row, emits `Event::FindingUpdated`, and returns the updated `Finding`.
//!
//! - `GET  /findings/{id}`                      — finding + its event timeline
//! - `POST /findings/{id}/accept`               — open → accepted
//! - `POST /findings/{id}/waive`                — → waived
//! - `POST /findings/{id}/false-positive`       — → false_positive
//! - `POST /findings/{id}/require-approval`     — set the human-approval gate
//! - `POST /findings/{id}/approve`              — approve (clear gate) / reject (→ FP)
//! - `POST /findings/{id}/jira`                 — convert to a Jira issue        (jira.rs pass)
//! - `POST /findings/{id}/repo-rule`            — generalize into a repo rule    (repo_rules pass)
//! - `POST /findings/{id}/fix|verify|regression-test` — agent-backed            (finding_agent pass)

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use otto_core::domain::{User, WorkspaceRole};
use otto_core::event::Event;
use otto_core::finding::{Finding, FindingDetail, FindingStatus};
use otto_state::FindingPatch;

use crate::auth::{require_ws_role, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

pub fn routes() -> Router<ServerCtx> {
    Router::new()
        .route("/findings/{id}", get(get_finding))
        .route("/findings/{id}/accept", post(accept))
        .route("/findings/{id}/waive", post(waive))
        .route("/findings/{id}/false-positive", post(false_positive))
        .route("/findings/{id}/require-approval", post(require_approval))
        .route("/findings/{id}/approve", post(approve))
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// The label recorded as a finding's `reviewer` / an event `actor`.
pub(crate) fn actor_label(user: &User) -> String {
    if user.display_name.trim().is_empty() {
        user.id.clone()
    } else {
        user.display_name.clone()
    }
}

/// Load a finding by id and enforce the caller's workspace role (the finding row
/// carries its own `workspace_id`, so no review/repo join is needed).
pub(crate) async fn load_for_role(
    ctx: &ServerCtx,
    user: &User,
    id: &str,
    min: WorkspaceRole,
) -> ApiResult<Finding> {
    let f = ctx.findings_store.get_full(id).await.map_err(ApiError)?;
    require_ws_role(ctx, user, &f.workspace_id, min).await?;
    Ok(f)
}

/// Broadcast a `finding_updated` so the board refetches live.
pub(crate) fn emit_updated(ctx: &ServerCtx, f: &Finding) {
    let _ = ctx.events.send(Event::FindingUpdated {
        workspace_id: f.workspace_id.clone(),
        review_id: f.review_id.clone(),
        finding_id: f.id.clone(),
        status: f.status.as_str().to_string(),
    });
}

/// Append one audit-trail event (best-effort log on failure).
pub(crate) async fn audit(
    ctx: &ServerCtx,
    f: &Finding,
    kind: &str,
    actor: &str,
    from: Option<&str>,
    to: Option<&str>,
    detail: serde_json::Value,
) {
    if let Err(e) = ctx
        .finding_events_store
        .append(&f.id, &f.workspace_id, kind, actor, from, to, detail)
        .await
    {
        tracing::warn!("finding audit append ({kind}): {e}");
    }
}

// ---------------------------------------------------------------------------
// Read
// ---------------------------------------------------------------------------

/// `GET /findings/{id}` → the finding plus its full event timeline.
async fn get_finding(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<FindingDetail>> {
    let finding = load_for_role(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    let events = ctx
        .finding_events_store
        .list_for_finding(&id)
        .await
        .map_err(ApiError)?;
    Ok(Json(FindingDetail { finding, events }))
}

// ---------------------------------------------------------------------------
// Pure-status triage actions
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ReasonReq {
    #[serde(default)]
    reason: Option<String>,
}

/// `POST /findings/{id}/accept` — open → accepted (this is a real issue to fix).
async fn accept(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Finding>> {
    let cur = load_for_role(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    let who = actor_label(&user);
    let f = ctx
        .findings_store
        .set_status(&id, FindingStatus::Accepted, &who)
        .await
        .map_err(ApiError)?;
    audit(&ctx, &f, "accepted", &who, Some(cur.status.as_str()), Some("accepted"), serde_json::json!({})).await;
    emit_updated(&ctx, &f);
    Ok(Json(f))
}

/// `POST /findings/{id}/waive` — accepted risk; intentionally not fixing.
async fn waive(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(body): Json<ReasonReq>,
) -> ApiResult<Json<Finding>> {
    let cur = load_for_role(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    let who = actor_label(&user);
    let f = ctx
        .findings_store
        .set_status(&id, FindingStatus::Waived, &who)
        .await
        .map_err(ApiError)?;
    audit(&ctx, &f, "waived", &who, Some(cur.status.as_str()), Some("waived"),
        serde_json::json!({ "reason": body.reason })).await;
    emit_updated(&ctx, &f);
    Ok(Json(f))
}

/// `POST /findings/{id}/false-positive` — not a real issue.
async fn false_positive(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(body): Json<ReasonReq>,
) -> ApiResult<Json<Finding>> {
    let cur = load_for_role(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    let who = actor_label(&user);
    let f = ctx
        .findings_store
        .set_status(&id, FindingStatus::FalsePositive, &who)
        .await
        .map_err(ApiError)?;
    audit(&ctx, &f, "false_positive", &who, Some(cur.status.as_str()), Some("false_positive"),
        serde_json::json!({ "reason": body.reason })).await;
    emit_updated(&ctx, &f);
    Ok(Json(f))
}

/// `POST /findings/{id}/require-approval` — flag that a human must sign off
/// before this finding can be auto-closed. Orthogonal to status (the gate).
async fn require_approval(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Finding>> {
    let _cur = load_for_role(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    let who = actor_label(&user);
    let f = ctx
        .findings_store
        .set_fields(&id, &FindingPatch { requires_human_approval: Some(true), ..Default::default() })
        .await
        .map_err(ApiError)?;
    audit(&ctx, &f, "approval_required", &who, None, None, serde_json::json!({})).await;
    emit_updated(&ctx, &f);
    Ok(Json(f))
}

#[derive(Deserialize)]
struct ApproveReq {
    /// "approve" | "reject"
    decision: String,
    #[serde(default)]
    note: Option<String>,
}

/// `POST /findings/{id}/approve` — resolve the human-approval gate. `approve`
/// clears the gate (and accepts an `open` finding); `reject` → `false_positive`.
async fn approve(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(body): Json<ApproveReq>,
) -> ApiResult<Json<Finding>> {
    let cur = load_for_role(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    let who = actor_label(&user);
    let now = chrono::Utc::now().to_rfc3339();
    let approved = matches!(body.decision.as_str(), "approve" | "approved" | "accept");
    let decision = if approved { "approved" } else { "rejected" };

    // Record the decision + clear the gate.
    let mut f = ctx
        .findings_store
        .set_fields(
            &id,
            &FindingPatch {
                requires_human_approval: Some(false),
                approved_by: Some(who.clone()),
                approved_at: Some(now),
                approval_decision: Some(decision.to_string()),
                ..Default::default()
            },
        )
        .await
        .map_err(ApiError)?;

    // Apply the status effect.
    if approved {
        // Accept an untriaged finding; otherwise leave its disposition as-is.
        if cur.status == FindingStatus::Open {
            f = ctx
                .findings_store
                .set_status(&id, FindingStatus::Accepted, &who)
                .await
                .map_err(ApiError)?;
        }
    } else {
        f = ctx
            .findings_store
            .set_status(&id, FindingStatus::FalsePositive, &who)
            .await
            .map_err(ApiError)?;
    }
    audit(&ctx, &f, decision, &who, Some(cur.status.as_str()), Some(f.status.as_str()),
        serde_json::json!({ "note": body.note })).await;
    emit_updated(&ctx, &f);
    Ok(Json(f))
}
