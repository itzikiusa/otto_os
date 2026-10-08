//! The SwarmCoordinator runtime: a per-swarm supervisor that schedules ready
//! tasks onto agents within the parallel-worker cap, runs each turn via
//! `run::run_turn`, and routes the result (delegation → subtasks, handoffs,
//! reviews, concerns, completion). Plus the lifecycle (start/pause/abort/resume),
//! manual run/stop, and the recruiter/planner endpoints.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use axum::extract::{Path, State};
use axum::routing::{get, patch, post};
use axum::{Extension, Json, Router};
use chrono::Utc;
use otto_core::auth::AuthUser;
use otto_core::domain::WorkspaceRole;
use otto_core::event::Event;
use otto_core::{Error, Id};
use otto_state::swarm::NewTask;
use otto_state::{
    GoalPatch, NewGoal, NewRun, NewTrigger, RunPatch, Swarm, SwarmAgent, SwarmChannelTrigger,
    SwarmGoal, SwarmTask, TaskPatch, TriggerPatch,
};
use serde::Deserialize;
use serde_json::json;

use crate::http::{ApiErr as ApiError, ApiResult};
use crate::runtime::host::SwarmRt;
use crate::runtime::run::{self, SwarmTurnResult};
use otto_core::cancel_signal::CancelSignal;

/// Lifecycle and session-dispatch share this boundary. Weak entries avoid
/// retaining every swarm ever visited after its last operation finishes.
pub(crate) async fn operation_guard(swarm_id: &str) -> tokio::sync::OwnedMutexGuard<()> {
    type Gates = HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>;
    static GATES: OnceLock<Mutex<Gates>> = OnceLock::new();
    let gate = {
        let mut gates = GATES
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap();
        gates.retain(|_, gate| gate.strong_count() > 0);
        let gate = gates
            .get(swarm_id)
            .and_then(std::sync::Weak::upgrade)
            .unwrap_or_else(|| Arc::new(tokio::sync::Mutex::new(())));
        gates.insert(swarm_id.to_string(), Arc::downgrade(&gate));
        gate
    };
    gate.lock_owned().await
}

pub(crate) async fn run_is_active(repo: &otto_state::SwarmRepo, run_id: &str) -> bool {
    matches!(repo.get_run(&run_id.to_string()).await,
        Ok(run) if matches!(run.status.as_str(), "queued" | "running" | "waiting"))
}

/// Readiness can take minutes. Never hold the lifecycle boundary while waiting;
/// a stop interrupts the wait, and each following write rechecks under the gate.
pub(crate) async fn while_run_active<T>(
    repo: &otto_state::SwarmRepo,
    run_id: &str,
    future: impl std::future::Future<Output = T>,
) -> Option<T> {
    tokio::select! {
        value = future => Some(value),
        _ = async {
            loop {
                if !run_is_active(repo, run_id).await { break; }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        } => None,
    }
}

async fn guarded_dispatch(
    repo: &otto_state::SwarmRepo,
    swarm_id: &str,
    run_id: &str,
    send: impl std::future::Future<Output = bool>,
) -> bool {
    let _operation = operation_guard(swarm_id).await;
    if !run_is_active(repo, run_id).await {
        return false;
    }
    send.await
}

pub(crate) async fn send_run_input(
    ctx: &SwarmRt,
    swarm_id: &str,
    run_id: &str,
    session_id: &str,
    input: &[u8],
) -> bool {
    guarded_dispatch(ctx.swarm_repo(), swarm_id, run_id, async {
        ctx.manager()
            .input(&session_id.to_string(), input)
            .await
            .is_ok()
    })
    .await
}

#[cfg(test)]
mod dispatch_tests {
    use super::*;
    #[test]
    fn run_count_is_admission_limit_not_active_turn_stop() {
        let swarm: Swarm = serde_json::from_value(json!({
            "id":"s", "workspace_id":"w", "name":"test", "description":"",
            "status":"active", "config":{}, "max_total_runs":1,
            "max_attempts":3, "created_by":"u",
            "created_at":Utc::now(), "updated_at":Utc::now()
        }))
        .unwrap();
        let spend = otto_state::swarm::SwarmSpend {
            total_runs: 1,
            cost_usd: 0.0,
        };
        assert!(
            budget_reason(&swarm, &spend).is_none(),
            "admitting the last permitted run must not interrupt it"
        );
    }

    #[tokio::test]
    async fn stopping_during_readiness_does_not_wait_or_send_late_input() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("../otto-state/migrations")
            .run(&pool)
            .await
            .unwrap();
        let repo = otto_state::SwarmRepo::new(pool);
        let sid = "dispatch-stop-test".to_string();
        let run = repo
            .create_run(NewRun {
                swarm_id: sid.clone(),
                workspace_id: "ws".into(),
                project_id: None,
                task_id: None,
                agent_id: "agent".into(),
                kind: "task".into(),
                trigger: "coordinator".into(),
            })
            .await
            .unwrap();
        let waiting = while_run_active(&repo, &run.id, std::future::pending::<()>());
        let stop = async {
            let _operation = operation_guard(&sid).await;
            repo.stop_active_runs(&sid).await.unwrap();
        };
        let (result, ()) = tokio::time::timeout(Duration::from_secs(2), async {
            tokio::join!(waiting, stop)
        })
        .await
        .unwrap();
        assert!(
            result.is_none(),
            "stop interrupts readiness without waiting for the provider"
        );
        let sent = AtomicBool::new(false);
        assert!(
            !guarded_dispatch(&repo, &sid, &run.id, async {
                sent.store(true, Ordering::Relaxed);
                true
            })
            .await
        );
        assert!(
            !sent.load(Ordering::Relaxed),
            "a stopped run cannot send a prompt after readiness completes"
        );
    }
}

// --- Registry --------------------------------------------------------------

/// A running Coordinator's control handles.
#[derive(Clone)]
pub struct CoordinatorHandle {
    pub cancel: CancelSignal,
    pub paused: Arc<AtomicBool>,
}

impl CoordinatorHandle {
    pub fn new() -> Self {
        Self {
            cancel: CancelSignal::new(),
            paused: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl Default for CoordinatorHandle {
    fn default() -> Self {
        Self::new()
    }
}

/// swarm_id → live Coordinator handle.
pub type CoordinatorRegistry = Arc<Mutex<HashMap<String, CoordinatorHandle>>>;

pub fn new_registry() -> CoordinatorRegistry {
    Arc::new(Mutex::new(HashMap::new()))
}

/// "Stuck" window for a planner / recruiter turn — NOT a wall-clock cap. The
/// old 120–150s caps killed perfectly healthy turns: the claude cold-start (MCP
/// handshake + hook init) alone could eat them before reasoning began. Planning
/// and recruiting are one-time, quality-sensitive operations the operator is
/// happy to let run long, so there is no total limit (the orchestrator caps a
/// truly-wedged session at 1h). This is only how long the turn may make NO
/// progress — no transcript growth and no PTY activity — before it is deemed
/// stuck and retried (the orchestrator re-runs it, review-style).
const AGENT_NO_PROGRESS: Duration = Duration::from_secs(240);

// --- Coordinator -----------------------------------------------------------

/// Start (or restart) the Coordinator for a swarm. Idempotent: an existing
/// handle is cancelled first.
pub fn start_coordinator(ctx: SwarmRt, swarm_id: Id) {
    let handle = CoordinatorHandle::new();
    {
        let mut reg = ctx.swarm_coords().lock().unwrap();
        if let Some(old) = reg.insert(swarm_id.clone(), handle.clone()) {
            old.cancel.cancel();
        }
    }
    // Finish local routing of durable successes before dispatching anything.
    // Then restore verification controllers; their agent locks must exist before
    // the first coordinator tick can claim another task on those branches.
    crate::runtime::wake::ensure_listener(&ctx);
    tokio::spawn(async move {
        recover_completed_results(&ctx, &swarm_id).await;
        if handle.cancel.is_cancelled() {
            return;
        }
        crate::runtime::verify::recover(&ctx, &swarm_id).await;
        coordinator_loop(ctx, swarm_id, handle).await;
    });
}

/// Stop the Coordinator for a swarm (abort/shutdown).
pub fn stop_coordinator(ctx: &SwarmRt, swarm_id: &str) {
    if let Some(h) = ctx.swarm_coords().lock().unwrap().remove(swarm_id) {
        h.cancel.cancel();
    }
    // The run is over: drop its shared-file tracking (a restart re-detects).
    // Its wake bell goes with the loop: the cancelled loop returns and drops
    // its `swarm_wake::BellGuard` (bells exist only while a coordinator runs).
    crate::runtime::run::forget_swarm_files(swarm_id);
}

pub fn set_paused(ctx: &SwarmRt, swarm_id: &str, paused: bool) {
    if let Some(h) = ctx.swarm_coords().lock().unwrap().get(swarm_id) {
        h.paused.store(paused, Ordering::Relaxed);
    }
}

/// One tick at a time per swarm. `start_coordinator` spawns the new loop
/// while the old one may be mid-tick (it only reads `cancel` between ticks),
/// and two overlapping ticks could read the same `todo` task / free agent and
/// dispatch it twice.
fn tick_lock(swarm_id: &str) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    let mut map = LOCKS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // Prune locks no loop holds (strong count 1 = only this map) so deleted
    // swarms don't leave an entry behind forever. A tick in progress keeps its
    // clone, so the overlap guard is unaffected.
    map.retain(|id, l| id == swarm_id || Arc::strong_count(l) > 1);
    map.entry(swarm_id.to_string()).or_default().clone()
}

async fn coordinator_loop(ctx: SwarmRt, swarm_id: Id, handle: CoordinatorHandle) {
    // Hold the swarm's wake bell for the loop's life (registered before the
    // first tick so its events are kept; released on every return — perf N7).
    let bell = crate::runtime::wake::register(&swarm_id);
    loop {
        let ticked_at = std::time::Instant::now();
        if handle.cancel.is_cancelled() {
            return;
        }
        if !handle.paused.load(Ordering::Relaxed) {
            let lock = tick_lock(&swarm_id);
            let _ticking = lock.lock().await;
            // Replaced (or stopped) while waiting for the previous loop's tick.
            if handle.cancel.is_cancelled() {
                return;
            }
            if let Err(e) = tick(&ctx, &swarm_id).await {
                // The swarm was deleted (the delete route only drops rows): stop
                // this loop — it used to tick and warn every 5s until restart.
                if matches!(e, Error::NotFound(_))
                    && matches!(
                        ctx.swarm_repo().get_swarm(&swarm_id).await,
                        Err(Error::NotFound(_))
                    )
                {
                    tracing::info!(swarm = %swarm_id, "swarm deleted — stopping its coordinator");
                    let mut reg = ctx.swarm_coords().lock().unwrap();
                    if reg
                        .get(&swarm_id)
                        .is_some_and(|h| h.cancel.same(&handle.cancel))
                    {
                        reg.remove(&swarm_id);
                    }
                    return;
                }
                tracing::warn!(swarm = %swarm_id, "swarm coordinator tick: {e}");
            }
        }
        // Event-driven (perf W7): park until a swarm event rings this swarm's
        // bell (≥ MIN_GAP after the last tick) or the 60 s safety tick; was a
        // fixed 5 s poll. Stop/restart still wakes it at once.
        if crate::runtime::wake::wait(&handle.cancel, &bell, ticked_at).await {
            return;
        }
    }
}

// --- Why a ready task isn't starting (12-mcp W1) ---------------------------
//
// `tick` used to `continue` silently past a ready task, so the board showed a
// "To do" card that never started with no hint why. Each tick now records a
// short reason per ready-but-unstarted task, in memory (it is derived state,
// rebuilt every tick), served on `GET /swarm/swarms/{sid}/utilization` as
// `waiting`.

/// One waiting reason. `code` is stable (`no_agent_fit`, `agent_busy`,
/// `verifying`, `capacity`, `run_budget`); `detail` is the board's text;
/// `since` is when the task first waited for THIS code.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct WaitingReason {
    pub code: &'static str,
    pub detail: String,
    pub since: chrono::DateTime<Utc>,
}

type WaitingMap = HashMap<Id, HashMap<Id, WaitingReason>>;

fn waiting_registry() -> &'static Mutex<WaitingMap> {
    static WAITING: OnceLock<Mutex<WaitingMap>> = OnceLock::new();
    WAITING.get_or_init(Default::default)
}

