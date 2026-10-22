//! SwarmScheduler: wakes scheduled agents on their cadence and enqueues a
//! `kind=scheduled` run the agent executes with its standing directive (e.g. a
//! daily trend researcher, a periodic PM status report). Modeled on
//! `otto-improve::Scheduler`: 60s tick (one timer, woken by `CancelSignal`), DB-cursor
//! idempotency (the agent's `schedule_json.last_run`).
//!
//! Shutdown: [`CancelSignal::cancel`] stops this scan from launching anything
//! further. The check is after each await and again immediately before
//! `tokio::spawn`, so a tick already waiting on the database does not start a
//! turn when stop wins that race.
//!
//! A scheduled occurrence reserved before that spawn is provably unstarted:
//! the row is still `queued` and has no session. [`run::emit_run`] only
//! broadcasts the row; it does not open a session. The scheduler stops that
//! row and rewinds only `schedule_json.last_run` when it still equals the
//! timestamp this reserve wrote. `once` uses that cursor as the already-fired
//! flag, so restoring its absence makes the one shot due again, and leaving
//! the timestamp consumes it. A recurring cadence (interval, daily, weekly,
//! cron) uses the cursor as the last served slot, so restoring the previous
//! value keeps this slot due, and leaving the new timestamp skips it until
//! the next one. A turn that reached `tokio::spawn` may have started: its
//! cursor stays, and this path does not replay the turn. Utilization reserves
//! have no cadence cursor; those queued rows stay for startup `fail_running`,
//! which stops them and does not rewind a schedule. Work already spawned
//! keeps running on its own handle.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use otto_state::{NewRun, RunPatch, TaskPatch};
use serde_json::{json, Value};

use crate::runtime::host::SwarmRt;
use crate::runtime::run;
use otto_core::cancel_signal::CancelSignal;

const SCAN: Duration = Duration::from_secs(60);

/// Start the scheduler supervisor. Returns its cancel signal.
pub fn start(ctx: SwarmRt) -> CancelSignal {
    let cancel = CancelSignal::new();
    tokio::spawn(supervise(ctx, cancel.clone()));
    cancel
}

async fn supervise(ctx: SwarmRt, cancel: CancelSignal) {
    loop {
        if cancel.is_cancelled() {
            return;
        }
        if let Err(e) = tick(&ctx, &cancel).await {
            tracing::warn!("swarm scheduler tick: {e}");
        }
        // One timer per scan; cancel() wakes it (no 500 ms polling slices).
        if cancel.sleep(SCAN).await {
            return;
        }
    }
}

/// Post-await / pre-spawn gate. False once the supervisor is cancelled.
fn launch_after_await(cancel: &CancelSignal) -> bool {
    otto_core::cancel_signal::supervisor_may_launch(cancel)
}

/// Stop a scheduled row that is still queued with no session, and rewind only
/// the cursor this reserve wrote. A false result means the turn may have
/// started or the cursor already moved; either way the turn is not spawned.
async fn release_unstarted_scheduled(
    ctx: &SwarmRt,
    run_id: &str,
    agent_id: &str,
    fired_at: &str,
    previous_last_run: Option<&str>,
) {
    match ctx
        .swarm_repo()
        .release_unstarted_scheduled_occurrence(run_id, agent_id, fired_at, previous_last_run)
        .await
    {
        Ok(true) => tracing::warn!(
            run = %run_id,
            "swarm scheduler: stop won before the turn started; unstarted occurrence released"
        ),
        Ok(false) => tracing::warn!(
            run = %run_id,
            "swarm scheduler: stop won after reserve; occurrence left because the turn may have started"
        ),
        Err(e) => tracing::warn!(
            run = %run_id,
            "swarm scheduler: release unstarted occurrence: {e}"
        ),
    }
}

