//! The swarm runtime's view of its host (the daemon): [`SwarmHost`] lists
//! exactly what the Coordinator, runners, verifier, scheduler and channel
//! triggers need from the server — narrow handles (repos, the session manager,
//! the event bus, the two runtime registries) plus a handful of server-owned
//! behaviours (budget gate, Product seeding, the PTY drive helpers shared with
//! the review engine). otto-server implements it for `SwarmRt`; this crate
//! never depends on otto-server.
//!
//! [`SwarmRt`] is the cheap, clonable handle the runtime passes around (and
//! extracts as axum `State` — the host implements `FromRef` for it).

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use otto_core::auth::{BoxFuture, RoleChecker};
use otto_core::domain::Workspace;
use otto_core::event::Event;
use otto_core::secrets::SecretStore;
use otto_core::Id;
use otto_orchestrator::Orchestrator;
use otto_sessions::SessionManager;
use otto_state::{IntegrationsRepo, SwarmProject, SwarmRepo, WorkspacesRepo};
use tokio::sync::broadcast;

use super::engine::CoordinatorRegistry;
use super::run::CancelRegistry;

/// The agent-runner result types are `otto_agent_run`'s own — one home, no
/// boundary conversion in the host.
pub use otto_agent_run::agent_run::{FailReason, RunOutcome, WatchStatus};

/// Incremental reader of the last completed claude turn in a transcript, from
/// a byte offset on (the server's turn oracle). Blocking IO — poll it off the
/// runtime.
pub trait TurnTail: Send {
    /// Read what was appended since the last poll; the text of the newest
    /// completed turn so far, if any.
    fn poll_last_turn_text(&mut self) -> Option<String>;
}

/// Timing constants of the shared PTY drive protocol (owned by the server's
/// review-session helpers, so swarm and review inject prompts identically).
#[derive(Debug, Clone, Copy)]
pub struct PtyTimings {
    /// Pause between a bracketed paste and the Enter that submits it.
    pub paste_to_enter: Duration,
    /// How long to wait for an injected prompt to land in the transcript.
    pub prompt_land_wait: Duration,
    /// Quiet window after which a running agent is reported as waiting.
    pub waiting_idle: Duration,
}

/// Recovery-loop attempt: `attempt number → outcome`.
pub type AttemptFn<'a> = dyn FnMut(u32) -> BoxFuture<'a, RunOutcome> + Send + 'a;
/// Waiting/Resumed hook of a watch.
pub type StatusFn<'a> = dyn FnMut(WatchStatus) -> BoxFuture<'a, ()> + Send + 'a;

/// Everything the swarm runtime needs from the daemon.
#[async_trait]
pub trait SwarmHost: Send + Sync + 'static {
    // ---- handles ----------------------------------------------------------
    fn swarm_repo(&self) -> &SwarmRepo;
    fn manager(&self) -> &Arc<SessionManager>;
    fn events(&self) -> &broadcast::Sender<Event>;
    fn roles(&self) -> &Arc<dyn RoleChecker>;
    fn secrets(&self) -> &Arc<dyn SecretStore>;
    fn workspaces(&self) -> &WorkspacesRepo;
    fn integrations_store(&self) -> &IntegrationsRepo;
    fn orchestrator(&self) -> &Arc<Orchestrator>;
    fn context_library(&self) -> &otto_context::Library;
    fn data_dir(&self) -> &Path;
    /// swarm_id → live Coordinator handle.
    fn swarm_coords(&self) -> &CoordinatorRegistry;
    /// run_id → cancel flag of an in-flight swarm run.
    fn swarm_run_cancels(&self) -> &CancelRegistry;

    // ---- server-owned behaviour ------------------------------------------
    /// Agent-capable providers currently installed (order = preference).
    fn available_providers(&self) -> Vec<String>;
    /// The configured default provider (workspace → global → "claude").
    async fn resolve_provider_or_fallback(
        &self,
        ws: Option<&Workspace>,
        requested: Option<&str>,
        site: &'static str,
    ) -> String;
    /// Point-of-action budget gate: `Some(reason)` when the workspace's spend
    /// cap blocks starting work (reason may be `None` inside — "cap reached").
    async fn budget_blocked(&self, workspace_id: &str) -> Option<Option<String>>;
    /// `(input, output, cost_usd)` token totals of `session_id` since `since`;
    /// `None` when usage tracking has nothing (or is unavailable).
    async fn session_usage_totals(
        &self,
        session_id: &str,
        since: chrono::DateTime<chrono::Utc>,
    ) -> Option<(u64, u64, f64)>;
    /// Seed a channel-launched project's tasks from its goal (Product planner).
    async fn seed_tasks(&self, project: &SwarmProject, creator: &Id, goal: &str);

    // ---- PTY drive helpers (shared with the review engine) ---------------
    fn pty_timings(&self) -> PtyTimings;
    fn bracketed_paste(&self, text: &str) -> Vec<u8>;
    async fn wait_for_tui(&self, sid: &Id) -> bool;
    async fn dispatched(&self, sid: &Id, before: Option<Instant>) -> bool;
    /// Byte length of claude's transcript for (cwd, provider session id).
    fn transcript_len(&self, cwd: &str, psid: &str) -> u64;
    async fn claude_prompt_landed(&self, sid: &Id, cwd: &str, offset: u64, wait: Duration) -> bool;
    /// Reader of the claude turn written after byte `offset` of `path`.
    fn claude_turn_tail(&self, path: PathBuf, offset: u64) -> Box<dyn TurnTail>;
    /// Watch a prompted session for its result (out-file / accepted turn).
    #[allow(clippy::too_many_arguments)]
    async fn watch_for_result<'a>(
        &'a self,
        sid: &'a Id,
        provider: &'a str,
        provider_session_id: Option<&'a str>,
        cwd: &'a str,
        out_path: &'a Path,
        timeout: Duration,
        waiting_idle: Duration,
        stuck_idle: Duration,
        transcript_ok: Option<for<'s> fn(&'s str) -> bool>,
        on_status: &'a mut StatusFn<'a>,
    ) -> RunOutcome;
    /// Run `attempt` up to `max_attempts` times with auto-recovery (kill the
    /// prior session, back off; a set `cancel` flag stops with `Stopped`).
    async fn run_with_recovery<'a>(
        &'a self,
        max_attempts: u32,
        backoff: &'a [Duration],
        cancel: Option<&'a Arc<AtomicBool>>,
        attempt: &'a mut AttemptFn<'a>,
    ) -> RunOutcome;
}

/// Clonable handle to the host the runtime runs inside.
#[derive(Clone)]
pub struct SwarmRt(Arc<dyn SwarmHost>);

impl SwarmRt {
    pub fn new(host: Arc<dyn SwarmHost>) -> Self {
        Self(host)
    }
}

impl std::ops::Deref for SwarmRt {
    type Target = dyn SwarmHost;
    fn deref(&self) -> &Self::Target {
        &*self.0
    }
}

/// `spawn_blocking(f).await`, with a panic in `f` re-raised on the caller —
/// the server's `offload::blocking`, same telemetry span name.
pub(crate) async fn blocking<T, F>(f: F) -> T
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    match otto_telemetry::context::measure("server.blocking", tokio::task::spawn_blocking(f)).await
    {
        Ok(v) => v,
        Err(e) => match e.try_into_panic() {
            Ok(payload) => std::panic::resume_unwind(payload),
            Err(e) => panic!("blocking task cancelled: {e}"),
        },
    }
}
