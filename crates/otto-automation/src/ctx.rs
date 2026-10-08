//! [`AutomationCtx`] — everything the scheduled-task and goal-loop engines need
//! from the daemon, and nothing more.
//!
//! The engines used to take otto-server's god-struct `ServerCtx`; they now take
//! `&impl AutomationCtx`, which otto-server implements for `ServerCtx` as thin
//! glue (`otto_server::automation_ctx`). Two kinds of members:
//!
//! - **narrow handles** — the pool, event bus, session manager and the repos the
//!   engines read/write directly;
//! - **server services** — things still owned by otto-server (the shared
//!   agent-run runner, managed role turns, proof packs, skill composition, the
//!   workflow engine, run notices, provider resolution), exposed as methods so
//!   this crate never depends on otto-server.

use std::future::Future;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use otto_core::domain::Workspace;
use otto_core::event::Event;
use otto_core::proof::{ProofArtifact, ProofArtifactKind, ProofArtifactStatus, ProofPack};
use otto_core::secrets::SecretStore;
use otto_core::workflows::{NodeRunState, RunScope, Workflow};
use otto_core::{Id, Result};
use otto_sessions::SessionManager;
use otto_state::{DbPool, GoalLoopsRepo, ProofRepo, ScheduledTasksRepo, WorkspacesRepo};
use serde_json::Value;
use tokio::sync::broadcast;

use crate::agent::{RoleTurn, RunOutcome, WatchStatus};
use crate::goal_loop::GoalLoopRegistry;

