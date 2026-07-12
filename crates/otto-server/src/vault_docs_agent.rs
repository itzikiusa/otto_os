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
//! Run state lives in the in-memory registry for LIVE orchestration and is
//! write-through mirrored to SQLite (`vault_docs_runs`, [`persist`]) at every
//! transition — runs therefore survive a daemon restart as HISTORY. Orchestration
//! itself does not survive: any row still non-terminal at startup was killed by
//! the restart, and [`recover_interrupted`] flips it (and its non-terminal
//! agents) to `interrupted` + soft-trashes the run's orphaned `_drafts` dir.
//! Cancel stops orchestrating between stages but never kills the agent sessions.
//! The sessions are BACKGROUND (`meta.source = "vault-docs"` is in
//! `BACKGROUND_SESSION_SOURCES`): embedded in the Vault view's run panel, never
//! listed in the sidebar Agents group.
//!
//! OKF vaults (`vault.okf`): notes MUST be OKF-conformant. claude agents get
//! complete staged skill packages: Claude loads the native `.claude/skills`
//! view through `meta.extra_dirs`; Codex/agy/custom providers read the same
//! package's provider-neutral `skills/<name>/` tree by explicit prompt path.
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
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VaultDocsAgent {
    pub index: usize,
    /// Display name, e.g. `"writer-1 · claude"`.
    pub name: String,
    pub provider: String,
    pub model: Option<String>,
    /// `pending | running | done | error | cancelled`.
    pub state: String,
    pub session_id: Option<String>,
    pub error: Option<String>,
    /// Vault-relative draft note paths this agent produced (multi-writer only).
    pub drafts: Vec<String>,
}

/// The consolidation agent's live view (skipped entirely for a single writer).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VaultDocsSummarizer {
    pub provider: String,
    pub model: Option<String>,
    /// `pending | running | done | error | skipped | interrupted | cancelled`.
    pub state: String,
    pub session_id: Option<String>,
    pub error: Option<String>,
}

/// One evidence location proving a review finding against either source code
/// or the generated bundle. At least one complete location is required for
/// every parsed finding; reviewers cannot submit unsupported prose.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VaultDocsFindingEvidence {
    #[serde(default)]
    pub repo_path: Option<String>,
    #[serde(default)]
    pub line: Option<u32>,
    #[serde(default)]
    pub doc_path: Option<String>,
    #[serde(default)]
    pub section: Option<String>,
}

/// A source-backed omission or defect reported by a review agent.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VaultDocsFinding {
    /// `blocking | major | minor`.
    pub severity: String,
    pub category: String,
    pub summary: String,
    pub evidence: Vec<VaultDocsFindingEvidence>,
    pub missed_item: String,
    pub required_fix: String,
}

/// One reviewer's resolved configuration and live/result state.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VaultDocsReviewer {
    pub index: usize,
    pub provider: String,
    pub model: Option<String>,
    pub skill: String,
    pub focus: Option<String>,
    /// `pending | running | done | error | cancelled | interrupted`.
    pub state: String,
    pub session_id: Option<String>,
    #[serde(default)]
    pub findings: Vec<VaultDocsFinding>,
    pub error: Option<String>,
}

/// The final author's repair turn after a round that found omissions.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VaultDocsRevision {
    /// `skipped | pending | running | done | error | cancelled | interrupted`.
    pub state: String,
    pub session_id: Option<String>,
    #[serde(default)]
    pub changed_paths: Vec<String>,
    pub error: Option<String>,
}

impl Default for VaultDocsRevision {
    fn default() -> Self {
        Self {
            state: "skipped".into(),
            session_id: None,
            changed_paths: Vec::new(),
            error: None,
        }
    }
}

/// One independently visible review/revision iteration.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VaultDocsReviewRound {
    pub iteration: u8,
    /// `reviewing | revising | clean | exhausted | error | cancelled |
    /// interrupted`.
    pub state: String,
    pub reviewers: Vec<VaultDocsReviewer>,
    #[serde(default)]
    pub revision: VaultDocsRevision,
}

/// Optional iterative quality gate persisted inside every docs-run payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VaultDocsReview {
    /// `skipped | pending | reviewing | revising | clean | exhausted | error |
    /// cancelled | interrupted`.
    pub state: String,
    pub max_iterations: u8,
    pub current_iteration: u8,
    pub outcome: Option<String>,
    #[serde(default)]
    pub reviewers: Vec<VaultDocsReviewer>,
    #[serde(default)]
    pub rounds: Vec<VaultDocsReviewRound>,
}

impl Default for VaultDocsReview {
    fn default() -> Self {
        Self {
            state: "skipped".into(),
            max_iterations: default_review_iterations(),
            current_iteration: 0,
            outcome: None,
            reviewers: Vec::new(),
            rounds: Vec::new(),
        }
    }
}

/// A whole docs run — the poll snapshot the UI renders every 1500ms, AND the
/// durable `payload` persisted per transition in `vault_docs_runs` (hence
/// `Deserialize` + defaults: rows written before a field existed must load).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VaultDocsRun {
    pub id: String,
    pub ws_id: String,
    pub vault_id: i64,
    /// `docs` (writer fan-out) | `refine` (one per-note edit turn).
    #[serde(default = "default_run_kind")]
    pub kind: String,
    pub prompt: String,
    pub target_dir: String,
    /// Refine turns: the note being edited (`""` for docs runs).
    #[serde(default)]
    pub note_path: String,
    /// `running | summarizing | reviewing | revising | done |
    /// done_with_findings | error | cancelled | interrupted`.
    pub state: String,
    pub agents: Vec<VaultDocsAgent>,
    pub summarizer: VaultDocsSummarizer,
    /// Defaulted so payloads persisted before iterative review deserialize as
    /// an explicitly skipped quality gate.
    #[serde(default)]
    pub review: VaultDocsReview,
    /// Final note paths in the vault (agent-reported, else server-side diff).
    pub written: Vec<String>,
    pub error: Option<String>,
    pub started_at: String,
    pub finished_at: Option<String>,
}

fn default_run_kind() -> String {
    "docs".into()
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
    /// Extra library skills available to every writer + the summarizer as
    /// complete staged packages. Prepared prompts pass e.g. `vault-repo-docs`.
    /// Capped; names validated.
    #[serde(default)]
    pub skills: Vec<String>,
    /// Optional independent reviewer loop. Omitted means review is skipped.
    #[serde(default)]
    pub review: Option<ReviewReq>,
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

#[derive(Clone, Debug, Deserialize)]
pub struct ReviewReq {
    /// Required and 1..=4 when the review block is present.
    pub reviewers: Vec<ReviewerReq>,
    #[serde(default = "default_review_iterations")]
    pub max_iterations: u8,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ReviewerReq {
    pub provider: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default = "default_reviewer_skill")]
    pub skill: String,
    #[serde(default)]
    pub focus: Option<String>,
}

fn default_review_iterations() -> u8 {
    3
}

fn default_reviewer_skill() -> String {
    "vault-docs-review".into()
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
// The run registry holds LIVE runs only — every transition is mirrored to the
// durable `vault_docs_runs` table ([`persist`]) and terminal runs are evicted
// after their final awaited upsert ([`finalize_run`]), so `get_run`/`list_runs`
// serve history (and post-restart polls) from the DB.
// ---------------------------------------------------------------------------

/// One live run: its poll snapshot + the cancel flag the orchestrator checks
/// between stages (mirrors the `product_run::CancelRegistry` shape).
pub struct RunEntry {
    pub run: VaultDocsRun,
    pub cancel: Arc<AtomicBool>,
    /// User-requested retries by writer index (`SUM_RETRY_IDX` = summarizer).
    /// The retry endpoint kills the target's session and inserts here; the
    /// orchestrator's turn loop consumes one entry per re-spawn.
    pub retries: Arc<Mutex<HashSet<usize>>>,
    /// Signal into the run's write-behind persister (`None` in unit tests,
    /// which have no runtime).
    pub persist_tx: Option<PersistTx>,
}

/// `retries` index standing for the summarizer (writers use their own index).
pub const SUM_RETRY_IDX: usize = usize::MAX;

/// Signal channel into a run's write-behind persister: `None` = "snapshot +
/// upsert soon" (coalesced), `Some(ack)` = "flush now and ack when durable".
pub type PersistTx = tokio::sync::mpsc::UnboundedSender<Option<tokio::sync::oneshot::Sender<()>>>;

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
    matches!(
        state,
        "done" | "done_with_findings" | "error" | "cancelled" | "interrupted"
    )
}

/// Flatten a run into its durable row (payload = the full JSON snapshot).
fn run_row(run: &VaultDocsRun) -> otto_state::VaultDocsRunRow {
    otto_state::VaultDocsRunRow {
        id: run.id.clone(),
        vault_id: run.vault_id,
        ws_id: run.ws_id.clone(),
        kind: run.kind.clone(),
        state: run.state.clone(),
        prompt: run.prompt.clone(),
        target_dir: run.target_dir.clone(),
        note_path: run.note_path.clone(),
        payload: serde_json::to_string(run).unwrap_or_else(|_| "{}".into()),
        started_at: run.started_at.clone(),
        finished_at: run.finished_at.clone(),
        updated_at: String::new(), // stamped by the repo on upsert
    }
}

/// Spawn a run's WRITE-BEHIND PERSISTER: one long-lived task that owns every
/// durable write for the run. Callers only signal it (sync, non-blocking);
/// signals coalesce to the latest snapshot and upserts run strictly
/// sequentially. This is deliberate: concurrent same-row upserts from
/// freshly-spawned tasks proved able to permanently lose an sqlx-sqlite
/// wakeup (future pending forever, no error, workers idle) — funneling every
/// write through one task removes that pattern entirely.
fn spawn_persister(
    repo: otto_state::VaultDocsRunsRepo,
    snapshot: impl Fn() -> Option<VaultDocsRun> + Send + 'static,
) -> PersistTx {
    let (tx, mut rx) =
        tokio::sync::mpsc::unbounded_channel::<Option<tokio::sync::oneshot::Sender<()>>>();
    tokio::spawn(async move {
        while let Some(first) = rx.recv().await {
            // Coalesce everything queued behind this signal; keep every ack.
            let mut acks: Vec<tokio::sync::oneshot::Sender<()>> = Vec::new();
            if let Some(a) = first {
                acks.push(a);
            }
            while let Ok(more) = rx.try_recv() {
                if let Some(a) = more {
                    acks.push(a);
                }
            }
            if let Some(run) = snapshot() {
                if let Err(e) = repo.upsert(&run_row(&run)).await {
                    warn!("vault_docs: persist run {}: {}", run.id, e);
                }
            }
            for a in acks {
                let _ = a.send(());
            }
        }
    });
    tx
}

