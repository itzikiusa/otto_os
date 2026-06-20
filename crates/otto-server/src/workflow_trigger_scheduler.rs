//! Workflow trigger scheduler: fires `schedule`-kind triggers on their cadence
//! (interval / daily / weekly) and starts a workflow run in the background.
//!
//! Modeled on [`crate::swarm_scheduler`]: 60-second tick, 500ms cancel slices,
//! DB-cursor idempotency via `last_run` stored in the trigger's `spec_json`.
//!
//! Schedule spec keys (mirrors the swarm-scheduler format):
//!   `cadence`    — "interval" | "daily" | "weekly" (default "interval")
//!   `every_min`  — minutes between fires (cadence=interval; default 60)
//!   `at`         — "HH:MM" UTC wall time to fire (daily/weekly)
//!   `weekday`    — 0-6, Mon=0 (weekly only; default Monday)
//!   `last_run`   — RFC-3339 timestamp of last fire (cursor; set by scheduler)
//!   `enabled`    — bool; missing/false → skip

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Datelike, TimeZone, Utc};
use otto_state::{TriggersRepo, WorkflowsRepo};
use serde_json::{json, Value};
use tracing::{info, warn};

use crate::state::ServerCtx;

const SCAN: Duration = Duration::from_secs(60);
const SLICE: Duration = Duration::from_millis(500);

/// Start the scheduler supervisor task. Returns a cancel flag; set to `true`
/// to stop the loop (mirrors the swarm/insights/cli-update pattern).
pub fn start(ctx: ServerCtx) -> Arc<AtomicBool> {
    let cancel = Arc::new(AtomicBool::new(false));
    tokio::spawn(supervise(ctx, cancel.clone()));
    cancel
}

async fn supervise(ctx: ServerCtx, cancel: Arc<AtomicBool>) {
    loop {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        if let Err(e) = tick(&ctx).await {
            warn!("workflow trigger scheduler tick: {e}");
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

async fn tick(ctx: &ServerCtx) -> otto_core::Result<()> {
    let triggers_repo = TriggersRepo::new(ctx.pool.clone());
    let workflows_repo = WorkflowsRepo::new(ctx.pool.clone());
    let now = Utc::now();

    for trigger in triggers_repo.list_enabled_by_kind("schedule").await? {
        if !is_due(&trigger.spec, now) {
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

        // Advance the cursor first (idempotency: a slow/failing run can't
        // double-fire on the next tick).
        let mut spec2 = trigger.spec.clone();
        if let Some(obj) = spec2.as_object_mut() {
            obj.insert("last_run".into(), json!(now.to_rfc3339()));
        }
        if let Err(e) = triggers_repo.set_spec(&trigger.id, spec2).await {
            warn!(trigger_id = %trigger.id, "workflow scheduler: advance cursor: {e}");
            continue;
        }

        // Create the run row, then execute in a background task.
        let run = match workflows_repo
            .create_run(&wf.id, &wf.workspace_id, &json!({"trigger": "schedule"}))
            .await
        {
            Ok(r) => r,
            Err(e) => {
                warn!(workflow_id = %wf.id, "workflow scheduler: create run: {e}");
                continue;
            }
        };

        info!(
            workflow_id = %wf.id,
            run_id = %run.id,
            "workflow trigger scheduler: firing schedule trigger"
        );

        let ctx2 = ctx.clone();
        let run_id = run.id.clone();
        tokio::spawn(async move {
            crate::workflow_engine::run_workflow(
                ctx2, ws, wf, run_id,
                json!({"trigger": "schedule"}),
                None, false,
            )
            .await;
        });
    }
    Ok(())
}

/// True when a schedule-trigger spec is due to fire at `now` (UTC).
/// Mirrors `swarm_scheduler::is_due` exactly so the same spec format works
/// for both swarm agents and workflow triggers.
pub fn is_due(spec: &Value, now: DateTime<Utc>) -> bool {
    let last = spec
        .get("last_run")
        .and_then(Value::as_str)
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc));

    match spec.get("cadence").and_then(Value::as_str).unwrap_or("interval") {
        "interval" => {
            let every = spec
                .get("every_min")
                .and_then(Value::as_i64)
                .unwrap_or(60)
                .max(1);
            match last {
                Some(l) => (now - l).num_minutes() >= every,
                None => true,
            }
        }
        "daily" => {
            let (h, m) = parse_hhmm(spec.get("at"));
            let target = Utc
                .with_ymd_and_hms(now.year(), now.month(), now.day(), h, m, 0)
                .single();
            match target {
                Some(t) => now >= t && last.is_none_or(|l| l < t),
                None => false,
            }
        }
        "weekly" => {
            let wd = spec
                .get("weekday")
                .and_then(Value::as_i64)
                .unwrap_or(1) as u32;
            if now.weekday().num_days_from_monday() != wd {
                return false;
            }
            let (h, m) = parse_hhmm(spec.get("at"));
            let target = Utc
                .with_ymd_and_hms(now.year(), now.month(), now.day(), h, m, 0)
                .single();
            match target {
                Some(t) => now >= t && last.is_none_or(|l| l < t),
                None => false,
            }
        }
        _ => false,
    }
}

fn parse_hhmm(v: Option<&Value>) -> (u32, u32) {
    v.and_then(Value::as_str)
        .and_then(|s| {
            let mut it = s.split(':');
            let h = it.next()?.parse::<u32>().ok()?;
            let m = it.next().unwrap_or("0").parse::<u32>().ok()?;
            Some((h.min(23), m.min(59)))
        })
        .unwrap_or((9, 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_due_when_never_run() {
        let s = json!({"cadence": "interval", "every_min": 30, "enabled": true});
        assert!(is_due(&s, Utc::now()));
    }

    #[test]
    fn interval_not_due_within_window() {
        let now = Utc::now();
        let s = json!({"cadence":"interval","every_min":60,"last_run": now.to_rfc3339()});
        assert!(!is_due(&s, now));
    }

    #[test]
    fn interval_due_after_window() {
        let now = Utc::now();
        let past = now - chrono::Duration::minutes(90);
        let s = json!({"cadence":"interval","every_min":60,"last_run": past.to_rfc3339()});
        assert!(is_due(&s, now));
    }

    #[test]
    fn unknown_cadence_is_never_due() {
        let s = json!({"cadence": "monthly", "enabled": true});
        assert!(!is_due(&s, Utc::now()));
    }
}
