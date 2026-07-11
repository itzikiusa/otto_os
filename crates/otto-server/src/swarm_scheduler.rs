//! SwarmScheduler: wakes scheduled agents on their cadence and enqueues a
//! `kind=scheduled` run the agent executes with its standing directive (e.g. a
//! daily trend researcher, a periodic PM status report). Modeled on
//! `otto-improve::Scheduler`: 60s tick, responsive cancel slices, DB-cursor
//! idempotency (the agent's `schedule_json.last_run`).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use chrono::{DateTime, Datelike, TimeZone, Utc};
use otto_state::{AgentPatch, NewRun, RunPatch, TaskPatch};
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
    // Board-utilization watchdog rides the same 60s scan (its own 5-min gate).
    utilization_pass(ctx).await;
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

// --- Board-utilization watchdog (every 5 min per active swarm) --------------
//
// "The manager keeps everyone in line": every UTIL_EVERY the watchdog checks
// each ACTIVE swarm for wasted capacity (live runs below the parallel cap).
// The cheap structural fix runs first and costs no tokens — ready tasks stuck
// behind a busy/inactive assignee are reassigned to idle teammates, and the 5s
// coordinator tick dispatches them. Only when work exists but NOTHING is
// schedulable (everything blocked/in review) does it wake the MANAGER with a
// directive run — rate-limited to one per UTIL_ESCALATE_EVERY — so the check
// itself stays free and the LLM only runs when a human-shaped decision is due.

const UTIL_EVERY: Duration = Duration::from_secs(300);
const UTIL_ESCALATE_EVERY: Duration = Duration::from_secs(1800);

/// Per-swarm `(last_check, last_escalation)` watchdog cursors. In-memory: a
/// daemon restart simply re-checks early, which is harmless.
type UtilCursors = HashMap<String, (Option<Instant>, Option<Instant>)>;

fn util_cursor() -> &'static Mutex<UtilCursors> {
    static CUR: OnceLock<Mutex<UtilCursors>> = OnceLock::new();
    CUR.get_or_init(|| Mutex::new(HashMap::new()))
}

async fn utilization_pass(ctx: &ServerCtx) {
    // Active swarms = the ones with a live coordinator (start/resume register it).
    let swarm_ids: Vec<String> = ctx.swarm_coords.lock().unwrap().keys().cloned().collect();
    let now = Instant::now();
    for sid in swarm_ids {
        {
            let mut cur = util_cursor().lock().unwrap();
            let e = cur.entry(sid.clone()).or_insert((None, None));
            if e.0.is_some_and(|t| now.duration_since(t) < UTIL_EVERY) {
                continue;
            }
            e.0 = Some(now);
        }
        if let Err(e) = check_utilization(ctx, &sid).await {
            tracing::warn!(swarm = %sid, "swarm utilization check: {e}");
        }
    }
}

