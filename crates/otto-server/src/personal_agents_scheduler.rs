//! Personal-agents supervisor: a 60-second tick that fires every enabled
//! schedule of every enabled personal agent whose cadence is due.
//!
//! Same concurrency model as `scheduled_tasks_scheduler` (the `cli_update`
//! ordering): the tick claims a per-**schedule** in-flight guard FIRST; a busy
//! or not-due schedule is skipped **without advancing its cursor**, so the
//! occurrence is retried rather than lost. The engine advances the fired
//! schedule's `last_run_at` cursor only on run completion — each schedule has
//! its own cursor, so an agent's daily recap and its 15-minute needs-attention
//! check never race each other's cursor (they can, however, overlap in time;
//! the engine's process-wide semaphore bounds total concurrency). On startup we
//! **reap** `running` rows left by a previous daemon life.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};
use tracing::{info, warn};

use otto_state::PersonalAgentsRepo;

use crate::cadence;
use crate::cancel_signal::CancelSignal;
use crate::personal_agents_engine::run_agent;
use crate::state::ServerCtx;

const SCAN: Duration = Duration::from_secs(60);

/// Clears a schedule id from the in-flight set on drop, so the entry is
/// released even if `run_agent` panics. Poison-tolerant.
struct InFlightGuard {
    set: Arc<Mutex<HashSet<String>>>,
    id: String,
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.set
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.id);
    }
}

/// Start the supervisor. Returns its cancel signal; `cancel()` stops the loop at once
/// (mirrors the scheduled-tasks / swarm / cli-update schedulers).
pub fn start(ctx: ServerCtx) -> CancelSignal {
    let cancel = CancelSignal::new();
    tokio::spawn(supervise(ctx, cancel.clone()));
    cancel
}

async fn supervise(ctx: ServerCtx, cancel: CancelSignal) {
    let repo = PersonalAgentsRepo::new(ctx.pool.clone());
    match repo.reap_running().await {
        Ok(n) if n > 0 => info!("personal agents: reaped {n} interrupted run(s) on startup"),
        Ok(_) => {}
        Err(e) => warn!("personal agents: startup reap failed: {e}"),
    }
    let in_flight: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
    loop {
        if cancel.is_cancelled() {
            return;
        }
        if let Err(e) = tick(&ctx, &repo, &in_flight).await {
            warn!("personal agents scheduler tick: {e}");
        }
        // One timer per scan; cancel() wakes it (no 500 ms polling slices).
        if cancel.sleep(SCAN).await {
            return;
        }
    }
}

async fn tick(
    ctx: &ServerCtx,
    repo: &PersonalAgentsRepo,
    in_flight: &Arc<Mutex<HashSet<String>>>,
) -> otto_core::Result<()> {
    let now = Utc::now();
    for (schedule, agent) in repo.list_enabled_schedules().await? {
        // Claim the in-flight guard FIRST. If busy or not due → skip, leaving
        // the cursor untouched (the engine advances it only on completion).
        {
            let mut set = in_flight.lock().unwrap_or_else(|e| e.into_inner());
            if set.contains(&schedule.id) {
                continue;
            }
            let last = schedule.last_run_at.as_deref().and_then(parse_ts);
            let tz = cadence::task_tz(&schedule.timezone);
            // The creation time anchors a never-run cron (first-fire catch-up).
            let created = parse_ts(&schedule.created_at);
            if !cadence::is_due_since(&schedule.schedule, last, created, now, tz) {
                continue;
            }
            set.insert(schedule.id.clone());
        }

        info!(agent = %agent.id, schedule = %schedule.id, "personal agents: firing due schedule");
        let ctx2 = ctx.clone();
        let guard = InFlightGuard {
            set: Arc::clone(in_flight),
            id: schedule.id.clone(),
        };
        tokio::spawn(async move {
            // The guard clears the in-flight entry on drop — including on panic.
            let _guard = guard;
            let _ = run_agent(&ctx2, &agent, Some(&schedule), "schedule").await;
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
        assert!(parse_ts("2026-09-01T10:00:00+00:00").is_some());
        assert!(parse_ts("not-a-time").is_none());
    }
}
