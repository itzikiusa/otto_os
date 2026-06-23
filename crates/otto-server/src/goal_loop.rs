//! The Goal Loop engine: a per-loop controller that runs bounded
//! Plan → Execute → Evaluate → Digest iterations toward a goal with
//! machine-checked acceptance criteria, on an isolated git branch, until the
//! goal is met or a hard limit (iterations / active time) is hit.
//!
//! Reuse is plumbing-only: executors run as live, openable [`SessionManager`]
//! sessions (review-style spawn/inject/watch via [`crate::agent_run`] +
//! [`crate::review_session`]); the planner/evaluator/digester/definer are
//! headless [`Orchestrator::run_agent`] turns, each timeout-wrapped. The
//! concurrency/safety model is OURS: v1 runs executors SEQUENTIALLY on one
//! worktree (no git-index races), the evaluator ground-truths command criteria,
//! and the controller is the sole writer of the loop's runtime fields.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::Utc;
use otto_core::domain::{
    GoalLoop, GoalLoopAgentCfg, GoalLoopEvaluation, GoalLoopPhase, GoalLoopRoleCfg, GoalLoopStatus,
    LoopAgentState, SessionKind,
};
use otto_core::api::CreateSessionReq;
use otto_core::event::Event;
use otto_core::{Error, Id, Result};

use crate::agent_run::{run_with_recovery, watch_for_result, FailReason, RunOutcome, WatchStatus};
use crate::goal_loop_parse::{parse_evaluation, parse_executor_result};
use crate::review_session::{bracketed_paste, dispatched, wait_for_tui, PASTE_TO_ENTER};
use crate::state::ServerCtx;

// --- Registry --------------------------------------------------------------

/// A running loop controller's control handles.
#[derive(Clone)]
pub struct LoopHandle {
    pub cancel: Arc<AtomicBool>,
    pub paused: Arc<AtomicBool>,
}

