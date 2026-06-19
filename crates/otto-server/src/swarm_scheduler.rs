//! SwarmScheduler: wakes scheduled agents on their cadence and enqueues a
//! `kind=scheduled` run the agent executes with its standing directive (e.g. a
//! daily trend researcher, a periodic PM status report). Modeled on
//! `otto-improve::Scheduler`: 60s tick, responsive cancel slices, DB-cursor
//! idempotency (the agent's `schedule_json.last_run`).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Datelike, TimeZone, Utc};
use otto_state::{AgentPatch, NewRun};
use serde_json::{json, Value};

use crate::state::ServerCtx;
use crate::swarm_run;

const SCAN: Duration = Duration::from_secs(60);
const SLICE: Duration = Duration::from_millis(500);

/// Start the scheduler supervisor. Returns a cancel flag.
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
            tracing::warn!("swarm scheduler tick: {e}");
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
    let now = Utc::now();
    for agent in ctx.swarm_repo.list_scheduled_agents().await? {
        let Some(sched) = agent.schedule.clone() else { continue };
        if !sched.get("enabled").and_then(Value::as_bool).unwrap_or(false) {
            continue;
        }
        if !is_due(&sched, now) {
            continue;
        }
        // Swarm must be active and under its parallel cap; one turn per agent.
        let swarm = match ctx.swarm_repo.get_swarm(&agent.swarm_id).await {
            Ok(s) if s.status == "active" => s,
            _ => continue,
        };
        let cap = swarm
            .config
            .get("max_parallel_sessions")
            .and_then(|v| v.as_i64())
            .unwrap_or(4)
            .max(1);
        if ctx.swarm_repo.active_run_count(&swarm.id).await.unwrap_or(0) >= cap {
            continue;
        }
        if ctx.swarm_repo.agent_has_active_run(&agent.id).await.unwrap_or(false) {
            continue;
        }

        // Advance the cursor first (so a slow run can't double-fire next tick).
        let mut sched2 = sched.clone();
        if let Some(obj) = sched2.as_object_mut() {
            obj.insert("last_run".into(), json!(now.to_rfc3339()));
        }
        let _ = ctx
            .swarm_repo
            .update_agent(&agent.id, AgentPatch { schedule: Some(Some(sched2)), ..Default::default() })
            .await;

        match ctx
            .swarm_repo
            .create_run(NewRun {
                swarm_id: swarm.id.clone(),
                workspace_id: swarm.workspace_id.clone(),
                project_id: None,
                task_id: None,
                agent_id: agent.id.clone(),
                kind: "scheduled".into(),
                trigger: "scheduled".into(),
            })
            .await
        {
            Ok(run) => {
                swarm_run::emit_run(ctx, &run.id).await;
                let ctx2 = ctx.clone();
                tokio::spawn(async move {
                    let _ = swarm_run::run_turn(ctx2, run).await;
                });
            }
            Err(e) => tracing::warn!("swarm scheduler: create run: {e}"),
        }
    }
    Ok(())
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

/// Is a scheduled agent due to fire? Times are interpreted in UTC.
pub fn is_due(sched: &Value, now: DateTime<Utc>) -> bool {
    let last = sched
        .get("last_run")
        .and_then(Value::as_str)
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc));
    match sched.get("cadence").and_then(Value::as_str).unwrap_or("interval") {
        "interval" => {
            let every = sched.get("every_min").and_then(Value::as_i64).unwrap_or(60).max(1);
            match last {
                Some(l) => (now - l).num_minutes() >= every,
                None => true,
            }
        }
        "daily" => {
            let (h, m) = parse_hhmm(sched.get("at"));
            let target = Utc
                .with_ymd_and_hms(now.year(), now.month(), now.day(), h, m, 0)
                .single();
            match target {
                Some(t) => now >= t && last.is_none_or(|l| l < t),
                None => false,
            }
        }
        "weekly" => {
            let wd = sched.get("weekday").and_then(Value::as_i64).unwrap_or(1) as u32;
            if now.weekday().num_days_from_monday() != wd {
                return false;
            }
            let (h, m) = parse_hhmm(sched.get("at"));
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
        let s = json!({"cadence":"interval","every_min":60,"last_run": (now).to_rfc3339()});
        assert!(!is_due(&s, now));
    }
}
