//! Goals slice of the outward MCP surface (goal.md §8.1): `otto.run_goal_loop`
//! (Goal Loops' request shape, now started as a goal) and
//! `otto.control_workflow_run` (pause / resume / extend any workflow run).
//! Both are governed and DANGEROUS (DR-104): a session reaches them through
//! the approval queue, never a native twin. Kept in their own module so
//! `catalog::otto_tool_specs` and `exec::route_for` stay under their ratchets.

use otto_core::Error;
use serde_json::{json, Value};

use super::exec::{arg_str, seg, SelfCall};

/// Tool specs appended to `otto_tool_specs()`.
pub fn specs() -> [Value; 2] {
    [
        json!({"name":"otto.run_goal_loop","mutating":true,"category":"Agents",
            "description":"Start a goal: agents work in an isolated worktree or research folder until every criterion is met with evidence or the budget runs out. definition={title,summary?,acceptance_criteria:[{id,text,verify,verify_kind:command|agent|human,verify_cmd?}]}; limits={max_iterations,max_runtime_secs,per_phase_timeout_secs}; config={executors:[{name,provider,model?}],evaluator?,require_review?,mode?}. Returns {workflow_id, run_id}. DANGEROUS: approval-gated.",
            "inputSchema":{"type":"object","required":["workspace_id","name","repo_path","definition","limits","config"],"properties":{
                "workspace_id":{"type":"string"},"name":{"type":"string"},"repo_path":{"type":"string"},
                "definition":{"type":"object"},"limits":{"type":"object"},"config":{"type":"object"}}}}),
        json!({"name":"otto.control_workflow_run","mutating":true,"category":"Workflows",
            "description":"Pause or resume a workflow run; extend={iterations?,active_s?} sets a goal's new budget totals first. DANGEROUS: approval-gated.",
            "inputSchema":{"type":"object","required":["run_id","action"],"properties":{
                "run_id":{"type":"string"},"action":{"type":"string","enum":["pause","resume"]},
                "extend":{"type":"object"}}}}),
    ]
}

/// Map the goal tools to their self-calls; anything else falls through.
pub fn route_for(tool: &str, args: &Value) -> Result<SelfCall, Error> {
    Ok(match tool {
        "run_goal_loop" => {
            let ws = arg_str(args, "workspace_id")?;
            let mut body = args.clone();
            body.as_object_mut().map(|o| o.remove("workspace_id"));
            SelfCall::post(
                format!("/api/v1/workspaces/{}/goals/from-loop", seg(&ws)),
                body,
            )
        }
        "control_workflow_run" => {
            let run = seg(&arg_str(args, "run_id")?);
            match arg_str(args, "action")?.as_str() {
                "pause" => SelfCall::post(format!("/api/v1/workflow-runs/{run}/pause"), json!({})),
                "resume" => {
                    let extend = args.get("extend").filter(|v| !v.is_null()).cloned();
                    SelfCall::post(
                        format!("/api/v1/workflow-runs/{run}/resume"),
                        json!({"extend": extend}),
                    )
                }
                other => {
                    return Err(Error::Invalid(format!(
                        "action must be pause or resume, not '{other}'"
                    )))
                }
            }
        }
        other => return super::simulators::route_for(other, args),
    })
}

/// The approval card's text for the goal tools.
pub fn dangerous_detail(tool: &str, args: &Value) -> Option<String> {
    let text = |k: &str| {
        args.get(k)
            .and_then(Value::as_str)
            .unwrap_or("?")
            .to_string()
    };
    match tool {
        "control_workflow_run" => {
            let run = text("run_id");
            let n = |k: &str| {
                args.pointer(&format!("/extend/{k}"))
                    .and_then(Value::as_u64)
            };
            Some(
                match (text("action").as_str(), n("iterations"), n("active_s")) {
                    ("pause", ..) => format!("Pause workflow run '{run}'"),
                    (_, None, None) => format!("Resume workflow run '{run}'"),
                    (_, i, s) => {
                        let mut parts = Vec::new();
                        parts.extend(i.map(|i| format!("up to {i} iterations")));
                        parts.extend(s.map(|s| format!("up to {} min of active time", s / 60)));
                        format!("Resume workflow run '{run}' with {}", parts.join(", "))
                    }
                },
            )
        }
        "run_goal_loop" => {
            Some(format!(
            "Start goal '{}' in {}: agents work until its criteria are met or the budget runs out",
            args.pointer("/definition/title").and_then(Value::as_str).unwrap_or("?"),
            text("repo_path")
        ))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_goal_tools_route_to_the_goal_and_run_routes() {
        let call = route_for(
            "run_goal_loop",
            &json!({"workspace_id": "w 1", "name": "n", "repo_path": "/r"}),
        )
        .unwrap();
        assert_eq!(call.path, "/api/v1/workspaces/w%201/goals/from-loop");
        assert_eq!(call.body, Some(json!({"name": "n", "repo_path": "/r"})));
        let pause = route_for(
            "control_workflow_run",
            &json!({"run_id": "r1", "action": "pause"}),
        )
        .unwrap();
        assert_eq!(pause.path, "/api/v1/workflow-runs/r1/pause");
        let resume = route_for(
            "control_workflow_run",
            &json!({"run_id": "r1", "action": "resume", "extend": {"iterations": 9}}),
        )
        .unwrap();
        assert_eq!(
            (resume.path.as_str(), resume.body),
            (
                "/api/v1/workflow-runs/r1/resume",
                Some(json!({"extend": {"iterations": 9}}))
            )
        );
        assert!(route_for(
            "control_workflow_run",
            &json!({"run_id": "r1", "action": "stop"})
        )
        .is_err());
    }

    #[test]
    fn the_approval_card_says_what_is_paused_resumed_or_extended() {
        let d = |a: Value| dangerous_detail("control_workflow_run", &a).unwrap();
        assert_eq!(
            d(json!({"run_id": "r1", "action": "pause"})),
            "Pause workflow run 'r1'"
        );
        assert_eq!(
            d(json!({"run_id": "r1", "action": "resume"})),
            "Resume workflow run 'r1'"
        );
        assert_eq!(
            d(
                json!({"run_id": "r1", "action": "resume", "extend": {"iterations": 9, "active_s": 7200}})
            ),
            "Resume workflow run 'r1' with up to 9 iterations, up to 120 min of active time"
        );
        assert!(dangerous_detail(
            "run_goal_loop",
            &json!({"definition": {"title": "Green"}, "repo_path": "/r"})
        )
        .unwrap()
        .contains("'Green' in /r"));
    }
}
