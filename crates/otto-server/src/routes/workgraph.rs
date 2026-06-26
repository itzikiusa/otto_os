//! Mission Control / work-graph HTTP API.
//!
//! Read-mostly surface over the projected work graph (see
//! `crate::workgraph_projector`). All routes nest under
//! `/workspaces/{wid}/workgraph/…` so a single `policy.rs` rule
//! (`Require(MissionControl, View|Edit)`) covers them; handlers add the
//! orthogonal workspace-role gate. Writes are limited to human annotation
//! (risk/goal/result), manual edges, approvals, and a re-derivation backfill —
//! the graph is otherwise a projection, never user-authored.

use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use otto_core::domain::WorkspaceRole;
use otto_core::{Error, Id};
use otto_state::{
    ApprovalStatus, EdgeRelation, GraphView, MissionFilter, MissionSummary, RiskLevel, WorkActor,
    WorkApproval, WorkEdge, WorkItem, WorkItemDetail, WorkKind, WorkStatus,
};
use serde::{Deserialize, Serialize};

use crate::auth::{require_ws_role, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

// ---------------------------------------------------------------------------
// Query / body DTOs
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
pub struct ItemQuery {
    pub kind: Option<String>,
    pub status: Option<String>,
    pub risk: Option<String>,
    pub q: Option<String>,
    pub limit: Option<i64>,
}

impl ItemQuery {
    fn into_filter(self) -> MissionFilter {
        MissionFilter {
            kind: self.kind.as_deref().and_then(WorkKind::parse),
            status: self.status.as_deref().and_then(WorkStatus::parse),
            risk: self.risk.as_deref().and_then(RiskLevel::parse),
            q: self.q.filter(|s| !s.trim().is_empty()),
            limit: self.limit,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct WorkItemPatch {
    pub risk_level: Option<String>,
    pub goal: Option<String>,
    pub result_summary: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct EdgeReq {
    pub to_item_id: Id,
    pub relation: String,
}

#[derive(Debug, Deserialize)]
pub struct ApprovalReq {
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DecideReq {
    /// "approved" | "rejected".
    pub decision: String,
    pub note: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct BackfillResp {
    pub ok: bool,
    pub summary: MissionSummary,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /workspaces/{wid}/workgraph/summary`
pub async fn get_summary(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<MissionSummary>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    let s = ctx.workgraph.repo().summary(&wid).await.map_err(ApiError)?;
    Ok(Json(s))
}

/// `GET /workspaces/{wid}/workgraph/items`
pub async fn list_items(
    Path(wid): Path<Id>,
    Query(q): Query<ItemQuery>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<WorkItem>>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    let items = ctx
        .workgraph
        .repo()
        .list_items(&wid, &q.into_filter())
        .await
        .map_err(ApiError)?;
    Ok(Json(items))
}

/// `GET /workspaces/{wid}/workgraph/graph`
pub async fn get_graph(
    Path(wid): Path<Id>,
    Query(q): Query<ItemQuery>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<GraphView>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    let g = ctx
        .workgraph
        .repo()
        .graph(&wid, &q.into_filter())
        .await
        .map_err(ApiError)?;
    Ok(Json(g))
}

/// `GET /workspaces/{wid}/workgraph/items/{id}`
pub async fn get_item(
    Path((wid, id)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<WorkItemDetail>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    // Refresh this item's cost on-demand (session/external_trigger → one usage
    // query) so the detail shows live cost without a background ClickHouse sweep.
    let item = ctx.workgraph.repo().get_item(&wid, &id).await.map_err(ApiError)?;
    crate::workgraph_projector::refresh_item_cost(&ctx, &wid, &item).await;
    let d = ctx
        .workgraph
        .repo()
        .item_detail(&wid, &id)
        .await
        .map_err(ApiError)?;
    Ok(Json(d))
}

/// `PATCH /workspaces/{wid}/workgraph/items/{id}` — annotate risk/goal/result.
pub async fn patch_item(
    Path((wid, id)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(body): Json<WorkItemPatch>,
) -> ApiResult<Json<WorkItem>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let risk = match body.risk_level.as_deref() {
        Some(s) => Some(
            RiskLevel::parse(s)
                .ok_or_else(|| ApiError(Error::Invalid(format!("bad risk_level '{s}'"))))?,
        ),
        None => None,
    };
    let item = ctx
        .workgraph
        .patch_item(&wid, &id, risk, body.goal, body.result_summary)
        .await
        .map_err(ApiError)?;
    Ok(Json(item))
}

/// `POST /workspaces/{wid}/workgraph/items/{id}/edges` — manual link.
pub async fn add_edge(
    Path((wid, id)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(body): Json<EdgeReq>,
) -> ApiResult<Json<WorkEdge>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let relation = EdgeRelation::parse(&body.relation)
        .ok_or_else(|| ApiError(Error::Invalid(format!("bad relation '{}'", body.relation))))?;
    // Validate both endpoints live in this workspace.
    ctx.workgraph.repo().get_item(&wid, &id).await.map_err(ApiError)?;
    ctx.workgraph
        .repo()
        .get_item(&wid, &body.to_item_id)
        .await
        .map_err(ApiError)?;
    let edge = ctx
        .workgraph
        .add_edge_emit(&wid, &id, &body.to_item_id, relation, WorkActor::User)
        .await
        .map_err(ApiError)?;
    Ok(Json(edge))
}

/// `POST /workspaces/{wid}/workgraph/items/{id}/approvals` — request a gate.
pub async fn request_approval(
    Path((wid, id)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(body): Json<ApprovalReq>,
) -> ApiResult<Json<WorkApproval>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    // Ensure the item exists in this workspace before opening a gate.
    ctx.workgraph.repo().get_item(&wid, &id).await.map_err(ApiError)?;
    let ap = ctx
        .workgraph
        .request_approval(&wid, &id, body.reason, &user.id)
        .await
        .map_err(ApiError)?;
    Ok(Json(ap))
}

/// `POST /workspaces/{wid}/workgraph/approvals/{aid}/decide`
pub async fn decide_approval(
    Path((wid, aid)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(body): Json<DecideReq>,
) -> ApiResult<Json<WorkApproval>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let status = match body.decision.as_str() {
        "approved" | "approve" => ApprovalStatus::Approved,
        "rejected" | "reject" => ApprovalStatus::Rejected,
        other => {
            return Err(ApiError(Error::Invalid(format!(
                "decision must be approved|rejected, got '{other}'"
            ))))
        }
    };
    let ap = ctx
        .workgraph
        .decide_approval(&wid, &aid, status, &user.id, body.note)
        .await
        .map_err(ApiError)?;
    Ok(Json(ap))
}

/// `POST /workspaces/{wid}/workgraph/backfill` — re-derive from source repos.
pub async fn backfill(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<BackfillResp>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    // Scope the user-triggered "Refresh" to THIS workspace (don't re-derive every
    // other workspace's sources), then refresh its session costs (a brief,
    // deliberate ClickHouse burst, bounded to one workspace).
    crate::workgraph_projector::backfill_one(&ctx, &wid).await;
    crate::workgraph_projector::refresh_session_costs(&ctx, &wid).await;
    let summary = ctx.workgraph.repo().summary(&wid).await.map_err(ApiError)?;
    Ok(Json(BackfillResp { ok: true, summary }))
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Work-graph routes; mounted as an api_extra in `module_routers`. Every path
/// nests under `/workspaces/{wid}/workgraph/` so one policy rule covers all.
pub fn workgraph_routes() -> Router<ServerCtx> {
    Router::new()
        .route("/workspaces/{wid}/workgraph/summary", get(get_summary))
        .route("/workspaces/{wid}/workgraph/items", get(list_items))
        .route("/workspaces/{wid}/workgraph/graph", get(get_graph))
        .route(
            "/workspaces/{wid}/workgraph/items/{id}",
            get(get_item).patch(patch_item),
        )
        .route(
            "/workspaces/{wid}/workgraph/items/{id}/edges",
            post(add_edge),
        )
        .route(
            "/workspaces/{wid}/workgraph/items/{id}/approvals",
            post(request_approval),
        )
        .route(
            "/workspaces/{wid}/workgraph/approvals/{aid}/decide",
            post(decide_approval),
        )
        .route("/workspaces/{wid}/workgraph/backfill", post(backfill))
}
