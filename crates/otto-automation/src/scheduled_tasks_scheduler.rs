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

use std::time::Duration;

use chrono::{DateTime, Utc};
use tracing::{debug, info, warn};

use crate::cadence;
use crate::cancel_signal::CancelSignal;
use crate::scheduled_tasks_engine::{in_flight, run_task};
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
pub async fn reap_interrupted(ctx: &impl AutomationCtx) {
    match ctx.scheduled_tasks().reap_running().await {
        Ok(n) if n > 0 => info!("scheduled tasks: reaped {n} interrupted run(s) on startup"),
        Ok(_) => {}
        Err(e) => warn!("scheduled tasks: startup reap failed: {e}"),
    }
}

async fn supervise(ctx: impl AutomationCtx, cancel: CancelSignal) {
    loop {
        if cancel.is_cancelled() {
            return;
        }
        if let Err(e) = tick(&ctx).await {
            warn!("scheduled tasks scheduler tick: {e}");
        }
        // One timer per scan; cancel() wakes it (no 500 ms polling slices).
        if cancel.sleep(SCAN).await {
            return;
        }
    }
}

async fn tick(ctx: &impl AutomationCtx) -> otto_core::Result<()> {
    let now = Utc::now();
    for task in ctx.scheduled_tasks().list_enabled().await? {
        // Not due → skip. Busy (a scheduled OR manual run in flight) → skip,
        // leaving the cursor untouched (the engine advances it only on
        // completion), so the occurrence is retried rather than lost.
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

        debug!(task = %task.id, "scheduled tasks: firing due task");
        let ctx2 = ctx.clone();
        tokio::spawn(async move {
            // The guard clears the in-flight entry on drop — including on panic.
            let _guard = guard;
            let _ = run_task(&ctx2, &task, "schedule").await;
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
}
