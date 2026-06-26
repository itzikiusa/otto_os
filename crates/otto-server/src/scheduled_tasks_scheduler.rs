//! Scheduled-tasks supervisor: a 60-second tick that fires every enabled task
//! whose cadence is due, in the background.
//!
//! Concurrency model (the `cli_update` ordering, per the design review): the tick
//! claims a per-task **in-flight guard FIRST**; if a task is already running it is
//! skipped **without advancing the cursor**, so the occurrence is retried rather
//! than lost. The engine advances the `last_run_at` cursor only on run completion.
//! On startup we **reap** any `running` rows left by a previous daemon life
//! (the in-flight guard is in-memory and resets empty across restarts).

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};
use tracing::{info, warn};

use crate::cadence;
use crate::scheduled_tasks_engine::run_task;
use crate::state::ServerCtx;

const SCAN: Duration = Duration::from_secs(60);
const SLICE: Duration = Duration::from_millis(500);

/// Start the supervisor. Returns a cancel flag; set to `true` to stop the loop
/// (mirrors the swarm / workflow-trigger / cli-update schedulers).
pub fn start(ctx: ServerCtx) -> Arc<AtomicBool> {
    let cancel = Arc::new(AtomicBool::new(false));
    tokio::spawn(supervise(ctx, cancel.clone()));
    cancel
}

async fn supervise(ctx: ServerCtx, cancel: Arc<AtomicBool>) {
    match ctx.scheduled_tasks.reap_running().await {
        Ok(n) if n > 0 => info!("scheduled tasks: reaped {n} interrupted run(s) on startup"),
        Ok(_) => {}
        Err(e) => warn!("scheduled tasks: startup reap failed: {e}"),
    }
    let in_flight: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
    loop {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        if let Err(e) = tick(&ctx, &in_flight).await {
            warn!("scheduled tasks scheduler tick: {e}");
        }
        let mut waited = Duration::ZERO;
        while waited < SCAN {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            tokio::time::sleep(SLICE).await;
            waited += SLICE;
        }
    }
}

async fn tick(ctx: &ServerCtx, in_flight: &Arc<Mutex<HashSet<String>>>) -> otto_core::Result<()> {
    let now = Utc::now();
    for task in ctx.scheduled_tasks.list_enabled().await? {
        // Claim the in-flight guard FIRST. If busy or not due → skip, leaving the
        // cursor untouched (the engine advances it only on completion).
        {
            let mut set = in_flight.lock().unwrap();
            if set.contains(&task.id) {
                continue;
            }
            let last = task.last_run_at.as_deref().and_then(parse_ts);
            if !cadence::is_due(&task.schedule, last, now) {
                continue;
            }
            set.insert(task.id.clone());
        }

        info!(task = %task.id, "scheduled tasks: firing due task");
        let ctx2 = ctx.clone();
        let in_flight2 = Arc::clone(in_flight);
        let task_id = task.id.clone();
        tokio::spawn(async move {
            let _ = run_task(&ctx2, &task, "schedule").await;
            in_flight2.lock().unwrap().remove(&task_id);
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