/// Replace a swarm's waiting set with this tick's, keeping `since` for a task
/// whose reason code didn't change.
fn set_waiting(swarm_id: &Id, fresh: HashMap<Id, (&'static str, String)>) {
    let mut reg = waiting_registry().lock().unwrap_or_else(|p| p.into_inner());
    let now = Utc::now();
    let prev = reg.remove(swarm_id).unwrap_or_default();
    if fresh.is_empty() {
        return;
    }
    let next = fresh
        .into_iter()
        .map(|(task, (code, detail))| {
            let since = prev
                .get(&task)
                .filter(|p| p.code == code)
                .map_or(now, |p| p.since);
            (
                task,
                WaitingReason {
                    code,
                    detail,
                    since,
                },
            )
        })
        .collect();
    reg.insert(swarm_id.clone(), next);
}

/// The swarm's current waiting reasons, by task id.
pub fn waiting_for(swarm_id: &Id) -> HashMap<Id, WaitingReason> {
    waiting_registry()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(swarm_id)
        .cloned()
        .unwrap_or_default()
}

async fn tick(ctx: &SwarmRt, swarm_id: &Id) -> otto_core::Result<()> {
    let started = std::time::Instant::now();
    let mut waiting: HashMap<Id, (&'static str, String)> = HashMap::new();
    let r = tick_inner(ctx, swarm_id, &mut waiting).await;
    let n_waiting = waiting.len();
    set_waiting(swarm_id, waiting);
    // Perf §15 M1: per-tick cost is observable (`RUST_LOG=otto_swarm::runtime::engine=debug`).
    tracing::debug!(
        swarm = %swarm_id,
        elapsed_ms = started.elapsed().as_millis() as u64,
        waiting = n_waiting,
        "swarm tick"
    );
    r
}

async fn tick_inner(
    ctx: &SwarmRt,
    swarm_id: &Id,
    waiting: &mut HashMap<Id, (&'static str, String)>,
) -> otto_core::Result<()> {
    let _operation = operation_guard(swarm_id).await;
    let repo = &ctx.swarm_repo();
    let swarm = repo.get_swarm(swarm_id).await?;
    if swarm.status != "active" {
        return Ok(());
    }

    // Budget guardrails (D3/D8): before doing anything, check whether any
    // per-swarm budget is exhausted. If so, pause the swarm with a clear reason
    // instead of scheduling more work — the user must raise the budget + resume.
    // Spend is read once per tick (perf §15 F9) and shared by the budget gate
    // and the run-count projection below.
    let spend = if has_budget(&swarm) {
        Some(repo.swarm_spend(swarm_id).await?)
    } else {
        None
    };
    if let Some(reason) = spend.as_ref().and_then(|sp| budget_reason(&swarm, sp)) {
        pause_for_budget(ctx, &swarm, &reason).await;
        return Ok(());
    }

    let cap = swarm
        .config
        .get("max_parallel_sessions")
        .and_then(|v| v.as_i64())
        .unwrap_or(4)
        .max(1);
    let active = repo.active_run_count(swarm_id).await?;
    if let Some(reason) = spend.as_ref().and_then(|sp| run_count_reason(&swarm, sp)) {
        if active == 0 && repo.pending_task_results(swarm_id).await?.is_empty() {
            pause_for_budget(ctx, &swarm, &reason).await;
        }
        return Ok(());
    }
    let mut budget = (cap - active).max(0);
    let capacity_reason = || format!("All {active}/{cap} parallel slots are busy");
    if budget <= 0 {
        for t in repo.ready_tasks(swarm_id).await? {
            waiting.insert(t.id, ("capacity", capacity_reason()));
        }
        return Ok(());
    }

    // Project the run-count budget across this tick. `budget_exceeded` above
    // only checks the budget once, so without this a single tick could enqueue
    // up to `cap` runs and overshoot `max_total_runs` by nearly the concurrency
    // cap. Track the projected total as we schedule and stop when the next run
    // would reach the ceiling. (Cost can't be projected — per-run cost isn't
    // known until the turn completes — so the cost ceiling stays a tick-top gate.)
    let mut projected_total_runs: Option<i64> = swarm
        .max_total_runs
        .and(spend.as_ref().map(|sp| sp.total_runs));

    let ready = repo.ready_tasks(swarm_id).await?;
    if ready.is_empty() {
        return Ok(());
    }
    // The roster once per tick — `pick_agent`/`has_reports` used to re-list
    // every agent per ready task (backlog B6 / SE-10).
    let agents = repo.list_agents(&swarm.id).await.unwrap_or_default();
    // Busy agents once per tick (perf §15 F3), kept current as this tick
    // dispatches — not a COUNT query per ready task.
    let mut busy = repo.busy_agents(&swarm.id).await.unwrap_or_default();
    let mut ready = ready.into_iter();
    while let Some(task) = ready.next() {
        if budget <= 0 {
            // This task and every one after it wait for a free slot.
            let reason = format!("All {cap}/{cap} parallel slots are busy");
            for t in std::iter::once(task).chain(ready.by_ref()) {
                waiting.insert(t.id, ("capacity", reason.clone()));
            }
            break;
        }
        // Stop scheduling once the projected run count would hit the budget, so
        // a single tick can't overshoot `max_total_runs`.
        if let (Some(max_runs), Some(projected)) = (swarm.max_total_runs, projected_total_runs) {
            if projected >= max_runs {
                let reason = format!("Run budget reached ({max_runs} runs)");
                for t in std::iter::once(task).chain(ready.by_ref()) {
                    waiting.insert(t.id, ("run_budget", reason.clone()));
                }
                break;
            }
        }
        let Some(agent) = pick_agent_from(ctx, &agents, &task).await else {
            waiting.insert(
                task.id.clone(),
                ("no_agent_fit", "No available agent fits this task".into()),
            );
            continue;
        };
        if busy.contains(&agent.id) {
            // one turn per agent at a time
            waiting.insert(
                task.id.clone(),
                (
                    "agent_busy",
                    format!("{} is busy with another task", agent.name),
                ),
            );
            continue;
        }
        // Don't start another task for an agent whose branch is under verification —
        // a second turn on the same worktree would pollute the diff being verified
        // and the branch about to be merged (review B1).
        if crate::runtime::verify::agent_under_verification(&agent.id) {
            waiting.insert(
                task.id.clone(),
                (
                    "verifying",
                    format!("{}'s branch is being verified", agent.name),
                ),
            );
            continue;
        }
        // Claim: move the task to in_progress so it isn't re-selected next tick
        // — atomically (only from `todo`), so a racing tick / manual run that
        // already took it makes this one skip it. Persist the picked agent on a
        // previously-unassigned task — the board must always show WHO owns the
        // work, not an unassigned card mid-run.
        match repo.claim_task(&task.id, Some(&agent.id)).await {
            Ok(true) => {}
            Ok(false) => continue,
            Err(e) => {
                tracing::warn!(task = %task.id, "swarm: claim failed: {e}");
                continue;
            }
        }
        // Count this scheduled turn against the task's attempt ceiling. The
        // ceiling itself is enforced in `route_result` once the turn returns a
        // non-terminal status, so the work still happens this tick.

        let is_leader = has_reports_in(&agents, &agent.id);
        let kind = if is_leader && !task.delegated {
            "planning"
        } else {
            "task"
        };
        let run = match repo
            .reserve_run(
                NewRun {
                    swarm_id: swarm.id.clone(),
                    workspace_id: swarm.workspace_id.clone(),
                    project_id: Some(task.project_id.clone()),
                    task_id: Some(task.id.clone()),
                    agent_id: agent.id.clone(),
                    kind: kind.to_string(),
                    trigger: "coordinator".to_string(),
                },
                false,
            )
            .await
        {
            Ok(run) => run,
            Err(e) => {
                // Don't abort the whole tick over one failed enqueue: roll the
                // task back to `todo` (we just claimed it) so it isn't stranded
                // in_progress, and let the rest of the batch proceed.
                tracing::warn!(task = %task.id, error = %e, "swarm: create_run failed; reverting task to todo");
                let _ = repo
                    .update_task(
                        &task.id,
                        TaskPatch {
                            status: Some("todo".into()),
                            ..Default::default()
                        },
                    )
                    .await;
                emit_task(ctx, &task.id).await;
                continue;
            }
        };
        let _ = repo.bump_task_attempt(&task.id).await;
        emit_task(ctx, &task.id).await;
        budget -= 1;
        busy.insert(agent.id.clone());
        if let Some(projected) = projected_total_runs.as_mut() {
            *projected += 1;
        }
        run::emit_run(ctx, &run.id).await;

        let ctx2 = ctx.clone();
        let task2 = task.clone();
        tokio::spawn(async move {
            let result = run::run_turn(ctx2.clone(), run.clone()).await;
            route_result(&ctx2, &run, &task2, result).await;
        });
    }
    Ok(())
}

/// Check the per-swarm budgets against current spend/runs/wall-clock. Returns a
/// human-facing reason string when a budget is exhausted, else `None`. All
/// limits are nullable = unlimited. Spend/run-count counts every run ever
/// enqueued for the swarm; the runtime budget is measured from `run_started_at`
/// (the last time the swarm went active).
async fn budget_exceeded(ctx: &SwarmRt, swarm: &Swarm) -> Option<String> {
    if !has_budget(swarm) {
        return None;
    }
    let spend = ctx.swarm_repo().swarm_spend(&swarm.id).await.ok()?;
    budget_reason(swarm, &spend).or_else(|| run_count_reason(swarm, &spend))
}

/// Whether any per-swarm budget is set (all nullable = unlimited).
fn has_budget(swarm: &Swarm) -> bool {
    swarm.max_total_runs.is_some()
        || swarm.max_cost_usd.is_some()
        || swarm.max_runtime_secs.is_some()
}

/// [`budget_exceeded`] against an already-read spend.
fn budget_reason(swarm: &Swarm, spend: &otto_state::swarm::SwarmSpend) -> Option<String> {
    if let Some(max_cost) = swarm.max_cost_usd {
        if spend.cost_usd >= max_cost {
            return Some(format!(
                "cost budget reached (${:.2}/${:.2})",
                spend.cost_usd, max_cost
            ));
        }
    }
    if let Some(max_secs) = swarm.max_runtime_secs {
        if let Some(started) = swarm.run_started_at {
            let elapsed = (Utc::now() - started).num_seconds().max(0);
            if elapsed >= max_secs {
                return Some(format!(
                    "runtime budget reached ({}s/{}s)",
                    elapsed, max_secs
                ));
            }
        }
    }
    None
}

/// Count caps limit admission; already-admitted turns retain their time budget.
fn run_count_reason(swarm: &Swarm, spend: &otto_state::swarm::SwarmSpend) -> Option<String> {
    if let Some(max_runs) = swarm.max_total_runs {
        if spend.total_runs >= max_runs {
            return Some(format!(
                "run budget reached ({}/{} runs)",
                spend.total_runs, max_runs
            ));
        }
    }
    None
}

/// Pause a swarm because a budget was hit: persist status+reason, flip the
/// coordinator's paused flag (so it idles without ticking), suspend idle swarm
/// sessions, post to the board, and notify.
async fn pause_for_budget(ctx: &SwarmRt, swarm: &Swarm, reason: &str) {
    let _ = ctx
        .swarm_repo()
        .pause_swarm_with_reason(&swarm.id, reason)
        .await;
    set_paused(ctx, &swarm.id, true);
    crate::runtime::verify::stop_swarm(ctx, &swarm.id).await;
    stop_runs_for_pause(ctx, &swarm.id).await;
    for s in swarm_session_ids(ctx, &swarm.workspace_id, &swarm.id).await {
        let _ = ctx.manager().suspend(&s).await;
    }
    emit_status(ctx, &swarm.workspace_id, &swarm.id, "paused");
    system_post(
        ctx,
        &swarm.id,
        None,
        None,
        "system",
        &format!("Swarm paused — {reason}. Raise the budget and resume to continue."),
    )
    .await;
    let _ = ctx.events().send(Event::Notice {
        level: "warn".into(),
        title: "Swarm paused (budget)".into(),
        body: format!("“{}”: {reason}", swarm.name),
    });
}

/// `swarm_runs.error` of a turn cut short by a swarm pause (vs. an operator
/// Stop): its task goes back to `todo` for the resume, attempt refunded.
pub(crate) const PAUSED_RUN_REASON: &str = "paused";

/// Cut a pausing swarm's in-flight turns short: mark them `stopped`
/// ([`PAUSED_RUN_REASON`]) and trip their cancel flags BEFORE the sessions are
/// suspended. Suspending alone killed the PTY mid-turn; the watch saw
/// `SessionGone`, the retry loop killed the (resumable) session, spawned a
/// fresh one and re-sent the whole brief — spending on while the swarm showed
/// "paused" (the budget auto-pause included), and burning an attempt each time.
async fn stop_runs_for_pause(ctx: &SwarmRt, swarm_id: &str) {
    match ctx
        .swarm_repo()
        .stop_active_runs_with_reason(&swarm_id.to_string(), PAUSED_RUN_REASON)
        .await
    {
        Ok(ids) => {
            for rid in &ids {
                run::signal_cancel(ctx.swarm_run_cancels(), rid);
                run::emit_run(ctx, rid).await;
            }
        }
        Err(e) => tracing::warn!(swarm = %swarm_id, "pause: stopping in-flight runs: {e}"),
    }
}

/// Keyword-overlap score of an agent's title+specialization against task text.
pub(crate) fn agent_fit_score(a: &SwarmAgent, hay: &str) -> i32 {
    let mut s = 0;
    for tok in format!("{} {}", a.title, a.specialization)
        .to_lowercase()
        .split_whitespace()
    {
        if tok.len() >= 4 && hay.contains(tok) {
            s += 1;
        }
    }
    s
}

/// Best-fit ACTIVE agent for free task text, else any active agent. Creation-
/// time fallback so a task whose `assignee_title` didn't resolve still lands
/// ASSIGNED — unassigned board items are a bug, not a state.
async fn best_fit_agent_id(ctx: &SwarmRt, swarm_id: &str, hay: &str) -> Option<Id> {
    let agents = ctx
        .swarm_repo()
        .list_agents(&swarm_id.to_string())
        .await
        .ok()?;
    let active: Vec<SwarmAgent> = agents
        .into_iter()
        .filter(|a| a.status == "active")
        .collect();
    let hay = hay.to_lowercase();
    active
        .iter()
        .max_by_key(|a| agent_fit_score(a, &hay))
        .or_else(|| active.first())
        .map(|a| a.id.clone())
}

/// Pick the agent to run a task: the explicit assignee, else best-fit by title/
/// specialization keyword overlap, else any active agent.
async fn pick_agent(ctx: &SwarmRt, swarm: &Swarm, task: &SwarmTask) -> Option<SwarmAgent> {
    let agents = ctx.swarm_repo().list_agents(&swarm.id).await.ok()?;
    pick_agent_from(ctx, &agents, task).await
}

/// [`pick_agent`] over an already-loaded roster (the coordinator tick loads
/// it once). An assignee outside the roster (another swarm's — or another
/// workspace's — agent, S4-05) is ignored and the task is best-fit picked.
async fn pick_agent_from(
    _ctx: &SwarmRt,
    agents: &[SwarmAgent],
    task: &SwarmTask,
) -> Option<SwarmAgent> {
    if let Some(a) = on_roster_assignee(agents, task) {
        return Some(a.clone());
    }
    let active: Vec<&SwarmAgent> = agents.iter().filter(|a| a.status == "active").collect();
    if active.is_empty() {
        return None;
    }
    let hay = format!("{} {}", task.title, task.description).to_lowercase();
    active
        .iter()
        .copied()
        .max_by_key(|a| agent_fit_score(a, &hay))
        .or_else(|| active.first().copied())
        .cloned()
}

/// The task's explicit assignee when it is an ACTIVE agent of this roster.
fn on_roster_assignee<'a>(agents: &'a [SwarmAgent], task: &SwarmTask) -> Option<&'a SwarmAgent> {
    let aid = task.assignee_agent_id.as_ref()?;
    agents
        .iter()
        .find(|a| &a.id == aid && a.swarm_id == task.swarm_id && a.status == "active")
}

async fn has_reports(ctx: &SwarmRt, swarm_id: &str, agent_id: &str) -> bool {
    ctx.swarm_repo()
        .list_agents(&swarm_id.to_string())
        .await
        .map(|all| has_reports_in(&all, agent_id))
        .unwrap_or(false)
}

fn has_reports_in(agents: &[SwarmAgent], agent_id: &str) -> bool {
    agents
        .iter()
        .any(|a| a.reports_to.as_deref() == Some(agent_id))
}

async fn resolve_agent_by_title(ctx: &SwarmRt, swarm_id: &str, title: &str) -> Option<Id> {
    let want = title.trim().to_lowercase();
    let agents = ctx
        .swarm_repo()
        .list_agents(&swarm_id.to_string())
        .await
        .ok()?;
    agents
        .iter()
        .find(|a| a.title.to_lowercase() == want)
        .or_else(|| {
            agents.iter().find(|a| {
                a.title.to_lowercase().contains(&want) || want.contains(&a.title.to_lowercase())
            })
        })
        .map(|a| a.id.clone())
}

/// Apply a finished turn's result: delegation → subtasks, handoffs, reviews,
/// concerns, completion (and parent roll-up).
/// Re-enter result application, not agent execution, after a crash between
/// saving a successful turn and updating its task/children.
async fn recover_completed_results(ctx: &SwarmRt, swarm_id: &Id) {
    let pending = match ctx.swarm_repo().pending_task_results(swarm_id).await {
        Ok(runs) => runs,
        Err(error) => {
            tracing::warn!(swarm = %swarm_id, "recover swarm results: {error}");
            return;
        }
    };
    for run in pending {
        let Some(task_id) = &run.task_id else {
            continue;
        };
        let Ok(task) = ctx.swarm_repo().get_task(task_id).await else {
            continue;
        };
        let result = run
            .result
            .as_ref()
            .and_then(|value| run::parse_turn_result(&value.to_string()));
        if result.is_some() {
            route_result(ctx, &run, &task, result).await;
        }
    }
}

async fn route_result(
    ctx: &SwarmRt,
    run: &otto_state::SwarmRun,
    task: &SwarmTask,
    result: Option<SwarmTurnResult>,
) {
    let _operation = operation_guard(&task.swarm_id).await;
    if ctx
        .swarm_repo()
        .get_run(&run.id)
        .await
        .ok()
        .and_then(|r| r.result)
        .is_some_and(|r| r.get("_routing_complete").and_then(|v| v.as_i64()) == Some(1))
    {
        return;
    }
    let completed = result.is_some();
    if completed
        && !matches!(ctx.swarm_repo().get_swarm(&task.swarm_id).await, Ok(s) if s.status == "active")
    {
        return;
    }
    route_result_inner(ctx, run, task, result).await;
    if completed {
        // Child completion may have preceded the parent's interrupted routing.
        if let Ok(current) = ctx.swarm_repo().get_task(&task.id).await {
            if current.status == "in_progress"
                && current.delegated
                && ctx
                    .swarm_repo()
                    .children_complete(&task.id)
                    .await
                    .unwrap_or(false)
            {
                let _ = ctx
                    .swarm_repo()
                    .set_task_status_if(&task.id, &["in_progress"], "done")
                    .await;
                emit_task(ctx, &task.id).await;
                complete_parent_if_done(ctx, &current).await;
            }
        }
        if let Err(error) = ctx.swarm_repo().finish_task_routing(&run.id).await {
            tracing::warn!(run = %run.id, "checkpoint swarm result routing: {error}");
        }
    }
}

async fn route_result_inner(
    ctx: &SwarmRt,
    run: &otto_state::SwarmRun,
    task: &SwarmTask,
    result: Option<SwarmTurnResult>,
) {
    let repo = &ctx.swarm_repo();
    // The board may have been cleared (or the task deleted) while this turn ran.
    // A finished turn for a deleted task must do NOTHING — no retries, no
    // handoffs, no feed posts — or a cleared board immediately repopulates.
    let Ok(current) = repo.get_task(&task.id).await else {
        return;
    };
    // The operator moved the card (cancelled / blocked / done / back to todo)
    // or reassigned it while the turn ran: their decision wins. The turn's
    // outcome is noted on the board, but no status change, retry, subtask,
    // verification or merge follows (a failed turn used to resurrect a
    // cancelled task as `todo`).
    let reassigned = current.assignee_agent_id != task.assignee_agent_id
        && current.assignee_agent_id.as_deref() != Some(run.agent_id.as_str());
    let recovering_review =
        current.status == "in_review" && result.as_ref().is_some_and(|r| !r.reviews.is_empty());
    if (current.status != "in_progress" && !recovering_review) || reassigned {
        let outcome = match &result {
            Some(r) if !r.summary.is_empty() => format!("finished ({})", clip(&r.summary, 160)),
            Some(_) => "finished".to_string(),
            None => "ended without a result".to_string(),
        };
        system_post(
            ctx,
            &task.swarm_id,
            Some(&task.project_id),
            Some(&task.id),
            "status",
            &format!(
                "A run for “{}” {outcome}, but the task was changed meanwhile ({}) — left as you set it.",
                task.title,
                if reassigned { "reassigned".to_string() } else { current.status.clone() }
            ),
        )
        .await;
        return;
    }
    let Some(res) = result else {
        // Stopped rather than failed? A PAUSE parks the task for the resume
        // (the interrupted turn doesn't count as an attempt); an operator Stop
        // parks it as `blocked` — re-queueing it as `todo` made Stop behave
        // like a restart on the next tick.
        let run_now = repo.get_run(&run.id).await.ok();
        if let Some(r) = run_now.filter(|r| r.status == "stopped") {
            let paused = r.error.as_deref() == Some(PAUSED_RUN_REASON);
            if paused {
                let _ = repo.refund_task_attempt(&task.id).await;
            }
            let _ = repo
                .update_task(
                    &task.id,
                    TaskPatch {
                        status: Some(if paused { "todo" } else { "blocked" }.into()),
                        ..Default::default()
                    },
                )
                .await;
            emit_task(ctx, &task.id).await;
            if !paused {
                system_post(
                    ctx,
                    &task.swarm_id,
                    Some(&task.project_id),
                    Some(&task.id),
                    "status",
                    &format!(
                        "Run for “{}” was stopped — the task is parked as blocked; move it back to To do to run it again.",
                        task.title
                    ),
                )
                .await;
            }
            return;
        }
        // Turn failed/stopped. Retry on the next tick up to the attempt ceiling
        // (D8); once exhausted, block the task so it isn't retried forever.
        if attempt_ceiling_reached(ctx, task).await {
            block_for_attempts(ctx, task).await;
        } else {
            let _ = repo
                .update_task(
                    &task.id,
                    TaskPatch {
                        status: Some("todo".into()),
                        ..Default::default()
                    },
                )
                .await;
            emit_task(ctx, &task.id).await;
            system_post(
                ctx,
                &task.swarm_id,
                Some(&task.project_id),
                Some(&task.id),
                "status",
                &format!("Run for “{}” did not complete — will retry.", task.title),
            )
            .await;
        }
        return;
    };

    // Concerns → board + notification (CTO/PM "wrong path" escalation).
    for c in &res.concerns {
        if c.text.trim().is_empty() {
            continue;
        }
        system_post(
            ctx,
            &task.swarm_id,
            Some(&task.project_id),
            Some(&task.id),
            "concern",
            &format!("[{}] {}", c.severity, c.text),
        )
        .await;
        let _ = ctx.events().send(Event::Notice {
            level: "warn".into(),
            title: "Swarm concern raised".into(),
            body: clip(&c.text, 160),
        });
    }

    // Delegation (planning) → create subtasks for reports.
    if run.kind == "planning" {
        if res.subtasks.is_empty() {
            // Leader produced nothing to delegate — let it act as an IC next time.
            let _ = repo
                .update_task(
                    &task.id,
                    TaskPatch {
                        status: Some("todo".into()),
                        delegated: Some(true),
                        ..Default::default()
                    },
                )
                .await;
            emit_task(ctx, &task.id).await;
            return;
        }
        let _ = repo
            .update_task(
                &task.id,
                TaskPatch {
                    status: Some("in_progress".into()),
                    delegated: Some(true),
                    ..Default::default()
                },
            )
            .await;
        create_subtasks(ctx, task, &res.subtasks).await;
        emit_task(ctx, &task.id).await;
        return;
    }

    // The MANAGER owns the plan. Agent-originated divergence (subtasks from an
    // IC, handoffs) must not grow the board directly — an unmanaged A↔B handoff
    // loop once inflated a 28-task plan to 150 mostly-blocked tasks. ICs with a
    // manager get their proposals routed to that manager as ONE triage task
    // (the manager's turn runs as `planning` and delegates properly, or drops
    // it); only manager-less agents keep direct creation. Every chain carries a
    // `hops:N` label — past MAX_HANDOFF_HOPS it escalates to a human instead of
    // creating yet another task.
    let run_agent = repo.get_agent(&run.agent_id).await.ok();
    let manager_id = run_agent.as_ref().and_then(|a| a.reports_to.clone());
    let agent_name = run_agent
        .as_ref()
        .map(|a| a.name.clone())
        .unwrap_or_else(|| "an agent".into());
    let hops = task_hops(task) + 1;

    // Subtasks from a normal task: managed ICs propose to their manager.
    if !res.subtasks.is_empty() {
        match &manager_id {
            Some(mid) if hops <= MAX_HANDOFF_HOPS => {
                let listing: String = res
                    .subtasks
                    .iter()
                    .map(|s| format!("- {}: {}\n", s.title, clip(&s.description, 160)))
                    .collect();
                create_agent_task(
                    ctx,
                    task,
                    run,
                    hops,
                    &format!(
                        "Triage proposal from {}: {}",
                        agent_name,
                        clip(&task.title, 50)
                    ),
                    &format!(
                        "While working “{}”, an IC proposed new subtasks. As the manager, \
                         delegate the ones that serve the plan and DROP the rest.\n\n{listing}",
                        task.title
                    ),
                    Some(mid.clone()),
                    "proposal",
                )
                .await;
            }
            Some(_) => escalate_chain(ctx, task, "subtask proposal").await,
            None => create_subtasks(ctx, task, &res.subtasks).await,
        }
    }

    // Handoffs → one follow-up each, manager-gated and chain-capped.
    for h in &res.handoffs {
        if h.to_role.trim().is_empty() {
            continue;
        }
        if hops > MAX_HANDOFF_HOPS {
            escalate_chain(ctx, task, &format!("handoff to {}", h.to_role)).await;
            continue;
        }
        match &manager_id {
            Some(mid) => {
                create_agent_task(
                    ctx,
                    task,
                    run,
                    hops,
                    &format!("Triage handoff → {}: {}", h.to_role, clip(&h.brief, 50)),
                    &format!(
                        "Handoff raised from “{}” aimed at “{}”. As the manager, decide: \
                         delegate it (as a subtask, to the right report) if it serves the \
                         plan, or close this task with status done to drop it.\n\n{}",
                        task.title, h.to_role, h.brief
                    ),
                    Some(mid.clone()),
                    "handoff",
                )
                .await;
            }
            None => {
                let assignee = resolve_agent_by_title(ctx, &task.swarm_id, &h.to_role).await;
                create_agent_task(
                    ctx,
                    task,
                    run,
                    hops,
                    &format!("Handoff: {}", clip(&h.brief, 60)),
                    &h.brief.clone(),
                    assignee,
                    "handoff",
                )
                .await;
            }
        }
    }

    // Apply the reported status to the task.
    let artifact_ref = res
        .artifacts
        .iter()
        .find_map(|a| a.path.clone().or_else(|| a.url.clone()));
    match res.status.as_str() {
        "done" => {
            // If a review was requested, go to in_review and enqueue a review run
            // (human-review flow takes precedence over goal verification).
            if !res.reviews.is_empty() {
                let _ = repo
                    .update_task(
                        &task.id,
                        TaskPatch {
                            status: Some("in_review".into()),
                            result_ref: Some(artifact_ref),
                            ..Default::default()
                        },
                    )
                    .await;
                if enqueue_reviews(ctx, task, run, &res).await == 0 {
                    // Nobody to review it: an `in_review` task without a review
                    // child never advances. Complete it, saying so.
                    let _ = repo
                        .update_task(
                            &task.id,
                            TaskPatch {
                                status: Some("done".into()),
                                ..Default::default()
                            },
                        )
                        .await;
                    complete_parent_if_done(ctx, task).await;
                    system_post(
                        ctx,
                        &task.swarm_id,
                        Some(&task.project_id),
                        Some(&task.id),
                        "status",
                        &format!(
                            "No reviewer could be found for “{}” — completed without review.",
                            task.title
                        ),
                    )
                    .await;
                }
            } else if crate::runtime::verify::task_has_goals(ctx, task).await {
                // Goals attached → the leader verifies each sequentially before the
                // task is done + its worktree branch is merged (requirement 3).
                // Persist the dev as the assignee so restart-recovery + the
                // coordinator's per-agent lock can find it.
                let _ = repo
                    .update_task(
                        &task.id,
                        TaskPatch {
                            status: Some("verifying".into()),
                            result_ref: Some(artifact_ref),
                            assignee_agent_id: Some(Some(run.agent_id.clone())),
                            ..Default::default()
                        },
                    )
                    .await;
                emit_task(ctx, &task.id).await;
                crate::runtime::verify::start_verification(ctx, task.clone(), run.agent_id.clone());
                return; // controller drives the task to done/blocked + posts summary
            } else {
                let _ = repo
                    .update_task(
                        &task.id,
                        TaskPatch {
                            status: Some("done".into()),
                            result_ref: Some(artifact_ref),
                            ..Default::default()
                        },
                    )
                    .await;
                complete_parent_if_done(ctx, task).await;
            }
        }
        "needs_review" => {
            let _ = repo
                .update_task(
                    &task.id,
                    TaskPatch {
                        status: Some("in_review".into()),
                        result_ref: Some(artifact_ref),
                        ..Default::default()
                    },
                )
                .await;
            if enqueue_reviews(ctx, task, run, &res).await == 0 {
                // A review is required but nobody can do it: park it for a
                // human instead of an `in_review` that never advances.
                let _ = repo
                    .update_task(
                        &task.id,
                        TaskPatch {
                            status: Some("blocked".into()),
                            ..Default::default()
                        },
                    )
                    .await;
                system_post(
                    ctx,
                    &task.swarm_id,
                    Some(&task.project_id),
                    Some(&task.id),
                    "status",
                    &format!(
                        "“{}” needs a review but no reviewer could be found — blocked for a human.",
                        task.title
                    ),
                )
                .await;
            }
        }
        "blocked" => {
            let _ = repo
                .update_task(
                    &task.id,
                    TaskPatch {
                        status: Some("blocked".into()),
                        ..Default::default()
                    },
                )
                .await;
        }
        _ => {
            // in_progress / unknown → allow another turn next tick, UNLESS the
            // task has hit its attempt ceiling (D8): otherwise a task that never
            // self-reports a terminal status would re-run forever, burning the
            // budget. Block it with a reason + notification instead.
            if attempt_ceiling_reached(ctx, task).await {
                block_for_attempts(ctx, task).await;
            } else {
                let _ = repo
                    .update_task(
                        &task.id,
                        TaskPatch {
                            status: Some("todo".into()),
                            ..Default::default()
                        },
                    )
                    .await;
            }
        }
    }
    emit_task(ctx, &task.id).await;
    if !res.summary.is_empty() {
        system_post(
            ctx,
            &task.swarm_id,
            Some(&task.project_id),
            Some(&task.id),
            "status",
            &format!("{} — {}", task.title, clip(&res.summary, 240)),
        )
        .await;
    }
}

/// Max agent-originated chain length (handoff → triage → handoff …). Past this
/// the swarm escalates to a human instead of creating another task — the cap
/// that kills A↔B ping-pong.
const MAX_HANDOFF_HOPS: i64 = 3;
/// Backstop: agent-originated creation (handoffs/proposals) may not grow a
/// project past this many open tasks. The plan itself and the human UI are
/// exempt — this only bounds runaway self-inflation.
const MAX_OPEN_TASKS_PER_PROJECT: usize = 60;

/// Chain depth carried on a task's labels as `hops:N` (0 for plan/human tasks).
fn task_hops(task: &SwarmTask) -> i64 {
    task.labels
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|l| l.as_str())
        .find_map(|l| l.strip_prefix("hops:").and_then(|n| n.parse().ok()))
        .unwrap_or(0)
}

