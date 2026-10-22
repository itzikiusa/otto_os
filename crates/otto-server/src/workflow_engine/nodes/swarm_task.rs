//! The `swarm_task` workflow node, moved verbatim out of
//! `execute_node`'s match (see `nodes/mod.rs`).

use super::super::*;
use super::NodeCall;

pub(in crate::workflow_engine) async fn execute(
    call: NodeCall<'_>,
) -> Result<(Value, Vec<String>)> {
    let NodeCall {
        ctx, node, input, ..
    } = call;
    let p = &node.params;
    let swarm_id: Id = p
        .get("swarm_id")
        .and_then(Value::as_str)
        .ok_or_else(|| otto_core::Error::Invalid("swarm_task: missing swarm_id".into()))?
        .to_string();
    let project_id: Id = p
        .get("project_id")
        .and_then(Value::as_str)
        .ok_or_else(|| otto_core::Error::Invalid("swarm_task: missing project_id".into()))?
        .to_string();
    let raw_title = p
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("Workflow-generated task")
        .to_string();
    let raw_desc = p
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    // Simple {key} substitution from the input object.
    let sub = |s: String| -> String {
        if let Some(obj) = input.as_object() {
            obj.iter().fold(s, |acc, (k, v)| {
                let r = match v {
                    Value::String(sv) => sv.clone(),
                    other => other.to_string(),
                };
                acc.replace(&format!("{{{k}}}"), &r)
            })
        } else {
            s
        }
    };
    let title = sub(raw_title);
    let description = sub(raw_desc);

    let project = ctx
        .swarm_repo
        .get_project(&project_id)
        .await
        .map_err(|e| otto_core::Error::NotFound(format!("swarm_task: project: {e}")))?;

    // Validate the project belongs to the expected swarm.
    if project.swarm_id != swarm_id {
        return Err(otto_core::Error::Invalid(
            "swarm_task: project not in given swarm".into(),
        ));
    }

    let task = ctx
        .swarm_repo
        .create_task(NewSwarmTask {
            project_id: project.id.clone(),
            swarm_id: swarm_id.clone(),
            workspace_id: project.workspace_id.clone(),
            title: title.clone(),
            description: description.clone(),
            assignee_agent_id: None,
            status: "todo".into(),
            priority: "medium".into(),
            parent_task_id: None,
            depends_on: json!([]),
            labels: json!([]),
            order_idx: 0,
            created_by: "workflow-engine".into(),
        })
        .await
        .map_err(|e| otto_core::Error::Upstream(format!("swarm_task: create: {e}")))?;

    Ok((
        json!({ "task_id": task.id, "title": task.title, "status": task.status }),
        vec![format!("swarm_task: enqueued '{}'", task.title)],
    ))
}
