//! Holds (goal.md §2.3–§2.4): Pause, Resume and Extend, steering, a
//! person's criterion record, Cancel of a held run, and settling a held run
//! at the driver's end. The rules and SQL are otto-workflows' (`hold`,
//! `steering`, `check_run`) and otto-state's; a Cancel always wins (DR-106).
//! The goal routes call the `pub(crate)` entry points.

use super::super::*;
use otto_core::api::{ExtendReq, VerifyCriterionReq};
use otto_core::workflows::HoldKind;
use otto_workflows::hold as rules;
use otto_workflows::step_util::StepNote;

/// Pause a pending or running run: the hold is durable before anything unwinds.
pub(crate) async fn pause(ctx: &ServerCtx, run_id: &Id, by: &User) -> Result<WorkflowRun> {
    let repo = WorkflowsRepo::new(ctx.pool.clone());
    let hold = rules::pause_hold(&by.id, &by.display_name);
    let Some(rev) = repo.request_pause(run_id, &hold).await? else {
        return Err(otto_core::Error::Conflict("the run is not running".into()));
    };
    let run = repo.get_run(run_id).await?;
    emit_run_updated(
        ctx,
        &run.workspace_id,
        run_id,
        "canceled",
        None,
        rev,
        None,
        &[],
        false,
    );
    Ok(run)
}

/// Resume a held run, with new totals for the loop that ran out.
pub(crate) async fn resume(
    ctx: &ServerCtx,
    run_id: &Id,
    extend: Option<&ExtendReq>,
) -> Result<WorkflowRun> {
    if driver_alive(run_id) {
        let msg = "the run is still stopping — try again in a few seconds";
        return Err(otto_core::Error::Conflict(msg.into()));
    }
    let repo = WorkflowsRepo::new(ctx.pool.clone());
    let ext = extend.map_or((None, None), |e| (e.iterations, e.active_s));
    let r = rules::prepare_resume(&repo, run_id, ext).await?;
    // A long hold must not let retention delete the threads it continues.
    step_thread::retain_run(ctx, &r.nodes, false).await;
    let ws = ctx.workspaces.get(&r.workflow.workspace_id).await?;
    let input = r.run.input;
    spawn_run(
        ctx.clone(),
        ws,
        r.workflow,
        run_id.clone(),
        input,
        r.scope,
        Some(r.nodes),
    );
    repo.get_run(run_id).await
}

/// Settle a held run — paused (the row says so) or stopped by a loop
/// (`_workflow_hold`): in-flight steps back to pending, the hold written
/// (CAS), no delivery, no reap. `false`: not held, or a Cancel won.
pub(in crate::workflow_engine) async fn settle_held(
    ctx: &ServerCtx,
    repo: &WorkflowsRepo,
    workflow: &Workflow,
    run_id: &Id,
    states: &mut [NodeRunState],
) -> bool {
    let row = repo.get_run(run_id).await.ok();
    let paused = row.and_then(|r| r.hold.filter(|h| h.is_effective(r.status)));
    let Some(mut hold) = paused.or_else(|| states.iter().find_map(rules::loop_hold)) else {
        return false;
    };
    rules::settle_states(&workflow.graph, states, &mut hold);
    step_thread::retain_run(ctx, states, false).await;
    let Ok(Some(rev)) = repo.write_hold(run_id, &hold, states).await else {
        return false;
    };
    super::finalize::store_overview(repo, workflow, run_id, rev).await;
    let status = if hold.kind == HoldKind::Paused {
        "canceled"
    } else {
        "error"
    };
    emit_run_updated(
        ctx,
        &workflow.workspace_id,
        run_id,
        status,
        None,
        rev,
        None,
        states,
        false,
    );
    clear_skip_markers(ctx, run_id);
    true
}

/// Cancel of a held run: drop the hold (the settled status stays) and reap
/// its worktrees. `false`: it held none.
pub(crate) async fn cancel_held(ctx: &ServerCtx, run_id: &Id) -> Result<bool> {
    let repo = WorkflowsRepo::new(ctx.pool.clone());
    if !repo.clear_hold(run_id).await? {
        return Ok(false);
    }
    let run = repo.get_run(run_id).await?;
    let status = run.status.as_str();
    emit_run_updated(
        ctx,
        &run.workspace_id,
        run_id,
        status,
        None,
        run.rev,
        None,
        &run.nodes,
        false,
    );
    reap_run_worktrees(ctx, run_id).await;
    Ok(true)
}

/// Queue a steering note for the run's next agent step.
pub(crate) async fn steer(
    ctx: &ServerCtx,
    run_id: &Id,
    by: &User,
    note: &str,
) -> Result<Vec<String>> {
    let repo = WorkflowsRepo::new(ctx.pool.clone());
    otto_workflows::steering::queue_for_run(&repo, &ctx.data_dir, run_id, note, &by.display_name)
        .await
}

/// The notes no step has taken yet (`GET /workflow-runs/{id}`).
pub(crate) fn steering_queued(ctx: &ServerCtx, run_id: &Id) -> Vec<String> {
    let dir = ctx.data_dir.join("workflow-context").join(run_id.as_str());
    otto_workflows::steering::peek(&dir)
        .into_iter()
        .map(|n| n.text)
        .collect()
}

/// The starting agent step takes the queue: its `[steering]` section ("" for
/// none), each note recorded on the node's `human_log`.
pub(in crate::workflow_engine) fn take_steering(
    env: &RunEnv,
    session_tx: &tokio::sync::mpsc::UnboundedSender<AgentAssociation>,
) -> String {
    let Some(dir) = env.files.dir_str() else {
        return String::new();
    };
    let (section, entries) = otto_workflows::steering::take_entries(std::path::Path::new(&dir));
    for e in entries {
        let _ = session_tx.send(AgentAssociation::Step(StepNote::Human(e)));
    }
    section
}

/// A person's record for the criterion a check is parked on (`node_id` =
/// the check, or the loop around it; DR-107).
pub(crate) async fn verify(
    _ctx: &ServerCtx,
    run_id: &Id,
    node_id: &str,
    criterion_id: &str,
    by: &User,
    req: &VerifyCriterionReq,
) -> Result<()> {
    let (met, evidence, name) = (req.met, &req.evidence, &by.display_name);
    otto_workflows::check_run::deliver(run_id, node_id, criterion_id, met, evidence, name)
}

#[cfg(test)]
#[path = "../../../tests/unit/workflow_goal.rs"]
mod tests;
