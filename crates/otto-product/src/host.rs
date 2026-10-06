//! What the product runners (`run`, `watcher`) need from the host daemon.
//!
//! [`ProductRunHost`] extends the router's [`ProductCtx`] with the handful of
//! daemon handles the background engines use: the event bus, the workspaces
//! repo, the per-agent Stop registry, and the one PTY primitive — running a
//! provider as a real, openable session with bounded auto-recovery. The PTY
//! mechanics stay in `otto-server` (beside its shared `agent_run` /
//! `review_session` infra); this crate owns the prompts, fan-out, persistence
//! and summarizers built on top of them.

use std::future::Future;
use std::path::Path;
use std::time::Duration;

use std::sync::Arc;

use otto_core::domain::{User, Workspace};
use otto_core::event::Event;
use otto_core::{Id, Result};
use otto_improve::ImprovementEngine;
use otto_orchestrator::Orchestrator;
use otto_state::{
    CanvasRepo, DbPool, DiscoveryChatRepo, ProductAttachmentRepo, ProductDiscoveryRepo,
    ProductMockupRepo, ProductRefinementRepo, SwarmRepo, WorkspacesRepo,
};
use tokio::sync::broadcast;

use crate::http::ProductCtx;
use crate::run::{CancelRegistry, LensRunResult, SessionAppearance};

/// Host-application context required by the product runners.
pub trait ProductRunHost: ProductCtx {
    /// The daemon's event bus (`ProductChanged`, `Notice`, `PlanRun`, …).
    fn events(&self) -> &broadcast::Sender<Event>;
    /// Workspaces repo — the startup reaper re-resolves an orphan's workspace.
    fn workspaces(&self) -> &WorkspacesRepo;
    /// Analysis-agent id → cancel flag; a manual Stop trips it via
    /// [`crate::run::signal_cancel`].
    fn agent_cancels(&self) -> &CancelRegistry;
    /// The skill library (lens / writer / tests / plan skill bodies).
    fn context_library(&self) -> &otto_context::Library;

    /// Run `provider` as a live agent session in `cwd` with automatic recovery
    /// (fresh session per attempt, up to [`crate::run::MAX_AGENT_ATTEMPTS`]):
    /// inject `prompt` (which must already tell the agent to write its JSON to
    /// `out_path`) and wait for the result, a timeout, an exit or a stall.
    ///
    /// When `agent_id` is `Some`, the session id is persisted to that analysis
    /// agent row as soon as the session exists, its waiting↔running transitions
    /// are mirrored onto the row, and a cancel flag is registered in
    /// [`Self::agent_cancels`] so a manual Stop ends the run as `stopped`
    /// without another retry. `on_session` (plan flow) sees each new session id
    /// the moment it exists. `model` / `work` go into the session's meta
    /// (`--model` injection, usage attribution).
    #[allow(clippy::too_many_arguments)]
    fn run_agent_with_recovery(
        &self,
        ws: &Workspace,
        user_id: &Id,
        provider: &str,
        model: Option<&str>,
        work: Option<serde_json::Value>,
        cwd: &str,
        prompt: &str,
        out_path: &Path,
        timeout: Duration,
        agent_id: Option<&Id>,
        appearance: &SessionAppearance,
        on_session: Option<&(dyn Fn(&Id) + Send + Sync)>,
    ) -> impl Future<Output = LensRunResult> + Send;
}

/// Point-of-action budget gate result (the host's usage budgets): `blocked`
/// with an optional human reason.
#[derive(Debug, Clone, Default)]
pub struct BudgetGate {
    pub blocked: bool,
    pub reason: Option<String>,
}

/// Host-application context required by the story studio handlers — the
/// discovery chat, refinement threads, media/annotations and the
/// Product↔Swarm bridge and the analysis/rewrite/test/plan launchers
/// (`analysis`, `chat`, `refine`, `media`, `swarm`).
pub trait ProductStudioHost: ProductRunHost {
    /// The daemon data dir (attachment files, scratch working dirs).
    fn data_dir(&self) -> &Path;
    fn attachments(&self) -> &ProductAttachmentRepo;
    fn discovery_chat_repo(&self) -> &DiscoveryChatRepo;
    fn discovery_repo(&self) -> &ProductDiscoveryRepo;
    fn refinement_repo(&self) -> &ProductRefinementRepo;
    fn mockup_repo(&self) -> &ProductMockupRepo;
    fn canvas_repo(&self) -> &CanvasRepo;
    fn swarms(&self) -> &SwarmRepo;
    /// Headless planner runs (swarm task seeding).
    fn orchestrator(&self) -> &Arc<Orchestrator>;
    /// The daemon DB pool (global settings: the default provider).
    fn pool(&self) -> &DbPool;
    /// Skill self-improvement (test-case approval feeds it).
    fn improve_engine(&self) -> &Arc<ImprovementEngine>;
    /// Kill a live session (manual Stop of an analysis agent).
    fn kill_session(&self, sid: &Id) -> impl Future<Output = Result<()>> + Send;

    /// Resolve the provider for a turn: the explicit request, else the
    /// workspace default, else the global default.
    fn resolve_provider(
        &self,
        ws: Option<&Workspace>,
        requested: Option<&str>,
    ) -> impl Future<Output = Result<String>> + Send;

    /// The no-output idle trip for interactive chat turns.
    fn session_stuck_idle(&self) -> Duration;

    /// Run one agent turn as a real, resumable session: resume `existing` or
    /// create a session titled `title` in `cwd` on `provider` (with `meta`),
    /// send `prompt`, and return `(reply_text, session_id)`. Persist the id so
    /// the next turn resumes the SAME session.
    #[allow(clippy::too_many_arguments)]
    fn run_session_turn(
        &self,
        ws: &Workspace,
        user: &User,
        existing: Option<&Id>,
        title: &str,
        cwd: &str,
        provider: &str,
        meta: serde_json::Value,
        prompt: &str,
        stuck_after: Duration,
    ) -> impl Future<Output = Result<(String, Id)>> + Send;

    /// The point-of-action usage-budget gate (`provider` empty = any) a swarm
    /// start or an analysis/rewrite/test/plan run must pass.
    fn usage_budget(
        &self,
        workspace_id: &str,
        provider: &str,
    ) -> impl Future<Output = BudgetGate> + Send;
    /// Start (or restart) the swarm's coordinator loop.
    fn start_swarm_coordinator(&self, swarm_id: Id);
    /// Broadcast a swarm status change.
    fn emit_swarm_status(&self, workspace_id: &Id, swarm_id: &str, status: &str);
}
