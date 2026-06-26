//! The SwarmCoordinator runtime: a per-swarm supervisor that schedules ready
//! tasks onto agents within the parallel-worker cap, runs each turn via
//! `swarm_run::run_turn`, and routes the result (delegation → subtasks, handoffs,
//! reviews, concerns, completion). Plus the lifecycle (start/pause/abort/resume),
//! manual run/stop, and the recruiter/planner endpoints.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
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

async fn coordinator_loop(ctx: ServerCtx, swarm_id: Id, handle: CoordinatorHandle) {
    loop {
        if handle.cancel.load(Ordering::Relaxed) {
            return;
        }
        if !handle.paused.load(Ordering::Relaxed) {
            if let Err(e) = tick(&ctx, &swarm_id).await {
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
        // Count this scheduled turn against the task's attempt ceiling. The
        // ceiling itself is enforced in `route_result` once the turn returns a
        // non-terminal status, so the work still happens this tick.
        let _ = repo.bump_task_attempt(&task.id).await;
        // Claim: move the task to in_progress so it isn't re-selected next tick.
        let _ = repo
            .update_task(&task.id, TaskPatch { status: Some("in_progress".into()), ..Default::default() })
            .await;
        emit_task(ctx, &task.id).await;

        let is_leader = has_reports(ctx, &swarm.id, &agent.id).await;
        let kind = if is_leader && !task.delegated { "planning" } else { "task" };
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
                    .update_task(&task.id, TaskPatch { status: Some("todo".into()), ..Default::default() })
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
    if let (None, None, None) = (swarm.max_total_runs, swarm.max_cost_usd, swarm.max_runtime_secs) {
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
    let active: Vec<SwarmAgent> = agents.into_iter().filter(|a| a.status == "active").collect();
    if active.is_empty() {
        return None;
    }
    let hay = format!("{} {}", task.title, task.description).to_lowercase();
    let score = |a: &SwarmAgent| -> i32 {
        let mut s = 0;
        for tok in format!("{} {}", a.title, a.specialization).to_lowercase().split_whitespace() {
            if tok.len() >= 4 && hay.contains(tok) {
                s += 1;
            }
        }
        s
    };
    active.iter().cloned().max_by_key(|a| score(a)).or_else(|| active.into_iter().next())
}

async fn has_reports(ctx: &ServerCtx, swarm_id: &str, agent_id: &str) -> bool {
    ctx.swarm_repo
        .list_agents(&swarm_id.to_string())
        .await
        .map(|all| all.iter().any(|a| a.reports_to.as_deref() == Some(agent_id)))
        .unwrap_or(false)
}

async fn resolve_agent_by_title(ctx: &ServerCtx, swarm_id: &str, title: &str) -> Option<Id> {
    let want = title.trim().to_lowercase();
    let agents = ctx.swarm_repo.list_agents(&swarm_id.to_string()).await.ok()?;
    agents
        .iter()
        .find(|a| a.title.to_lowercase() == want)
        .or_else(|| agents.iter().find(|a| a.title.to_lowercase().contains(&want) || want.contains(&a.title.to_lowercase())))
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
    let Some(res) = result else {
        // Turn failed/stopped. Retry on the next tick up to the attempt ceiling
        // (D8); once exhausted, block the task so it isn't retried forever.
        if attempt_ceiling_reached(ctx, task).await {
            block_for_attempts(ctx, task).await;
        } else {
            let _ = repo
                .update_task(&task.id, TaskPatch { status: Some("todo".into()), ..Default::default() })
                .await;
            emit_task(ctx, &task.id).await;
            system_post(ctx, &task.swarm_id, Some(&task.project_id), Some(&task.id), "status",
                &format!("Run for “{}” did not complete — will retry.", task.title)).await;
        }
        return;
    };

    // Concerns → board + notification (CTO/PM "wrong path" escalation).
    for c in &res.concerns {
        if c.text.trim().is_empty() {
            continue;
        }
        system_post(ctx, &task.swarm_id, Some(&task.project_id), Some(&task.id), "concern",
            &format!("[{}] {}", c.severity, c.text)).await;
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
            let _ = repo.update_task(&task.id, TaskPatch {
                status: Some("todo".into()), delegated: Some(true), ..Default::default()
            }).await;
            emit_task(ctx, &task.id).await;
            return;
        }
        let _ = repo.update_task(&task.id, TaskPatch {
            status: Some("in_progress".into()), delegated: Some(true), ..Default::default()
        }).await;
        create_subtasks(ctx, task, &res.subtasks).await;
        emit_task(ctx, &task.id).await;
        return;
    }

    // Subtasks from a normal task (rare but allowed).
    if !res.subtasks.is_empty() {
        create_subtasks(ctx, task, &res.subtasks).await;
    }

    // Handoffs → a follow-up task for the named role, dependent on this one.
    for h in &res.handoffs {
        if h.to_role.trim().is_empty() {
            continue;
        }
        let assignee = resolve_agent_by_title(ctx, &task.swarm_id, &h.to_role).await;
        let _ = repo.create_task(NewTask {
            project_id: task.project_id.clone(),
            swarm_id: task.swarm_id.clone(),
            workspace_id: task.workspace_id.clone(),
            title: format!("Handoff: {}", clip(&h.brief, 60)),
            description: h.brief.clone(),
            assignee_agent_id: assignee,
            status: "todo".into(),
            priority: "medium".into(),
            parent_task_id: None,
            depends_on: json!([]),
            labels: json!(["handoff"]),
            order_idx: 0,
            created_by: run.agent_id.clone(),
        }).await;
    }

    // Apply the reported status to the task.
    let artifact_ref = res.artifacts.iter().find_map(|a| a.path.clone().or_else(|| a.url.clone()));
    match res.status.as_str() {
        "done" => {
            // If a review was requested, go to in_review and enqueue a review run
            // (human-review flow takes precedence over goal verification).
            if !res.reviews.is_empty() {
                let _ = repo.update_task(&task.id, TaskPatch {
                    status: Some("in_review".into()), result_ref: Some(artifact_ref), ..Default::default()
                }).await;
                enqueue_reviews(ctx, task, run, &res).await;
            } else if crate::swarm_verify::task_has_goals(ctx, task).await {
                // Goals attached → the leader verifies each sequentially before the
                // task is done + its worktree branch is merged (requirement 3).
                // Persist the dev as the assignee so restart-recovery + the
                // coordinator's per-agent lock can find it.
                let _ = repo.update_task(&task.id, TaskPatch {
                    status: Some("verifying".into()),
                    result_ref: Some(artifact_ref),
                    assignee_agent_id: Some(Some(run.agent_id.clone())),
                    ..Default::default()
                }).await;
                emit_task(ctx, &task.id).await;
                crate::swarm_verify::start_verification(ctx, task.clone(), run.agent_id.clone());
                return; // controller drives the task to done/blocked + posts summary
            } else {
                let _ = repo.update_task(&task.id, TaskPatch {
                    status: Some("done".into()), result_ref: Some(artifact_ref), ..Default::default()
                }).await;
                complete_parent_if_done(ctx, task).await;
            }
        }
        "needs_review" => {
            let _ = repo.update_task(&task.id, TaskPatch {
                status: Some("in_review".into()), result_ref: Some(artifact_ref), ..Default::default()
            }).await;
            enqueue_reviews(ctx, task, run, &res).await;
        }
        "blocked" => {
            let _ = repo.update_task(&task.id, TaskPatch { status: Some("blocked".into()), ..Default::default() }).await;
        }
        _ => {
            // in_progress / unknown → allow another turn next tick, UNLESS the
            // task has hit its attempt ceiling (D8): otherwise a task that never
            // self-reports a terminal status would re-run forever, burning the
            // budget. Block it with a reason + notification instead.
            if attempt_ceiling_reached(ctx, task).await {
                block_for_attempts(ctx, task).await;
            } else {
                let _ = repo.update_task(&task.id, TaskPatch { status: Some("todo".into()), ..Default::default() }).await;
            }
        }
    }
    emit_task(ctx, &task.id).await;
    if !res.summary.is_empty() {
        system_post(ctx, &task.swarm_id, Some(&task.project_id), Some(&task.id), "status",
            &format!("{} — {}", task.title, clip(&res.summary, 240))).await;
    }
}

/// Has a task exhausted its swarm's per-task attempt ceiling? Re-reads the task
/// for the up-to-date attempt counter (the Coordinator bumps it when it queues
/// each turn) and compares against the swarm's `max_attempts` (default 3, min 1).
async fn attempt_ceiling_reached(ctx: &ServerCtx, task: &SwarmTask) -> bool {
    let repo = &ctx.swarm_repo;
    let attempts = repo.get_task(&task.id).await.map(|t| t.attempts).unwrap_or(task.attempts);
    let ceiling = repo.get_swarm(&task.swarm_id).await.map(|s| s.max_attempts.max(1)).unwrap_or(3);
    attempts >= ceiling
}

/// Mark a task `blocked` because it hit the attempt ceiling, post to the board,
/// and notify. Used both for hard failures and tasks that never self-report a
/// terminal status.
async fn block_for_attempts(ctx: &ServerCtx, task: &SwarmTask) {
    let repo = &ctx.swarm_repo;
    let attempts = repo.get_task(&task.id).await.map(|t| t.attempts).unwrap_or(task.attempts);
    let _ = repo
        .update_task(&task.id, TaskPatch { status: Some("blocked".into()), ..Default::default() })
        .await;
    emit_task(ctx, &task.id).await;
    let body = format!(
        "Task “{}” blocked after {attempts} attempt(s) without completing — needs a human.",
        task.title
    );
    system_post(ctx, &task.swarm_id, Some(&task.project_id), Some(&task.id), "escalation", &body).await;
    let _ = ctx.events.send(Event::Notice {
        level: "warn".into(),
        title: "Swarm task blocked (attempts)".into(),
        body: clip(&body, 160),
    });
}

async fn create_subtasks(ctx: &ServerCtx, parent: &SwarmTask, subs: &[swarm_run::TurnSubtask]) {
    let repo = &ctx.swarm_repo;
    for (i, st) in subs.iter().enumerate() {
        if st.title.trim().is_empty() {
            continue;
        }
        let assignee = match &st.assignee_role {
            Some(role) if !role.is_empty() => resolve_agent_by_title(ctx, &parent.swarm_id, role).await,
            _ => None,
        };
        let priority = st.priority.clone().filter(|p| !p.is_empty()).unwrap_or_else(|| "medium".into());
        let _ = repo.create_task(NewTask {
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
            labels: json!([]),
            order_idx: i as i64,
            created_by: parent.created_by.clone(),
        }).await;
    }
}

async fn enqueue_reviews(ctx: &ServerCtx, task: &SwarmTask, run: &otto_state::SwarmRun, res: &SwarmTurnResult) {
    let repo = &ctx.swarm_repo;
    for rv in &res.reviews {
        let reviewer = resolve_agent_by_title(ctx, &task.swarm_id, &rv.reviewer_role).await;
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
    }
    system_post(ctx, &task.swarm_id, Some(&task.project_id), Some(&task.id), "review_request",
        &format!("Review requested on “{}”.", task.title)).await;
}

/// When a task completes, if it has a parent and all the parent's children are
/// done, complete the parent too (recursively).
async fn complete_parent_if_done(ctx: &ServerCtx, task: &SwarmTask) {
    let repo = &ctx.swarm_repo;
    let Some(parent_id) = &task.parent_task_id else { return };
    if repo.children_complete(parent_id).await.unwrap_or(false) {
        if let Ok(parent) = repo.get_task(parent_id).await {
            if parent.status != "done" {
                let _ = repo.update_task(parent_id, TaskPatch { status: Some("done".into()), ..Default::default() }).await;
                emit_task(ctx, parent_id).await;
                Box::pin(complete_parent_if_done(ctx, &parent)).await;
            }
        }
    }
}

async fn system_post(ctx: &ServerCtx, swarm_id: &str, project_id: Option<&str>, task_id: Option<&str>, kind: &str, body: &str) {
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
    if let Ok(msg) = ctx.swarm_repo.create_message(otto_state::NewMessage {
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
    }).await {
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
        .route("/workspaces/{id}/swarm/swarms/{sid}/agent-stop", post(agent_stop))
        // Goals (requirement 3)
        .route("/swarm/tasks/{tid}/goals", get(list_task_goals).post(create_task_goal))
        .route("/swarm/projects/{pid}/goals", get(list_project_goals).post(create_project_goal))
        .route("/swarm/goals/{gid}", patch(update_goal_h).delete(delete_goal_h))
        .route(
            "/swarm/swarms/{sid}/standing-goals",
            get(list_standing_goals_h).put(put_standing_goals_h),
        )
        // Verification controller
        .route("/swarm/tasks/{tid}/verify", post(verify_task_h))
        .route("/swarm/tasks/{tid}/verify/stop", post(stop_verify_h))
        .route("/swarm/tasks/{tid}/verification", get(get_verification_h))
        // Channel triggers (requirement 4)
        .route("/swarm/swarms/{sid}/triggers", get(list_triggers_h).post(create_trigger_h))
        .route("/swarm/triggers/{tid}", patch(update_trigger_h).delete(delete_trigger_h))
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
    Ok(Json(ctx.swarm_repo.list_goals_for_task(&tid).await.map_err(ApiError)?))
}

async fn list_project_goals(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(pid): Path<Id>,
) -> ApiResult<Json<Vec<SwarmGoal>>> {
    let project = ctx.swarm_repo.get_project(&pid).await.map_err(ApiError)?;
    check(&ctx, &user, &project.workspace_id, WorkspaceRole::Viewer).await?;
    Ok(Json(ctx.swarm_repo.list_goals_for_project(&pid).await.map_err(ApiError)?))
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
        .create_goal(new_goal_from(req, &task.swarm_id, &task.workspace_id, Some(task.project_id.clone()), Some(tid), &user.0.id))
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
        .create_goal(new_goal_from(req, &project.swarm_id, &project.workspace_id, Some(pid), None, &user.0.id))
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
    crate::swarm_verify::ensure_standing_goals(&ctx, &swarm.id, &swarm.workspace_id, &swarm.created_by).await;
    Ok(Json(ctx.swarm_repo.list_standing_goals(&sid).await.map_err(ApiError)?))
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
    for g in ctx.swarm_repo.list_standing_goals(&sid).await.unwrap_or_default() {
        let _ = ctx.swarm_repo.delete_goal(&g.id).await;
    }
    for (i, r) in req.goals.into_iter().enumerate() {
        let mut ng = new_goal_from(r, &swarm.id, &swarm.workspace_id, None, None, &user.0.id);
        ng.kind = "standing".into();
        ng.order_idx = i as i64;
        let _ = ctx.swarm_repo.create_goal(ng).await;
    }
    Ok(Json(ctx.swarm_repo.list_standing_goals(&sid).await.map_err(ApiError)?))
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
        return Ok(Json(json!({"started": false, "reason": "already verifying"})));
    }
    let dev = task
        .assignee_agent_id
        .clone()
        .ok_or_else(|| ApiError(Error::Invalid("task has no assignee to verify".into())))?;
    let _ = ctx
        .swarm_repo
        .update_task(&tid, TaskPatch { status: Some("verifying".into()), ..Default::default() })
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
    let goals = ctx.swarm_repo.list_goals_for_task(&tid).await.map_err(ApiError)?;
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
    Ok(Json(ctx.swarm_repo.list_triggers(&sid).await.map_err(ApiError)?))
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
    ctx.swarm_repo.delete_trigger(&tid).await.map_err(ApiError)?;
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

    ctx.swarm_repo.set_swarm_status(&sid, "active").await.map_err(ApiError)?;
    start_coordinator(ctx.clone(), sid.clone());
    emit_status(&ctx, &ws, &sid, "active");
    Ok(Json(ctx.swarm_repo.get_swarm(&sid).await.map_err(ApiError)?))
}

async fn pause(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path((ws, sid)): Path<(Id, Id)>,
) -> ApiResult<Json<Swarm>> {
    check(&ctx, &user, &ws, WorkspaceRole::Editor).await?;
    ctx.swarm_repo.set_swarm_status(&sid, "paused").await.map_err(ApiError)?;
    set_paused(&ctx, &sid, true);
    // Suspend idle swarm sessions to free RAM (resume-friendly).
    for s in swarm_session_ids(&ctx, &ws, &sid).await {
        let _ = ctx.manager.suspend(&s).await;
    }
    emit_status(&ctx, &ws, &sid, "paused");
    Ok(Json(ctx.swarm_repo.get_swarm(&sid).await.map_err(ApiError)?))
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
    let stopped = ctx.swarm_repo.stop_active_runs(&sid).await.map_err(ApiError)?;
    for rid in &stopped {
        swarm_run::signal_cancel(&ctx.swarm_run_cancels, rid);
    }
    // Kill swarm sessions.
    for s in swarm_session_ids(&ctx, &ws, &sid).await {
        let _ = ctx.manager.kill_session(&s).await;
    }
    ctx.swarm_repo.set_swarm_status(&sid, "aborted").await.map_err(ApiError)?;
    emit_status(&ctx, &ws, &sid, "aborted");
    Ok(Json(ctx.swarm_repo.get_swarm(&sid).await.map_err(ApiError)?))
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

    ctx.swarm_repo.set_swarm_status(&sid, "active").await.map_err(ApiError)?;
    set_paused(&ctx, &sid, false);
    start_coordinator(ctx.clone(), sid.clone());
    emit_status(&ctx, &ws, &sid, "active");
    Ok(Json(ctx.swarm_repo.get_swarm(&sid).await.map_err(ApiError)?))
}

pub(crate) fn emit_status(ctx: &ServerCtx, ws: &Id, sid: &str, status: &str) {
    let _ = ctx.events.send(Event::SwarmStatus {
        workspace_id: ws.clone(),
        swarm_id: sid.to_string(),
        status: status.to_string(),
    });
}

async fn run_task(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(tid): Path<Id>,
) -> ApiResult<Json<otto_state::SwarmRun>> {
    let task = ctx.swarm_repo.get_task(&tid).await.map_err(ApiError)?;
    check(&ctx, &user, &task.workspace_id, WorkspaceRole::Editor).await?;
    let swarm = ctx.swarm_repo.get_swarm(&task.swarm_id).await.map_err(ApiError)?;
    let agent = pick_agent(&ctx, &swarm, &task)
        .await
        .ok_or_else(|| ApiError(Error::Invalid("no active agent to run this task".into())))?;
    let is_leader = has_reports(&ctx, &swarm.id, &agent.id).await;
    let kind = if is_leader && !task.delegated { "planning" } else { "task" };
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
        .update_task(&tid, TaskPatch { status: Some("in_progress".into()), ..Default::default() })
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
    if matches!(run.status.as_str(), "queued" | "running" | "waiting") {
        let _ = ctx
            .swarm_repo
            .update_run(&rid, RunPatch { status: Some("stopped".into()), finished_at: Some(Some(Utc::now())), ..Default::default() })
            .await;
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
            let titles = ctx.swarm_repo.list_agents(sid).await.unwrap_or_default()
                .into_iter().map(|a| a.title).collect::<Vec<_>>();
            (s.name, s.description, titles)
        }
        None => ("New swarm".to_string(), String::new(), Vec::new()),
    };
    // Collect ALL known skill names so we can validate the recruiter's reply,
    // but only inject a bounded subset into the prompt.  Injecting the full
    // library (potentially hundreds of skills) wastes tokens and can produce
    // bloated, irrelevant skill lists.  `cap_skills_for_role` ranks by name-
    // relevance to the requested role and hard-caps at `RECRUITER_SKILL_CAP`.
    let all_skills: Vec<String> = ctx.context_library.list_skills().into_iter().map(|s| s.name).collect();
    let capped_skills = otto_swarm::recruiter::cap_skills_for_role(
        &all_skills,
        &req.role,
        otto_swarm::recruiter::RECRUITER_SKILL_CAP,
    );
    tracing::debug!(
        "recruiter: injecting {} / {} skills into prompt (cap={})",
        capped_skills.len(), all_skills.len(), otto_swarm::recruiter::RECRUITER_SKILL_CAP
    );
    let providers = {
        use otto_swarm::SwarmCtx;
        ctx.available_providers()
    };
    let prompt = otto_swarm::recruiter::recruiter_prompt(
        &req.role, &swarm_name, &mission, &titles, &capped_skills, &providers,
        req.context.as_deref(), req.naming_theme.as_deref(),
    );
    let cwd = std::env::temp_dir().to_string_lossy().to_string();
    // When recruiting into an existing swarm, run as a REAL, openable session
    // (watchable live + Stop-able). With no swarm yet (brand-new), fall back to a
    // headless one-shot turn (nothing to attach a run/session to).
    let (reply, run_id): (String, Option<Id>) = match &req.swarm_id {
        Some(sid) => {
            let ws_obj = ctx.workspaces.get(&ws).await.map_err(ApiError)?;
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
                &ctx, &ws_obj, &user.0, sid, None, None, &nominal, "claude", "recruit",
                &format!("Recruit: {}", req.role), &cwd, &prompt,
                |t| otto_swarm::recruiter::parse_recruited(t).is_some(),
                &cancel,
            )
            .await;
            crate::swarm_agent_run::end(sid);
            let raw = raw.ok_or_else(|| {
                ApiError(Error::Upstream("recruiter produced nothing (stopped or stuck)".into()))
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
    let mut recruited = otto_swarm::recruiter::parse_recruited(&reply)
        .ok_or_else(|| ApiError(Error::Upstream("recruiter returned no usable definition".into())))?;
    // Validate skills against the FULL library (not just the capped list); any
    // skill the recruiter invents that is not in the real library is dropped.
    let known: std::collections::HashSet<String> = all_skills.into_iter().collect();
    recruited.skills.retain(|s| known.contains(&s.name));
    // Force the provider to an available one.
    if !providers.iter().any(|p| p == &recruited.suggested_provider) {
        recruited.suggested_provider = providers.first().cloned().unwrap_or_else(|| "claude".into());
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
        return Err(ApiError(Error::Invalid("project has no goal to plan".into())));
    }
    let agents = ctx.swarm_repo.list_agents(&project.swarm_id).await.unwrap_or_default();
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
    let cwd = project
        .repo_path
        .clone()
        .unwrap_or_else(|| std::env::temp_dir().to_string_lossy().to_string());
    let ws_obj = ctx.workspaces.get(&ws).await.map_err(ApiError)?;
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
            &ctx, &ws_obj, &user.0, &project.swarm_id, Some(&project.id), None, &nominal_agent,
            "claude", "plan", &title, &cwd, &prompt,
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
            &project.name, &goal, &preset_agents, &candidates,
        );
        let (raw, _) = crate::swarm_agent_run::run_swarm_agent(
            &ctx, &ws_obj, &user.0, &project.swarm_id, Some(&project.id), None, &nominal_agent,
            "claude", "plan", &format!("Plan summary: {}", project.name), &cwd, &sum_prompt,
            |t| otto_swarm::recruiter::extract_json(t).is_some(),
            &cancel,
        )
        .await;
        raw.and_then(|r| otto_swarm::recruiter::extract_json(&r))
            .or_else(|| otto_swarm::recruiter::extract_json(&candidates[0]))
    } else {
        candidates.first().and_then(|c| otto_swarm::recruiter::extract_json(c))
    };
    crate::swarm_agent_run::end(&project.swarm_id);
    let v = final_json
        .ok_or_else(|| ApiError(Error::Upstream("planner produced no tasks (stopped or stuck)".into())))?;
    let tasks_json = v.get("tasks").and_then(|t| t.as_array()).cloned().unwrap_or_default();

    // Two passes: create tasks, then wire depends_on by matching titles.
    let mut created: Vec<SwarmTask> = Vec::new();
    let mut by_title: HashMap<String, Id> = HashMap::new();
    for (i, t) in tasks_json.iter().enumerate() {
        let title = t.get("title").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
        if title.is_empty() {
            continue;
        }
        let description = t.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let priority = t.get("priority").and_then(|v| v.as_str()).unwrap_or("medium").to_string();
        let assignee = t.get("assignee_title").and_then(|v| v.as_str())
            .and_then(|title| agents.iter().find(|a| a.title.eq_ignore_ascii_case(title.trim())).map(|a| a.id.clone()));
        if let Ok(task) = ctx.swarm_repo.create_task(NewTask {
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
        }).await {
            by_title.insert(title.to_lowercase(), task.id.clone());
            created.push(task);
        }
    }
    // Wire dependencies.
    for (t, created_task) in tasks_json.iter().zip(created.iter()) {
        if let Some(deps) = t.get("depends_on_titles").and_then(|v| v.as_array()) {
            let dep_ids: Vec<String> = deps.iter()
                .filter_map(|d| d.as_str())
                .filter_map(|d| by_title.get(&d.to_lowercase()).cloned())
                .collect();
            if !dep_ids.is_empty() {
                let _ = ctx.swarm_repo.update_task(&created_task.id, TaskPatch {
                    depends_on: Some(json!(dep_ids)), ..Default::default()
                }).await;
            }
        }
    }
    let result = ctx.swarm_repo.list_tasks(&pid).await.map_err(ApiError)?;
    Ok(Json(result))
}