/// Signal the run's persister (sync + non-blocking, so callable from
/// `on_ready` closures). No-op once the run is evicted or in unit tests.
fn persist(reg: &RunRegistry, run_id: &str) {
    let tx = reg
        .lock()
        .unwrap()
        .get(run_id)
        .and_then(|e| e.persist_tx.clone());
    if let Some(tx) = tx {
        let _ = tx.send(None);
    }
}

/// Flush a persister and wait (bounded) until its queued snapshot is durable.
async fn flush_persister(tx: &PersistTx) {
    let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
    if tx.send(Some(ack_tx)).is_err() {
        return;
    }
    let _ = tokio::time::timeout(std::time::Duration::from_secs(10), ack_rx).await;
}

/// Terminal cleanup: flush the persister (acked, bounded), write the final
/// snapshot directly (the persister is idle after the ack, so this write has
/// no concurrent twin), then evict the registry entry — the durable row is
/// now the single source of truth (`get_run`/`list_runs` fall back to it) and
/// the registry stays bounded for the daemon's life.
async fn finalize_run(reg: &RunRegistry, repo: &otto_state::VaultDocsRunsRepo, run_id: &str) {
    let (snap, tx) = {
        let reg = reg.lock().unwrap();
        let e = reg.get(run_id);
        (
            e.map(|e| e.run.clone()),
            e.and_then(|e| e.persist_tx.clone()),
        )
    };
    if let Some(tx) = tx {
        flush_persister(&tx).await;
    }
    if let Some(run) = snap {
        if let Err(e) = repo.upsert(&run_row(&run)).await {
            warn!("vault_docs: persist final run {}: {}", run.id, e);
            // Keep the entry — the DB row is stale/absent, memory is all we have.
            return;
        }
    }
    // Dropping the entry drops its sender → the persister task exits.
    reg.lock().unwrap().remove(run_id);
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
        .route(
            "/workspaces/{ws}/vault/vaults/{id}/docs-agents/runs",
            get(list_runs),
        )
        // NOT ws-scoped: the run id carries its ws, and the handlers re-check
        // the caller's role against it (policy row: /vault/docs-agents/*).
        .route("/vault/docs-agents/runs/{run_id}", get(get_run))
        .route("/vault/docs-agents/runs/{run_id}/cancel", post(cancel_run))
        .route(
            "/vault/docs-agents/runs/{run_id}/agents/{index}/retry",
            post(retry_agent),
        )
        .route(
            "/vault/docs-agents/runs/{run_id}/summarizer/retry",
            post(retry_summarizer),
        )
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
    if let Some(review) = req.review.as_ref() {
        validate_review_request(review).map_err(|message| ApiError(Error::Invalid(message)))?;
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
    let review = req
        .review
        .as_ref()
        .map(|requested| VaultDocsReview {
            state: "pending".into(),
            max_iterations: requested.max_iterations,
            current_iteration: 0,
            outcome: None,
            reviewers: requested
                .reviewers
                .iter()
                .enumerate()
                .map(|(index, reviewer)| VaultDocsReviewer {
                    index,
                    provider: resolve(&reviewer.provider),
                    model: reviewer
                        .model
                        .clone()
                        .filter(|model| !model.trim().is_empty()),
                    skill: reviewer.skill.trim().to_string(),
                    focus: reviewer
                        .focus
                        .clone()
                        .map(|focus| focus.trim().to_string())
                        .filter(|focus| !focus.is_empty()),
                    state: "pending".into(),
                    session_id: None,
                    findings: Vec::new(),
                    error: None,
                })
                .collect(),
            rounds: Vec::new(),
        })
        .unwrap_or_default();

    let run_id = otto_core::new_id().to_string();
    let multi = writers.len() > 1;
    let run = VaultDocsRun {
        id: run_id.clone(),
        ws_id: ws_id.clone(),
        vault_id,
        kind: "docs".into(),
        prompt: req.prompt.clone(),
        target_dir: target_dir.clone(),
        note_path: String::new(),
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
        review,
        written: Vec::new(),
        error: None,
        started_at: chrono::Utc::now().to_rfc3339(),
        finished_at: None,
    };
    let repo = otto_state::VaultDocsRunsRepo::new(ctx.pool.clone());
    let persist_tx = {
        let reg = ctx.vault_docs_runs.clone();
        let snap_id = run_id.clone();
        spawn_persister(repo.clone(), move || {
            reg.lock().unwrap().get(&snap_id).map(|e| e.run.clone())
        })
    };
    ctx.vault_docs_runs.lock().unwrap().insert(
        run_id.clone(),
        RunEntry {
            run: run.clone(),
            cancel: Arc::new(AtomicBool::new(false)),
            retries: Arc::new(Mutex::new(HashSet::new())),
            persist_tx: Some(persist_tx),
        },
    );
    // Awaited (not fire-and-forget): the row must exist before the response so
    // an immediate runs-list reload in another tab already sees this run.
    if let Err(e) = repo.upsert(&run_row(&run)).await {
        warn!("vault_docs: persist new run {run_id}: {e}");
    }

    // Sanitize requested skills: simple names only (the stager also guards),
    // deduped, at most 4 — a prepared prompt sends one or two.
    let skills: Vec<String> = {
        let mut seen = HashSet::new();
        req.skills
            .iter()
            .map(|s| s.trim().to_string())
            .filter(|s| {
                !s.is_empty()
                    && s.len() <= 64
                    && s.chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                    && seen.insert(s.clone())
            })
            .take(4)
            .collect()
    };

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
        skills,
    ));

    Ok(Json(run))
}

