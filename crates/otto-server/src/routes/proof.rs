//! HTTP routes for Proof Packs. The engine (assembly, recompute, gates) lives in
//! [`crate::proof`]; this module is the REST surface. Feature-axis access is
//! enforced by `policy.rs` (`Feature::ProofPack`); each handler additionally
//! checks the caller's workspace role.

use axum::extract::{Path, Query, State};
use axum::routing::{delete, get, post};
use axum::{Extension, Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use otto_core::api::{
    AddArtifactReq, AssembleReq, CreateProofPackReq, ProofArtifactView, ProofPackDetailResp,
    ProofPackResp, ProofSummaryResp, ProofSummaryRow, WaiveReq,
};
use otto_core::auth::AuthUser;
use otto_core::domain::WorkspaceRole;
use otto_core::proof::{
    ProofArtifact, ProofArtifactKind, ProofArtifactStatus, ProofPack, WorkItemKind,
};
use otto_core::{Error, Id};

use crate::error::{ApiError, ApiResult};
use crate::proof as engine;
use crate::state::ServerCtx;

pub fn routes() -> Router<ServerCtx> {
    Router::new()
        .route(
            "/workspaces/{id}/proof-packs",
            get(list).post(create),
        )
        .route("/workspaces/{id}/proof-summary", get(summary))
        .route(
            "/proof-packs/{id}",
            get(detail).patch(patch_pack).delete(remove),
        )
        .route("/proof-packs/{id}/artifacts", post(add_artifact))
        .route("/proof-packs/{id}/assemble", post(assemble))
        .route("/proof-packs/{id}/waive", post(waive))
        .route("/proof-artifacts/{id}", delete(remove_artifact))
        .route("/proof-artifacts/{id}/content", get(artifact_content))
}

// --- helpers ---------------------------------------------------------------

async fn check(ctx: &ServerCtx, user: &AuthUser, ws: &Id, role: WorkspaceRole) -> ApiResult<()> {
    ctx.roles.check(&user.0, ws, role).await.map_err(ApiError)
}

/// Resolve a pack and verify the caller's role on its workspace.
async fn pack_for(
    ctx: &ServerCtx,
    user: &AuthUser,
    id: &Id,
    role: WorkspaceRole,
) -> ApiResult<ProofPack> {
    let pack = ctx.proof_repo.get_pack(id).await.map_err(ApiError)?;
    check(ctx, user, &pack.workspace_id, role).await?;
    Ok(pack)
}

fn parse_kind(s: &str) -> ApiResult<ProofArtifactKind> {
    ProofArtifactKind::parse(s)
        .ok_or_else(|| ApiError(Error::Invalid(format!("unknown artifact kind '{s}'"))))
}

fn parse_work_kind(s: &str) -> ApiResult<WorkItemKind> {
    WorkItemKind::parse(s)
        .ok_or_else(|| ApiError(Error::Invalid(format!("unknown work item kind '{s}'"))))
}

async fn pack_resp(ctx: &ServerCtx, pack: ProofPack) -> ApiResult<ProofPackResp> {
    let arts = ctx
        .proof_repo
        .list_artifacts(&pack.id)
        .await
        .map_err(ApiError)?;
    let badges = engine::badge_strings(&pack, &arts);
    Ok(ProofPackResp {
        badges,
        artifact_count: arts.len() as u32,
        pack,
    })
}

fn artifact_view(a: ProofArtifact) -> ProofArtifactView {
    // Inline content gets a capped preview; url/file refs do not.
    let ref_kind = a
        .metadata
        .get("ref_kind")
        .and_then(|v| v.as_str())
        .unwrap_or("inline");
    let (preview, truncated) = if ref_kind == "inline" {
        match &a.content_ref {
            Some(c) => {
                let (p, t) = engine::preview(c);
                (Some(p), t)
            }
            None => (None, false),
        }
    } else {
        (None, false)
    };
    ProofArtifactView {
        artifact: a,
        preview,
        truncated,
    }
}

// --- handlers --------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ListQuery {
    status: Option<String>,
    work_item_kind: Option<String>,
    work_item_id: Option<String>,
}

