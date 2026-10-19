//! Scheduled-tasks supervisor: a 60-second tick that fires every enabled task
//! whose cadence is due, in the background.
//!
//! Concurrency model (the `cli_update` ordering, per the design review): the tick
//! claims a per-task **in-flight guard FIRST**; if a task is already running it is
//! skipped **without advancing the cursor**, so the occurrence is retried rather
//! than lost. The engine advances the `last_run_at` cursor only on run completion.
//! On startup the daemon awaits [`reap_interrupted`] (before serving) to reap
//! any `running` rows left by a previous daemon life (the in-flight guard is
//! in-memory and resets empty across restarts). The
//! guard set is the engine's process-wide [`in_flight`] set, shared with the
//! manual "Run now" path, so a due occurrence never starts on top of a manual
//! run that is still going.
//!
//! Shutdown: [`CancelSignal::cancel`] stops this scan from launching anything
//! further. The check is after each await and again immediately before
//! `tokio::spawn`, so a tick already waiting on the database does not fire
//! when stop wins that race. Admission inside the spawned task is checked
//! again: a `running` row committed during that await stays for
//! [`reap_interrupted`] and does not start execution. Work that already
//! passed the post-admit check is detached. This token does not stop it, and
//! a cancelled token is not a claim that those runs have finished.
//!
//! That stop does not consume the occurrence. Admission inserts the `running`
//! row and does not write `last_run_at`. The cursor moves only when the run
//! completes. Reap marks the row `error` ("interrupted by daemon restart")
//! and does not settle. For `once`, an empty cursor means the shot has not
//! fired, so it stays due. For a recurring cadence the cursor still names the
//! previous slot, so the same occurrence stays due once reap clears the
//! overlap. A skipped overlap does not settle `once` either. A workflow
//! hand-off (`workflow_run_id` set) is reattached, not reaped, and is not
//! replayed from here.

use std::time::Duration;

use chrono::{DateTime, Utc};
use tracing::{debug, info, warn};

use crate::cadence;
use crate::cancel_signal::CancelSignal;
use crate::scheduled_tasks_engine::{
    in_flight, resume_workflow_handoff, run_task_after_supervisor,
};
use crate::AutomationCtx;

const SCAN: Duration = Duration::from_secs(60);

/// Start the supervisor. Returns its cancel signal; `cancel()` stops the loop at once
/// (mirrors the swarm / workflow-trigger / cli-update schedulers).
pub fn start(ctx: impl AutomationCtx) -> CancelSignal {
    let cancel = CancelSignal::new();
    tokio::spawn(supervise(ctx, cancel.clone()));
    cancel
}

/// Startup reap: mark every run a previous daemon life left `running` as
/// interrupted (the in-flight guard is in-memory and resets across restarts).
/// The daemon AWAITS this before serving the router and before [`start`] — run
/// inside the spawned supervisor it raced the first manual "Run now" and could
/// mark that brand-new run interrupted.
///
/// A run that had handed off to a workflow is not reaped: the workflow
/// survives the restart, so its waiter is re-attached (in the background —
/// this stays a pair of DB statements on the boot path) and the run records
/// the workflow's real outcome (S3-303).
pub async fn reap_interrupted(ctx: &impl AutomationCtx) {
    match ctx.scheduled_tasks().reap_running().await {
        Ok(n) if n > 0 => info!("scheduled tasks: reaped {n} interrupted run(s) on startup"),
        Ok(_) => {}
        Err(e) => warn!("scheduled tasks: startup reap failed: {e}"),
    }
    match ctx.scheduled_tasks().list_running_workflow_handoffs().await {
        Ok(runs) => {
            for run in runs {
                resume_workflow_handoff(ctx, run);
            }
        }
        Err(e) => warn!("scheduled tasks: listing workflow hand-offs failed: {e}"),
    }
}

async fn supervise(ctx: impl AutomationCtx, cancel: CancelSignal) {
    loop {
        if cancel.is_cancelled() {
            return;
        }
        if let Err(e) = tick(&ctx, &cancel).await {
            warn!("scheduled tasks scheduler tick: {e}");
        }
        // One timer per scan; cancel() wakes it (no 500 ms polling slices).
        if cancel.sleep(SCAN).await {
            return;
        }
    }
}

/// Post-await / pre-spawn gate. False once the supervisor is cancelled.
fn launch_after_await(cancel: &CancelSignal) -> bool {
    crate::cancel_signal::supervisor_may_launch(cancel)
}

async fn tick(ctx: &impl AutomationCtx, cancel: &CancelSignal) -> otto_core::Result<()> {
    let now = Utc::now();
    let tasks = ctx.scheduled_tasks().list_enabled().await?;
    // Stop can land while `list_enabled` is in flight. Do not claim or spawn.
    if !launch_after_await(cancel) {
        return Ok(());
    }
    for task in tasks {
        if !launch_after_await(cancel) {
            break;
        }
        // Not due → skip. Busy (a scheduled OR manual run in flight) → skip,
        // leaving the cursor untouched (the engine advances it only on
        // completion, and then to the served occurrence, not the completion
        // clock), so the occurrence is retried rather than lost.
        // The cursor never predates the arm instant (created / resumed /
        // re-timed), so a resumed task doesn't fire what it missed while off.
        let last = cadence::effective_cursor(
            &task.schedule,
            task.last_run_at.as_deref().and_then(parse_ts),
            task.armed_at.as_deref().and_then(parse_ts),
        );
        let tz = cadence::task_tz(&task.timezone);
        // The creation time anchors a never-run cron, so its first fire is
        // caught up when the Mac slept / the daemon was down at that minute.
        let created = parse_ts(&task.created_at);
        if !cadence::is_due_since(&task.schedule, last, created, now, tz) {
            continue;
        }
        let Some(guard) = in_flight().claim(&task.id) else {
            continue;
        };
        // Re-check after the claim. A stop that arrived since the list await
        // drops the guard and does not spawn. The guard's drop is the release.
        if !launch_after_await(cancel) {
            drop(guard);
            break;
        }

        debug!(task = %task.id, "scheduled tasks: firing due task");
        let ctx2 = ctx.clone();
        let supervisor = cancel.clone();
        tokio::spawn(async move {
            // The guard clears the in-flight entry on drop — including on panic.
            let _guard = guard;
            let _ = run_task_after_supervisor(&ctx2, &task, &supervisor).await;
        });
    }
    Ok(())
}

fn parse_ts(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ts_roundtrips() {
        let s = "2026-06-26T10:00:00+00:00";
        assert!(parse_ts(s).is_some());
        assert!(parse_ts("not-a-time").is_none());
    }

    /// The scan is parked on its await. Stop is signaled before that await
    /// completes. The post-await check must not admit a launch. A worker that
    /// already started is still running afterwards.
    #[tokio::test]
    async fn awaiting_tick_does_not_launch_after_stop() {
        let cancel = CancelSignal::new();
        let (release_scan, scan_done) = tokio::sync::oneshot::channel::<()>();
        let (release_work, work_done) = tokio::sync::oneshot::channel::<()>();
        let scan = {
            let cancel = cancel.clone();
            tokio::spawn(async move {
                let _ = scan_done.await;
                launch_after_await(&cancel)
            })
        };
        let detached = tokio::spawn(async move { work_done.await.ok() });
        cancel.cancel();
        release_scan.send(()).unwrap();
        assert!(!scan.await.unwrap());
        assert!(!detached.is_finished());
        release_work.send(()).unwrap();
        detached.await.unwrap();
    }
}