async fn check_utilization(ctx: &ServerCtx, sid: &str) -> otto_core::Result<()> {
    let repo = &ctx.swarm_repo;
    let swarm = repo.get_swarm(&sid.to_string()).await?;
    if swarm.status != "active" {
        return Ok(());
    }
    let cap = swarm
        .config
        .get("max_parallel_sessions")
        .and_then(|v| v.as_i64())
        .unwrap_or(4)
        .max(1);
    let active = repo.active_run_count(&swarm.id).await?;
    if active >= cap {
        return Ok(()); // fully utilized
    }

    let agents = repo.list_agents(&swarm.id).await?;
    let mut idle: Vec<otto_state::SwarmAgent> = Vec::new();
    for a in agents.iter().filter(|a| a.status == "active") {
        if !repo.agent_has_active_run(&a.id).await.unwrap_or(false)
            && !crate::swarm_verify::agent_under_verification(&a.id)
        {
            idle.push(a.clone());
        }
    }
    if idle.is_empty() {
        return Ok(()); // every active agent is already busy — cap is aspirational
    }

    // Structural rebalance (free): ready tasks whose assignee is busy or
    // inactive move to the best-fitting idle teammate.
    let ready = repo.ready_tasks(&swarm.id).await?;
    let mut slots = (cap - active).max(0) as usize;
    let mut moved: Vec<String> = Vec::new();
    for t in &ready {
        if slots == 0 || idle.is_empty() {
            break;
        }
        let assignee_is_idle = t
            .assignee_agent_id
            .as_ref()
            .is_some_and(|aid| idle.iter().any(|a| &a.id == aid));
        if assignee_is_idle {
            slots -= 1; // will be dispatched by the next coordinator tick as-is
            continue;
        }
        let hay = format!("{} {}", t.title, t.description).to_lowercase();
        let Some(pos) = idle
            .iter()
            .enumerate()
            .max_by_key(|(_, a)| crate::swarm_runtime::agent_fit_score(a, &hay))
            .map(|(i, _)| i)
        else {
            break;
        };
        let agent = idle.remove(pos);
        let _ = repo
            .update_task(
                &t.id,
                TaskPatch { assignee_agent_id: Some(Some(agent.id.clone())), ..Default::default() },
            )
            .await;
        crate::swarm_runtime::emit_task_pub(ctx, &t.id).await;
        moved.push(format!("“{}” → {}", t.title, agent.name));
        slots -= 1;
    }
    if !moved.is_empty() {
        crate::swarm_runtime::system_post_meta(
            ctx,
            &swarm.id,
            None,
            None,
            "status",
            &format!(
                "⚖️ Utilization check: {}/{} sessions busy — rebalanced {} ready task(s): {}.",
                active, cap, moved.len(), moved.join("; ")
            ),
            json!({ "event": "utilization_rebalance", "moved": moved.len() }),
        )
        .await;
        return Ok(()); // the coordinator tick will dispatch the moved work
    }

    // Nothing schedulable. If open work exists (blocked / stuck in review), wake
    // the manager to make the call — rate-limited so a stuck board doesn't burn
    // a manager turn every 5 minutes.
    if !ready.is_empty() {
        return Ok(()); // ready work is on idle agents; the tick handles it
    }
    let open = repo
        .list_tasks_for_swarm(&swarm.id)
        .await?
        .into_iter()
        .filter(|t| !matches!(t.status.as_str(), "done" | "cancelled"))
        .count();
    if open == 0 {
        return Ok(()); // board is simply finished
    }
    {
        let mut cur = util_cursor().lock().unwrap();
        let e = cur.entry(sid.to_string()).or_insert((None, None));
        if e.1.is_some_and(|t| Instant::now().duration_since(t) < UTIL_ESCALATE_EVERY) {
            return Ok(());
        }
        e.1 = Some(Instant::now());
    }
    // The manager: an ACTIVE agent someone reports to, itself idle right now.
    let Some(leader) = agents
        .iter()
        .find(|a| {
            a.status == "active"
                && agents.iter().any(|b| b.reports_to.as_deref() == Some(a.id.as_str()))
                && idle.iter().any(|i| i.id == a.id)
        })
        .cloned()
    else {
        return Ok(());
    };
    let mut run = repo
        .create_run(NewRun {
            swarm_id: swarm.id.clone(),
            workspace_id: swarm.workspace_id.clone(),
            project_id: None,
            task_id: None,
            agent_id: leader.id.clone(),
            kind: "scheduled".into(),
            trigger: "utilization".into(),
        })
        .await?;
    let directive = format!(
        "UTILIZATION CHECK — the board is under-utilized: {active}/{cap} sessions busy, 0 ready \
         tasks, {open} open task(s) stuck (blocked / in review / waiting). You are the manager: \
         use the otto MCP tools to fix it — `swarm_utilization` for the live picture, \
         `swarm_list_projects` + `swarm_list_tasks` to inspect, `swarm_update_task` to unblock, \
         reprioritize, reassign or close stale items, `swarm_create_task` for genuinely missing \
         work, `swarm_run_task` to dispatch, `swarm_stop_run` to kill a wedged run. Get the team \
         back to full capacity, then post a one-paragraph summary with `./otto-post`."
    );
    let _ = repo
        .update_run(&run.id, RunPatch { result: Some(Some(json!({ "directive": directive }))), ..Default::default() })
        .await;
    run.result = Some(json!({ "directive": directive }));
    swarm_run::emit_run(ctx, &run.id).await;
    crate::swarm_runtime::system_post_meta(
        ctx,
        &swarm.id,
        None,
        None,
        "status",
        &format!(
            "🕒 Utilization check: {active}/{cap} sessions busy with {open} open task(s) and \
             nothing schedulable — waking {} to triage.",
            leader.name
        ),
        json!({ "event": "utilization_escalation", "agent_id": leader.id }),
    )
    .await;
    let ctx2 = ctx.clone();
    tokio::spawn(async move {
        let _ = swarm_run::run_turn(ctx2, run).await;
    });
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
