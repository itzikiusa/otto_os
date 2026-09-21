//! The Goal Loop engine: a per-loop controller that runs bounded
//! Plan → Execute → Evaluate → Digest iterations toward a goal with
//! machine-checked acceptance criteria, on an isolated git branch, until the
//! goal is met or a hard limit (iterations / active time) is hit.
//!
//! Reuse is plumbing-only: executors run as live, openable [`SessionManager`]
//! sessions (review-style spawn/inject/watch via [`crate::agent_run`] +
//! [`crate::review_session`]); the planner/evaluator/digester/definer are
//! managed provider-aware turns with absolute deadlines. The
//! concurrency/safety model is OURS: v1 runs executors SEQUENTIALLY on one
//! worktree (no git-index races), the evaluator ground-truths command criteria,
//! and the controller is the sole writer of the loop's runtime fields.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::Utc;
use otto_core::api::CreateSessionReq;
use otto_core::domain::{
    GoalLoop, GoalLoopAgentCfg, GoalLoopEvaluation, GoalLoopPhase, GoalLoopRoleCfg, GoalLoopStatus,
    LoopAgentState, SessionKind,
};
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
    pub interrupted: Arc<AtomicBool>,
}

impl LoopHandle {
    pub fn new() -> Self {
        Self {
            cancel: Arc::new(AtomicBool::new(false)),
            paused: Arc::new(AtomicBool::new(false)),
            interrupted: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl Default for LoopHandle {
    fn default() -> Self {
        Self::new()
    }
}

/// loop_id → live controller handle.
pub type GoalLoopRegistry = Arc<LoopRegistry>;

#[derive(Default)]
pub struct LoopRegistry {
    handles: Mutex<HashMap<String, LoopHandle>>,
    operations: Mutex<HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>>,
}
impl LoopRegistry {
    pub fn lock(
        &self,
    ) -> std::sync::LockResult<std::sync::MutexGuard<'_, HashMap<String, LoopHandle>>> {
        self.handles.lock()
    }
    /// Serialize API decisions and mutations, including approval versus retry.
    pub fn operation(&self, id: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut operations = self.operations.lock().unwrap();
        operations.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = operations.get(id).and_then(std::sync::Weak::upgrade) {
            return lock;
        }
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        operations.insert(id.into(), Arc::downgrade(&lock));
        lock
    }
    pub fn require_idle(&self, id: &str) -> Result<()> {
        if self.handles.lock().unwrap().contains_key(id) {
            return Err(Error::Conflict(
                "loop work is still active or stopping; retry when it settles".into(),
            ));
        }
        Ok(())
    }
}

pub fn new_registry() -> GoalLoopRegistry {
    Arc::new(LoopRegistry::default())
}

/// Idle thresholds for an executor session (mirror review's tuning).
const EXECUTOR_WAITING_IDLE: Duration = Duration::from_secs(45);
const EXECUTOR_STUCK_IDLE: Duration = Duration::from_secs(180);
const EXECUTOR_RETRY_BACKOFF: Duration = Duration::from_secs(3);
/// Absolute controller-lifetime backstop, regardless of config.
const HARD_CAP: Duration = Duration::from_secs(4 * 60 * 60);

// --- Lifecycle -------------------------------------------------------------

/// Start (or resume) a loop: provision its worktree, mark it Running, register
/// the control handle (cancelling any prior controller), and spawn the
/// controller task. Errors (e.g. bad repo) surface to the caller.
pub async fn start_loop(ctx: &ServerCtx, loop_id: &Id) -> Result<()> {
    ctx.goal_loops.require_idle(loop_id)?;
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
    emit(
        ctx,
        &ws,
        &loop_id,
        GoalLoopStatus::Running,
        GoalLoopPhase::Planning,
        loop_.current_iteration,
        loop_.progress_pct,
    );
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
    if let Some(h) = ctx.goal_loops.lock().unwrap().get(loop_id) {
        h.paused.store(true, Ordering::Relaxed);
        h.interrupted.store(true, Ordering::Relaxed);
    }
    cleanup_executor_sessions(ctx, &loop_.workspace_id, loop_id).await;
    if let Some(started) = loop_.run_started_at {
        let secs = (Utc::now() - started).num_seconds().max(0) as u64;
        ctx.goal_loops_repo.add_elapsed(loop_id, secs).await?;
    }
    ctx.goal_loops_repo
        .set_run_started_at(loop_id, None)
        .await?;
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
    emit(
        ctx,
        &loop_.workspace_id,
        loop_id,
        GoalLoopStatus::Paused,
        loop_.phase,
        loop_.current_iteration,
        loop_.progress_pct,
    );
    Ok(())
}

/// Stop a loop: cancel the controller or finalize directly. Execution resources
/// are released while tracked and untracked working files remain intact.
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
                h.interrupted.store(true, Ordering::Relaxed);
                true
            }
            None => false,
        }
    };
    cleanup_executor_sessions(ctx, &loop_.workspace_id, loop_id).await;
    if !had_controller {
        // No live controller (e.g. Blocked/Exhausted) — finalize directly.
        finalize(
            ctx,
            loop_id,
            GoalLoopStatus::Stopped,
            Some("stopped by user"),
            None,
        )
        .await;
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
    ctx.goal_loops.require_idle(loop_id)?;
    if iter_idx != loop_.current_iteration {
        return Err(Error::Invalid(
            "only the current iteration can be retried".into(),
        ));
    }
    let prompt = std::fs::read_to_string(prompt_path(loop_id, iter_idx, agent_index))
        .map_err(|_| Error::NotFound("executor prompt (nothing to retry)".into()))?;
    let wt = loop_
        .worktree_path
        .clone()
        .ok_or_else(|| Error::Invalid("loop has no worktree".into()))?;
    let mut ledger = loop_.ledger.clone();
    ledger.verifications.clear();
    ledger.review_passed = false;
    ctx.goal_loops_repo.set_ledger(loop_id, &ledger).await?;
    // Old acceptance applies to old work. A retry must earn a fresh evaluation.
    ctx.goal_loops_repo
        .set_iter_evaluation(
            &iter.id,
            &fallback_eval("executor retried; fresh evaluation required"),
        )
        .await?;
    let out_path = executor_out_path(loop_id, iter_idx, agent_index);
    if let Err(error) = std::fs::remove_file(&out_path) {
        if error.kind() != std::io::ErrorKind::NotFound {
            return Err(Error::Internal(error.to_string()));
        }
    }
    ctx.goal_loops_repo
        .mark_running(loop_id, Utc::now())
        .await?;
    let handle = LoopHandle::new();
    ctx.goal_loops
        .lock()
        .unwrap()
        .insert(loop_id.clone(), handle.clone());
    set_phase(
        ctx,
        &loop_.workspace_id,
        &loop_,
        GoalLoopPhase::Executing,
        iter_idx,
    )
    .await;
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
            Some(&handle.interrupted),
        )
        .await;
        let operation = ctx2.goal_loops.operation(&loop_.id);
        let _guard = operation.lock().await;
        if is_current(&ctx2, &loop_.id, &handle) {
            if handle.cancel.load(Ordering::Relaxed) {
                finalize(
                    &ctx2,
                    &loop_.id,
                    GoalLoopStatus::Stopped,
                    Some("stopped by user"),
                    None,
                )
                .await;
            } else if handle.paused.load(Ordering::Relaxed) {
                cleanup_executor_sessions(&ctx2, &loop_.workspace_id, &loop_.id).await;
            } else {
                block(
                    &ctx2,
                    &loop_.id,
                    "Executor retry finished. Resume for a fresh evaluation.",
                )
                .await;
                emit(
                    &ctx2,
                    &loop_.workspace_id,
                    &loop_.id,
                    GoalLoopStatus::Blocked,
                    GoalLoopPhase::Done,
                    iter_idx,
                    loop_.progress_pct,
                );
            }
        }
        deregister(&ctx2, &loop_.id, &handle);
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
                finalize(
                    &ctx,
                    &loop_id,
                    GoalLoopStatus::Stopped,
                    Some("stopped by user"),
                    None,
                )
                .await;
            }
            deregister(&ctx, &loop_id, &handle);
            return;
        }
        if handle.paused.load(Ordering::Relaxed) {
            if let Ok(loop_) = ctx.goal_loops_repo.get(&loop_id).await {
                cleanup_executor_sessions(&ctx, &loop_.workspace_id, &loop_id).await;
            }
            deregister(&ctx, &loop_id, &handle);
            return;
        }
        if started.elapsed() >= HARD_CAP {
            finalize(
                &ctx,
                &loop_id,
                GoalLoopStatus::Exhausted,
                Some("controller lifetime cap reached"),
                None,
            )
            .await;
            deregister(&ctx, &loop_id, &handle);
            return;
        }

        // Re-read fresh state (picks up raised limits / latest digest).
        let mut loop_ = match ctx.goal_loops_repo.get(&loop_id).await {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!(loop = %loop_id, "goal-loop: load failed, stopping: {e}");
                deregister(&ctx, &loop_id, &handle);
                return;
            }
        };
        let ws = loop_.workspace_id.clone();
        let limits = loop_.limits.clone();

        // Human acceptance finishes the existing iteration without re-running
        // executors or spending another iteration. Verification still has its
        // phase/runtime deadline and command checks run again.
        let verification_only = prior_eval.as_ref().is_some_and(|e| {
            e.criteria
                .iter()
                .any(|c| c.evidence == "Awaiting human verification")
                && loop_.definition.acceptance_criteria.iter().all(|c| {
                    if c.verify_kind == "human" {
                        loop_.ledger.verification(c).is_some()
                    } else {
                        e.criteria.iter().any(|v| v.id == c.id && v.met)
                    }
                })
        });

        // ---- HARD-LIMIT GATE (before starting an iteration) ----
        if !verification_only && loop_.iterations_started >= limits.max_iterations {
            finalize(
                &ctx,
                &loop_id,
                GoalLoopStatus::Exhausted,
                Some("iteration cap reached"),
                None,
            )
            .await;
            deregister(&ctx, &loop_id, &handle);
            return;
        }
        if loop_.elapsed_secs_at(Utc::now()) >= limits.max_runtime_secs {
            finalize(
                &ctx,
                &loop_id,
                GoalLoopStatus::Exhausted,
                Some("time cap reached"),
                None,
            )
            .await;
            deregister(&ctx, &loop_id, &handle);
            return;
        }
        if limits.max_cost_usd.is_some() {
            block(&ctx, &loop_id, "Per-loop cost accounting is unavailable. Remove the cost limit and use iteration/runtime limits to resume.").await;
            deregister(&ctx, &loop_id, &handle);
            return;
        }

        // ---- new iteration (or final human verification of the current one) ----
        let context_in = loop_.context_digest.clone();
        let created = if verification_only {
            ctx.goal_loops_repo
                .get_iteration(&loop_id, loop_.current_iteration)
                .await
        } else {
            // Any further executor work invalidates approvals of the earlier state.
            loop_.ledger.verifications.clear();
            loop_.ledger.review_passed = false;
            let _ = ctx
                .goal_loops_repo
                .set_ledger(&loop_id, &loop_.ledger)
                .await;
            match ctx.goal_loops_repo.bump_iterations_started(&loop_id).await {
                Ok(idx) => {
                    ctx.goal_loops_repo
                        .add_iteration(&loop_id, &ws, idx, &context_in, &loop_.config.executors)
                        .await
                }
                Err(e) => Err(e),
            }
        };
        let iter = match created {
            Ok(it) => it,
            Err(e) => {
                finalize(
                    &ctx,
                    &loop_id,
                    GoalLoopStatus::Failed,
                    None,
                    Some(&e.to_string()),
                )
                .await;
                deregister(&ctx, &loop_id, &handle);
                return;
            }
        };
        let idx = iter.idx;
        let wt = loop_
            .worktree_path
            .clone()
            .unwrap_or_else(|| loop_.repo_path.clone());

        // ---- PLAN ----
        set_phase(&ctx, &ws, &loop_, GoalLoopPhase::Planning, idx).await;
        let plan = if verification_only {
            iter.plan.clone()
        } else {
            match run_role(
                &ctx,
                &loop_,
                &iter.id,
                "Planner",
                &loop_.config.planner,
                planner_prompt(&loop_, &context_in, prior_eval.as_ref(), idx),
                &wt,
                limits.per_phase_timeout_secs,
            )
            .await
            {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!(loop = %loop_id, "goal-loop: planner failed: {e}");
                    String::from("(planner unavailable — executors should work directly toward the acceptance criteria)")
                }
            }
        };
        let _ = ctx.goal_loops_repo.set_iter_plan(&iter.id, &plan).await;
        if stop_requested(&handle) {
            continue;
        }

        // ---- EXECUTE (sequential) ----
        set_phase(&ctx, &ws, &loop_, GoalLoopPhase::Executing, idx).await;
        let _ = ctx
            .goal_loops_repo
            .update_iteration_status(&iter.id, "executing", false)
            .await;
        let mut exec_summaries: Vec<String> = if verification_only {
            iter.agents
                .iter()
                .filter_map(|a| a.output_summary.clone())
                .collect()
        } else {
            Vec::new()
        };
        for (i, exec) in loop_.config.executors.iter().enumerate() {
            if verification_only {
                break;
            }
            if stop_requested(&handle) {
                break;
            }
            let out_path = executor_out_path(&loop_id, idx, i);
            let _ = std::fs::remove_file(&out_path);
            let prompt = executor_prompt(
                &loop_,
                exec,
                &plan,
                &context_in,
                &out_path.to_string_lossy(),
            );
            let _ = std::fs::write(prompt_path(&loop_id, idx, i), &prompt);
            let summary = run_executor(
                &ctx,
                &loop_,
                &iter.id,
                idx,
                i,
                exec,
                &wt,
                &prompt,
                Some(&handle.interrupted),
            )
            .await;
            exec_summaries.push(format!("{}: {}", exec.name, summary));
        }
        if stop_requested(&handle) {
            continue;
        }

        // ---- EVALUATE ----
        set_phase(&ctx, &ws, &loop_, GoalLoopPhase::Evaluating, idx).await;
        let _ = ctx
            .goal_loops_repo
            .update_iteration_status(&iter.id, "evaluating", false)
            .await;
        let (mut eval, verify_caps) = evaluate(
            &ctx,
            &loop_,
            &iter.id,
            &wt,
            &exec_summaries,
            limits.per_phase_timeout_secs,
            &handle,
        )
        .await;
        if stop_requested(&handle) {
            continue;
        }
        let mut ledger = loop_.ledger.clone();
        let stagnant = crate::goal_loop_policy::record_progress(&mut ledger, &eval);
        if stagnant && eval.verdict != "blocked" {
            eval.verdict = "blocked".into();
            eval.feedback = "Two iterations produced the same unmet criteria and evidence. Choose a new approach before resuming.".into();
        }
        if eval.verdict == "blocked" && !eval.feedback.starts_with("Work is ready for human") {
            ledger.questions.push(otto_core::domain::GoalQuestion {
                id: otto_core::new_id(),
                question: eval.feedback.clone(),
                answer: None,
                answered_by: None,
                answered_at: None,
            });
        }
        ledger.next_action = eval.feedback.clone();
        let _ = ctx.goal_loops_repo.set_ledger(&loop_id, &ledger).await;
        let _ = ctx
            .goal_loops_repo
            .set_iter_evaluation(&iter.id, &eval)
            .await;
        let progress = eval.progress_pct.min(100);
        let _ = ctx
            .goal_loops_repo
            .update_runtime(
                &loop_id,
                GoalLoopStatus::Running,
                GoalLoopPhase::Evaluating,
                idx,
                progress,
            )
            .await;
        emit(
            &ctx,
            &ws,
            &loop_id,
            GoalLoopStatus::Running,
            GoalLoopPhase::Evaluating,
            idx,
            progress,
        );

        // ---- DIGEST ----
        set_phase(&ctx, &ws, &loop_, GoalLoopPhase::Digesting, idx).await;
        let _ = ctx
            .goal_loops_repo
            .update_iteration_status(&iter.id, "digesting", false)
            .await;
        let digest = run_role(
            &ctx,
            &loop_,
            &iter.id,
            "Digester",
            &loop_.config.digester,
            digester_prompt(&context_in, &plan, &exec_summaries, &eval),
            &wt,
            limits.per_phase_timeout_secs,
        )
        .await
        .unwrap_or(context_in);
        let _ = ctx
            .goal_loops_repo
            .set_iter_context_out(&iter.id, &digest)
            .await;
        let _ = ctx
            .goal_loops_repo
            .set_context_digest(&loop_id, &digest)
            .await;
        let _ = ctx
            .goal_loops_repo
            .update_iteration_status(&iter.id, "done", true)
            .await;
        if stop_requested(&handle) {
            continue;
        }
        prior_eval = Some(eval.clone());

        // ---- DECISION ----
        let all_met = !eval.criteria.is_empty() && eval.criteria.iter().all(|c| c.met);
        if eval.verdict == "achieved" && all_met {
            // Package this iteration's evidence into a proof pack (diff, verify
            // commands, self-review, criteria summary) and read back its derived
            // status. Always attached — this is the "no done without evidence" record.
            let proof_status =
                assemble_goal_loop_proof(&ctx, &loop_, &eval, &verify_caps, &wt).await;

            // Teeth (opt-in via OTTO_PROOF_REQUIRE_GOAL_LOOP): refuse to finalize
            // "achieved" without a passing machine-checked test, including the
            // final iteration. Missing proof remains unmet when limits expire.
            if crate::goal_loop_policy::missing_required_proof(
                require_goal_loop_proof(),
                proof_status,
            ) {
                if let Some(pe) = prior_eval.as_mut() {
                    pe.feedback = format!(
                        "{}\n\n[Otto proof] You reported the goal done but did not produce a passing, \
                         machine-checked verification (test) command. Add one to an acceptance \
                         criterion (verify_kind=command) or run your tests, then report done.",
                        pe.feedback
                    );
                }
                let _ = ctx.events.send(Event::Notice {
                    level: "warn".into(),
                    title: "Goal loop: proof required".into(),
                    body:
                        "Reported done without passing test evidence — continuing to gather proof."
                            .into(),
                });
                continue;
            }

            if stop_requested(&handle) {
                continue;
            }
            let proof_note = match proof_status {
                Some(otto_core::proof::ProofStatus::Passed) => " Proof: passed.",
                Some(otto_core::proof::ProofStatus::Partial) => {
                    " Proof: partial (no machine-checked test)."
                }
                _ => "",
            };
            if loop_.config.require_review {
                let review = run_role(&ctx, &loop_, &iter.id, "Completion reviewer", &loop_.config.evaluator,
                    format!("{}\nIndependently review the final work against the goal. Report concrete unresolved defects. Return JSON {{\"approved\":true|false,\"findings\":[string]}}. Approve only when no material issue remains.", goal_context(&loop_)),
                    &wt, limits.per_phase_timeout_secs).await;
                let result = review
                    .as_ref()
                    .ok()
                    .and_then(|text| crate::goal_loop_parse::find_json_object(text))
                    .and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok());
                ledger.review_passed = result.as_ref().is_some_and(|v| {
                    v["approved"] == true && v["findings"].as_array().is_some_and(Vec::is_empty)
                });
                ledger.review_summary = review.unwrap_or_else(|e| e.to_string());
                let _ = ctx.goal_loops_repo.set_ledger(&loop_id, &ledger).await;
                if stop_requested(&handle) {
                    continue;
                }
                if !ledger.review_passed {
                    block(&ctx, &loop_id, "Completion review has unresolved findings. Inspect the review evidence before resuming.").await;
                    deregister(&ctx, &loop_id, &handle);
                    return;
                }
            }
            let summary = format!(
                "Goal achieved in {idx} iteration(s). {}{}",
                eval.rationale, proof_note
            );
            finalize(
                &ctx,
                &loop_id,
                GoalLoopStatus::Succeeded,
                Some(&summary),
                None,
            )
            .await;
            deregister(&ctx, &loop_id, &handle);
            return;
        }
        if eval.verdict == "blocked" {
            let summary = format!("Blocked — needs input. {}", eval.feedback);
            // Bank the active window and stop ticking; resume requires user action.
            block(&ctx, &loop_id, &summary).await;
            emit(
                &ctx,
                &ws,
                &loop_id,
                GoalLoopStatus::Blocked,
                GoalLoopPhase::Done,
                idx,
                progress,
            );
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
        .update_runtime(
            &loop_.id,
            GoalLoopStatus::Running,
            phase,
            idx,
            loop_.progress_pct,
        )
        .await;
    emit(
        ctx,
        ws,
        &loop_.id,
        GoalLoopStatus::Running,
        phase,
        idx,
        loop_.progress_pct,
    );
}