async fn list(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(ws): Path<Id>,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Vec<ProofPackResp>>> {
    check(&ctx, &user, &ws, WorkspaceRole::Viewer).await?;
    let packs = ctx
        .proof_repo
        .list_packs(
            &ws,
            q.status.as_deref(),
            q.work_item_kind.as_deref(),
            q.work_item_id.as_deref(),
        )
        .await
        .map_err(ApiError)?;
    let mut out = Vec::with_capacity(packs.len());
    for p in packs {
        out.push(pack_resp(&ctx, p).await?);
    }
    Ok(Json(out))
}

async fn summary(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(ws): Path<Id>,
) -> ApiResult<Json<ProofSummaryResp>> {
    check(&ctx, &user, &ws, WorkspaceRole::Viewer).await?;
    let packs = ctx
        .proof_repo
        .list_packs(&ws, None, None, None)
        .await
        .map_err(ApiError)?;
    let mut rows = Vec::with_capacity(packs.len());
    for p in packs {
        let arts = ctx
            .proof_repo
            .list_artifacts(&p.id)
            .await
            .map_err(ApiError)?;
        rows.push(ProofSummaryRow {
            work_item_kind: p.work_item_kind.as_str().to_string(),
            work_item_id: p.work_item_id.clone(),
            proof_pack_id: p.id.clone(),
            status: p.status.as_str().to_string(),
            risk_score: p.risk_score,
            badges: engine::badge_strings(&p, &arts),
        });
    }
    Ok(Json(ProofSummaryResp { rows }))
}

async fn create(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(ws): Path<Id>,
    Json(req): Json<CreateProofPackReq>,
) -> ApiResult<Json<ProofPackResp>> {
    check(&ctx, &user, &ws, WorkspaceRole::Editor).await?;
    let kind = parse_work_kind(&req.work_item_kind)?;
    let title = req.title.unwrap_or_default();
    // Reuse the existing pack for this work item if present (ensure semantics),
    // optionally setting a parent on first create.
    let pack = if let Some(existing) = ctx
        .proof_repo
        .find_by_work_item(kind, &req.work_item_id)
        .await
        .map_err(ApiError)?
    {
        existing
    } else {
        ctx.proof_repo
            .create_pack(
                &ws,
                kind,
                &req.work_item_id,
                &title,
                &user.0.id,
                req.parent_pack_id.as_deref(),
            )
            .await
            .map_err(ApiError)?
    };
    let pack = engine::recompute_and_emit(&ctx, &pack.id)
        .await
        .map_err(ApiError)?;
    Ok(Json(pack_resp(&ctx, pack).await?))
}

async fn detail(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<ProofPackDetailResp>> {
    let pack = pack_for(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    let arts = ctx
        .proof_repo
        .list_artifacts(&pack.id)
        .await
        .map_err(ApiError)?;
    let badges = engine::badge_strings(&pack, &arts);
    let artifacts = arts.into_iter().map(artifact_view).collect();
    // Child packs (rollup), each as a summary row.
    let children_packs = ctx
        .proof_repo
        .list_children(&pack.id)
        .await
        .map_err(ApiError)?;
    let mut children = Vec::with_capacity(children_packs.len());
    for c in children_packs {
        children.push(pack_resp(&ctx, c).await?);
    }
    Ok(Json(ProofPackDetailResp {
        pack,
        badges,
        artifacts,
        children,
    }))
}

#[derive(Debug, Deserialize)]
struct PatchReq {
    title: Option<String>,
    summary: Option<String>,
}

async fn patch_pack(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<PatchReq>,
) -> ApiResult<Json<ProofPackResp>> {
    let pack = pack_for(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    ctx.proof_repo
        .update_meta(&pack.id, req.title.as_deref(), req.summary.as_deref())
        .await
        .map_err(ApiError)?;
    let pack = ctx.proof_repo.get_pack(&id).await.map_err(ApiError)?;
    Ok(Json(pack_resp(&ctx, pack).await?))
}

async fn remove(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<Value>> {
    let pack = pack_for(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    ctx.proof_repo.delete_pack(&pack.id).await.map_err(ApiError)?;
    Ok(Json(json!({ "ok": true })))
}

async fn add_artifact(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<AddArtifactReq>,
) -> ApiResult<Json<ProofPackResp>> {
    let pack = pack_for(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    let kind = parse_kind(&req.kind)?;
    let status = req
        .status
        .as_deref()
        .and_then(ProofArtifactStatus::parse)
        .unwrap_or(ProofArtifactStatus::Info);
    let meta = req.metadata.unwrap_or_else(|| json!({}));
    engine::add_content_artifact(
        &ctx,
        &pack,
        kind,
        &req.title,
        req.content.as_deref(),
        req.content_url.as_deref(),
        status,
        meta,
        &user.0.id,
    )
    .await
    .map_err(ApiError)?;
    let pack = engine::recompute_and_emit(&ctx, &pack.id)
        .await
        .map_err(ApiError)?;
    Ok(Json(pack_resp(&ctx, pack).await?))
}

async fn assemble(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<AssembleReq>,
) -> ApiResult<Json<ProofPackResp>> {
    let pack = pack_for(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    if let Some(cwd) = req.cwd.as_deref() {
        // Best-effort diff assembly.
        let _ = engine::assemble_diff(&ctx, &pack, cwd, req.base.as_deref()).await;
        // Run any requested commands as command artifacts.
        for c in req.commands.unwrap_or_default() {
            let _ = engine::run_command_artifact(&ctx, &pack, cwd, &c.cmd, c.kind.as_deref()).await;
        }
    }
    let pack = engine::recompute_and_emit(&ctx, &pack.id)
        .await
        .map_err(ApiError)?;
    Ok(Json(pack_resp(&ctx, pack).await?))
}

async fn waive(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<WaiveReq>,
) -> ApiResult<Json<ProofPackResp>> {
    let pack = pack_for(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    ctx.proof_repo
        .waive(&pack.id, &user.0.id, &req.reason)
        .await
        .map_err(ApiError)?;
    let pack = engine::recompute_and_emit(&ctx, &pack.id)
        .await
        .map_err(ApiError)?;
    Ok(Json(pack_resp(&ctx, pack).await?))
}

async fn remove_artifact(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<Value>> {
    let art = ctx.proof_repo.get_artifact(&id).await.map_err(ApiError)?;
    let pack = pack_for(&ctx, &user, &art.proof_pack_id, WorkspaceRole::Editor).await?;
    ctx.proof_repo.delete_artifact(&id).await.map_err(ApiError)?;
    let _ = engine::recompute_and_emit(&ctx, &pack.id).await;
    Ok(Json(json!({ "ok": true })))
}

async fn artifact_content(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<Value>> {
    let art = ctx.proof_repo.get_artifact(&id).await.map_err(ApiError)?;
    // Workspace-membership check via the owning pack.
    let _pack = pack_for(&ctx, &user, &art.proof_pack_id, WorkspaceRole::Viewer).await?;
    let ref_kind = art
        .metadata
        .get("ref_kind")
        .and_then(|v| v.as_str())
        .unwrap_or("inline")
        .to_string();
    Ok(Json(json!({
        "content": art.content_ref,
        "ref_kind": ref_kind,
        "kind": art.kind.as_str(),
        "status": art.status.as_str(),
        "metadata": art.metadata,
    })))
}
