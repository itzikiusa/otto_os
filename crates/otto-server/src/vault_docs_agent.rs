//! Vault docs agents — AI writers that produce documentation INTO a vault.
//!
//! Two flows, both built on [`crate::agent_session::run_session_turn`] (real,
//! visible, resumable managed sessions — never throwaway PTYs):
//!
//! - **run**: 1..=4 WRITER agents (per-agent provider/model) fan out
//!   concurrently. A single writer writes final notes straight into
//!   `target_dir`; with >1 writers each drafts under
//!   `_drafts/docs-run-<run8>/agent-<n>/` and one SUMMARIZER session
//!   consolidates the drafts into `target_dir`, after which the whole drafts
//!   dir is moved to `<vault>/.trash/` (soft — never destroys files). Agents
//!   write through the session-injected otto MCP vault tools
//!   (`otto_vault_write` etc.), so every note lands via the engine's guarded
//!   path checks + rescan. Mirrors the `product_run` fan-out + summarizer
//!   shape (JoinSet, per-agent error isolation, sessions left open).
//! - **refine**: ONE resumed session per (vault, note) applies user
//!   instructions to an existing note across turns (the `canvas_assist`
//!   one-session-per-artifact pattern; provider honored on the FIRST turn only).
//!
//! Run state is IN-MEMORY and poll-only: runs do NOT survive a daemon restart
//! (the sessions themselves do — they're ordinary managed sessions). Cancel
//! stops orchestrating between stages but never kills the agent sessions
//! (they're the user's, visible and closeable in the UI).
//!
//! OKF vaults (`vault.okf`): notes MUST be OKF-conformant. claude agents get
//! the staged `okf-authoring` skill library via `meta.extra_dirs` →
//! `--add-dir` (see `modules::stage_review_skills`); codex/agy get the skill's
//! SKILL.md text inlined into the prompt (they can't load out-of-tree skills).
//!
//! Contract: `docs/contracts/api.md` §"Vault v3 — the docs home" (docs-agents
//! table); DTOs mirrored in `ui/src/lib/api/types.ts`.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use otto_core::domain::{User, Workspace, WorkspaceRole};
use otto_core::{Error, Id};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::warn;

use crate::auth::CurrentUser;
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

/// Total chars of draft content inlined into the summarizer prompt. Drafts
/// beyond the cap are listed by path only (the agent reads them via
/// `otto_vault_read`); the cap is stated in the prompt so the agent knows.
const DRAFT_INLINE_CAP: usize = 60_000;
/// Chars of the current note inlined into the FIRST refine turn.
const REFINE_INLINE_CAP: usize = 30_000;

// ---------------------------------------------------------------------------
// DTOs — exactly the shapes in the binding contract (serde snake_case).
// ---------------------------------------------------------------------------

/// One writer agent's live view inside a run.
#[derive(Clone, Debug, Serialize)]
pub struct VaultDocsAgent {
    pub index: usize,
    /// Display name, e.g. `"writer-1 · claude"`.
    pub name: String,
    pub provider: String,
    pub model: Option<String>,
    /// `pending | running | done | error`.
    pub state: String,
    pub session_id: Option<String>,
    pub error: Option<String>,
    /// Vault-relative draft note paths this agent produced (multi-writer only).
    pub drafts: Vec<String>,
}

/// The consolidation agent's live view (skipped entirely for a single writer).
#[derive(Clone, Debug, Serialize)]
pub struct VaultDocsSummarizer {
    pub provider: String,
    pub model: Option<String>,
    /// `pending | running | done | error | skipped`.
    pub state: String,
    pub session_id: Option<String>,
    pub error: Option<String>,
}

