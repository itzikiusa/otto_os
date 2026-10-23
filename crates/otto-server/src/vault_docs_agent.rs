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
//!   path checks + rescan. The writers, the summarizer and every per-slot
//!   user retry run on the shared fan-out runner (`otto_agent_run::fanout`:
//!   per-agent error isolation, a retry re-runs only its slot, sessions left
//!   open); see `vault_docs_agent/{run,authors}.rs`.
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
//! Cancel signals every admitted turn and prevents later stage admission.
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

mod authors;
mod prompts;
mod refine;
mod retry;
mod review_loop;
mod run;
use authors::*;
pub use prompts::*;
use refine::*;
use retry::*;
use review_loop::*;
use run::*;

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
    /// `reviewing | revising | revised | clean | exhausted | error | cancelled |
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
    /// Immutable resolved reviewer templates retained for wire compatibility.
    /// Live session/state/findings exist only in `rounds[*].reviewers`, avoiding
    /// two mutable sources of truth in the persisted payload.
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
/// between stages (mirrors the `otto_product::run::CancelRegistry` shape).
pub struct RunEntry {
    pub run: VaultDocsRun,
    pub cancel: Arc<AtomicBool>,
    /// User-requested retries, keyed by the fan-out runner's slot index
    /// (writer index, `fanout::SUMMARIZER_RETRY_IDX` = summarizer) and by
    /// [`reviewer_retry_key`] / [`revision_retry_key`] for the review loop.
    /// The retry endpoint kills the target's session and requests here; the
    /// runner's slot loop consumes one request per re-spawn and closes a slot
    /// once its final permitted attempt is admitted.
    pub retries: otto_agent_run::fanout::Retries,
    /// Signal into the run's write-behind persister (`None` in unit tests,
    /// which have no runtime).
    pub persist_tx: Option<PersistTx>,
}

const REVIEW_RETRY_BASE: usize = 10_000;
const REVISION_RETRY_BASE: usize = 20_000;

fn reviewer_retry_key(iteration: u8, index: usize) -> usize {
    REVIEW_RETRY_BASE + usize::from(iteration) * 10 + index
}

fn revision_retry_key(iteration: u8) -> usize {
    REVISION_RETRY_BASE + usize::from(iteration)
}

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
    /// Fences a detached turn's callbacks against reset or a newer turn.
    turn_id: Option<String>,
}

/// Workspace and creator scope conversational state even though vault files are global.
pub type RefineKey = (String, String, i64, String);
pub type RefineRegistry = Arc<Mutex<HashMap<RefineKey, RefineEntry>>>;

pub fn new_refine_registry() -> RefineRegistry {
    Arc::new(Mutex::new(HashMap::new()))
}

fn update_refine_entry(
    registry: &RefineRegistry,
    key: &RefineKey,
    turn_id: &str,
    session: Option<&Id>,
    finished: bool,
) {
    let mut registry = registry.lock().unwrap();
    let Some(entry) = registry
        .get_mut(key)
        .filter(|entry| entry.turn_id.as_deref() == Some(turn_id))
    else {
        return;
    };
    if let Some(session) = session {
        entry.session_id = Some(session.clone());
    }
    if finished {
        entry.running = false;
    }
}

