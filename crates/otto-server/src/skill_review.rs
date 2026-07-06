//! Skills Lab — multi-agent review of a `SKILL.md` package.
//!
//! Mirrors the code-review engine (`modules.rs::run_review_core`) but targets a
//! skill package instead of a diff. A review has three layers:
//!
//! 1. **Static pass** — a deterministic, dependency-free port of the bundled
//!    `skills-reviewer` static reviewer (native Rust; always runs, instant).
//! 2. **Agent pass** (opt-in) — N visible provider PTY sessions, each running the
//!    `skills-reviewer` method over the target, tagged `meta.source="skillreview"`
//!    so they never clutter the Agents grid but embed live in the Review panel.
//! 3. **Summarizer** — a headless `orchestrator.run_agent` turn that folds the
//!    static + agent findings into one ranked report + patch plan.
//!
//! `agent_mode="static"` (or no providers) yields a complete review from the
//! static pass alone — the deterministic path used by tests/CI.
//!
//! Every review runs on a staged temp COPY of the package with local machine
//! artifacts stripped ([`IGNORED_ENTRIES`] — `.mcp.json`, `.DS_Store`, …), so
//! secrets in those files are never scanned or quoted. User-supplied
//! `instructions` ride on the review row and are appended to every reviewer +
//! summarizer prompt. After a review completes, `POST /skill-reviews/{id}/apply`
//! hands the findings to a **fixer agent** that edits the REAL skill directory.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{Path as AxPath, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use otto_core::api::{CreateSessionReq, StartSkillReviewReq};
use otto_core::domain::{
    SessionKind, SkillFinding, SkillReview, SkillReviewAgent, SkillReviewSummary, SkillScoreRow,
    SkillStaticReport, User, Workspace, WorkspaceRole,
};
use otto_core::event::Event;
use otto_core::{Id, Result};
use tempfile::TempDir;

use crate::auth::{require_ws_role, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::review_session::{bracketed_paste, dispatched, wait_for_tui};
use crate::skill_eval::CancelRegistry;
use crate::state::ServerCtx;

// ---------------------------------------------------------------------------
// Timeouts
// ---------------------------------------------------------------------------

const AGENT_TIMEOUT: Duration = Duration::from_secs(600); // 10 min per reviewer agent
const FIX_TIMEOUT: Duration = Duration::from_secs(900); // 15 min for the apply-fixes agent
const SUMMARIZER_TIMEOUT: Duration = Duration::from_secs(120);
const PASTE_TO_ENTER: Duration = Duration::from_millis(250);
const OUTPUT_POLL: Duration = Duration::from_millis(1000);
const WAITING_IDLE: Duration = Duration::from_secs(60);

// ---------------------------------------------------------------------------
// Cancel registry helpers (re-declared; skill_eval's are private)
// ---------------------------------------------------------------------------

fn register_cancel(reg: &CancelRegistry, id: &str) -> Arc<AtomicBool> {
    let flag = Arc::new(AtomicBool::new(false));
    if let Ok(mut map) = reg.lock() {
        map.insert(id.to_string(), Arc::clone(&flag));
    }
    flag
}
fn signal_cancel(reg: &CancelRegistry, id: &str) {
    if let Ok(map) = reg.lock() {
        if let Some(flag) = map.get(id) {
            flag.store(true, Ordering::SeqCst);
        }
    }
}
fn unregister_cancel(reg: &CancelRegistry, id: &str) {
    if let Ok(mut map) = reg.lock() {
        map.remove(id);
    }
}
fn is_cancelled(flag: &Arc<AtomicBool>) -> bool {
    flag.load(Ordering::SeqCst)
}

// ---------------------------------------------------------------------------
// Target resolution
// ---------------------------------------------------------------------------

/// Local machine artifacts that live inside skill directories but are NOT part
/// of the published package. They are stripped from the staged review copy so
/// neither the static pass nor the reviewer agents ever see them — `.mcp.json`
/// in particular can carry a live API token that would otherwise be flagged
/// (or worse, quoted) in every review.
const IGNORED_ENTRIES: &[&str] =
    &[".git", ".DS_Store", ".mcp.json", ".env", "node_modules", "__pycache__"];

fn is_ignored(name: &str) -> bool {
    IGNORED_ENTRIES.contains(&name) || name.starts_with(".env.")
}

/// A skill package staged for review: always a temp-dir copy (held so the tree
/// outlives the review) with [`IGNORED_ENTRIES`] stripped.
struct Staged(#[allow(dead_code)] TempDir, PathBuf);
impl Staged {
    fn path(&self) -> &Path {
        &self.1
    }
}

/// Recursively copy a skill tree, skipping [`IGNORED_ENTRIES`] (and symlinks —
/// a symlink could point back outside the package at the very files we strip).
fn copy_skill_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        if is_ignored(&name.to_string_lossy()) {
            continue;
        }
        let ft = entry.file_type()?;
        let dst = to.join(&name);
        if ft.is_dir() {
            copy_skill_tree(&entry.path(), &dst)?;
        } else if ft.is_file() {
            std::fs::copy(entry.path(), &dst)?;
        }
    }
    Ok(())
}

/// Remove any [`IGNORED_ENTRIES`] from an already-staged tree (bundled skills
/// are written by the install primitive, so they get pruned instead of copied).
fn prune_ignored(dir: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let name = entry.file_name();
        let p = entry.path();
        if is_ignored(&name.to_string_lossy()) {
            let _ = if p.is_dir() { std::fs::remove_dir_all(&p) } else { std::fs::remove_file(&p) };
        } else if p.is_dir() {
            prune_ignored(&p);
        }
    }
}

