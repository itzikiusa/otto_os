//! Otto automation engines: **Scheduled Tasks** (cadence, the 60 s supervisor,
//! the run engine + report delivery), the verification command runner, and
//! what older **Goal Loops** still need (goals are workflows now, goal.md).
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
pub mod goal_loop_history;
pub mod report_delivery;
pub mod scheduled_tasks_engine;
pub mod scheduled_tasks_scheduler;
pub mod verify_command;

pub use ctx::AutomationCtx;