/// A whole docs run — the poll snapshot the UI renders every 1500ms.
#[derive(Clone, Debug, Serialize)]
pub struct VaultDocsRun {
    pub id: String,
    pub ws_id: String,
    pub vault_id: i64,
    pub prompt: String,
    pub target_dir: String,
    /// `running | summarizing | done | error | cancelled`.
    pub state: String,
    pub agents: Vec<VaultDocsAgent>,
    pub summarizer: VaultDocsSummarizer,
    /// Final note paths in the vault (agent-reported, else server-side diff).
    pub written: Vec<String>,
    pub error: Option<String>,
    pub started_at: String,
    pub finished_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RunReq {
    pub prompt: String,
    /// Vault-relative folder the finals land in (`""`/absent = vault root).
    #[serde(default)]
    pub target_dir: Option<String>,
    /// 1..=4 writer agents.
    pub agents: Vec<AgentReq>,
    #[serde(default)]
    pub summarizer: Option<SummarizerReq>,
}

#[derive(Debug, Deserialize)]
pub struct AgentReq {
    pub provider: String,
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct SummarizerReq {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RefineReq {
    pub path: String,
    pub prompt: String,
    /// Honored on the FIRST turn only (later turns resume the same session).
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RefineResp {
    pub session_id: String,
    pub reply: String,
}

#[derive(Debug, Deserialize)]
pub struct RefineSessionQ {
    pub path: String,
}

#[derive(Debug, Serialize)]
pub struct RefineSessionResp {
    pub session_id: Option<String>,
    pub running: bool,
}

// ---------------------------------------------------------------------------
// In-memory registries (on ServerCtx; constructed in ottod main + test ctxs).
// Runs are poll-only and do NOT survive a daemon restart — by design (the
// sessions themselves persist as ordinary managed sessions).
// ---------------------------------------------------------------------------

/// One live run: its poll snapshot + the cancel flag the orchestrator checks
/// between stages (mirrors the `product_run::CancelRegistry` shape).
pub struct RunEntry {
    pub run: VaultDocsRun,
    pub cancel: Arc<AtomicBool>,
}

pub type RunRegistry = Arc<Mutex<HashMap<String, RunEntry>>>;

pub fn new_run_registry() -> RunRegistry {
    Arc::new(Mutex::new(HashMap::new()))
}

/// The resumable refine session for one (vault, note) — `running` flips true
/// for the duration of a turn so the UI knows when to attach the live shell.
#[derive(Clone, Default)]
pub struct RefineEntry {
    pub session_id: Option<Id>,
    pub running: bool,
}

pub type RefineRegistry = Arc<Mutex<HashMap<(i64, String), RefineEntry>>>;

pub fn new_refine_registry() -> RefineRegistry {
    Arc::new(Mutex::new(HashMap::new()))
}

/// Mutate one run's snapshot under the registry lock (no-op when gone).
fn with_run(reg: &RunRegistry, run_id: &str, f: impl FnOnce(&mut VaultDocsRun)) {
    if let Some(e) = reg.lock().unwrap().get_mut(run_id) {
        f(&mut e.run);
    }
}

/// True when the run reached a terminal state (guards late stage transitions
/// from clobbering a cancel that landed while a stage was in flight).
fn is_terminal(state: &str) -> bool {
    matches!(state, "done" | "error" | "cancelled")
}

/// Set a terminal run state + `finished_at`, unless one already landed.
fn finish_run(reg: &RunRegistry, run_id: &str, state: &str, error: Option<String>) {
    with_run(reg, run_id, |r| {
        if !is_terminal(&r.state) {
            r.state = state.to_string();
            r.error = error;
            r.finished_at = Some(chrono::Utc::now().to_rfc3339());
        }
    });
}

// ---------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------

pub fn routes() -> Router<ServerCtx> {
    Router::new()
        .route(
            "/workspaces/{ws}/vault/vaults/{id}/docs-agents/run",
            post(start_run),
        )
        .route(
            "/workspaces/{ws}/vault/vaults/{id}/docs-agents/refine",
            post(refine),
        )
        .route(
            "/workspaces/{ws}/vault/vaults/{id}/docs-agents/refine-session",
            get(refine_session),
        )
        // NOT ws-scoped: the run id carries its ws, and the handlers re-check
        // the caller's role against it (policy row: /vault/docs-agents/*).
        .route("/vault/docs-agents/runs/{run_id}", get(get_run))
        .route("/vault/docs-agents/runs/{run_id}/cancel", post(cancel_run))
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `POST /workspaces/{ws}/vault/vaults/{id}/docs-agents/run` — validate, seed
/// the registry snapshot, spawn the orchestration task, return the snapshot.
async fn start_run(
    Path((ws_id, vault_id)): Path<(String, i64)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<RunReq>,
) -> ApiResult<Json<VaultDocsRun>> {
    crate::auth::require_ws_role(&ctx, &user, &Id::from(ws_id.clone()), WorkspaceRole::Editor)
        .await?;
    let vault = ctx
        .vault
        .get_scoped(&ws_id, vault_id)
        .await
        .map_err(ApiError)?;
    let ws = ctx
        .workspaces
        .get(&Id::from(ws_id.clone()))
        .await
        .map_err(ApiError)?;

    if req.prompt.trim().is_empty() {
        return Err(ApiError(Error::Invalid("prompt is required".into())));
    }
    if req.agents.is_empty() || req.agents.len() > 4 {
        return Err(ApiError(Error::Invalid("agents must be 1..=4".into())));
    }
    let target_dir = normalize_target_dir(req.target_dir.as_deref().unwrap_or(""))?;

    // Provider precedence per agent: request → workspace default → global
    // default → claude (the mockup_assist / Discovery Chat shape).
    let global_default = otto_state::SettingsRepo::new(ctx.pool.clone())
        .get("default_provider")
        .await
        .ok()
        .flatten();
    let resolve = |explicit: &str| {
        otto_core::provider::resolve_provider(&[
            explicit,
            otto_core::provider::workspace_default(&ws.settings),
            otto_core::provider::global_default(global_default.as_ref()),
        ])
    };

    let writers: Vec<WriterSpec> = req
        .agents
        .iter()
        .map(|a| WriterSpec {
            provider: resolve(&a.provider),
            model: a.model.clone().filter(|m| !m.trim().is_empty()),
        })
        .collect();
    let sum_req = req.summarizer.unwrap_or_default();
    let summarizer = WriterSpec {
        provider: resolve(sum_req.provider.as_deref().unwrap_or("")),
        model: sum_req.model.clone().filter(|m| !m.trim().is_empty()),
    };

    let run_id = otto_core::new_id().to_string();
    let multi = writers.len() > 1;
    let run = VaultDocsRun {
        id: run_id.clone(),
        ws_id: ws_id.clone(),
        vault_id,
        prompt: req.prompt.clone(),
        target_dir: target_dir.clone(),
        state: "running".into(),
        agents: writers
            .iter()
            .enumerate()
            .map(|(i, w)| VaultDocsAgent {
                index: i,
                name: format!("writer-{} \u{00b7} {}", i + 1, w.provider),
                provider: w.provider.clone(),
                model: w.model.clone(),
                state: "pending".into(),
                session_id: None,
                error: None,
                drafts: Vec::new(),
            })
            .collect(),
        summarizer: VaultDocsSummarizer {
            provider: summarizer.provider.clone(),
            model: summarizer.model.clone(),
            state: if multi { "pending" } else { "skipped" }.into(),
            session_id: None,
            error: None,
        },
        written: Vec::new(),
        error: None,
        started_at: chrono::Utc::now().to_rfc3339(),
        finished_at: None,
    };
    ctx.vault_docs_runs.lock().unwrap().insert(
        run_id.clone(),
        RunEntry {
            run: run.clone(),
            cancel: Arc::new(AtomicBool::new(false)),
        },
    );

    tokio::spawn(run_docs(
        ctx.clone(),
        ws,
        user,
        vault,
        run_id,
        req.prompt,
        target_dir,
        writers,
        summarizer,
    ));

    Ok(Json(run))
}

/// `GET /vault/docs-agents/runs/{run_id}` — the poll snapshot. The run's ws is
/// re-checked against the caller (the path itself is not ws-scoped).
async fn get_run(
    Path(run_id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<VaultDocsRun>> {
    let run = ctx
        .vault_docs_runs
        .lock()
        .unwrap()
        .get(&run_id)
        .map(|e| e.run.clone())
        .ok_or_else(|| ApiError(Error::NotFound(format!("docs run {run_id}"))))?;
    crate::auth::require_ws_role(
        &ctx,
        &user,
        &Id::from(run.ws_id.clone()),
        WorkspaceRole::Viewer,
    )
    .await?;
    Ok(Json(run))
}

/// `POST /vault/docs-agents/runs/{run_id}/cancel` — trip the flag and mark the
/// run cancelled NOW (the UI must not keep polling a "running" corpse while a
/// long writer turn drains). The agent sessions are NOT killed — they're real,
/// visible sessions the user can inspect/close; we only stop orchestrating.
async fn cancel_run(
    Path(run_id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<axum::http::StatusCode> {
    let (ws_id, cancel) = {
        let reg = ctx.vault_docs_runs.lock().unwrap();
        let e = reg
            .get(&run_id)
            .ok_or_else(|| ApiError(Error::NotFound(format!("docs run {run_id}"))))?;
        (e.run.ws_id.clone(), Arc::clone(&e.cancel))
    };
    crate::auth::require_ws_role(&ctx, &user, &Id::from(ws_id), WorkspaceRole::Editor).await?;
    cancel.store(true, Ordering::Relaxed);
    finish_run(&ctx.vault_docs_runs, &run_id, "cancelled", None);
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// `POST /workspaces/{ws}/vault/vaults/{id}/docs-agents/refine` — one resumed
/// session per (vault, note); returns when the turn completes (LONG request).
async fn refine(
    Path((ws_id, vault_id)): Path<(String, i64)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<RefineReq>,
) -> ApiResult<Json<RefineResp>> {
    crate::auth::require_ws_role(&ctx, &user, &Id::from(ws_id.clone()), WorkspaceRole::Editor)
        .await?;
    let vault = ctx
        .vault
        .get_scoped(&ws_id, vault_id)
        .await
        .map_err(ApiError)?;
    let ws = ctx
        .workspaces
        .get(&Id::from(ws_id.clone()))
        .await
        .map_err(ApiError)?;
    let note = ctx
        .vault
        .note(&ws_id, vault_id, &req.path)
        .await
        .map_err(ApiError)?;
    let key = (vault_id, req.path.clone());

    // Resume the note's session when one exists; the request's provider is
    // honored on the FIRST turn only. A resumed turn must poll the transcript
    // of the session's REAL provider, so read it back from the session record.
    let existing = ctx
        .vault_docs_refine
        .lock()
        .unwrap()
        .get(&key)
        .and_then(|e| e.session_id.clone());
    let first_turn = existing.is_none();
    let global_default = otto_state::SettingsRepo::new(ctx.pool.clone())
        .get("default_provider")
        .await
        .ok()
        .flatten();
    let mut provider = otto_core::provider::resolve_provider(&[
        req.provider.as_deref().unwrap_or(""),
        otto_core::provider::workspace_default(&ws.settings),
        otto_core::provider::global_default(global_default.as_ref()),
    ]);
    if let Some(sid) = &existing {
        if let Ok(s) = ctx.manager.get(sid).await {
            provider = s.provider;
        }
    }
    otto_sessions::trust::ensure_trusted(&provider, &vault.root_path);

    let okf_inline = okf_inline_for(&ctx, &vault, &provider);
    let prompt = build_refine_prompt(
        &req.prompt,
        vault_id,
        &req.path,
        &note.meta.hash,
        first_turn.then_some(note.raw.as_str()),
        vault.okf,
        okf_inline.as_deref(),
    );
    let mut meta = serde_json::json!({
        "source": "vault-docs",
        "vault_id": vault_id,
    });
    if let Some(m) = &req.model {
        meta["model"] = Value::String(m.clone());
    }
    apply_okf_extra_dirs(&ctx, &vault, &provider, &mut meta);

    // Mark the turn in flight + surface the session id the MOMENT it exists,
    // so `GET refine-session` can attach the live shell while the turn runs.
    {
        let mut reg = ctx.vault_docs_refine.lock().unwrap();
        let e = reg.entry(key.clone()).or_default();
        e.running = true;
    }
    let ready_reg = ctx.vault_docs_refine.clone();
    let ready_key = key.clone();
    let on_ready = move |sid: &Id| {
        let mut reg = ready_reg.lock().unwrap();
        let e = reg.entry(ready_key).or_default();
        e.session_id = Some(sid.clone());
    };

    let turn = crate::agent_session::run_session_turn(
        &ctx,
        &ws,
        &user,
        existing.as_ref(),
        &format!("Vault refine: {}", req.path),
        &vault.root_path,
        &provider,
        meta,
        &prompt,
        crate::agent_session::STUCK_IDLE,
        on_ready,
    )
    .await;

    // The turn is over either way — clear `running` before propagating errors.
    if let Some(e) = ctx.vault_docs_refine.lock().unwrap().get_mut(&key) {
        e.running = false;
    }
    let (reply, sid) = turn?;
    if let Some(e) = ctx.vault_docs_refine.lock().unwrap().get_mut(&key) {
        e.session_id = Some(sid.clone());
    }
    Ok(Json(RefineResp {
        session_id: sid.to_string(),
        reply,
    }))
}

/// `GET /workspaces/{ws}/vault/vaults/{id}/docs-agents/refine-session?path=` —
/// the (vault, note) session + whether a turn is currently in flight.
async fn refine_session(
    Path((ws_id, vault_id)): Path<(String, i64)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Query(q): Query<RefineSessionQ>,
) -> ApiResult<Json<RefineSessionResp>> {
    crate::auth::require_ws_role(&ctx, &user, &Id::from(ws_id.clone()), WorkspaceRole::Viewer)
        .await?;
    // Scope check only — the registry key is (vault_id, path).
    ctx.vault
        .get_scoped(&ws_id, vault_id)
        .await
        .map_err(ApiError)?;
    let entry = ctx
        .vault_docs_refine
        .lock()
        .unwrap()
        .get(&(vault_id, q.path))
        .cloned()
        .unwrap_or_default();
    Ok(Json(RefineSessionResp {
        session_id: entry.session_id.map(|s| s.to_string()),
        running: entry.running,
    }))
}

// ---------------------------------------------------------------------------
// Orchestration (background task; all errors isolated — never panics)
// ---------------------------------------------------------------------------

/// One writer's resolved spec (post provider-precedence).
#[derive(Clone)]
struct WriterSpec {
    provider: String,
    model: Option<String>,
}

/// The whole run: writers fan out → (multi) summarizer consolidates → drafts
/// dir soft-trashed → `written` resolved (results file, else before/after diff).
#[allow(clippy::too_many_arguments)]
async fn run_docs(
    ctx: ServerCtx,
    ws: Workspace,
    user: User,
    vault: otto_vault::VaultRec,
    run_id: String,
    prompt: String,
    target_dir: String,
    writers: Vec<WriterSpec>,
    summarizer: WriterSpec,
) {
    let reg = ctx.vault_docs_runs.clone();
    let cancel = match reg.lock().unwrap().get(&run_id) {
        Some(e) => Arc::clone(&e.cancel),
        None => return,
    };
    let m = writers.len();
    let run8: String = run_id.chars().take(8).collect();

    // The FINAL writer (single-writer or summarizer) reports what it wrote to
    // this pre-created temp file; parsed tolerantly, with a before/after diff
    // of note paths under target_dir as the fallback.
    let results_path = std::env::temp_dir().join(format!("otto-vaultdocs-{run_id}.json"));
    let _ = std::fs::write(&results_path, "");
    let results_str = results_path.to_string_lossy().to_string();

    // Pre-trust every distinct provider on the vault root (mirrors product_run)
    // so no session stalls on the interactive "trust this folder?" prompt.
    {
        let mut trusted = HashSet::<String>::new();
        for p in writers
            .iter()
            .map(|w| w.provider.clone())
            .chain(std::iter::once(summarizer.provider.clone()))
        {
            if trusted.insert(p.clone()) {
                otto_sessions::trust::ensure_trusted(&p, &vault.root_path);
            }
        }
    }

    // Baseline note-path snapshot under target_dir (the `written` fallback).
    let _ = ctx.vault.scan(vault.id).await;
    let before: HashSet<String> = note_paths(&ctx, vault.id).await;

    // OKF vehicles, resolved once: the staged skill bundle for claude (via
    // meta.extra_dirs) and the inlined SKILL.md text for everyone else.
    let okf_bundle = vault
        .okf
        .then(|| {
            crate::modules::stage_review_skills(&ctx.context_library, &["okf-authoring".into()])
        })
        .flatten();
    let okf_text = vault
        .okf
        .then(|| okf_skill_text(&ctx.context_library))
        .filter(|t| !t.is_empty());

    // ---- Stage 1: writers, concurrently (per-writer errors isolated) --------
    let mut set = tokio::task::JoinSet::new();
    for (i, w) in writers.iter().cloned().enumerate() {
        let ctx = ctx.clone();
        let ws = ws.clone();
        let user = user.clone();
        let reg = reg.clone();
        let run_id = run_id.clone();
        let run8 = run8.clone();
        let root = vault.root_path.clone();
        let prompt = build_writer_prompt(
            &prompt,
            vault.id,
            i + 1,
            m,
            &run8,
            &target_dir,
            vault.okf,
            inline_for_provider(&w.provider, okf_text.as_deref()),
            (m == 1).then_some(results_str.as_str()),
        );
        let mut meta = serde_json::json!({
            "source": "vault-docs",
            "vault_id": vault.id,
            "run_id": run_id.clone(),
        });
        if let Some(model) = &w.model {
            meta["model"] = Value::String(model.clone());
        }
        if let Some(dirs) =
            crate::review_session::review_skills_extra_dirs(&w.provider, okf_bundle.as_deref())
        {
            meta["extra_dirs"] = dirs;
        }
        set.spawn(async move {
            let ready_reg = reg.clone();
            let ready_run = run_id.clone();
            let on_ready = move |sid: &Id| {
                let sid = sid.to_string();
                with_run(&ready_reg, &ready_run, |r| {
                    if let Some(a) = r.agents.get_mut(i) {
                        a.state = "running".into();
                        a.session_id = Some(sid);
                    }
                });
            };
            let res = crate::agent_session::run_session_turn(
                &ctx,
                &ws,
                &user,
                None,
                &format!("Vault docs: writer {}/{}", i + 1, m),
                &root,
                &w.provider,
                meta,
                &prompt,
                crate::agent_session::STUCK_IDLE,
                on_ready,
            )
            .await;
            match res {
                Ok((_reply, sid)) => {
                    with_run(&reg, &run_id, |r| {
                        if let Some(a) = r.agents.get_mut(i) {
                            a.state = "done".into();
                            a.session_id = Some(sid.to_string());
                        }
                    });
                    true
                }
                Err(e) => {
                    warn!("vault_docs: writer {} failed: {}", i + 1, e.0);
                    with_run(&reg, &run_id, |r| {
                        if let Some(a) = r.agents.get_mut(i) {
                            a.state = "error".into();
                            a.error = Some(e.0.to_string());
                        }
                    });
                    false
                }
            }
        });
    }
    let mut any_ok = false;
    while let Some(joined) = set.join_next().await {
        if let Ok(ok) = joined {
            any_ok = any_ok || ok;
        }
    }

    if cancel.load(Ordering::Relaxed) {
        finish_run(&reg, &run_id, "cancelled", None);
        let _ = std::fs::remove_file(&results_path);
        return;
    }

    // ---- Stage 2 (single writer): finals are already in target_dir ----------
    if m == 1 {
        let _ = ctx.vault.scan(vault.id).await;
        let after = note_paths(&ctx, vault.id).await;
        let written = resolve_written(&results_path, &before, &after, &target_dir);
        with_run(&reg, &run_id, |r| r.written = written);
        if any_ok {
            finish_run(&reg, &run_id, "done", None);
        } else {
            finish_run(&reg, &run_id, "error", Some("writer agent failed".into()));
        }
        let _ = std::fs::remove_file(&results_path);
        return;
    }

    // ---- Stage 2 (multi): collect drafts, then the summarizer ---------------
    let _ = ctx.vault.scan(vault.id).await;
    let all_after_writers = note_paths(&ctx, vault.id).await;
    let mut drafts: Vec<(String, String)> = Vec::new(); // (path, raw content)
    for i in 0..m {
        let prefix = format!("{}/", draft_dir(&run8, i + 1));
        let mut agent_drafts: Vec<String> = all_after_writers
            .iter()
            .filter(|p| p.starts_with(&prefix))
            .cloned()
            .collect();
        agent_drafts.sort();
        for p in &agent_drafts {
            let raw = match ctx.vault.note(&vault.ws_id, vault.id, p).await {
                Ok(n) => n.raw,
                Err(_) => String::new(),
            };
            drafts.push((p.clone(), raw));
        }
        with_run(&reg, &run_id, |r| {
            if let Some(a) = r.agents.get_mut(i) {
                a.drafts = agent_drafts;
            }
        });
    }

    if !any_ok {
        // Nothing to consolidate — every writer errored.
        with_run(&reg, &run_id, |r| r.summarizer.state = "skipped".into());
        finish_run(
            &reg,
            &run_id,
            "error",
            Some("all writer agents failed".into()),
        );
        let _ = std::fs::remove_file(&results_path);
        return;
    }
    if cancel.load(Ordering::Relaxed) {
        finish_run(&reg, &run_id, "cancelled", None);
        let _ = std::fs::remove_file(&results_path);
        return;
    }

    with_run(&reg, &run_id, |r| {
        if !is_terminal(&r.state) {
            r.state = "summarizing".into();
        }
        r.summarizer.state = "running".into();
    });

    let sum_prompt = build_summarizer_prompt(
        &prompt,
        vault.id,
        m,
        &target_dir,
        &drafts,
        vault.okf,
        inline_for_provider(&summarizer.provider, okf_text.as_deref()),
        &results_str,
    );
    let mut sum_meta = serde_json::json!({
        "source": "vault-docs",
        "vault_id": vault.id,
        "run_id": run_id.clone(),
    });
    if let Some(model) = &summarizer.model {
        sum_meta["model"] = Value::String(model.clone());
    }
    if let Some(dirs) =
        crate::review_session::review_skills_extra_dirs(&summarizer.provider, okf_bundle.as_deref())
    {
        sum_meta["extra_dirs"] = dirs;
    }
    let ready_reg = reg.clone();
    let ready_run = run_id.clone();
    let on_ready = move |sid: &Id| {
        let sid = sid.to_string();
        with_run(&ready_reg, &ready_run, |r| {
            r.summarizer.session_id = Some(sid);
        });
    };
    let sum_res = crate::agent_session::run_session_turn(
        &ctx,
        &ws,
        &user,
        None,
        "Vault docs: summarizer",
        &vault.root_path,
        &summarizer.provider,
        sum_meta,
        &sum_prompt,
        crate::agent_session::STUCK_IDLE,
        on_ready,
    )
    .await;

    let sum_err = match sum_res {
        Ok((_reply, sid)) => {
            with_run(&reg, &run_id, |r| {
                r.summarizer.state = "done".into();
                r.summarizer.session_id = Some(sid.to_string());
            });
            None
        }
        Err(e) => {
            warn!("vault_docs: summarizer failed: {}", e.0);
            with_run(&reg, &run_id, |r| {
                r.summarizer.state = "error".into();
                r.summarizer.error = Some(e.0.to_string());
            });
            Some(e.0.to_string())
        }
    };

    // ---- Stage 3: soft-trash the drafts dir + resolve `written` -------------
    // std::fs::rename is atomic within a filesystem; a cross-device rename (or
    // a missing drafts dir, e.g. the E2E stub) fails and is deliberately
    // ignored — a leftover `_drafts/` is harmless and user-visible.
    let src = std::path::Path::new(&vault.root_path)
        .join("_drafts")
        .join(format!("docs-run-{run8}"));
    let trash = std::path::Path::new(&vault.root_path).join(".trash");
    let _ = std::fs::create_dir_all(&trash);
    let _ = std::fs::rename(&src, trash.join(format!("docs-run-{run8}")));
    let _ = ctx.vault.scan(vault.id).await;

    let after = note_paths(&ctx, vault.id).await;
    let written = resolve_written(&results_path, &before, &after, &target_dir);
    with_run(&reg, &run_id, |r| r.written = written);
    match sum_err {
        None => finish_run(&reg, &run_id, "done", None),
        Some(e) => finish_run(
            &reg,
            &run_id,
            "error",
            Some(format!("summarizer failed: {e}")),
        ),
    }
    let _ = std::fs::remove_file(&results_path);
}

/// All indexed note paths of a vault (recursive; the fallback-diff universe).
async fn note_paths(ctx: &ServerCtx, vault_id: i64) -> HashSet<String> {
    match ctx.vault.store().all_notes(vault_id).await {
        Ok(rows) => rows.into_iter().map(|(path, ..)| path).collect(),
        Err(e) => {
            warn!("vault_docs: all_notes({vault_id}): {e}");
            HashSet::new()
        }
    }
}

/// `written` = the final agent's results file when it parses to a non-empty
/// list of paths that really exist in the vault; otherwise the server-side
/// before/after diff of notes under `target_dir`.
fn resolve_written(
    results_path: &std::path::Path,
    before: &HashSet<String>,
    after: &HashSet<String>,
    target_dir: &str,
) -> Vec<String> {
    let reported = std::fs::read_to_string(results_path)
        .ok()
        .and_then(|c| parse_results(&c))
        .map(|paths| {
            paths
                .into_iter()
                .filter(|p| after.contains(p))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if !reported.is_empty() {
        return reported;
    }
    written_fallback(before, after, target_dir)
}

// ---------------------------------------------------------------------------
// OKF helpers
// ---------------------------------------------------------------------------

/// The `okf-authoring` skill's instructional text for inlining: Library /
/// operator dirs first (via the shared review resolver), else the compiled-in
/// bundled copy in `otto-skills`.
fn okf_skill_text(library: &otto_context::Library) -> String {
    let t = crate::modules::resolve_skill_inline(library, "okf-authoring");
    if !t.is_empty() {
        return t;
    }
    otto_skills::bundled_body("okf-authoring").unwrap_or_default()
}

/// The inlined-skill text for this provider: `None` for claude (it gets the
/// staged bundle via extra_dirs instead), the SKILL.md text otherwise.
fn inline_for_provider<'a>(provider: &str, okf_text: Option<&'a str>) -> Option<&'a str> {
    (provider != "claude").then_some(okf_text).flatten()
}

/// Refine-path variant of the OKF plumbing above (single provider, so the
/// bundle staging + inline decision collapse into two small helpers).
fn okf_inline_for(ctx: &ServerCtx, vault: &otto_vault::VaultRec, provider: &str) -> Option<String> {
    if !vault.okf || provider == "claude" {
        return None;
    }
    Some(okf_skill_text(&ctx.context_library)).filter(|t| !t.is_empty())
}

/// Attach the staged okf-authoring bundle to a claude session's meta (OKF
/// vaults only). No-op for other providers — they get the text inlined.
fn apply_okf_extra_dirs(
    ctx: &ServerCtx,
    vault: &otto_vault::VaultRec,
    provider: &str,
    meta: &mut Value,
) {
    if !vault.okf {
        return;
    }
    let bundle =
        crate::modules::stage_review_skills(&ctx.context_library, &["okf-authoring".into()]);
    if let Some(dirs) = crate::review_session::review_skills_extra_dirs(provider, bundle.as_deref())
    {
        meta["extra_dirs"] = dirs;
    }
}

// ---------------------------------------------------------------------------
// Pure helpers + prompt builders (unit-tested; no DB / no agent)
// ---------------------------------------------------------------------------

/// The vault-relative drafts folder for writer `n` (1-based) of a run.
fn draft_dir(run8: &str, n: usize) -> String {
    format!("_drafts/docs-run-{run8}/agent-{n}")
}

/// Normalize + validate a vault-relative target dir: strip surrounding
/// slashes/whitespace, reject traversal, backslashes, and hidden segments.
/// `""` = the vault root.
fn normalize_target_dir(raw: &str) -> Result<String, ApiError> {
    let t = raw.trim().trim_matches('/').to_string();
    if t.is_empty() {
        return Ok(t);
    }
    if t.contains('\\')
        || t.split('/')
            .any(|seg| seg.is_empty() || seg == "." || seg == ".." || seg.starts_with('.'))
    {
        return Err(ApiError(Error::Invalid(format!(
            "invalid target_dir: {raw}"
        ))));
    }
    Ok(t)
}

/// Human phrasing of the target dir for prompts.
fn dir_display(target_dir: &str) -> String {
    if target_dir.is_empty() {
        "the vault root (no folder prefix)".to_string()
    } else {
        format!("`{target_dir}/`")
    }
}

/// Take the first `cap` CHARS of `s` (never splits a char). Returns the
/// (possibly shortened) text + whether it was truncated.
fn cap_chars(s: &str, cap: usize) -> (String, bool) {
    let mut it = s.char_indices();
    match it.nth(cap) {
        Some((byte, _)) => (s[..byte].to_string(), true),
        None => (s.to_string(), false),
    }
}

/// The shared OKF requirements block (present iff the vault is OKF). For
/// non-claude providers `inline` carries the full okf-authoring SKILL.md text;
/// claude instead has the skill staged first-class via extra_dirs.
fn okf_block(okf: bool, inline: Option<&str>) -> String {
    if !okf {
        return String::new();
    }
    let mut b = String::from(
        "\nOKF — this vault is OKF-conformant and EVERY note you write MUST comply:\n\
         - YAML frontmatter on every note with at least `type:` (concept/service/endpoint/\
         dataset/decision/runbook/metric/…) and a one-sentence `description:`.\n\
         - Internal references use standard markdown links (relative, `.md`).\n\
         - Keep each folder's `index.md` listing its children up to date.\n",
    );
    match inline {
        Some(text) => {
            b.push_str("\n--- OKF AUTHORING SKILL (follow this method exactly) ---\n");
            b.push_str(text);
            b.push_str("\n--- END OKF AUTHORING SKILL ---\n");
        }
        None => {
            b.push_str(
                "You have the `okf-authoring` skill available — invoke it before writing.\n",
            );
        }
    }
    b
}

/// Build one writer's prompt. Multi-writer (`m > 1`): drafts ONLY under its own
/// `_drafts/docs-run-<run8>/agent-<n>/` folder. Single writer: finals straight
/// into `target_dir` + the results JSON file (`results_path` is `Some`).
#[allow(clippy::too_many_arguments)]
fn build_writer_prompt(
    user_prompt: &str,
    vault_id: i64,
    n: usize,
    m: usize,
    run8: &str,
    target_dir: &str,
    okf: bool,
    okf_inline: Option<&str>,
    results_path: Option<&str>,
) -> String {
    let mut p = String::from("OTTO_TASK: vault_docs_write\n");
    if m > 1 {
        let dir = draft_dir(run8, n);
        p.push_str(&format!(
            "You are WRITER AGENT {n} of {m} documenting the request below into Otto vault \
             {vault_id}. Peers cover the same request independently; a summarizer consolidates \
             all drafts afterwards.\n\n\
             RULES — follow exactly:\n\
             - Write DRAFT notes ONLY under `{dir}/` (vault-relative). NO edits anywhere else \
             in the vault.\n\
             - Write every note via the otto MCP tool `otto_vault_write` with vault_id \
             {vault_id} and a path like `{dir}/<topic>.md` (parent folders are auto-created).\n\
             - Cover the request thoroughly; split into multiple focused notes where that \
             helps, cross-linked with standard markdown links.\n\
             - Read existing notes first (`otto_vault_list` / `otto_vault_read`) when the \
             request builds on them.\n"
        ));
    } else {
        p.push_str(&format!(
            "You are the WRITER AGENT producing FINAL documentation for the request below \
             into Otto vault {vault_id}.\n\n\
             RULES — follow exactly:\n\
             - Write the final notes directly under {dd} via the otto MCP tool \
             `otto_vault_write` with vault_id {vault_id} (parent folders are auto-created).\n\
             - Cover the request thoroughly; split into multiple focused notes where that \
             helps, cross-linked with standard markdown links.\n\
             - Read existing notes first (`otto_vault_list` / `otto_vault_read`) when the \
             request builds on them.\n",
            dd = dir_display(target_dir),
        ));
        if okf {
            p.push_str(&format!(
                "- Update (or create) {dd}'s `index.md` to reference the notes you wrote.\n",
                dd = dir_display(target_dir),
            ));
        }
    }
    p.push_str(&okf_block(okf, okf_inline));
    if let Some(rp) = results_path {
        p.push_str(&format!(
            "\nFINALLY, write a results file to this exact filesystem path: `{rp}` \
             containing ONLY this JSON (every vault-relative note path you wrote): \
             {{\"written\": [\"path/to/note.md\", ...]}}\n"
        ));
    }
    p.push_str("\nWhen done, reply with a ONE-LINE summary of what you wrote.\n");
    p.push_str(&format!("\nRequest:\n{user_prompt}\n"));
    p
}

/// Build the summarizer prompt: every draft inlined (path + content) under a
/// total cap; drafts past the cap are listed by path only, and the cap is
/// stated so the agent knows to `otto_vault_read` the rest.
#[allow(clippy::too_many_arguments)]
fn build_summarizer_prompt(
    user_prompt: &str,
    vault_id: i64,
    m: usize,
    target_dir: &str,
    drafts: &[(String, String)],
    okf: bool,
    okf_inline: Option<&str>,
    results_path: &str,
) -> String {
    let dd = dir_display(target_dir);
    let mut p = format!(
        "OTTO_TASK: vault_docs_summarize\n\
         You are the SUMMARIZER. {m} writer agents drafted documentation for the SAME request \
         in Otto vault {vault_id}; their drafts are inlined below (total inlined content is \
         capped at {DRAFT_INLINE_CAP} chars — a draft marked [truncated] or [not inlined] has \
         its full content on disk, read it with `otto_vault_read`).\n\n\
         CONSOLIDATE — follow exactly:\n\
         - Produce ONE coherent, deduplicated set of FINAL notes under {dd} via \
         `otto_vault_write` (vault_id {vault_id}).\n\
         - Prefer the best content per topic; merge overlaps; fix contradictions. Do NOT \
         copy drafts in verbatim as duplicates.\n\
         - Do NOT write anything under `_drafts/` — finals only.\n"
    );
    if okf {
        p.push_str(&format!(
            "- OKF vault: give every final note conformant frontmatter, refresh {dd}'s \
             `index.md`, then run `otto_vault_okf_validate` (vault_id {vault_id}) and fix \
             EVERY error it reports.\n",
        ));
    }
    p.push_str(&okf_block(okf, okf_inline));
    p.push_str(&format!(
        "\nFINALLY, write a results file to this exact filesystem path: `{results_path}` \
         containing ONLY this JSON (every vault-relative note path you wrote): \
         {{\"written\": [\"path/to/note.md\", ...]}}\n\
         Then reply with a ONE-LINE summary.\n"
    ));
    p.push_str(&format!(
        "\nRequest the writers documented:\n{user_prompt}\n"
    ));

    p.push_str("\nDRAFTS:\n");
    let mut budget = DRAFT_INLINE_CAP;
    for (path, content) in drafts {
        p.push_str(&format!("\n### {path}\n"));
        if budget == 0 {
            p.push_str("[not inlined — read via otto_vault_read]\n");
            continue;
        }
        let (chunk, truncated) = cap_chars(content, budget);
        budget -= chunk.chars().count();
        p.push_str(&chunk);
        if truncated {
            p.push_str("\n[truncated]\n");
        } else {
            p.push('\n');
        }
    }
    p
}

/// Build a refine-turn prompt. First turn (`content` = `Some`): full contract
/// with the current note inlined (capped). Later turns: just the instruction +
/// the CURRENT hash (the resumed session already holds the context).
#[allow(clippy::too_many_arguments)]
fn build_refine_prompt(
    user_prompt: &str,
    vault_id: i64,
    path: &str,
    hash: &str,
    content: Option<&str>,
    okf: bool,
    okf_inline: Option<&str>,
) -> String {
    let Some(raw) = content else {
        return format!(
            "OTTO_TASK: vault_docs_refine\n\
             Apply this instruction to the SAME note `{path}` in Otto vault {vault_id}. Its \
             current content hash is now `{hash}` — pass it as if_hash when you write \
             (re-read + re-apply on a conflict).\n\n\
             Instruction:\n{user_prompt}\n\n\
             Reply with ONE line describing the change.\n"
        );
    };
    let (inlined, truncated) = cap_chars(raw, REFINE_INLINE_CAP);
    let mut p = format!(
        "OTTO_TASK: vault_docs_refine\n\
         You are refining the note `{path}` in Otto vault {vault_id}. Its current content \
         hash is `{hash}`; the current content is inlined below (capped at \
         {REFINE_INLINE_CAP} chars{trunc} — the full note is on disk via `otto_vault_read`).\n\n\
         RULES — follow exactly:\n\
         - Apply the instruction by writing the FULL revised note via `otto_vault_write` \
         (vault_id {vault_id}, path `{path}`, if_hash `{hash}`).\n\
         - On a hash conflict, re-read with `otto_vault_read` and re-apply.\n",
        trunc = if truncated { "; TRUNCATED here" } else { "" },
    );
    if okf {
        p.push_str(
            "- OKF vault: keep the note conformant. AUGMENT, don't rewrite — existing \
             headings survive unless the instruction says otherwise.\n",
        );
    }
    p.push_str(&okf_block(okf, okf_inline));
    p.push_str(&format!(
        "\nCURRENT CONTENT:\n{inlined}\n\nInstruction:\n{user_prompt}\n\n\
         Reply with ONE line describing the change.\n"
    ));
    p
}

/// Parse the agent-written results file TOLERANTLY: accept a bare JSON array,
/// a `{"written": [...]}` object (direct or buried in prose via the shared
/// balanced-object extractor), normalize paths (strip `./` and leading `/`),
/// drop empties, dedupe preserving order. `None` = nothing parseable.
fn parse_results(content: &str) -> Option<Vec<String>> {
    let v: Value = serde_json::from_str(content.trim())
        .ok()
        .or_else(|| crate::product_run::extract_json_block(content))?;
    let arr = match &v {
        Value::Array(a) => a.clone(),
        Value::Object(_) => v.get("written")?.as_array()?.clone(),
        _ => return None,
    };
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for item in arr {
        if let Some(s) = item.as_str() {
            let p = s.trim().trim_start_matches("./").trim_start_matches('/');
            if !p.is_empty() && seen.insert(p.to_string()) {
                out.push(p.to_string());
            }
        }
    }
    Some(out)
}

/// Fallback `written`: notes present after but not before, under `target_dir`
/// (`""` = anywhere), never counting leftover drafts. Sorted for determinism.
fn written_fallback(
    before: &HashSet<String>,
    after: &HashSet<String>,
    target_dir: &str,
) -> Vec<String> {
    let prefix = if target_dir.is_empty() {
        String::new()
    } else {
        format!("{target_dir}/")
    };
    let mut out: Vec<String> = after
        .difference(before)
        .filter(|p| p.starts_with(&prefix) && !p.starts_with("_drafts/"))
        .cloned()
        .collect();
    out.sort();
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const OKF_SKILL: &str = "# OKF — Open Knowledge Format authoring\nfrontmatter rules…";

    #[test]
    fn multi_writer_prompt_confines_to_its_draft_dir() {
        let p = build_writer_prompt(
            "document the auth flow",
            7,
            2,
            3,
            "abc12345",
            "docs",
            false,
            None,
            None,
        );
        assert!(p.contains("OTTO_TASK: vault_docs_write"));
        assert!(p.contains("WRITER AGENT 2 of 3"));
        assert!(p.contains("`_drafts/docs-run-abc12345/agent-2/`"));
        assert!(p.contains("otto_vault_write"));
        assert!(
            p.contains("vault_id 7")
                || p.contains("vault \n7")
                || p.contains("{7}")
                || p.contains("7")
        );
        // Multi-writer drafts: no results file, no target-dir instruction.
        assert!(!p.contains("results file"));
        assert!(!p.contains("`docs/`"));
        // Not an OKF vault → no OKF block.
        assert!(!p.contains("OKF"));
        assert!(p.contains("document the auth flow"));
    }

    #[test]
    fn single_writer_prompt_targets_finals_and_results_file() {
        let p = build_writer_prompt(
            "document the auth flow",
            7,
            1,
            1,
            "abc12345",
            "docs/auth",
            true,
            None,
            Some("/tmp/otto-vaultdocs-r1.json"),
        );
        assert!(p.contains("FINAL documentation"));
        assert!(p.contains("`docs/auth/`"));
        assert!(p.contains("/tmp/otto-vaultdocs-r1.json"));
        assert!(p.contains("\"written\""));
        assert!(!p.contains("_drafts/")); // single writer never drafts
                                          // OKF vault: block present; claude-style (no inline text) mentions the skill.
        assert!(p.contains("OKF — this vault is OKF-conformant"));
        assert!(p.contains("okf-authoring"));
        assert!(p.contains("index.md"));
    }

    #[test]
    fn okf_block_inlines_skill_for_non_claude() {
        let p = build_writer_prompt("x", 1, 1, 2, "run8run8", "", true, Some(OKF_SKILL), None);
        assert!(p.contains("OKF AUTHORING SKILL"));
        assert!(p.contains(OKF_SKILL));
        // And the claude variant carries the skill NAME, not the body.
        let c = build_writer_prompt("x", 1, 1, 2, "run8run8", "", true, None, None);
        assert!(c.contains("okf-authoring"));
        assert!(!c.contains(OKF_SKILL));
    }

    #[test]
    fn summarizer_prompt_inlines_drafts_under_the_cap() {
        let drafts = vec![
            (
                "_drafts/docs-run-r/agent-1/a.md".to_string(),
                "short draft A".to_string(),
            ),
            (
                "_drafts/docs-run-r/agent-2/b.md".to_string(),
                "b".repeat(DRAFT_INLINE_CAP),
            ),
            (
                "_drafts/docs-run-r/agent-2/c.md".to_string(),
                "unreachable".to_string(),
            ),
        ];
        let p = build_summarizer_prompt(
            "req",
            3,
            2,
            "docs",
            &drafts,
            true,
            Some(OKF_SKILL),
            "/tmp/r.json",
        );
        assert!(p.contains("OTTO_TASK: vault_docs_summarize"));
        assert!(p.contains("### _drafts/docs-run-r/agent-1/a.md"));
        assert!(p.contains("short draft A"));
        // The huge second draft exhausts the budget → truncated marker…
        assert!(p.contains("[truncated]"));
        // …and the third is listed by path only.
        assert!(p.contains("### _drafts/docs-run-r/agent-2/c.md"));
        assert!(p.contains("[not inlined — read via otto_vault_read]"));
        assert!(!p.contains("unreachable"));
        // OKF: validate + index refresh + inlined skill; results file; target dir.
        assert!(p.contains("otto_vault_okf_validate"));
        assert!(p.contains(OKF_SKILL));
        assert!(p.contains("/tmp/r.json"));
        assert!(p.contains("`docs/`"));
        // The stated cap matches the enforced one.
        assert!(p.contains(&DRAFT_INLINE_CAP.to_string()));
    }

    #[test]
    fn refine_prompt_first_turn_inlines_capped_content() {
        let long = "x".repeat(REFINE_INLINE_CAP + 10);
        let p = build_refine_prompt(
            "tighten the intro",
            5,
            "docs/a.md",
            "hash123",
            Some(&long),
            true,
            None,
        );
        assert!(p.contains("OTTO_TASK: vault_docs_refine"));
        assert!(p.contains("if_hash `hash123`"));
        assert!(p.contains("TRUNCATED"));
        assert!(p.contains("AUGMENT, don't rewrite"));
        // Enforced: inlined content really is capped (prompt shorter than raw note).
        assert!(p.len() < long.len() + 3_000);
        // Follow-up turn: no content, just instruction + fresh hash.
        let f = build_refine_prompt("more", 5, "docs/a.md", "hash456", None, true, None);
        assert!(f.contains("hash456"));
        assert!(f.contains("SAME note"));
        assert!(!f.contains("CURRENT CONTENT"));
    }

    #[test]
    fn cap_chars_is_char_boundary_safe() {
        let (s, t) = cap_chars("héllo wörld", 4);
        assert_eq!(s, "héll");
        assert!(t);
        let (s, t) = cap_chars("short", 100);
        assert_eq!(s, "short");
        assert!(!t);
    }

    #[test]
    fn parse_results_is_tolerant() {
        // Canonical object.
        assert_eq!(
            parse_results(r#"{"written": ["a.md", "b/c.md"]}"#).unwrap(),
            vec!["a.md", "b/c.md"]
        );
        // Bare array.
        assert_eq!(parse_results(r#"["a.md"]"#).unwrap(), vec!["a.md"]);
        // Buried in prose + fenced.
        let prose = "Done!\n```json\n{\"written\": [\"./x.md\", \"/y.md\", \"\", \"x.md\"]}\n```";
        // Normalized (./ and / stripped), empties dropped, deduped in order.
        assert_eq!(parse_results(prose).unwrap(), vec!["x.md", "y.md"]);
        // Garbage / wrong shapes → None.
        assert!(parse_results("no json here").is_none());
        assert!(parse_results(r#"{"other": 1}"#).is_none());
        assert!(parse_results("").is_none());
    }

    #[test]
    fn written_fallback_diffs_under_target_dir_only() {
        let before: HashSet<String> = ["docs/old.md".to_string()].into();
        let after: HashSet<String> = [
            "docs/old.md".to_string(),
            "docs/new-b.md".to_string(),
            "docs/new-a.md".to_string(),
            "elsewhere/x.md".to_string(),
            "_drafts/docs-run-r/agent-1/d.md".to_string(),
        ]
        .into();
        // Scoped to target_dir, sorted, drafts never counted.
        assert_eq!(
            written_fallback(&before, &after, "docs"),
            vec!["docs/new-a.md", "docs/new-b.md"]
        );
        // Root target: everything new except drafts.
        assert_eq!(
            written_fallback(&before, &after, ""),
            vec!["docs/new-a.md", "docs/new-b.md", "elsewhere/x.md"]
        );
    }

    #[test]
    fn target_dir_normalizes_and_rejects_traversal() {
        assert_eq!(normalize_target_dir("").unwrap(), "");
        assert_eq!(normalize_target_dir(" /docs/api/ ").unwrap(), "docs/api");
        assert!(normalize_target_dir("../up").is_err());
        assert!(normalize_target_dir("a/../b").is_err());
        assert!(normalize_target_dir(".trash/x").is_err());
        assert!(normalize_target_dir("a\\b").is_err());
    }

    #[test]
    fn inline_is_withheld_from_claude_only() {
        assert_eq!(inline_for_provider("claude", Some("text")), None);
        assert_eq!(inline_for_provider("codex", Some("text")), Some("text"));
        assert_eq!(inline_for_provider("agy", Some("text")), Some("text"));
        assert_eq!(inline_for_provider("codex", None), None);
    }

    #[test]
    fn run_state_transitions_respect_terminal_states() {
        let reg = new_run_registry();
        let run = VaultDocsRun {
            id: "r1".into(),
            ws_id: "w1".into(),
            vault_id: 1,
            prompt: "p".into(),
            target_dir: String::new(),
            state: "running".into(),
            agents: vec![],
            summarizer: VaultDocsSummarizer {
                provider: "claude".into(),
                model: None,
                state: "pending".into(),
                session_id: None,
                error: None,
            },
            written: vec![],
            error: None,
            started_at: "t".into(),
            finished_at: None,
        };
        reg.lock().unwrap().insert(
            "r1".into(),
            RunEntry {
                run,
                cancel: Arc::new(AtomicBool::new(false)),
            },
        );
        // Cancel lands first…
        finish_run(&reg, "r1", "cancelled", None);
        // …and a late "done" from the orchestrator cannot overwrite it.
        finish_run(&reg, "r1", "done", None);
        let snap = reg.lock().unwrap().get("r1").unwrap().run.clone();
        assert_eq!(snap.state, "cancelled");
        assert!(snap.finished_at.is_some());
    }
}