/// The daemon surface the automation engines run against. Cheap to clone (the
/// engines clone it into spawned controllers / run tasks).
pub trait AutomationCtx: Clone + Send + Sync + 'static {
    // ---- narrow handles ---------------------------------------------------

    fn pool(&self) -> &DbPool;
    fn secrets(&self) -> &Arc<dyn SecretStore>;
    fn events(&self) -> &broadcast::Sender<Event>;
    /// The daemon data dir (`<data_dir>/scheduled`, `<data_dir>/goal-loops`).
    fn data_dir(&self) -> &Path;
    fn manager(&self) -> &Arc<SessionManager>;
    fn workspaces(&self) -> &WorkspacesRepo;
    /// The headless runner (owner-less scheduled tasks).
    fn orchestrator(&self) -> &Arc<otto_orchestrator::Orchestrator>;
    fn proof_repo(&self) -> &ProofRepo;
    fn scheduled_tasks(&self) -> &ScheduledTasksRepo;
    fn goal_loops_repo(&self) -> &GoalLoopsRepo;
    /// Live goal-loop controllers (owned by the daemon state, run by this crate).
    fn goal_loops(&self) -> &GoalLoopRegistry;

    // ---- provider resolution ----------------------------------------------

    /// The provider for a workspace-scoped run: `requested`, else the
    /// workspace / daemon default.
    fn resolve_provider(
        &self,
        ws: Option<&Workspace>,
        requested: Option<&str>,
    ) -> impl Future<Output = Result<String>> + Send;

    /// [`Self::resolve_provider`] by workspace id.
    fn resolve_provider_for_ws(
        &self,
        ws_id: &Id,
        requested: Option<&str>,
    ) -> impl Future<Output = Result<String>> + Send;

    // ---- the shared agent-run runner --------------------------------------

    /// Run `attempt` up to `max_attempts` times with auto-recovery (kill the
    /// failed session, back off `backoff[min(i, last)]`); `cancel` short-circuits
    /// with `Stopped`. Returns the first success, or the last failure.
    fn run_with_recovery<F, Fut>(
        &self,
        max_attempts: u32,
        backoff: &[Duration],
        cancel: Option<&Arc<AtomicBool>>,
        attempt: F,
    ) -> impl Future<Output = RunOutcome> + Send
    where
        F: FnMut(u32) -> Fut + Send,
        Fut: Future<Output = RunOutcome> + Send;

    /// Watch a freshly-spawned, already-prompted session for its result (the
    /// out-file, or for claude an accepted transcript turn); classifies exit /
    /// stuck / timeout. `on_status` fires on each Waiting/Resumed transition.
    #[allow(clippy::too_many_arguments)]
    fn watch_for_result<F, Fut>(
        &self,
        sid: &Id,
        provider: &str,
        provider_session_id: Option<&str>,
        cwd: &str,
        out_path: &Path,
        timeout: Duration,
        waiting_idle: Duration,
        stuck_idle: Duration,
        transcript_ok: Option<fn(&str) -> bool>,
        result_ok: Option<fn(&str) -> bool>,
        on_status: F,
    ) -> impl Future<Output = RunOutcome> + Send
    where
        F: FnMut(WatchStatus) -> Fut + Send,
        Fut: Future<Output = ()> + Send;

    /// Wait (bounded) until the session's TUI has drawn its input box.
    fn wait_for_tui(&self, sid: &Id) -> impl Future<Output = bool> + Send;

    /// Whether a submitted prompt visibly dispatched since `before`.
    fn dispatched(&self, sid: &Id, before: Option<Instant>) -> impl Future<Output = bool> + Send;

    /// `text` wrapped in bracketed-paste markers.
    fn bracketed_paste(&self, text: &str) -> Vec<u8>;

    /// Settle time between a paste and the Enter that submits it.
    fn paste_to_enter(&self) -> Duration;

    /// One managed role turn (goal-loop planner / evaluator / digester /
    /// definer): spawn the session, publish its id through `publish` as soon as
    /// it is ready, end on a validated `done_file`, stop the session on
    /// `cancelled` / deadline. Returns the done-file text or an error string.
    fn run_role_turn<C, P, PF>(
        &self,
        turn: RoleTurn<'_>,
        cancelled: C,
        publish: P,
    ) -> impl Future<Output = std::result::Result<String, String>> + Send
    where
        C: Future<Output = ()> + Send,
        P: FnOnce(String) -> PF + Send,
        PF: Future<Output = std::result::Result<(), String>> + Send;

    // ---- other server services --------------------------------------------

    /// Compose a skill (resolved from the context library, inlined) with a
    /// wrapped task prompt.
    fn compose_skill_prompt(&self, skill: &str, wrapped: &str) -> String;

    /// Hand a workflow run (already admitted via
    /// `WorkflowsRepo::admit_run_if_idle`) to the workflow engine.
    #[allow(clippy::too_many_arguments)]
    fn spawn_workflow_run(
        &self,
        ws: Workspace,
        workflow: Workflow,
        run_id: Id,
        input: Value,
        scope: RunScope,
        prior_nodes: Option<Vec<NodeRunState>>,
    );

    /// Ensure the proof pack for a work item (the proof gate).
    fn proof_gate(
        &self,
        kind: otto_core::proof::WorkItemKind,
        work_item_id: &str,
        workspace_id: &str,
        title: &str,
        created_by: &str,
    ) -> impl Future<Output = Result<ProofPack>> + Send;

    /// Store a text artifact on a proof pack (offloading large content).
    #[allow(clippy::too_many_arguments)]
    fn proof_upsert_content_artifact(
        &self,
        pack: &ProofPack,
        kind: ProofArtifactKind,
        title: &str,
        content: &str,
        status: ProofArtifactStatus,
        extra_meta: Value,
        created_by: &str,
    ) -> impl Future<Output = Result<ProofArtifact>> + Send;

    /// Recompute a pack's status/risk/done score and broadcast it.
    fn proof_recompute_and_emit(
        &self,
        pack_id: &str,
    ) -> impl Future<Output = Result<ProofPack>> + Send;

    /// Notification-center streak for an unattended entity (`kind`, `id`):
    /// `failure = Some((title, body))` posts once per failure streak (clicking
    /// opens `route`); `None` ends the streak.
    #[allow(clippy::too_many_arguments)]
    fn run_notice(
        &self,
        kind: &str,
        id: &str,
        failure: Option<(String, String)>,
        route: String,
        workspace_id: Option<String>,
        user_id: Option<String>,
    ) -> impl Future<Output = ()> + Send;
}