/// Create one agent-originated task (handoff / triage proposal): dedups against
/// open tasks with the same title, enforces the per-project open-task backstop,
/// stamps the `hops:N` chain label, and emits the board update.
#[allow(clippy::too_many_arguments)]
async fn create_agent_task(
    ctx: &SwarmRt,
    origin: &SwarmTask,
    run: &otto_state::SwarmRun,
    hops: i64,
    title: &str,
    description: &str,
    assignee: Option<Id>,
    label: &str,
) {
    let repo = &ctx.swarm_repo();
    let existing = repo
        .list_tasks(&origin.project_id)
        .await
        .unwrap_or_default();
    let origin_label = format!("source-run:{}", run.id);
    if existing.iter().any(|t| {
        t.title.trim().eq_ignore_ascii_case(title.trim())
            && t.labels
                .as_array()
                .is_some_and(|labels| labels.contains(&json!(origin_label)))
    }) {
        return;
    }
    let open: Vec<_> = existing
        .into_iter()
        .filter(|t| !matches!(t.status.as_str(), "done" | "cancelled"))
        .collect();
    // Dedup: an identical open item means the loop is repeating itself.
    let want = title.trim().to_lowercase();
    if open.iter().any(|t| t.title.trim().to_lowercase() == want) {
        return;
    }
    if open.len() >= MAX_OPEN_TASKS_PER_PROJECT {
        system_post(ctx, &origin.swarm_id, Some(&origin.project_id), Some(&origin.id), "escalation",
            &format!(
                "Board full ({} open tasks) — dropping agent-created “{}”. Close or prune tasks to resume.",
                open.len(), clip(title, 60)
            )).await;
        return;
    }
    // Never create an unassigned card: fall back to the best-fitting agent.
    let assignee = match assignee {
        Some(a) => Some(a),
        None => best_fit_agent_id(ctx, &origin.swarm_id, &format!("{title} {description}")).await,
    };
    if let Ok(task) = repo
        .create_task(NewTask {
            project_id: origin.project_id.clone(),
            swarm_id: origin.swarm_id.clone(),
            workspace_id: origin.workspace_id.clone(),
            title: title.to_string(),
            description: description.to_string(),
            assignee_agent_id: assignee,
            status: "todo".into(),
            priority: "medium".into(),
            parent_task_id: None,
            depends_on: json!([]),
            labels: json!([label, format!("hops:{hops}"), origin_label]),
            order_idx: 0,
            created_by: run.agent_id.clone(),
        })
        .await
    {
        emit_task(ctx, &task.id).await;
    }
}