/// Bank the final active window, mark the loop terminal, preserve working files
/// and release any lingering managed sessions.
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
    let _ = ctx
        .goal_loops_repo
        .finalize(loop_id, status, summary, error)
        .await;
    // Keep the worktree even on failure/stop/success: commits are optional and
    // the retained branch cannot preserve dirty or untracked work.
    cleanup_executor_sessions(ctx, &loop_.workspace_id, loop_id).await;
    emit(
        ctx,
        &loop_.workspace_id,
        loop_id,
        status,
        GoalLoopPhase::Done,
        loop_.current_iteration,
        loop_.progress_pct,
    );
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

// --- Roles (managed, timeout-wrapped) -------------------------------------

#[allow(clippy::too_many_arguments)]
async fn run_role(
    ctx: &ServerCtx,
    loop_: &GoalLoop,
    iter_id: &Id,
    name: &str,
    role: &GoalLoopRoleCfg,
    prompt: String,
    cwd: &str,
    per_phase_secs: u64,
) -> Result<String> {
    crate::goal_loop_roles::run(ctx, loop_, iter_id, name, role, prompt, cwd, per_phase_secs).await
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
    if !loop_.config.source_links.is_empty() {
        s.push_str(&format!(
            "\n## Approved sources\n{}\n",
            loop_.config.source_links.join("\n")
        ));
    }
    if !loop_.config.skills.is_empty() {
        s.push_str(&format!("\n## Selected skills\nUse the relevant skills; report missing skills as blockers.\n{}\n", loop_.config.skills.join("\n")));
    }
    s.push_str(&format!(
        "\n## Persisted decisions and next action\n{}\n",
        serde_json::to_string(&loop_.ledger).unwrap_or_default()
    ));
    s
}