/// `GET /vault/docs-agents/runs/{run_id}` — the poll snapshot: the live
/// registry first, else the durable row (history / after a restart, where a
/// stale open poll now resolves to `interrupted` instead of a 404). The run's
/// ws is re-checked against the caller (the path itself is not ws-scoped).
async fn get_run(
    Path(run_id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<VaultDocsRun>> {
    let mem = ctx
        .vault_docs_runs
        .lock()
        .unwrap()
        .get(&run_id)
        .map(|e| e.run.clone());
    let run = match mem {
        Some(r) => r,
        None => otto_state::VaultDocsRunsRepo::new(ctx.pool.clone())
            .get(&run_id)
            .await
            .map_err(ApiError)?
            .and_then(|row| row_to_run_dto(&row))
            .ok_or_else(|| ApiError(Error::NotFound(format!("docs run {run_id}"))))?,
    };
    crate::auth::require_ws_role(
        &ctx,
        &user,
        &Id::from(run.ws_id.clone()),
        WorkspaceRole::Viewer,
    )
    .await?;
    Ok(Json(run))
}

/// Parse a durable row back into the DTO. The row's flat `state` wins over the
/// payload's (they only diverge if a payload ever failed to parse during the
/// startup sweep); an unparseable payload drops the row with a warning.
fn row_to_run_dto(row: &otto_state::VaultDocsRunRow) -> Option<VaultDocsRun> {
    match serde_json::from_str::<VaultDocsRun>(&row.payload) {
        Ok(mut run) => {
            run.state = row.state.clone();
            Some(run)
        }
        Err(e) => {
            warn!("vault_docs: unparseable payload for run {}: {}", row.id, e);
            None
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct ListRunsQ {
    limit: Option<i64>,
}

/// `GET /workspaces/{ws}/vault/vaults/{id}/docs-agents/runs?limit=` — the
/// vault's runs (docs + refine), newest-first: durable rows overlaid with the
/// fresher live-registry snapshot where one exists.
async fn list_runs(
    Path((ws_id, vault_id)): Path<(String, i64)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Query(q): Query<ListRunsQ>,
) -> ApiResult<Json<Vec<VaultDocsRun>>> {
    crate::auth::require_ws_role(&ctx, &user, &Id::from(ws_id.clone()), WorkspaceRole::Viewer)
        .await?;
    // Scope check only — rows are keyed by vault id.
    ctx.vault
        .get_scoped(&ws_id, vault_id)
        .await
        .map_err(ApiError)?;
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let rows = otto_state::VaultDocsRunsRepo::new(ctx.pool.clone())
        .list_for_vault(vault_id, limit)
        .await
        .map_err(ApiError)?;

    let mut runs: Vec<VaultDocsRun> = Vec::with_capacity(rows.len());
    {
        let reg = ctx.vault_docs_runs.lock().unwrap();
        let mut seen: HashSet<String> = HashSet::new();
        for row in &rows {
            seen.insert(row.id.clone());
            match reg.get(&row.id) {
                Some(e) => runs.push(e.run.clone()),
                None => {
                    if let Some(run) = row_to_run_dto(row) {
                        runs.push(run);
                    }
                }
            }
        }
        // A live run whose first upsert failed (DB hiccup) is registry-only —
        // still surface it rather than letting it vanish from the list.
        for e in reg.values() {
            if e.run.vault_id == vault_id && !seen.contains(&e.run.id) {
                runs.push(e.run.clone());
            }
        }
    }
    runs.sort_by(|a, b| b.started_at.cmp(&a.started_at).then(b.id.cmp(&a.id)));
    Ok(Json(runs))
}

/// `POST /vault/docs-agents/runs/{run_id}/cancel` — trip the flag, mark the
/// run cancelled NOW (the UI must not keep polling a "running" corpse while a
/// long writer turn drains), and KILL the writer/summarizer sessions. The
/// cancel flag alone only stops orchestration BETWEEN stages — a writer mid-
/// turn would keep producing drafts for minutes, and vault-docs sessions are
/// embedded-only (hidden from the sidebar), so this endpoint is the only kill
/// switch the user has.
async fn cancel_run(
    Path(run_id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<axum::http::StatusCode> {
    let (ws_id, cancel, session_ids) = {
        let reg = ctx.vault_docs_runs.lock().unwrap();
        let e = reg
            .get(&run_id)
            .ok_or_else(|| ApiError(Error::NotFound(format!("docs run {run_id}"))))?;
        let mut sids: Vec<Id> = e
            .run
            .agents
            .iter()
            .filter(|a| matches!(a.state.as_str(), "pending" | "running"))
            .filter_map(|a| a.session_id.clone())
            .collect();
        if matches!(e.run.summarizer.state.as_str(), "pending" | "running") {
            if let Some(s) = e.run.summarizer.session_id.clone() {
                sids.push(s);
            }
        }
        (e.run.ws_id.clone(), Arc::clone(&e.cancel), sids)
    };
    crate::auth::require_ws_role(&ctx, &user, &Id::from(ws_id), WorkspaceRole::Editor).await?;
    cancel.store(true, Ordering::Relaxed);
    // Reflect the cancel on every still-moving agent NOW — a cancelled run
    // must never keep showing RUNNING chips (finished agents keep done/error).
    with_run(&ctx.vault_docs_runs, &run_id, |r| {
        for a in &mut r.agents {
            if matches!(a.state.as_str(), "pending" | "running") {
                a.state = "cancelled".into();
            }
        }
        if matches!(r.summarizer.state.as_str(), "pending" | "running") {
            r.summarizer.state = "cancelled".into();
        }
    });
    finish_run(&ctx.vault_docs_runs, &run_id, "cancelled", None);
    persist(&ctx.vault_docs_runs, &run_id);
    // Best-effort, detached: the response must not wait on PTY teardown.
    // Ctrl+C first — TUIs that front a detached worker (grok's leader) abort
    // the in-flight request on interrupt, which a hard PTY kill alone might
    // orphan — then the kill after a short grace.
    for sid in session_ids {
        let manager = ctx.manager.clone();
        tokio::spawn(async move {
            let _ = manager.input(&sid, b"\x03").await;
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            let _ = manager.input(&sid, b"\x03").await;
            tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
            let _ = manager.kill_session(&sid).await;
        });
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// `POST /vault/docs-agents/runs/{run_id}/agents/{index}/retry` — LIVE-run
/// retry for one stuck/failed writer, exactly like a PR-review agent retry:
/// flag the slot, kill its current session (Ctrl+C grace, then kill) so the
/// in-flight turn errors out, and the orchestrator's turn loop re-spawns a
/// FRESH session with the same prompt. 409 once the run is terminal (history
/// runs re-run via a new run) or when the slot isn't still moving.
async fn retry_agent(
    Path((run_id, index)): Path<(String, usize)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<axum::http::StatusCode> {
    retry_target(ctx, user, run_id, index).await
}

/// `POST /vault/docs-agents/runs/{run_id}/summarizer/retry` — same, for the
/// consolidation stage.
async fn retry_summarizer(
    Path(run_id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<axum::http::StatusCode> {
    retry_target(ctx, user, run_id, SUM_RETRY_IDX).await
}

async fn retry_target(
    ctx: ServerCtx,
    user: User,
    run_id: String,
    index: usize,
) -> ApiResult<axum::http::StatusCode> {
    let (ws_id, sid, retries) = {
        let reg = ctx.vault_docs_runs.lock().unwrap();
        let e = reg
            .get(&run_id)
            .ok_or_else(|| ApiError(Error::NotFound(format!("docs run {run_id}"))))?;
        if is_terminal(&e.run.state) {
            return Err(ApiError(Error::Conflict(
                "run already finished — start a new run instead".into(),
            )));
        }
        let sid = if index == SUM_RETRY_IDX {
            if e.run.state != "summarizing"
                || !matches!(e.run.summarizer.state.as_str(), "running" | "pending")
            {
                return Err(ApiError(Error::Conflict(
                    "summarizer is not running".into(),
                )));
            }
            e.run.summarizer.session_id.clone()
        } else {
            let a = e
                .run
                .agents
                .get(index)
                .ok_or_else(|| ApiError(Error::NotFound(format!("writer {index}"))))?;
            if !matches!(a.state.as_str(), "running" | "pending") {
                return Err(ApiError(Error::Conflict(
                    "writer is not running — cancel and start a new run".into(),
                )));
            }
            a.session_id.clone()
        };
        (e.run.ws_id.clone(), sid, Arc::clone(&e.retries))
    };
    crate::auth::require_ws_role(&ctx, &user, &Id::from(ws_id), WorkspaceRole::Editor).await?;
    retries.lock().unwrap().insert(index);
    // Reflect immediately — the UI's poll shows "pending" while the fresh
    // session spins up.
    with_run(&ctx.vault_docs_runs, &run_id, |r| {
        if index == SUM_RETRY_IDX {
            r.summarizer.state = "pending".into();
            r.summarizer.error = None;
        } else if let Some(a) = r.agents.get_mut(index) {
            a.state = "pending".into();
            a.error = None;
        }
    });
    persist(&ctx.vault_docs_runs, &run_id);
    // Kill the current session so the in-flight turn errors and the loop
    // consumes the retry flag. Same interrupt-then-kill dance as cancel.
    if let Some(sid) = sid {
        let manager = ctx.manager.clone();
        let sid = Id::from(sid);
        tokio::spawn(async move {
            let _ = manager.input(&sid, b"\x03").await;
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            let _ = manager.input(&sid, b"\x03").await;
            tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
            let _ = manager.kill_session(&sid).await;
        });
    }
    Ok(axum::http::StatusCode::ACCEPTED)
}

/// The note's most recent persisted refine session, IF that session still
/// exists (retention may have pruned it — then a fresh first turn is correct).
async fn rehydrate_refine_session(ctx: &ServerCtx, vault_id: i64, path: &str) -> Option<Id> {
    let row = otto_state::VaultDocsRunsRepo::new(ctx.pool.clone())
        .latest_refine_for_note(vault_id, path)
        .await
        .ok()
        .flatten()?;
    let run: VaultDocsRun = serde_json::from_str(&row.payload).ok()?;
    let sid = Id::from(run.agents.first()?.session_id.clone()?);
    ctx.manager.get(&sid).await.is_ok().then_some(sid)
}

/// `POST /workspaces/{ws}/vault/vaults/{id}/docs-agents/refine` — one resumed
/// session per (vault, note); returns when the turn completes (LONG request).
/// The turn itself runs in a DETACHED task: a client disconnect (page reload)
/// drops this handler's future, but the turn still completes and finalizes
/// both the refine registry and the recorded turn row.
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
    // The registry is memory-only — on a miss (fresh daemon), rehydrate the
    // note's session from its newest persisted refine turn so "one session per
    // note" survives restarts.
    let mut existing = ctx
        .vault_docs_refine
        .lock()
        .unwrap()
        .get(&key)
        .and_then(|e| e.session_id.clone());
    if existing.is_none() {
        existing = rehydrate_refine_session(&ctx, vault_id, &req.path).await;
        if let Some(sid) = &existing {
            let mut reg = ctx.vault_docs_refine.lock().unwrap();
            reg.entry(key.clone()).or_default().session_id = Some(sid.clone());
        }
    }
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

    let refine_skill_names = vault
        .okf
        .then(|| vec!["okf-authoring".to_string()])
        .unwrap_or_default();
    let refine_bundle_dir = otto_context::materialize::default_context_root()
        .join("vault-docs-skills")
        .join(format!("refine-{}", otto_core::new_id()));
    let refine_bundle = (!refine_skill_names.is_empty())
        .then(|| {
            crate::modules::stage_skill_packages_at(
                &ctx.context_library,
                &refine_skill_names,
                &refine_bundle_dir,
            )
        })
        .flatten();
    let refine_fallback = skill_fallbacks(&ctx.context_library, &refine_skill_names);
    let skill_guidance = skill_package_guidance(
        &provider,
        refine_bundle.as_ref(),
        &refine_skill_names,
        &refine_fallback,
    );
    let prompt = build_refine_prompt(
        &req.prompt,
        vault_id,
        &req.path,
        &note.meta.hash,
        first_turn.then_some(note.raw.as_str()),
        vault.okf,
        skill_guidance.as_deref(),
    );
    let (done_path, done_line) = done_marker();
    let prompt = format!("{prompt}{done_line}");
    let mut meta = serde_json::json!({
        "source": "vault-docs",
        "vault_id": vault_id,
    });
    if let Some(m) = &req.model {
        meta["model"] = Value::String(m.clone());
    }
    if let Some(dirs) = crate::review_session::review_skills_extra_dirs(
        &provider,
        staged_package_root(refine_bundle.as_ref()),
    ) {
        meta["extra_dirs"] = dirs;
    }

    // Mark the turn in flight + surface the session id the MOMENT it exists,
    // so `GET refine-session` can attach the live shell while the turn runs.
    {
        let mut reg = ctx.vault_docs_refine.lock().unwrap();
        let e = reg.entry(key.clone()).or_default();
        e.running = true;
    }

    // Each refine turn is recorded as a `kind:"refine"` run (one agent entry)
    // so edits share the docs runs history. Not run-registry-held — the turn
    // is a single await; the durable row is the whole story. Shared behind a
    // mutex: on_ready, the finalizer, and any in-flight persist snapshot the
    // SAME object, so no persist can regress the row.
    let repo = otto_state::VaultDocsRunsRepo::new(ctx.pool.clone());
    let turn_run = VaultDocsRun {
        id: otto_core::new_id().to_string(),
        ws_id: ws_id.clone(),
        vault_id,
        kind: "refine".into(),
        prompt: req.prompt.clone(),
        target_dir: String::new(),
        note_path: req.path.clone(),
        state: "running".into(),
        agents: vec![VaultDocsAgent {
            index: 0,
            name: format!("refine \u{00b7} {provider}"),
            provider: provider.clone(),
            model: req.model.clone().filter(|m| !m.trim().is_empty()),
            state: "running".into(),
            session_id: existing.as_ref().map(|s| s.to_string()),
            error: None,
            drafts: Vec::new(),
        }],
        summarizer: VaultDocsSummarizer {
            provider: String::new(),
            model: None,
            state: "skipped".into(),
            session_id: None,
            error: None,
        },
        review: VaultDocsReview::default(),
        written: Vec::new(),
        error: None,
        started_at: chrono::Utc::now().to_rfc3339(),
        finished_at: None,
    };
    let turn_id = turn_run.id.clone();
    if let Err(e) = repo.upsert(&run_row(&turn_run)).await {
        warn!("vault_docs: persist refine turn {turn_id}: {e}");
    }
    let turn_run = Arc::new(Mutex::new(turn_run));
    // The turn's own write-behind persister (same single-writer discipline as
    // docs runs; see `spawn_persister`). Exits when the last sender drops.
    let persist_tx = {
        let snap = Arc::clone(&turn_run);
        spawn_persister(repo.clone(), move || Some(snap.lock().unwrap().clone()))
    };

    let ready_reg = ctx.vault_docs_refine.clone();
    let ready_key = key.clone();
    let ready_tx = persist_tx.clone();
    let ready_turn = Arc::clone(&turn_run);
    let on_ready = move |sid: &Id| {
        {
            let mut reg = ready_reg.lock().unwrap();
            let e = reg.entry(ready_key).or_default();
            e.session_id = Some(sid.clone());
        }
        // Durable ASAP (a restart mid-turn must still know the session).
        if let Some(a) = ready_turn.lock().unwrap().agents.get_mut(0) {
            a.session_id = Some(sid.to_string());
        }
        let _ = ready_tx.send(None);
    };

    let task = {
        let ctx = ctx.clone();
        let refine_reg = ctx.vault_docs_refine.clone();
        let key = key.clone();
        let repo = repo.clone();
        let turn_run = Arc::clone(&turn_run);
        let persist_tx = persist_tx.clone();
        let path = req.path.clone();
        let title = format!("Vault refine: {}", req.path);
        let root = vault.root_path.clone();
        let provider = provider.clone();
        tokio::spawn(async move {
            let turn = crate::agent_session::run_session_turn_with(
                &ctx,
                &ws,
                &user,
                existing.as_ref(),
                &title,
                &root,
                &provider,
                meta,
                &prompt,
                crate::agent_session::STUCK_IDLE,
                crate::agent_session::TurnOpts {
                    done_file: Some(done_path.clone()),
                    quiet_done: Some(QUIET_DONE),
                },
                on_ready,
            )
            .await;
            let _ = std::fs::remove_file(&done_path);

            // The turn is over either way — clear `running` + finish the
            // recorded turn (this task survives a client disconnect, so the
            // row can never be stranded `running` until the next restart).
            if let Some(e) = refine_reg.lock().unwrap().get_mut(&key) {
                e.running = false;
            }
            let final_row = {
                let mut run = turn_run.lock().unwrap();
                run.finished_at = Some(chrono::Utc::now().to_rfc3339());
                match &turn {
                    Ok((_reply, sid)) => {
                        run.state = "done".into();
                        run.written = vec![path.clone()];
                        if let Some(a) = run.agents.get_mut(0) {
                            a.state = "done".into();
                            a.session_id = Some(sid.to_string());
                        }
                    }
                    Err(e) => {
                        run.state = "error".into();
                        run.error = Some(e.0.to_string());
                        if let Some(a) = run.agents.get_mut(0) {
                            a.state = "error".into();
                            a.error = Some(e.0.to_string());
                        }
                    }
                }
                run_row(&run)
            };
            if let Ok((_reply, sid)) = &turn {
                if let Some(e) = refine_reg.lock().unwrap().get_mut(&key) {
                    e.session_id = Some(sid.clone());
                }
            }
            // Flush any queued on_ready write, then land the terminal row
            // directly — the persister is idle after the ack, so this write
            // has no concurrent twin.
            flush_persister(&persist_tx).await;
            if let Err(e) = repo.upsert(&final_row).await {
                warn!("vault_docs: persist refine turn {}: {}", final_row.id, e);
            }
            turn
        })
    };

    let (reply, sid) = task
        .await
        .map_err(|e| ApiError(Error::Internal(format!("refine turn panicked: {e}"))))??;
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
    let mut entry = ctx
        .vault_docs_refine
        .lock()
        .unwrap()
        .get(&(vault_id, q.path.clone()))
        .cloned()
        .unwrap_or_default();
    // Registry miss (fresh daemon): rehydrate the note's persisted session so
    // the drawer re-attaches the SAME conversation after a restart.
    if entry.session_id.is_none() && !entry.running {
        if let Some(sid) = rehydrate_refine_session(&ctx, vault_id, &q.path).await {
            let mut reg = ctx.vault_docs_refine.lock().unwrap();
            reg.entry((vault_id, q.path.clone()))
                .or_default()
                .session_id = Some(sid.clone());
            entry.session_id = Some(sid);
        }
    }
    Ok(Json(RefineSessionResp {
        session_id: entry.session_id.map(|s| s.to_string()),
        running: entry.running,
    }))
}

// ---------------------------------------------------------------------------
// Startup recovery — a restart kills the orchestrator, not the durable rows.
// ---------------------------------------------------------------------------

/// Flip every non-terminal state in a run to `interrupted` (run, agents,
/// summarizer) + stamp `finished_at`. No-op on already-terminal runs.
fn mark_run_interrupted(run: &mut VaultDocsRun) -> bool {
    if is_terminal(&run.state) {
        return false;
    }
    run.state = "interrupted".into();
    run.error
        .get_or_insert_with(|| "interrupted by a daemon restart".into());
    run.finished_at = Some(chrono::Utc::now().to_rfc3339());
    for a in &mut run.agents {
        if matches!(a.state.as_str(), "pending" | "running") {
            a.state = "interrupted".into();
        }
    }
    if matches!(run.summarizer.state.as_str(), "pending" | "running") {
        run.summarizer.state = "interrupted".into();
    }
    if matches!(
        run.review.state.as_str(),
        "pending" | "reviewing" | "revising"
    ) {
        run.review.state = "interrupted".into();
    }
    for reviewer in &mut run.review.reviewers {
        if matches!(reviewer.state.as_str(), "pending" | "running") {
            reviewer.state = "interrupted".into();
        }
    }
    for round in &mut run.review.rounds {
        if matches!(round.state.as_str(), "reviewing" | "revising") {
            round.state = "interrupted".into();
        }
        for reviewer in &mut round.reviewers {
            if matches!(reviewer.state.as_str(), "pending" | "running") {
                reviewer.state = "interrupted".into();
            }
        }
        if matches!(round.revision.state.as_str(), "pending" | "running") {
            round.revision.state = "interrupted".into();
        }
    }
    true
}

/// Soft-move an interrupted run's orphaned `_drafts/docs-run-<run8>` dir into
/// `.trash/` (suffix `-interrupted` on a name collision with an earlier trash
/// of the same run). Same never-destroy stance as the normal stage-3 cleanup.
/// Returns whether anything moved.
fn trash_orphan_drafts(root: &str, run_id: &str) -> bool {
    let run8: String = run_id.chars().take(8).collect();
    let src = std::path::Path::new(root)
        .join("_drafts")
        .join(format!("docs-run-{run8}"));
    if !src.is_dir() {
        return false;
    }
    let trash = std::path::Path::new(root).join(".trash");
    let _ = std::fs::create_dir_all(&trash);
    let mut dst = trash.join(format!("docs-run-{run8}"));
    if dst.exists() {
        dst = trash.join(format!("docs-run-{run8}-interrupted"));
    }
    match std::fs::rename(&src, &dst) {
        Ok(()) => true,
        Err(e) => {
            warn!("vault_docs: trash orphan drafts {}: {}", src.display(), e);
            false
        }
    }
}

/// Startup sweep (spawned once from `ottod` main): every run row still
/// non-terminal was killed by the restart — mark it `interrupted`, soft-trash
/// its orphaned `_drafts` dir (multi-writer docs runs), and rescan the touched
/// vaults so the tree stops showing the leftovers.
pub async fn recover_interrupted(ctx: ServerCtx) {
    let repo = otto_state::VaultDocsRunsRepo::new(ctx.pool.clone());
    let rows = match repo.list_unfinished().await {
        Ok(rows) => rows,
        Err(e) => {
            warn!("vault_docs: recover: list unfinished runs: {e}");
            return;
        }
    };
    if rows.is_empty() {
        return;
    }
    let mut marked = 0usize;
    let mut rescan: HashSet<i64> = HashSet::new();
    for mut row in rows {
        row.state = "interrupted".into();
        match serde_json::from_str::<VaultDocsRun>(&row.payload) {
            Ok(mut run) => {
                mark_run_interrupted(&mut run);
                row.payload = serde_json::to_string(&run).unwrap_or(row.payload);
                row.finished_at = run.finished_at.clone();
                if run.kind == "docs" && run.agents.len() > 1 {
                    // Vault may be gone/re-scoped — resolve tolerantly.
                    if let Ok(vault) = ctx.vault.get_scoped(&row.ws_id, row.vault_id).await {
                        if trash_orphan_drafts(&vault.root_path, &run.id) {
                            rescan.insert(row.vault_id);
                        }
                    }
                }
            }
            // Unparseable payload (should never happen — we wrote it): still
            // flip the flat state so the run stops counting as live.
            Err(e) => warn!("vault_docs: recover: bad payload for {}: {}", row.id, e),
        }
        if let Err(e) = repo.upsert(&row).await {
            warn!("vault_docs: recover: persist {}: {}", row.id, e);
        } else {
            marked += 1;
        }
    }
    for vault_id in &rescan {
        let _ = ctx.vault.scan(*vault_id).await;
    }
    tracing::info!(
        "vault_docs: marked {marked} interrupted run(s) from before restart \
         ({} vault(s) rescanned after drafts cleanup)",
        rescan.len()
    );
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
    skills: Vec<String>,
) {
    let reg = ctx.vault_docs_runs.clone();
    let (cancel, retries) = match reg.lock().unwrap().get(&run_id) {
        Some(e) => (Arc::clone(&e.cancel), Arc::clone(&e.retries)),
        None => return,
    };
    let repo = otto_state::VaultDocsRunsRepo::new(ctx.pool.clone());
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

    // Skill vehicles, resolved once: request-selected skills (prepared prompts
    // pass e.g. `vault-repo-docs`) + `okf-authoring` on OKF vaults. The complete
    // packages are staged per run: Claude loads the native view via extra_dirs;
    // every other provider reads the provider-neutral view by explicit path.
    let mut skill_names = skills;
    if vault.okf && !skill_names.iter().any(|s| s == "okf-authoring") {
        skill_names.push("okf-authoring".into());
    }
    let skill_bundle_dir = otto_context::materialize::default_context_root()
        .join("vault-docs-skills")
        .join(&run_id);
    let skill_bundle = (!skill_names.is_empty())
        .then(|| {
            crate::modules::stage_skill_packages_at(
                &ctx.context_library,
                &skill_names,
                &skill_bundle_dir,
            )
        })
        .flatten();
    let fallback_text = skill_fallbacks(&ctx.context_library, &skill_names);

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
        let skill_guidance = skill_package_guidance(
            &w.provider,
            skill_bundle.as_ref(),
            &skill_names,
            &fallback_text,
        );
        let prompt = build_writer_prompt(
            &prompt,
            vault.id,
            i + 1,
            m,
            &run8,
            &target_dir,
            vault.okf,
            skill_guidance.as_deref(),
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
        if let Some(dirs) = crate::review_session::review_skills_extra_dirs(
            &w.provider,
            staged_package_root(skill_bundle.as_ref()),
        ) {
            meta["extra_dirs"] = dirs;
        }
        let cancel = Arc::clone(&cancel);
        let retries = Arc::clone(&retries);
        set.spawn(async move {
            // Turn loop: normally one pass. A user retry kills the session and
            // flags the slot — the resulting Err consumes the flag and this
            // loop re-spawns a FRESH session with the same prompt (new done-
            // marker each attempt; the old one is embedded in the old prompt).
            let mut user_retries: u32 = 0;
            let ok = loop {
                let (done_path, done_line) = done_marker();
                let attempt_prompt = format!("{prompt}{done_line}");
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
                    // Durable ASAP: a restart mid-turn must still know the session.
                    persist(&ready_reg, &ready_run);
                };
                let res = crate::agent_session::run_session_turn_with(
                    &ctx,
                    &ws,
                    &user,
                    None,
                    &format!("Vault docs: writer {}/{}", i + 1, m),
                    &root,
                    &w.provider,
                    meta.clone(),
                    &attempt_prompt,
                    crate::agent_session::STUCK_IDLE,
                    crate::agent_session::TurnOpts {
                        done_file: Some(done_path.clone()),
                        quiet_done: Some(QUIET_DONE),
                    },
                    on_ready,
                )
                .await;
                let _ = std::fs::remove_file(&done_path);
                let want_retry = retries.lock().unwrap().remove(&i);
                match res {
                    Ok((_reply, sid)) => {
                        with_run(&reg, &run_id, |r| {
                            if let Some(a) = r.agents.get_mut(i) {
                                a.state = "done".into();
                                a.session_id = Some(sid.to_string());
                            }
                        });
                        break true;
                    }
                    Err(e) => {
                        // A cancel KILLS the session — the resulting "vanished"/
                        // "exited" error is expected, not a writer failure.
                        if cancel.load(Ordering::Relaxed) {
                            with_run(&reg, &run_id, |r| {
                                if let Some(a) = r.agents.get_mut(i) {
                                    a.state = "cancelled".into();
                                }
                            });
                            break false;
                        }
                        if want_retry && user_retries < MAX_USER_RETRIES {
                            user_retries += 1;
                            with_run(&reg, &run_id, |r| {
                                if let Some(a) = r.agents.get_mut(i) {
                                    a.state = "pending".into();
                                    a.error = None;
                                }
                            });
                            persist(&reg, &run_id);
                            continue;
                        }
                        warn!("vault_docs: writer {} failed: {}", i + 1, e.0);
                        with_run(&reg, &run_id, |r| {
                            if let Some(a) = r.agents.get_mut(i) {
                                a.state = "error".into();
                                a.error = Some(e.0.to_string());
                            }
                        });
                        break false;
                    }
                }
            };
            persist(&reg, &run_id);
            ok
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
        finalize_run(&reg, &repo, &run_id).await;
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
        finalize_run(&reg, &repo, &run_id).await;
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
        finalize_run(&reg, &repo, &run_id).await;
        let _ = std::fs::remove_file(&results_path);
        return;
    }
    if cancel.load(Ordering::Relaxed) {
        finish_run(&reg, &run_id, "cancelled", None);
        finalize_run(&reg, &repo, &run_id).await;
        let _ = std::fs::remove_file(&results_path);
        return;
    }

    with_run(&reg, &run_id, |r| {
        if !is_terminal(&r.state) {
            r.state = "summarizing".into();
        }
        r.summarizer.state = "running".into();
    });
    persist(&reg, &run_id);

    let sum_guidance = skill_package_guidance(
        &summarizer.provider,
        skill_bundle.as_ref(),
        &skill_names,
        &fallback_text,
    );
    let sum_prompt = build_summarizer_prompt(
        &prompt,
        vault.id,
        m,
        &target_dir,
        &drafts,
        vault.okf,
        sum_guidance.as_deref(),
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
    if let Some(dirs) = crate::review_session::review_skills_extra_dirs(
        &summarizer.provider,
        staged_package_root(skill_bundle.as_ref()),
    ) {
        sum_meta["extra_dirs"] = dirs;
    }
    // Same turn loop as the writers: a user retry (SUM_RETRY_IDX) kills the
    // session; the Err consumes the flag and a fresh summarizer re-spawns.
    let mut sum_user_retries: u32 = 0;
    let sum_err = loop {
        let (sum_done_path, sum_done_line) = done_marker();
        let attempt_prompt = format!("{sum_prompt}{sum_done_line}");
        let ready_reg = reg.clone();
        let ready_run = run_id.clone();
        let on_ready = move |sid: &Id| {
            let sid = sid.to_string();
            with_run(&ready_reg, &ready_run, |r| {
                r.summarizer.state = "running".into();
                r.summarizer.session_id = Some(sid);
            });
            persist(&ready_reg, &ready_run);
        };
        let sum_res = crate::agent_session::run_session_turn_with(
            &ctx,
            &ws,
            &user,
            None,
            "Vault docs: summarizer",
            &vault.root_path,
            &summarizer.provider,
            sum_meta.clone(),
            &attempt_prompt,
            crate::agent_session::STUCK_IDLE,
            crate::agent_session::TurnOpts {
                done_file: Some(sum_done_path.clone()),
                quiet_done: Some(QUIET_DONE),
            },
            on_ready,
        )
        .await;
        let _ = std::fs::remove_file(&sum_done_path);
        let want_retry = retries.lock().unwrap().remove(&SUM_RETRY_IDX);
        match sum_res {
            Ok((_reply, sid)) => {
                with_run(&reg, &run_id, |r| {
                    r.summarizer.state = "done".into();
                    r.summarizer.session_id = Some(sid.to_string());
                });
                break None;
            }
            Err(e) => {
                if cancel.load(Ordering::Relaxed) {
                    with_run(&reg, &run_id, |r| r.summarizer.state = "cancelled".into());
                    break Some(e.0.to_string());
                }
                if want_retry && sum_user_retries < MAX_USER_RETRIES {
                    sum_user_retries += 1;
                    with_run(&reg, &run_id, |r| {
                        r.summarizer.state = "pending".into();
                        r.summarizer.error = None;
                    });
                    persist(&reg, &run_id);
                    continue;
                }
                warn!("vault_docs: summarizer failed: {}", e.0);
                with_run(&reg, &run_id, |r| {
                    r.summarizer.state = "error".into();
                    r.summarizer.error = Some(e.0.to_string());
                });
                break Some(e.0.to_string());
            }
        }
    };
    persist(&reg, &run_id);

    // ---- Stage 3: soft-trash the drafts dir + resolve `written` -------------
    // std::fs::rename is atomic within a filesystem; a cross-device rename (or
    // a missing drafts dir, e.g. the E2E stub) fails and is deliberately
    // ignored — a leftover `_drafts/` is harmless and user-visible.
    // A FAILED summarizer keeps the drafts in place: they are the only copy of
    // the writers' work at that point, and the user can consolidate manually
    // or re-run from them.
    if sum_err.is_none() {
        let src = std::path::Path::new(&vault.root_path)
            .join("_drafts")
            .join(format!("docs-run-{run8}"));
        let trash = std::path::Path::new(&vault.root_path).join(".trash");
        let _ = std::fs::create_dir_all(&trash);
        let _ = std::fs::rename(&src, trash.join(format!("docs-run-{run8}")));
    }
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
    finalize_run(&reg, &repo, &run_id).await;
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

/// A skill's instructional text for inlining: Library / operator dirs first
/// (via the shared review resolver), else the compiled-in bundled copy in
/// `otto-skills`.
fn skill_inline_text(library: &otto_context::Library, name: &str) -> String {
    let t = crate::modules::resolve_skill_inline(library, name);
    if !t.is_empty() {
        return t;
    }
    otto_skills::bundled_body(name).unwrap_or_default()
}

fn skill_fallbacks(library: &otto_context::Library, names: &[String]) -> HashMap<String, String> {
    names
        .iter()
        .filter_map(|name| {
            let body = skill_inline_text(library, name);
            (!body.is_empty()).then(|| (name.clone(), body))
        })
        .collect()
}

fn staged_package_root(staged: Option<&crate::modules::StagedSkillPackages>) -> Option<&str> {
    staged.map(|packages| packages.root.as_str())
}

/// Provider-specific directions for a materialized multi-file skill bundle.
/// Claude receives first-class skills through `extra_dirs` and only needs an
/// invocation reminder. Other providers receive the neutral package root and
/// a deterministic file manifest, then load `SKILL.md` plus only the resources
/// it routes them to. If staging failed, preserve the old body-inline fallback.
fn skill_package_guidance(
    provider: &str,
    staged: Option<&crate::modules::StagedSkillPackages>,
    names: &[String],
    fallback_inline: &HashMap<String, String>,
) -> Option<String> {
    if names.is_empty() {
        return None;
    }
    let staged_names = names
        .iter()
        .filter(|name| staged.is_some_and(|packages| packages.files.contains_key(*name)))
        .collect::<Vec<_>>();
    let mut sections = Vec::new();

    if !staged_names.is_empty() {
        if provider == "claude" {
            sections.push(format!(
                "Skills staged for this task: {}. You must invoke each relevant skill before writing.",
                staged_names.iter().map(|name| name.as_str()).collect::<Vec<_>>().join(", ")
            ));
        } else if let Some(packages) = staged {
            let root = std::path::Path::new(&packages.root).join("skills");
            let files = staged_names
                .iter()
                .flat_map(|name| {
                    packages.files[*name]
                        .iter()
                        .map(move |path| format!("{name}/{path}"))
                })
                .collect::<Vec<_>>();
            sections.push(format!(
                "AUTHORING SKILL PACKAGES are available at `{}`. Before working, read each selected \
                 skill's `SKILL.md`, then read only the references/examples it directs you to; run its \
                 scripts when the workflow requires them. Package files:\n- {}",
                root.display(),
                files.join("\n- ")
            ));
        }
    }

    let missing_bodies = names
        .iter()
        .filter(|name| !staged.is_some_and(|packages| packages.files.contains_key(*name)))
        .filter_map(|name| fallback_inline.get(name))
        .collect::<Vec<_>>();
    if !missing_bodies.is_empty() {
        sections.push(
            missing_bodies
                .into_iter()
                .cloned()
                .collect::<Vec<_>>()
                .join("\n\n---\n\n"),
        );
    }

    (!sections.is_empty()).then(|| sections.join("\n\n"))
}

// ---------------------------------------------------------------------------
// Pure helpers + prompt builders (unit-tested; no DB / no agent)
// ---------------------------------------------------------------------------

/// Execution rules appended to every docs-agent prompt (writer, summarizer,
/// refine). Mirrors the workflow engine's WF_STEP_RULES: run completion is
/// detected when the TURN ends, so an agent that fans work out to sub-agents
/// gets marked done while its background workers are still writing — exactly
/// the "DONE but 7 agents still running" failure observed live.
const DOCS_STEP_RULES: &str = "\nEXECUTION RULES:\n\
    - Do ALL of the work yourself in THIS turn. Do NOT spawn, launch, or \
    delegate to sub-agents, background agents, parallel workers, or the Task \
    tool. No run_in_background, no fan-out — you are the only agent.\n\
    - Do NOT end your turn until the work is fully complete. Never stop early \
    to \"wait for\" something you started.\n";

/// Non-transcript providers (codex/agy/grok/custom): this much PTY silence
/// after the prompt landed = the turn is over (a WORKING TUI keeps painting).
/// Belt to the done-marker's suspenders — see `done_marker`.
const QUIET_DONE: std::time::Duration = std::time::Duration::from_secs(150);

/// Cap on user-requested retries per writer/summarizer slot within one run —
/// a sanity bound, not a policy (each retry is an explicit click).
const MAX_USER_RETRIES: u32 = 5;

/// Bundled, read-only review methods accepted by the public run request.
pub const VAULT_REVIEWER_SKILLS: [&str; 5] = [
    "vault-docs-review",
    "vault-api-review",
    "vault-data-review",
    "vault-runtime-review",
    "vault-evidence-review",
];

/// Categories shared by every bundled reviewer finding contract.
pub const VAULT_REVIEW_CATEGORIES: [&str; 6] =
    ["coverage", "api", "data", "runtime", "evidence", "quality"];

/// Validate the optional review block before any sessions or durable run rows
/// are created. Provider resolution follows the same defaults as writers, so
/// an empty provider is intentionally valid here.
pub fn validate_review_request(review: &ReviewReq) -> Result<(), String> {
    if review.reviewers.is_empty() || review.reviewers.len() > 4 {
        return Err("review.reviewers must be 1..=4 when review is enabled".into());
    }
    if !(1..=10).contains(&review.max_iterations) {
        return Err("review.max_iterations must be 1..=10".into());
    }
    for (index, reviewer) in review.reviewers.iter().enumerate() {
        let skill = reviewer.skill.trim();
        if !VAULT_REVIEWER_SKILLS.contains(&skill) {
            return Err(format!(
                "review.reviewers[{index}] has unknown reviewer skill '{}'; expected one of: {}",
                reviewer.skill,
                VAULT_REVIEWER_SKILLS.join(", ")
            ));
        }
    }
    Ok(())
}

/// Parse the reviewer's result artifact. Only a bare JSON array satisfying the
/// findings contract is accepted: prose, wrapper objects, incomplete evidence,
/// and unsupported severities are errors rather than accidental clean verdicts.
pub fn parse_review_findings(content: &str) -> Result<Vec<VaultDocsFinding>, String> {
    let findings: Vec<VaultDocsFinding> = serde_json::from_str(content.trim())
        .map_err(|error| format!("review output must be a valid JSON finding array: {error}"))?;
    for (index, finding) in findings.iter().enumerate() {
        if !matches!(finding.severity.as_str(), "blocking" | "major" | "minor") {
            return Err(format!(
                "review finding {index} severity must be blocking, major, or minor"
            ));
        }
        if !VAULT_REVIEW_CATEGORIES.contains(&finding.category.as_str()) {
            return Err(format!(
                "review finding {index} category must be one of: {}",
                VAULT_REVIEW_CATEGORIES.join(", ")
            ));
        }
        for (field, value) in [
            ("summary", finding.summary.as_str()),
            ("missed_item", finding.missed_item.as_str()),
            ("required_fix", finding.required_fix.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(format!("review finding {index} requires non-empty {field}"));
            }
        }
        if finding.evidence.is_empty() {
            return Err(format!(
                "review finding {index} requires at least one evidence location"
            ));
        }
        for (evidence_index, evidence) in finding.evidence.iter().enumerate() {
            let repo_complete = evidence
                .repo_path
                .as_deref()
                .is_some_and(|path| !path.trim().is_empty())
                && evidence.line.is_some_and(|line| line > 0);
            let doc_complete = evidence
                .doc_path
                .as_deref()
                .is_some_and(|path| !path.trim().is_empty())
                && evidence
                    .section
                    .as_deref()
                    .is_some_and(|section| !section.trim().is_empty());
            if !repo_complete && !doc_complete {
                return Err(format!(
                    "review finding {index} evidence {evidence_index} requires repo_path+line or doc_path+section"
                ));
            }
        }
    }
    Ok(findings)
}

/// A round is clean only after every configured reviewer has completed in the
/// same round and every one returned an empty findings array.
pub fn all_reviewers_clean(reviewers: &[VaultDocsReviewer]) -> bool {
    !reviewers.is_empty()
        && reviewers
            .iter()
            .all(|reviewer| reviewer.state == "done" && reviewer.findings.is_empty())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReviewAction {
    Pending,
    Clean,
    Revise,
    Exhausted,
    Error,
}

/// Decide the next stage without advancing an incomplete or failed round.
pub fn next_review_action(
    reviewers: &[VaultDocsReviewer],
    current_iteration: u8,
    max_iterations: u8,
) -> ReviewAction {
    if reviewers.is_empty()
        || reviewers
            .iter()
            .any(|reviewer| !matches!(reviewer.state.as_str(), "pending" | "running" | "done"))
    {
        ReviewAction::Error
    } else if reviewers
        .iter()
        .any(|reviewer| matches!(reviewer.state.as_str(), "pending" | "running"))
    {
        ReviewAction::Pending
    } else if all_reviewers_clean(reviewers) {
        ReviewAction::Clean
    } else if current_iteration >= max_iterations {
        ReviewAction::Exhausted
    } else {
        ReviewAction::Revise
    }
}

/// A fresh done-marker path + the prompt line instructing the agent to write
/// it LAST. Turn completion is transcript-based and only claude has one — for
/// every other provider this file (or `QUIET_DONE` silence) is what flips a
/// finished writer to `done` instead of "running until the 10h idle trip"
/// (the live "codex/grok finished but still RUNNING" bug).
fn done_marker() -> (std::path::PathBuf, String) {
    let path =
        std::env::temp_dir().join(format!("otto-vaultdocs-done-{}.txt", otto_core::new_id()));
    let _ = std::fs::write(&path, "");
    let line = format!(
        "\nLAST OF ALL — strictly after every other step above is complete — write your ONE-LINE \
         summary as plain text to this exact filesystem path: `{}`. Writing this file ends your \
         turn; never write it early.\n",
        path.display()
    );
    (path, line)
}

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
    let mut b = String::new();
    if okf {
        b.push_str(
            "\nOKF — this vault is OKF-conformant and EVERY note you write MUST comply:\n\
             - YAML frontmatter on every note with at least `type:` (concept/service/endpoint/\
             dataset/decision/runbook/metric/…) and a one-sentence `description:`.\n\
             - Internal references use standard markdown links (relative, `.md`).\n\
             - Keep each folder's `index.md` listing its children up to date.\n",
        );
    }
    // `inline` carries EVERY skill selected for this run (okf-authoring plus
    // any prepared-prompt skills like vault-repo-docs) for providers that
    // can't load a staged bundle; claude gets the bundle via extra_dirs and
    // only the invoke hint below.
    match inline {
        Some(text) => {
            b.push_str("\n--- AUTHORING SKILLS (follow these methods exactly) ---\n");
            b.push_str(text);
            b.push_str("\n--- END AUTHORING SKILLS ---\n");
        }
        None if okf => {
            b.push_str(
                "You have the `okf-authoring` skill available — invoke it before writing.\n",
            );
        }
        None => {}
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
    p.push_str(DOCS_STEP_RULES);
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
    p.push_str(DOCS_STEP_RULES);
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
    p.push_str(DOCS_STEP_RULES);
    p.push_str(&okf_block(okf, okf_inline));
    p.push_str(&format!(
        "\nCURRENT CONTENT:\n{inlined}\n\nInstruction:\n{user_prompt}\n\n\
         Reply with ONE line describing the change.\n"
    ));
    p
}

/// Build an independent review turn. The package is referenced by its staged
/// path and the only allowed mutation is the server-owned JSON result file.
#[allow(clippy::too_many_arguments)]
pub fn build_reviewer_prompt(
    original_request: &str,
    vault_id: i64,
    target_dir: &str,
    iteration: u8,
    skill: &str,
    focus: Option<&str>,
    package_path: &str,
    results_path: &str,
) -> String {
    let focus = focus
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| format!("\nADDITIONAL FOCUS (narrows, never waives the method):\n{value}\n"))
        .unwrap_or_default();
    format!(
        "OTTO_TASK: vault_docs_review\n\
         You are an INDEPENDENT REVIEWER in review round {iteration}. Review the current FINAL \
         documentation under {target} in Otto vault {vault_id} against the original request and \
         the real repository in your working directory.\n\n\
         METHOD — open and follow the complete `{skill}` package starting at:\n\
         `{package_path}`\n\
         Read its referenced checklists and examples before judging the bundle.{focus}\n\
         REVIEW RULES — follow exactly:\n\
         - This is read-only with respect to the vault: never call a vault write, rename, or \
         delete tool and never repair findings yourself.\n\
         - Reconcile the current `coverage.md`, final notes, indexes, and deterministic audit \
         output against real code. Do not blindly repeat audit candidates.\n\
         - Every finding must be proven by a real repository path and line and/or an exact \
         documentation path and section. Reject speculative findings and stylistic preferences.\n\
         - Re-read the current bundle in this round; do not repeat a finding already repaired.\n\
         - `[]` is clean only after completing every mandatory check in the method.\n\n\
         OUTPUT — write ONLY one JSON array to `{results_path}`. Each item must have exactly:\n\
         {{\"severity\":\"blocking|major|minor\",\
         \"category\":\"coverage|api|data|runtime|evidence|quality\",\
         \"summary\":\"...\",\"evidence\":[{{\"repo_path\":\"path\",\"line\":1}},\
         {{\"doc_path\":\"path.md\",\"section\":\"Heading\"}}],\
         \"missed_item\":\"...\",\"required_fix\":\"...\"}}\n\
         Missing, malformed, wrapped, or prose-contaminated output is an error.\n\
         {rules}\n\
         ORIGINAL REQUEST:\n{original_request}\n",
        target = dir_display(target_dir),
        rules = DOCS_STEP_RULES,
    )
}

/// Build the resumed final-author repair turn from the combined, persisted
/// findings of one round.
pub fn build_revision_prompt(
    original_request: &str,
    vault_id: i64,
    target_dir: &str,
    iteration: u8,
    findings: &[VaultDocsFinding],
    results_path: &str,
) -> String {
    let findings_json = serde_json::to_string_pretty(findings).unwrap_or_else(|_| "[]".into());
    format!(
        "OTTO_TASK: vault_docs_revise\n\
         Resume as the FINAL AUTHOR for review round {iteration}. Repair the current final \
         documentation under {target} in Otto vault {vault_id}.\n\n\
         REVISION RULES — follow exactly:\n\
         - Re-read the affected source and current notes; apply every valid required_fix below.\n\
         - Preserve correct existing material. Resolve the underlying omission, not only its wording.\n\
         - Refresh `coverage.md`, neighboring `index.md` files, links, examples, diagrams, and text \
         artifacts affected by a repair.\n\
         - Run `otto_vault_okf_validate` and the bundle audit; fix every introduced error.\n\
         - Write through the guarded Otto vault tools only.\n\
         {rules}\n\
         FINALLY, write ONLY this JSON shape to `{results_path}` with every vault-relative path \
         changed in this revision: {{\"written\": [\"path/to/changed.md\", ...]}}\n\n\
         ORIGINAL REQUEST:\n{original_request}\n\n\
         COMBINED SOURCE-BACKED FINDINGS:\n{findings_json}\n",
        target = dir_display(target_dir),
        rules = DOCS_STEP_RULES,
    )
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
        assert!(p.contains("AUTHORING SKILLS"));
        assert!(p.contains(OKF_SKILL));
        // And the claude variant carries the skill NAME, not the body.
        let c = build_writer_prompt("x", 1, 1, 2, "run8run8", "", true, None, None);
        assert!(c.contains("okf-authoring"));
        assert!(!c.contains(OKF_SKILL));
    }

    #[test]
    fn skills_block_renders_on_non_okf_vaults_too() {
        // Prepared-prompt skills must reach the prompt even when okf=false.
        let p = build_writer_prompt(
            "x",
            1,
            1,
            2,
            "run8run8",
            "",
            false,
            Some("SKILL BODY"),
            None,
        );
        assert!(p.contains("AUTHORING SKILLS"));
        assert!(p.contains("SKILL BODY"));
        assert!(!p.contains("OKF — this vault is OKF-conformant"));
        // No skills, no OKF → no block at all.
        let n = build_writer_prompt("x", 1, 1, 2, "run8run8", "", false, None, None);
        assert!(!n.contains("AUTHORING SKILLS"));
    }

    #[test]
    fn staged_package_guidance_is_provider_specific() {
        let mut files = std::collections::HashMap::new();
        files.insert(
            "okf-authoring".to_string(),
            vec!["SKILL.md".to_string(), "references/spec.md".to_string()],
        );
        let staged = crate::modules::StagedSkillPackages {
            root: "/tmp/staged-skills".to_string(),
            files,
        };
        let names = vec!["okf-authoring".to_string()];
        let fallback =
            std::collections::HashMap::from([("okf-authoring".to_string(), "method".to_string())]);

        let codex = skill_package_guidance("codex", Some(&staged), &names, &fallback).unwrap();
        assert!(codex.contains("okf-authoring/SKILL.md"));
        assert!(codex.contains("references/spec.md"));
        assert!(
            !codex.contains("method"),
            "package bodies must not be inlined"
        );

        let claude = skill_package_guidance("claude", Some(&staged), &names, &fallback).unwrap();
        assert!(claude.contains("invoke"));
        assert!(claude.contains("okf-authoring"));
        assert!(!claude.contains("references/spec.md"));
    }

    #[test]
    fn partial_staging_inlines_only_failed_packages() {
        let staged = crate::modules::StagedSkillPackages {
            root: "/tmp/staged-skills".to_string(),
            files: std::collections::HashMap::from([(
                "okf-authoring".to_string(),
                vec!["SKILL.md".to_string()],
            )]),
        };
        let names = vec!["okf-authoring".to_string(), "jira-story-writer".to_string()];
        let fallback = std::collections::HashMap::from([
            ("okf-authoring".to_string(), "OKF BODY".to_string()),
            ("jira-story-writer".to_string(), "JIRA BODY".to_string()),
        ]);

        for provider in ["codex", "claude"] {
            let guidance =
                skill_package_guidance(provider, Some(&staged), &names, &fallback).unwrap();
            assert!(guidance.contains("JIRA BODY"));
            assert!(!guidance.contains("OKF BODY"));
        }
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
        // OKF: validate + index refresh + skill guidance; results file; target dir.
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
    fn review_request_defaults_method_and_iterations() {
        let req: RunReq = serde_json::from_value(serde_json::json!({
            "prompt": "document everything",
            "agents": [{"provider": "codex"}],
            "review": {"reviewers": [{"provider": "claude"}]}
        }))
        .unwrap();
        let review = req.review.as_ref().unwrap();
        assert_eq!(review.max_iterations, 3);
        assert_eq!(review.reviewers[0].skill, "vault-docs-review");
        assert!(validate_review_request(review).is_ok());
    }

    #[test]
    fn review_request_validation_is_actionable() {
        let review = |reviewers: Vec<ReviewerReq>, max_iterations| ReviewReq {
            reviewers,
            max_iterations,
        };
        let reviewer = |skill: &str| ReviewerReq {
            provider: "claude".into(),
            model: None,
            skill: skill.into(),
            focus: None,
        };

        let err = validate_review_request(&review(vec![], 3)).unwrap_err();
        assert!(err.contains("reviewers must be 1..=4"));
        let err = validate_review_request(&review(
            (0..5).map(|_| reviewer("vault-docs-review")).collect(),
            3,
        ))
        .unwrap_err();
        assert!(err.contains("reviewers must be 1..=4"));
        let err =
            validate_review_request(&review(vec![reviewer("vault-docs-review")], 0)).unwrap_err();
        assert!(err.contains("max_iterations must be 1..=10"));
        let err =
            validate_review_request(&review(vec![reviewer("vault-docs-review")], 11)).unwrap_err();
        assert!(err.contains("max_iterations must be 1..=10"));
        let err =
            validate_review_request(&review(vec![reviewer("unknown-review")], 3)).unwrap_err();
        assert!(err.contains("unknown reviewer skill 'unknown-review'"));
        assert!(err.contains("vault-docs-review"));
    }

    fn sample_reviewer(state: &str, findings: Vec<VaultDocsFinding>) -> VaultDocsReviewer {
        VaultDocsReviewer {
            index: 0,
            provider: "claude".into(),
            model: None,
            skill: "vault-docs-review".into(),
            focus: None,
            state: state.into(),
            session_id: None,
            findings,
            error: None,
        }
    }

    #[test]
    fn review_findings_parser_accepts_empty_and_source_backed_arrays() {
        assert!(parse_review_findings("[]").unwrap().is_empty());
        let findings = parse_review_findings(
            r#"[{
                "severity":"blocking",
                "category":"api",
                "summary":"POST /widgets omits its 422 response body",
                "evidence":[
                    {"repo_path":"src/routes/widgets.rs","line":91},
                    {"doc_path":"widgets/api.md","section":"POST /widgets"}
                ],
                "missed_item":"ValidationError response schema and example",
                "required_fix":"Document the 422 schema/example and add it to OpenAPI"
            }]"#,
        )
        .unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "blocking");
        assert_eq!(findings[0].evidence[0].line, Some(91));
        assert_eq!(
            findings[0].evidence[1].doc_path.as_deref(),
            Some("widgets/api.md")
        );
    }

    #[test]
    fn review_findings_parser_rejects_malformed_or_unproven_output() {
        for malformed in [
            "not json",
            r#"{"findings": []}"#,
            r#"[{"severity":"blocking"}]"#,
            r#"[{"severity":"urgent","category":"api","summary":"x","evidence":[{"repo_path":"a.rs","line":1}],"missed_item":"x","required_fix":"x"}]"#,
            r#"[{"severity":"major","category":"banana","summary":"x","evidence":[{"repo_path":"a.rs","line":1}],"missed_item":"x","required_fix":"x"}]"#,
            r#"[{"severity":"major","category":"api","summary":"x","evidence":[],"missed_item":"x","required_fix":"x"}]"#,
            r#"[{"severity":"major","category":"api","summary":"x","evidence":[{}],"missed_item":"x","required_fix":"x"}]"#,
        ] {
            assert!(
                parse_review_findings(malformed).is_err(),
                "must reject: {malformed}"
            );
        }
    }

    #[test]
    fn review_round_decision_requires_same_round_clean_and_honors_cap() {
        let clean = sample_reviewer("done", vec![]);
        let finding = VaultDocsFinding {
            severity: "major".into(),
            category: "data".into(),
            summary: "Missing write impact".into(),
            evidence: vec![VaultDocsFindingEvidence {
                repo_path: Some("src/dao.rs".into()),
                line: Some(42),
                doc_path: Some("data/orders.md".into()),
                section: Some("Writes".into()),
            }],
            missed_item: "transaction boundary".into(),
            required_fix: "Document the transaction boundary".into(),
        };
        let dirty = sample_reviewer("done", vec![finding]);
        let still_running = sample_reviewer("running", vec![]);

        assert!(all_reviewers_clean(&[clean.clone(), clean.clone()]));
        assert!(!all_reviewers_clean(&[]));
        assert!(!all_reviewers_clean(&[clean.clone(), dirty.clone()]));
        assert!(!all_reviewers_clean(&[clean.clone(), still_running]));
        assert_eq!(next_review_action(&[clean], 1, 3), ReviewAction::Clean);
        assert_eq!(
            next_review_action(&[dirty.clone()], 1, 3),
            ReviewAction::Revise
        );
        assert_eq!(next_review_action(&[dirty], 3, 3), ReviewAction::Exhausted);

        for state in ["pending", "running"] {
            let incomplete = sample_reviewer(state, vec![]);
            assert_eq!(
                next_review_action(&[incomplete.clone()], 1, 3),
                ReviewAction::Pending
            );
            assert_eq!(
                next_review_action(&[incomplete], 3, 3),
                ReviewAction::Pending
            );
        }
        for state in ["error", "cancelled", "interrupted"] {
            let failed = sample_reviewer(state, vec![]);
            assert_eq!(
                next_review_action(&[failed.clone()], 1, 3),
                ReviewAction::Error
            );
            assert_eq!(next_review_action(&[failed], 3, 3), ReviewAction::Error);
        }
        assert_eq!(next_review_action(&[], 1, 3), ReviewAction::Error);
    }

    #[test]
    fn review_prompt_carries_method_focus_evidence_and_result_contract() {
        let prompt = build_reviewer_prompt(
            "Document the widget service",
            9,
            "widgets",
            2,
            "vault-api-review",
            Some("Prioritize externally consumed contracts"),
            "/tmp/staged/skills/vault-api-review/SKILL.md",
            "/tmp/review-round-2-agent-1.json",
        );
        assert!(prompt.contains("OTTO_TASK: vault_docs_review"));
        assert!(prompt.contains("round 2"));
        assert!(prompt.contains("vault-api-review"));
        assert!(prompt.contains("/tmp/staged/skills/vault-api-review/SKILL.md"));
        assert!(prompt.contains("Prioritize externally consumed contracts"));
        assert!(prompt.contains("real repository path and line"));
        assert!(prompt.contains("speculative"));
        assert!(prompt.contains("read-only"));
        assert!(prompt.contains("missed_item"));
        assert!(prompt.contains("required_fix"));
        assert!(prompt.contains("coverage|api|data|runtime|evidence|quality"));
        assert!(prompt.contains("/tmp/review-round-2-agent-1.json"));
        assert!(prompt.contains("Document the widget service"));
    }

    #[test]
    fn review_revision_prompt_carries_findings_and_changed_path_results_contract() {
        let finding = VaultDocsFinding {
            severity: "major".into(),
            category: "api".into(),
            summary: "Missing response body".into(),
            evidence: vec![VaultDocsFindingEvidence {
                repo_path: Some("src/routes.rs".into()),
                line: Some(8),
                doc_path: Some("api/widgets.md".into()),
                section: Some("POST /widgets".into()),
            }],
            missed_item: "422 schema".into(),
            required_fix: "Add schema and example".into(),
        };
        let prompt = build_revision_prompt(
            "Document the widget service",
            9,
            "widgets",
            2,
            &[finding],
            "/tmp/revision-round-2.json",
        );
        assert!(prompt.contains("OTTO_TASK: vault_docs_revise"));
        assert!(prompt.contains("Missing response body"));
        assert!(prompt.contains("Add schema and example"));
        assert!(prompt.contains("coverage.md"));
        assert!(prompt.contains("index.md"));
        assert!(prompt.contains("otto_vault_okf_validate"));
        assert!(prompt.contains(r#"{"written": ["path/to/changed.md", ...]}"#));
        assert!(prompt.contains("/tmp/revision-round-2.json"));
    }

    fn sample_run(state: &str) -> VaultDocsRun {
        VaultDocsRun {
            id: "run-12345678".into(),
            ws_id: "w1".into(),
            vault_id: 1,
            kind: "docs".into(),
            prompt: "p".into(),
            target_dir: String::new(),
            note_path: String::new(),
            state: state.into(),
            agents: vec![
                VaultDocsAgent {
                    index: 0,
                    name: "writer-1 · claude".into(),
                    provider: "claude".into(),
                    model: None,
                    state: "done".into(),
                    session_id: Some("s1".into()),
                    error: None,
                    drafts: vec![],
                },
                VaultDocsAgent {
                    index: 1,
                    name: "writer-2 · claude".into(),
                    provider: "claude".into(),
                    model: None,
                    state: "running".into(),
                    session_id: Some("s2".into()),
                    error: None,
                    drafts: vec![],
                },
                VaultDocsAgent {
                    index: 2,
                    name: "writer-3 · claude".into(),
                    provider: "claude".into(),
                    model: None,
                    state: "pending".into(),
                    session_id: None,
                    error: None,
                    drafts: vec![],
                },
            ],
            summarizer: VaultDocsSummarizer {
                provider: "claude".into(),
                model: None,
                state: "pending".into(),
                session_id: None,
                error: None,
            },
            review: VaultDocsReview::default(),
            written: vec![],
            error: None,
            started_at: "2026-07-12T10:00:00Z".into(),
            finished_at: None,
        }
    }

    #[test]
    fn interrupted_sweep_flips_only_non_terminal_states() {
        let mut run = sample_run("running");
        let reviewer = sample_reviewer("running", vec![]);
        run.review = VaultDocsReview {
            state: "reviewing".into(),
            max_iterations: 3,
            current_iteration: 1,
            outcome: None,
            reviewers: vec![reviewer.clone()],
            rounds: vec![VaultDocsReviewRound {
                iteration: 1,
                state: "revising".into(),
                reviewers: vec![reviewer],
                revision: VaultDocsRevision {
                    state: "pending".into(),
                    ..VaultDocsRevision::default()
                },
            }],
        };
        assert!(mark_run_interrupted(&mut run));
        assert_eq!(run.state, "interrupted");
        assert!(run.finished_at.is_some());
        assert!(run.error.as_deref().unwrap().contains("restart"));
        // done stays done; running/pending flip.
        assert_eq!(run.agents[0].state, "done");
        assert_eq!(run.agents[1].state, "interrupted");
        assert_eq!(run.agents[2].state, "interrupted");
        assert_eq!(run.summarizer.state, "interrupted");
        assert_eq!(run.review.state, "interrupted");
        assert_eq!(run.review.reviewers[0].state, "interrupted");
        assert_eq!(run.review.rounds[0].state, "interrupted");
        assert_eq!(run.review.rounds[0].reviewers[0].state, "interrupted");
        assert_eq!(run.review.rounds[0].revision.state, "interrupted");

        // Terminal runs are untouched (idempotent across sweeps).
        let mut done = sample_run("done");
        assert!(!mark_run_interrupted(&mut done));
        assert_eq!(done.state, "done");
        let mut exhausted = sample_run("done_with_findings");
        assert!(!mark_run_interrupted(&mut exhausted));
        assert_eq!(exhausted.state, "done_with_findings");
        let mut twice = sample_run("running");
        mark_run_interrupted(&mut twice);
        assert!(!mark_run_interrupted(&mut twice));

        // A skipped summarizer (single-writer / all-failed) stays skipped.
        let mut single = sample_run("running");
        single.summarizer.state = "skipped".into();
        mark_run_interrupted(&mut single);
        assert_eq!(single.summarizer.state, "skipped");
    }

    #[test]
    fn payload_without_kind_or_note_path_still_deserializes_as_docs() {
        // Forward-compat: rows written before the fields existed.
        let legacy = r#"{
            "id": "r1", "ws_id": "w1", "vault_id": 1, "prompt": "p",
            "target_dir": "", "state": "running", "agents": [],
            "summarizer": {"provider": "claude", "model": null, "state": "pending",
                           "session_id": null, "error": null},
            "written": [], "error": null,
            "started_at": "t", "finished_at": null
        }"#;
        let run: VaultDocsRun = serde_json::from_str(legacy).unwrap();
        assert_eq!(run.kind, "docs");
        assert_eq!(run.note_path, "");
        assert_eq!(run.review.state, "skipped");
        assert_eq!(run.review.max_iterations, 3);
        assert!(run.review.rounds.is_empty());
        // And a full round-trip preserves the new fields.
        let mut run = sample_run("running");
        run.kind = "refine".into();
        run.note_path = "docs/a.md".into();
        let back: VaultDocsRun =
            serde_json::from_str(&serde_json::to_string(&run).unwrap()).unwrap();
        assert_eq!(back.kind, "refine");
        assert_eq!(back.note_path, "docs/a.md");
    }

    #[test]
    fn orphan_drafts_move_to_trash_with_collision_suffix() {
        let tmp = std::env::temp_dir().join(format!("otto-vdr-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let drafts = tmp.join("_drafts/docs-run-run-1234/agent-1");
        std::fs::create_dir_all(&drafts).unwrap();
        std::fs::write(drafts.join("a.md"), "draft").unwrap();
        let root = tmp.to_string_lossy().to_string();

        // "run-1234" are the first 8 chars of the run id.
        assert!(trash_orphan_drafts(&root, "run-12345678"));
        assert!(!tmp.join("_drafts/docs-run-run-1234").exists());
        assert!(tmp.join(".trash/docs-run-run-1234/agent-1/a.md").exists());

        // Missing dir → no-op.
        assert!(!trash_orphan_drafts(&root, "run-12345678"));

        // Collision (same run trashed before) → `-interrupted` suffix.
        std::fs::create_dir_all(tmp.join("_drafts/docs-run-run-1234")).unwrap();
        assert!(trash_orphan_drafts(&root, "run-12345678"));
        assert!(tmp.join(".trash/docs-run-run-1234-interrupted").exists());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn run_state_transitions_respect_terminal_states() {
        let reg = new_run_registry();
        let run = VaultDocsRun {
            id: "r1".into(),
            ws_id: "w1".into(),
            vault_id: 1,
            kind: "docs".into(),
            prompt: "p".into(),
            target_dir: String::new(),
            note_path: String::new(),
            state: "running".into(),
            agents: vec![],
            summarizer: VaultDocsSummarizer {
                provider: "claude".into(),
                model: None,
                state: "pending".into(),
                session_id: None,
                error: None,
            },
            review: VaultDocsReview::default(),
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
                retries: Arc::new(Mutex::new(HashSet::new())),
                persist_tx: None,
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
