//! A held run (goal.md §2.3) — what the engine's `run::hold` needs without
//! the server: the hold a Pause or a loop asks for, the node states a held
//! run settles with, Resume's preparation (limits, the unsettled steps, the
//! retry re-entry) and a loop's banked time.

use otto_core::workflows::{
    HoldKind, NodeRunState, NodeStatus, RunHold, RunScope, StepPhase, Workflow, WorkflowGraph,
    WorkflowRun,
};
use otto_core::{Error, Id, Result};
use otto_state::WorkflowsRepo;

/// A person's Pause.
pub fn pause_hold(by: &Id, name: &str) -> RunHold {
    RunHold {
        kind: HoldKind::Paused,
        reason: otto_core::agent_runtime::clip(&format!("Paused by {name}"), 300),
        since: chrono::Utc::now(),
        by: Some(by.clone()),
        node_ids: vec![],
        iteration: None,
    }
}

/// The hold a loop asked for: `output._workflow_hold = {kind, reason, iteration}`.
pub fn loop_hold(s: &NodeRunState) -> Option<RunHold> {
    let h = s.output.as_ref()?.get("_workflow_hold")?;
    Some(RunHold {
        kind: serde_json::from_value(h.get("kind")?.clone()).ok()?,
        reason: h.get("reason")?.as_str()?.to_string(),
        since: chrono::Utc::now(),
        by: None,
        node_ids: vec![s.node_id.clone()],
        iteration: h
            .get("iteration")
            .and_then(|i| i.as_u64())
            .map(|i| i as u32),
    })
}

/// The states a held run settles with: the steps in flight and the stopped
/// loop go back to `pending` with a `queued` mark noting the kind (no new
/// phase), the branch the loop's stop skipped too; in-flight steps join
/// `hold.node_ids`.
pub fn settle_states(graph: &WorkflowGraph, states: &mut [NodeRunState], hold: &mut RunHold) {
    let branch: std::collections::HashSet<String> = hold
        .node_ids
        .iter()
        .flat_map(|id| crate::graph::descendants_inclusive(graph, id))
        .collect();
    let note = serde_json::to_value(hold.kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string));
    for s in states.iter_mut() {
        let stopped = hold.node_ids.contains(&s.node_id);
        let in_flight = s.status == NodeStatus::Running;
        if in_flight && !stopped {
            hold.node_ids.push(s.node_id.clone());
        }
        let skipped = s.status == NodeStatus::Skipped && branch.contains(&s.node_id);
        if in_flight || stopped || skipped {
            s.status = NodeStatus::Pending;
            s.error = None;
            // A paused step is not "canceled", nor asking or waiting, when it resumes.
            (s.error_class, s.attention, s.wait) = (None, None, None);
        }
        if in_flight || stopped {
            // The run's unwind marked it "canceled"; it is held instead.
            if s.phases
                .last()
                .is_some_and(|m| m.note.as_deref() == Some("canceled"))
            {
                s.phases.pop();
            }
            crate::step_util::note_mark(s, StepPhase::Queued, note.as_deref().unwrap_or(""));
            s.phase = None;
        }
    }
}

/// A loop's banked active time: the sum over its inner checkpoints (G6).
pub async fn loop_active_ms(repo: &WorkflowsRepo, run_id: &Id, loop_id: &str) -> u64 {
    let cps = repo.checkpoints(run_id).await.unwrap_or_default();
    cps.iter()
        .filter(|c| c.loop_id == loop_id && c.node_id != loop_id)
        .map(|c| c.active_ms)
        .sum()
}

/// What Resume re-enters with.
pub struct Resume {
    pub run: WorkflowRun,
    pub workflow: Workflow,
    pub nodes: Vec<NodeRunState>,
    pub scope: RunScope,
}

/// Resume a held run: the stopped loop's new totals (`extend`, refused with
/// the person's words when they leave no room), then the retry re-entry over
/// every unsettled step — finished steps and iterations are adopted. The
/// caller has made sure no driver is still alive.
pub async fn prepare_resume(
    repo: &WorkflowsRepo,
    run_id: &Id,
    extend: (Option<u32>, Option<u64>),
) -> Result<Resume> {
    let run = repo.get_run(run_id).await?;
    let Some(hold) = run.hold.clone().filter(|h| h.is_effective(run.status)) else {
        return Err(Error::Conflict("the run is not held".into()));
    };
    let workflow = repo.definition_for_run(&run).await?;
    let stopped = hold.node_ids.iter().find_map(|id| {
        let n = workflow.graph.nodes.iter().find(|n| &n.id == id);
        n.filter(|n| n.kind == "loop")
    });
    if let Some(node) = stopped {
        let mut limits = run.limits.clone().unwrap_or_default();
        let used = (
            hold.iteration.unwrap_or(0),
            loop_active_ms(repo, run_id, &node.id).await,
        );
        let prev = limits.loops.get(&node.id);
        let l = crate::criteria::resume_limits(&node.params, prev, &hold, used, extend)
            .map_err(Error::Conflict)?;
        limits.loops.insert(node.id.clone(), l);
        repo.set_run_limits(run_id, &limits).await?;
    }
    let mut nodes = run.nodes.clone();
    for n in nodes.iter_mut().filter(|n| n.status == NodeStatus::Running) {
        n.status = NodeStatus::Pending;
    }
    let unsettled: Vec<String> = nodes
        .iter()
        .filter(|n| n.status == NodeStatus::Pending)
        .map(|n| n.node_id.clone())
        .collect();
    let scope = RunScope {
        continue_unfinished: true,
        ..Default::default()
    };
    repo.prepare_retry(run_id, &unsettled, false, &scope)
        .await?;
    Ok(Resume {
        run,
        workflow,
        nodes,
        scope,
    })
}
