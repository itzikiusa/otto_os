//! Workflow preflight: syntax/structure errors are separate from dynamic values.
use otto_core::workflows::{WorkflowGraph, WorkflowNode};
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Serialize)]
pub struct Issue {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edge_id: Option<String>,
    pub field: String,
    pub message: String,
}

pub fn validate(graph: &WorkflowGraph) -> Vec<Issue> {
    let mut issues = vec![];
    if graph.nodes.is_empty() {
        issues.push(Issue {
            node_id: None,
            edge_id: None,
            field: "nodes".into(),
            message: "Add at least one step before running.".into(),
        });
    }
    let mut ids = HashSet::new();
    for node in &graph.nodes {
        if node.id.trim().is_empty() || !ids.insert(node.id.clone()) {
            issues.push(Issue {
                node_id: Some(node.id.clone()),
                edge_id: None,
                field: "id".into(),
                message: "Step IDs must be nonempty and unique.".into(),
            });
        }
        validate_node(node, &node.id, 0, &mut issues);
    }
    let mut edge_ids = HashSet::new();
    let mut indegree: HashMap<&str, usize> = ids.iter().map(|id| (id.as_str(), 0)).collect();
    for edge in &graph.edges {
        let message = if edge.id.trim().is_empty() || !edge_ids.insert(&edge.id) {
            Some("Edge IDs must be nonempty and unique.".into())
        } else if !ids.contains(&edge.source) || !ids.contains(&edge.target) {
            Some("Reconnect or remove this edge: its source or target step is missing.".into())
        } else if let Some(condition) = &edge.condition {
            otto_core::expr::validate(condition)
                .err()
                .map(|e| format!("Invalid branch expression: {e}"))
        } else {
            None
        };
        if let Some(message) = message {
            issues.push(Issue {
                node_id: None,
                edge_id: Some(edge.id.clone()),
                field: "edge".into(),
                message,
            });
        }
        if ids.contains(&edge.source) {
            if let Some(degree) = indegree.get_mut(edge.target.as_str()) {
                *degree += 1;
            }
        }
    }
    let mut queue: Vec<&str> = indegree
        .iter()
        .filter(|(_, degree)| **degree == 0)
        .map(|(id, _)| *id)
        .collect();
    let mut visited = 0;
    while let Some(id) = queue.pop() {
        visited += 1;
        for edge in graph.edges.iter().filter(|e| e.source == id) {
            if let Some(degree) = indegree.get_mut(edge.target.as_str()) {
                *degree = degree.saturating_sub(1);
                if *degree == 0 {
                    queue.push(&edge.target);
                }
            }
        }
    }
    if visited < ids.len() {
        for (id, degree) in indegree {
            if degree > 0 {
                issues.push(Issue {
                    node_id: Some(id.into()),
                    edge_id: None,
                    field: "edges".into(),
                    message:
                        "This step belongs to or follows a cycle. Use a bounded Loop step instead."
                            .into(),
                });
            }
        }
    }
    issues
}

fn validate_node(node: &WorkflowNode, root_id: &str, depth: usize, issues: &mut Vec<Issue>) {
    let mut issue = |field: &str, message: String| {
        issues.push(Issue {
            node_id: Some(root_id.into()),
            edge_id: None,
            field: field.into(),
            message,
        })
    };
    if !crate::workflow_engine::is_known_kind(&node.kind) {
        issue("kind", format!("Unsupported step kind '{}'.", node.kind));
    }
    let required: &[&str] = match node.kind.as_str() {
        "http_request" | "api_run" => &["url"],
        "db_query" => &["connection_id", "statement"],
        "broker_peek" => &["cluster_id", "topic"],
        "swarm_task" => &["swarm_id", "project_id"],
        _ => &[],
    };
    for key in required {
        if node
            .params
            .get(*key)
            .and_then(Value::as_str)
            .is_none_or(|s| s.trim().is_empty())
        {
            issue(key, format!("{}: set {key} before running.", node.name));
        }
    }
    let expression_key = match node.kind.as_str() {
        "condition" => Some("expr"),
        "loop" => Some("until"),
        _ => None,
    };
    if let Some(key) = expression_key {
        if let Some(source) = node
            .params
            .get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
        {
            if let Err(error) = otto_core::expr::validate(source) {
                issue(key, format!("Invalid {key} expression: {error}"));
            }
        }
    }
    if node.kind == "loop" {
        if depth >= 1 {
            issue(
                "steps",
                "Nested loops are not supported; place this loop beside its parent.".into(),
            );
            return;
        }
        let Some(steps) = node
            .params
            .get("steps")
            .and_then(Value::as_array)
            .filter(|s| !s.is_empty())
        else {
            issue("steps", "Add at least one inner step to the loop.".into());
            return;
        };
        for (index, value) in steps.iter().enumerate() {
            let mut value = value.clone();
            if let Some(object) = value.as_object_mut() {
                object.insert("id".into(), Value::String(format!("{}#{index}", node.id)));
            }
            match serde_json::from_value::<WorkflowNode>(value) {
                Ok(inner) => validate_node(&inner, root_id, depth + 1, issues),
                Err(error) => issues.push(Issue {
                    node_id: Some(root_id.into()),
                    edge_id: None,
                    field: "steps".into(),
                    message: format!("Inner step {} is invalid: {error}", index + 1),
                }),
            }
        }
    }
}

pub fn ensure_valid(graph: &WorkflowGraph) -> otto_core::Result<()> {
    let issues = validate(graph);
    if issues.is_empty() {
        return Ok(());
    }
    Err(otto_core::Error::Invalid(
        issues
            .iter()
            .map(|i| {
                format!(
                    "{}: {}",
                    i.node_id
                        .as_deref()
                        .or(i.edge_id.as_deref())
                        .unwrap_or("graph"),
                    i.message
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn rejects_missing_nodes_cycles_parameters_and_bad_expressions() {
        let graph = serde_json::from_value(json!({"nodes":[{"id":"a","kind":"http_request"},{"id":"b","kind":"condition","params":{"expr":"input.x =="}}],"edges":[{"id":"ab","source":"a","target":"b"},{"id":"ba","source":"b","target":"a"},{"id":"missing","source":"a","target":"no"}]})).unwrap();
        let issues = validate(&graph);
        for expected in ["url", "Invalid expr", "cycle", "missing"] {
            assert!(
                issues.iter().any(|i| i.message.contains(expected)),
                "missing finding {expected}: {issues:?}"
            );
        }
    }
    #[test]
    fn dynamic_values_and_bounded_loops_are_valid() {
        let graph = serde_json::from_value(json!({"nodes":[{"id":"loop","kind":"loop","params":{"until":"last.passed == true","steps":[{"kind":"api_run","params":{"url":"{{endpoint}}"}}]}}],"edges":[]})).unwrap();
        assert!(validate(&graph).is_empty());
    }
}