/// A chain hit MAX_HANDOFF_HOPS: stop creating tasks, tell the humans.
async fn escalate_chain(ctx: &SwarmRt, task: &SwarmTask, what: &str) {
    let body = format!(
        "Handoff chain from “{}” exceeded {MAX_HANDOFF_HOPS} hops ({what}) — not creating \
         another task. A human (or the manager) should decide how to proceed.",
        task.title
    );
    system_post(
        ctx,
        &task.swarm_id,
        Some(&task.project_id),
        Some(&task.id),
        "escalation",
        &body,
    )
    .await;
    let _ = ctx.events().send(Event::Notice {
        level: "warn".into(),
        title: "Swarm handoff chain capped".into(),
        body: clip(&body, 160),
    });
}

/// Has a task exhausted its swarm's per-task attempt ceiling? Re-reads the task
/// for the up-to-date attempt counter (the Coordinator bumps it when it queues
/// each turn) and compares against the swarm's `max_attempts` (default 3, min 1).
async fn attempt_ceiling_reached(ctx: &SwarmRt, task: &SwarmTask) -> bool {
    let repo = &ctx.swarm_repo();
    let attempts = repo
        .get_task(&task.id)
        .await
        .map(|t| t.attempts)
        .unwrap_or(task.attempts);
    let ceiling = repo
        .get_swarm(&task.swarm_id)
        .await
        .map(|s| s.max_attempts.max(1))
        .unwrap_or(3);
    attempts >= ceiling
}

/// Mark a task `blocked` because it hit the attempt ceiling, post to the board,
/// and notify. Used both for hard failures and tasks that never self-report a
/// terminal status.
async fn block_for_attempts(ctx: &SwarmRt, task: &SwarmTask) {
    let repo = &ctx.swarm_repo();
    let attempts = repo
        .get_task(&task.id)
        .await
        .map(|t| t.attempts)
        .unwrap_or(task.attempts);
    let _ = repo
        .update_task(
            &task.id,
            TaskPatch {
                status: Some("blocked".into()),
                ..Default::default()
            },
        )
        .await;
    emit_task(ctx, &task.id).await;
    let body = format!(
        "Task “{}” blocked after {attempts} attempt(s) without completing — needs a human.",
        task.title
    );
    system_post(
        ctx,
        &task.swarm_id,
        Some(&task.project_id),
        Some(&task.id),
        "escalation",
        &body,
    )
    .await;
    let _ = ctx.events().send(Event::Notice {
        level: "warn".into(),
        title: "Swarm task blocked (attempts)".into(),
        body: clip(&body, 160),
    });
}

async fn create_subtasks(ctx: &SwarmRt, parent: &SwarmTask, subs: &[run::TurnSubtask]) {
    let repo = &ctx.swarm_repo();
    // Open-title set for dedup + the backstop count: delegation must not
    // re-create board items that already exist (a repeated planning turn used
    // to double every subtask), nor inflate the project past the cap.
    let existing = repo
        .list_tasks(&parent.project_id)
        .await
        .unwrap_or_default();
    let mut titles: std::collections::HashSet<String> = existing
        .iter()
        .filter(|t| {
            t.parent_task_id.as_deref() == Some(parent.id.as_str())
                || !matches!(t.status.as_str(), "done" | "cancelled")
        })
        .map(|t| t.title.trim().to_lowercase())
        .collect();
    let mut open_count = existing
        .iter()
        .filter(|t| !matches!(t.status.as_str(), "done" | "cancelled"))
        .count();
    // Subtasks inherit the parent's chain depth so a triage-spawned subtask
    // that hands off again still walks toward the MAX_HANDOFF_HOPS cap.
    let hops = task_hops(parent);
    for (i, st) in subs.iter().enumerate() {
        if st.title.trim().is_empty() {
            continue;
        }
        if !titles.insert(st.title.trim().to_lowercase()) {
            continue; // already on the board
        }
        if open_count >= MAX_OPEN_TASKS_PER_PROJECT {
            system_post(
                ctx,
                &parent.swarm_id,
                Some(&parent.project_id),
                Some(&parent.id),
                "escalation",
                &format!(
                    "Board full ({open_count} open tasks) — dropping remaining subtasks of “{}”.",
                    parent.title
                ),
            )
            .await;
            break;
        }
        let mut assignee = match &st.assignee_role {
            Some(role) if !role.is_empty() => {
                resolve_agent_by_title(ctx, &parent.swarm_id, role).await
            }
            _ => None,
        };
        if assignee.is_none() {
            // A role that didn't resolve (or none given) must not land unassigned.
            assignee = best_fit_agent_id(
                ctx,
                &parent.swarm_id,
                &format!("{} {}", st.title, st.description),
            )
            .await;
        }
        let priority = st
            .priority
            .clone()
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| "medium".into());
        let labels = if hops > 0 {
            json!([format!("hops:{hops}")])
        } else {
            json!([])
        };
        if let Ok(task) = repo
            .create_task(NewTask {
                project_id: parent.project_id.clone(),
                swarm_id: parent.swarm_id.clone(),
                workspace_id: parent.workspace_id.clone(),
                title: st.title.clone(),
                description: st.description.clone(),
                assignee_agent_id: assignee,
                status: "todo".into(),
                priority,
                parent_task_id: Some(parent.id.clone()),
                depends_on: json!([]),
                labels,
                order_idx: i as i64,
                created_by: parent.created_by.clone(),
            })
            .await
        {
            open_count += 1;
            emit_task(ctx, &task.id).await;
        }
    }
}

/// Create one review task per requested review. A reviewer role that doesn't
/// resolve falls back to the working agent's manager. Returns how many review
/// tasks were created (0 ⇒ the caller must not leave the task `in_review`).
async fn enqueue_reviews(
    ctx: &SwarmRt,
    task: &SwarmTask,
    run: &otto_state::SwarmRun,
    res: &SwarmTurnResult,
) -> usize {
    let repo = &ctx.swarm_repo();
    let manager = repo
        .get_agent(&run.agent_id)
        .await
        .ok()
        .and_then(|a| a.reports_to);
    let mut created = 0usize;
    for (index, rv) in res.reviews.iter().enumerate() {
        let origin = format!("review-source:{}:{index}", run.id);
        if repo
            .list_tasks(&task.project_id)
            .await
            .unwrap_or_default()
            .iter()
            .any(|t| {
                t.labels
                    .as_array()
                    .is_some_and(|ls| ls.iter().any(|l| l.as_str() == Some(origin.as_str())))
            })
        {
            created += 1;
            continue;
        }
        let reviewer = resolve_agent_by_title(ctx, &task.swarm_id, &rv.reviewer_role)
            .await
            .or_else(|| manager.clone());
        let Some(reviewer) = reviewer else { continue };
        // A review run: a new task assigned to the reviewer.
        let _ = repo.create_task(NewTask {
            project_id: task.project_id.clone(),
            swarm_id: task.swarm_id.clone(),
            workspace_id: task.workspace_id.clone(),
            title: format!("Review: {}", clip(&task.title, 60)),
            description: format!(
                "Review the work of {} on “{}”. Artifact: {}. Reply with a `review` board post and a result.",
                run.agent_id, task.title, rv.of
            ),
            assignee_agent_id: Some(reviewer),
            status: "todo".into(),
            priority: "high".into(),
            parent_task_id: Some(task.id.clone()),
            depends_on: json!([]),
            labels: json!(["review", origin]),
            order_idx: 0,
            created_by: run.agent_id.clone(),
        }).await;
        created += 1;
    }
    if created > 0 {
        system_post(
            ctx,
            &task.swarm_id,
            Some(&task.project_id),
            Some(&task.id),
            "review_request",
            &format!("Review requested on “{}”.", task.title),
        )
        .await;
    }
    created
}

/// When a task completes, if it has a parent and all the parent's children are
/// done, complete the parent too (recursively). Also called by the goal
/// verification controller when it completes a task.
pub(crate) async fn complete_parent_if_done(ctx: &SwarmRt, task: &SwarmTask) {
    let repo = &ctx.swarm_repo();
    let Some(parent_id) = &task.parent_task_id else {
        return;
    };
    if repo.children_complete(parent_id).await.unwrap_or(false) {
        if let Ok(parent) = repo.get_task(parent_id).await {
            if (parent.status == "in_progress" && parent.delegated) || parent.status == "in_review"
            {
                let _ = repo
                    .update_task(
                        parent_id,
                        TaskPatch {
                            status: Some("done".into()),
                            ..Default::default()
                        },
                    )
                    .await;
                emit_task(ctx, parent_id).await;
                Box::pin(complete_parent_if_done(ctx, &parent)).await;
            }
        }
    }
}

async fn system_post(
    ctx: &SwarmRt,
    swarm_id: &str,
    project_id: Option<&str>,
    task_id: Option<&str>,
    kind: &str,
    body: &str,
) {
    system_post_meta(ctx, swarm_id, project_id, task_id, kind, body, json!({})).await;
}