async fn tick(ctx: &SwarmRt, cancel: &CancelSignal) -> otto_core::Result<()> {
    // Board-utilization watchdog rides the same 60s scan (its own 5-min gate).
    utilization_pass(ctx, cancel).await;
    if !launch_after_await(cancel) {
        return Ok(());
    }
    let now = Utc::now();
    let agents = ctx.swarm_repo().list_scheduled_agents().await?;
    // Stop can land while the agent list is in flight. Do not reserve or spawn.
    if !launch_after_await(cancel) {
        return Ok(());
    }
    for agent in agents {
        if !launch_after_await(cancel) {
            break;
        }
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
        let _operation = crate::runtime::engine::operation_guard(&agent.swarm_id).await;
        if !launch_after_await(cancel) {
            break;
        }
        let fired_at = now.to_rfc3339();
        let previous_last_run = sched
            .get("last_run")
            .and_then(Value::as_str)
            .map(str::to_string);
        // Swarm must be active, not held by a pause reason, and under its
        // parallel cap; one turn per agent. A verify/fix lock is not an
        // active run, so the busy check alone used to launch over it.
        let swarm = match ctx.swarm_repo().get_swarm(&agent.swarm_id).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(agent = %agent.id, "swarm scheduler: load swarm: {e}");
                continue;
            }
        };
        if !scheduled_turn_open(&swarm.status, swarm.pause_reason.as_deref(), false) {
            continue;
        }
        if let Some(reason) = ctx.budget_blocked(&swarm.workspace_id).await {
            let text = reason.unwrap_or_else(|| "workspace budget reached".to_string());
            crate::runtime::engine::pause_for_budget(ctx, &swarm, &text).await;
            continue;
        }
        if let Some(reason) = crate::runtime::engine::budget_exceeded(ctx, &swarm).await {
            crate::runtime::engine::pause_for_budget(ctx, &swarm, &reason).await;
            continue;
        }
        if !scheduled_turn_open(
            &swarm.status,
            swarm.pause_reason.as_deref(),
            crate::runtime::verify::agent_under_verification(&agent.id),
        ) {
            continue;
        }
        let cap = swarm
            .config
            .get("max_parallel_sessions")
            .and_then(|v| v.as_i64())
            .unwrap_or(4)
            .max(1);
        // A failed count used to read as zero and a failed busy-check as
        // idle, so a tick could open another run while the board was full.
        let active = match ctx.swarm_repo().active_run_count(&swarm.id).await {
            Ok(n) => n,
            Err(e) => {
                tracing::warn!(swarm = %swarm.id, "swarm scheduler: active run count: {e}");
                continue;
            }
        };
        if active >= cap {
            continue;
        }
        let busy = match ctx.swarm_repo().agent_has_active_run(&agent.id).await {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(agent = %agent.id, "swarm scheduler: active run check: {e}");
                continue;
            }
        };
        if !schedule_slot_open(Some(active), Some(busy), cap) {
            continue;
        }
        if !launch_after_await(cancel) {
            break;
        }

        // Reservation + cursor advance commit together, guarded on the stored
        // schedule still being the one judged due (see reserve_scheduled_run).
        match ctx
            .swarm_repo()
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
                &fired_at,
            )
            .await
        {
            Ok(None) => {}
            Ok(Some(run)) => {
                if !launch_after_await(cancel) {
                    // Reserved, not started. Give the occurrence back. Do not
                    // emit, and do not start the turn.
                    release_unstarted_scheduled(
                        ctx,
                        &run.id,
                        &agent.id,
                        &fired_at,
                        previous_last_run.as_deref(),
                    )
                    .await;
                    break;
                }
                run::emit_run(ctx, &run.id).await;
                if !launch_after_await(cancel) {
                    // The broadcast is not a session. The row is still queued.
                    release_unstarted_scheduled(
                        ctx,
                        &run.id,
                        &agent.id,
                        &fired_at,
                        previous_last_run.as_deref(),
                    )
                    .await;
                    break;
                }
                let ctx2 = ctx.clone();
                tokio::spawn(async move {
                    let _ = run::run_turn(ctx2, run).await;
                });
            }
            // An exhausted run budget (Conflict) is a steady state, not an
            // incident: it re-occurs every 60 s tick per due agent (S4-25a).
            Err(otto_core::Error::Conflict(m)) => {
                tracing::debug!(swarm = %swarm.id, agent = %agent.id, "swarm scheduler: not scheduled: {m}")
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

async fn utilization_pass(ctx: &SwarmRt, cancel: &CancelSignal) {
    // Active swarms = the ones with a live coordinator (start/resume register it).
    let swarm_ids: Vec<String> = ctx.swarm_coords().lock().unwrap().keys().cloned().collect();
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
        if !launch_after_await(cancel) {
            return;
        }
        if let Err(e) = check_utilization(ctx, &sid, cancel).await {
            tracing::warn!(swarm = %sid, "swarm utilization check: {e}");
        }
    }
}

async fn check_utilization(
    ctx: &SwarmRt,
    sid: &str,
    cancel: &CancelSignal,
) -> otto_core::Result<()> {
    let _operation = crate::runtime::engine::operation_guard(sid).await;
    if !launch_after_await(cancel) {
        return Ok(());
    }
    let repo = &ctx.swarm_repo();
    let swarm = repo.get_swarm(&sid.to_string()).await?;
    if !scheduled_turn_open(&swarm.status, swarm.pause_reason.as_deref(), false) {
        return Ok(());
    }
    if let Some(reason) = ctx.budget_blocked(&swarm.workspace_id).await {
        let text = reason.unwrap_or_else(|| "workspace budget reached".to_string());
        crate::runtime::engine::pause_for_budget(ctx, &swarm, &text).await;
        return Ok(());
    }
    if let Some(reason) = crate::runtime::engine::budget_exceeded(ctx, &swarm).await {
        crate::runtime::engine::pause_for_budget(ctx, &swarm, &reason).await;
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
    let idle = idle_agents(
        &agents,
        &busy,
        crate::runtime::verify::agent_under_verification,
    );
    if idle.is_empty() {
        return Ok(()); // every active agent is already busy — cap is aspirational
    }

    // Structural rebalance (free): ready tasks whose assignee is busy or
    // inactive move to the best-fitting idle teammate.
    let ready = repo.ready_tasks(&swarm.id).await?;
    let mut moved: Vec<String> = Vec::new();
    for (i, agent) in plan_rebalance(&ready, &idle, cap, active) {
        let t = &ready[i];
        let _ = repo
            .update_task(
                &t.id,
                TaskPatch {
                    assignee_agent_id: Some(Some(agent.id.clone())),
                    ..Default::default()
                },
            )
            .await;
        crate::runtime::engine::emit_task_pub(ctx, &t.id).await;
        moved.push(format!("“{}” → {}", t.title, agent.name));
    }
    if !moved.is_empty() {
        crate::runtime::engine::system_post_meta(
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
    if !take_escalation_slot(sid, Instant::now()) {
        return Ok(());
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
    if !launch_after_await(cancel) {
        return Ok(());
    }
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
    if !launch_after_await(cancel) {
        // Reserved. Utilization has no cadence cursor, so the row stays for
        // startup `fail_running`. Do not write a directive, emit, post, or
        // start the turn while shutdown is in progress.
        tracing::warn!(
            run = %run.id,
            "swarm scheduler: stop won after utilization reserve; turn not started"
        );
        return Ok(());
    }
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
    run::emit_run(ctx, &run.id).await;
    crate::runtime::engine::system_post_meta(
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
    if !launch_after_await(cancel) {
        // Reserved and announced. Do not start the turn. The supervisor
        // token does not mark this row stopped; shutdown settles running
        // rows on its own path.
        tracing::warn!(
            run = %run.id,
            "swarm scheduler: stop won after utilization reserve; turn not started"
        );
        return Ok(());
    }
    let ctx2 = ctx.clone();
    tokio::spawn(async move {
        let _ = run::run_turn(ctx2, run).await;
    });
    Ok(())
}

/// Active agents that are neither running a turn (`busy`) nor under
/// verification — the only ones the watchdog may hand work to or wake.
pub(crate) fn idle_agents(
    agents: &[otto_state::SwarmAgent],
    busy: &std::collections::HashSet<otto_core::Id>,
    under_verification: impl Fn(&str) -> bool,
) -> Vec<otto_state::SwarmAgent> {
    agents
        .iter()
        .filter(|a| a.status == "active" && !busy.contains(&a.id) && !under_verification(&a.id))
        .cloned()
        .collect()
}

/// The watchdog's structural rebalance, as `(index into ready, new assignee)`
/// moves. A ready task already on an idle agent keeps its assignee (and takes
/// a slot: the next tick dispatches it as-is); one on a busy or inactive agent
/// moves to the best-fitting idle teammate. At most `cap - active` slots, and
/// nothing at all when the board is at its cap.
pub(crate) fn plan_rebalance(
    ready: &[otto_state::SwarmTask],
    idle: &[otto_state::SwarmAgent],
    cap: i64,
    active: i64,
) -> Vec<(usize, otto_state::SwarmAgent)> {
    let mut idle = idle.to_vec();
    let mut slots = (cap - active).max(0) as usize;
    let mut moves = Vec::new();
    for (i, t) in ready.iter().enumerate() {
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
            .max_by_key(|(_, a)| crate::runtime::engine::agent_fit_score(a, &hay))
            .map(|(i, _)| i)
        else {
            break;
        };
        moves.push((i, idle.remove(pos)));
        slots -= 1;
    }
    moves
}

/// Claim swarm `sid`'s manager escalation: true at most once per
/// [`UTIL_ESCALATE_EVERY`], so a stuck board does not buy a paid manager turn
/// on every 5-minute check.
fn take_escalation_slot(sid: &str, now: Instant) -> bool {
    let mut cur = util_cursor().lock().unwrap_or_else(|e| e.into_inner());
    let e = cur.entry(sid.to_string()).or_insert((None, None));
    if e.1
        .is_some_and(|t| now.saturating_duration_since(t) < UTIL_ESCALATE_EVERY)
    {
        return false;
    }
    e.1 = Some(now);
    true
}

fn parse_ts(v: Option<&Value>) -> Option<DateTime<Utc>> {
    v.and_then(Value::as_str)
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc))
}

/// A scheduled or utilization turn needs an active swarm, no pause reason, and
/// an agent that is not held by verification. The busy-run check is separate.
/// `reserve_run_on` applies the same pause-reason rule in the admission
/// write: an active row with a nonempty `pause_reason` is refused, and
/// `allow_paused` admits only `paused` with an empty reason. Whitespace is
/// not empty, and the reason is not trimmed.
pub(crate) fn scheduled_turn_open(
    status: &str,
    pause_reason: Option<&str>,
    under_verification: bool,
) -> bool {
    status == "active"
        && pause_reason.filter(|reason| !reason.is_empty()).is_none()
        && !under_verification
}

/// Whether a due agent may be given a new scheduled run. `None` is a failed
/// read of the active-run count or the agent's busy flag, and stays closed.
pub(crate) fn schedule_slot_open(
    active_runs: Option<i64>,
    agent_busy: Option<bool>,
    cap: i64,
) -> bool {
    matches!(
        (active_runs, agent_busy),
        (Some(n), Some(false)) if n < cap
    )
}

/// Is a scheduled agent due to fire? Delegates to the shared
/// [`otto_core::cadence`] engine (Scheduled Tasks / workflow triggers), so agent
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
    let last = otto_core::cadence::effective_cursor(
        &spec,
        parse_ts(spec.get("last_run")),
        parse_ts(spec.get("armed_at")),
    );
    let tz =
        otto_core::cadence::task_tz(spec.get("timezone").and_then(Value::as_str).unwrap_or(""));
    otto_core::cadence::is_due_since(&spec, last, created, now, tz)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn agent(id: &str, status: &str) -> otto_state::SwarmAgent {
        otto_state::SwarmAgent {
            id: id.into(),
            swarm_id: "s".into(),
            workspace_id: "w".into(),
            name: id.into(),
            title: String::new(),
            reports_to: None,
            provider: "claude".into(),
            model: None,
            soul_name: None,
            soul_md: None,
            specialization: String::new(),
            scope_md: String::new(),
            skills: Value::Null,
            schedule: None,
            cwd_mode: None,
            avatar: String::new(),
            status: status.into(),
            order_idx: 0,
            created_by: "u".into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn task(id: &str, assignee: Option<&str>) -> otto_state::SwarmTask {
        otto_state::SwarmTask {
            id: id.into(),
            project_id: "p".into(),
            swarm_id: "s".into(),
            workspace_id: "w".into(),
            title: id.into(),
            description: String::new(),
            assignee_agent_id: assignee.map(Into::into),
            status: "todo".into(),
            priority: "normal".into(),
            parent_task_id: None,
            depends_on: Value::Null,
            labels: Value::Null,
            result_ref: None,
            delegated: false,
            attempts: 0,
            order_idx: 0,
            created_by: "u".into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    /// test-quality-3-01 (1): a ready task on a BUSY agent moves to the idle
    /// teammate; a busy, verifying or inactive agent is never a target.
    #[test]
    fn rebalance_moves_work_off_a_busy_agent_onto_an_idle_one() {
        let agents = [
            agent("busy", "active"),
            agent("idle", "active"),
            agent("verifying", "active"),
            agent("off", "inactive"),
        ];
        let busy: std::collections::HashSet<String> = ["busy".to_string()].into();
        let idle = idle_agents(&agents, &busy, |id| id == "verifying");
        assert_eq!(
            idle.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
            ["idle"]
        );
        let moves = plan_rebalance(&[task("t", Some("busy"))], &idle, 2, 1);
        assert_eq!(moves.len(), 1);
        assert_eq!((moves[0].0, moves[0].1.id.as_str()), (0, "idle"));
    }

    /// (2): a ready task already on an idle agent stays where it is.
    #[test]
    fn rebalance_leaves_a_task_on_an_idle_agent_alone() {
        let idle = [agent("a", "active"), agent("b", "active")];
        assert!(plan_rebalance(&[task("t", Some("a"))], &idle, 4, 0).is_empty());
        // ...and it takes a slot: with one slot left, the next task waits.
        let ready = [task("t", Some("a")), task("u", None)];
        assert!(plan_rebalance(&ready, &idle, 1, 0).is_empty());
    }

    /// (3): the manager escalation is rate-limited per swarm.
    #[test]
    fn escalation_is_rate_limited_per_swarm() {
        let sid = otto_core::new_id();
        let t0 = Instant::now();
        assert!(take_escalation_slot(&sid, t0));
        assert!(!take_escalation_slot(&sid, t0 + Duration::from_secs(300)));
        assert!(take_escalation_slot(&otto_core::new_id(), t0));
        assert!(take_escalation_slot(&sid, t0 + UTIL_ESCALATE_EVERY));
    }

    /// (4): a board at its cap changes nothing.
    #[test]
    fn rebalance_at_cap_moves_nothing() {
        let idle = [agent("idle", "active")];
        assert!(plan_rebalance(&[task("t", Some("busy"))], &idle, 2, 2).is_empty());
        assert!(plan_rebalance(&[task("t", Some("busy"))], &idle, 2, 3).is_empty());
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

    #[test]
    fn pause_reason_or_verification_closes_a_scheduled_turn() {
        assert!(scheduled_turn_open("active", None, false));
        assert!(scheduled_turn_open("active", Some(""), false));
        assert!(!scheduled_turn_open(
            "active",
            Some("cost budget reached"),
            false
        ));
        assert!(!scheduled_turn_open("paused", None, false));
        assert!(!scheduled_turn_open("active", None, true));
    }

    #[test]
    fn failed_cap_or_busy_read_does_not_open_a_scheduled_slot() {
        assert!(schedule_slot_open(Some(0), Some(false), 4));
        assert!(!schedule_slot_open(Some(4), Some(false), 4));
        assert!(!schedule_slot_open(Some(0), Some(true), 4));
        assert!(!schedule_slot_open(None, Some(false), 4));
        assert!(!schedule_slot_open(Some(0), None, 4));
    }

    /// `once` stores the fired flag in `last_run`: restoring its absence makes
    /// the shot due again, and leaving the timestamp consumes it. A recurring
    /// cadence stores the served slot: restoring the previous cursor keeps
    /// that slot due, and leaving the fired timestamp skips it.
    #[test]
    fn once_cursor_is_the_fired_flag_and_recurring_cursor_is_the_served_slot() {
        let now = Utc.with_ymd_and_hms(2026, 10, 5, 10, 0, 0).unwrap();
        let once = json!({
            "cadence": "once",
            "run_at": "2026-10-05T09:00:00+00:00",
            "enabled": true
        });
        assert!(is_due(&once, None, now));
        let consumed = json!({
            "cadence": "once",
            "run_at": "2026-10-05T09:00:00+00:00",
            "enabled": true,
            "last_run": "2026-10-05T09:00:01+00:00"
        });
        assert!(!is_due(&consumed, None, now));

        let previous = "2026-10-05T08:00:00+00:00";
        let interval = json!({
            "cadence": "interval",
            "every_min": 60,
            "enabled": true,
            "last_run": previous
        });
        assert!(is_due(&interval, None, now));
        let fired = json!({
            "cadence": "interval",
            "every_min": 60,
            "enabled": true,
            "last_run": "2026-10-05T10:00:00+00:00"
        });
        assert!(!is_due(&fired, None, now));
    }

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
