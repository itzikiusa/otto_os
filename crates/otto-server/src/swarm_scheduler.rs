//! SwarmScheduler: wakes scheduled agents on their cadence and enqueues a
//! `kind=scheduled` run the agent executes with its standing directive (e.g. a
//! daily trend researcher, a periodic PM status report). Modeled on
//! `otto-improve::Scheduler`: 60s tick (one timer, woken by `CancelSignal`), DB-cursor
//! idempotency (the agent's `schedule_json.last_run`).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use otto_state::{NewRun, RunPatch, TaskPatch};
use serde_json::{json, Value};

use crate::cancel_signal::CancelSignal;
use crate::state::ServerCtx;
use crate::swarm_run;

const SCAN: Duration = Duration::from_secs(60);

/// Start the scheduler supervisor. Returns its cancel signal.
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
        if let Err(e) = tick(&ctx).await {
            tracing::warn!("swarm scheduler tick: {e}");
        }
        // One timer per scan; cancel() wakes it (no 500 ms polling slices).
        if cancel.sleep(SCAN).await {
            return;
        }
    }
}

async fn tick(ctx: &ServerCtx) -> otto_core::Result<()> {
    // Board-utilization watchdog rides the same 60s scan (its own 5-min gate).
    utilization_pass(ctx).await;
    let now = Utc::now();
    for agent in ctx.swarm_repo.list_scheduled_agents().await? {
        let Some(sched) = agent.schedule.clone() else {
            continue;
        };
        if !sched
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            continue;
        }
        if !is_due(&sched, Some(agent.created_at), now) {
            continue;
        }
        let _operation = crate::swarm_runtime::operation_guard(&agent.swarm_id).await;
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
        if ctx
            .swarm_repo
            .active_run_count(&swarm.id)
            .await
            .unwrap_or(0)
            >= cap
        {
            continue;
        }
        if ctx
            .swarm_repo
            .agent_has_active_run(&agent.id)
            .await
            .unwrap_or(false)
        {
            continue;
        }

        // Reservation + cursor advance commit together, guarded on the stored
        // schedule still being the one judged due (see reserve_scheduled_run).
        match ctx
            .swarm_repo
            .reserve_scheduled_run(
                NewRun {
                    swarm_id: swarm.id.clone(),
                    workspace_id: swarm.workspace_id.clone(),
                    project_id: None,
                    task_id: None,
                    agent_id: agent.id.clone(),
                    kind: "scheduled".into(),
                    trigger: "scheduled".into(),
                },
                &sched,
                &now.to_rfc3339(),
            )
            .await
        {
            Ok(None) => {}
            Ok(Some(run)) => {
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
    let _operation = crate::swarm_runtime::operation_guard(sid).await;
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
    // One busy-agents read, not a COUNT per agent (perf §15 F3).
    let busy = repo.busy_agents(&swarm.id).await.unwrap_or_default();
    let mut idle: Vec<otto_state::SwarmAgent> = Vec::new();
    for a in agents.iter().filter(|a| a.status == "active") {
        if !busy.contains(&a.id) && !crate::swarm_verify::agent_under_verification(&a.id) {
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
                TaskPatch {
                    assignee_agent_id: Some(Some(agent.id.clone())),
                    ..Default::default()
                },
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
                active,
                cap,
                moved.len(),
                moved.join("; ")
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
        if e.1
            .is_some_and(|t| Instant::now().duration_since(t) < UTIL_ESCALATE_EVERY)
        {
            return Ok(());
        }
        e.1 = Some(Instant::now());
    }
    // The manager: an ACTIVE agent someone reports to, itself idle right now.
    let Some(leader) = agents
        .iter()
        .find(|a| {
            a.status == "active"
                && agents
                    .iter()
                    .any(|b| b.reports_to.as_deref() == Some(a.id.as_str()))
                && idle.iter().any(|i| i.id == a.id)
        })
        .cloned()
    else {
        return Ok(());
    };
    let mut run = repo
        .reserve_run(
            NewRun {
                swarm_id: swarm.id.clone(),
                workspace_id: swarm.workspace_id.clone(),
                project_id: None,
                task_id: None,
                agent_id: leader.id.clone(),
                kind: "scheduled".into(),
                trigger: "utilization".into(),
            },
            false,
        )
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
        .update_run(
            &run.id,
            RunPatch {
                result: Some(Some(json!({ "directive": directive }))),
                ..Default::default()
            },
        )
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

fn parse_ts(v: Option<&Value>) -> Option<DateTime<Utc>> {
    v.and_then(Value::as_str)
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc))
}

/// Is a scheduled agent due to fire? Delegates to the shared
/// [`crate::cadence`] engine (Scheduled Tasks / workflow triggers), so agent
/// schedules get the same semantics: `at` in the schedule's IANA `timezone`
/// (UTC when absent — every pre-existing agent behaves as before), `cron`, and
/// an arm floor — the cursor is `max(last_run, armed_at)`, where `armed_at` is
/// stamped server-side when the schedule is created, resumed, or re-timed
/// (`otto_state::merge_agent_schedule`). Without it a new `daily 09:00` saved
/// at 15:00 fired at once. `created` anchors a never-run cron's first fire.
pub fn is_due(sched: &Value, created: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    let mut spec = sched.clone();
    // Swarm schedules historically defaulted a missing weekday to Tuesday (1).
    if spec.get("cadence").and_then(Value::as_str) == Some("weekly")
        && spec.get("weekday").is_none()
    {
        if let Some(o) = spec.as_object_mut() {
            o.insert("weekday".into(), json!(1));
        }
    }
    let last = crate::cadence::effective_cursor(
        &spec,
        parse_ts(spec.get("last_run")),
        parse_ts(spec.get("armed_at")),
    );
    let tz = crate::cadence::task_tz(spec.get("timezone").and_then(Value::as_str).unwrap_or(""));
    crate::cadence::is_due_since(&spec, last, created, now, tz)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn interval_due_when_never_run() {
        let s = json!({"cadence": "interval", "every_min": 30, "enabled": true});
        assert!(is_due(&s, None, Utc::now()));
    }

    #[test]
    fn interval_not_due_within_window() {
        let now = Utc::now();
        let s = json!({"cadence":"interval","every_min":60,"last_run": (now).to_rfc3339()});
        assert!(!is_due(&s, None, now));
    }

    #[test]
    fn daily_armed_after_today_slot_waits_for_tomorrow() {
        // Saved at 15:00 UTC with `at: 09:00`: the arm instant is past today's
        // slot, so it must not fire until tomorrow (the old UTC-only check
        // fired at once because there was no cursor).
        let now = Utc.with_ymd_and_hms(2026, 10, 5, 15, 0, 30).unwrap();
        let armed = Utc.with_ymd_and_hms(2026, 10, 5, 15, 0, 0).unwrap();
        let s = json!({"cadence":"daily","at":"09:00","enabled":true,
                       "armed_at": armed.to_rfc3339()});
        assert!(!is_due(&s, None, now));
        let tomorrow = Utc.with_ymd_and_hms(2026, 10, 6, 9, 0, 30).unwrap();
        assert!(is_due(&s, None, tomorrow));
    }

    #[test]
    fn daily_respects_schedule_timezone() {
        // 09:00 in Jerusalem (UTC+3 in October) is 06:00 UTC.
        let s = json!({"cadence":"daily","at":"09:00","timezone":"Asia/Jerusalem","enabled":true});
        let before = Utc.with_ymd_and_hms(2026, 10, 5, 5, 59, 0).unwrap();
        let after = Utc.with_ymd_and_hms(2026, 10, 5, 6, 0, 30).unwrap();
        assert!(!is_due(&s, None, before));
        assert!(is_due(&s, None, after));
    }

    #[test]
    fn edited_schedule_keeps_cursor_so_it_does_not_refire() {
        // Regression (finding 2): an agent edit rebuilt the schedule without
        // `last_run`, so the very next tick fired a duplicate run.
        let fired = Utc.with_ymd_and_hms(2026, 10, 5, 9, 0, 10).unwrap();
        let stored = json!({"cadence":"daily","at":"09:00","enabled":true,
                            "directive":"old","last_run": fired.to_rfc3339(),
                            "armed_at": "2026-10-01T00:00:00+00:00"});
        let edited = json!({"cadence":"daily","at":"09:00","enabled":true,"directive":"new"});
        let merged =
            otto_state::merge_agent_schedule(Some(&stored), edited, "2026-10-05T10:00:00+00:00");
        assert_eq!(merged["last_run"], stored["last_run"]);
        assert_eq!(
            merged["armed_at"], stored["armed_at"],
            "directive edit must not re-arm"
        );
        let next_tick = Utc.with_ymd_and_hms(2026, 10, 5, 10, 1, 0).unwrap();
        assert!(!is_due(&merged, None, next_tick));
    }
}