/// A system board post carrying structured `meta` (e.g. worktree/shared/merge/verify
/// events). Used across the swarm runtime + verification controller.
pub(crate) async fn system_post_meta(
    ctx: &SwarmRt,
    swarm_id: &str,
    project_id: Option<&str>,
    task_id: Option<&str>,
    kind: &str,
    body: &str,
    meta: serde_json::Value,
) {
    let swarm = match ctx.swarm_repo().get_swarm(&swarm_id.to_string()).await {
        Ok(s) => s,
        Err(_) => return,
    };
    if let Ok(msg) = ctx
        .swarm_repo()
        .create_message(otto_state::NewMessage {
            swarm_id: swarm_id.to_string(),
            workspace_id: swarm.workspace_id.clone(),
            project_id: project_id.map(str::to_string),
            task_id: task_id.map(str::to_string),
            run_id: None,
            author_agent_id: None,
            author_user_id: None,
            to_agent_id: None,
            kind: kind.to_string(),
            body: body.to_string(),
            meta,
        })
        .await
    {
        let _ = ctx.events().send(Event::SwarmMessagePosted {
            workspace_id: swarm.workspace_id,
            swarm_id: swarm_id.to_string(),
            message: serde_json::to_value(&msg).unwrap_or_default(),
        });
    }
}

async fn emit_task(ctx: &SwarmRt, task_id: &str) {
    if let Ok(t) = ctx.swarm_repo().get_task(&task_id.to_string()).await {
        let _ = ctx.events().send(Event::SwarmTaskUpdated {
            workspace_id: t.workspace_id.clone(),
            swarm_id: t.swarm_id.clone(),
            project_id: t.project_id.clone(),
            task: serde_json::to_value(&t).unwrap_or_default(),
        });
    }
}

/// Public re-export for the verification controller.
pub(crate) async fn emit_task_pub(ctx: &SwarmRt, task_id: &str) {
    emit_task(ctx, task_id).await;
}

/// True if the swarm is paused or over any budget — the verification controller
/// consults this between goals/fixes so it doesn't run past the budget gate.
pub(crate) async fn is_over_budget(ctx: &SwarmRt, swarm_id: &str) -> bool {
    match ctx.swarm_repo().get_swarm(&swarm_id.to_string()).await {
        Ok(s) => s.status == "paused" || budget_exceeded(ctx, &s).await.is_some(),
        Err(_) => true,
    }
}

pub(crate) use otto_core::text::clip_chars as clip;

// --- Session teardown for pause/abort --------------------------------------

async fn swarm_session_ids(ctx: &SwarmRt, ws: &Id, swarm_id: &str) -> Vec<Id> {
    // Only live sessions matter here (callers suspend/stop them); filtered in
    // SQL rather than decoding the workspace's whole history (perf §15 F7).
    ctx.manager()
        .list_live_by_meta(ws, None, "swarm_id", swarm_id)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|s| s.id)
        .collect()
}

// --- HTTP: lifecycle + run/stop + recruit + plan ---------------------------

pub fn routes<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    SwarmRt: axum::extract::FromRef<S>,
{
    Router::new()
        .route("/workspaces/{id}/swarm/swarms/{sid}/start", post(start))
        .route("/workspaces/{id}/swarm/swarms/{sid}/pause", post(pause))
        .route("/workspaces/{id}/swarm/swarms/{sid}/abort", post(abort))
        .route("/workspaces/{id}/swarm/swarms/{sid}/resume", post(resume))
        .route("/swarm/tasks/{tid}/run", post(run_task))
        .route("/swarm/runs/{rid}/stop", post(stop_run))
        .route("/workspaces/{id}/swarm/recruit", post(recruit))
        .route("/workspaces/{id}/swarm/projects/{pid}/plan", post(plan))
        .route("/swarm/projects/{pid}/clear", post(clear_project_h))
        .route("/swarm/swarms/{sid}/utilization", get(utilization_h))
        .route("/swarm/swarms/{sid}/waiting", get(waiting_h))
        .route(
            "/workspaces/{id}/swarm/swarms/{sid}/agent-stop",
            post(agent_stop),
        )
        // Goals (requirement 3)
        .route(
            "/swarm/tasks/{tid}/goals",
            get(list_task_goals).post(create_task_goal),
        )
        .route(
            "/swarm/projects/{pid}/goals",
            get(list_project_goals).post(create_project_goal),
        )
        .route(
            "/swarm/goals/{gid}",
            patch(update_goal_h).delete(delete_goal_h),
        )
        .route(
            "/swarm/swarms/{sid}/standing-goals",
            get(list_standing_goals_h).put(put_standing_goals_h),
        )
        // Verification controller
        .route("/swarm/tasks/{tid}/verify", post(verify_task_h))
        .route("/swarm/tasks/{tid}/verify/stop", post(stop_verify_h))
        .route("/swarm/tasks/{tid}/verification", get(get_verification_h))
        // Channel triggers (requirement 4)
        .route(
            "/swarm/swarms/{sid}/triggers",
            get(list_triggers_h).post(create_trigger_h),
        )
        .route(
            "/swarm/triggers/{tid}",
            patch(update_trigger_h).delete(delete_trigger_h),
        )
}

// --- HTTP: goals + verification + triggers ---------------------------------

#[derive(Deserialize)]
struct CreateGoalReq {
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    metric: Option<String>,
    #[serde(default)]
    comparator: Option<String>,
    #[serde(default)]
    target_value: Option<f64>,
    #[serde(default)]
    block_value: Option<f64>,
    #[serde(default)]
    verify_cmd: Option<String>,
    #[serde(default)]
    max_retries: Option<i64>,
    #[serde(default)]
    blocking: Option<bool>,
    #[serde(default)]
    order_idx: Option<i64>,
}

#[derive(Deserialize, Default)]
struct UpdateGoalReq {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default, deserialize_with = "crate::types::de_double_option")]
    metric: Option<Option<String>>,
    #[serde(default, deserialize_with = "crate::types::de_double_option")]
    comparator: Option<Option<String>>,
    #[serde(default, deserialize_with = "crate::types::de_double_option")]
    target_value: Option<Option<f64>>,
    #[serde(default, deserialize_with = "crate::types::de_double_option")]
    block_value: Option<Option<f64>>,
    #[serde(default, deserialize_with = "crate::types::de_double_option")]
    verify_cmd: Option<Option<String>>,
    #[serde(default)]
    max_retries: Option<i64>,
    #[serde(default)]
    blocking: Option<bool>,
    #[serde(default)]
    order_idx: Option<i64>,
}

async fn emit_goal(ctx: &SwarmRt, goal: &SwarmGoal) {
    let _ = ctx.events().send(Event::SwarmGoalUpdated {
        workspace_id: goal.workspace_id.clone(),
        swarm_id: goal.swarm_id.clone(),
        task_id: goal.task_id.clone(),
        goal: serde_json::to_value(goal).unwrap_or_default(),
    });
}

async fn list_task_goals(
    State(ctx): State<SwarmRt>,
    Extension(user): Extension<AuthUser>,
    Path(tid): Path<Id>,
) -> ApiResult<Json<Vec<SwarmGoal>>> {
    let task = ctx.swarm_repo().get_task(&tid).await.map_err(ApiError)?;
    check(&ctx, &user, &task.workspace_id, WorkspaceRole::Viewer).await?;
    Ok(Json(
        ctx.swarm_repo()
            .list_goals_for_task(&tid)
            .await
            .map_err(ApiError)?,
    ))
}

async fn list_project_goals(
    State(ctx): State<SwarmRt>,
    Extension(user): Extension<AuthUser>,
    Path(pid): Path<Id>,
) -> ApiResult<Json<Vec<SwarmGoal>>> {
    let project = ctx.swarm_repo().get_project(&pid).await.map_err(ApiError)?;
    check(&ctx, &user, &project.workspace_id, WorkspaceRole::Viewer).await?;
    Ok(Json(
        ctx.swarm_repo()
            .list_goals_for_project(&pid)
            .await
            .map_err(ApiError)?,
    ))
}

async fn create_task_goal(
    State(ctx): State<SwarmRt>,
    Extension(user): Extension<AuthUser>,
    Path(tid): Path<Id>,
    Json(req): Json<CreateGoalReq>,
) -> ApiResult<Json<SwarmGoal>> {
    let task = ctx.swarm_repo().get_task(&tid).await.map_err(ApiError)?;
    check(&ctx, &user, &task.workspace_id, WorkspaceRole::Editor).await?;
    let goal = ctx
        .swarm_repo()
        .create_goal(new_goal_from(
            req,
            &task.swarm_id,
            &task.workspace_id,
            Some(task.project_id.clone()),
            Some(tid),
            &user.0.id,
        ))
        .await
        .map_err(ApiError)?;
    emit_goal(&ctx, &goal).await;
    Ok(Json(goal))
}

async fn create_project_goal(
    State(ctx): State<SwarmRt>,
    Extension(user): Extension<AuthUser>,
    Path(pid): Path<Id>,
    Json(req): Json<CreateGoalReq>,
) -> ApiResult<Json<SwarmGoal>> {
    let project = ctx.swarm_repo().get_project(&pid).await.map_err(ApiError)?;
    check(&ctx, &user, &project.workspace_id, WorkspaceRole::Editor).await?;
    let goal = ctx
        .swarm_repo()
        .create_goal(new_goal_from(
            req,
            &project.swarm_id,
            &project.workspace_id,
            Some(pid),
            None,
            &user.0.id,
        ))
        .await
        .map_err(ApiError)?;
    emit_goal(&ctx, &goal).await;
    Ok(Json(goal))
}

fn new_goal_from(
    req: CreateGoalReq,
    swarm_id: &str,
    workspace_id: &str,
    project_id: Option<Id>,
    task_id: Option<Id>,
    created_by: &str,
) -> NewGoal {
    NewGoal {
        swarm_id: swarm_id.to_string(),
        workspace_id: workspace_id.to_string(),
        project_id,
        task_id,
        kind: "explicit".into(),
        title: req.title,
        description: req.description,
        metric: req.metric,
        comparator: req.comparator,
        target_value: req.target_value,
        block_value: req.block_value,
        verify_cmd: req.verify_cmd,
        max_retries: req.max_retries.unwrap_or(3),
        blocking: req.blocking.unwrap_or(true),
        order_idx: req.order_idx.unwrap_or(0),
        created_by: created_by.to_string(),
    }
}

async fn update_goal_h(
    State(ctx): State<SwarmRt>,
    Extension(user): Extension<AuthUser>,
    Path(gid): Path<Id>,
    Json(req): Json<UpdateGoalReq>,
) -> ApiResult<Json<SwarmGoal>> {
    let cur = ctx.swarm_repo().get_goal(&gid).await.map_err(ApiError)?;
    check(&ctx, &user, &cur.workspace_id, WorkspaceRole::Editor).await?;
    let goal = ctx
        .swarm_repo()
        .update_goal(
            &gid,
            GoalPatch {
                title: req.title,
                description: req.description,
                metric: req.metric,
                comparator: req.comparator,
                target_value: req.target_value,
                block_value: req.block_value,
                verify_cmd: req.verify_cmd,
                max_retries: req.max_retries,
                blocking: req.blocking,
                order_idx: req.order_idx,
                ..Default::default()
            },
        )
        .await
        .map_err(ApiError)?;
    emit_goal(&ctx, &goal).await;
    Ok(Json(goal))
}

async fn delete_goal_h(
    State(ctx): State<SwarmRt>,
    Extension(user): Extension<AuthUser>,
    Path(gid): Path<Id>,
) -> ApiResult<Json<serde_json::Value>> {
    let cur = ctx.swarm_repo().get_goal(&gid).await.map_err(ApiError)?;
    check(&ctx, &user, &cur.workspace_id, WorkspaceRole::Editor).await?;
    ctx.swarm_repo().delete_goal(&gid).await.map_err(ApiError)?;
    Ok(Json(json!({})))
}

async fn list_standing_goals_h(
    State(ctx): State<SwarmRt>,
    Extension(user): Extension<AuthUser>,
    Path(sid): Path<Id>,
) -> ApiResult<Json<Vec<SwarmGoal>>> {
    let swarm = ctx.swarm_repo().get_swarm(&sid).await.map_err(ApiError)?;
    check(&ctx, &user, &swarm.workspace_id, WorkspaceRole::Viewer).await?;
    // A Viewer GET never writes (S4-24). Swarms that predate seeding get
    // their defaults lazily — but only on an Editor's read.
    if ctx
        .roles()
        .check(&user.0, &swarm.workspace_id, WorkspaceRole::Editor)
        .await
        .is_ok()
    {
        crate::runtime::verify::ensure_standing_goals(
            &ctx,
            &swarm.id,
            &swarm.workspace_id,
            &swarm.created_by,
        )
        .await;
    }
    Ok(Json(
        ctx.swarm_repo()
            .list_standing_goals(&sid)
            .await
            .map_err(ApiError)?,
    ))
}

#[derive(Deserialize)]
struct StandingGoalsReq {
    goals: Vec<CreateGoalReq>,
}

/// Replace the swarm's standing-goal set (delete existing templates + insert new).
async fn put_standing_goals_h(
    State(ctx): State<SwarmRt>,
    Extension(user): Extension<AuthUser>,
    Path(sid): Path<Id>,
    Json(req): Json<StandingGoalsReq>,
) -> ApiResult<Json<Vec<SwarmGoal>>> {
    let swarm = ctx.swarm_repo().get_swarm(&sid).await.map_err(ApiError)?;
    check(&ctx, &user, &swarm.workspace_id, WorkspaceRole::Editor).await?;
    let goals = req
        .goals
        .into_iter()
        .enumerate()
        .map(|(i, r)| {
            let mut ng = new_goal_from(r, &swarm.id, &swarm.workspace_id, None, None, &user.0.id);
            ng.kind = "standing".into();
            ng.order_idx = i as i64;
            ng
        })
        .collect();
    // One transaction, errors propagated (S4-24).
    Ok(Json(
        ctx.swarm_repo()
            .replace_standing_goals(&sid, goals)
            .await
            .map_err(ApiError)?,
    ))
}

