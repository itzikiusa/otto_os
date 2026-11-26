//! Workflow preflight, in two classes (DR-028). **Save** problems make a
//! definition invalid (a broken structure, an unknown enum value, a file name
//! that leaves the run dir, a bad `continue_from`) — create, update and
//! import refuse them. **Run** problems are a draft's missing pieces (an empty
//! graph, a blank prompt or command) — the draft saves, Run and triggers
//! refuse it. **Warnings** never block.
use otto_core::workflows::{WorkflowGraph, WorkflowNode};
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

use crate::graph::ancestors;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueClass {
    /// The definition is invalid: saving is refused.
    Save,
    /// The definition is a draft: it saves, running is refused.
    Run,
    /// Worth a look; never blocks.
    Warning,
}

#[derive(Debug, Serialize)]
pub struct Issue {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edge_id: Option<String>,
    pub field: String,
    pub message: String,
    pub class: IssueClass,
}

/// Every problem of `graph`, both classes and warnings.
pub fn validate(graph: &WorkflowGraph) -> Vec<Issue> {
    let mut issues = vec![];
    let mut push = |node_id: Option<&str>,
                    edge_id: Option<&str>,
                    field: &str,
                    message: String,
                    class: IssueClass| {
        issues.push(Issue {
            node_id: node_id.map(str::to_string),
            edge_id: edge_id.map(str::to_string),
            field: field.into(),
            message,
            class,
        })
    };
    if graph.nodes.is_empty() {
        push(
            None,
            None,
            "nodes",
            "Add at least one step before running.".into(),
            IssueClass::Run,
        );
    }
    let mut ids = HashSet::new();
    for node in &graph.nodes {
        if node.id.trim().is_empty() || !ids.insert(node.id.clone()) {
            push(
                Some(&node.id),
                None,
                "id",
                "Step IDs must be nonempty and unique.".into(),
                IssueClass::Save,
            );
        }
        validate_node(node, &node.id, 0, &mut push);
    }
    let mut edge_ids = HashSet::new();
    let mut indegree: HashMap<&str, usize> = ids.iter().map(|id| (id.as_str(), 0)).collect();
    for edge in &graph.edges {
        let problem = if edge.id.trim().is_empty() || !edge_ids.insert(&edge.id) {
            Some((
                "Edge IDs must be nonempty and unique.".to_string(),
                IssueClass::Save,
            ))
        } else if !ids.contains(&edge.source) || !ids.contains(&edge.target) {
            Some((
                "Reconnect or remove this edge: its source or target step is missing.".to_string(),
                IssueClass::Save,
            ))
        } else if let Some(condition) = &edge.condition {
            otto_core::expr::validate(condition)
                .err()
                .map(|e| (format!("Invalid branch expression: {e}"), IssueClass::Run))
        } else {
            None
        };
        if let Some((message, class)) = problem {
            push(None, Some(&edge.id), "edge", message, class);
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
    let acyclic = visited >= ids.len();
    if !acyclic {
        for (id, degree) in indegree {
            if degree > 0 {
                push(
                    Some(id),
                    None,
                    "edges",
                    "This step belongs to or follows a cycle. Use a bounded Loop step instead."
                        .into(),
                    IssueClass::Save,
                );
            }
        }
    }
    if acyclic {
        graph_rules(graph, &mut push);
    }
    issues
}

type Push<'a> = dyn FnMut(Option<&str>, Option<&str>, &str, String, IssueClass) + 'a;

/// Rules over the graph's shape: `continue_from`, the review a posting step
/// reads, and conditions on `run.repos` beside a repos writer.
fn graph_rules(graph: &WorkflowGraph, push: &mut Push<'_>) {
    let node = |id: &str| graph.nodes.iter().find(|n| n.id == id);
    let related =
        |a: &str, b: &str| ancestors(graph, a).contains(b) || ancestors(graph, b).contains(a);
    let mut continuers: HashMap<&str, Vec<&str>> = HashMap::new();
    for n in &graph.nodes {
        let up = ancestors(graph, &n.id);
        if let Some(from) = str_param(&n.params, "continue_from") {
            let message = match node(from) {
                _ if n.kind != "agent_prompt" => Some("continue_from is only for Agent steps.".to_string()),
                Some(src) if !up.contains(from) || src.kind != "agent_prompt" => {
                    Some(format!("continue_from must name an earlier Agent step this one depends on; '{from}' is not."))
                }
                Some(src) if provider(src) != provider(n) => Some(format!(
                    "continue_from: '{from}' runs on another provider; a thread continues only on its own."
                )),
                None => Some(format!("continue_from names a step that does not exist: '{from}'.")),
                Some(_) => None,
            };
            match message {
                Some(m) => push(Some(&n.id), None, "continue_from", m, IssueClass::Save),
                None => continuers.entry(from).or_default().push(&n.id),
            }
        }
        if n.kind == "post_review_comments" {
            let review = match str_param(&n.params, "review_from") {
                Some(from) => {
                    up.contains(from) && node(from).is_some_and(|r| r.kind == "review_run")
                }
                None => up
                    .iter()
                    .any(|a| node(a).is_some_and(|r| r.kind == "review_run")),
            };
            if !review {
                push(
                    Some(&n.id),
                    None,
                    "review_from",
                    format!(
                        "{}: needs a Review Run step before it to post from.",
                        n.name
                    ),
                    IssueClass::Run,
                );
            }
            if let Some(from) =
                str_param(&n.params, "summary_intro_from").filter(|f| !up.contains(*f))
            {
                push(
                    Some(&n.id),
                    None,
                    "summary_intro_from",
                    format!("summary_intro_from must name an earlier step this one depends on; '{from}' is not."),
                    IssueClass::Run,
                );
            }
        }
    }
    for (from, steps) in continuers {
        for (i, a) in steps.iter().enumerate() {
            for b in &steps[i + 1..] {
                if !related(a, b) {
                    push(
                        Some(b),
                        None,
                        "continue_from",
                        format!("'{a}' and '{b}' both continue '{from}' and can run at the same time; a thread takes one turn at a time."),
                        IssueClass::Save,
                    );
                }
            }
        }
    }
    // A condition reads run.repos when its source settles: a step that
    // rewrites repos.json must not run beside it (engine.md §3.3, DR-013).
    let writers: Vec<&WorkflowNode> = graph.nodes.iter().filter(|n| writes_repos(n)).collect();
    let mut readers: Vec<(&str, Option<&str>)> = graph
        .edges
        .iter()
        .filter(|e| e.condition.as_deref().is_some_and(reads_run_refs))
        .map(|e| (e.source.as_str(), Some(e.id.as_str())))
        .collect();
    readers.extend(
        graph
            .nodes
            .iter()
            .filter(|n| {
                n.kind == "condition" && str_param(&n.params, "expr").is_some_and(reads_run_refs)
            })
            .map(|n| (n.id.as_str(), None)),
    );
    for (reader, edge) in readers {
        for w in &writers {
            if w.id != reader && !related(&w.id, reader) {
                push(
                    Some(reader),
                    edge,
                    "condition",
                    format!(
                        "This condition reads run.repos / run.prs while '{}' may rewrite repos.json beside it; make one depend on the other so the result does not depend on timing.",
                        w.name
                    ),
                    IssueClass::Warning,
                );
            }
        }
    }
}

fn reads_run_refs(expr: &str) -> bool {
    expr.contains("run.repos") || expr.contains("run.prs")
}

/// A step that rewrites `repos.json`: `git_checkout`, or an agent whose
/// prompt says it rewrites or updates the file.
fn writes_repos(n: &WorkflowNode) -> bool {
    n.kind == "git_checkout"
        || n.kind == "agent_prompt"
            && str_param(&n.params, "prompt").is_some_and(|p| {
                let p = p.to_lowercase();
                p.contains("repos.json")
                    && ["rewrite", "update", "write"].iter().any(|w| p.contains(w))
            })
}

fn provider(n: &WorkflowNode) -> &str {
    str_param(&n.params, "provider").unwrap_or("")
}

fn str_param<'a>(params: &'a Value, key: &str) -> Option<&'a str> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// Allowed values of the new enum params (absent = the default).
const ENUMS: &[(&str, &str, &[&str])] = &[
    ("git_checkout", "mode", &["pr", "branch"]),
    ("script", "foreach", &["none", "repos"]),
    ("script", "cwd", &["worktree", "context"]),
    ("post_review_comments", "info", &["combined", "skip"]),
    ("manual_trigger", "chat_updates", &["live", "per_step"]),
    ("manual_trigger", "lane", &["normal", "long"]),
    ("review_run", "scope_assessment", &["configured"]),
];

/// Kinds that run an agent: `timeout_s` 60–36,000 s.
const AGENT_KINDS: &[&str] = &["agent_prompt", "prepare_context", "review_run", "canvas"];

fn validate_node(node: &WorkflowNode, root_id: &str, depth: usize, push: &mut Push<'_>) {
    let mut issue = |field: &str, message: String, class: IssueClass| {
        push(Some(root_id), None, field, message, class)
    };
    if !crate::catalog::is_known_kind(&node.kind) {
        issue(
            "kind",
            format!("Unsupported step kind '{}'.", node.kind),
            IssueClass::Save,
        );
    }
    let p = &node.params;
    let required: &[&str] = match node.kind.as_str() {
        "http_request" | "api_run" => &["url"],
        "db_query" => &["connection_id", "statement"],
        "broker_peek" => &["cluster_id", "topic"],
        "script" => &["command"],
        _ => &[],
    };
    for key in required {
        if str_param(p, key).is_none() {
            issue(
                key,
                format!("{}: set {key} before running.", node.name),
                IssueClass::Run,
            );
        }
    }
    for (kind, key, allowed) in ENUMS {
        if node.kind != *kind {
            continue;
        }
        if let Some(v) = p.get(*key).filter(|v| !v.is_null()) {
            if !v.as_str().is_some_and(|s| allowed.contains(&s)) {
                issue(
                    key,
                    format!(
                        "{}: {key} must be one of {}.",
                        node.name,
                        allowed.join(", ")
                    ),
                    IssueClass::Save,
                );
            }
        }
    }
    let range = if node.kind == "script" {
        Some(10..=7_200)
    } else {
        AGENT_KINDS
            .contains(&node.kind.as_str())
            .then_some(60..=36_000)
    };
    if let Some(range) = range {
        match p.get("timeout_s").filter(|v| !v.is_null()) {
            Some(v) if !v.as_u64().is_some_and(|s| range.contains(&s)) => issue(
                "timeout_s",
                format!(
                    "{}: timeout_s must be {} to {} seconds.",
                    node.name,
                    range.start(),
                    range.end()
                ),
                IssueClass::Save,
            ),
            None if node.kind == "script" => issue(
                "timeout_s",
                format!("{}: set timeout_s before running.", node.name),
                IssueClass::Run,
            ),
            _ => {}
        }
    }
    // Context-file names from a definition stay inside the run dir (DR-002).
    let mut names: Vec<(&str, &Value)> = vec![];
    if node.kind == "script" {
        names.extend(p.get("output_file").map(|v| ("output_file", v)));
    }
    if node.kind == "post_review_comments" {
        if let Some(files) = p.get("summary_files").and_then(Value::as_array) {
            names.extend(files.iter().map(|v| ("summary_files", v)));
        }
    }
    for (key, v) in names {
        if !v.as_str().is_some_and(crate::context::is_bare_name) {
            issue(
                key,
                format!(
                    "{}: {key} must be a file name in the run's context folder (no path, no '..').",
                    node.name
                ),
                IssueClass::Save,
            );
        }
    }
    if node.kind == "check" {
        validate_criteria(node, &mut issue);
    }
    let expression_key = match node.kind.as_str() {
        "condition" => Some("expr"),
        "loop" => Some("until"),
        _ => None,
    };
    if let Some(key) = expression_key {
        if let Some(source) = str_param(p, key) {
            if let Err(error) = otto_core::expr::validate(source) {
                issue(
                    key,
                    format!("Invalid {key} expression: {error}"),
                    IssueClass::Run,
                );
            }
        }
    }
    if node.kind == "loop" {
        if depth >= 1 {
            issue(
                "steps",
                "Nested loops are not supported; place this loop beside its parent.".into(),
                IssueClass::Save,
            );
            return;
        }
        let Some(steps) = p
            .get("steps")
            .and_then(Value::as_array)
            .filter(|s| !s.is_empty())
        else {
            issue(
                "steps",
                "Add at least one inner step to the loop.".into(),
                IssueClass::Run,
            );
            return;
        };
        for (index, value) in steps.iter().enumerate() {
            let mut value = value.clone();
            if let Some(object) = value.as_object_mut() {
                object.insert("id".into(), Value::String(format!("{}#{index}", node.id)));
            }
            match serde_json::from_value::<WorkflowNode>(value) {
                Ok(inner) => validate_node(&inner, root_id, depth + 1, push),
                Err(error) => push(
                    Some(root_id),
                    None,
                    "steps",
                    format!("Inner step {} is invalid: {error}", index + 1),
                    IssueClass::Save,
                ),
            }
        }
    }
}

/// A `check` node's criteria (goal.md §2.1): duplicate ids, unknown verify
/// kinds and unparsable rules are invalid; no criteria, blank text or a blank
/// command are a draft's missing pieces.
fn validate_criteria(node: &WorkflowNode, issue: &mut impl FnMut(&str, String, IssueClass)) {
    let criteria = node
        .params
        .get("criteria")
        .and_then(Value::as_array)
        .filter(|c| !c.is_empty());
    let Some(criteria) = criteria else {
        issue(
            "criteria",
            format!("{}: add at least one criterion before running.", node.name),
            IssueClass::Run,
        );
        return;
    };
    let mut seen = HashSet::new();
    for c in criteria {
        let id = str_param(c, "id").unwrap_or("");
        if !seen.insert(id) {
            issue(
                "criteria",
                format!("{}: criterion ids must be unique ('{id}').", node.name),
                IssueClass::Save,
            );
        }
        let verify = str_param(c, "verify").unwrap_or("");
        if !["command", "agent", "human", "expr"].contains(&verify) {
            issue(
                "criteria",
                format!(
                    "{}: criterion '{id}' has an unknown check '{verify}'.",
                    node.name
                ),
                IssueClass::Save,
            );
        }
        if str_param(c, "text").is_none() {
            issue(
                "criteria",
                format!("{}: criterion '{id}' needs its text.", node.name),
                IssueClass::Run,
            );
        }
        if verify == "command" && str_param(c, "command").is_none() {
            issue(
                "criteria",
                format!("{}: criterion '{id}' needs its command.", node.name),
                IssueClass::Run,
            );
        }
        if let Some(expr) = str_param(c, "expr").filter(|_| verify == "expr") {
            if let Err(e) = otto_core::expr::validate(expr) {
                issue(
                    "criteria",
                    format!("{}: criterion '{id}': invalid rule: {e}", node.name),
                    IssueClass::Save,
                );
            }
        }
    }
}

/// The run gate: refuse a graph with any Save or Run problem.
pub fn ensure_valid(graph: &WorkflowGraph) -> otto_core::Result<()> {
    refuse(graph, |c| c != IssueClass::Warning)
}

/// The save gate (create, update, import): refuse only Save problems, so a
/// blank new workflow or a half-configured step saves (DR-028).
pub fn ensure_savable(graph: &WorkflowGraph) -> otto_core::Result<()> {
    refuse(graph, |c| c == IssueClass::Save)
}

fn refuse(graph: &WorkflowGraph, blocks: impl Fn(IssueClass) -> bool) -> otto_core::Result<()> {
    let issues: Vec<Issue> = validate(graph)
        .into_iter()
        .filter(|i| blocks(i.class))
        .collect();
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
mod tests;
