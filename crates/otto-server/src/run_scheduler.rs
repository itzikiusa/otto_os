//! Run with Otto — the boot reaper + supervisor tick.
//!
//! On startup: runs caught mid live-work (`executing`/`reviewing`) are **failed**
//! (their agent PTYs / review agents are gone — mirrors goal-loop `fail_running`),
//! and runs in short, idempotent stages are re-driven. A 30-second tick then
//! re-drives any still-active resumable run (the per-run in-flight guard makes
//! that a no-op for runs already advancing). See design §20.8.

use std::time::Duration;

use otto_core::event::Event;
use otto_core::run::RunStatus;
use otto_state::runs::NewRunEvent;

use crate::cancel_signal::CancelSignal;
use crate::run_engine;
use crate::state::ServerCtx;

const TICK: Duration = Duration::from_secs(30);

pub fn spawn(ctx: ServerCtx) -> CancelSignal {
    let stop = CancelSignal::new();
    tokio::spawn(run_until_stopped(ctx, stop.clone()));
    stop
}

async fn run_until_stopped(ctx: ServerCtx, stop: CancelSignal) {
    loop {
        tokio::select! {
            biased;
            _ = stop.cancelled() => return,
            _ = redrive_resumable(&ctx, &stop) => {},
        }
        if stop.sleep(TICK).await {
            return;
        }
    }
}

/// Settle the previous daemon life before any new work can be admitted.
pub(crate) async fn recover_interrupted(ctx: &ServerCtx) -> otto_core::Result<()> {
    let interrupted = ctx.runs.list_interrupted().await?;
    for run in interrupted {
        if !ctx
            .runs
            .set_error(&run.id, "interrupted by daemon restart")
            .await?
        {
            continue;
        }
        let _ = ctx
            .runs
            .add_event(NewRunEvent {
                run_id: run.id.clone(),
                workspace_id: run.workspace_id.clone(),
                kind: "stage_error".to_string(),
                status: Some(RunStatus::Failed.as_str().to_string()),
                message: "Interrupted by a daemon restart (live work was lost). \
                              The branch's commits are preserved; relaunch to continue."
                    .to_string(),
                detail: None,
            })
            .await;
        let _ = ctx.events.send(Event::OttoRunUpdated {
            workspace_id: run.workspace_id.clone(),
            run_id: run.id.clone(),
            status: RunStatus::Failed.as_str().to_string(),
        });
        if let Ok(fresh) = ctx.runs.get(&run.id).await {
            run_engine::project(ctx, &fresh).await;
        }
    }
    Ok(())
}

async fn redrive_resumable(ctx: &ServerCtx, stop: &CancelSignal) {
    if let Ok(runs) = ctx.runs.list_resumable().await {
        for run in runs {
            if stop.is_cancelled() {
                return;
            }
            let c = ctx.clone();
            let id = run.id.clone();
            let stop = stop.clone();
            tokio::spawn(async move {
                // A spawned but not yet polled task must not start a stage
                // after shutdown. Work already advancing keeps its own cleanup.
                if !stop.is_cancelled() {
                    run_engine::advance(&c, id).await;
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stopping_during_run_scan_finishes_before_database_is_released() {
        let dir = tempfile::tempdir().unwrap();
        let pool = crate::test_support::mem_pool().await;
        let ctx = ServerCtx::for_tests(&pool, dir.path()).await;
        let connection = pool.acquire().await.unwrap();
        let stop = CancelSignal::new();
        let running = run_until_stopped(ctx, stop.clone());
        tokio::pin!(running);
        tokio::select! {
            biased;
            _ = &mut running => panic!("supervisor returned before shutdown"),
            _ = tokio::time::sleep(Duration::from_millis(20)) => {},
        }
        stop.cancel();
        // The sole DB connection is still held: teardown cannot wait on a
        // scan, and no scan result can launch fresh work after cancellation.
        tokio::time::timeout(Duration::from_secs(1), &mut running)
            .await
            .expect("shutdown interrupts an in-flight read-only scan");
        drop(connection);
    }
}