/// The real, editable on-disk directory of a skill — where the apply-fixes
/// agent works. Library skills live in the context library; provider skills
/// (`claude`/`codex`/`agy`) in `~/.<provider>/skills`. Bundled skills are
/// embedded in the binary and cannot be edited.
fn real_skill_dir(ctx: &ServerCtx, skill_name: &str, source: &str) -> Result<PathBuf> {
    if source == "bundled" {
        return Err(otto_core::Error::Invalid(
            "bundled skills are read-only — install the skill to the library first".into(),
        ));
    }
    let dir = if otto_context::provider_skills::PROVIDERS.contains(&source) {
        otto_context::provider_skills::skill_dir(source, skill_name)
            .ok_or_else(|| otto_core::Error::Invalid("unsafe skill name".into()))?
    } else {
        ctx.context_library
            .skill_dir(skill_name)
            .ok_or_else(|| otto_core::Error::Invalid("unsafe skill name".into()))?
    };
    if !dir.join("SKILL.md").exists() {
        return Err(otto_core::Error::NotFound(format!("skill '{source}/{skill_name}'")));
    }
    Ok(dir)
}

/// Stage the skill package to review into a private temp copy. Bundled skills
/// go through the tested install primitive (handles the full multi-file tree,
/// binaries included); library/provider skills are copied from their real dir.
/// Either way the copy excludes [`IGNORED_ENTRIES`], so reviews never scan
/// local machine files like `.mcp.json`.
fn stage_target(ctx: &ServerCtx, skill_name: &str, source: &str) -> Result<Staged> {
    let tmp = tempfile::tempdir()
        .map_err(|e| otto_core::Error::Internal(format!("stage skill: {e}")))?;
    if source == "bundled" {
        let templib = otto_context::Library::new(tmp.path());
        let ok = otto_skills::install_into(&templib, skill_name)
            .map_err(|e| otto_core::Error::Internal(format!("stage bundled skill: {e}")))?;
        if !ok {
            return Err(otto_core::Error::NotFound(format!("bundled skill '{skill_name}'")));
        }
        let dir = tmp.path().join("skills").join(skill_name);
        prune_ignored(&dir);
        Ok(Staged(tmp, dir))
    } else {
        let src = real_skill_dir(ctx, skill_name, source)?;
        let dir = tmp.path().join(skill_name);
        copy_skill_tree(&src, &dir)
            .map_err(|e| otto_core::Error::Internal(format!("stage skill copy: {e}")))?;
        Ok(Staged(tmp, dir))
    }
}

// ---------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------

pub fn routes() -> Router<ServerCtx> {
    Router::new()
        .route(
            "/workspaces/{id}/skill-reviews",
            post(start_review).get(list_reviews),
        )
        .route("/skill-reviews/{id}", get(get_review).delete(delete_review))
        .route("/skill-reviews/{id}/cancel", post(cancel_review))
        .route(
            "/skill-reviews/{id}/agents/{index}/retry",
            post(retry_agent),
        )
        .route("/skill-reviews/{id}/apply", post(apply_fixes))
}

async fn start_review(
    AxPath(ws_id): AxPath<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<StartSkillReviewReq>,
) -> ApiResult<Json<SkillReview>> {
    require_ws_role(&ctx, &user, &ws_id, WorkspaceRole::Editor).await?;
    if req.skill_name.trim().is_empty() {
        return Err(ApiError(otto_core::Error::Invalid("skill_name is required".into())));
    }
    // Accept "library", "bundled", or a provider name (claude/codex/agy).
    let source: &str = if req.skill_source == "bundled" {
        "bundled"
    } else if otto_context::provider_skills::PROVIDERS.contains(&req.skill_source.as_str()) {
        req.skill_source.as_str()
    } else {
        "library"
    };
    // Validate the target resolves before creating the row.
    stage_target(&ctx, &req.skill_name, source).map_err(ApiError)?;

    // Determine mode + providers.
    let want_agents = req.agent_mode != "static" && !req.providers.is_empty();
    let mode = if want_agents { "agents" } else { "static" };
    let providers: Vec<String> = if want_agents { req.providers.clone() } else { Vec::new() };

    let ws = ctx.workspaces.get(&ws_id).await.map_err(ApiError)?;
    let instructions = req.instructions.trim().to_string();
    let review = ctx
        .skill_reviews_store
        .create(&ws_id, &req.skill_name, source, mode, &instructions, Some(&user.id))
        .await
        .map_err(ApiError)?;

    let ctx_bg = ctx.clone();
    let review_id = review.id.clone();
    let skill_name = req.skill_name.clone();
    let source_s = source.to_string();
    let cancel = register_cancel(&ctx.skill_review_cancels, &review_id);
    tokio::spawn(async move {
        run_review(ctx_bg, review_id, ws, user, skill_name, source_s, providers, instructions, cancel)
            .await;
    });

    Ok(Json(review))
}

