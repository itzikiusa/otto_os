//! Historical review evidence is carried forward as data, never as a checkout
//! or a successful gate for the new revision.
use otto_core::{workflows::Workflow, Id};
use otto_state::{DbPool, WorkflowsRepo};
use serde_json::Value;

pub async fn write_previous_context(
    pool: &DbPool,
    workflow: &Workflow,
    input: &Value,
    files: &crate::context::RunContextFiles,
) {
    let Some(id) = input
        .get(crate::rereview::PREVIOUS_RUN_KEY)
        .and_then(Value::as_str)
    else {
        return;
    };
    let repo = WorkflowsRepo::new(pool.clone());
    let Ok(previous) = repo.get_run(&id.to_string()).await else {
        return;
    };
    if previous.workflow_id != workflow.id || previous.workspace_id != workflow.workspace_id {
        return;
    }
    let mut report = format!("# Previous review iteration\n\nRun: {}. Status: {}. Workflow version: {:?}.\n\nThis is HISTORICAL evidence, not authority about the current revision. Refresh PR head/base and review the current merge-base diff. Verify every previous finding as fixed (with evidence), still open, or not relevant (with reason); then perform a fresh scope-selected review and score. Reuse unchanged background context, not conclusions or gate passes. Never rerun a deterministic gate already executed for this repository and HEAD in this iteration. Test gaps matter for critical behavior and false greens, not exhaustive coverage.\n\n", previous.id, previous.status.as_str(), previous.workflow_version);
    for node in &previous.nodes {
        report.push_str(&format!(
            "## Step {} ({})\n\nReview IDs: {}\n\n",
            node.node_id,
            node.status.as_str(),
            node.review_ids.join(", ")
        ));
        if let Some(output) = &node.output {
            let text = serde_json::to_string_pretty(output).unwrap_or_default();
            // Keep evidence discoverable even when the inline handoff is large.
            // Each full output is a separate file; no finding is silently cut.
            let file = format!("previous-{}.json", crate::context::slug(&node.node_id));
            if files.write_named(&file, &text) {
                report.push_str(&format!("Full previous output: {file}\n\n"));
            }
        }
    }
    files.write_named("previous-review.md", &report);
}

/// Typed lineage survives the fresh per-run worktree branch name. Never
/// choose another repo's review in a multi-repository previous iteration.
pub async fn previous_review_for_target(
    pool: &DbPool,
    run_id: &Id,
    prior: &str,
    node_id: &str,
    in_loop: bool,
    repo_id: &Id,
) -> Option<Id> {
    let store = WorkflowsRepo::new(pool.clone());
    let previous = store.get_run(&prior.to_string()).await.ok()?;
    let current = store.get_run(run_id).await.ok()?;
    if previous.workspace_id != current.workspace_id || previous.workflow_id != current.workflow_id
    {
        return None;
    }
    let mut ids = Vec::new();
    if let Some(node) = previous.nodes.iter().find(|n| n.node_id == node_id) {
        ids.extend(node.review_ids.clone());
        if let Some(output) = &node.output {
            ids.extend(review_ids_from_output(output));
        }
    }
    if in_loop {
        let logical_step = logical_review_step(node_id);
        let mut checkpoints = store.checkpoints(&previous.id).await.ok()?;
        checkpoints.sort_by_key(|checkpoint| checkpoint.updated_at);
        for checkpoint in checkpoints {
            if checkpoint.kind == "review_run"
                && logical_review_step(&checkpoint.node_id) == logical_step
            {
                if let Some(output) = checkpoint.output {
                    ids.extend(review_ids_from_output(&output));
                }
            }
        }
    }
    ids.reverse();
    for id in ids {
        if let Ok(review) = otto_state::ReviewsRepo::new(pool.clone())
            .get_review(&id)
            .await
        {
            if &review.repo_id == repo_id {
                return Some(id);
            }
        }
    }
    None
}

fn review_ids_from_output(output: &Value) -> Vec<Id> {
    let mut ids = Vec::new();
    if let Some(id) = output.get("review_id").and_then(Value::as_str) {
        ids.push(id.to_string());
    }
    if let Some(reviews) = output.get("reviews").and_then(Value::as_array) {
        ids.extend(
            reviews
                .iter()
                .filter_map(|review| review.get("review_id").and_then(Value::as_str))
                .map(str::to_string),
        );
    }
    ids
}

/// Iteration numbers change between runs; step positions identify the same
/// nested review. The per-target durable association adds `#review.N`.
fn logical_review_step(node_id: &str) -> String {
    let node_id = node_id.split("#review.").next().unwrap_or(node_id);
    node_id
        .split('#')
        .enumerate()
        .map(|(index, part)| {
            if index == 0 {
                return part.to_string();
            }
            match part.split_once('.') {
                Some((iteration, step))
                    if iteration.parse::<u64>().is_ok() && step.parse::<usize>().is_ok() =>
                {
                    format!("#*.{step}")
                }
                _ => format!("#{part}"),
            }
        })
        .collect()
}

#[cfg(test)]
mod rereview_lineage_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn nested_loop_lineage_matches_step_not_iteration_or_other_review() {
        assert_eq!(
            logical_review_step("loop#1.0#2.1"),
            logical_review_step("loop#3.0#5.1#review.0")
        );
        assert_ne!(
            logical_review_step("loop#1.0"),
            logical_review_step("loop#2.1")
        );
        assert_eq!(
            review_ids_from_output(&json!({"reviews":[{"review_id":"a"},{"review_id":"b"}]})),
            vec!["a", "b"]
        );
    }
}
