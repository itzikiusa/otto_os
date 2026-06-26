//! HTTP routes for Proof Packs. The engine (assembly, recompute, gates) lives in
//! [`crate::proof`]; this module is the REST surface. Feature-axis access is
//! enforced by `policy.rs` (`Feature::ProofPack`); each handler additionally
//! checks the caller's workspace role.

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::Response;
use axum::routing::{delete, get, post};
use axum::{Extension, Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use otto_core::api::{
    AddArtifactReq, ApiEvidenceReq, AssembleReq, AttachMediaReq, CiRefreshReq, CreateProofPackReq,
    CreateSnapshotReq, DbEvidenceReq, KafkaEvidenceReq, PrCheckReq, ProofArtifactView,
    ProofPackDetailResp, ProofPackResp, ProofSnapshotMeta, ProofSnapshotResp, ProofSummaryResp,
    ProofSummaryRow, RepoProofConfigResp, WaiveReq,
};
use otto_core::auth::AuthUser;
use otto_core::domain::WorkspaceRole;
use otto_core::proof::{
    CiSummary, ProofArtifact, ProofArtifactKind, ProofArtifactStatus, ProofPack, RepoProofConfig,
    WorkItemKind, MEDIA_CAP,
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
        // -- v2 --------------------------------------------------------------
        .route("/proof-packs/{id}/snapshot", post(create_snapshot))
        .route("/proof-packs/{id}/snapshots", get(list_snapshots))
        .route("/proof-snapshots/{id}", get(get_snapshot))
        .route("/proof-packs/{id}/media", post(add_media))
        .route("/proof-artifacts/{id}/blob", get(artifact_blob))
        .route("/proof-packs/{id}/evidence/api", post(evidence_api))
        .route("/proof-packs/{id}/evidence/db", post(evidence_db))
        .route("/proof-packs/{id}/evidence/kafka", post(evidence_kafka))
        .route("/proof-packs/{id}/pr-check", post(pr_check))
        .route("/proof-packs/{id}/ci-refresh", post(ci_refresh))
        .route("/proof-packs/{id}/report", get(report))
        .route("/repos/{id}/proof-config", get(get_repo_config).put(put_repo_config))
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
            done_score: p.done_score,
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
    // Recompute the done-contract LIVE so the meter is accurate even for packs
    // created before this feature (their persisted done_score may be stale).
    let done_contract = engine::live_contract(&ctx, &pack, &arts).await;
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
    let snapshots = ctx
        .proof_repo
        .list_snapshots(&pack.id)
        .await
        .map_err(ApiError)?
        .into_iter()
        .map(snapshot_meta)
        .collect();
    Ok(Json(ProofPackDetailResp {
        pack,
        badges,
        artifacts,
        children,
        done_contract,
        snapshots,
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

/// Whether waiving requires workspace Admin. Opt-in via
/// `OTTO_PROOF_WAIVER_MIN_ROLE=admin` (default `edit`) — closes the
/// service-principal self-waive path defensively (S1).
fn waiver_requires_admin() -> bool {
    std::env::var("OTTO_PROOF_WAIVER_MIN_ROLE")
        .map(|v| v.eq_ignore_ascii_case("admin"))
        .unwrap_or(false)
}

async fn waive(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<WaiveReq>,
) -> ApiResult<Json<ProofPackResp>> {
    // A waiver is an accountable human act — require a real reason.
    let reason = req.reason.trim();
    if reason.len() < 10 {
        return Err(ApiError(Error::Invalid(
            "a waiver reason of at least 10 characters is required".into(),
        )));
    }
    let role = if waiver_requires_admin() {
        WorkspaceRole::Admin
    } else {
        WorkspaceRole::Editor
    };
    let pack = pack_for(&ctx, &user, &id, role).await?;
    // The approver is ALWAYS the authenticated request principal (never a client
    // field) — that's the "human approver" of R10.
    ctx.proof_repo
        .waive(&pack.id, &user.0.id, reason)
        .await
        .map_err(ApiError)?;
    // Immutable audit trail: record the waiver as an approval artifact.
    let _ = engine::add_content_artifact(
        &ctx,
        &pack,
        ProofArtifactKind::Approval,
        "Proof waived",
        Some(&format!("Proof requirement waived by {}: {}", user.0.id, reason)),
        None,
        ProofArtifactStatus::Passed,
        json!({"kind": "waiver", "approver": user.0.id, "reason": reason}),
        &user.0.id,
    )
    .await;
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

// --- v2 handlers -----------------------------------------------------------

fn snapshot_meta(r: otto_state::ProofSnapshotRow) -> ProofSnapshotMeta {
    ProofSnapshotMeta {
        id: r.id,
        proof_pack_id: r.proof_pack_id,
        seq: r.seq,
        sha256: r.sha256,
        status: r.status,
        done_score: r.done_score,
        risk_score: r.risk_score,
        note: r.note,
        created_by: r.created_by,
        created_at: r.created_at,
    }
}

async fn create_snapshot(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<CreateSnapshotReq>,
) -> ApiResult<Json<ProofSnapshotResp>> {
    let pack = pack_for(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    let row = engine::make_snapshot(&ctx, &pack, req.note.as_deref().unwrap_or(""), &user.0.id)
        .await
        .map_err(ApiError)?;
    let bundle: Value = serde_json::from_str(&row.bundle_json).unwrap_or(Value::Null);
    let report_md = row.report_md.clone();
    let report_html = row.report_html.clone();
    Ok(Json(ProofSnapshotResp {
        meta: snapshot_meta(row),
        bundle,
        report_md,
        report_html,
    }))
}

async fn list_snapshots(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<Vec<ProofSnapshotMeta>>> {
    let pack = pack_for(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    let rows = ctx
        .proof_repo
        .list_snapshots(&pack.id)
        .await
        .map_err(ApiError)?;
    Ok(Json(rows.into_iter().map(snapshot_meta).collect()))
}

async fn get_snapshot(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<ProofSnapshotResp>> {
    let row = ctx.proof_repo.get_snapshot(&id).await.map_err(ApiError)?;
    // Membership check via the owning pack's workspace.
    check(&ctx, &user, &row.workspace_id, WorkspaceRole::Viewer).await?;
    let bundle: Value = serde_json::from_str(&row.bundle_json).unwrap_or(Value::Null);
    let report_md = row.report_md.clone();
    let report_html = row.report_html.clone();
    Ok(Json(ProofSnapshotResp {
        meta: snapshot_meta(row),
        bundle,
        report_md,
        report_html,
    }))
}

async fn add_media(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<AttachMediaReq>,
) -> ApiResult<Json<ProofPackResp>> {
    use base64::Engine;
    let pack = pack_for(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    let kind = parse_kind(&req.kind)?;
    if !kind.is_media() {
        return Err(ApiError(Error::Invalid(
            "media kind must be 'screenshot' or 'video'".into(),
        )));
    }
    if !engine::ALLOWED_MEDIA_MIMES.contains(&req.mime.as_str()) {
        return Err(ApiError(Error::Invalid(format!(
            "unsupported media mime '{}'",
            req.mime
        ))));
    }
    let data = base64::engine::general_purpose::STANDARD
        .decode(req.data_base64.as_bytes())
        .map_err(|_| ApiError(Error::Invalid("data_base64 is not valid base64".into())))?;
    if data.is_empty() {
        return Err(ApiError(Error::Invalid("empty media".into())));
    }
    if data.len() > MEDIA_CAP {
        return Err(ApiError(Error::Invalid(format!(
            "media exceeds {} byte cap",
            MEDIA_CAP
        ))));
    }
    engine::attach_media(
        &ctx,
        &pack,
        kind,
        &req.title,
        &req.mime,
        &data,
        req.metadata.unwrap_or_else(|| json!({})),
        &user.0.id,
    )
    .await
    .map_err(ApiError)?;
    let pack = ctx.proof_repo.get_pack(&id).await.map_err(ApiError)?;
    Ok(Json(pack_resp(&ctx, pack).await?))
}

async fn artifact_blob(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Response> {
    let art = ctx.proof_repo.get_artifact(&id).await.map_err(ApiError)?;
    let _pack = pack_for(&ctx, &user, &art.proof_pack_id, WorkspaceRole::Viewer).await?;
    let blob = ctx
        .proof_repo
        .blob_for_artifact(&id)
        .await
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::NotFound(format!("no blob for artifact {id}"))))?;
    let fname = art.title.replace(['"', '\n', '\r'], "_");
    let resp = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, blob.mime)
        .header(
            header::CONTENT_DISPOSITION,
            format!("inline; filename=\"{fname}\""),
        )
        .body(Body::from(blob.data))
        .map_err(|e| ApiError(Error::Internal(format!("blob response: {e}"))))?;
    Ok(resp)
}

async fn evidence_api(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<ApiEvidenceReq>,
) -> ApiResult<Json<ProofPackResp>> {
    let pack = pack_for(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    engine::attach_api_evidence(&ctx, &pack, &req, &user.0.id)
        .await
        .map_err(ApiError)?;
    let pack = ctx.proof_repo.get_pack(&id).await.map_err(ApiError)?;
    Ok(Json(pack_resp(&ctx, pack).await?))
}

async fn evidence_db(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<DbEvidenceReq>,
) -> ApiResult<Json<ProofPackResp>> {
    let pack = pack_for(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    engine::attach_db_evidence(&ctx, &pack, &req, &user.0.id)
        .await
        .map_err(ApiError)?;
    let pack = ctx.proof_repo.get_pack(&id).await.map_err(ApiError)?;
    Ok(Json(pack_resp(&ctx, pack).await?))
}

async fn evidence_kafka(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<KafkaEvidenceReq>,
) -> ApiResult<Json<ProofPackResp>> {
    let pack = pack_for(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    engine::attach_kafka_evidence(&ctx, &pack, &req, &user.0.id)
        .await
        .map_err(ApiError)?;
    let pack = ctx.proof_repo.get_pack(&id).await.map_err(ApiError)?;
    Ok(Json(pack_resp(&ctx, pack).await?))
}

async fn pr_check(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<PrCheckReq>,
) -> ApiResult<Json<ProofPackResp>> {
    let pack = pack_for(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    engine::run_pr_check(
        &ctx,
        &pack,
        &req.title,
        &req.description,
        req.base.as_deref(),
        req.cwd.as_deref(),
        &user.0.id,
    )
    .await
    .map_err(ApiError)?;
    let pack = ctx.proof_repo.get_pack(&id).await.map_err(ApiError)?;
    Ok(Json(pack_resp(&ctx, pack).await?))
}

async fn ci_refresh(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<CiRefreshReq>,
) -> ApiResult<Json<ProofPackResp>> {
    let pack = pack_for(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    let repo_id = req
        .repo_id
        .clone()
        .or_else(|| pack.repo_id.clone())
        .ok_or_else(|| ApiError(Error::Invalid("no repo linked to this pack".into())))?;
    let pr_number = req
        .pr_number
        .or(pack.pr_number)
        .ok_or_else(|| ApiError(Error::Invalid("no PR number linked to this pack".into())))?;
    let repo = ctx.git_store.get_repo(&repo_id).await.map_err(ApiError)?;
    check(&ctx, &user, &repo.workspace_id, WorkspaceRole::Editor).await?;
    let (provider, remote) = crate::modules::resolve_provider_remote(&ctx, &user.0, &repo)
        .await
        .map_err(ApiError)?;
    let ci = provider.ci_status(&remote, pr_number as u64).await;
    let summary = CiSummary {
        state: ci.state,
        total: ci.total,
        passed: ci.passed,
        failed: ci.failed,
        url: ci.url,
    };
    // Persist the link so later refreshes/report can resolve it.
    let _ = ctx
        .proof_repo
        .set_repo_link(&pack.id, Some(&repo_id), Some(pr_number))
        .await;
    engine::record_ci_artifact(&ctx, &pack, &summary)
        .await
        .map_err(ApiError)?;
    let pack = ctx.proof_repo.get_pack(&id).await.map_err(ApiError)?;
    Ok(Json(pack_resp(&ctx, pack).await?))
}

#[derive(Debug, Deserialize)]
struct ReportQuery {
    format: Option<String>,
}

async fn report(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Query(q): Query<ReportQuery>,
) -> ApiResult<Response> {
    let pack = pack_for(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    let html = q.format.as_deref() == Some("html");
    let body = engine::render_report(&ctx, &pack.id, html)
        .await
        .map_err(ApiError)?;
    let ctype = if html {
        "text/html; charset=utf-8"
    } else {
        "text/markdown; charset=utf-8"
    };
    let resp = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, ctype)
        .body(Body::from(body))
        .map_err(|e| ApiError(Error::Internal(format!("report response: {e}"))))?;
    Ok(resp)
}

async fn get_repo_config(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<RepoProofConfigResp>> {
    let repo = ctx.git_store.get_repo(&id).await.map_err(ApiError)?;
    check(&ctx, &user, &repo.workspace_id, WorkspaceRole::Viewer).await?;
    let config = ctx
        .git_store
        .get_proof_config(&id)
        .await
        .map_err(ApiError)?;
    Ok(Json(RepoProofConfigResp { repo_id: id, config }))
}

async fn put_repo_config(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(config): Json<RepoProofConfig>,
) -> ApiResult<Json<RepoProofConfigResp>> {
    let repo = ctx.git_store.get_repo(&id).await.map_err(ApiError)?;
    check(&ctx, &user, &repo.workspace_id, WorkspaceRole::Editor).await?;
    ctx.git_store
        .set_proof_config(&id, &config)
        .await
        .map_err(ApiError)?;
    Ok(Json(RepoProofConfigResp { repo_id: id, config }))
}
