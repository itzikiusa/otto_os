//! HTTP routes for Proof Packs. The engine (assembly, recompute, gates) lives in
//! [`crate::proof`]; this module is the REST surface. Feature-axis access is
//! enforced by `policy.rs` (`Feature::ProofPack`); each handler additionally
//! checks the caller's workspace role.

use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
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
    WorkItemKind, MEDIA_CAP, PREVIEW_CAP,
};
use otto_core::{Error, Id};

use crate::error::{ApiError, ApiResult};
use crate::proof as engine;
use crate::state::ServerCtx;

/// Request-body cap for the base64 media upload: `MEDIA_CAP` × 4/3 base64
/// inflation plus JSON framing, rounded up to 40 MiB.
const MEDIA_BODY_LIMIT: usize = 40 * 1024 * 1024;
const _: () = assert!(MEDIA_BODY_LIMIT > MEDIA_CAP / 3 * 4 + 64 * 1024);

pub fn routes() -> Router<ServerCtx> {
    Router::new()
        .route("/workspaces/{id}/proof-packs", get(list).post(create))
        .route("/workspaces/{id}/proof-summary", get(summary))
        .route(
            "/workspaces/{id}/proof-packs/archive-sessions",
            post(archive_sessions),
        )
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
        // Media arrives as base64 JSON: the 25 MiB decoded cap (`MEDIA_CAP`, a
        // clean 413 from the handler) needs ~34 MiB of body, far past axum's
        // 2 MiB default — which also refuses before reading and closes the
        // socket mid-upload. Same 40 MiB envelope as story attachments.
        .route(
            "/proof-packs/{id}/media",
            post(add_media).layer(DefaultBodyLimit::max(MEDIA_BODY_LIMIT)),
        )
        .route("/proof-artifacts/{id}/blob", get(artifact_blob))
        .route("/proof-packs/{id}/evidence/api", post(evidence_api))
        .route("/proof-packs/{id}/evidence/db", post(evidence_db))
        .route("/proof-packs/{id}/evidence/kafka", post(evidence_kafka))
        .route("/proof-packs/{id}/pr-check", post(pr_check))
        .route("/proof-packs/{id}/ci-refresh", post(ci_refresh))
        .route("/proof-packs/{id}/report", get(report))
        .route(
            "/repos/{id}/proof-config",
            get(get_repo_config).put(put_repo_config),
        )
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
    // Badges + count only — never the inline content.
    let arts = ctx
        .proof_repo
        .list_artifacts_meta(&pack.id)
        .await
        .map_err(ApiError)?;
    let badges = engine::badge_strings(&pack, &arts);
    Ok(ProofPackResp {
        badges,
        artifact_count: arts.len() as u32,
        pack,
    })
}

