//! Graph orchestration shares the controller's executor, evaluator and deadlines.
use super::*;
use crate::goal_loop_graph as graph;
use otto_core::domain::GoalGraphNode;

pub(super) struct Task {
    pub selected: Option<String>,
    pub scoped: GoalLoop,
    pub execute: bool,
}

pub(super) async fn prepare(
    ctx: &impl AutomationCtx,
    loop_: &mut GoalLoop,
    iter: &otto_core::domain::GoalLoopIteration,
    wt: &str,
    handle: &LoopHandle,
    verification_only: bool,
) -> Result<Task> {
    let Some(dag) = loop_.config.graph.clone() else {
        return Ok(Task {
            selected: None,
            scoped: loop_.clone(),
            execute: !verification_only,
        });
    };
    graph::initialize(&dag, &mut loop_.ledger);
    if verification_only {
        // Reuse the task that produced this iteration's output, never choose a
        // different ready task while evaluating that output after retry/approval.
        let selected = loop_
            .ledger
            .graph_nodes
            .iter()
            .find(|s| s.iteration == iter.idx && s.status != "pending")
            .map(|s| s.node_id.clone());
        return Ok(Task {
            selected,
            scoped: loop_.clone(),
            execute: false,
        });
    }
    let mut check = loop_.clone();
    check.definition.acceptance_criteria.retain(|c| {
        dag.nodes.iter().any(|n| {
            n.criterion_ids.contains(&c.id)
                && loop_
                    .ledger
                    .graph_nodes
                    .iter()
                    .any(|s| s.node_id == n.id && graph::needs_refresh(s))
        })
    });
    if !check.definition.acceptance_criteria.is_empty() {
        set_phase(
            ctx,
            &loop_.workspace_id,
            loop_,
            GoalLoopPhase::Evaluating,
            iter.idx,
        )
        .await;
        let evidence = evaluate(ctx, &check, &iter.id, wt, &["Read-only prerequisite revalidation before task admission. Inspect current files and rerun checks; previous evidence is not a pass.".into()], loop_.limits.per_phase_timeout_secs, handle).await;
        if stop_requested(handle) {
            return Err(Error::Conflict("task admission interrupted".into()));
        }
        graph::reconcile(&dag, &mut loop_.ledger, &evidence, None);
    }
    let selected = graph::next_node(&dag, &loop_.ledger).cloned();
    let mut scoped = loop_.clone();
    if let Some(node) = &selected {
        let state = loop_
            .ledger
            .graph_nodes
            .iter_mut()
            .find(|s| s.node_id == node.id)
            .unwrap();
        state.status = "running".into();
        state.iteration = iter.idx;
        state.evidence = "Prerequisites reverified; task admitted".into();
        state.criteria.clear();
        scoped = scope_task(loop_, node);
    }
    // Admission must persist before starting a planner or executor; errors fail closed.
    ctx.goal_loops_repo()
        .set_ledger(&loop_.id, &loop_.ledger)
        .await?;
    emit(
        ctx,
        &loop_.workspace_id,
        &loop_.id,
        GoalLoopStatus::Running,
        GoalLoopPhase::Planning,
        iter.idx,
        loop_.progress_pct,
    );
    Ok(Task {
        selected: selected.as_ref().map(|n| n.id.clone()),
        scoped,
        execute: selected.is_some(),
    })
}

fn scope_task(loop_: &GoalLoop, node: &GoalGraphNode) -> GoalLoop {
    let mut scoped = loop_.clone();
    scoped.definition.title = format!("{} — task: {}", loop_.definition.title, node.title);
    scoped.definition.summary = format!(
        "Execute ONLY graph task {} this iteration.\n{}",
        node.id, node.instructions
    );
    scoped.definition.objectives = vec![node.instructions.clone()];
    scoped
        .definition
        .acceptance_criteria
        .retain(|c| node.criterion_ids.contains(&c.id));
    scoped.definition.constraints.push("Do not execute other graph tasks. Preserve prerequisites; report regressions for the controller to repair.".into());
    scoped
}

pub(super) async fn verify_executor_completion(
    ctx: &impl AutomationCtx,
    loop_: &GoalLoop,
    iteration: u32,
    selected: Option<&str>,
    eval: &mut GoalLoopEvaluation,
) {
    let Some(node) = loop_
        .config
        .graph
        .as_ref()
        .and_then(|g| g.nodes.iter().find(|n| Some(n.id.as_str()) == selected))
    else {
        return;
    };
    let complete = ctx
        .goal_loops_repo()
        .get_iteration(&loop_.id, iteration)
        .await
        .is_ok_and(|i| {
            i.agents.len() == loop_.config.executors.len()
                && i.agents.iter().all(|a| a.status == "done")
        });
    if !complete {
        for id in &node.criterion_ids {
            set_criterion(
                eval,
                id,
                false,
                "Task executors did not all complete; use Resume to retry",
            );
        }
        eval.verdict = "continue".into();
    }
}