impl LoopHandle {
    pub fn new() -> Self {
        Self {
            cancel: Arc::new(AtomicBool::new(false)),
            paused: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl Default for LoopHandle {
    fn default() -> Self {
        Self::new()
    }
}

/// loop_id → live controller handle.
pub type GoalLoopRegistry = Arc<Mutex<HashMap<String, LoopHandle>>>;

pub fn new_registry() -> GoalLoopRegistry {
    Arc::new(Mutex::new(HashMap::new()))
}

/// Stuck window for a headless role turn (planner/evaluator/digester). Not a
/// wall-clock cap; the turn is additionally bounded by `per_phase_timeout_secs`.
const ROLE_NO_PROGRESS: Duration = Duration::from_secs(240);
/// Idle thresholds for an executor session (mirror review's tuning).
const EXECUTOR_WAITING_IDLE: Duration = Duration::from_secs(45);
const EXECUTOR_STUCK_IDLE: Duration = Duration::from_secs(180);
const EXECUTOR_RETRY_BACKOFF: Duration = Duration::from_secs(3);
/// Cooperative-cancel poll slice.
const SLICE: Duration = Duration::from_millis(500);
/// Absolute controller-lifetime backstop, regardless of config.
const HARD_CAP: Duration = Duration::from_secs(4 * 60 * 60);

fn model_opt(model: &str) -> Option<&str> {
    if model.trim().is_empty() {
        None
    } else {
        Some(model)
    }
}

// --- Lifecycle -------------------------------------------------------------

/// Start (or resume) a loop: provision its worktree, mark it Running, register
/// the control handle (cancelling any prior controller), and spawn the
/// controller task. Errors (e.g. bad repo) surface to the caller.
pub async fn start_loop(ctx: &ServerCtx, loop_id: &Id) -> Result<()> {
    let loop_ = ctx.goal_loops_repo.get(loop_id).await?;
    let (branch, wt, base) = crate::goal_loop_workspace::provision_worktree(ctx, &loop_).await?;
    ctx.goal_loops_repo
        .set_branch(loop_id, &branch, &wt, &base)
        .await?;
    ctx.goal_loops_repo
        .mark_running(loop_id, Utc::now())
        .await?;

    let handle = LoopHandle::new();
    {
        let mut reg = ctx.goal_loops.lock().unwrap();
        if let Some(old) = reg.insert(loop_id.to_string(), handle.clone()) {
            old.cancel.store(true, Ordering::Relaxed);
        }
    }
    let loop_id = loop_id.clone();
    let ws = loop_.workspace_id.clone();
    emit(ctx, &ws, &loop_id, GoalLoopStatus::Running, GoalLoopPhase::Planning, loop_.current_iteration, loop_.progress_pct);
    let ctx2 = ctx.clone();
    tokio::spawn(async move {
        controller(ctx2, loop_id, handle).await;
    });
    Ok(())
}

/// Pause a running loop: bank the current active window so the time budget can't
/// be refunded, then flag the controller to idle.
pub async fn pause_loop(ctx: &ServerCtx, loop_id: &Id) -> Result<()> {
    let loop_ = ctx.goal_loops_repo.get(loop_id).await?;
    if let Some(started) = loop_.run_started_at {
        let secs = (Utc::now() - started).num_seconds().max(0) as u64;
        ctx.goal_loops_repo.add_elapsed(loop_id, secs).await?;
    }
    ctx.goal_loops_repo.set_run_started_at(loop_id, None).await?;
    ctx.goal_loops_repo
        .update_runtime(
            loop_id,
            GoalLoopStatus::Paused,
            loop_.phase,
            loop_.current_iteration,
            loop_.progress_pct,
        )
        .await?;
    if let Some(h) = ctx.goal_loops.lock().unwrap().get(&loop_id.to_string()) {
        h.paused.store(true, Ordering::Relaxed);
    }
    emit(ctx, &loop_.workspace_id, loop_id, GoalLoopStatus::Paused, loop_.phase, loop_.current_iteration, loop_.progress_pct);
    Ok(())
}

/// Stop a loop: cancel the controller (it finalizes), or finalize directly when
/// no controller is live. Always kills executor sessions and removes the worktree.
pub async fn stop_loop(ctx: &ServerCtx, loop_id: &Id) -> Result<()> {
    let loop_ = ctx.goal_loops_repo.get(loop_id).await?;
    if loop_.status.is_terminal() {
        return Ok(());
    }
    // Signal the live controller (if any) WITHOUT removing its registry entry —
    // it finalizes itself (the cancel path checks `is_current`, so it still
    // sees its own handle) and deregisters on exit.
    let had_controller = {
        let reg = ctx.goal_loops.lock().unwrap();
        match reg.get(&loop_id.to_string()) {
            Some(h) => {
                h.cancel.store(true, Ordering::Relaxed);
                true
            }
            None => false,
        }
    };
    cleanup_executor_sessions(ctx, &loop_.workspace_id, loop_id).await;
    if !had_controller {
        // No live controller (e.g. Blocked/Exhausted) — finalize directly.
        finalize(ctx, loop_id, GoalLoopStatus::Stopped, Some("stopped by user"), None).await;
    }
    Ok(())
}

/// Re-run one stuck/errored executor for an iteration that is still in flight.
/// (Best-effort convenience mirroring review's per-agent retry; the controller's
/// own recovery loop already retries within an attempt budget.)
pub async fn retry_executor(
    ctx: &ServerCtx,
    loop_id: &Id,
    iter_idx: u32,
    agent_index: usize,
) -> Result<()> {
    let loop_ = ctx.goal_loops_repo.get(loop_id).await?;
    // Only when Blocked: a Running loop's controller may be actively running this
    // very executor slot, and a second run would create a duplicate session and
    // race the `agents_json[index]` write. (Blocked has no live controller.)
    if loop_.status != GoalLoopStatus::Blocked {
        return Err(Error::Invalid(
            "can only retry an executor while the loop is blocked".into(),
        ));
    }
    let iter = ctx.goal_loops_repo.get_iteration(loop_id, iter_idx).await?;
    let exec = loop_
        .config
        .executors
        .get(agent_index)
        .cloned()
        .ok_or_else(|| Error::NotFound("executor".into()))?;
    // Re-run using the persisted prompt for that executor.
    let prompt = std::fs::read_to_string(prompt_path(loop_id, iter_idx, agent_index))
        .map_err(|_| Error::NotFound("executor prompt (nothing to retry)".into()))?;
    let wt = loop_
        .worktree_path
        .clone()
        .ok_or_else(|| Error::Invalid("loop has no worktree".into()))?;
    let cancel = ctx
        .goal_loops
        .lock()
        .unwrap()
        .get(&loop_id.to_string())
        .map(|h| h.cancel.clone());
    let ctx2 = ctx.clone();
    tokio::spawn(async move {
        let _ = run_executor(
            &ctx2,
            &loop_,
            &iter.id,
            iter_idx,
            agent_index,
            &exec,
            &wt,
            &prompt,
            cancel.as_ref(),
        )
        .await;
    });
    Ok(())
}

// --- Controller ------------------------------------------------------------

async fn controller(ctx: ServerCtx, loop_id: Id, handle: LoopHandle) {
    let started = Instant::now();
    // Resume continuity: seed prior evaluation from the last iteration if any.
    let mut prior_eval: Option<GoalLoopEvaluation> = ctx
        .goal_loops_repo
        .get_detail(&loop_id)
        .await
        .ok()
        .and_then(|d| d.iterations.last().and_then(|i| i.evaluation.clone()));

    loop {
        if handle.cancel.load(Ordering::Relaxed) {
            // Finalize ONLY if we're still the registered controller. A
            // start_loop that superseded us (rapid restart) installed a new
            // handle and cancelled ours; in that case the new controller owns
            // the loop and must NOT have its run torn down by us.
            if is_current(&ctx, &loop_id, &handle) {
                finalize(&ctx, &loop_id, GoalLoopStatus::Stopped, Some("stopped by user"), None).await;
            }
            deregister(&ctx, &loop_id, &handle);
            return;
        }
        if handle.paused.load(Ordering::Relaxed) {
            tokio::time::sleep(SLICE).await;
            continue;
        }
        if started.elapsed() >= HARD_CAP {
            finalize(&ctx, &loop_id, GoalLoopStatus::Exhausted, Some("controller lifetime cap reached"), None).await;
            deregister(&ctx, &loop_id, &handle);
            return;
        }

        // Re-read fresh state (picks up raised limits / latest digest).
        let loop_ = match ctx.goal_loops_repo.get(&loop_id).await {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!(loop = %loop_id, "goal-loop: load failed, stopping: {e}");
                deregister(&ctx, &loop_id, &handle);
                return;
            }
        };
        let ws = loop_.workspace_id.clone();
        let limits = loop_.limits.clone();

        // ---- HARD-LIMIT GATE (before starting an iteration) ----
        if loop_.iterations_started >= limits.max_iterations {
            finalize(&ctx, &loop_id, GoalLoopStatus::Exhausted, Some("iteration cap reached"), None).await;
            deregister(&ctx, &loop_id, &handle);
            return;
        }
        if loop_.elapsed_secs_at(Utc::now()) >= limits.max_runtime_secs {
            finalize(&ctx, &loop_id, GoalLoopStatus::Exhausted, Some("time cap reached"), None).await;
            deregister(&ctx, &loop_id, &handle);
            return;
        }
        if let Some(cap) = limits.max_cost_usd {
            if loop_.cost_usd >= cap {
                finalize(&ctx, &loop_id, GoalLoopStatus::Exhausted, Some("cost cap reached"), None).await;
                deregister(&ctx, &loop_id, &handle);
                return;
            }
        }

        // ---- new iteration ----
        let idx = match ctx.goal_loops_repo.bump_iterations_started(&loop_id).await {
            Ok(n) => n,
            Err(e) => {
                tracing::warn!(loop = %loop_id, "goal-loop: bump failed: {e}");
                finalize(&ctx, &loop_id, GoalLoopStatus::Failed, None, Some(&e.to_string())).await;
                deregister(&ctx, &loop_id, &handle);
                return;
            }
        };
        let context_in = loop_.context_digest.clone();
        let iter = match ctx
            .goal_loops_repo
            .add_iteration(&loop_id, &ws, idx, &context_in, &loop_.config.executors)
            .await
        {
            Ok(it) => it,
            Err(e) => {
                tracing::warn!(loop = %loop_id, "goal-loop: add_iteration failed: {e}");
                finalize(&ctx, &loop_id, GoalLoopStatus::Failed, None, Some(&e.to_string())).await;
                deregister(&ctx, &loop_id, &handle);
                return;
            }
        };
        let wt = loop_.worktree_path.clone().unwrap_or_else(|| loop_.repo_path.clone());

        // ---- PLAN ----
        set_phase(&ctx, &ws, &loop_, GoalLoopPhase::Planning, idx).await;
        let plan = match run_role(&ctx, &loop_.config.planner, planner_prompt(&loop_, &context_in, prior_eval.as_ref(), idx), &wt, limits.per_phase_timeout_secs).await {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(loop = %loop_id, "goal-loop: planner failed: {e}");
                String::from("(planner unavailable — executors should work directly toward the acceptance criteria)")
            }
        };
        let _ = ctx.goal_loops_repo.set_iter_plan(&iter.id, &plan).await;
        if stop_requested(&handle) {
            continue;
        }

        // ---- EXECUTE (sequential) ----
        set_phase(&ctx, &ws, &loop_, GoalLoopPhase::Executing, idx).await;
        let _ = ctx.goal_loops_repo.update_iteration_status(&iter.id, "executing", false).await;
        let mut exec_summaries: Vec<String> = Vec::new();
        for (i, exec) in loop_.config.executors.iter().enumerate() {
            if stop_requested(&handle) {
                break;
            }
            let out_path = executor_out_path(&loop_id, idx, i);
            let _ = std::fs::remove_file(&out_path);
            let prompt = executor_prompt(&loop_, exec, &plan, &context_in, &out_path.to_string_lossy());
            let _ = std::fs::write(prompt_path(&loop_id, idx, i), &prompt);
            let summary = run_executor(&ctx, &loop_, &iter.id, idx, i, exec, &wt, &prompt, Some(&handle.cancel)).await;
            exec_summaries.push(format!("{}: {}", exec.name, summary));
        }
        if stop_requested(&handle) {
            continue;
        }

        // ---- EVALUATE ----
        set_phase(&ctx, &ws, &loop_, GoalLoopPhase::Evaluating, idx).await;
        let _ = ctx.goal_loops_repo.update_iteration_status(&iter.id, "evaluating", false).await;
        let eval = evaluate(&ctx, &loop_, &wt, &exec_summaries, limits.per_phase_timeout_secs).await;
        let _ = ctx.goal_loops_repo.set_iter_evaluation(&iter.id, &eval).await;
        let progress = eval.progress_pct.min(100);
        let _ = ctx
            .goal_loops_repo
            .update_runtime(&loop_id, GoalLoopStatus::Running, GoalLoopPhase::Evaluating, idx, progress)
            .await;
        emit(&ctx, &ws, &loop_id, GoalLoopStatus::Running, GoalLoopPhase::Evaluating, idx, progress);

        // ---- DIGEST ----
        set_phase(&ctx, &ws, &loop_, GoalLoopPhase::Digesting, idx).await;
        let _ = ctx.goal_loops_repo.update_iteration_status(&iter.id, "digesting", false).await;
        let digest = run_role(&ctx, &loop_.config.digester, digester_prompt(&context_in, &plan, &exec_summaries, &eval), &wt, limits.per_phase_timeout_secs)
            .await
            .unwrap_or(context_in);
        let _ = ctx.goal_loops_repo.set_iter_context_out(&iter.id, &digest).await;
        let _ = ctx.goal_loops_repo.set_context_digest(&loop_id, &digest).await;
        let _ = ctx.goal_loops_repo.update_iteration_status(&iter.id, "done", true).await;
        prior_eval = Some(eval.clone());

        // ---- DECISION ----
        let all_met = !eval.criteria.is_empty() && eval.criteria.iter().all(|c| c.met);
        if eval.verdict == "achieved" && all_met {
            let summary = format!("Goal achieved in {idx} iteration(s). {}", eval.rationale);
            finalize(&ctx, &loop_id, GoalLoopStatus::Succeeded, Some(&summary), None).await;
            deregister(&ctx, &loop_id, &handle);
            return;
        }
        if eval.verdict == "blocked" {
            let summary = format!("Blocked — needs input. {}", eval.feedback);
            // Bank the active window and stop ticking; resume requires user action.
            block(&ctx, &loop_id, &summary).await;
            emit(&ctx, &ws, &loop_id, GoalLoopStatus::Blocked, GoalLoopPhase::Done, idx, progress);
            // Surface a free-form notice so the user knows to intervene.
            let _ = ctx.events.send(Event::Notice {
                level: "warn".into(),
                title: "Goal loop blocked — needs input".into(),
                body: summary.clone(),
            });
            deregister(&ctx, &loop_id, &handle);
            return;
        }
        // verdict == "continue" (or anything else) → next iteration.
    }
}

fn stop_requested(handle: &LoopHandle) -> bool {
    handle.cancel.load(Ordering::Relaxed) || handle.paused.load(Ordering::Relaxed)
}

/// True when `handle` is still the registry's current controller for this loop
/// (i.e. a later start_loop hasn't superseded it).
fn is_current(ctx: &ServerCtx, loop_id: &Id, handle: &LoopHandle) -> bool {
    ctx.goal_loops
        .lock()
        .unwrap()
        .get(&loop_id.to_string())
        .map(|h| Arc::ptr_eq(&h.cancel, &handle.cancel))
        .unwrap_or(false)
}

fn deregister(ctx: &ServerCtx, loop_id: &Id, handle: &LoopHandle) {
    let mut reg = ctx.goal_loops.lock().unwrap();
    // Only remove the entry if it is still OURS (a newer start_loop may have
    // replaced it).
    if let Some(h) = reg.get(&loop_id.to_string()) {
        if Arc::ptr_eq(&h.cancel, &handle.cancel) {
            reg.remove(&loop_id.to_string());
        }
    }
}

async fn set_phase(ctx: &ServerCtx, ws: &Id, loop_: &GoalLoop, phase: GoalLoopPhase, idx: u32) {
    let _ = ctx
        .goal_loops_repo
        .update_runtime(&loop_.id, GoalLoopStatus::Running, phase, idx, loop_.progress_pct)
        .await;
    emit(ctx, ws, &loop_.id, GoalLoopStatus::Running, phase, idx, loop_.progress_pct);
}

/// Bank the final active window, mark the loop terminal, remove the worktree
/// (keeping the branch), and kill any lingering executor sessions.
async fn finalize(
    ctx: &ServerCtx,
    loop_id: &Id,
    status: GoalLoopStatus,
    summary: Option<&str>,
    error: Option<&str>,
) {
    let loop_ = match ctx.goal_loops_repo.get(loop_id).await {
        Ok(l) => l,
        Err(_) => return,
    };
    if let Some(started) = loop_.run_started_at {
        let secs = (Utc::now() - started).num_seconds().max(0) as u64;
        let _ = ctx.goal_loops_repo.add_elapsed(loop_id, secs).await;
    }
    let _ = ctx.goal_loops_repo.finalize(loop_id, status, summary, error).await;
    // Remove the worktree only for TRULY terminal states. Exhausted is
    // resumable (raise limits + Resume), so keep its worktree — recreating it
    // later would `-B`-reset the branch and destroy the loop's commits.
    if matches!(
        status,
        GoalLoopStatus::Succeeded | GoalLoopStatus::Failed | GoalLoopStatus::Stopped
    ) {
        crate::goal_loop_workspace::remove_worktree(ctx, &loop_).await;
    }
    cleanup_executor_sessions(ctx, &loop_.workspace_id, loop_id).await;
    emit(ctx, &loop_.workspace_id, loop_id, status, GoalLoopPhase::Done, loop_.current_iteration, loop_.progress_pct);
}

/// Block the loop (awaiting user) without treating it as terminal-finished: bank
/// the window, clear the anchor, set status Blocked. Resume re-spawns a controller.
async fn block(ctx: &ServerCtx, loop_id: &Id, summary: &str) {
    let loop_ = match ctx.goal_loops_repo.get(loop_id).await {
        Ok(l) => l,
        Err(_) => return,
    };
    if let Some(started) = loop_.run_started_at {
        let secs = (Utc::now() - started).num_seconds().max(0) as u64;
        let _ = ctx.goal_loops_repo.add_elapsed(loop_id, secs).await;
    }
    let _ = ctx
        .goal_loops_repo
        .finalize(loop_id, GoalLoopStatus::Blocked, Some(summary), None)
        .await;
    // Blocked is resumable — KEEP the worktree (and its commits) so Resume
    // continues on top of the loop's work. Live executor sessions are killed.
    cleanup_executor_sessions(ctx, &loop_.workspace_id, loop_id).await;
}

/// List + kill all executor sessions tagged for this loop.
pub async fn cleanup_executor_sessions(ctx: &ServerCtx, ws_id: &Id, loop_id: &Id) {
    let sessions = match ctx.manager.list_by_workspace(ws_id).await {
        Ok(s) => s,
        Err(_) => return,
    };
    for s in sessions {
        let is_ours = s.meta.get("source").and_then(|v| v.as_str()) == Some("goal_loop")
            && s.meta.get("loop_id").and_then(|v| v.as_str()) == Some(loop_id.as_str());
        if is_ours {
            let _ = ctx.manager.kill_session(&s.id).await;
        }
    }
}

fn emit(
    ctx: &ServerCtx,
    ws: &Id,
    loop_id: &Id,
    status: GoalLoopStatus,
    phase: GoalLoopPhase,
    current_iteration: u32,
    progress_pct: u32,
) {
    let _ = ctx.events.send(Event::GoalLoopUpdated {
        workspace_id: ws.clone(),
        loop_id: loop_id.clone(),
        status: status.as_str().to_string(),
        phase: phase.as_str().to_string(),
        current_iteration,
        progress_pct,
    });
}

// --- Roles (headless, timeout-wrapped) -------------------------------------

async fn run_role(
    ctx: &ServerCtx,
    role: &GoalLoopRoleCfg,
    prompt: String,
    cwd: &str,
    per_phase_secs: u64,
) -> Result<String> {
    let fut = ctx
        .orchestrator
        .run_agent(&prompt, cwd, model_opt(&role.model), ROLE_NO_PROGRESS);
    match tokio::time::timeout(Duration::from_secs(per_phase_secs), fut).await {
        Ok(r) => r,
        Err(_) => Err(Error::Internal("role turn exceeded per-phase timeout".into())),
    }
}

/// The public goal context shared by every role/executor prompt.
fn goal_context(loop_: &GoalLoop) -> String {
    let d = &loop_.definition;
    let mut s = format!("# GOAL: {}\n", d.title);
    if !d.summary.is_empty() {
        s.push_str(&format!("{}\n", d.summary));
    }
    if !d.objectives.is_empty() {
        s.push_str("\n## Objectives\n");
        for o in &d.objectives {
            s.push_str(&format!("- {o}\n"));
        }
    }
    s.push_str("\n## Acceptance criteria (the loop stops only when ALL are met)\n");
    for c in &d.acceptance_criteria {
        s.push_str(&format!("- [{}] {} — verify: {}\n", c.id, c.text, c.verify));
    }
    if !d.constraints.is_empty() {
        s.push_str("\n## Constraints\n");
        for c in &d.constraints {
            s.push_str(&format!("- {c}\n"));
        }
    }
    if !d.out_of_scope.is_empty() {
        s.push_str("\n## Out of scope\n");
        for o in &d.out_of_scope {
            s.push_str(&format!("- {o}\n"));
        }
    }
    s
}

fn planner_prompt(loop_: &GoalLoop, context_in: &str, prior: Option<&GoalLoopEvaluation>, idx: u32) -> String {
    let mut s = format!("{}\n\n{}\n\n", loop_.config.planner.prompt, goal_context(loop_));
    s.push_str(&format!("This is iteration {idx} of at most {}.\n", loop_.limits.max_iterations));
    if !context_in.is_empty() {
        s.push_str(&format!("\n## Context so far (auxiliary memory)\n{context_in}\n"));
    }
    if let Some(p) = prior {
        let unmet: Vec<&str> = p.criteria.iter().filter(|c| !c.met).map(|c| c.id.as_str()).collect();
        s.push_str(&format!(
            "\n## Previous evaluation\nprogress: {}% · verdict: {}\nunmet criteria: {}\nfeedback: {}\n",
            p.progress_pct, p.verdict, unmet.join(", "), p.feedback
        ));
    }
    s.push_str("\nProduce a concrete, minimal plan for THIS iteration to close the unmet criteria. Reply with the plan as plain text.");
    s
}

fn executor_prompt(loop_: &GoalLoop, exec: &GoalLoopAgentCfg, plan: &str, context_in: &str, out_path: &str) -> String {
    let mut s = format!(
        "You are \"{}\", an executor on an autonomous goal loop working in THIS git repository (an isolated worktree — make changes freely).\n\n{}\n\n",
        exec.name,
        goal_context(loop_)
    );
    if !exec.prompt_extra.trim().is_empty() {
        s.push_str(&format!("## Your focus\n{}\n\n", exec.prompt_extra));
    }
    if !context_in.is_empty() {
        s.push_str(&format!("## Context so far\n{context_in}\n\n"));
    }
    s.push_str(&format!("## This iteration's plan\n{plan}\n\n"));
    s.push_str(&format!(
        "---\nDo the work toward the acceptance criteria, then COMMIT your changes (git add -A && git commit). \
         Finally, write a JSON object describing what you did to this absolute file path, overwriting any existing content:\n\n{out_path}\n\n\
         Schema: {{\"summary\": string, \"changed_files\": [string], \"notes\": string, \"blockers\": [string]}}.\n\
         Write ONLY the JSON object to that file (no prose, no markdown fence). Writing the file is the LAST thing you do."
    ));
    s
}

fn digester_prompt(context_in: &str, plan: &str, exec_summaries: &[String], eval: &GoalLoopEvaluation) -> String {
    format!(
        "Compress the running context plus this iteration into a concise summary (a few hundred words max): what is done, current state, what remains, key decisions, blockers. The git worktree is the source of truth; this is auxiliary memory.\n\n## Prior context\n{context_in}\n\n## This iteration plan\n{plan}\n\n## Executor results\n{}\n\n## Evaluation\nprogress {}% · verdict {} · feedback: {}\n\nReply with ONLY the updated summary text.",
        exec_summaries.join("\n"),
        eval.progress_pct,
        eval.verdict,
        eval.feedback,
    )
}

/// Run the evaluator headless in the worktree, then GROUND-TRUTH every
/// command-verify criterion by running its command (exit 0 = met), overriding the
/// model. An "achieved" verdict with any unmet criterion is coerced to "continue".
async fn evaluate(
    ctx: &ServerCtx,
    loop_: &GoalLoop,
    wt: &str,
    exec_summaries: &[String],
    per_phase_secs: u64,
) -> GoalLoopEvaluation {
    let prompt = format!(
        "{}\n\n{}\n\n## Executor results this iteration\n{}\n\nReply with ONLY a JSON object: {{\"progress_pct\": 0-100, \"verdict\": \"achieved|continue|blocked\", \"criteria\": [{{\"id\": string, \"met\": bool, \"evidence\": string}}], \"feedback\": string, \"rationale\": string}}. Include one entry per acceptance criterion. Provide concrete evidence for every met=true.",
        loop_.config.evaluator.prompt,
        goal_context(loop_),
        exec_summaries.join("\n"),
    );
    let mut eval = match run_role(ctx, &loop_.config.evaluator, prompt, wt, per_phase_secs).await {
        Ok(text) => parse_evaluation(&text).unwrap_or_else(|| fallback_eval("evaluator produced no parseable verdict")),
        Err(e) => fallback_eval(&format!("evaluator turn failed: {e}")),
    };

    // Ground-truth command criteria by exit code.
    for crit in &loop_.definition.acceptance_criteria {
        if crit.verify_kind == "command" {
            if let Some(cmd) = &crit.verify_cmd {
                let passed = run_verify_cmd(wt, cmd, per_phase_secs).await;
                set_criterion(&mut eval, &crit.id, passed, if passed { "verify command exited 0" } else { "verify command failed" });
            }
        }
    }

    // Ensure every defined criterion is represented (default unmet).
    for crit in &loop_.definition.acceptance_criteria {
        if !eval.criteria.iter().any(|c| c.id == crit.id) {
            eval.criteria.push(otto_core::domain::EvalCriterion {
                id: crit.id.clone(),
                met: false,
                evidence: "not assessed".into(),
            });
        }
    }

    // Reconcile an over-optimistic verdict against ground truth.
    let all_met = !eval.criteria.is_empty() && eval.criteria.iter().all(|c| c.met);
    if eval.verdict == "achieved" && !all_met {
        eval.verdict = "continue".into();
        eval.rationale = format!("{} (coerced: verdict was 'achieved' but not all criteria met)", eval.rationale);
    }
    eval
}

fn fallback_eval(reason: &str) -> GoalLoopEvaluation {
    GoalLoopEvaluation {
        progress_pct: 0,
        verdict: "continue".into(),
        criteria: Vec::new(),
        feedback: reason.to_string(),
        rationale: reason.to_string(),
    }
}

fn set_criterion(eval: &mut GoalLoopEvaluation, id: &str, met: bool, evidence: &str) {
    if let Some(c) = eval.criteria.iter_mut().find(|c| c.id == id) {
        c.met = met;
        c.evidence = evidence.to_string();
    } else {
        eval.criteria.push(otto_core::domain::EvalCriterion {
            id: id.to_string(),
            met,
            evidence: evidence.to_string(),
        });
    }
}

/// Run a verify command in the worktree, returning true on exit 0. Bounded by
/// the per-phase timeout so a hanging test can't wedge the loop.
async fn run_verify_cmd(cwd: &str, cmd: &str, per_phase_secs: u64) -> bool {
    let fut = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(cwd)
        .output();
    match tokio::time::timeout(Duration::from_secs(per_phase_secs), fut).await {
        Ok(Ok(out)) => out.status.success(),
        _ => false,
    }
}

// --- Executors (live sessions) ---------------------------------------------

fn tmp_dir() -> PathBuf {
    PathBuf::from(std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".to_string()))
}

fn executor_out_path(loop_id: &str, idx: u32, exec: usize) -> PathBuf {
    tmp_dir().join(format!("otto-goalloop-{loop_id}-{idx}-{exec}.json"))
}

fn prompt_path(loop_id: &str, idx: u32, exec: usize) -> PathBuf {
    tmp_dir().join(format!("otto-goalloop-{loop_id}-{idx}-{exec}.prompt"))
}

fn never(_: &str) -> bool {
    false
}

/// Run one executor with bounded recovery; persists its live state throughout.
/// Returns a short result summary for the evaluator/digester.
#[allow(clippy::too_many_arguments)]
async fn run_executor(
    ctx: &ServerCtx,
    loop_: &GoalLoop,
    iter_id: &Id,
    idx: u32,
    exec_index: usize,
    exec: &GoalLoopAgentCfg,
    cwd: &str,
    prompt: &str,
    cancel: Option<&Arc<AtomicBool>>,
) -> String {
    let out_path = executor_out_path(&loop_.id, idx, exec_index);
    let timeout = Duration::from_secs(loop_.limits.per_phase_timeout_secs);
    let attempts = loop_.limits.max_attempts_per_executor.max(1);

    let outcome = run_with_recovery(
        &ctx.manager,
        attempts,
        &[EXECUTOR_RETRY_BACKOFF],
        cancel,
        |_attempt| {
            run_executor_attempt(ctx, loop_, iter_id, idx, exec_index, exec, cwd, prompt, &out_path, timeout)
        },
    )
    .await;

    // Persist terminal state once.
    if let Some(raw) = outcome.raw.as_deref() {
        let parsed = parse_executor_result(raw);
        let summary = parsed
            .as_ref()
            .map(|r| r.summary.clone())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "(completed; no summary)".to_string());
        persist_agent(
            ctx,
            iter_id,
            exec_index,
            exec,
            "done",
            &summary,
            outcome.session_id.clone(),
            Some(summary.clone()),
        )
        .await;
        summary
    } else {
        let note = executor_error_note(outcome.reason);
        persist_agent(ctx, iter_id, exec_index, exec, "error", &note, outcome.session_id.clone(), None).await;
        format!("(error: {note})")
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_executor_attempt(
    ctx: &ServerCtx,
    loop_: &GoalLoop,
    iter_id: &Id,
    idx: u32,
    exec_index: usize,
    exec: &GoalLoopAgentCfg,
    cwd: &str,
    prompt: &str,
    out_path: &std::path::Path,
    timeout: Duration,
) -> RunOutcome {
    let provider = if exec.provider.is_empty() { "claude" } else { &exec.provider };
    let meta = serde_json::json!({
        "source": "goal_loop",
        "loop_id": loop_.id,
        "iter_idx": idx,
        "agent_index": exec_index,
    });
    let req = CreateSessionReq {
        kind: SessionKind::Agent,
        provider: Some(provider.to_string()),
        title: Some(format!("Loop: {} (iter {idx})", exec.name)),
        cwd: Some(cwd.to_string()),
        connection_id: None,
        meta: Some(meta),
    };
    // We need a Workspace + user to create the session. The session manager
    // accepts the workspace by value; load it.
    let ws = match ctx.workspaces.get(&loop_.workspace_id).await {
        Ok(w) => w,
        Err(e) => {
            tracing::warn!("goal-loop: load workspace: {e}");
            return RunOutcome::failed(None, FailReason::CreateFailed);
        }
    };
    let session = match ctx.manager.create(&ws, &loop_.created_by, req, None).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("goal-loop: create executor session: {e}");
            return RunOutcome::failed(None, FailReason::CreateFailed);
        }
    };
    let sid = session.id.clone();
    persist_agent(ctx, iter_id, exec_index, exec, "running", "", Some(sid.clone()), None).await;

    if wait_for_tui(&ctx.manager, &sid).await {
        let _ = ctx.manager.input(&sid, &bracketed_paste(prompt)).await;
        tokio::time::sleep(PASTE_TO_ENTER).await;
        let before = ctx.manager.live_handle(&sid).map(|h| h.last_output_at());
        let _ = ctx.manager.input(&sid, b"\r").await;
        if !dispatched(&ctx.manager, &sid, before).await {
            let _ = ctx.manager.input(&sid, b"\r").await;
        }
    }

    watch_for_result(
        &ctx.manager,
        &sid,
        provider,
        session.provider_session_id.as_deref(),
        cwd,
        out_path,
        timeout,
        EXECUTOR_WAITING_IDLE,
        EXECUTOR_STUCK_IDLE,
        never,
        |st| {
            let ctx = ctx.clone();
            let exec = exec.clone();
            let iter_id = iter_id.clone();
            let sid = sid.clone();
            let ws = loop_.workspace_id.clone();
            let loop_id = loop_.id.clone();
            let progress = loop_.progress_pct;
            async move {
                let (status, note, phase) = match st {
                    WatchStatus::Waiting => (
                        "waiting",
                        "looks blocked on input — Open it to respond".to_string(),
                        GoalLoopPhase::Waiting,
                    ),
                    WatchStatus::Resumed => ("running", String::new(), GoalLoopPhase::Executing),
                };
                persist_agent(&ctx, &iter_id, exec_index, &exec, status, &note, Some(sid), None).await;
                let _ = ctx.goal_loops_repo.set_phase(&loop_id, phase).await;
                emit(&ctx, &ws, &loop_id, GoalLoopStatus::Running, phase, idx, progress);
            }
        },
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn persist_agent(
    ctx: &ServerCtx,
    iter_id: &Id,
    index: usize,
    exec: &GoalLoopAgentCfg,
    status: &str,
    note: &str,
    session_id: Option<Id>,
    output_summary: Option<String>,
) {
    let st = LoopAgentState {
        name: exec.name.clone(),
        provider: exec.provider.clone(),
        model: exec.model.clone(),
        status: status.to_string(),
        note: note.to_string(),
        session_id,
        output_summary,
    };
    let _ = ctx.goal_loops_repo.set_iter_agent_at(iter_id, index, &st).await;
}

fn executor_error_note(reason: Option<FailReason>) -> String {
    match reason {
        Some(FailReason::Stuck) => "stuck — no output for ~3m",
        Some(FailReason::Timeout) => "timed out (per-phase grace elapsed)",
        Some(FailReason::Exited) => "session exited before writing its result",
        Some(FailReason::SessionGone) => "session is no longer live",
        Some(FailReason::CreateFailed) => "could not start",
        Some(FailReason::Stopped) => "stopped",
        None => "unknown error",
    }
    .to_string()
}
