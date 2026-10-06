//! Phase 3 — settle what the previous daemon life left behind, BEFORE the
//! router serves (every step here is awaited) — plus the post-listen sweep
//! task that is spawned between them.

use std::path::PathBuf;

use otto_state::{ReviewsRepo, SkillEvalsRepo};

use super::BootPhases;
use crate::state::ServerCtx;

/// Restore sessions, fail orphaned reviews / skill evals, settle interrupted
/// API runs and recover workflow runs. Marks the `restore` boot phase.
pub async fn recover_before_serve(ctx: &ServerCtx, boot: &mut BootPhases) -> Result<(), String> {
    restore_sessions(ctx).await?;
    boot.mark("restore");
    fail_orphaned_reviews(ctx).await;
    fail_orphaned_skill_evals(ctx).await;
    // Do not replay API requests with uncertain external outcomes after a crash.
    otto_state::api_runs::ApiRunsRepo(ctx.pool.clone())
        .recover_interrupted()
        .await
        .map_err(|e| e.to_string())?;
    recover_workflow_runs(ctx).await;
    Ok(())
}

/// Restore sessions from the previous daemon run: resumable agent
/// sessions respawn, everything else becomes reconnectable.
async fn restore_sessions(ctx: &ServerCtx) -> Result<(), String> {
    let ws_paths: std::collections::HashMap<String, String> = ctx
        .workspaces
        .list_all()
        .await
        .map_err(|e| format!("list workspaces: {e}"))?
        .into_iter()
        .map(|w| (w.id, w.root_path))
        .collect();
    match ctx
        .manager
        .restore_all(&move |ws_id| ws_paths.get(ws_id.as_str()).cloned())
        .await
    {
        Ok(summary) => {
            tracing::info!(
                kept_running = summary.kept_running,
                suspended = summary.suspended,
                "session restore"
            );
            crate::transport::set_boot_restore(crate::transport::BootRestore {
                kept_running: summary.kept_running,
                suspended: summary.suspended,
            });
        }
        Err(e) => tracing::warn!("session restore: {e}"),
    }
    // Holders whose adoption hit a transient DB error: retry shortly.
    ctx.manager.spawn_deferred_adoption_retries();
    Ok(())
}

/// Fail any reviews orphaned by the previous process exit: a review's
/// background task dies with the process, so a row left `running` would
/// otherwise poll forever in the UI. Mark them error so they're re-runnable.
async fn fail_orphaned_reviews(ctx: &ServerCtx) {
    match ReviewsRepo::new(ctx.pool.clone())
        .fail_running("Interrupted by a daemon restart — re-run the review.")
        .await
    {
        Ok(n) if n > 0 => tracing::info!("review recovery: marked {n} orphaned review(s) as error"),
        Ok(_) => {}
        Err(e) => tracing::warn!("review recovery: {e}"),
    }
}

/// Same recovery for orphaned skill-evaluation runs.
async fn fail_orphaned_skill_evals(ctx: &ServerCtx) {
    match SkillEvalsRepo::new(ctx.pool.clone())
        .fail_running("Interrupted by a daemon restart — re-run the evaluation.")
        .await
    {
        Ok(n) if n > 0 => {
            tracing::info!("skill-eval recovery: marked {n} orphaned run(s) as error")
        }
        Ok(_) => {}
        Err(e) => tracing::warn!("skill-eval recovery: {e}"),
    }
}

/// Workflow recovery: runs a dead daemon left EXECUTING are RESUMED from
/// their persisted per-node progress (adopting finished steps, re-entering
/// at the interrupted one) unless the workflow opts out via
/// `on_restart = 'fail'`, the resume cap is hit, or the interrupted step
/// has external side effects (unknown outcome → failed with a pointer at
/// the manual retry-a-step flow). QUEUED runs (fresh `pending`, parked
/// behind the parallel-run gate) re-enqueue in order so the persistent run
/// queue survives the restart. Order matters: reconcile flips resumable
/// rows back to `pending` BEFORE the worktree sweep in
/// [`spawn_post_listen_work`], so their provisioned worktrees survive for the
/// resumed steps.
async fn recover_workflow_runs(ctx: &ServerCtx) {
    let (resumed, settled) = crate::workflow_engine::reconcile_interrupted_runs(ctx).await;
    if resumed > 0 || settled > 0 {
        tracing::info!(
            "workflow recovery: resumed {resumed} interrupted run(s), settled {settled}"
        );
    }
    let n = crate::workflow_engine::resume_queued_runs(ctx).await;
    if n > 0 {
        tracing::info!("workflow recovery: re-enqueued {n} queued run(s)");
    }
}

/// Work no route depends on runs AFTER the listener starts serving
/// (perf2/03 N7): plugin sidecars (a request before they are up gets the
/// proxy's "not running" answer, as during any plugin restart), the
/// binary's own blocking `housekeeping` sweeps (launchd jobs, dead data-dir
/// files — given the data dir, run on the blocking pool) and the workflow
/// worktree sweep (git subprocesses per run).
pub fn spawn_post_listen_work<F>(ctx: &ServerCtx, housekeeping: F)
where
    F: FnOnce(PathBuf) + Send + 'static,
{
    let ctx = ctx.clone();
    tokio::spawn(async move {
        let t = std::time::Instant::now();
        // Spawn enabled runtime plugins (sidecar processes Otto supervises
        // + proxies). Best-effort: per-plugin spawn failures are logged
        // inside, never fatal.
        ctx.plugins.start_enabled().await;
        let plugins_ms = t.elapsed().as_millis();
        let data_dir = ctx.data_dir.clone();
        let _ = tokio::task::spawn_blocking(move || housekeeping(data_dir)).await;
        // And leftover workflow run worktrees (+ safe otto-wf/<id> branch
        // cleanup) — finalize-time reaping can't run for a crashed daemon,
        // and pre-reap versions left one worktree per run in the user's
        // real repos. Runs the reconciler chose to resume (in
        // `recover_before_serve`, before this task was spawned) are
        // `pending` again and keep theirs.
        crate::workflow_engine::sweep_stale_run_worktrees(&ctx).await;
        // Runtime orphan sweep: a live run row whose driver is gone (a failed
        // terminal write, a panicked driver) is errored instead of blocking
        // its workflow's triggers until the next restart.
        crate::workflow_engine::start_orphan_run_sweep(&ctx);
        tracing::info!(
            "boot: post-listen work done in {} ms (plugins={plugins_ms})",
            t.elapsed().as_millis()
        );
    });
}

/// Goal loops: each loop's controller dies with the process, so a row left
/// running/paused/blocked is orphaned. Pause active loops, preserve blocked
/// decisions and all working files, and reap only execution resources.
pub async fn recover_goal_loops(ctx: &ServerCtx) {
    match ctx
        .goal_loops_repo
        .fail_running("Interrupted by a daemon restart — Resume to continue with preserved work.")
        .await
    {
        Ok(loops) => {
            if !loops.is_empty() {
                tracing::info!(
                    "goal-loop recovery: preserved {} interrupted loop(s)",
                    loops.len()
                );
            }
            for l in &loops {
                crate::goal_loop::cleanup_executor_sessions(ctx, &l.workspace_id, &l.id).await;
            }
        }
        Err(e) => tracing::warn!("goal-loop recovery: {e}"),
    }
}