pub(super) fn finish(
    loop_: &mut GoalLoop,
    selected: Option<&str>,
    eval: &mut GoalLoopEvaluation,
) -> bool {
    let Some(dag) = &loop_.config.graph else {
        return crate::goal_loop_policy::record_progress(&mut loop_.ledger, eval);
    };
    graph::reconcile(dag, &mut loop_.ledger, eval, selected);
    graph::reconcile_verdict(dag, &loop_.ledger, eval);
    let mut task_eval = eval.clone();
    if let Some(node) = dag.nodes.iter().find(|n| Some(n.id.as_str()) == selected) {
        task_eval
            .criteria
            .retain(|c| node.criterion_ids.contains(&c.id));
    } else {
        task_eval.criteria.clear();
    }
    // Future tasks' unmet criteria must not count as stagnation of this task.
    crate::goal_loop_policy::record_progress(&mut loop_.ledger, &task_eval)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> GoalLoop {
        serde_json::from_value(serde_json::json!({
            "id":"g", "workspace_id":"w", "name":"Goal", "repo_path":"/tmp/isolated",
            "definition":{"title":"Goal","objectives":["Do all work"],"acceptance_criteria":[
                {"id":"a","text":"Current","verify":"inspect"},{"id":"b","text":"Future","verify":"inspect"}
            ]},
            "config":otto_core::domain::GoalLoopConfig::default(),
            "limits":{"max_iterations":1,"max_runtime_secs":3600,"per_phase_timeout_secs":60},
            "status":"blocked", "phase":"done", "iterations_started":1, "current_iteration":1,
            "progress_pct":0, "elapsed_secs":0, "cost_usd":0, "created_by":"u",
            "created_at":Utc::now(), "updated_at":Utc::now()
        })).unwrap()
    }
    fn node() -> GoalGraphNode {
        GoalGraphNode {
            id: "a".into(),
            title: "Inspect".into(),
            instructions: "Trace entrypoint".into(),
            depends_on: vec![],
            criterion_ids: vec!["a".into()],
        }
    }
    #[test]
    fn task_scope_does_not_expose_future_criteria_as_executor_work() {
        let original = fixture();
        let scoped = scope_task(&original, &node());
        assert_eq!(scoped.definition.acceptance_criteria.len(), 1);
        assert_eq!(scoped.definition.acceptance_criteria[0].id, "a");
        assert_eq!(scoped.definition.objectives, vec!["Trace entrypoint"]);
        let prompt = executor_prompt(
            &scoped,
            &scoped.config.executors[0],
            "Inspect",
            "",
            "/tmp/result",
        );
        assert!(prompt.contains("Execute ONLY graph task a"));
        assert!(!prompt.contains("[b] Future"));
        assert_eq!(original.definition.acceptance_criteria.len(), 2);
    }
    #[test]
    fn completing_tasks_does_not_stagnate_on_future_criteria() {
        let mut goal = fixture();
        let mut second = node();
        second.id = "b".into();
        second.criterion_ids = vec!["b".into()];
        second.depends_on = vec!["a".into()];
        goal.config.graph = Some(otto_core::domain::GoalLoopGraph {
            nodes: vec![node(), second],
        });
        graph::initialize(goal.config.graph.as_ref().unwrap(), &mut goal.ledger);
        goal.ledger.graph_nodes[0].status = "running".into();
        goal.ledger.graph_nodes[0].iteration = 1;
        let mut eval: GoalLoopEvaluation = serde_json::from_value(serde_json::json!({"progress_pct":50,"verdict":"continue","criteria":[
            {"id":"a","met":true,"evidence":"checked"},{"id":"b","met":false,"evidence":"not yet"}
        ]})).unwrap();
        assert!(!finish(&mut goal, Some("a"), &mut eval));
        assert_eq!(goal.ledger.repeated_failures, 0);
        assert_eq!(goal.ledger.graph_nodes[0].status, "passed");
        assert_eq!(
            graph::next_node(goal.config.graph.as_ref().unwrap(), &goal.ledger)
                .unwrap()
                .id,
            "b"
        );
    }
    #[test]
    fn human_continuation_rechecks_existing_iteration_without_budget_refund() {
        let mut goal = fixture();
        goal.definition.acceptance_criteria[0].verify_kind = "human".into();
        goal.config.graph = Some(otto_core::domain::GoalLoopGraph {
            nodes: vec![node()],
        });
        graph::initialize(goal.config.graph.as_ref().unwrap(), &mut goal.ledger);
        goal.ledger.graph_nodes[0].status = "blocked".into();
        goal.ledger.graph_nodes[0].iteration = 1;
        goal.ledger.graph_nodes[0].criteria = vec![otto_core::domain::EvalCriterion {
            id: "a".into(),
            met: false,
            evidence: "Awaiting human verification".into(),
        }];
        assert!(!evaluation_only(&goal, None));
        goal.ledger
            .verifications
            .push(otto_core::domain::GoalHumanVerification {
                criterion_id: "a".into(),
                criterion_revision: goal.definition.acceptance_criteria[0].revision(),
                verified_by: "person".into(),
                evidence: "Inspected output".into(),
                verified_at: Utc::now(),
            });
        assert!(evaluation_only(&goal, None));
        assert_eq!(goal.iterations_started, goal.limits.max_iterations);
    }
}