fn planner_prompt(
    loop_: &GoalLoop,
    context_in: &str,
    prior: Option<&GoalLoopEvaluation>,
    idx: u32,
) -> String {
    let mut s = format!(
        "{}\n\n{}\n\n",
        loop_.config.planner.prompt,
        goal_context(loop_)
    );
    s.push_str(&format!(
        "This is iteration {idx} of at most {}.\n",
        loop_.limits.max_iterations
    ));
    if !context_in.is_empty() {
        s.push_str(&format!(
            "\n## Context so far (auxiliary memory)\n{context_in}\n"
        ));
    }
    if let Some(p) = prior {
        let unmet: Vec<&str> = p
            .criteria
            .iter()
            .filter(|c| !c.met)
            .map(|c| c.id.as_str())
            .collect();
        s.push_str(&format!(
            "\n## Previous evaluation\nprogress: {}% · verdict: {}\nunmet criteria: {}\nfeedback: {}\n",
            p.progress_pct, p.verdict, unmet.join(", "), p.feedback
        ));
    }
    s.push_str("\nProduce a concrete, minimal plan for THIS iteration to close the unmet criteria. Reply with the plan as plain text.");
    s
}

fn executor_prompt(
    loop_: &GoalLoop,
    exec: &GoalLoopAgentCfg,
    plan: &str,
    context_in: &str,
    out_path: &str,
) -> String {
    let mut s = format!(
        "You are \"{}\", an executor on an autonomous goal loop working in this isolated directory.\n\n{}\n\n",
        exec.name,
        goal_context(loop_)
    );
    s.push_str(if loop_.config.allow_commits {
        "## Execution policy\nLocal commits are allowed. Never push, publish, open a PR or send messages without a separately authorized user action.\n\n"
    } else {
        "## Execution policy\nDo not stage or commit changes. Leave tracked and untracked work in this directory for review. Never push, publish, open a PR or send messages.\n\n"
    });
    if loop_.config.mode == "research" {
        s.push_str("This is a research goal. Produce a findings.md report with cited evidence; do not change repository code or create git history.\n");
    }
    if !exec.prompt_extra.trim().is_empty() {
        s.push_str(&format!("## Your focus\n{}\n\n", exec.prompt_extra));
    }
    if !context_in.is_empty() {
        s.push_str(&format!("## Context so far\n{context_in}\n\n"));
    }
    s.push_str(&format!("## This iteration's plan\n{plan}\n\n"));
    s.push_str(&format!(
        "---\nDo the work toward the acceptance criteria, following the execution policy above. \
         Finally, write a JSON object describing what you did to this absolute file path, overwriting any existing content:\n\n{out_path}\n\n\
         Schema: {{\"summary\": string, \"changed_files\": [string], \"notes\": string, \"blockers\": [string]}}.\n\
         Write ONLY the JSON object to that file (no prose, no markdown fence). Writing the file is the LAST thing you do."
    ));
    s
}