/// `a.content_ref` arrives already cut to [`PREVIEW_CAP`] characters by SQL
/// (`list_artifacts_preview`); `full_len` is the stored content's byte length.
/// Inline content gets a capped preview; url/file refs keep their (short) ref.
fn artifact_view(mut a: ProofArtifact, full_len: i64) -> ProofArtifactView {
    let ref_kind = a
        .metadata
        .get("ref_kind")
        .and_then(|v| v.as_str())
        .unwrap_or("inline");
    let (preview, truncated) = if ref_kind == "inline" {
        match &a.content_ref {
            Some(c) => {
                let (p, t) = engine::preview(c);
                let truncated = t || full_len > p.len() as i64;
                // The detail response never ships more than the preview; the
                // full body is `GET /proof-artifacts/{id}/content`.
                a.content_ref = Some(p.clone());
                (Some(p), truncated)
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
    /// Page size (1..=500). Absent = every pack (legacy callers).
    limit: Option<u32>,
    /// Opaque keyset cursor from a previous page's `x-next-cursor` header.
    cursor: Option<String>,
    /// Also list packs the opt-in session archive hid (default false).
    #[serde(default)]
    include_archived: bool,
}

/// `<updated_at>|<id>` — both are RFC3339 / ULID-ish text without `|`.
fn encode_cursor(c: &otto_state::PackCursor) -> String {
    format!("{}|{}", c.0, c.1)
}

fn decode_cursor(s: &str) -> ApiResult<otto_state::PackCursor> {
    s.split_once('|')
        .map(|(u, id)| (u.to_string(), id.to_string()))
        .ok_or_else(|| ApiError(Error::Invalid("bad cursor".into())))
}

async fn list(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(ws): Path<Id>,
    Query(q): Query<ListQuery>,
) -> ApiResult<Response> {
    check(&ctx, &user, &ws, WorkspaceRole::Viewer).await?;
    let after = q.cursor.as_deref().map(decode_cursor).transpose()?;
    let (packs, next) = ctx
        .proof_repo
        .list_packs_page(
            &ws,
            q.status.as_deref(),
            q.work_item_kind.as_deref(),
            q.work_item_id.as_deref(),
            q.limit.map(|l| l.clamp(1, 500)),
            after.as_ref(),
            q.include_archived,
        )
        .await
        .map_err(ApiError)?;
    // Badge inputs for exactly this page's packs: one narrow `IN (…)` query
    // (r3-07-01), never the inline content and never the whole workspace.
    let ids: Vec<String> = packs.iter().map(|p| p.id.clone()).collect();
    let mut arts = ctx
        .proof_repo
        .artifacts_meta_for_packs(&ids)
        .await
        .map_err(ApiError)?;
    let out: Vec<ProofPackResp> = packs
        .into_iter()
        .map(|pack| {
            let a = arts.remove(&pack.id).unwrap_or_default();
            ProofPackResp {
                badges: engine::badge_strings(&pack, &a),
                artifact_count: a.len() as u32,
                pack,
            }
        })
        .collect();
    let body = serde_json::to_vec(&out)
        .map_err(|e| ApiError(Error::Internal(format!("serialize packs: {e}"))))?;
    let mut b = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(c) = next {
        b = b.header("x-next-cursor", encode_cursor(&c));
    }
    b.body(Body::from(body))
        .map_err(|e| ApiError(Error::Internal(format!("packs response: {e}"))))
}

/// Most `kind:id` entries one scoped summary request may name.
const SUMMARY_MAX_WORK_ITEMS: usize = 1000;

#[derive(Deserialize, Default)]
struct SummaryQuery {
    /// `kind:id,kind:id,…` — only these work items' packs are read (R3). Absent
    /// = the whole workspace (the legacy full read, kept as a fallback).
    #[serde(default)]
    work_items: Option<String>,
}

/// Parse `kind:id,…` (blank entries skipped). Every kind must be a known
/// work-item kind; at most [`SUMMARY_MAX_WORK_ITEMS`] entries.
fn parse_work_items(raw: &str) -> Result<Vec<(String, String)>, Error> {
    let mut out = Vec::new();
    for entry in raw.split(',').map(str::trim).filter(|e| !e.is_empty()) {
        let Some((kind, id)) = entry.split_once(':') else {
            return Err(Error::Invalid(format!(
                "work_items entry {entry:?} is not kind:id"
            )));
        };
        let kind = parse_work_kind(kind).map_err(|e| e.0)?;
        if id.is_empty() {
            return Err(Error::Invalid(format!(
                "work_items entry {entry:?} has no id"
            )));
        }
        out.push((kind.as_str().to_string(), id.to_string()));
    }
    if out.len() > SUMMARY_MAX_WORK_ITEMS {
        return Err(Error::Invalid(format!(
            "at most {SUMMARY_MAX_WORK_ITEMS} work_items per request"
        )));
    }
    Ok(out)
}

async fn summary(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(ws): Path<Id>,
    Query(q): Query<SummaryQuery>,
) -> ApiResult<Json<ProofSummaryResp>> {
    check(&ctx, &user, &ws, WorkspaceRole::Viewer).await?;
    let (packs, mut by_pack) = match q.work_items.as_deref() {
        // Scoped: only the named work items' packs + their badge artifacts,
        // both index probes — rows read match the filter, not the workspace.
        Some(raw) => {
            let items = parse_work_items(raw).map_err(ApiError)?;
            let packs = ctx
                .proof_repo
                .list_packs_for_work_items(&ws, &items)
                .await
                .map_err(ApiError)?;
            let ids: Vec<String> = packs.iter().map(|p| p.id.clone()).collect();
            let arts = ctx
                .proof_repo
                .artifacts_meta_for_packs(&ids)
                .await
                .map_err(ApiError)?;
            (packs, arts)
        }
        None => {
            let packs = ctx
                .proof_repo
                .list_packs(&ws, None, None, None)
                .await
                .map_err(ApiError)?;
            let arts = ctx
                .proof_repo
                .badge_artifacts(&ws)
                .await
                .map_err(ApiError)?;
            (packs, arts)
        }
    };
    let mut rows = Vec::with_capacity(packs.len());
    for p in packs {
        let arts = by_pack.remove(&p.id).unwrap_or_default();
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

/// Default / minimum age (days) for the opt-in session-pack archive.
const ARCHIVE_DEFAULT_DAYS: u32 = 30;
const ARCHIVE_MIN_DAYS: u32 = 7;

#[derive(Deserialize, Default)]
struct ArchiveSessionsReq {
    /// Only packs last updated more than this many days ago (default 30, min 7).
    #[serde(default)]
    older_than_days: Option<u32>,
    /// Dry run unless true.
    #[serde(default)]
    apply: bool,
}

/// `POST /workspaces/{id}/proof-packs/archive-sessions` — OPT-IN, workspace
/// admin. Hides stale `session` packs that never got any evidence from the
/// summary + default list by stamping `archived_at`; nothing is deleted, and a
/// pack un-archives itself the moment it changes or gains evidence. Dry run
/// (count only) unless `apply: true`.
async fn archive_sessions(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(ws): Path<Id>,
    body: Option<Json<ArchiveSessionsReq>>,
) -> ApiResult<Json<Value>> {
    check(&ctx, &user, &ws, WorkspaceRole::Admin).await?;
    let req = body.map(|b| b.0).unwrap_or_default();
    let days = req.older_than_days.unwrap_or(ARCHIVE_DEFAULT_DAYS);
    if days < ARCHIVE_MIN_DAYS {
        return Err(ApiError(Error::Invalid(format!(
            "older_than_days must be at least {ARCHIVE_MIN_DAYS}"
        ))));
    }
    // Same RFC3339 shape the repo stamps `updated_at` with (lexical compare).
    let cutoff = (chrono::Utc::now() - chrono::Duration::days(i64::from(days))).to_rfc3339();
    let matched = ctx
        .proof_repo
        .archive_stale_session_packs(&ws, &cutoff, false)
        .await
        .map_err(ApiError)?;
    let archived = if req.apply && matched > 0 {
        ctx.proof_repo
            .archive_stale_session_packs(&ws, &cutoff, true)
            .await
            .map_err(ApiError)?
    } else {
        0
    };
    Ok(Json(json!({
        "applied": req.apply,
        "older_than_days": days,
        "cutoff": cutoff,
        "matched": matched,
        "archived": archived,
    })))
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
    // Optionally link to a registered repo so its proof policy applies. Best-effort
    // and strengthen-only — an unresolvable repo just leaves the pack unlinked.
    if let Some(repo_id) = req.repo_id.as_deref() {
        if pack.repo_id.as_deref() != Some(repo_id) {
            ctx.proof_repo
                .set_repo_link(&pack.id, Some(repo_id), None)
                .await
                .map_err(ApiError)?;
        }
    }
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
    // Content cut to the preview cap in SQL — a pack of ten 2 MiB diffs used
    // to read (and ship) 20 MiB to render 80 KiB of previews.
    let rows = ctx
        .proof_repo
        .list_artifacts_preview(&pack.id, PREVIEW_CAP + 1)
        .await
        .map_err(ApiError)?;
    let (arts, lens): (Vec<ProofArtifact>, Vec<i64>) = rows.into_iter().unzip();
    let badges = engine::badge_strings(&pack, &arts);
    // Recompute the done-contract LIVE so the meter is accurate even for packs
    // created before this feature (their persisted done_score may be stale).
    let done_contract = engine::live_contract(&ctx, &pack, &arts).await;
    let artifacts = arts
        .into_iter()
        .zip(lens)
        .map(|(a, n)| artifact_view(a, n))
        .collect();
    // Child packs (rollup), each as a summary row — one metadata query for
    // all children instead of one full read each.
    let children_packs = ctx
        .proof_repo
        .list_children(&pack.id)
        .await
        .map_err(ApiError)?;
    let child_ids: Vec<String> = children_packs.iter().map(|c| c.id.clone()).collect();
    let mut child_arts = ctx
        .proof_repo
        .artifacts_meta_for_packs(&child_ids)
        .await
        .map_err(ApiError)?;
    let children: Vec<ProofPackResp> = children_packs
        .into_iter()
        .map(|c| {
            let a = child_arts.remove(&c.id).unwrap_or_default();
            ProofPackResp {
                badges: engine::badge_strings(&c, &a),
                artifact_count: a.len() as u32,
                pack: c,
            }
        })
        .collect();
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
    ctx.proof_repo
        .delete_pack(&pack.id)
        .await
        .map_err(ApiError)?;
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
        Some(&format!(
            "Proof requirement waived by {}: {}",
            user.0.id, reason
        )),
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
    ctx.proof_repo
        .delete_artifact(&id)
        .await
        .map_err(ApiError)?;
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

/// Query for a raw-body media upload (`POST /proof-packs/{id}/media?kind=…&title=…`
/// with the bytes as the body and the mime as `Content-Type`).
#[derive(Debug, Deserialize)]
struct RawMediaQuery {
    kind: Option<String>,
    title: Option<String>,
}

async fn add_media(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Query(rq): Query<RawMediaQuery>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> ApiResult<Json<ProofPackResp>> {
    use base64::Engine;
    let pack = pack_for(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    let ct = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    // Raw body (the UI's path: no base64 inflation, no JSON parse) or the
    // legacy base64 JSON envelope — both decoded/parsed off the runtime.
    struct MediaIn {
        kind: String,
        title: String,
        mime: String,
        data_base64: String,
        metadata: Option<Value>,
        raw: Option<Vec<u8>>,
    }
    let req: MediaIn = if ct == "application/json" || ct.is_empty() {
        let j =
            tokio::task::spawn_blocking(move || serde_json::from_slice::<AttachMediaReq>(&body))
                .await
                .map_err(|e| ApiError(Error::Internal(format!("media parse join: {e}"))))?
                .map_err(|e| ApiError(Error::Invalid(format!("invalid media request: {e}"))))?;
        MediaIn {
            kind: j.kind,
            title: j.title,
            mime: j.mime,
            data_base64: j.data_base64,
            metadata: j.metadata,
            raw: None,
        }
    } else {
        MediaIn {
            kind: rq.kind.unwrap_or_else(|| {
                if ct.starts_with("video/") {
                    "video".into()
                } else {
                    "screenshot".into()
                }
            }),
            title: rq.title.unwrap_or_default(),
            mime: ct.clone(),
            data_base64: String::new(),
            metadata: None,
            raw: Some(body.to_vec()),
        }
    };
    let kind = parse_kind(&req.kind)?;
    if !kind.is_media() {
        return Err(ApiError(Error::Invalid(
            "media kind must be 'screenshot' or 'video'".into(),
        )));
    }
    if !engine::ALLOWED_MEDIA_MIMES.contains(&req.mime.as_str()) {
        return Err(ApiError(Error::UnsupportedMedia(format!(
            "unsupported media mime '{}' (allowed: {})",
            req.mime,
            engine::ALLOWED_MEDIA_MIMES.join(", ")
        ))));
    }
    let data = match req.raw {
        Some(raw) => raw,
        None => {
            // Decoding ~34 MiB of base64 is CPU work — off the async workers.
            let b64 = req.data_base64;
            tokio::task::spawn_blocking(move || {
                base64::engine::general_purpose::STANDARD.decode(b64.as_bytes())
            })
            .await
            .map_err(|e| ApiError(Error::Internal(format!("media decode join: {e}"))))?
            .map_err(|_| ApiError(Error::Invalid("data_base64 is not valid base64".into())))?
        }
    };
    if data.is_empty() {
        return Err(ApiError(Error::Invalid("empty media".into())));
    }
    if data.len() > MEDIA_CAP {
        return Err(ApiError(Error::PayloadTooLarge(format!(
            "media is {} bytes, exceeds the {} byte cap",
            data.len(),
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
    headers: axum::http::HeaderMap,
) -> ApiResult<Response> {
    let art = ctx.proof_repo.get_artifact(&id).await.map_err(ApiError)?;
    let _pack = pack_for(&ctx, &user, &art.proof_pack_id, WorkspaceRole::Viewer).await?;
    // A media artifact's bytes never change (one blob per artifact, written
    // once), so its sha is a strong validator: answer a revalidation with
    // 304 before touching the blob.
    let etag = art
        .metadata
        .get("sha256")
        .and_then(|v| v.as_str())
        .map(|s| format!("\"{s}\""));
    if let (Some(tag), Some(inm)) = (etag.as_deref(), headers.get(header::IF_NONE_MATCH)) {
        if inm
            .to_str()
            .is_ok_and(|v| v.split(',').any(|t| t.trim() == tag))
        {
            return Response::builder()
                .status(StatusCode::NOT_MODIFIED)
                .header(header::ETAG, tag)
                .header(
                    header::CACHE_CONTROL,
                    "private, max-age=31536000, immutable",
                )
                .body(Body::empty())
                .map_err(|e| ApiError(Error::Internal(format!("blob response: {e}"))));
        }
    }
    let blob = ctx
        .proof_repo
        .blob_for_artifact(&id)
        .await
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::NotFound(format!("no blob for artifact {id}"))))?;
    let fname = art.title.replace(['"', '\n', '\r'], "_");
    let mut resp = Response::builder().status(StatusCode::OK).header(
        header::CACHE_CONTROL,
        "private, max-age=31536000, immutable",
    );
    if let Some(tag) = etag.as_deref() {
        resp = resp.header(header::ETAG, tag);
    }
    let resp = resp
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
    Ok(Json(RepoProofConfigResp {
        repo_id: id,
        config,
    }))
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
    Ok(Json(RepoProofConfigResp {
        repo_id: id,
        config,
    }))
}

#[cfg(test)]
mod proof_route_tests {
    use super::*;

    fn art(content: &str) -> ProofArtifact {
        ProofArtifact {
            id: "a".into(),
            proof_pack_id: "p".into(),
            workspace_id: "w".into(),
            kind: ProofArtifactKind::Diff,
            title: "diff".into(),
            content_ref: Some(content.into()),
            status: ProofArtifactStatus::Info,
            metadata: json!({}),
            content_sha256: None,
            created_by: "u".into(),
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn proof_artifact_view_flags_sql_trimmed_content_as_truncated() {
        // Short content: as stored, not truncated.
        let v = artifact_view(art("hello"), 5);
        assert_eq!(v.preview.as_deref(), Some("hello"));
        assert!(!v.truncated);
        // SQL already cut the body; the full length says there is more.
        let v = artifact_view(art("hello"), 50_000);
        assert!(v.truncated);
        // A long cut body is capped to the preview and never shipped whole.
        let long = "x".repeat(PREVIEW_CAP + 1);
        let v = artifact_view(art(&long), (PREVIEW_CAP + 1) as i64);
        assert!(v.truncated);
        assert!(v.artifact.content_ref.unwrap().len() <= PREVIEW_CAP);
    }

    #[test]
    fn proof_cursor_roundtrips() {
        let c = ("2026-10-03T12:00:00Z".to_string(), "01ABC".to_string());
        assert_eq!(decode_cursor(&encode_cursor(&c)).unwrap(), c);
        assert!(decode_cursor("nobar").is_err());
    }

    #[test]
    fn summary_work_items_parse_and_validate() {
        let items = parse_work_items("session:s1, goal_loop:g2,,").unwrap();
        assert_eq!(
            items,
            vec![
                ("session".to_string(), "s1".to_string()),
                ("goal_loop".to_string(), "g2".to_string())
            ]
        );
        assert!(parse_work_items("").unwrap().is_empty());
        assert!(parse_work_items("nokind").is_err());
        assert!(parse_work_items("bogus:x").is_err());
        assert!(parse_work_items("session:").is_err());
        let too_many = (0..=SUMMARY_MAX_WORK_ITEMS)
            .map(|i| format!("session:s{i}"))
            .collect::<Vec<_>>()
            .join(",");
        assert!(parse_work_items(&too_many).is_err());
    }
}
