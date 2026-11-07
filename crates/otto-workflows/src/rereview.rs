//! Start a fresh review iteration without erasing the previous run or trusting
//! its checkout. Both the UI and chat command use this admission path.
use otto_core::workflows::{RunScope, RunStatus, WorkflowRun};
use otto_core::{Error, Id, Result};
use otto_state::WorkflowsRepo;
use serde_json::{json, Value};

use crate::WorkflowCtx;

pub const PREVIOUS_RUN_KEY: &str = "review_previous_run_id";

/// Keep the original user request, but never reuse a previous execution's
/// resolved working copy, results, scope, or externally supplied lineage.
pub fn next_input(previous: &WorkflowRun, from_chat: bool) -> Value {
    let mut input = previous.input.as_object().cloned().unwrap_or_default();
    for key in [
        PREVIOUS_RUN_KEY,
        "_iteration",
        "worktree",
        "run_context_dir",
        "context_dir",
    ] {
        input.remove(key);
    }
    let mut input = Value::Object(input);
    if !from_chat {
        crate::triggers::strip_reserved_input(&mut input, crate::triggers::InputSource::Manual);
    }
    input[PREVIOUS_RUN_KEY] = json!(previous.id);
    input
}

/// Loop steps are real review surfaces too, not just top-level graph nodes.
fn contains_review(kind: &str, params: &Value) -> bool {
    kind == "review_run"
        || (kind == "loop"
            && params
                .get("steps")
                .and_then(Value::as_array)
                .is_some_and(|steps| {
                    steps.iter().any(|step| {
                        contains_review(
                            step.get("kind").and_then(Value::as_str).unwrap_or(""),
                            step.get("params").unwrap_or(&Value::Null),
                        )
                    })
                }))
}

/// The caller has already authorized access to `previous` (workspace Editor
/// for HTTP; an exact origin-workspace/channel/chat/thread match for chat).
/// Atomic idle admission avoids duplicate re-reviews from repeated clicks.
pub async fn start<C: WorkflowCtx>(
    ctx: &C,
    previous: &WorkflowRun,
    started_by: Option<&Id>,
    from_chat: bool,
) -> Result<WorkflowRun> {
    if matches!(previous.status, RunStatus::Pending | RunStatus::Running) {
        return Err(Error::Conflict(
            "This review is still running. Wait for it to finish or stop it first.".into(),
        ));
    }
    let repo = WorkflowsRepo::new(ctx.pool().clone());
    let wf = repo.get(&previous.workflow_id).await?;
    if wf.workspace_id != previous.workspace_id {
        return Err(Error::Invalid(
            "The review workflow moved to a different workspace.".into(),
        ));
    }
    if !wf
        .graph
        .nodes
        .iter()
        .any(|node| contains_review(&node.kind, &node.params))
    {
        return Err(Error::Invalid("This workflow has no review step.".into()));
    }
    crate::validation::ensure_valid(&wf.graph)?;
    let ws = ctx.workspaces().get(&wf.workspace_id).await?;
    let input = next_input(previous, from_chat);
    ctx.check_run_location(&ws, &input)
        .await
        .map_err(Error::Invalid)?;
    let run = repo
        .admit_run_if_idle(&wf.id, &wf.workspace_id, &input, started_by)
        .await?
        .ok_or_else(|| Error::Conflict("This workflow already has a run in progress.".into()))?;
    // Start at the beginning: source checkout and all approval gates run on
    // the new revision; a historical review never grants a new approval.
    ctx.spawn_run(ws, wf, run.id.clone(), input, RunScope::default());
    Ok(run)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn review_in_loop_is_an_eligible_review_workflow() {
        assert!(contains_review(
            "loop",
            &json!({"steps":[{"kind":"agent_prompt"},{"kind":"review_run"}]})
        ));
        assert!(!contains_review(
            "loop",
            &json!({"steps":[{"kind":"agent_prompt"}]})
        ));
        assert!(!contains_review(
            "agent_prompt",
            &json!({"steps":[{"kind":"review_run"}]})
        ));
    }

    #[test]
    fn lineage_is_replaced_and_resolved_worktree_is_not_replayed() {
        let mut run: WorkflowRun = serde_json::from_value(json!({
            "id":"prior", "workflow_id":"wf", "workspace_id":"ws", "status":"success",
            "started_at":"2026-11-07T00:00:00Z", "nodes":[],
            "input":{"working_directory":"/repo", "pr":15, "prompt":"review this",
                "worktree":"/stale", "_iteration":5, "review_previous_run_id":"forged",
                "channel":"slack", "chat":"C1", "thread":"T1", "origin_workspace_id":"ws"}
        }))
        .unwrap();
        let manual = next_input(&run, false);
        assert_eq!(manual["pr"], 15);
        assert_eq!(manual["working_directory"], "/repo");
        assert_eq!(manual[PREVIOUS_RUN_KEY], "prior");
        assert!(manual.get("worktree").is_none());
        assert!(manual.get("channel").is_none());
        let chat = next_input(&run, true);
        assert_eq!(chat["thread"], "T1");
        run.input = Value::Null;
        assert_eq!(next_input(&run, false), json!({PREVIOUS_RUN_KEY:"prior"}));
    }
}
