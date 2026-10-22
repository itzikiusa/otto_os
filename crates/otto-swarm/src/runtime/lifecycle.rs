//! Swarm lifecycle transitions — activate, pause, abort — in ONE place.
//!
//! "Make a swarm active" is a fixed sequence: take the per-swarm operation
//! guard, apply the entry point's gate, write the status, clear the in-memory
//! pause, start the coordinator and broadcast the status. It used to be written
//! out by the start and resume routes, the channel launcher and Product
//! discovery, and the copies drifted: a channel message flipped an ABORTED or
//! budget-paused swarm back to active, and discovery skipped the guard and
//! could interleave with an abort (a swarm marked "active" whose sessions had
//! just been killed). Every entry point now calls [`activate`].

use otto_core::{Error, Id, Result};
use otto_state::Swarm;

use crate::runtime::engine::{
    emit_status, operation_guard, set_paused, start_coordinator, stop_coordinator,
    stop_runs_for_pause, swarm_session_ids,
};
use crate::runtime::host::SwarmRt;
use crate::runtime::run;

/// Which gate an activation must pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivatePolicy {
    /// The start route (and Product discovery): the workspace spend cap.
    UserStart,
    /// The resume route: the workspace spend cap (a budget pause stays shut).
    Resume,
    /// A channel/webhook launch: refuses an aborted swarm, an auto-paused one
    /// (budget or limit reason) and the workspace spend cap. Refusal is
    /// `Error::Conflict(<why>)`, ready to post back to the channel.
    Channel,
}

/// Make swarm `sid` (of workspace `ws`) active under its operation guard,
/// after `policy`'s gate. Returns the stored swarm.
pub async fn activate(ctx: &SwarmRt, ws: &Id, sid: &Id, policy: ActivatePolicy) -> Result<Swarm> {
    let _operation = operation_guard(sid).await;
    match policy {
        ActivatePolicy::UserStart | ActivatePolicy::Resume => {
            // Point-of-action budget gate (A2), before the Coordinator starts
            // scheduling runs. On resume a pause may have come from a
            // BudgetExceeded event: still over cap, it stays paused.
            if let Some(reason) = ctx.budget_blocked(ws).await {
                let what = if policy == ActivatePolicy::Resume {
                    "swarm resume blocked"
                } else {
                    "swarm blocked"
                };
                return Err(Error::Invalid(format!(
                    "Budget exceeded — {what}: {}",
                    reason.unwrap_or_else(|| "cap reached".to_string())
                )));
            }
        }
        ActivatePolicy::Channel => {
            if let Some(why) = crate::runtime::channels::channel_start_refused(ctx, sid, ws).await {
                return Err(Error::Conflict(why));
            }
        }
    }
    let repo = ctx.swarm_repo();
    repo.set_swarm_status(sid, "active").await?;
    set_paused(ctx, sid, false);
    start_coordinator(ctx.clone(), sid.clone());
    emit_status(ctx, ws, sid, "active");
    repo.get_swarm(sid).await
}

/// Pause swarm `sid`: status, in-memory pause, verifiers stopped, in-flight
/// turns ended (their tasks re-queue for the resume) and sessions suspended to
/// free RAM (resume-friendly).
pub async fn pause(ctx: &SwarmRt, ws: &Id, sid: &Id) -> Result<Swarm> {
    let _operation = operation_guard(sid).await;
    ctx.swarm_repo().set_swarm_status(sid, "paused").await?;
    set_paused(ctx, sid, true);
    crate::runtime::verify::stop_swarm(ctx, sid).await;
    // In-flight turns end first so the retry loop can't respawn them.
    stop_runs_for_pause(ctx, sid).await;
    for s in swarm_session_ids(ctx, ws, sid).await {
        let _ = ctx.manager().suspend(&s).await;
    }
    emit_status(ctx, ws, sid, "paused");
    ctx.swarm_repo().get_swarm(sid).await
}

/// Abort swarm `sid`: status, coordinator stopped, verifiers stopped,
/// in-flight runs cancelled and marked stopped, sessions killed.
pub async fn abort(ctx: &SwarmRt, ws: &Id, sid: &Id) -> Result<Swarm> {
    let _operation = operation_guard(sid).await;
    ctx.swarm_repo().set_swarm_status(sid, "aborted").await?;
    stop_coordinator(ctx, sid);
    // Stop any in-flight verification controllers (own cancel + kill
    // verify/fix sessions, short-circuiting run_swarm_agent retries; review B3).
    crate::runtime::verify::stop_swarm(ctx, sid).await;
    let stopped = ctx.swarm_repo().stop_active_runs(sid).await?;
    for rid in &stopped {
        run::signal_cancel(ctx.swarm_run_cancels(), rid);
    }
    for s in swarm_session_ids(ctx, ws, sid).await {
        let _ = ctx.manager().kill_session(&s).await;
    }
    emit_status(ctx, ws, sid, "aborted");
    ctx.swarm_repo().get_swarm(sid).await
}