/// Manually kick the verification controller for a task (e.g. after a fix).
async fn verify_task_h(
    State(ctx): State<SwarmRt>,
    Extension(user): Extension<AuthUser>,
    Path(tid): Path<Id>,
) -> ApiResult<Json<serde_json::Value>> {
    let task = ctx.swarm_repo().get_task(&tid).await.map_err(ApiError)?;
    check(&ctx, &user, &task.workspace_id, WorkspaceRole::Editor).await?;
    let _operation = operation_guard(&task.swarm_id).await;
    let task = ctx.swarm_repo().get_task(&tid).await.map_err(ApiError)?;
    if ctx
        .swarm_repo()
        .get_swarm(&task.swarm_id)
        .await
        .map_err(ApiError)?
        .status
        != "active"
    {
        return Err(ApiError(Error::Invalid(
            "resume the swarm before verification".into(),
        )));
    }
    if crate::runtime::verify::is_verifying(&tid) {
        return Ok(Json(
            json!({"started": false, "reason": "already verifying"}),
        ));
    }
    let dev = task
        .assignee_agent_id
        .clone()
        .ok_or_else(|| ApiError(Error::Invalid("task has no assignee to verify".into())))?;
    for goal in ctx
        .swarm_repo()
        .list_goals_for_task(&tid)
        .await
        .map_err(ApiError)?
    {
        if task.status == "done" || !matches!(goal.status.as_str(), "passed" | "warned") {
            ctx.swarm_repo()
                .update_goal(
                    &goal.id,
                    otto_state::GoalPatch {
                        status: Some("pending".into()),
                        iterations: Some(0),
                        verdict: Some(None),
                        ..Default::default()
                    },
                )
                .await
                .map_err(ApiError)?;
        }
    }
    let _ = ctx
        .swarm_repo()
        .update_task(
            &tid,
            TaskPatch {
                status: Some("verifying".into()),
                ..Default::default()
            },
        )
        .await;
    emit_task(&ctx, &tid).await;
    crate::runtime::verify::start_verification(&ctx, task, dev);
    Ok(Json(json!({"started": true})))
}

async fn stop_verify_h(
    State(ctx): State<SwarmRt>,
    Extension(user): Extension<AuthUser>,
    Path(tid): Path<Id>,
) -> ApiResult<Json<serde_json::Value>> {
    let task = ctx.swarm_repo().get_task(&tid).await.map_err(ApiError)?;
    check(&ctx, &user, &task.workspace_id, WorkspaceRole::Editor).await?;
    crate::runtime::verify::stop_task(&ctx, &tid).await;
    Ok(Json(json!({"stopped": true})))
}

async fn get_verification_h(
    State(ctx): State<SwarmRt>,
    Extension(user): Extension<AuthUser>,
    Path(tid): Path<Id>,
) -> ApiResult<Json<serde_json::Value>> {
    let task = ctx.swarm_repo().get_task(&tid).await.map_err(ApiError)?;
    check(&ctx, &user, &task.workspace_id, WorkspaceRole::Viewer).await?;
    let goals = ctx
        .swarm_repo()
        .list_goals_for_task(&tid)
        .await
        .map_err(ApiError)?;
    Ok(Json(json!({
        "running": crate::runtime::verify::is_verifying(&tid),
        "task_status": task.status,
        "goals": goals,
    })))
}

#[derive(Deserialize)]
struct CreateTriggerReq {
    channel: String,
    #[serde(default)]
    match_chat: Option<String>,
    #[serde(default)]
    keyword: Option<String>,
    #[serde(default)]
    repo_path: Option<String>,
    #[serde(default)]
    auto_start: Option<bool>,
    #[serde(default)]
    reply: Option<bool>,
    #[serde(default)]
    enabled: Option<bool>,
}

#[derive(Deserialize, Default)]
struct UpdateTriggerReq {
    #[serde(default)]
    channel: Option<String>,
    #[serde(default)]
    match_chat: Option<String>,
    #[serde(default)]
    keyword: Option<String>,
    #[serde(default, deserialize_with = "crate::types::de_double_option")]
    repo_path: Option<Option<String>>,
    #[serde(default)]
    auto_start: Option<bool>,
    #[serde(default)]
    reply: Option<bool>,
    #[serde(default)]
    enabled: Option<bool>,
}

async fn list_triggers_h(
    State(ctx): State<SwarmRt>,
    Extension(user): Extension<AuthUser>,
    Path(sid): Path<Id>,
) -> ApiResult<Json<Vec<SwarmChannelTrigger>>> {
    let swarm = ctx.swarm_repo().get_swarm(&sid).await.map_err(ApiError)?;
    check(&ctx, &user, &swarm.workspace_id, WorkspaceRole::Viewer).await?;
    Ok(Json(
        ctx.swarm_repo()
            .list_triggers(&sid)
            .await
            .map_err(ApiError)?,
    ))
}

async fn create_trigger_h(
    State(ctx): State<SwarmRt>,
    Extension(user): Extension<AuthUser>,
    Path(sid): Path<Id>,
    Json(req): Json<CreateTriggerReq>,
) -> ApiResult<Json<SwarmChannelTrigger>> {
    let swarm = ctx.swarm_repo().get_swarm(&sid).await.map_err(ApiError)?;
    check(&ctx, &user, &swarm.workspace_id, WorkspaceRole::Editor).await?;
    let t = ctx
        .swarm_repo()
        .create_trigger(NewTrigger {
            swarm_id: swarm.id.clone(),
            workspace_id: swarm.workspace_id.clone(),
            channel: req.channel,
            match_chat: req.match_chat.unwrap_or_default(),
            keyword: req.keyword.unwrap_or_default(),
            repo_path: req.repo_path,
            auto_start: req.auto_start.unwrap_or(true),
            reply: req.reply.unwrap_or(true),
            enabled: req.enabled.unwrap_or(true),
            created_by: user.0.id.clone(),
        })
        .await
        .map_err(ApiError)?;
    Ok(Json(t))
}

async fn update_trigger_h(
    State(ctx): State<SwarmRt>,
    Extension(user): Extension<AuthUser>,
    Path(tid): Path<Id>,
    Json(req): Json<UpdateTriggerReq>,
) -> ApiResult<Json<SwarmChannelTrigger>> {
    let cur = ctx.swarm_repo().get_trigger(&tid).await.map_err(ApiError)?;
    check(&ctx, &user, &cur.workspace_id, WorkspaceRole::Editor).await?;
    let t = ctx
        .swarm_repo()
        .update_trigger(
            &tid,
            TriggerPatch {
                channel: req.channel,
                match_chat: req.match_chat,
                keyword: req.keyword,
                repo_path: req.repo_path,
                auto_start: req.auto_start,
                reply: req.reply,
                enabled: req.enabled,
            },
        )
        .await
        .map_err(ApiError)?;
    Ok(Json(t))
}

async fn delete_trigger_h(
    State(ctx): State<SwarmRt>,
    Extension(user): Extension<AuthUser>,
    Path(tid): Path<Id>,
) -> ApiResult<Json<serde_json::Value>> {
    let cur = ctx.swarm_repo().get_trigger(&tid).await.map_err(ApiError)?;
    check(&ctx, &user, &cur.workspace_id, WorkspaceRole::Editor).await?;
    ctx.swarm_repo()
        .delete_trigger(&tid)
        .await
        .map_err(ApiError)?;
    Ok(Json(json!({})))
}

/// Stop an in-flight plan/recruit for this swarm: kills the live agent
/// session(s) and prevents further retries.
#[derive(serde::Deserialize)]
struct AgentStopQuery {
    /// `plan` | `recruit` — stop only that turn (S17-307); absent stops both.
    kind: Option<String>,
}

async fn agent_stop(
    State(ctx): State<SwarmRt>,
    Extension(user): Extension<AuthUser>,
    Path((ws, sid)): Path<(Id, Id)>,
    axum::extract::Query(q): axum::extract::Query<AgentStopQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    swarm_in_ws(&ctx, &user, &ws, &sid, WorkspaceRole::Editor).await?;
    let kind = q.kind.as_deref().filter(|k| !k.is_empty());
    if let Some(k) = kind {
        if !crate::runtime::agent_run::AGENT_KINDS.contains(&k) {
            return Err(ApiError(Error::Invalid(format!(
                "unknown agent kind '{k}' (want plan|recruit)"
            ))));
        }
    }
    let stopped = crate::runtime::agent_run::stop(&ctx, &sid, kind).await;
    Ok(Json(json!({ "ok": true, "stopped": stopped })))
}

async fn check(ctx: &SwarmRt, user: &AuthUser, ws: &Id, role: WorkspaceRole) -> ApiResult<()> {
    ctx.roles().check(&user.0, ws, role).await.map_err(ApiError)
}

/// Role-check `ws` AND require that swarm `sid` lives in it (S4-01). The
/// lifecycle routes are keyed by the path workspace, but every repo call below
/// them is keyed by `sid` alone — without this an Editor of workspace A could
/// abort/plan/recruit against workspace B's swarm. A foreign swarm answers 404
/// (indistinguishable from a missing one, so ids don't leak across tenants).
async fn swarm_in_ws(
    ctx: &SwarmRt,
    user: &AuthUser,
    ws: &Id,
    sid: &Id,
    role: WorkspaceRole,
) -> ApiResult<Swarm> {
    check(ctx, user, ws, role).await?;
    let swarm = ctx.swarm_repo().get_swarm(sid).await.map_err(ApiError)?;
    if &swarm.workspace_id != ws {
        return Err(ApiError(Error::NotFound(format!("swarm {sid}"))));
    }
    Ok(swarm)
}

/// Resolve the default agent provider a swarm meta-agent (recruiter / planner /
/// summarizer) should run on: the workspace's `default_provider`, else the global
/// `default_provider` setting, else "claude". Keeps these coordinator-spawned
/// sessions on the user's configured default instead of a bare "claude" literal.
async fn swarm_meta_provider(ctx: &SwarmRt, ws: &otto_core::domain::Workspace) -> String {
    ctx.resolve_provider_or_fallback(Some(ws), None, "swarm_meta_provider")
        .await
}

async fn start(
    State(ctx): State<SwarmRt>,
    Extension(user): Extension<AuthUser>,
    Path((ws, sid)): Path<(Id, Id)>,
) -> ApiResult<Json<Swarm>> {
    swarm_in_ws(&ctx, &user, &ws, &sid, WorkspaceRole::Editor).await?;
    let _operation = operation_guard(&sid).await;

    // Point-of-action budget gate (A2): check workspace-level cap before the
    // Coordinator starts scheduling runs. Mirrors the review start_review gate.
    {
        if let Some(reason) = ctx.budget_blocked(&ws).await {
            return Err(ApiError(Error::Invalid(format!(
                "Budget exceeded — swarm blocked: {}",
                reason.unwrap_or_else(|| "cap reached".to_string())
            ))));
        }
    }

    ctx.swarm_repo()
        .set_swarm_status(&sid, "active")
        .await
        .map_err(ApiError)?;
    start_coordinator(ctx.clone(), sid.clone());
    emit_status(&ctx, &ws, &sid, "active");
    Ok(Json(
        ctx.swarm_repo().get_swarm(&sid).await.map_err(ApiError)?,
    ))
}

async fn pause(
    State(ctx): State<SwarmRt>,
    Extension(user): Extension<AuthUser>,
    Path((ws, sid)): Path<(Id, Id)>,
) -> ApiResult<Json<Swarm>> {
    swarm_in_ws(&ctx, &user, &ws, &sid, WorkspaceRole::Editor).await?;
    let _operation = operation_guard(&sid).await;
    ctx.swarm_repo()
        .set_swarm_status(&sid, "paused")
        .await
        .map_err(ApiError)?;
    set_paused(&ctx, &sid, true);
    crate::runtime::verify::stop_swarm(&ctx, &sid).await;
    // In-flight turns end first (their tasks re-queue for the resume) so the
    // retry loop can't respawn them; then suspend the sessions to free RAM
    // (resume-friendly).
    stop_runs_for_pause(&ctx, &sid).await;
    for s in swarm_session_ids(&ctx, &ws, &sid).await {
        let _ = ctx.manager().suspend(&s).await;
    }
    emit_status(&ctx, &ws, &sid, "paused");
    Ok(Json(
        ctx.swarm_repo().get_swarm(&sid).await.map_err(ApiError)?,
    ))
}

async fn abort(
    State(ctx): State<SwarmRt>,
    Extension(user): Extension<AuthUser>,
    Path((ws, sid)): Path<(Id, Id)>,
) -> ApiResult<Json<Swarm>> {
    swarm_in_ws(&ctx, &user, &ws, &sid, WorkspaceRole::Editor).await?;
    let _operation = operation_guard(&sid).await;
    ctx.swarm_repo()
        .set_swarm_status(&sid, "aborted")
        .await
        .map_err(ApiError)?;
    stop_coordinator(&ctx, &sid);
    // Stop any in-flight verification controllers (own cancel + kill verify/fix
    // sessions, short-circuiting run_swarm_agent retries; review B3).
    crate::runtime::verify::stop_swarm(&ctx, &sid).await;
    // Cancel in-flight runs and mark them stopped.
    let stopped = ctx
        .swarm_repo()
        .stop_active_runs(&sid)
        .await
        .map_err(ApiError)?;
    for rid in &stopped {
        run::signal_cancel(ctx.swarm_run_cancels(), rid);
    }
    // Kill swarm sessions.
    for s in swarm_session_ids(&ctx, &ws, &sid).await {
        let _ = ctx.manager().kill_session(&s).await;
    }
    emit_status(&ctx, &ws, &sid, "aborted");
    Ok(Json(
        ctx.swarm_repo().get_swarm(&sid).await.map_err(ApiError)?,
    ))
}

