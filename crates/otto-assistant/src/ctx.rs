//! [`AssistantCtx`] — everything the Assistant and the Personal Agents engine
//! need from the daemon. `otto-server` implements it for `ServerCtx`; this
//! crate never names the server.
//!
//! Two kinds of members:
//! * **handles** — the shared services the moved code used as `ctx.<field>`
//!   (pool, event bus, session manager, memory, MCP approvals, …);
//! * **hooks** — daemon-owned helpers that belong to other subsystems
//!   (session driving / retries, report delivery, run notices, transcript
//!   resolution, cadence maths). They are thin delegations in the server; the
//!   pure ones are associated functions so unit tests can call them through
//!   the real implementation without building a context.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use otto_core::cancel_signal::CancelSignal;
use otto_core::domain::Session;
use otto_core::event::Event;
use otto_core::{Id, Result};
use otto_state::{DbPool, PersonalAgent, WorkspacesRepo};
use serde_json::Value;
use tokio::sync::broadcast;

use crate::personal_agents_engine::RunPlan;

/// Boxed `Send` future for the hook methods.
pub type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// One personal-agent run's visible-session attempt loop (see
/// [`AssistantCtx::run_agent_session`]).
pub struct AgentSessionRun<'a> {
    pub agent: &'a PersonalAgent,
    pub run_id: &'a str,
    /// The owning user the session opens under.
    pub owner: &'a str,
    pub cwd: &'a str,
    /// The fully wrapped prompt (user context + read-only preamble applied).
    pub prompt: &'a str,
    pub plan: &'a RunPlan,
    pub cancel: &'a CancelSignal,
}

/// What the attempt loop produced.
pub struct AgentSessionOutcome {
    /// The report file's contents (`None` when no attempt produced one).
    pub report: Option<String>,
    /// The last session the loop opened.
    pub session_id: Option<String>,
    /// Attempts made (≥ 1 once the loop ran).
    pub attempts: i64,
    /// `Some(reason)` when the loop errored (`FailReason::as_str`, or
    /// `"unknown"`).
    pub failure: Option<String>,
}

/// A notification-center notice for an unattended run's failure streak
/// (`run_notices::RunNotice`, severity Error).
pub struct RunFailureNotice {
    pub key: String,
    pub title: String,
    pub body: String,
    pub route: String,
    pub workspace_id: Option<String>,
    pub user_id: Option<String>,
}

/// Server-side context required by the Assistant + Personal Agents engines.
pub trait AssistantCtx: Clone + Send + Sync + 'static {
    // ---- handles -----------------------------------------------------------
    fn pool(&self) -> &DbPool;
    fn events(&self) -> &broadcast::Sender<Event>;
    /// Daemon data dir (`~/Library/Application Support/Otto`).
    fn data_dir(&self) -> &Path;
    fn manager(&self) -> &Arc<otto_sessions::SessionManager>;
    fn workspaces(&self) -> &WorkspacesRepo;
    fn mcp(&self) -> &Arc<otto_mcp::McpService>;
    fn orchestrator(&self) -> &Arc<otto_orchestrator::Orchestrator>;
    fn context_library(&self) -> &otto_context::Library;
    fn memory(&self) -> &Arc<otto_memory::MemoryService>;
    fn roles(&self) -> &Arc<dyn otto_core::auth::RoleChecker>;
    /// Persist a notification-center notice and push it live
    /// (`ServerCtx::notifications().create`).
    fn create_notice(&self, notice: otto_state::NewNotice) -> BoxFut<'_, Result<()>>;

    // ---- session hooks -----------------------------------------------------
    /// Paste `prompt` into a live agent TUI and press Enter, verifying the
    /// submit (`review_session::submit_prompt`). `false` = not delivered.
    fn submit_prompt<'a>(&'a self, sid: &'a Id, prompt: &'a str) -> BoxFut<'a, bool>;

    /// Run a personal agent's fresh visible session(s): create the session,
    /// paste the report-augmented prompt, watch for the report file, retry per
    /// `agent_run::run_with_recovery`. `Err` only for setup failures before
    /// the first attempt (e.g. the workspace is gone).
    fn run_agent_session<'a>(
        &'a self,
        run: AgentSessionRun<'a>,
    ) -> BoxFut<'a, Result<AgentSessionOutcome>>;

    /// The transcript file behind `session` (`routes::transcript`), or `None`
    /// when it is unavailable.
    fn session_transcript<'a>(
        &'a self,
        session: &'a Session,
    ) -> BoxFut<'a, Option<(otto_transcript::Provider, PathBuf)>>;

    /// Run blocking work on the measured blocking pool (`offload::blocking`).
    fn blocking<T, F>(f: F) -> impl Future<Output = T> + Send
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static;

    // ---- report delivery + notices ----------------------------------------
    /// Deliver a report to `destination` (`report_delivery::deliver_destination`).
    #[allow(clippy::too_many_arguments)]
    fn deliver_destination<'a>(
        &'a self,
        workspace_id: &'a str,
        owner: Option<&'a str>,
        name: &'a str,
        destination: &'a Value,
        summary: &'a str,
        report: &'a str,
    ) -> BoxFut<'a, (bool, Option<String>)>;

    /// Post a failure notice once per streak (`run_notices::notify_failure`).
    fn notify_run_failure(&self, notice: RunFailureNotice) -> BoxFut<'_, ()>;
    /// `run_notices::streak_key`.
    fn run_streak_key(kind: &str, id: &str) -> String;
    /// `run_notices::clear_streak`.
    fn clear_run_streak(key: &str);
    /// `report_delivery::extract_summary`.
    fn extract_summary(report: &str) -> String;
    /// `report_delivery::report_hash`.
    fn report_hash(report: &str) -> String;
    /// `report_delivery::write_report`.
    fn write_report<'a>(abs: &'a Path, report: &'a str) -> BoxFut<'a, Result<()>>;

    // ---- cadence (pure; `tz` is the IANA name, `cadence::task_tz` applied) --
    fn cadence_validate(spec: &Value) -> Result<()>;
    fn cadence_once_at(spec: &Value, tz: &str) -> Option<DateTime<Utc>>;
    fn cadence_is_due(
        spec: &Value,
        last_run: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
        tz: &str,
    ) -> bool;
    fn cadence_is_due_since(
        spec: &Value,
        last_run: Option<DateTime<Utc>>,
        created: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
        tz: &str,
    ) -> bool;
    fn cadence_effective_cursor(
        spec: &Value,
        last_run: Option<DateTime<Utc>>,
        armed: Option<DateTime<Utc>>,
    ) -> Option<DateTime<Utc>>;
    fn cadence_next_run(spec: &Value, from: DateTime<Utc>, tz: &str) -> Option<DateTime<Utc>>;
}
