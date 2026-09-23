//! The SwarmCoordinator runtime: a per-swarm supervisor that schedules ready
//! tasks onto agents within the parallel-worker cap, runs each turn via
//! `swarm_run::run_turn`, and routes the result (delegation → subtasks, handoffs,
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

use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;
use crate::swarm_run::{self, SwarmTurnResult};

// --- Registry --------------------------------------------------------------

/// A running Coordinator's control handles.
#[derive(Clone)]
pub struct CoordinatorHandle {
    pub cancel: Arc<AtomicBool>,
    pub paused: Arc<AtomicBool>,
}

impl CoordinatorHandle {
    pub fn new() -> Self {
        Self {
            cancel: Arc::new(AtomicBool::new(false)),
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

const TICK: Duration = Duration::from_secs(5);
const SLICE: Duration = Duration::from_millis(500);
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
pub fn start_coordinator(ctx: ServerCtx, swarm_id: Id) {
    let handle = CoordinatorHandle::new();
    {
        let mut reg = ctx.swarm_coords.lock().unwrap();
        if let Some(old) = reg.insert(swarm_id.clone(), handle.clone()) {
            old.cancel.store(true, Ordering::Relaxed);
        }
    }
    // Re-spawn any verification controllers stranded in `verifying` by a restart
    // (review B2), then start the tick loop.
    {
        let ctx = ctx.clone();
        let swarm_id = swarm_id.clone();
        tokio::spawn(async move {
            crate::swarm_verify::recover(&ctx, &swarm_id).await;
        });
    }
    tokio::spawn(coordinator_loop(ctx, swarm_id, handle));
}

/// Stop the Coordinator for a swarm (abort/shutdown).
pub fn stop_coordinator(ctx: &ServerCtx, swarm_id: &str) {
    if let Some(h) = ctx.swarm_coords.lock().unwrap().remove(swarm_id) {
        h.cancel.store(true, Ordering::Relaxed);
    }
}

pub fn set_paused(ctx: &ServerCtx, swarm_id: &str, paused: bool) {
    if let Some(h) = ctx.swarm_coords.lock().unwrap().get(swarm_id) {
        h.paused.store(paused, Ordering::Relaxed);
    }
}

/// One tick at a time per swarm. `start_coordinator` spawns the new loop
/// while the old one may be mid-tick (it only reads `cancel` between ticks),
/// and two overlapping ticks could read the same `todo` task / free agent and
/// dispatch it twice.
fn tick_lock(swarm_id: &str) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    LOCKS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(swarm_id.to_string())
        .or_default()
        .clone()
}

async fn coordinator_loop(ctx: ServerCtx, swarm_id: Id, handle: CoordinatorHandle) {
    loop {
        if handle.cancel.load(Ordering::Relaxed) {
            return;
        }
        if !handle.paused.load(Ordering::Relaxed) {
            let lock = tick_lock(&swarm_id);
            let _ticking = lock.lock().await;
            // Replaced (or stopped) while waiting for the previous loop's tick.
            if handle.cancel.load(Ordering::Relaxed) {
                return;
            }
            if let Err(e) = tick(&ctx, &swarm_id).await {
                // The swarm was deleted (the delete route only drops rows): stop
                // this loop — it used to tick and warn every 5s until restart.
                if matches!(e, Error::NotFound(_))
                    && matches!(
                        ctx.swarm_repo.get_swarm(&swarm_id).await,
                        Err(Error::NotFound(_))
                    )
                {
                    tracing::info!(swarm = %swarm_id, "swarm deleted — stopping its coordinator");
                    let mut reg = ctx.swarm_coords.lock().unwrap();
                    if reg
                        .get(&swarm_id)
                        .is_some_and(|h| Arc::ptr_eq(&h.cancel, &handle.cancel))
                    {
                        reg.remove(&swarm_id);
                    }
                    return;
                }
                tracing::warn!(swarm = %swarm_id, "swarm coordinator tick: {e}");
            }
        }
        let mut waited = Duration::ZERO;
        while waited < TICK {
            if handle.cancel.load(Ordering::Relaxed) {
                return;
            }
            tokio::time::sleep(SLICE).await;
            waited += SLICE;
        }
    }
}

async fn tick(ctx: &ServerCtx, swarm_id: &Id) -> otto_core::Result<()> {
    let repo = &ctx.swarm_repo;
    let swarm = repo.get_swarm(swarm_id).await?;
    if swarm.status != "active" {
        return Ok(());
    }

    // Budget guardrails (D3/D8): before doing anything, check whether any
    // per-swarm budget is exhausted. If so, pause the swarm with a clear reason
    // instead of scheduling more work — the user must raise the budget + resume.
    if let Some(reason) = budget_exceeded(ctx, &swarm).await {
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
    let mut budget = (cap - active).max(0);
    if budget <= 0 {
        return Ok(());
    }

    // Project the run-count budget across this tick. `budget_exceeded` above
    // only checks the budget once, so without this a single tick could enqueue
    // up to `cap` runs and overshoot `max_total_runs` by nearly the concurrency
    // cap. Track the projected total as we schedule and stop when the next run
    // would reach the ceiling. (Cost can't be projected — per-run cost isn't
    // known until the turn completes — so the cost ceiling stays a tick-top gate.)
    let mut projected_total_runs: Option<i64> = if swarm.max_total_runs.is_some() {
        Some(repo.swarm_spend(swarm_id).await?.total_runs)
    } else {
        None
    };

    for task in repo.ready_tasks(swarm_id).await? {
        if budget <= 0 {
            break;
        }
        // Stop scheduling once the projected run count would hit the budget, so
        // a single tick can't overshoot `max_total_runs`.
        if let (Some(max_runs), Some(projected)) = (swarm.max_total_runs, projected_total_runs) {
            if projected >= max_runs {
                break;
            }
        }
        let Some(agent) = pick_agent(ctx, &swarm, &task).await else {
            continue;
        };
        if repo.agent_has_active_run(&agent.id).await.unwrap_or(false) {
            continue; // one turn per agent at a time
        }
        // Don't start another task for an agent whose branch is under verification —
        // a second turn on the same worktree would pollute the diff being verified
        // and the branch about to be merged (review B1).
        if crate::swarm_verify::agent_under_verification(&agent.id) {
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
        let _ = repo.bump_task_attempt(&task.id).await;
        emit_task(ctx, &task.id).await;

        let is_leader = has_reports(ctx, &swarm.id, &agent.id).await;
        let kind = if is_leader && !task.delegated {
            "planning"
        } else {
            "task"
        };
        let run = match repo
            .create_run(NewRun {
                swarm_id: swarm.id.clone(),
                workspace_id: swarm.workspace_id.clone(),
                project_id: Some(task.project_id.clone()),
                task_id: Some(task.id.clone()),
                agent_id: agent.id.clone(),
                kind: kind.to_string(),
                trigger: "coordinator".to_string(),
            })
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
        budget -= 1;
        if let Some(projected) = projected_total_runs.as_mut() {
            *projected += 1;
        }
        swarm_run::emit_run(ctx, &run.id).await;

        let ctx2 = ctx.clone();
        let task2 = task.clone();
        tokio::spawn(async move {
            let result = swarm_run::run_turn(ctx2.clone(), run.clone()).await;
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
async fn budget_exceeded(ctx: &ServerCtx, swarm: &Swarm) -> Option<String> {
    if let (None, None, None) = (
        swarm.max_total_runs,
        swarm.max_cost_usd,
        swarm.max_runtime_secs,
    ) {
        return None;
    }
    let spend = ctx.swarm_repo.swarm_spend(&swarm.id).await.ok()?;
    if let Some(max_runs) = swarm.max_total_runs {
        if spend.total_runs >= max_runs {
            return Some(format!(
                "run budget reached ({}/{} runs)",
                spend.total_runs, max_runs
            ));
        }
    }
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

/// Pause a swarm because a budget was hit: persist status+reason, flip the
/// coordinator's paused flag (so it idles without ticking), suspend idle swarm
/// sessions, post to the board, and notify.
async fn pause_for_budget(ctx: &ServerCtx, swarm: &Swarm, reason: &str) {
    let _ = ctx
        .swarm_repo
        .pause_swarm_with_reason(&swarm.id, reason)
        .await;
    set_paused(ctx, &swarm.id, true);
    stop_runs_for_pause(ctx, &swarm.id).await;
    for s in swarm_session_ids(ctx, &swarm.workspace_id, &swarm.id).await {
        let _ = ctx.manager.suspend(&s).await;
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
    let _ = ctx.events.send(Event::Notice {
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
async fn stop_runs_for_pause(ctx: &ServerCtx, swarm_id: &str) {
    match ctx
        .swarm_repo
        .stop_active_runs_with_reason(&swarm_id.to_string(), PAUSED_RUN_REASON)
        .await
    {
        Ok(ids) => {
            for rid in &ids {
                swarm_run::signal_cancel(&ctx.swarm_run_cancels, rid);
                swarm_run::emit_run(ctx, rid).await;
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
async fn best_fit_agent_id(ctx: &ServerCtx, swarm_id: &str, hay: &str) -> Option<Id> {
    let agents = ctx
        .swarm_repo
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
async fn pick_agent(ctx: &ServerCtx, swarm: &Swarm, task: &SwarmTask) -> Option<SwarmAgent> {
    let repo = &ctx.swarm_repo;
    if let Some(aid) = &task.assignee_agent_id {
        if let Ok(a) = repo.get_agent(aid).await {
            if a.status == "active" {
                return Some(a);
            }
        }
    }
    let agents = repo.list_agents(&swarm.id).await.ok()?;
    let active: Vec<SwarmAgent> = agents
        .into_iter()
        .filter(|a| a.status == "active")
        .collect();
    if active.is_empty() {
        return None;
    }
    let hay = format!("{} {}", task.title, task.description).to_lowercase();
    active
        .iter()
        .cloned()
        .max_by_key(|a| agent_fit_score(a, &hay))
        .or_else(|| active.into_iter().next())
}

async fn has_reports(ctx: &ServerCtx, swarm_id: &str, agent_id: &str) -> bool {
    ctx.swarm_repo
        .list_agents(&swarm_id.to_string())
        .await
        .map(|all| {
            all.iter()
                .any(|a| a.reports_to.as_deref() == Some(agent_id))
        })
        .unwrap_or(false)
}

async fn resolve_agent_by_title(ctx: &ServerCtx, swarm_id: &str, title: &str) -> Option<Id> {
    let want = title.trim().to_lowercase();
    let agents = ctx
        .swarm_repo
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
async fn route_result(
    ctx: &ServerCtx,
    run: &otto_state::SwarmRun,
    task: &SwarmTask,
    result: Option<SwarmTurnResult>,
) {
    let repo = &ctx.swarm_repo;
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
    if current.status != "in_progress" || reassigned {
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
        let _ = ctx.events.send(Event::Notice {
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
            } else if crate::swarm_verify::task_has_goals(ctx, task).await {
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
                crate::swarm_verify::start_verification(ctx, task.clone(), run.agent_id.clone());
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
    ctx: &ServerCtx,
    origin: &SwarmTask,
    run: &otto_state::SwarmRun,
    hops: i64,
    title: &str,
    description: &str,
    assignee: Option<Id>,
    label: &str,
) {
    let repo = &ctx.swarm_repo;
    let open: Vec<SwarmTask> = repo
        .list_tasks(&origin.project_id)
        .await
        .unwrap_or_default()
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
            labels: json!([label, format!("hops:{hops}")]),
            order_idx: 0,
            created_by: run.agent_id.clone(),
        })
        .await
    {
        emit_task(ctx, &task.id).await;
    }
}

/// A chain hit MAX_HANDOFF_HOPS: stop creating tasks, tell the humans.
async fn escalate_chain(ctx: &ServerCtx, task: &SwarmTask, what: &str) {
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
    let _ = ctx.events.send(Event::Notice {
        level: "warn".into(),
        title: "Swarm handoff chain capped".into(),
        body: clip(&body, 160),
    });
}

/// Has a task exhausted its swarm's per-task attempt ceiling? Re-reads the task
/// for the up-to-date attempt counter (the Coordinator bumps it when it queues
/// each turn) and compares against the swarm's `max_attempts` (default 3, min 1).
async fn attempt_ceiling_reached(ctx: &ServerCtx, task: &SwarmTask) -> bool {
    let repo = &ctx.swarm_repo;
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
async fn block_for_attempts(ctx: &ServerCtx, task: &SwarmTask) {
    let repo = &ctx.swarm_repo;
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
    let _ = ctx.events.send(Event::Notice {
        level: "warn".into(),
        title: "Swarm task blocked (attempts)".into(),
        body: clip(&body, 160),
    });
}

async fn create_subtasks(ctx: &ServerCtx, parent: &SwarmTask, subs: &[swarm_run::TurnSubtask]) {
    let repo = &ctx.swarm_repo;
    // Open-title set for dedup + the backstop count: delegation must not
    // re-create board items that already exist (a repeated planning turn used
    // to double every subtask), nor inflate the project past the cap.
    let open: Vec<SwarmTask> = repo
        .list_tasks(&parent.project_id)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|t| !matches!(t.status.as_str(), "done" | "cancelled"))
        .collect();
    let mut titles: std::collections::HashSet<String> =
        open.iter().map(|t| t.title.trim().to_lowercase()).collect();
    let mut open_count = open.len();
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
    ctx: &ServerCtx,
    task: &SwarmTask,
    run: &otto_state::SwarmRun,
    res: &SwarmTurnResult,
) -> usize {
    let repo = &ctx.swarm_repo;
    let manager = repo
        .get_agent(&run.agent_id)
        .await
        .ok()
        .and_then(|a| a.reports_to);
    let mut created = 0usize;
    for rv in &res.reviews {
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
            labels: json!(["review"]),
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
pub(crate) async fn complete_parent_if_done(ctx: &ServerCtx, task: &SwarmTask) {
    let repo = &ctx.swarm_repo;
    let Some(parent_id) = &task.parent_task_id else {
        return;
    };
    if repo.children_complete(parent_id).await.unwrap_or(false) {
        if let Ok(parent) = repo.get_task(parent_id).await {
            if parent.status != "done" {
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
    ctx: &ServerCtx,
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
    ctx: &ServerCtx,
    swarm_id: &str,
    project_id: Option<&str>,
    task_id: Option<&str>,
    kind: &str,
    body: &str,
    meta: serde_json::Value,
) {
    let swarm = match ctx.swarm_repo.get_swarm(&swarm_id.to_string()).await {
        Ok(s) => s,
        Err(_) => return,
    };
    if let Ok(msg) = ctx
        .swarm_repo
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
        let _ = ctx.events.send(Event::SwarmMessagePosted {
            workspace_id: swarm.workspace_id,
            swarm_id: swarm_id.to_string(),
            message: serde_json::to_value(&msg).unwrap_or_default(),
        });
    }
}

async fn emit_task(ctx: &ServerCtx, task_id: &str) {
    if let Ok(t) = ctx.swarm_repo.get_task(&task_id.to_string()).await {
        let _ = ctx.events.send(Event::SwarmTaskUpdated {
            workspace_id: t.workspace_id.clone(),
            swarm_id: t.swarm_id.clone(),
            project_id: t.project_id.clone(),
            task: serde_json::to_value(&t).unwrap_or_default(),
        });
    }
}

/// Public re-export for the verification controller.
pub(crate) async fn emit_task_pub(ctx: &ServerCtx, task_id: &str) {
    emit_task(ctx, task_id).await;
}

/// True if the swarm is paused or over any budget — the verification controller
/// consults this between goals/fixes so it doesn't run past the budget gate.
pub(crate) async fn is_over_budget(ctx: &ServerCtx, swarm_id: &str) -> bool {
    match ctx.swarm_repo.get_swarm(&swarm_id.to_string()).await {
        Ok(s) => s.status == "paused" || budget_exceeded(ctx, &s).await.is_some(),
        Err(_) => true,
    }
}

pub(crate) fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n).collect::<String>() + "…"
    }
}

// --- Session teardown for pause/abort --------------------------------------

async fn swarm_session_ids(ctx: &ServerCtx, ws: &Id, swarm_id: &str) -> Vec<Id> {
    ctx.manager
        .list_by_workspace(ws)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|s| s.meta.get("swarm_id").and_then(|v| v.as_str()) == Some(swarm_id))
        .map(|s| s.id)
        .collect()
}

// --- HTTP: lifecycle + run/stop + recruit + plan ---------------------------

pub fn routes() -> Router<ServerCtx> {
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
    #[serde(default)]
    metric: Option<Option<String>>,
    #[serde(default)]
    comparator: Option<Option<String>>,
    #[serde(default)]
    target_value: Option<Option<f64>>,
    #[serde(default)]
    block_value: Option<Option<f64>>,
    #[serde(default)]
    verify_cmd: Option<Option<String>>,
    #[serde(default)]
    max_retries: Option<i64>,
    #[serde(default)]
    blocking: Option<bool>,
    #[serde(default)]
    order_idx: Option<i64>,
}

async fn emit_goal(ctx: &ServerCtx, goal: &SwarmGoal) {
    let _ = ctx.events.send(Event::SwarmGoalUpdated {
        workspace_id: goal.workspace_id.clone(),
        swarm_id: goal.swarm_id.clone(),
        task_id: goal.task_id.clone(),
        goal: serde_json::to_value(goal).unwrap_or_default(),
    });
}

async fn list_task_goals(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(tid): Path<Id>,
) -> ApiResult<Json<Vec<SwarmGoal>>> {
    let task = ctx.swarm_repo.get_task(&tid).await.map_err(ApiError)?;
    check(&ctx, &user, &task.workspace_id, WorkspaceRole::Viewer).await?;
    Ok(Json(
        ctx.swarm_repo
            .list_goals_for_task(&tid)
            .await
            .map_err(ApiError)?,
    ))
}

async fn list_project_goals(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(pid): Path<Id>,
) -> ApiResult<Json<Vec<SwarmGoal>>> {
    let project = ctx.swarm_repo.get_project(&pid).await.map_err(ApiError)?;
    check(&ctx, &user, &project.workspace_id, WorkspaceRole::Viewer).await?;
    Ok(Json(
        ctx.swarm_repo
            .list_goals_for_project(&pid)
            .await
            .map_err(ApiError)?,
    ))
}

async fn create_task_goal(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(tid): Path<Id>,
    Json(req): Json<CreateGoalReq>,
) -> ApiResult<Json<SwarmGoal>> {
    let task = ctx.swarm_repo.get_task(&tid).await.map_err(ApiError)?;
    check(&ctx, &user, &task.workspace_id, WorkspaceRole::Editor).await?;
    let goal = ctx
        .swarm_repo
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
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(pid): Path<Id>,
    Json(req): Json<CreateGoalReq>,
) -> ApiResult<Json<SwarmGoal>> {
    let project = ctx.swarm_repo.get_project(&pid).await.map_err(ApiError)?;
    check(&ctx, &user, &project.workspace_id, WorkspaceRole::Editor).await?;
    let goal = ctx
        .swarm_repo
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
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(gid): Path<Id>,
    Json(req): Json<UpdateGoalReq>,
) -> ApiResult<Json<SwarmGoal>> {
    let cur = ctx.swarm_repo.get_goal(&gid).await.map_err(ApiError)?;
    check(&ctx, &user, &cur.workspace_id, WorkspaceRole::Editor).await?;
    let goal = ctx
        .swarm_repo
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
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(gid): Path<Id>,
) -> ApiResult<Json<serde_json::Value>> {
    let cur = ctx.swarm_repo.get_goal(&gid).await.map_err(ApiError)?;
    check(&ctx, &user, &cur.workspace_id, WorkspaceRole::Editor).await?;
    ctx.swarm_repo.delete_goal(&gid).await.map_err(ApiError)?;
    Ok(Json(json!({})))
}

async fn list_standing_goals_h(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(sid): Path<Id>,
) -> ApiResult<Json<Vec<SwarmGoal>>> {
    let swarm = ctx.swarm_repo.get_swarm(&sid).await.map_err(ApiError)?;
    check(&ctx, &user, &swarm.workspace_id, WorkspaceRole::Viewer).await?;
    // Seed defaults on first read so the UI has something to edit.
    crate::swarm_verify::ensure_standing_goals(
        &ctx,
        &swarm.id,
        &swarm.workspace_id,
        &swarm.created_by,
    )
    .await;
    Ok(Json(
        ctx.swarm_repo
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
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(sid): Path<Id>,
    Json(req): Json<StandingGoalsReq>,
) -> ApiResult<Json<Vec<SwarmGoal>>> {
    let swarm = ctx.swarm_repo.get_swarm(&sid).await.map_err(ApiError)?;
    check(&ctx, &user, &swarm.workspace_id, WorkspaceRole::Editor).await?;
    for g in ctx
        .swarm_repo
        .list_standing_goals(&sid)
        .await
        .unwrap_or_default()
    {
        let _ = ctx.swarm_repo.delete_goal(&g.id).await;
    }
    for (i, r) in req.goals.into_iter().enumerate() {
        let mut ng = new_goal_from(r, &swarm.id, &swarm.workspace_id, None, None, &user.0.id);
        ng.kind = "standing".into();
        ng.order_idx = i as i64;
        let _ = ctx.swarm_repo.create_goal(ng).await;
    }
    Ok(Json(
        ctx.swarm_repo
            .list_standing_goals(&sid)
            .await
            .map_err(ApiError)?,
    ))
}

/// Manually kick the verification controller for a task (e.g. after a fix).
async fn verify_task_h(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(tid): Path<Id>,
) -> ApiResult<Json<serde_json::Value>> {
    let task = ctx.swarm_repo.get_task(&tid).await.map_err(ApiError)?;
    check(&ctx, &user, &task.workspace_id, WorkspaceRole::Editor).await?;
    if crate::swarm_verify::is_verifying(&tid) {
        return Ok(Json(
            json!({"started": false, "reason": "already verifying"}),
        ));
    }
    let dev = task
        .assignee_agent_id
        .clone()
        .ok_or_else(|| ApiError(Error::Invalid("task has no assignee to verify".into())))?;
    let _ = ctx
        .swarm_repo
        .update_task(
            &tid,
            TaskPatch {
                status: Some("verifying".into()),
                ..Default::default()
            },
        )
        .await;
    emit_task(&ctx, &tid).await;
    crate::swarm_verify::start_verification(&ctx, task, dev);
    Ok(Json(json!({"started": true})))
}

async fn stop_verify_h(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(tid): Path<Id>,
) -> ApiResult<Json<serde_json::Value>> {
    let task = ctx.swarm_repo.get_task(&tid).await.map_err(ApiError)?;
    check(&ctx, &user, &task.workspace_id, WorkspaceRole::Editor).await?;
    crate::swarm_verify::stop_task(&ctx, &tid).await;
    Ok(Json(json!({"stopped": true})))
}

async fn get_verification_h(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(tid): Path<Id>,
) -> ApiResult<Json<serde_json::Value>> {
    let task = ctx.swarm_repo.get_task(&tid).await.map_err(ApiError)?;
    check(&ctx, &user, &task.workspace_id, WorkspaceRole::Viewer).await?;
    let goals = ctx
        .swarm_repo
        .list_goals_for_task(&tid)
        .await
        .map_err(ApiError)?;
    Ok(Json(json!({
        "running": crate::swarm_verify::is_verifying(&tid),
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
    #[serde(default)]
    repo_path: Option<Option<String>>,
    #[serde(default)]
    auto_start: Option<bool>,
    #[serde(default)]
    reply: Option<bool>,
    #[serde(default)]
    enabled: Option<bool>,
}

async fn list_triggers_h(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(sid): Path<Id>,
) -> ApiResult<Json<Vec<SwarmChannelTrigger>>> {
    let swarm = ctx.swarm_repo.get_swarm(&sid).await.map_err(ApiError)?;
    check(&ctx, &user, &swarm.workspace_id, WorkspaceRole::Viewer).await?;
    Ok(Json(
        ctx.swarm_repo.list_triggers(&sid).await.map_err(ApiError)?,
    ))
}

async fn create_trigger_h(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(sid): Path<Id>,
    Json(req): Json<CreateTriggerReq>,
) -> ApiResult<Json<SwarmChannelTrigger>> {
    let swarm = ctx.swarm_repo.get_swarm(&sid).await.map_err(ApiError)?;
    check(&ctx, &user, &swarm.workspace_id, WorkspaceRole::Editor).await?;
    let t = ctx
        .swarm_repo
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
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(tid): Path<Id>,
    Json(req): Json<UpdateTriggerReq>,
) -> ApiResult<Json<SwarmChannelTrigger>> {
    let cur = ctx.swarm_repo.get_trigger(&tid).await.map_err(ApiError)?;
    check(&ctx, &user, &cur.workspace_id, WorkspaceRole::Editor).await?;
    let t = ctx
        .swarm_repo
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
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(tid): Path<Id>,
) -> ApiResult<Json<serde_json::Value>> {
    let cur = ctx.swarm_repo.get_trigger(&tid).await.map_err(ApiError)?;
    check(&ctx, &user, &cur.workspace_id, WorkspaceRole::Editor).await?;
    ctx.swarm_repo
        .delete_trigger(&tid)
        .await
        .map_err(ApiError)?;
    Ok(Json(json!({})))
}

/// Stop an in-flight plan/recruit for this swarm: kills the live agent
/// session(s) and prevents further retries.
async fn agent_stop(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path((ws, sid)): Path<(Id, Id)>,
) -> ApiResult<Json<serde_json::Value>> {
    check(&ctx, &user, &ws, WorkspaceRole::Editor).await?;
    crate::swarm_agent_run::stop(&ctx, &sid).await;
    Ok(Json(json!({ "ok": true })))
}

async fn check(ctx: &ServerCtx, user: &AuthUser, ws: &Id, role: WorkspaceRole) -> ApiResult<()> {
    ctx.roles.check(&user.0, ws, role).await.map_err(ApiError)
}

/// Resolve the default agent provider a swarm meta-agent (recruiter / planner /
/// summarizer) should run on: the workspace's `default_provider`, else the global
/// `default_provider` setting, else "claude". Keeps these coordinator-spawned
/// sessions on the user's configured default instead of a bare "claude" literal.
async fn swarm_meta_provider(ctx: &ServerCtx, ws: &otto_core::domain::Workspace) -> String {
    let global_default = otto_state::SettingsRepo::new(ctx.pool.clone())
        .get("default_provider")
        .await
        .ok()
        .flatten();
    otto_core::provider::resolve_provider(&[
        otto_core::provider::workspace_default(&ws.settings),
        otto_core::provider::global_default(global_default.as_ref()),
    ])
}

async fn start(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path((ws, sid)): Path<(Id, Id)>,
) -> ApiResult<Json<Swarm>> {
    check(&ctx, &user, &ws, WorkspaceRole::Editor).await?;

    // Point-of-action budget gate (A2): check workspace-level cap before the
    // Coordinator starts scheduling runs. Mirrors the review start_review gate.
    {
        let verdict = crate::routes::usage::check_budget(&ctx, &ws, "").await;
        if verdict.blocked {
            return Err(ApiError(Error::Invalid(format!(
                "Budget exceeded — swarm blocked: {}",
                verdict.reason.unwrap_or_else(|| "cap reached".to_string())
            ))));
        }
    }

    ctx.swarm_repo
        .set_swarm_status(&sid, "active")
        .await
        .map_err(ApiError)?;
    start_coordinator(ctx.clone(), sid.clone());
    emit_status(&ctx, &ws, &sid, "active");
    Ok(Json(
        ctx.swarm_repo.get_swarm(&sid).await.map_err(ApiError)?,
    ))
}

async fn pause(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path((ws, sid)): Path<(Id, Id)>,
) -> ApiResult<Json<Swarm>> {
    check(&ctx, &user, &ws, WorkspaceRole::Editor).await?;
    ctx.swarm_repo
        .set_swarm_status(&sid, "paused")
        .await
        .map_err(ApiError)?;
    set_paused(&ctx, &sid, true);
    // In-flight turns end first (their tasks re-queue for the resume) so the
    // retry loop can't respawn them; then suspend the sessions to free RAM
    // (resume-friendly).
    stop_runs_for_pause(&ctx, &sid).await;
    for s in swarm_session_ids(&ctx, &ws, &sid).await {
        let _ = ctx.manager.suspend(&s).await;
    }
    emit_status(&ctx, &ws, &sid, "paused");
    Ok(Json(
        ctx.swarm_repo.get_swarm(&sid).await.map_err(ApiError)?,
    ))
}

async fn abort(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path((ws, sid)): Path<(Id, Id)>,
) -> ApiResult<Json<Swarm>> {
    check(&ctx, &user, &ws, WorkspaceRole::Editor).await?;
    stop_coordinator(&ctx, &sid);
    // Stop any in-flight verification controllers (own cancel + kill verify/fix
    // sessions, short-circuiting run_swarm_agent retries; review B3).
    crate::swarm_verify::stop_swarm(&ctx, &sid).await;
    // Cancel in-flight runs and mark them stopped.
    let stopped = ctx
        .swarm_repo
        .stop_active_runs(&sid)
        .await
        .map_err(ApiError)?;
    for rid in &stopped {
        swarm_run::signal_cancel(&ctx.swarm_run_cancels, rid);
    }
    // Kill swarm sessions.
    for s in swarm_session_ids(&ctx, &ws, &sid).await {
        let _ = ctx.manager.kill_session(&s).await;
    }
    ctx.swarm_repo
        .set_swarm_status(&sid, "aborted")
        .await
        .map_err(ApiError)?;
    emit_status(&ctx, &ws, &sid, "aborted");
    Ok(Json(
        ctx.swarm_repo.get_swarm(&sid).await.map_err(ApiError)?,
    ))
}

async fn resume(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path((ws, sid)): Path<(Id, Id)>,
) -> ApiResult<Json<Swarm>> {
    check(&ctx, &user, &ws, WorkspaceRole::Editor).await?;

    // Point-of-action budget gate (A2): also checked on resume (a pause may have
    // been triggered by a BudgetExceeded event; block the resume when still over cap).
    {
        let verdict = crate::routes::usage::check_budget(&ctx, &ws, "").await;
        if verdict.blocked {
            return Err(ApiError(Error::Invalid(format!(
                "Budget exceeded — swarm resume blocked: {}",
                verdict.reason.unwrap_or_else(|| "cap reached".to_string())
            ))));
        }
    }

    ctx.swarm_repo
        .set_swarm_status(&sid, "active")
        .await
        .map_err(ApiError)?;
    set_paused(&ctx, &sid, false);
    start_coordinator(ctx.clone(), sid.clone());
    emit_status(&ctx, &ws, &sid, "active");
    Ok(Json(
        ctx.swarm_repo.get_swarm(&sid).await.map_err(ApiError)?,
    ))
}

pub(crate) fn emit_status(ctx: &ServerCtx, ws: &Id, sid: &str, status: &str) {
    let _ = ctx.events.send(Event::SwarmStatus {
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
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(sid): Path<Id>,
) -> ApiResult<Json<serde_json::Value>> {
    let swarm = ctx.swarm_repo.get_swarm(&sid).await.map_err(ApiError)?;
    check(&ctx, &user, &swarm.workspace_id, WorkspaceRole::Viewer).await?;
    let cap = swarm
        .config
        .get("max_parallel_sessions")
        .and_then(|v| v.as_i64())
        .unwrap_or(4)
        .max(1);
    let active = ctx
        .swarm_repo
        .active_run_count(&sid)
        .await
        .map_err(ApiError)?;
    let ready = ctx
        .swarm_repo
        .ready_tasks(&sid)
        .await
        .map_err(ApiError)?
        .len();
    let mut by_status: HashMap<String, i64> = HashMap::new();
    for t in ctx
        .swarm_repo
        .list_tasks_for_swarm(&sid)
        .await
        .map_err(ApiError)?
    {
        *by_status.entry(t.status).or_insert(0) += 1;
    }
    let mut agents_out = Vec::new();
    for a in ctx.swarm_repo.list_agents(&sid).await.map_err(ApiError)? {
        let busy = ctx
            .swarm_repo
            .agent_has_active_run(&a.id)
            .await
            .unwrap_or(false);
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
    })))
}

/// Clear a project's board: stop + cancel every in-flight run for the project
/// (so finishing turns can't repopulate it), delete all its tasks and the
/// project-scoped feed, and broadcast one `SwarmProjectCleared` so every open
/// client drops its local state. The project itself (and the run history —
/// spend accounting) stays.
async fn clear_project_h(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(pid): Path<Id>,
) -> ApiResult<Json<serde_json::Value>> {
    let project = ctx.swarm_repo.get_project(&pid).await.map_err(ApiError)?;
    check(&ctx, &user, &project.workspace_id, WorkspaceRole::Editor).await?;
    let stopped = ctx
        .swarm_repo
        .stop_active_runs_for_project(&pid)
        .await
        .map_err(ApiError)?;
    for rid in &stopped {
        swarm_run::signal_cancel(&ctx.swarm_run_cancels, rid);
        // Stop the agent itself — the flag alone left it working (and
        // burning tokens) in its worktree on a board that no longer exists.
        if let Some(sid) = ctx
            .swarm_repo
            .get_run(rid)
            .await
            .ok()
            .and_then(|r| r.session_id)
        {
            let _ = ctx.manager.kill_session(&sid).await;
        }
        swarm_run::emit_run(&ctx, rid).await;
    }
    let (tasks_deleted, messages_deleted) = ctx
        .swarm_repo
        .clear_project_board(&pid)
        .await
        .map_err(ApiError)?;
    let _ = ctx.events.send(Event::SwarmProjectCleared {
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
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(tid): Path<Id>,
) -> ApiResult<Json<otto_state::SwarmRun>> {
    let task = ctx.swarm_repo.get_task(&tid).await.map_err(ApiError)?;
    check(&ctx, &user, &task.workspace_id, WorkspaceRole::Editor).await?;
    let swarm = ctx
        .swarm_repo
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
        .swarm_repo
        .agent_has_active_run(&agent.id)
        .await
        .unwrap_or(false)
        || crate::swarm_verify::agent_under_verification(&agent.id)
    {
        return Err(ApiError(Error::Conflict(format!(
            "{} is busy with another turn — try again when it finishes",
            agent.name
        ))));
    }
    let _ = ctx.swarm_repo.bump_task_attempt(&tid).await;
    let is_leader = has_reports(&ctx, &swarm.id, &agent.id).await;
    let kind = if is_leader && !task.delegated {
        "planning"
    } else {
        "task"
    };
    let run = ctx
        .swarm_repo
        .create_run(NewRun {
            swarm_id: swarm.id.clone(),
            workspace_id: swarm.workspace_id.clone(),
            project_id: Some(task.project_id.clone()),
            task_id: Some(task.id.clone()),
            agent_id: agent.id.clone(),
            kind: kind.to_string(),
            trigger: "manual".to_string(),
        })
        .await
        .map_err(ApiError)?;
    let _ = ctx
        .swarm_repo
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
        let result = swarm_run::run_turn(ctx2.clone(), run2.clone()).await;
        route_result(&ctx2, &run2, &task2, result).await;
    });
    Ok(Json(run))
}

async fn stop_run(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(rid): Path<Id>,
) -> ApiResult<Json<otto_state::SwarmRun>> {
    let run = ctx.swarm_repo.get_run(&rid).await.map_err(ApiError)?;
    check(&ctx, &user, &run.workspace_id, WorkspaceRole::Editor).await?;
    swarm_run::signal_cancel(&ctx.swarm_run_cancels, &rid);
    let stopped = ctx
        .swarm_repo
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
        let _ = ctx.manager.kill_session(&sid).await;
    }
    swarm_run::emit_run(&ctx, &rid).await;
    Ok(Json(ctx.swarm_repo.get_run(&rid).await.map_err(ApiError)?))
}

async fn recruit(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(ws): Path<Id>,
    Json(req): Json<otto_swarm::RecruitReq>,
) -> ApiResult<Json<otto_swarm::RecruitedAgent>> {
    check(&ctx, &user, &ws, WorkspaceRole::Editor).await?;
    let (swarm_name, mission, titles) = match &req.swarm_id {
        Some(sid) => {
            let s = ctx.swarm_repo.get_swarm(sid).await.map_err(ApiError)?;
            let titles = ctx
                .swarm_repo
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
        .context_library
        .list_skills()
        .into_iter()
        .map(|s| s.name)
        .collect();
    let capped_skills = otto_swarm::recruiter::cap_skills_for_role(
        &all_skills,
        &req.role,
        otto_swarm::recruiter::RECRUITER_SKILL_CAP,
    );
    tracing::debug!(
        "recruiter: injecting {} / {} skills into prompt (cap={})",
        capped_skills.len(),
        all_skills.len(),
        otto_swarm::recruiter::RECRUITER_SKILL_CAP
    );
    let providers = {
        use otto_swarm::SwarmCtx;
        ctx.available_providers()
    };
    // The provider the recruiter meta-agent itself runs on — the configured
    // default (workspace → global → "claude"), not a bare literal.
    let workspace = ctx.workspaces.get(&ws).await.map_err(ApiError)?;
    let meta_provider = swarm_meta_provider(&ctx, &workspace).await;
    let prompt = otto_swarm::recruiter::recruiter_prompt(
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
                .swarm_repo
                .list_agents(sid)
                .await
                .unwrap_or_default()
                .first()
                .map(|a| a.id.clone())
                .unwrap_or_else(|| "recruiter".to_string());
            let cancel = crate::swarm_agent_run::begin(sid);
            let (raw, rid) = crate::swarm_agent_run::run_swarm_agent(
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
                |t| otto_swarm::recruiter::parse_recruited(t).is_some(),
                &cancel,
            )
            .await;
            crate::swarm_agent_run::end(sid);
            let raw = raw.ok_or_else(|| {
                ApiError(Error::Upstream(
                    "recruiter produced nothing (stopped or stuck)".into(),
                ))
            })?;
            (raw, Some(rid))
        }
        None => (
            ctx.orchestrator
                .run_agent(&prompt, &cwd, None, AGENT_NO_PROGRESS)
                .await
                .map_err(ApiError)?,
            None,
        ),
    };
    let mut recruited = otto_swarm::recruiter::parse_recruited(&reply).ok_or_else(|| {
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
            .swarm_repo
            .update_run(
                &rid,
                RunPatch {
                    result: Some(Some(serde_json::to_value(&recruited).unwrap_or_default())),
                    ..Default::default()
                },
            )
            .await;
        crate::swarm_run::emit_run(&ctx, &rid).await;
    }
    Ok(Json(recruited))
}

async fn plan(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path((ws, pid)): Path<(Id, Id)>,
    Json(_req): Json<otto_swarm::PlanReq>,
) -> ApiResult<Json<Vec<SwarmTask>>> {
    check(&ctx, &user, &ws, WorkspaceRole::Editor).await?;
    let project = ctx.swarm_repo.get_project(&pid).await.map_err(ApiError)?;
    let goal = project.goal_md.clone().unwrap_or_default();
    if goal.trim().is_empty() {
        return Err(ApiError(Error::Invalid(
            "project has no goal to plan".into(),
        )));
    }
    let agents = ctx
        .swarm_repo
        .list_agents(&project.swarm_id)
        .await
        .unwrap_or_default();
    let preset_agents: Vec<otto_swarm::PresetAgent> = agents
        .iter()
        .map(|a| otto_swarm::PresetAgent {
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
    let ws_obj = ctx.workspaces.get(&ws).await.map_err(ApiError)?;
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
    let cancel = crate::swarm_agent_run::begin(&project.swarm_id);
    let mut candidates: Vec<String> = Vec::new();
    let angles = otto_swarm::recruiter::PLANNER_ANGLES;
    for (i, angle) in angles.iter().enumerate() {
        let prompt =
            otto_swarm::recruiter::planner_prompt(&project.name, &goal, &preset_agents, angle);
        let title = format!("Plan {}/{}: {}", i + 1, angles.len(), project.name);
        let (raw, _) = crate::swarm_agent_run::run_swarm_agent(
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
            |t| otto_swarm::recruiter::extract_json(t).is_some(),
            &cancel,
        )
        .await;
        if let Some(raw) = raw {
            if otto_swarm::recruiter::extract_json(&raw).is_some() {
                candidates.push(raw);
            }
        }
    }
    let final_json = if candidates.len() > 1 {
        let sum_prompt = otto_swarm::recruiter::planner_summarizer_prompt(
            &project.name,
            &goal,
            &preset_agents,
            &candidates,
        );
        let (raw, _) = crate::swarm_agent_run::run_swarm_agent(
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
            |t| otto_swarm::recruiter::extract_json(t).is_some(),
            &cancel,
        )
        .await;
        raw.and_then(|r| otto_swarm::recruiter::extract_json(&r))
            .or_else(|| otto_swarm::recruiter::extract_json(&candidates[0]))
    } else {
        candidates
            .first()
            .and_then(|c| otto_swarm::recruiter::extract_json(c))
    };
    crate::swarm_agent_run::end(&project.swarm_id);
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
            .swarm_repo
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
                    .swarm_repo
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
    let result = ctx.swarm_repo.list_tasks(&pid).await.map_err(ApiError)?;
    Ok(Json(result))
}
