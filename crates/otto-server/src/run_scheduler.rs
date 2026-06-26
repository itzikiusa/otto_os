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

use crate::run_engine;
use crate::state::ServerCtx;

const TICK: Duration = Duration::from_secs(30);

pub fn spawn(ctx: ServerCtx) {
    tokio::spawn(async move {
        reap(&ctx).await;
        loop {
            redrive_resumable(&ctx).await;
            tokio::time::sleep(TICK).await;
        }
    });
}

/// Fail interrupted runs; re-drive resumable ones once on boot.
async fn reap(ctx: &ServerCtx) {
    if let Ok(interrupted) = ctx.runs.list_interrupted().await {
        for run in interrupted {
            let _ = ctx
                .runs
                .set_error(&run.id, "interrupted by daemon restart")
                .await;
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
    }
    redrive_resumable(ctx).await;
}

async fn redrive_resumable(ctx: &ServerCtx) {
    if let Ok(runs) = ctx.runs.list_resumable().await {
        for run in runs {
            let c = ctx.clone();
            let id = run.id.clone();
            tokio::spawn(async move {
                run_engine::advance(&c, id).await;
            });
        }
    }
}
