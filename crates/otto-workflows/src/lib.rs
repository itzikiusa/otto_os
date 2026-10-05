//! Workflow engine building blocks with no server dependency.
//!
//! Split out of the otto-server god-crate: everything here is pure (or talks
//! only to `otto-state`), so it compiles — and its tests run — without
//! rebuilding otto-server. The engine (`otto_server::workflow_engine`) and the
//! routes re-use these through `crate::workflow_*` aliases in otto-server.

pub mod catalog;
pub mod checkpoint;
pub mod context;
pub mod retry;
pub mod triggers;
pub mod validation;

use otto_core::domain::Workspace;
use otto_core::event::Event;
use otto_core::workflows::{RunScope, Workflow};
use otto_core::Id;
use otto_state::{DbPool, WorkspacesRepo};
use serde_json::Value;
use tokio::sync::broadcast;

/// What this crate needs from the daemon. Implemented by otto-server's
/// `ServerCtx`; derived from the `ctx.` uses of the moved code (the event
/// trigger listener) — grow it only as more of the engine moves here.
pub trait WorkflowCtx: Clone + Send + Sync + 'static {
    /// The daemon state DB (workflows, triggers, runs).
    fn pool(&self) -> &DbPool;
    /// The daemon event bus (subscribed to by the event-trigger listener).
    fn events(&self) -> &broadcast::Sender<Event>;
    /// Workspace lookups (a run executes in its workflow's workspace).
    fn workspaces(&self) -> &WorkspacesRepo;
    /// Drive an already-admitted run on a background task — the engine's
    /// `spawn_run` entry point (it still lives in otto-server).
    fn spawn_run(&self, ws: Workspace, wf: Workflow, run_id: Id, input: Value, scope: RunScope);
}
