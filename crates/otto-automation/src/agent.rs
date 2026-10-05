//! The agent-run vocabulary the engines speak through [`crate::AutomationCtx`].
//!
//! These mirror otto-server's shared `agent_run` types (the PR-review runner the
//! scheduled-task and goal-loop executors reuse) value-for-value: the server's
//! `AutomationCtx` impl converts at the boundary, so this crate needs neither
//! the runner nor otto-server. Once the runner lives in its own crate these
//! become re-exports of it.

use otto_core::Id;

/// Why an agent run failed (`None` reason ⇒ success). Stable string forms feed
/// notifications and per-agent error notes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailReason {
    /// No output and no result for the stuck window — wedged.
    Stuck,
    /// Overall grace timeout elapsed AND the agent had gone quiet.
    Timeout,
    /// PTY exited before writing a result.
    Exited,
    /// Session vanished from the manager.
    SessionGone,
    /// Could not create the session at all.
    CreateFailed,
    /// Cancelled by a manual Stop (must NOT be auto-retried).
    Stopped,
    /// Gave up retrying: the work is already covered elsewhere.
    Superseded,
}

impl FailReason {
    pub fn as_str(self) -> &'static str {
        match self {
            FailReason::Stuck => "stuck",
            FailReason::Timeout => "timeout",
            FailReason::Exited => "exited",
            FailReason::SessionGone => "session-gone",
            FailReason::CreateFailed => "create-failed",
            FailReason::Stopped => "stopped",
            FailReason::Superseded => "superseded",
        }
    }
}

/// Outcome of one agent run (one attempt, or the final result after recovery).
pub struct RunOutcome {
    /// Result text the agent produced (its out-file, or an accepted claude turn).
    pub raw: Option<String>,
    /// The session id (so the agent stays openable; killed between retries).
    pub session_id: Option<Id>,
    /// `None` ⇒ success; `Some(_)` ⇒ failed with this reason.
    pub reason: Option<FailReason>,
}

impl RunOutcome {
    pub fn ok(raw: String, sid: Id) -> Self {
        Self {
            raw: Some(raw),
            session_id: Some(sid),
            reason: None,
        }
    }
    pub fn failed(sid: Option<Id>, reason: FailReason) -> Self {
        Self {
            raw: None,
            session_id: sid,
            reason: Some(reason),
        }
    }
    pub fn errored(&self) -> bool {
        self.reason.is_some()
    }
}

/// Idle-state transition emitted by [`crate::AutomationCtx::watch_for_result`]
/// so the caller can reflect it (flip an agent row to "waiting" / "running").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchStatus {
    /// Quiet past `waiting_idle` with no result yet — possibly blocked on input.
    Waiting,
    /// Output resumed after a `Waiting`.
    Resumed,
}

/// One managed, provider-aware role turn ([`crate::AutomationCtx::run_role_turn`]):
/// a visible session that completes when it writes a validated `done_file`.
pub struct RoleTurn<'a> {
    pub workspace: &'a otto_core::domain::Workspace,
    pub user: &'a otto_core::domain::User,
    /// Session title.
    pub title: &'a str,
    pub cwd: &'a str,
    pub provider: &'a str,
    pub meta: serde_json::Value,
    pub prompt: &'a str,
    /// Absolute deadline (also the session's stall trip).
    pub budget: std::time::Duration,
    /// Completion marker the prompt asks the agent to write.
    pub done_file: std::path::PathBuf,
    /// Only a file this accepts ends the turn.
    pub done_file_validator: fn(&str) -> bool,
}