async fn list_reviews(
    AxPath(ws_id): AxPath<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<SkillReview>>> {
    require_ws_role(&ctx, &user, &ws_id, WorkspaceRole::Viewer).await?;
    let list = ctx.skill_reviews_store.list(&ws_id).await.map_err(ApiError)?;
    Ok(Json(list))
}

async fn get_review(
    AxPath(id): AxPath<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<SkillReview>> {
    let review = ctx.skill_reviews_store.get(&id).await.map_err(ApiError)?;
    require_ws_role(&ctx, &user, &review.workspace_id, WorkspaceRole::Viewer).await?;
    Ok(Json(review))
}

async fn cancel_review(
    AxPath(id): AxPath<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<SkillReview>> {
    let review = ctx.skill_reviews_store.get(&id).await.map_err(ApiError)?;
    require_ws_role(&ctx, &user, &review.workspace_id, WorkspaceRole::Editor).await?;
    signal_cancel(&ctx.skill_review_cancels, &id);
    for a in &review.agents {
        if let Some(sid) = &a.session_id {
            let _ = ctx.manager.archive(sid).await;
        }
    }
    if let Some(sid) = review.fix_agent.as_ref().and_then(|f| f.session_id.as_ref()) {
        let _ = ctx.manager.archive(sid).await;
    }
    let _ = ctx.skill_reviews_store.set_status(&id, "cancelled", Some("Cancelled by user")).await;
    emit(&ctx, &review.workspace_id, &id, "cancelled");
    let review = ctx.skill_reviews_store.get(&id).await.map_err(ApiError)?;
    Ok(Json(review))
}

async fn delete_review(
    AxPath(id): AxPath<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<axum::http::StatusCode> {
    let review = ctx.skill_reviews_store.get(&id).await.map_err(ApiError)?;
    require_ws_role(&ctx, &user, &review.workspace_id, WorkspaceRole::Editor).await?;
    signal_cancel(&ctx.skill_review_cancels, &id);
    for a in &review.agents {
        if let Some(sid) = &a.session_id {
            let _ = ctx.manager.archive(sid).await;
        }
    }
    if let Some(sid) = review.fix_agent.as_ref().and_then(|f| f.session_id.as_ref()) {
        let _ = ctx.manager.archive(sid).await;
    }
    ctx.skill_reviews_store.delete(&id).await.map_err(ApiError)?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

async fn retry_agent(
    AxPath((id, index)): AxPath<(Id, usize)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<SkillReview>> {
    let review = ctx.skill_reviews_store.get(&id).await.map_err(ApiError)?;
    require_ws_role(&ctx, &user, &review.workspace_id, WorkspaceRole::Editor).await?;
    let Some(agent) = review.agents.get(index).cloned() else {
        return Err(ApiError(otto_core::Error::NotFound(format!("agent #{index}"))));
    };
    if agent.name == "summarizer" {
        return Err(ApiError(otto_core::Error::Invalid("cannot retry the summarizer".into())));
    }
    let ctx_bg = ctx.clone();
    let review_id = id.clone();
    let ws = ctx.workspaces.get(&review.workspace_id).await.map_err(ApiError)?;
    let skill_name = review.skill_name.clone();
    let source = review.skill_source.clone();
    let instructions = review.instructions.clone();
    let provider = agent.provider.clone();
    let cancel = Arc::new(AtomicBool::new(false));
    tokio::spawn(async move {
        let staged = match stage_target(&ctx_bg, &skill_name, &source) {
            Ok(s) => s,
            Err(_) => return,
        };
        let reviewer = reviewer_method();
        let base = SkillReviewAgent {
            name: provider.clone(),
            provider: provider.clone(),
            model: String::new(),
            status: "pending".into(),
            note: String::new(),
            session_id: None,
            findings: vec![],
        };
        otto_sessions::trust::ensure_trusted(&provider, &staged.path().to_string_lossy());
        run_skill_review_agent(
            &ctx_bg, &ws, &user, &review_id, index, base, &provider, staged.path(), &skill_name,
            &reviewer, &instructions, &cancel,
        )
        .await;
        emit(&ctx_bg, &ws.id, &review_id, "running");
    });
    let review = ctx.skill_reviews_store.get(&id).await.map_err(ApiError)?;
    Ok(Json(review))
}

/// Send the review's findings to a fixer agent that applies them to the REAL
/// skill directory (reviews themselves run on a staged copy). One fixer at a
/// time per review; bundled skills are rejected (read-only).
async fn apply_fixes(
    AxPath(id): AxPath<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<otto_core::api::ApplySkillFixReq>,
) -> ApiResult<Json<SkillReview>> {
    let review = ctx.skill_reviews_store.get(&id).await.map_err(ApiError)?;
    require_ws_role(&ctx, &user, &review.workspace_id, WorkspaceRole::Editor).await?;
    if review.status == "running" {
        return Err(ApiError(otto_core::Error::Invalid(
            "wait for the review to finish before applying fixes".into(),
        )));
    }
    if let Some(fix) = &review.fix_agent {
        if fix.status == "pending" || fix.status == "running" || fix.status == "waiting" {
            return Err(ApiError(otto_core::Error::Invalid(
                "a fix agent is already running for this review".into(),
            )));
        }
    }
    // The findings to hand over: the summarizer's aggregate when present, else
    // the deterministic static report.
    let (findings, patch_plan) = match (&review.summary, &review.static_report) {
        (Some(s), _) => (s.findings.clone(), s.patch_plan.clone()),
        (None, Some(st)) => (st.findings.clone(), Vec::new()),
        (None, None) => (Vec::new(), Vec::new()),
    };
    if findings.is_empty() && patch_plan.is_empty() {
        return Err(ApiError(otto_core::Error::Invalid("this review has no findings to apply".into())));
    }
    let dir = real_skill_dir(&ctx, &review.skill_name, &review.skill_source).map_err(ApiError)?;

    let provider = if req.provider.trim().is_empty() { "claude".to_string() } else { req.provider.trim().to_string() };
    let row = SkillReviewAgent {
        name: "fixer".into(),
        provider: provider.clone(),
        model: String::new(),
        status: "pending".into(),
        note: String::new(),
        session_id: None,
        findings: vec![],
    };
    ctx.skill_reviews_store.set_fix(&id, &row).await.map_err(ApiError)?;

    let ws = ctx.workspaces.get(&review.workspace_id).await.map_err(ApiError)?;
    let out = fix_result_path(&id);
    let prompt = fixer_prompt(&review, &findings, &patch_plan, &req.instructions, &dir, &out.to_string_lossy());
    let ctx_bg = ctx.clone();
    let review_id = id.clone();
    otto_sessions::trust::ensure_trusted(&provider, &dir.to_string_lossy());
    tokio::spawn(async move {
        run_fix_agent(&ctx_bg, &ws, &user, &review_id, row, &provider, &dir, &prompt).await;
    });

    let review = ctx.skill_reviews_store.get(&id).await.map_err(ApiError)?;
    Ok(Json(review))
}

// ---------------------------------------------------------------------------
// Orchestration
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
async fn run_review(
    ctx: ServerCtx,
    review_id: Id,
    ws: Workspace,
    user: User,
    skill_name: String,
    source: String,
    providers: Vec<String>,
    instructions: String,
    cancel: Arc<AtomicBool>,
) {
    let result = run_review_inner(
        &ctx, &review_id, &ws, &user, &skill_name, &source, &providers, &instructions, &cancel,
    )
    .await;
    let status = if is_cancelled(&cancel) {
        "cancelled"
    } else if result.is_err() {
        "error"
    } else {
        "done"
    };
    if let Err(e) = &result {
        let _ = ctx.skill_reviews_store.set_status(&review_id, status, Some(&e.to_string())).await;
    } else {
        let _ = ctx.skill_reviews_store.set_status(&review_id, status, None).await;
    }
    unregister_cancel(&ctx.skill_review_cancels, &review_id);
    emit(&ctx, &ws.id, &review_id, status);
}

#[allow(clippy::too_many_arguments)]
async fn run_review_inner(
    ctx: &ServerCtx,
    review_id: &Id,
    ws: &Workspace,
    user: &User,
    skill_name: &str,
    source: &str,
    providers: &[String],
    instructions: &str,
    cancel: &Arc<AtomicBool>,
) -> Result<()> {
    let staged = stage_target(ctx, skill_name, source)?;
    let dir = staged.path();

    // 1. Static pass — always, instant, deterministic.
    let static_report = static_review(dir);
    ctx.skill_reviews_store.set_static(review_id, &static_report).await?;
    emit(ctx, &ws.id, review_id, "running");

    if providers.is_empty() {
        return Ok(()); // static-only mode
    }

    // 2. Seed agent rows (one per provider + trailing summarizer) so the UI
    //    renders the agent list + embedded terminals immediately.
    let mut agents: Vec<SkillReviewAgent> = providers
        .iter()
        .map(|p| SkillReviewAgent {
            name: p.clone(),
            provider: p.clone(),
            model: String::new(),
            status: "pending".into(),
            note: String::new(),
            session_id: None,
            findings: vec![],
        })
        .collect();
    agents.push(SkillReviewAgent {
        name: "summarizer".into(),
        provider: "claude".into(),
        model: String::new(),
        status: "pending".into(),
        note: String::new(),
        session_id: None,
        findings: vec![],
    });
    ctx.skill_reviews_store.set_agents(review_id, &agents).await?;

    // Pre-trust the staged skill dir for every provider so no agent stalls on the
    // interactive "trust this folder?" dialog.
    let dir_str = dir.to_string_lossy().into_owned();
    for p in providers {
        otto_sessions::trust::ensure_trusted(p, &dir_str);
    }
    let reviewer = reviewer_method();

    // 3. Fan-out the reviewer agents concurrently.
    let mut set = tokio::task::JoinSet::new();
    for (index, p) in providers.iter().enumerate() {
        let ctx = ctx.clone();
        let ws = ws.clone();
        let user = user.clone();
        let review_id = review_id.clone();
        let provider = p.clone();
        let dir = dir.to_path_buf();
        let skill_name = skill_name.to_string();
        let reviewer = reviewer.clone();
        let instructions = instructions.to_string();
        let cancel = Arc::clone(cancel);
        let base = agents[index].clone();
        set.spawn(async move {
            run_skill_review_agent(
                &ctx, &ws, &user, &review_id, index, base, &provider, &dir, &skill_name,
                &reviewer, &instructions, &cancel,
            )
            .await
        });
    }
    let mut agent_batches: Vec<String> = Vec::new();
    let mut agent_findings: Vec<SkillFinding> = Vec::new();
    while let Some(res) = set.join_next().await {
        if let Ok(findings) = res {
            if !findings.is_empty() {
                agent_batches.push(serde_json::to_string(&findings).unwrap_or_default());
                agent_findings.extend(findings);
            }
        }
    }

    if is_cancelled(cancel) {
        return Ok(());
    }

    // 4. Summarizer — visible headless turn; enrich verdict/patch plan.
    let summarizer_index = providers.len();
    let mut summarizer_row = agents[summarizer_index].clone();
    summarizer_row.status = "running".into();
    let _ = ctx.skill_reviews_store.set_agent_at(review_id, summarizer_index, &summarizer_row).await;
    emit(ctx, &ws.id, review_id, "running");

    let mut summary = merge_summary(&static_report, &agent_findings);
    let prompt = summarizer_prompt(skill_name, &static_report, &agent_batches, instructions);
    match ctx.orchestrator.run_agent(&prompt, &dir.to_string_lossy(), None, SUMMARIZER_TIMEOUT).await {
        Ok(text) => {
            if let Some(parsed) = parse_summary(&text) {
                // Prefer the model's verdict + patch plan; keep the merged findings.
                if !parsed.verdict.trim().is_empty() {
                    summary.verdict = parsed.verdict;
                }
                if !parsed.patch_plan.is_empty() {
                    summary.patch_plan = parsed.patch_plan;
                }
                if !parsed.findings.is_empty() {
                    summary.findings = dedupe_findings(
                        summary.findings.into_iter().chain(parsed.findings).collect(),
                    );
                }
            }
            summarizer_row.status = "done".into();
            summarizer_row.note = format!("{} finding(s)", summary.findings.len());
        }
        Err(e) => {
            summarizer_row.status = "error".into();
            summarizer_row.note = format!("summarizer failed: {e}");
        }
    }
    let _ = ctx.skill_reviews_store.set_agent_at(review_id, summarizer_index, &summarizer_row).await;
    ctx.skill_reviews_store.set_summary(review_id, &summary).await?;
    Ok(())
}

/// Emit a workspace-scoped `skill_review_updated` event (best-effort).
fn emit(ctx: &ServerCtx, ws_id: &Id, review_id: &Id, status: &str) {
    let ev = Event::SkillReviewUpdated {
        workspace_id: ws_id.clone(),
        review_id: review_id.clone(),
        status: status.to_string(),
    };
    let _ = ctx.events.send(ev);
}

// ---------------------------------------------------------------------------
// One visible reviewer agent (forked from skill_eval::run_agent_capture)
// ---------------------------------------------------------------------------

fn findings_path(review_id: &str, index: usize) -> PathBuf {
    let dir = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(dir).join(format!("otto-skillreview-{review_id}-{index}.json"))
}

fn fix_result_path(review_id: &str) -> PathBuf {
    let dir = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(dir).join(format!("otto-skillreview-{review_id}-fix.json"))
}

#[allow(clippy::too_many_arguments)]
async fn run_skill_review_agent(
    ctx: &ServerCtx,
    ws: &Workspace,
    user: &User,
    review_id: &Id,
    index: usize,
    mut row: SkillReviewAgent,
    provider: &str,
    dir: &Path,
    skill_name: &str,
    reviewer: &str,
    instructions: &str,
    cancel: &Arc<AtomicBool>,
) -> Vec<SkillFinding> {
    let out = findings_path(review_id, index);
    let _ = std::fs::remove_file(&out);
    let repo = &ctx.skill_reviews_store;
    let cwd = dir.to_string_lossy().into_owned();

    if is_cancelled(cancel) {
        row.status = "error".into();
        let _ = repo.set_agent_at(review_id, index, &row).await;
        return vec![];
    }

    let prompt = agent_prompt(skill_name, dir, reviewer, instructions, &out.to_string_lossy());
    let meta = serde_json::json!({ "source": "skillreview", "review_id": review_id, "agent_index": index });
    let req = CreateSessionReq {
        kind: SessionKind::Agent,
        provider: Some(provider.to_string()),
        title: None,
        cwd: Some(cwd.clone()),
        connection_id: None,
        meta: Some(meta),
    };
    let session = match ctx.manager.create(ws, &user.id, req, None).await {
        Ok(s) => s,
        Err(e) => {
            row.status = "error".into();
            row.note = format!("could not start: {e}");
            let _ = repo.set_agent_at(review_id, index, &row).await;
            return vec![];
        }
    };
    let sid = session.id.clone();
    row.status = "running".into();
    row.session_id = Some(sid.clone());
    row.note = String::new();
    let _ = repo.set_agent_at(review_id, index, &row).await;

    if wait_for_tui(&ctx.manager, &sid).await {
        let _ = ctx.manager.input(&sid, &bracketed_paste(&prompt)).await;
        tokio::time::sleep(PASTE_TO_ENTER).await;
        let before = ctx.manager.live_handle(&sid).map(|h| h.last_output_at());
        let _ = ctx.manager.input(&sid, b"\r").await;
        if !dispatched(&ctx.manager, &sid, before).await {
            let _ = ctx.manager.input(&sid, b"\r").await;
        }
    }

    let deadline = Instant::now() + AGENT_TIMEOUT;
    let mut flagged_waiting = false;
    loop {
        if is_cancelled(cancel) {
            let _ = ctx.manager.archive(&sid).await;
            row.status = "error".into();
            let _ = repo.set_agent_at(review_id, index, &row).await;
            return vec![];
        }
        if let Ok(text) = std::fs::read_to_string(&out) {
            let _ = std::fs::remove_file(&out);
            let findings = parse_skill_findings(&text);
            let n = findings.len();
            row.status = "done".into();
            row.note = format!("{n} finding{}", if n == 1 { "" } else { "s" });
            row.findings = findings.clone();
            let _ = repo.set_agent_at(review_id, index, &row).await;
            return findings;
        }
        if provider == "claude" {
            if let Some(psid) = session.provider_session_id.as_deref() {
                let jsonl = otto_orchestrator::claude_pty::session_jsonl_path(&cwd, psid);
                if let Ok(raw) = std::fs::read_to_string(&jsonl) {
                    if let Some(turn) = otto_orchestrator::claude_pty::completed_turn_text(&raw) {
                        let findings = parse_skill_findings(&turn);
                        if !findings.is_empty() {
                            let n = findings.len();
                            row.status = "done".into();
                            row.note = format!("{n} finding(s)");
                            row.findings = findings.clone();
                            let _ = repo.set_agent_at(review_id, index, &row).await;
                            return findings;
                        }
                    }
                }
            }
        }
        match ctx.manager.live_handle(&sid) {
            Some(handle) => {
                if handle.on_exit().borrow().is_some() {
                    if let Ok(text) = std::fs::read_to_string(&out) {
                        let _ = std::fs::remove_file(&out);
                        let findings = parse_skill_findings(&text);
                        row.status = "done".into();
                        row.note = format!("{} finding(s)", findings.len());
                        row.findings = findings.clone();
                        let _ = repo.set_agent_at(review_id, index, &row).await;
                        return findings;
                    }
                    row.status = "error".into();
                    row.note = "session exited before writing findings".into();
                    let _ = repo.set_agent_at(review_id, index, &row).await;
                    return vec![];
                }
                let idle = handle.last_output_at().elapsed();
                if idle >= WAITING_IDLE && !flagged_waiting {
                    flagged_waiting = true;
                    row.status = "waiting".into();
                    row.note = "looks blocked on input — Open it to respond".into();
                    let _ = repo.set_agent_at(review_id, index, &row).await;
                } else if idle < WAITING_IDLE && flagged_waiting {
                    flagged_waiting = false;
                    row.status = "running".into();
                    row.note = String::new();
                    let _ = repo.set_agent_at(review_id, index, &row).await;
                }
            }
            None => {
                row.status = "error".into();
                row.note = "session is no longer live".into();
                let _ = repo.set_agent_at(review_id, index, &row).await;
                return vec![];
            }
        }
        if Instant::now() >= deadline {
            row.status = "error".into();
            row.note = "timed out".into();
            let _ = repo.set_agent_at(review_id, index, &row).await;
            return vec![];
        }
        tokio::time::sleep(OUTPUT_POLL).await;
    }
}

// ---------------------------------------------------------------------------
// The apply-fixes agent (same visible-PTY pattern; edits the real skill dir)
// ---------------------------------------------------------------------------

/// Lenient parse of the fixer's result file → a short status note.
fn parse_fix_note(text: &str) -> String {
    #[derive(serde::Deserialize, Default)]
    struct RawFix {
        #[serde(default)]
        applied: Vec<String>,
        #[serde(default)]
        skipped: Vec<String>,
        #[serde(default)]
        notes: String,
    }
    let raw: RawFix = slice_json(text, '{', '}')
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    let mut parts = vec![format!("{} fix(es) applied", raw.applied.len())];
    if !raw.skipped.is_empty() {
        parts.push(format!("{} skipped", raw.skipped.len()));
    }
    let mut note = parts.join(", ");
    if !raw.notes.trim().is_empty() {
        note.push_str(" — ");
        note.push_str(raw.notes.trim());
    }
    note
}

#[allow(clippy::too_many_arguments)]
async fn run_fix_agent(
    ctx: &ServerCtx,
    ws: &Workspace,
    user: &User,
    review_id: &Id,
    mut row: SkillReviewAgent,
    provider: &str,
    dir: &Path,
    prompt: &str,
) {
    let out = fix_result_path(review_id);
    let _ = std::fs::remove_file(&out);
    let repo = &ctx.skill_reviews_store;
    let cwd = dir.to_string_lossy().into_owned();

    let meta = serde_json::json!({ "source": "skillreview", "review_id": review_id, "role": "fixer" });
    let req = CreateSessionReq {
        kind: SessionKind::Agent,
        provider: Some(provider.to_string()),
        title: None,
        cwd: Some(cwd.clone()),
        connection_id: None,
        meta: Some(meta),
    };
    let session = match ctx.manager.create(ws, &user.id, req, None).await {
        Ok(s) => s,
        Err(e) => {
            row.status = "error".into();
            row.note = format!("could not start: {e}");
            let _ = repo.set_fix(review_id, &row).await;
            emit(ctx, &ws.id, review_id, "done");
            return;
        }
    };
    let sid = session.id.clone();
    row.status = "running".into();
    row.session_id = Some(sid.clone());
    row.note = String::new();
    let _ = repo.set_fix(review_id, &row).await;
    emit(ctx, &ws.id, review_id, "done");

    if wait_for_tui(&ctx.manager, &sid).await {
        let _ = ctx.manager.input(&sid, &bracketed_paste(prompt)).await;
        tokio::time::sleep(PASTE_TO_ENTER).await;
        let before = ctx.manager.live_handle(&sid).map(|h| h.last_output_at());
        let _ = ctx.manager.input(&sid, b"\r").await;
        if !dispatched(&ctx.manager, &sid, before).await {
            let _ = ctx.manager.input(&sid, b"\r").await;
        }
    }

    let deadline = Instant::now() + FIX_TIMEOUT;
    let mut flagged_waiting = false;
    loop {
        if let Ok(text) = std::fs::read_to_string(&out) {
            let _ = std::fs::remove_file(&out);
            row.status = "done".into();
            row.note = parse_fix_note(&text);
            let _ = repo.set_fix(review_id, &row).await;
            emit(ctx, &ws.id, review_id, "done");
            return;
        }
        match ctx.manager.live_handle(&sid) {
            Some(handle) => {
                if handle.on_exit().borrow().is_some() {
                    row.status = "error".into();
                    row.note = "session exited before writing its result".into();
                    let _ = repo.set_fix(review_id, &row).await;
                    emit(ctx, &ws.id, review_id, "done");
                    return;
                }
                let idle = handle.last_output_at().elapsed();
                if idle >= WAITING_IDLE && !flagged_waiting {
                    flagged_waiting = true;
                    row.status = "waiting".into();
                    row.note = "looks blocked on input — Open it to respond".into();
                    let _ = repo.set_fix(review_id, &row).await;
                    emit(ctx, &ws.id, review_id, "done");
                } else if idle < WAITING_IDLE && flagged_waiting {
                    flagged_waiting = false;
                    row.status = "running".into();
                    row.note = String::new();
                    let _ = repo.set_fix(review_id, &row).await;
                    emit(ctx, &ws.id, review_id, "done");
                }
            }
            None => {
                row.status = "error".into();
                row.note = "session is no longer live".into();
                let _ = repo.set_fix(review_id, &row).await;
                emit(ctx, &ws.id, review_id, "done");
                return;
            }
        }
        if Instant::now() >= deadline {
            row.status = "error".into();
            row.note = "timed out".into();
            let _ = repo.set_fix(review_id, &row).await;
            emit(ctx, &ws.id, review_id, "done");
            return;
        }
        tokio::time::sleep(OUTPUT_POLL).await;
    }
}

// ---------------------------------------------------------------------------
// Prompts
// ---------------------------------------------------------------------------

/// The reviewer method inlined into each agent prompt: the bundled
/// skills-reviewer `SKILL.md` body, or a compact built-in fallback.
fn reviewer_method() -> String {
    otto_skills::bundled_body("skills-reviewer").unwrap_or_else(|| {
        "Review the Agent Skill for spec compliance, trigger precision, workflow quality, \
         examples, references, eval coverage, bloat, conflicts, and risky scripts. Score each \
         0-5 and give a verdict of Ready / Ready with fixes / Do not publish."
            .to_string()
    })
}

/// Render the user's extra instructions as a prompt block ("" when none).
fn instructions_block(instructions: &str) -> String {
    let t = instructions.trim();
    if t.is_empty() {
        String::new()
    } else {
        format!(
            "\n\nAdditional instructions from the user — honor these on top of the review \
method (they may add context such as recent commits fixing a previous review round, or known \
issues from earlier implementations):\n{t}\n"
        )
    }
}

fn agent_prompt(skill_name: &str, dir: &Path, reviewer: &str, instructions: &str, out_path: &str) -> String {
    format!(
        "You are auditing the Agent Skill package `{skill_name}` located at:\n{dir}\n\n\
Read its SKILL.md and every file under references/, examples/, scripts/, evals/. Local machine \
files (e.g. `.mcp.json`, `.DS_Store`, `.git/`, `.env*`) are NOT part of the package — ignore \
them and do not report on them. Apply the following review method:\n\n---\n{reviewer}\n---\
{extra}\n\
When finished, write your findings as a JSON array to this absolute path, overwriting any \
existing content:\n\n{out}\n\n\
Each element MUST be an object with these string fields: \
`severity` (Critical|High|Medium|Low), `code` (a short SCREAMING_SNAKE code), `title`, \
`evidence` (file/quote), `why`, `fix`. Write ONLY the JSON array to that file (no prose, no \
markdown fence). Writing the file is the last thing you do.",
        dir = dir.display(),
        extra = instructions_block(instructions),
        out = out_path,
    )
}

fn summarizer_prompt(
    skill_name: &str,
    static_report: &SkillStaticReport,
    batches: &[String],
    instructions: &str,
) -> String {
    let static_json = serde_json::to_string(static_report).unwrap_or_default();
    format!(
        "Aggregate these skill-review findings for `{skill_name}` into ONE deduped, \
severity-ranked report.\n\nStatic analysis:\n{static_json}\n\nReviewer agent findings (JSON \
arrays):\n{batches}\
{extra}\n\
Respond with ONLY a JSON object (no prose, no fence) with fields: `verdict` \
(Ready|Ready with fixes|Do not publish), `average_score` (0-5 number), `scorecard` (array of \
{{area,score,notes}}), `findings` (array of {{severity,code,title,evidence,why,fix}} — deduped, \
most severe first), `patch_plan` (array of short, highest-leverage-first fix strings).",
        batches = batches.join("\n"),
        extra = instructions_block(instructions),
    )
}

/// The apply-fixes prompt: the review's verdict, patch plan and findings, plus
/// the review-time and apply-time user instructions.
fn fixer_prompt(
    review: &SkillReview,
    findings: &[SkillFinding],
    patch_plan: &[String],
    apply_instructions: &str,
    dir: &Path,
    out_path: &str,
) -> String {
    let findings_json = serde_json::to_string_pretty(findings).unwrap_or_default();
    let plan = if patch_plan.is_empty() {
        String::new()
    } else {
        let steps: String = patch_plan
            .iter()
            .enumerate()
            .map(|(i, s)| format!("{}. {s}\n", i + 1))
            .collect();
        format!("\nPatch plan (highest leverage first):\n{steps}")
    };
    let verdict = review
        .summary
        .as_ref()
        .map(|s| s.verdict.clone())
        .or_else(|| review.static_report.as_ref().map(|s| s.verdict.clone()))
        .unwrap_or_default();
    format!(
        "A multi-agent review of the Agent Skill package `{name}` (verdict: {verdict}) produced \
the findings below. Apply the fixes directly to the package, which lives at:\n{dir}\n\n\
Work through the patch plan and findings most-severe first. Edit files in place; add missing \
files (examples/, evals/evals.json, README…) where a finding calls for them. Skip a finding if \
it is wrong or does not apply — note why instead of forcing a change. Do NOT touch local \
machine files (`.mcp.json`, `.DS_Store`, `.git/`, `.env*`).{plan}\n\
Findings (JSON):\n{findings_json}\
{review_extra}{apply_extra}\n\
When finished, write ONLY a JSON object (no prose, no fence) summarizing what you did to this \
absolute path, overwriting any existing content:\n\n{out}\n\n\
{{\"applied\": [\"…\"], \"skipped\": [\"…\"], \"notes\": \"…\"}} — writing the file is the last \
thing you do.",
        name = review.skill_name,
        dir = dir.display(),
        review_extra = instructions_block(&review.instructions),
        apply_extra = instructions_block(apply_instructions),
        out = out_path,
    )
}

// ---------------------------------------------------------------------------
// Parsing (lenient)
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize, Default)]
struct RawFinding {
    #[serde(default)]
    severity: String,
    #[serde(default)]
    code: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    evidence: String,
    #[serde(default)]
    why: String,
    #[serde(default)]
    fix: String,
}

fn norm_severity(s: &str) -> String {
    match s.trim().to_lowercase().as_str() {
        "critical" => "Critical",
        "high" => "High",
        "low" => "Low",
        "" | "medium" => "Medium",
        _ => "Medium",
    }
    .to_string()
}

/// Extract the first balanced JSON array/object substring from `text`.
fn slice_json(text: &str, open: char, close: char) -> Option<&str> {
    let start = text.find(open)?;
    let mut depth = 0i32;
    let mut in_str = false;
    let mut esc = false;
    for (i, c) in text[start..].char_indices() {
        if in_str {
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            x if x == open => depth += 1,
            x if x == close => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[start..start + i + c.len_utf8()]);
                }
            }
            _ => {}
        }
    }
    None
}

fn parse_skill_findings(text: &str) -> Vec<SkillFinding> {
    let Some(arr) = slice_json(text, '[', ']') else {
        return vec![];
    };
    let raw: Vec<RawFinding> = serde_json::from_str(arr).unwrap_or_default();
    raw.into_iter()
        .filter(|r| !(r.title.trim().is_empty() && r.evidence.trim().is_empty()))
        .map(|r| SkillFinding {
            severity: norm_severity(&r.severity),
            code: r.code,
            title: r.title,
            evidence: r.evidence,
            why: r.why,
            fix: r.fix,
        })
        .collect()
}

#[derive(serde::Deserialize, Default)]
struct RawSummary {
    #[serde(default)]
    verdict: String,
    #[serde(default)]
    average_score: f32,
    #[serde(default)]
    scorecard: Vec<RawScore>,
    #[serde(default)]
    findings: Vec<RawFinding>,
    #[serde(default)]
    patch_plan: Vec<String>,
}
#[derive(serde::Deserialize, Default)]
struct RawScore {
    #[serde(default)]
    area: String,
    #[serde(default)]
    score: f32,
    #[serde(default)]
    notes: String,
}

fn parse_summary(text: &str) -> Option<SkillReviewSummary> {
    let obj = slice_json(text, '{', '}')?;
    let raw: RawSummary = serde_json::from_str(obj).ok()?;
    Some(SkillReviewSummary {
        verdict: raw.verdict,
        average_score: raw.average_score,
        scorecard: raw
            .scorecard
            .into_iter()
            .map(|s| SkillScoreRow { area: s.area, score: s.score.round().clamp(0.0, 5.0) as u8, notes: s.notes })
            .collect(),
        findings: raw
            .findings
            .into_iter()
            .filter(|r| !(r.title.trim().is_empty() && r.evidence.trim().is_empty()))
            .map(|r| SkillFinding {
                severity: norm_severity(&r.severity),
                code: r.code,
                title: r.title,
                evidence: r.evidence,
                why: r.why,
                fix: r.fix,
            })
            .collect(),
        patch_plan: raw.patch_plan,
    })
}

fn dedupe_findings(findings: Vec<SkillFinding>) -> Vec<SkillFinding> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out = Vec::new();
    for f in findings {
        let key = format!("{}|{}", f.code.to_lowercase(), f.title.to_lowercase());
        if seen.insert(key) {
            out.push(f);
        }
    }
    out.sort_by_key(|f| sev_rank(&f.severity));
    out
}

fn sev_rank(s: &str) -> u8 {
    match s {
        "Critical" => 0,
        "High" => 1,
        "Medium" => 2,
        _ => 3,
    }
}

/// Build the agents-mode summary from the deterministic static report plus the
/// union of agent findings (the model's summarizer output enriches this).
fn merge_summary(static_report: &SkillStaticReport, agent_findings: &[SkillFinding]) -> SkillReviewSummary {
    let findings = dedupe_findings(
        static_report
            .findings
            .iter()
            .cloned()
            .chain(agent_findings.iter().cloned())
            .collect(),
    );
    let patch_plan: Vec<String> = findings
        .iter()
        .filter(|f| f.severity == "Critical" || f.severity == "High")
        .take(5)
        .map(|f| if f.fix.trim().is_empty() { f.title.clone() } else { f.fix.clone() })
        .collect();
    SkillReviewSummary {
        verdict: static_report.verdict.clone(),
        average_score: static_report.average_score,
        scorecard: static_report.scorecard.clone(),
        findings,
        patch_plan,
    }
}

// ---------------------------------------------------------------------------
// Native static reviewer (deterministic port of scripts/skill_review.py)
// ---------------------------------------------------------------------------

include!("skill_review_static.rs");