/// Mutate one run's snapshot under the registry lock (no-op when gone).
fn with_run(reg: &RunRegistry, run_id: &str, f: impl FnOnce(&mut VaultDocsRun)) {
    if let Some(e) = reg.lock().unwrap().get_mut(run_id) {
        if !is_terminal(&e.run.state) {
            f(&mut e.run);
        }
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
            get(refine_session).delete(reset_refine_session),
        )
        .route(
            "/workspaces/{ws}/vault/vaults/{id}/docs-agents/runs",
            get(list_runs),
        )
        // NOT ws-scoped: the run id carries its ws, and the handlers re-check
        // the caller's role against it (policy row: /vault/docs-agents/*).
        .route(
            "/vault/docs-agents/runs/{run_id}",
            get(get_run).delete(delete_run),
        )
        .route("/vault/docs-agents/runs/{run_id}/cancel", post(cancel_run))
        .route(
            "/vault/docs-agents/runs/{run_id}/resolve",
            post(resolve_run),
        )
        .route(
            "/vault/docs-agents/runs/{run_id}/agents/{index}/retry",
            post(retry_agent),
        )
        .route(
            "/vault/docs-agents/runs/{run_id}/summarizer/retry",
            post(retry_summarizer),
        )
        .route(
            "/vault/docs-agents/runs/{run_id}/review/rounds/{iteration}/reviewers/{index}/retry",
            post(retry_reviewer),
        )
        .route(
            "/vault/docs-agents/runs/{run_id}/review/rounds/{iteration}/revision/retry",
            post(retry_revision),
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
    let ws_fallback = ctx
        .resolve_provider(Some(&ws), None)
        .await
        .map_err(ApiError)?;
    let resolve =
        |explicit: &str| otto_core::provider::resolve_provider(&[explicit, ws_fallback.as_str()]);

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
            retries: Default::default(),
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

#[derive(Debug, Deserialize)]
struct ResolveReq {
    /// `ok` — the findings were reviewed and accepted as-is;
    /// `fixed` — the findings were addressed (by hand or a follow-up agent).
    outcome: String,
}

/// `POST /vault/docs-agents/runs/{run_id}/resolve` — user disposition for a
/// `done_with_findings` run: flip it to `done` and stamp `review.outcome` with
/// `resolved_ok` / `resolved_fixed`. Terminal runs live only in the durable
/// row (the registry evicts on finalize), so this mutates the row directly;
/// 409 for any other state — an active run resolves through its own lifecycle.
async fn resolve_run(
    Path(run_id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<ResolveReq>,
) -> ApiResult<Json<VaultDocsRun>> {
    let outcome = match req.outcome.as_str() {
        "ok" => "resolved_ok",
        "fixed" => "resolved_fixed",
        other => {
            return Err(ApiError(Error::Invalid(format!(
                "outcome must be `ok` or `fixed`, got `{other}`"
            ))));
        }
    };
    let repo = otto_state::VaultDocsRunsRepo::new(ctx.pool.clone());
    let row = repo
        .get(&run_id)
        .await
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::NotFound(format!("docs run {run_id}"))))?;
    let mut run = row_to_run_dto(&row)
        .ok_or_else(|| ApiError(Error::Internal(format!("unparseable run {run_id}"))))?;
    crate::auth::require_ws_role(
        &ctx,
        &user,
        &Id::from(run.ws_id.clone()),
        WorkspaceRole::Editor,
    )
    .await?;
    if run.state != "done_with_findings" {
        return Err(ApiError(Error::Conflict(format!(
            "only a done_with_findings run can be resolved (state: {})",
            run.state
        ))));
    }
    run.state = "done".into();
    run.review.outcome = Some(outcome.into());
    repo.upsert(&run_row(&run)).await.map_err(ApiError)?;
    // A just-finished run can still sit in the registry until finalize evicts
    // it — keep that snapshot consistent so a racing poll doesn't resurrect
    // the old state.
    with_run(&ctx.vault_docs_runs, &run_id, |r| {
        r.state = "done".into();
        r.review.outcome = Some(outcome.into());
    });
    Ok(Json(run))
}

/// `DELETE /vault/docs-agents/runs/{run_id}` — history cleanup: drop one
/// TERMINAL run's durable row (and any lingering registry snapshot). An
/// active run must be cancelled first — 409 otherwise.
async fn delete_run(
    Path(run_id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<axum::http::StatusCode> {
    let repo = otto_state::VaultDocsRunsRepo::new(ctx.pool.clone());
    let row = repo
        .get(&run_id)
        .await
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::NotFound(format!("docs run {run_id}"))))?;
    crate::auth::require_ws_role(
        &ctx,
        &user,
        &Id::from(row.ws_id.clone()),
        WorkspaceRole::Editor,
    )
    .await?;
    // The registry snapshot is fresher than the row while a run is live.
    let live_state = ctx
        .vault_docs_runs
        .lock()
        .unwrap()
        .get(&run_id)
        .map(|e| e.run.state.clone());
    let state = live_state.unwrap_or_else(|| row.state.clone());
    if !is_terminal(&state) {
        return Err(ApiError(Error::Conflict(
            "run is still active — cancel it before deleting".into(),
        )));
    }
    repo.delete(&run_id).await.map_err(ApiError)?;
    ctx.vault_docs_runs.lock().unwrap().remove(&run_id);
    Ok(axum::http::StatusCode::NO_CONTENT)
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
        .list_for_vault(&ws_id, vault_id, limit)
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
            if e.run.vault_id == vault_id && e.run.ws_id == ws_id && !seen.contains(&e.run.id) {
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
        if let Some(round) = e.run.review.rounds.last() {
            for reviewer in &round.reviewers {
                if matches!(reviewer.state.as_str(), "pending" | "running") {
                    if let Some(sid) = reviewer.session_id.clone() {
                        sids.push(Id::from(sid));
                    }
                }
            }
            if matches!(round.revision.state.as_str(), "pending" | "running") {
                if let Some(sid) = round.revision.session_id.clone() {
                    sids.push(Id::from(sid));
                }
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
        if matches!(
            r.review.state.as_str(),
            "pending" | "reviewing" | "revising"
        ) {
            r.review.state = "cancelled".into();
            r.review.outcome = Some("cancelled".into());
        }
        if let Some(round) = r.review.rounds.last_mut() {
            round.state = "cancelled".into();
            for reviewer in &mut round.reviewers {
                if matches!(reviewer.state.as_str(), "pending" | "running") {
                    reviewer.state = "cancelled".into();
                }
            }
            if matches!(round.revision.state.as_str(), "pending" | "running") {
                round.revision.state = "cancelled".into();
            }
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

/// Apply restart semantics to one durable row and persist the transformed
/// nested payload together with the flat state. Keeping this composition in
/// one seam prevents recovery from updating the list column while leaving a
/// stale live reviewer/revision inside `payload`.
async fn persist_interrupted_row(
    repo: &otto_state::VaultDocsRunsRepo,
    row: &mut otto_state::VaultDocsRunRow,
) -> otto_core::Result<Option<VaultDocsRun>> {
    row.state = "interrupted".into();
    let run = match serde_json::from_str::<VaultDocsRun>(&row.payload) {
        Ok(mut run) => {
            mark_run_interrupted(&mut run);
            row.payload = serde_json::to_string(&run).unwrap_or_else(|_| row.payload.clone());
            row.finished_at = run.finished_at.clone();
            Some(run)
        }
        Err(error) => {
            warn!("vault_docs: recover: bad payload for {}: {}", row.id, error);
            None
        }
    };
    repo.upsert(row).await?;
    Ok(run)
}

/// Startup sweep (awaited before admission): every run row still
/// non-terminal was killed by the restart — mark it `interrupted`, soft-trash
/// its orphaned `_drafts` dir (multi-writer docs runs), and rescan the touched
/// vaults (in the background) so the tree stops showing the leftovers.
pub async fn recover_interrupted(ctx: ServerCtx) -> otto_core::Result<()> {
    let repo = otto_state::VaultDocsRunsRepo::new(ctx.pool.clone());
    let rows = repo.list_unfinished().await?;
    let mut marked = 0usize;
    let mut rescan: HashSet<i64> = HashSet::new();
    for mut row in rows {
        if let Some(run) = persist_interrupted_row(&repo, &mut row).await? {
            if run.kind == "docs" && run.agents.len() > 1 {
                // Vault may be gone/re-scoped — resolve tolerantly.
                if let Ok(vault) = ctx.vault.get_scoped(&row.ws_id, row.vault_id).await {
                    if trash_orphan_drafts(&vault.root_path, &run.id) {
                        rescan.insert(row.vault_id);
                    }
                }
            }
        }
        marked += 1;
    }
    tracing::info!(
        "vault_docs: marked {marked} interrupted run(s) from before restart \
         ({} vault(s) to rescan after drafts cleanup)",
        rescan.len()
    );
    // A rescan walks the whole vault: off boot's critical path (it is awaited
    // before the listener serves), so a large vault never delays startup.
    if !rescan.is_empty() {
        tokio::spawn(async move {
            for vault_id in rescan {
                let _ = ctx.vault.scan(vault_id).await;
            }
        });
    }
    Ok(())
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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "../tests/unit/vault_docs_agent.rs"]
mod tests;

#[cfg(test)]
#[path = "../tests/unit/vault_refine_scope.rs"]
mod refine_scope_tests;