async fn resume(
    State(ctx): State<SwarmRt>,
    Extension(user): Extension<AuthUser>,
    Path((ws, sid)): Path<(Id, Id)>,
) -> ApiResult<Json<Swarm>> {
    swarm_in_ws(&ctx, &user, &ws, &sid, WorkspaceRole::Editor).await?;
    let _operation = operation_guard(&sid).await;

    // Point-of-action budget gate (A2): also checked on resume (a pause may have
    // been triggered by a BudgetExceeded event; block the resume when still over cap).
    {
        if let Some(reason) = ctx.budget_blocked(&ws).await {
            return Err(ApiError(Error::Invalid(format!(
                "Budget exceeded — swarm resume blocked: {}",
                reason.unwrap_or_else(|| "cap reached".to_string())
            ))));
        }
    }

    ctx.swarm_repo()
        .set_swarm_status(&sid, "active")
        .await
        .map_err(ApiError)?;
    set_paused(&ctx, &sid, false);
    start_coordinator(ctx.clone(), sid.clone());
    emit_status(&ctx, &ws, &sid, "active");
    Ok(Json(
        ctx.swarm_repo().get_swarm(&sid).await.map_err(ApiError)?,
    ))
}

pub fn emit_status(ctx: &SwarmRt, ws: &Id, sid: &str, status: &str) {
    let _ = ctx.events().send(Event::SwarmStatus {
        workspace_id: ws.clone(),
        swarm_id: sid.to_string(),
        status: status.to_string(),
    });
}

/// Board-utilization snapshot: parallel cap vs live runs, schedulable (ready)
/// vs open work, and which agents are busy/idle. The manager's 5-minute
/// utilization check (and the `swarm_utilization` MCP tool) read this to
/// decide whether capacity is being wasted.
async fn utilization_h(
    State(ctx): State<SwarmRt>,
    Extension(user): Extension<AuthUser>,
    Path(sid): Path<Id>,
) -> ApiResult<Json<serde_json::Value>> {
    let swarm = ctx.swarm_repo().get_swarm(&sid).await.map_err(ApiError)?;
    check(&ctx, &user, &swarm.workspace_id, WorkspaceRole::Viewer).await?;
    let cap = swarm
        .config
        .get("max_parallel_sessions")
        .and_then(|v| v.as_i64())
        .unwrap_or(4)
        .max(1);
    let active = ctx
        .swarm_repo()
        .active_run_count(&sid)
        .await
        .map_err(ApiError)?;
    let ready = ctx
        .swarm_repo()
        .ready_tasks(&sid)
        .await
        .map_err(ApiError)?
        .len();
    // Counted in SQL and one busy-agents read (perf §15 F2/F3) — no task row
    // decoded, no query per agent.
    let by_status = ctx
        .swarm_repo()
        .task_status_counts(&sid)
        .await
        .map_err(ApiError)?;
    let busy_set = ctx.swarm_repo().busy_agents(&sid).await.map_err(ApiError)?;
    let mut agents_out = Vec::new();
    for a in ctx.swarm_repo().list_agents(&sid).await.map_err(ApiError)? {
        let busy = busy_set.contains(&a.id);
        agents_out.push(json!({
            "id": a.id, "name": a.name, "title": a.title,
            "status": a.status, "active_run": busy,
        }));
    }
    Ok(Json(json!({
        "swarm_id": sid,
        "status": swarm.status,
        "parallel_cap": cap,
        "active_runs": active,
        "ready_tasks": ready,
        "tasks_by_status": by_status,
        "agents": agents_out,
        // 12-mcp W1: why each ready task isn't starting (last coordinator tick).
        "waiting": waiting_for(&sid),
    })))
}

/// Just the coordinator's in-memory waiting reasons (perf §15 F2) — what the
/// Kanban's "why isn't this starting" chips read. No DB work beyond the
/// swarm/auth lookup, unlike the full utilization snapshot.
async fn waiting_h(
    State(ctx): State<SwarmRt>,
    Extension(user): Extension<AuthUser>,
    Path(sid): Path<Id>,
) -> ApiResult<Json<serde_json::Value>> {
    let swarm = ctx.swarm_repo().get_swarm(&sid).await.map_err(ApiError)?;
    check(&ctx, &user, &swarm.workspace_id, WorkspaceRole::Viewer).await?;
    Ok(Json(
        json!({ "swarm_id": sid, "waiting": waiting_for(&sid) }),
    ))
}

/// Clear a project's board: stop + cancel every in-flight run for the project
/// (so finishing turns can't repopulate it), delete all its tasks and the
/// project-scoped feed, and broadcast one `SwarmProjectCleared` so every open
/// client drops its local state. The project itself (and the run history —
/// spend accounting) stays.
async fn clear_project_h(
    State(ctx): State<SwarmRt>,
    Extension(user): Extension<AuthUser>,
    Path(pid): Path<Id>,
) -> ApiResult<Json<serde_json::Value>> {
    let project = ctx.swarm_repo().get_project(&pid).await.map_err(ApiError)?;
    check(&ctx, &user, &project.workspace_id, WorkspaceRole::Editor).await?;
    let _operation = operation_guard(&project.swarm_id).await;
    let stopped = ctx
        .swarm_repo()
        .stop_active_runs_for_project(&pid)
        .await
        .map_err(ApiError)?;
    for rid in &stopped {
        run::signal_cancel(ctx.swarm_run_cancels(), rid);
        // Stop the agent itself — the flag alone left it working (and
        // burning tokens) in its worktree on a board that no longer exists.
        if let Some(sid) = ctx
            .swarm_repo()
            .get_run(rid)
            .await
            .ok()
            .and_then(|r| r.session_id)
        {
            let _ = ctx.manager().kill_session(&sid).await;
        }
        run::emit_run(&ctx, rid).await;
    }
    let (tasks_deleted, messages_deleted) = ctx
        .swarm_repo()
        .clear_project_board(&pid)
        .await
        .map_err(ApiError)?;
    let _ = ctx.events().send(Event::SwarmProjectCleared {
        workspace_id: project.workspace_id.clone(),
        swarm_id: project.swarm_id.clone(),
        project_id: pid.clone(),
    });
    Ok(Json(json!({
        "ok": true,
        "runs_stopped": stopped.len(),
        "tasks_deleted": tasks_deleted,
        "messages_deleted": messages_deleted,
    })))
}

async fn run_task(
    State(ctx): State<SwarmRt>,
    Extension(user): Extension<AuthUser>,
    Path(tid): Path<Id>,
) -> ApiResult<Json<otto_state::SwarmRun>> {
    let task = ctx.swarm_repo().get_task(&tid).await.map_err(ApiError)?;
    check(&ctx, &user, &task.workspace_id, WorkspaceRole::Editor).await?;
    let _operation = operation_guard(&task.swarm_id).await;
    let swarm = ctx
        .swarm_repo()
        .get_swarm(&task.swarm_id)
        .await
        .map_err(ApiError)?;
    // The same gates the coordinator applies: a manual (or manager-agent
    // `swarm_run_task`) run must not start a second turn for a task already
    // running, run on an aborted swarm, or bypass the budget pause. (A swarm
    // that is merely paused — including a new one, which starts paused — may
    // still run a task by hand.)
    if !matches!(task.status.as_str(), "todo" | "blocked" | "backlog") {
        return Err(ApiError(Error::Conflict(format!(
            "task is {} — only a todo, blocked or backlog task can be run",
            task.status
        ))));
    }
    if swarm.status == "aborted" {
        return Err(ApiError(Error::Conflict(
            "swarm is aborted — tasks can't run".into(),
        )));
    }
    if let Some(reason) = swarm.pause_reason.as_deref().filter(|r| !r.is_empty()) {
        return Err(ApiError(Error::Conflict(format!(
            "swarm is paused: {reason} — raise the budget and resume first"
        ))));
    }
    if let Some(reason) = budget_exceeded(&ctx, &swarm).await {
        return Err(ApiError(Error::Conflict(format!(
            "swarm budget exhausted: {reason}"
        ))));
    }
    let agent = pick_agent(&ctx, &swarm, &task)
        .await
        .ok_or_else(|| ApiError(Error::Invalid("no active agent to run this task".into())))?;
    if ctx
        .swarm_repo()
        .agent_has_active_run(&agent.id)
        .await
        .unwrap_or(false)
        || crate::runtime::verify::agent_under_verification(&agent.id)
    {
        return Err(ApiError(Error::Conflict(format!(
            "{} is busy with another turn — try again when it finishes",
            agent.name
        ))));
    }
    let is_leader = has_reports(&ctx, &swarm.id, &agent.id).await;
    let kind = if is_leader && !task.delegated {
        "planning"
    } else {
        "task"
    };
    let run = ctx
        .swarm_repo()
        .reserve_run(
            NewRun {
                swarm_id: swarm.id.clone(),
                workspace_id: swarm.workspace_id.clone(),
                project_id: Some(task.project_id.clone()),
                task_id: Some(task.id.clone()),
                agent_id: agent.id.clone(),
                kind: kind.to_string(),
                trigger: "manual".to_string(),
            },
            true,
        )
        .await
        .map_err(ApiError)?;
    // Count the attempt only once a slot was actually reserved (S4-18): a
    // run refused at capacity (Conflict) must not walk the task toward its
    // attempt ceiling — the coordinator bumps after its reserve too.
    let _ = ctx.swarm_repo().bump_task_attempt(&tid).await;
    let _ = ctx
        .swarm_repo()
        .update_task(
            &tid,
            TaskPatch {
                status: Some("in_progress".into()),
                // Persist the pick on an unassigned task (see coordinator claim).
                assignee_agent_id: task
                    .assignee_agent_id
                    .is_none()
                    .then(|| Some(agent.id.clone())),
                ..Default::default()
            },
        )
        .await;
    emit_task(&ctx, &tid).await;
    let ctx2 = ctx.clone();
    let run2 = run.clone();
    let task2 = task.clone();
    tokio::spawn(async move {
        let result = run::run_turn(ctx2.clone(), run2.clone()).await;
        route_result(&ctx2, &run2, &task2, result).await;
    });
    Ok(Json(run))
}

async fn stop_run(
    State(ctx): State<SwarmRt>,
    Extension(user): Extension<AuthUser>,
    Path(rid): Path<Id>,
) -> ApiResult<Json<otto_state::SwarmRun>> {
    let run = ctx.swarm_repo().get_run(&rid).await.map_err(ApiError)?;
    check(&ctx, &user, &run.workspace_id, WorkspaceRole::Editor).await?;
    let _operation = operation_guard(&run.swarm_id).await;
    run::signal_cancel(ctx.swarm_run_cancels(), &rid);
    let stopped = ctx
        .swarm_repo()
        .update_run_if_status(
            &rid,
            &["queued", "running", "waiting"],
            RunPatch {
                status: Some("stopped".into()),
                finished_at: Some(Some(Utc::now())),
                ..Default::default()
            },
        )
        .await
        .ok()
        .flatten();
    // Stop the agent, not just the row: the session kept working for up to
    // the whole turn, and the slot freed by `stopped` then pasted the next
    // brief into this same busy session.
    if let Some(sid) = stopped.and_then(|r| r.session_id) {
        let _ = ctx.manager().kill_session(&sid).await;
    }
    run::emit_run(&ctx, &rid).await;
    Ok(Json(
        ctx.swarm_repo().get_run(&rid).await.map_err(ApiError)?,
    ))
}

async fn recruit(
    State(ctx): State<SwarmRt>,
    Extension(user): Extension<AuthUser>,
    Path(ws): Path<Id>,
    Json(req): Json<crate::RecruitReq>,
) -> ApiResult<Json<crate::RecruitedAgent>> {
    check(&ctx, &user, &ws, WorkspaceRole::Editor).await?;
    let (swarm_name, mission, titles) = match &req.swarm_id {
        Some(sid) => {
            // S4-01: never read (or recruit into) another workspace's swarm.
            let s = swarm_in_ws(&ctx, &user, &ws, sid, WorkspaceRole::Editor).await?;
            let titles = ctx
                .swarm_repo()
                .list_agents(sid)
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|a| a.title)
                .collect::<Vec<_>>();
            (s.name, s.description, titles)
        }
        None => ("New swarm".to_string(), String::new(), Vec::new()),
    };
    // Collect ALL known skill names so we can validate the recruiter's reply,
    // but only inject a bounded subset into the prompt.  Injecting the full
    // library (potentially hundreds of skills) wastes tokens and can produce
    // bloated, irrelevant skill lists.  `cap_skills_for_role` ranks by name-
    // relevance to the requested role and hard-caps at `RECRUITER_SKILL_CAP`.
    let all_skills: Vec<String> = ctx
        .context_library()
        .list_skills()
        .into_iter()
        .map(|s| s.name)
        .collect();
    let capped_skills = crate::recruiter::cap_skills_for_role(
        &all_skills,
        &req.role,
        crate::recruiter::RECRUITER_SKILL_CAP,
    );
    tracing::debug!(
        "recruiter: injecting {} / {} skills into prompt (cap={})",
        capped_skills.len(),
        all_skills.len(),
        crate::recruiter::RECRUITER_SKILL_CAP
    );
    let providers = ctx.available_providers();
    // The provider the recruiter meta-agent itself runs on — the configured
    // default (workspace → global → "claude"), not a bare literal.
    let workspace = ctx.workspaces().get(&ws).await.map_err(ApiError)?;
    let meta_provider = swarm_meta_provider(&ctx, &workspace).await;
    let prompt = crate::recruiter::recruiter_prompt(
        &req.role,
        &swarm_name,
        &mission,
        &titles,
        &capped_skills,
        &providers,
        req.context.as_deref(),
        req.naming_theme.as_deref(),
    );
    let cwd = std::env::temp_dir().to_string_lossy().to_string();
    // When recruiting into an existing swarm, run as a REAL, openable session
    // (watchable live + Stop-able). With no swarm yet (brand-new), fall back to a
    // headless one-shot turn (nothing to attach a run/session to).
    let (reply, run_id): (String, Option<Id>) = match &req.swarm_id {
        Some(sid) => {
            let nominal = ctx
                .swarm_repo()
                .list_agents(sid)
                .await
                .unwrap_or_default()
                .first()
                .map(|a| a.id.clone())
                .unwrap_or_else(|| "recruiter".to_string());
            let cancel = crate::runtime::agent_run::begin(sid, "recruit").map_err(ApiError)?;
            let (raw, rid) = crate::runtime::agent_run::run_swarm_agent(
                &ctx,
                &workspace,
                &user.0,
                sid,
                None,
                None,
                &nominal,
                &meta_provider,
                None,
                "recruit",
                &format!("Recruit: {}", req.role),
                &cwd,
                &prompt,
                |t| crate::recruiter::parse_recruited(t).is_some(),
                &cancel,
            )
            .await;
            crate::runtime::agent_run::end(sid, "recruit");
            let raw = raw.ok_or_else(|| {
                ApiError(Error::Upstream(
                    "recruiter produced nothing (stopped or stuck)".into(),
                ))
            })?;
            (raw, Some(rid))
        }
        None => (
            ctx.orchestrator()
                .run_agent(&prompt, &cwd, None, AGENT_NO_PROGRESS)
                .await
                .map_err(ApiError)?,
            None,
        ),
    };
    let mut recruited = crate::recruiter::parse_recruited(&reply).ok_or_else(|| {
        ApiError(Error::Upstream(
            "recruiter returned no usable definition".into(),
        ))
    })?;
    // Validate skills against the FULL library (not just the capped list); any
    // skill the recruiter invents that is not in the real library is dropped.
    let known: std::collections::HashSet<String> = all_skills.into_iter().collect();
    recruited.skills.retain(|s| known.contains(&s.name));
    // Force the provider to an available one. Prefer the configured default when
    // it's actually available, else the first available provider, else "claude".
    if !providers.iter().any(|p| p == &recruited.suggested_provider) {
        recruited.suggested_provider = if providers.iter().any(|p| p == &meta_provider) {
            meta_provider.clone()
        } else {
            providers
                .first()
                .cloned()
                .unwrap_or_else(|| "claude".into())
        };
    }
    // Persist the proposal on the run so it can be hired straight from the Runs
    // list even if the Recruit modal was closed while the agent worked.
    if let Some(rid) = run_id {
        let _ = ctx
            .swarm_repo()
            .update_run(
                &rid,
                RunPatch {
                    result: Some(Some(serde_json::to_value(&recruited).unwrap_or_default())),
                    ..Default::default()
                },
            )
            .await;
        crate::runtime::run::emit_run(&ctx, &rid).await;
    }
    Ok(Json(recruited))
}

