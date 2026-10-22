//! The `schedule`-kind trigger tick, behind [`WorkflowCtx`] (mirrors the
//! event-trigger listener next door). otto-server's
//! `workflow_trigger_scheduler` owns only the 60-second supervisor loop and its
//! `CancelSignal`; one pass over the enabled schedule triggers lives here.
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

use chrono::{DateTime, Utc};
use otto_state::{TriggersRepo, WorkflowsRepo};
use serde_json::{json, Value};
use tracing::{info, warn};

use super::copy_result_destinations;
use crate::WorkflowCtx;

/// One scheduler pass: admit and start every enabled schedule trigger that is
/// due at `now`. `cancelled` is polled between steps; once it returns true no
/// further run is admitted or started (an admitted-but-unstarted run stays a
/// durable pending row for `resume_queued_runs` on next boot).
pub async fn tick<C: WorkflowCtx>(
    ctx: &C,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> otto_core::Result<()> {
    if cancelled() {
        return Ok(());
    }
    let triggers_repo = TriggersRepo::new(ctx.pool().clone());
    let workflows_repo = WorkflowsRepo::new(ctx.pool().clone());
    let now = Utc::now();

    for trigger in triggers_repo.list_enabled_by_kind("schedule").await? {
        if cancelled() {
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
        let ws = match ctx.workspaces().get(&wf.workspace_id).await {
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

        let input = schedule_run_input(&trigger.spec);

        // Revalidate eligibility and the overlap guard while atomically claiming
        // the cursor and creating the run, then execute in a background task.
        if cancelled() {
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
        if cancelled() {
            return Ok(());
        }

        info!(
            workflow_id = %wf.id,
            run_id = %run.id,
            "workflow trigger scheduler: firing schedule trigger"
        );

        ctx.spawn_run(
            ws,
            wf,
            run.id.clone(),
            input,
            otto_core::workflows::RunScope::default(),
        );
    }
    Ok(())
}

/// The run input of a scheduled fire, shared by `create_run` and the spawned
/// run. `spec.prompt` (if set) threads a fixed prompt through to the engine's
/// `normalize_prompt`, same as a chat-started run; the trigger's `result_*`
/// destinations are copied too — `deliver_run_result` reads exactly these
/// keys, so a scheduled run can report to a chat/webhook instead of finishing
/// silently with no notification.
pub fn schedule_run_input(spec: &Value) -> Value {
    let mut input = serde_json::Map::new();
    input.insert("trigger".into(), json!("schedule"));
    if let Some(p) = spec
        .get("prompt")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
    {
        input.insert("prompt".into(), json!(p));
    }
    copy_result_destinations(spec, &mut input);
    Value::Object(input)
}

/// The scheduler's durable admission boundary, separated from background engine
/// execution so stale captured ticks can be exercised against the real database.
pub async fn admit_schedule_run<C: WorkflowCtx>(
    ctx: &C,
    trigger: &otto_state::WorkflowTrigger,
    workspace_id: &otto_core::Id,
    input: &Value,
    now: DateTime<Utc>,
) -> otto_core::Result<Option<otto_core::workflows::WorkflowRun>> {
    TriggersRepo::new(ctx.pool().clone())
        .claim_scheduled_run(trigger, workspace_id, input, &now.to_rfc3339())
        .await
}

/// True when a schedule-trigger spec is due to fire at `now`.
///
/// Delegates to the shared [`otto_core::cadence`] engine (the same one
/// Scheduled Tasks use) so workflow schedule triggers get **cron**
/// (`cadence:"cron"`, `expr`) and **IANA timezone** (`timezone`) parity for
/// free, while interval/daily/weekly behave exactly as before. The cursor
/// (`last_run`) is read from the spec.
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
    let last = otto_core::cadence::effective_cursor(spec, last, armed);
    let tz =
        otto_core::cadence::task_tz(spec.get("timezone").and_then(Value::as_str).unwrap_or(""));
    otto_core::cadence::is_due_since(spec, last, created, now, tz)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_input_carries_prompt_and_result_destinations_only() {
        let input = schedule_run_input(&json!({
            "cadence": "daily", "prompt": "summarise", "result_channel": "slack",
            "result_chat": "C1", "working_directory": "~/x",
        }));
        assert_eq!(input["trigger"], "schedule");
        assert_eq!(input["prompt"], "summarise");
        assert_eq!(input["result_chat"], "C1");
        assert!(input.get("working_directory").is_none());
        let blank = schedule_run_input(&json!({"prompt": "  "}));
        assert!(blank.get("prompt").is_none());
    }
}