fn digester_prompt(
    context_in: &str,
    plan: &str,
    exec_summaries: &[String],
    eval: &GoalLoopEvaluation,
) -> String {
    format!(
        "Compress the running context plus this iteration into a concise summary (a few hundred words max): what is done, current state, what remains, key decisions, blockers. The git worktree is the source of truth; this is auxiliary memory.\n\n## Prior context\n{context_in}\n\n## This iteration plan\n{plan}\n\n## Executor results\n{}\n\n## Evaluation\nprogress {}% · verdict {} · feedback: {}\n\nReply with ONLY the updated summary text.",
        exec_summaries.join("\n"),
        eval.progress_pct,
        eval.verdict,
        eval.feedback,
    )
}

/// Run the managed evaluator in the worktree, then GROUND-TRUTH every
/// command-verify criterion by running its command (exit 0 = met), overriding the
/// model. An "achieved" verdict with any unmet criterion is coerced to "continue".
async fn evaluate(
    ctx: &ServerCtx,
    loop_: &GoalLoop,
    iter_id: &Id,
    wt: &str,
    exec_summaries: &[String],
    per_phase_secs: u64,
    handle: &LoopHandle,
) -> (GoalLoopEvaluation, Vec<VerifyCapture>) {
    let prompt = format!(
        "{}\n\n{}\n\n## Executor results this iteration\n{}\n\nReply with ONLY a JSON object: {{\"progress_pct\": 0-100, \"verdict\": \"achieved|continue|blocked\", \"criteria\": [{{\"id\": string, \"met\": bool, \"evidence\": string}}], \"feedback\": string, \"rationale\": string}}. Include one entry per acceptance criterion. Provide concrete evidence for every met=true.",
        loop_.config.evaluator.prompt,
        goal_context(loop_),
        exec_summaries.join("\n"),
    );
    let mut eval = match run_role(
        ctx,
        loop_,
        iter_id,
        "Evaluator",
        &loop_.config.evaluator,
        prompt,
        wt,
        per_phase_secs,
    )
    .await
    {
        Ok(text) => parse_evaluation(&text)
            .unwrap_or_else(|| fallback_eval("evaluator produced no parseable verdict")),
        Err(e) => fallback_eval(&format!("evaluator turn failed: {e}")),
    };

    // Ground-truth command criteria by exit code, capturing the output so the
    // proof pack can keep each as a `command` test-evidence artifact.
    let mut captures: Vec<VerifyCapture> = Vec::new();
    for crit in &loop_.definition.acceptance_criteria {
        if stop_requested(handle) {
            break;
        }
        if crit.verify_kind == "command" {
            if let Some(cmd) = &crit.verify_cmd {
                let run =
                    crate::goal_loop_commands::run(wt, cmd, per_phase_secs, &handle.interrupted)
                        .await;
                set_criterion(
                    &mut eval,
                    &crit.id,
                    run.success,
                    if run.success {
                        "verify command exited 0"
                    } else {
                        "verify command failed"
                    },
                );
                captures.push((crit.id.clone(), cmd.clone(), run));
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

    crate::goal_loop_policy::reconcile_human(&loop_.definition, &loop_.ledger, &mut eval);

    // Reconcile an over-optimistic verdict against ground truth.
    let all_met = !eval.criteria.is_empty() && eval.criteria.iter().all(|c| c.met);
    if eval.verdict == "achieved" && !all_met {
        eval.verdict = "continue".into();
        eval.rationale = format!(
            "{} (coerced: verdict was 'achieved' but not all criteria met)",
            eval.rationale
        );
    }
    (eval, captures)
}

/// One captured verify-command run: (criterion id, command, run outcome).
type VerifyCapture = (String, String, crate::proof::CmdRun);

/// Whether to ENFORCE machine-checked test evidence before a goal loop may
/// finalize "achieved". Opt-in (default off) — evidence is always packaged
/// regardless; this only controls the hard block, which is bounded by the
/// iteration cap when on.
fn require_goal_loop_proof() -> bool {
    std::env::var("OTTO_PROOF_REQUIRE_GOAL_LOOP")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Assemble the proof pack for a successful goal-loop iteration and return its
/// recomputed status. Each verify command becomes a `command` test artifact, the
/// worktree diff a `diff` artifact, the evaluator's feedback a `self_review`, and
/// the acceptance criteria a `review` summary. Best-effort.
async fn assemble_goal_loop_proof(
    ctx: &ServerCtx,
    loop_: &GoalLoop,
    eval: &GoalLoopEvaluation,
    verify_caps: &[VerifyCapture],
    wt: &str,
) -> Option<otto_core::proof::ProofStatus> {
    use otto_core::proof::{ProofArtifactKind as K, ProofArtifactStatus as S, WorkItemKind};
    let pack = crate::proof::gate(
        ctx,
        WorkItemKind::GoalLoop,
        &loop_.id,
        &loop_.workspace_id,
        &loop_.definition.title,
        &loop_.created_by,
    )
    .await
    .ok()?;

    // Verify-command test artifacts (machine-checked evidence).
    for (crit_id, cmd, run) in verify_caps {
        let status = if run.success { S::Passed } else { S::Failed };
        let meta = serde_json::json!({
            "test_kind": "test", "criterion_id": crit_id,
            "exit_code": run.exit_code, "duration_ms": run.duration_ms,
        });
        let _ = crate::proof::upsert_content_artifact(
            ctx,
            &pack,
            K::Command,
            cmd,
            &run.output,
            status,
            meta,
            "otto",
        )
        .await;
    }

    // Working-tree diff vs the loop's base commit.
    if loop_.config.mode == "research" {
        if let Ok(report) =
            tokio::fs::read_to_string(std::path::Path::new(wt).join("findings.md")).await
        {
            let _ = crate::proof::upsert_content_artifact(
                ctx,
                &pack,
                K::SelfReview,
                "Research findings",
                &report,
                S::Info,
                serde_json::json!({}),
                "otto",
            )
            .await;
        }
    } else if let Ok(diff) =
        crate::goal_loop_workspace::capture_work(wt, loop_.base_commit.as_deref()).await
    {
        let parsed = otto_git::parse::parse_diff(&diff);
        let meta = serde_json::json!({
            "files_changed": parsed.files.len(),
            "additions": parsed.files.iter().filter_map(|f| f.added).sum::<u32>(),
            "deletions": parsed.files.iter().filter_map(|f| f.deleted).sum::<u32>(),
            "includes_uncommitted": true,
        });
        let _ = crate::proof::upsert_content_artifact(
            ctx,
            &pack,
            K::Diff,
            "Working tree diff",
            &diff,
            S::Info,
            meta,
            "otto",
        )
        .await;
    }

    // Self-review from the evaluator's feedback + rationale + per-criterion evidence.
    let crit_lines: Vec<String> = eval
        .criteria
        .iter()
        .map(|c| {
            format!(
                "- [{}] {}: {}",
                if c.met { "x" } else { " " },
                c.id,
                c.evidence
            )
        })
        .collect();
    let selfreview = format!(
        "Feedback: {}\n\nRationale: {}\n\nCriteria:\n{}",
        eval.feedback,
        eval.rationale,
        crit_lines.join("\n")
    );
    let _ = crate::proof::upsert_content_artifact(
        ctx,
        &pack,
        K::SelfReview,
        "Goal-loop self-review",
        &selfreview,
        S::Info,
        serde_json::json!({}),
        "otto",
    )
    .await;

    // Acceptance-criteria review summary.
    let unmet = eval.criteria.iter().filter(|c| !c.met).count();
    let met = eval.criteria.len().saturating_sub(unmet);
    let rev_status = if unmet == 0 { S::Passed } else { S::Failed };
    let rev = format!("{met} of {} acceptance criteria met.", eval.criteria.len());
    let _ = crate::proof::upsert_content_artifact(
        ctx,
        &pack,
        K::Review,
        "Acceptance criteria",
        &rev,
        rev_status,
        serde_json::json!({"met": met, "unmet": unmet}),
        "otto",
    )
    .await;

    crate::proof::recompute_and_emit(ctx, &pack.id)
        .await
        .ok()
        .map(|p| p.status)
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

// --- Executors (live sessions) ---------------------------------------------

fn tmp_dir() -> PathBuf {
    PathBuf::from(std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".to_string()))
}

/// Loop ids are daemon-generated ULIDs, but they also arrive as HTTP path
/// params (retry), so re-validate before the tmp-dir join — a hostile id fails
/// closed to a name that never exists instead of escaping the directory.
fn safe_loop_id(loop_id: &str) -> &str {
    otto_core::paths::safe_component(loop_id).unwrap_or("invalid")
}

fn executor_out_path(loop_id: &str, idx: u32, exec: usize) -> PathBuf {
    tmp_dir().join(format!(
        "otto-goalloop-{}-{idx}-{exec}.json",
        safe_loop_id(loop_id)
    ))
}

fn prompt_path(loop_id: &str, idx: u32, exec: usize) -> PathBuf {
    tmp_dir().join(format!(
        "otto-goalloop-{}-{idx}-{exec}.prompt",
        safe_loop_id(loop_id)
    ))
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
            run_executor_attempt(
                ctx, loop_, iter_id, idx, exec_index, exec, cwd, prompt, &out_path, timeout, cancel,
            )
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
        persist_agent(
            ctx,
            iter_id,
            exec_index,
            exec,
            "error",
            &note,
            outcome.session_id.clone(),
            None,
        )
        .await;
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
    cancel: Option<&Arc<AtomicBool>>,
) -> RunOutcome {
    // A blank executor provider resolves to the workspace/global default agent
    // (not a bare hardcoded "claude"), honoring Settings → Providers.
    let resolved_provider;
    let provider = if exec.provider.trim().is_empty() {
        let global = otto_state::SettingsRepo::new(ctx.pool.clone())
            .get("default_provider")
            .await
            .ok()
            .flatten();
        let ws_default = ctx
            .workspaces
            .get(&loop_.workspace_id)
            .await
            .ok()
            .map(|w| otto_core::provider::workspace_default(&w.settings).to_string())
            .unwrap_or_default();
        resolved_provider = otto_core::provider::resolve_provider(&[
            ws_default.as_str(),
            otto_core::provider::global_default(global.as_ref()),
        ]);
        resolved_provider.as_str()
    } else {
        exec.provider.trim()
    };
    let mut meta = serde_json::json!({
        "source": "goal_loop",
        "loop_id": loop_.id,
        "iter_idx": idx,
        "agent_index": exec_index,
    });
    // Executor model override → `--model` at spawn (SessionManager::model_args
    // reads `meta.model`; providers without the flag ignore it).
    if !exec.model.trim().is_empty() {
        meta["model"] = serde_json::Value::String(exec.model.trim().to_string());
    }
    let req = CreateSessionReq {
        kind: SessionKind::Agent,
        provider: Some(provider.to_string()),
        title: Some(format!("Loop: {} (iter {idx})", exec.name)),
        cwd: Some(cwd.to_string()),
        connection_id: None,
        model: None,
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
    persist_agent(
        ctx,
        iter_id,
        exec_index,
        exec,
        "running",
        "",
        Some(sid.clone()),
        None,
    )
    .await;

    if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
        let _ = ctx.manager.kill_session(&sid).await;
        return RunOutcome::failed(Some(sid.clone()), FailReason::Stopped);
    }
    let work = async {
        if wait_for_tui(&ctx.manager, &sid).await {
            if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
                let _ = ctx.manager.kill_session(&sid).await;
                return RunOutcome::failed(Some(sid.clone()), FailReason::Stopped);
            }
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
            |_| false,
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
                        WatchStatus::Resumed => {
                            ("running", String::new(), GoalLoopPhase::Executing)
                        }
                    };
                    persist_agent(
                        &ctx,
                        &iter_id,
                        exec_index,
                        &exec,
                        status,
                        &note,
                        Some(sid),
                        None,
                    )
                    .await;
                    let _ = ctx.goal_loops_repo.set_phase(&loop_id, phase).await;
                    emit(
                        &ctx,
                        &ws,
                        &loop_id,
                        GoalLoopStatus::Running,
                        phase,
                        idx,
                        progress,
                    );
                }
            },
        )
        .await
    };
    tokio::select! {
        biased;
        _ = async {
            loop {
                if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) { break; }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        } => {
            let _ = ctx.manager.kill_session(&sid).await;
            RunOutcome::failed(Some(sid.clone()), FailReason::Stopped)
        }
        outcome = work => outcome,
    }
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
    let _ = ctx
        .goal_loops_repo
        .set_iter_agent_at(iter_id, index, &st)
        .await;
}

fn executor_error_note(reason: Option<FailReason>) -> String {
    match reason {
        Some(FailReason::Stuck) => "stuck — no output for ~3m",
        Some(FailReason::Timeout) => "timed out (per-phase grace elapsed)",
        Some(FailReason::Exited) => "session exited before writing its result",
        Some(FailReason::SessionGone) => "session is no longer live",
        Some(FailReason::CreateFailed) => "could not start",
        Some(FailReason::Stopped) => "stopped",
        Some(FailReason::Superseded) => "skipped — superseded",
        None => "unknown error",
    }
    .to_string()
}

#[cfg(test)]
mod goal_loop_tests {
    use super::*;
    #[tokio::test]
    async fn lifecycle_operations_serialize_retry_approval_and_resume() {
        let registry = new_registry();
        let retry_lock = registry.operation("g");
        let retry = retry_lock.lock().await;
        let approval_lock = registry.operation("g");
        assert!(approval_lock.try_lock().is_err());
        let other = registry.operation("other");
        assert!(other.try_lock().is_ok());
        drop(retry);
        assert!(approval_lock.try_lock().is_ok());
    }

    #[test]
    fn executor_prompt_requires_explicit_commit_permission() {
        let mut goal: GoalLoop = serde_json::from_value(serde_json::json!({
            "id":"g", "workspace_id":"w", "name":"Goal", "repo_path":"/tmp/isolated",
            "definition":{"title":"Goal","acceptance_criteria":[]},
            "config":otto_core::domain::GoalLoopConfig::default(),
            "limits":otto_core::domain::GoalLoopLimits::default(),
            "status":"draft", "phase":"done", "iterations_started":0, "current_iteration":0,
            "progress_pct":0, "elapsed_secs":0, "cost_usd":0, "created_by":"u",
            "created_at":Utc::now(), "updated_at":Utc::now()
        }))
        .unwrap();
        let prompt = executor_prompt(
            &goal,
            &goal.config.executors[0],
            "plan",
            "",
            "/tmp/result.json",
        );
        assert!(prompt.contains("Do not stage or commit"));
        assert!(!prompt.contains("git add -A"));
        goal.config.allow_commits = true;
        let prompt = executor_prompt(
            &goal,
            &goal.config.executors[0],
            "plan",
            "",
            "/tmp/result.json",
        );
        assert!(prompt.contains("Local commits are allowed"));
        assert!(!prompt.contains("git add -A"));
    }
}
