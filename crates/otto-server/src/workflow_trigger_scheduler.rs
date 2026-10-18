//! Workflow trigger scheduler: fires `schedule`-kind triggers on their cadence
//! and starts a workflow run in the background.
//!
//! Modeled on [`otto_swarm::runtime::scheduler`]: one 60-second tick (a single
//! timer, woken early by `CancelSignal`). The cursor/eligibility claim and the
//! queued run are committed together, so a captured tick can never admit a run
//! after its trigger was retimed or disabled.
//!
//! Schedule spec keys (the shared [`otto_core::cadence`] format):
//!   `cadence`    — "interval" | "daily" | "weekly" | "cron" | "once"
//!                  (default "interval")
//!   `every_min`  — minutes between fires (cadence=interval; default 60)
//!   `at`         — "HH:MM" wall time in `timezone` (daily/weekly)
//!   `weekday`    — 0-6, Mon=0 (weekly only; default Monday)
//!   `expr`       — 5-field cron expression (cadence=cron)
//!   `run_at`     — the single fire instant (cadence=once)
//!   `timezone`   — IANA zone for `at`/`expr`/`run_at` (default UTC)
//!   `last_run`   — RFC-3339 timestamp of last fire (cursor; set by scheduler)
//!   `enabled`    — bool; missing/false → skip

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use otto_state::{TriggersRepo, WorkflowsRepo};
use serde_json::{json, Value};
use tracing::{info, warn};

use crate::cancel_signal::CancelSignal;
use crate::state::ServerCtx;
use otto_workflows::triggers::copy_result_destinations;

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
    if cancel.is_cancelled() {
        return Ok(());
    }
    let triggers_repo = TriggersRepo::new(ctx.pool.clone());
    let workflows_repo = WorkflowsRepo::new(ctx.pool.clone());
    let now = Utc::now();

    for trigger in triggers_repo.list_enabled_by_kind("schedule").await? {
        if cancel.is_cancelled() {
            return Ok(());
        }
        // The cursor never predates the arm instant (created / resumed /
        // re-timed), so a resumed trigger doesn't fire the run it missed
        // while off — the scheduled-tasks fix from PR #70.
        if !is_due_armed(
            &trigger.spec,
            Some(trigger.created_at),
            trigger.armed_at,
            now,
        ) {
            continue;
        }

        // Resolve the workflow; skip silently if it was deleted.
        let wf = match workflows_repo.get(&trigger.workflow_id).await {
            Ok(w) => w,
            Err(_) => continue,
        };
        let ws = match ctx.workspaces.get(&wf.workspace_id).await {
            Ok(w) => w,
            Err(_) => continue,
        };

        // Overlap guard: a run longer than the cadence must not stack a second
        // concurrent copy (each provisions its own worktrees). The cursor is
        // NOT advanced, so the missed tick fires once the run finishes.
        match workflows_repo.has_active_run(&wf.id).await {
            Ok(true) => {
                info!(workflow_id = %wf.id, "workflow scheduler: previous run still active — skipping tick");
                continue;
            }
            Ok(false) => {}
            Err(e) => {
                warn!(workflow_id = %wf.id, "workflow scheduler: active-run check: {e}");
                continue;
            }
        }

        // Build the run input once, shared by `create_run` and the spawned
        // `run_workflow` call. `spec.prompt` (if set) threads a fixed prompt
        // through to the engine's `normalize_prompt`, same as a chat-started run.
        let mut input = serde_json::Map::new();
        input.insert("trigger".into(), json!("schedule"));
        if let Some(p) = trigger
            .spec
            .get("prompt")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
        {
            input.insert("prompt".into(), json!(p));
        }
        // Result delivery: thread the trigger's result_* destinations into the
        // run input — `deliver_run_result` reads exactly these keys, so a
        // scheduled run can report to a chat/webhook instead of finishing
        // silently with no notification.
        copy_result_destinations(&trigger.spec, &mut input);
        let input = Value::Object(input);

        // Revalidate eligibility and the overlap guard while atomically claiming
        // the cursor and creating the run, then execute in a background task.
        if cancel.is_cancelled() {
            return Ok(());
        }
        let run = match admit_schedule_run(ctx, &trigger, &wf.workspace_id, &input, now).await {
            Ok(Some(run)) => run,
            Ok(None) => continue,
            Err(e) => {
                warn!(workflow_id = %wf.id, "workflow scheduler: admit run: {e}");
                continue;
            }
        };

        // Admission may have committed while shutdown arrived. Keep that
        // durable pending row for resume_queued_runs on next boot; never start
        // its side effects while dependencies are being torn down.
        if cancel.is_cancelled() {
            return Ok(());
        }

        info!(
            workflow_id = %wf.id,
            run_id = %run.id,
            "workflow trigger scheduler: firing schedule trigger"
        );

        crate::workflow_engine::spawn_run(
            ctx.clone(),
            ws,
            wf,
            run.id.clone(),
            input,
            otto_core::workflows::RunScope::default(),
            None,
        );
    }
    Ok(())
}

/// The scheduler's durable admission boundary, separated from background engine
/// execution so stale captured ticks can be exercised against the real database.
async fn admit_schedule_run(
    ctx: &ServerCtx,
    trigger: &otto_state::WorkflowTrigger,
    workspace_id: &otto_core::Id,
    input: &Value,
    now: DateTime<Utc>,
) -> otto_core::Result<Option<otto_core::workflows::WorkflowRun>> {
    TriggersRepo::new(ctx.pool.clone())
        .claim_scheduled_run(trigger, workspace_id, input, &now.to_rfc3339())
        .await
}

/// True when a schedule-trigger spec is due to fire at `now`.
///
/// Delegates to the shared [`crate::cadence`] engine (the same one Scheduled
/// Tasks use) so workflow schedule triggers get **cron** (`cadence:"cron"`,
/// `expr`) and **IANA timezone** (`timezone`) parity for free, while
/// interval/daily/weekly behave exactly as before. The cursor (`last_run`) is
/// read from the spec.
pub fn is_due(spec: &Value, now: DateTime<Utc>) -> bool {
    is_due_since(spec, None, now)
}

/// [`is_due`] anchored on the trigger's creation time: a never-run CRON only
/// looked one minute back, so its first fire was lost (for a whole day/week)
/// when the Mac slept or the daemon was down at that minute — the scheduled
/// tasks / personal agents ticks already anchor on creation.
pub fn is_due_since(spec: &Value, created: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    is_due_armed(spec, created, None, now)
}

/// [`is_due_since`] with the trigger's arm instant as a floor on the cursor
/// (see `WorkflowTrigger::armed_at`): `max(last_run, armed_at)`.
pub fn is_due_armed(
    spec: &Value,
    created: Option<DateTime<Utc>>,
    armed: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> bool {
    let last = spec
        .get("last_run")
        .and_then(Value::as_str)
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc));
    let last = crate::cadence::effective_cursor(spec, last, armed);
    let tz = crate::cadence::task_tz(spec.get("timezone").and_then(Value::as_str).unwrap_or(""));
    crate::cadence::is_due_since(spec, last, created, now, tz)
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