async fn plan(
    State(ctx): State<SwarmRt>,
    Extension(user): Extension<AuthUser>,
    Path((ws, pid)): Path<(Id, Id)>,
    Json(_req): Json<crate::PlanReq>,
) -> ApiResult<Json<Vec<SwarmTask>>> {
    check(&ctx, &user, &ws, WorkspaceRole::Editor).await?;
    let project = ctx.swarm_repo().get_project(&pid).await.map_err(ApiError)?;
    // S4-01: the project must belong to the path workspace — `plan` spawns
    // planners in the project's repo and writes tasks into its swarm.
    if project.workspace_id != ws {
        return Err(ApiError(Error::NotFound(format!("project {pid}"))));
    }
    let goal = project.goal_md.clone().unwrap_or_default();
    if goal.trim().is_empty() {
        return Err(ApiError(Error::Invalid(
            "project has no goal to plan".into(),
        )));
    }
    let agents = ctx
        .swarm_repo()
        .list_agents(&project.swarm_id)
        .await
        .unwrap_or_default();
    let preset_agents: Vec<crate::PresetAgent> = agents
        .iter()
        .map(|a| crate::PresetAgent {
            key: a.id.clone(),
            name: a.name.clone(),
            title: a.title.clone(),
            reports_to: None,
            provider: a.provider.clone(),
            specialization: a.specialization.clone(),
        })
        .collect();
    // Expand tilde-form repo paths (older projects persisted them raw): a
    // literal `~` cwd makes the planner session spawn fall back to $HOME while
    // watch_for_result polls a transcript dir derived from the raw string — the
    // completed turn is never seen and the plan run churns until stuck.
    let cwd = project
        .repo_path
        .as_deref()
        .map(otto_core::paths::expand_tilde)
        .unwrap_or_else(|| std::env::temp_dir().to_string_lossy().to_string());
    let ws_obj = ctx.workspaces().get(&ws).await.map_err(ApiError)?;
    // The provider the planner/summarizer meta-agents run on — the configured
    // default (workspace → global → "claude").
    let meta_provider = swarm_meta_provider(&ctx, &ws_obj).await;
    let nominal_agent = agents
        .first()
        .map(|a| a.id.clone())
        .unwrap_or_else(|| "planner".to_string());

    // Multi-agent plan: run one planner per angle as a REAL, openable session
    // (watchable live in the Runs list, Stop-able), then a summarizer reconciles
    // the candidate task lists. Each turn has no wall-clock cap + stuck-retry.
    let cancel = crate::runtime::agent_run::begin(&project.swarm_id, "plan").map_err(ApiError)?;
    let mut candidates: Vec<String> = Vec::new();
    let angles = crate::recruiter::PLANNER_ANGLES;
    for (i, angle) in angles.iter().enumerate() {
        let prompt = crate::recruiter::planner_prompt(&project.name, &goal, &preset_agents, angle);
        let title = format!("Plan {}/{}: {}", i + 1, angles.len(), project.name);
        let (raw, _) = crate::runtime::agent_run::run_swarm_agent(
            &ctx,
            &ws_obj,
            &user.0,
            &project.swarm_id,
            Some(&project.id),
            None,
            &nominal_agent,
            &meta_provider,
            None,
            "plan",
            &title,
            &cwd,
            &prompt,
            |t| crate::recruiter::extract_json(t).is_some(),
            &cancel,
        )
        .await;
        if let Some(raw) = raw {
            if crate::recruiter::extract_json(&raw).is_some() {
                candidates.push(raw);
            }
        }
    }
    let final_json = if candidates.len() > 1 {
        let sum_prompt = crate::recruiter::planner_summarizer_prompt(
            &project.name,
            &goal,
            &preset_agents,
            &candidates,
        );
        let (raw, _) = crate::runtime::agent_run::run_swarm_agent(
            &ctx,
            &ws_obj,
            &user.0,
            &project.swarm_id,
            Some(&project.id),
            None,
            &nominal_agent,
            &meta_provider,
            None,
            "plan",
            &format!("Plan summary: {}", project.name),
            &cwd,
            &sum_prompt,
            |t| crate::recruiter::extract_json(t).is_some(),
            &cancel,
        )
        .await;
        raw.and_then(|r| crate::recruiter::extract_json(&r))
            .or_else(|| crate::recruiter::extract_json(&candidates[0]))
    } else {
        candidates
            .first()
            .and_then(|c| crate::recruiter::extract_json(c))
    };
    crate::runtime::agent_run::end(&project.swarm_id, "plan");
    let v = final_json.ok_or_else(|| {
        ApiError(Error::Upstream(
            "planner produced no tasks (stopped or stuck)".into(),
        ))
    })?;
    let tasks_json = v
        .get("tasks")
        .and_then(|t| t.as_array())
        .cloned()
        .unwrap_or_default();

    // Two passes: create tasks, then wire depends_on by matching titles.
    let mut created: Vec<SwarmTask> = Vec::new();
    let mut by_title: HashMap<String, Id> = HashMap::new();
    for (i, t) in tasks_json.iter().enumerate() {
        let title = t
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if title.is_empty() {
            continue;
        }
        let description = t
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let priority = t
            .get("priority")
            .and_then(|v| v.as_str())
            .unwrap_or("medium")
            .to_string();
        let mut assignee = t
            .get("assignee_title")
            .and_then(|v| v.as_str())
            .and_then(|title| {
                agents
                    .iter()
                    .find(|a| a.title.eq_ignore_ascii_case(title.trim()))
                    .map(|a| a.id.clone())
            });
        if assignee.is_none() {
            // Planner named a role that doesn't exist (or none) — best-fit instead
            // of leaving the card unassigned.
            assignee =
                best_fit_agent_id(&ctx, &project.swarm_id, &format!("{title} {description}")).await;
        }
        if let Ok(task) = ctx
            .swarm_repo()
            .create_task(NewTask {
                project_id: project.id.clone(),
                swarm_id: project.swarm_id.clone(),
                workspace_id: project.workspace_id.clone(),
                title: title.clone(),
                description,
                assignee_agent_id: assignee,
                status: "todo".into(),
                priority,
                parent_task_id: None,
                depends_on: json!([]),
                labels: json!([]),
                order_idx: i as i64,
                created_by: user.0.id.clone(),
            })
            .await
        {
            // Live-update open boards: without this, plan-created tasks (and the
            // column counts) only appear after a manual reload.
            emit_task(&ctx, &task.id).await;
            by_title.insert(title.to_lowercase(), task.id.clone());
            created.push(task);
        }
    }
    // Wire dependencies.
    for (t, created_task) in tasks_json.iter().zip(created.iter()) {
        if let Some(deps) = t.get("depends_on_titles").and_then(|v| v.as_array()) {
            let dep_ids: Vec<String> = deps
                .iter()
                .filter_map(|d| d.as_str())
                .filter_map(|d| by_title.get(&d.to_lowercase()).cloned())
                .collect();
            if !dep_ids.is_empty() {
                let _ = ctx
                    .swarm_repo()
                    .update_task(
                        &created_task.id,
                        TaskPatch {
                            depends_on: Some(json!(dep_ids)),
                            ..Default::default()
                        },
                    )
                    .await;
                emit_task(&ctx, &created_task.id).await;
            }
        }
    }
    let result = ctx.swarm_repo().list_tasks(&pid).await.map_err(ApiError)?;
    Ok(Json(result))
}

#[cfg(test)]
mod waiting_tests {
    use super::*;

    #[test]
    fn waiting_keeps_since_per_code_and_clears_started_tasks() {
        let sid: Id = "swarm-waiting-test".into();
        let mut fresh = HashMap::new();
        fresh.insert("t1".to_string(), ("agent_busy", "Dev is busy".to_string()));
        fresh.insert("t2".to_string(), ("capacity", "All 2/2".to_string()));
        set_waiting(&sid, fresh);
        let first = waiting_for(&sid);
        assert_eq!(first.len(), 2);
        std::thread::sleep(std::time::Duration::from_millis(5));

        // t1 still busy (same code → same `since`), t2 now waits for another
        // reason (new `since`), t3 started (gone).
        let mut fresh = HashMap::new();
        fresh.insert("t1".to_string(), ("agent_busy", "Dev is busy".to_string()));
        fresh.insert("t2".to_string(), ("run_budget", "Run budget".to_string()));
        set_waiting(&sid, fresh);
        let second = waiting_for(&sid);
        assert_eq!(second["t1"].since, first["t1"].since);
        assert!(second["t2"].since > first["t2"].since);
        assert_eq!(second["t2"].code, "run_budget");

        set_waiting(&sid, HashMap::new());
        assert!(waiting_for(&sid).is_empty());
    }
}

#[cfg(test)]
mod row_workspace_guard {
    /// Every handler in `code` whose path extractor is a TUPLE (`Path<(…`)
    /// — i.e. a path workspace plus a row id, however the binding is spelled
    /// (`Path((ws, sid))`, `Path((ws_id, pid))`, `Path((_ws, x))`,
    /// `Path(p)`) — must tie the row to that workspace. Returns the names of
    /// the offenders and the number of handlers checked.
    fn scan(code: &str) -> (Vec<String>, usize) {
        let mut bad = Vec::new();
        let mut checked = 0;
        for (i, _) in code.match_indices("Path<(") {
            let Some(fn_start) = code[..i].rfind("async fn ") else {
                continue;
            };
            let name = code[fn_start + 9..]
                .split(['(', '<'])
                .next()
                .unwrap()
                .to_string();
            let body_end = ["\nasync fn ", "\nfn ", "\npub fn ", "\npub async fn "]
                .iter()
                .filter_map(|m| code[i..].find(m))
                .min()
                .map_or(code.len(), |e| i + e);
            let body = &code[i..body_end];
            if !(body.contains("swarm_in_ws(") || body.contains("workspace_id != ")) {
                bad.push(name);
            }
            checked += 1;
        }
        (bad, checked)
    }

    /// Guard (S4-01 / S4-307): every handler that takes BOTH a path workspace
    /// and a row id must tie the row to that workspace — `swarm_in_ws(…)` or
    /// an explicit `workspace_id != …` check — because the role check alone
    /// only covers the path workspace. Scans the runtime engine AND the CRUD
    /// router; handlers keyed by the row alone derive the workspace from the
    /// row and take a non-tuple `Path<Id>`.
    #[test]
    fn workspace_scoped_row_handlers_check_row_ownership() {
        let engine = include_str!("engine.rs");
        let engine = &engine[..engine
            .find("#[cfg(test)]\nmod row_workspace_guard")
            .unwrap()];
        let http = include_str!("../http.rs");
        let http = &http[..http.find("#[cfg(test)]").unwrap_or(http.len())];
        let mut checked = 0;
        for code in [engine, http] {
            let (bad, n) = scan(code);
            assert!(
                bad.is_empty(),
                "Path<(ws, row)> handler(s) never check the row belongs to the workspace: {bad:?}"
            );
            checked += n;
        }
        assert!(
            checked >= 6,
            "guard matched {checked} handlers — pattern drifted?"
        );
    }

    /// The scanner itself catches every binding spelling (S4-307).
    #[test]
    fn scanner_flags_unchecked_handlers_of_any_binding_shape() {
        for sig in [
            "Path((ws, sid)): Path<(Id, Id)>",
            "Path((ws_id, sid)): Path<(Id, Id)>",
            "Path((_ws, pid)): Path<(Id, Id)>",
            "Path(p): Path<(Id, Id)>",
        ] {
            let bad = format!("async fn h(\n    {sig},\n) {{\n    get(p.1)\n}}\n");
            assert_eq!(scan(&bad).0, vec!["h".to_string()], "{sig}");
            let ok =
                format!("async fn h(\n    {sig},\n) {{\n    swarm_in_ws(&rt, &p.0, &p.1)\n}}\n");
            assert!(scan(&ok).0.is_empty(), "{sig}");
        }
    }
}
