//! Historical review evidence is carried forward as data, never as a checkout
//! or a successful gate for the new revision.
use super::super::*;

pub(super) async fn write_previous_context(
    ctx: &ServerCtx,
    workflow: &Workflow,
    input: &Value,
    files: &crate::workflow_context::RunContextFiles,
) {
    let Some(id) = input
        .get(otto_workflows::rereview::PREVIOUS_RUN_KEY)
        .and_then(Value::as_str)
    else {
        return;
    };
    let repo = WorkflowsRepo::new(ctx.pool.clone());
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
            let file = format!(
                "previous-{}.json",
                crate::workflow_context::slug(&node.node_id)
            );
            if files.write_named(&file, &text) {
                report.push_str(&format!("Full previous output: {file}\n\n"));
            }
        }
    }
    files.write_named("previous-review.md", &report);
}
