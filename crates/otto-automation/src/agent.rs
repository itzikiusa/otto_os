//! The agent-run vocabulary the engines speak through [`crate::AutomationCtx`].
//!
//! The runner result types are `otto_agent_run`'s own (re-exported here so the
//! engines keep their `crate::agent::*` paths); one home, no boundary
//! conversion in the server's `AutomationCtx` impl.

pub use otto_agent_run::agent_run::{FailReason, RunOutcome, WatchStatus};

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
