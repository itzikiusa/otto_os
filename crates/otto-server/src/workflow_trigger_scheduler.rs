//! Workflow trigger scheduler: fires `schedule`-kind triggers on their cadence
//! and starts a workflow run in the background.
//!
//! Runs on one 60-second tick (a single
//! timer, woken early by `CancelSignal`). The cursor/eligibility claim and the
//! queued run are committed together, so a captured tick can never admit a run
//! after its trigger was retimed or disabled.
//!
//! The tick itself (and the spec keys it reads) lives in
//! [`otto_workflows::triggers::schedule`], driven through this daemon's
//! [`otto_workflows::WorkflowCtx`] impl — mirroring the event-trigger listener.
//! This module keeps the supervisor loop and its cancel signal.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use tracing::warn;

use crate::cancel_signal::CancelSignal;
use crate::state::ServerCtx;
pub use otto_workflows::triggers::schedule::{is_due, is_due_armed, is_due_since};
// The unit tests (`tests/unit/workflow_trigger_scheduler.rs`) reach these
// through `use super::*`.
#[cfg(test)]
use chrono::{DateTime, Utc};
#[cfg(test)]
use otto_state::{TriggersRepo, WorkflowsRepo};
#[cfg(test)]
use serde_json::{json, Value};

const SCAN: Duration = Duration::from_secs(60);

/// Start the scheduler supervisor task. Returns its cancel signal; `cancel()`
/// stops the loop at once (mirrors the swarm/insights/cli-update pattern).
pub fn start(ctx: ServerCtx) -> CancelSignal {
    let cancel = CancelSignal::new();
    tokio::spawn(supervise(ctx, cancel.clone()));
    cancel
}

async fn supervise(ctx: ServerCtx, cancel: CancelSignal) {
    loop {
        if cancel.is_cancelled() {
            return;
        }
        if let Err(e) = tick(&ctx, &cancel).await {
            warn!("workflow trigger scheduler tick: {e}");
        }
        // One timer per scan; cancel() wakes it (no 500 ms polling slices).
        if cancel.sleep(SCAN).await {
            return;
        }
    }
}

async fn tick(ctx: &ServerCtx, cancel: &CancelSignal) -> otto_core::Result<()> {
    otto_workflows::triggers::schedule::tick(ctx, &|| cancel.is_cancelled()).await
}

/// The scheduler's durable admission boundary (see
/// [`otto_workflows::triggers::schedule::admit_schedule_run`]).
#[cfg(test)]
async fn admit_schedule_run(
    ctx: &ServerCtx,
    trigger: &otto_state::WorkflowTrigger,
    workspace_id: &otto_core::Id,
    input: &Value,
    now: DateTime<Utc>,
) -> otto_core::Result<Option<otto_core::workflows::WorkflowRun>> {
    otto_workflows::triggers::schedule::admit_schedule_run(ctx, trigger, workspace_id, input, now)
        .await
}

/// Start the event-trigger listener task. Returns a cancel flag; set to `true`
/// to stop the loop. The listener lives in `otto_workflows::triggers`, driven
/// through this daemon's [`otto_workflows::WorkflowCtx`] impl.
pub fn spawn_workflow_event_trigger_listener(ctx: ServerCtx) -> Arc<AtomicBool> {
    otto_workflows::triggers::spawn_event_trigger_listener(ctx)
}

#[cfg(test)]
#[path = "../tests/unit/workflow_trigger_scheduler.rs"]
mod tests;
