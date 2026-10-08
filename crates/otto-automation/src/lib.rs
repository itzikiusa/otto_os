//! Otto automation engines: **Scheduled Tasks** (cadence, the 60 s supervisor,
//! the run engine + report delivery) and **Goal Loops** (the bounded
//! Plan → Execute → Evaluate → Digest controller and its roles, verification
//! commands, worktree provisioning and pure completion policy).
//!
//! Everything daemon-side comes in through [`AutomationCtx`], which otto-server
//! implements for its `ServerCtx`; this crate never depends on otto-server.
//! Module names match their old `otto_server::*` paths, which otto-server
//! re-exports, so existing callers keep compiling unchanged.

pub mod agent;
pub mod cadence;
pub mod cancel_signal;
mod command_output;
mod ctx;
pub mod goal_loop;
pub mod goal_loop_commands;
pub mod goal_loop_parse;
pub mod goal_loop_policy;
pub mod goal_loop_roles;
pub mod goal_loop_workspace;
pub mod report_delivery;
pub mod scheduled_tasks_engine;
pub mod scheduled_tasks_scheduler;

pub use ctx::AutomationCtx;
