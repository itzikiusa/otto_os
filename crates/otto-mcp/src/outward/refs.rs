//! Friendly references + the workspace pin: which governed arguments take a
//! friendly reference ([`REF_ARGS`]), which list tools the cross-workspace
//! directory serves ([`DIRECTORY_TOOLS`]), the pin probes, and the pure pin /
//! normalization verdicts. The lookups themselves (`agent_refs`) stay in
//! `otto-server`.

use otto_core::auth::McpScope;
use otto_core::domain::WorkspaceRole;
use otto_core::{Error, Id};
use serde_json::{json, Value};

use super::catalog::tool_is_mutating;
use super::exec::i64_lenient;
use super::ui_commands;

/// `(tool, argument, agent_refs kind)` — a governed argument that takes a
/// friendly reference (the id, or a name / title / Jira key / label) resolved
/// by `mcp_outward::fill_refs` (otto-server) through `agent_refs`. A `Workspace`-scoped kind resolves
/// within the call's `workspace_id` when it carries one, else across every
/// workspace the caller can read (and then fills in the object's own
/// `workspace_id` for the pin, the audit and the approval scope).
pub const REF_ARGS: &[(&str, &str, &str)] = &[
    ("query_db_readonly", "connection_id", "connection"),
    ("get_workflow", "workflow_id", "workflow"),
    ("list_workflow_runs", "workflow_id", "workflow"),
    ("run_workflow", "workflow_id", "workflow"),
    ("list_broker_topics", "cluster_id", "broker_cluster"),
    ("get_broker_topic", "cluster_id", "broker_cluster"),
    ("list_consumer_groups", "cluster_id", "broker_cluster"),
    ("consume_broker_messages", "cluster_id", "broker_cluster"),
    ("produce_broker_message", "cluster_id", "broker_cluster"),
    ("api_get_request", "request_id", "api_request"),
    ("api_history", "request_id", "api_request"),
    ("api_execute", "request_id", "api_request"),
    ("api_execute", "environment_id", "api_environment"),
    ("api_upsert_request", "request_id", "api_request"),
    ("api_run_automation", "automation_id", "api_automation"),
    ("search_issues", "account_id", "issue_account"),
    ("get_issue", "account_id", "issue_account"),
    ("list_issue_transitions", "account_id", "issue_account"),
    ("comment_issue", "account_id", "issue_account"),
    ("transition_issue", "account_id", "issue_account"),
    ("search_confluence", "account_id", "issue_account"),
    ("get_confluence_page", "account_id", "issue_account"),
    (
        "list_confluence_page_comments",
        "account_id",
        "issue_account",
    ),
    ("create_confluence_page", "account_id", "issue_account"),
    ("update_confluence_page", "account_id", "issue_account"),
    ("comment_confluence_page", "account_id", "issue_account"),
    ("start_pr_review", "issue_account_id", "issue_account"),
    ("get_swarm", "swarm_id", "swarm"),
    ("list_swarm_runs", "swarm_id", "swarm"),
    ("list_swarm_projects", "swarm_id", "swarm"),
    ("get_swarm_board", "swarm_id", "swarm"),
    ("post_swarm_board", "swarm_id", "swarm"),
    ("vault_dir", "vault_id", "vault"),
    ("vault_read", "vault_id", "vault"),
    ("vault_search", "vault_id", "vault"),
    ("vault_backlinks", "vault_id", "vault"),
    ("vault_tags", "vault_id", "vault"),
    ("vault_graph", "vault_id", "vault"),
    ("vault_okf_validate", "vault_id", "vault"),
    ("vault_write", "vault_id", "vault"),
    ("vault_write_file", "vault_id", "vault"),
    ("vault_rename", "vault_id", "vault"),
    ("vault_delete", "vault_id", "vault"),
    ("design_get", "artifact_id", "design_artifact"),
    ("design_links", "artifact_id", "design_artifact"),
    ("design_assist", "artifact_id", "design_artifact"),
    ("design_link", "artifact_id", "design_artifact"),
    ("get_product_story", "story_id", "product_story"),
    ("get_context_packet", "story_id", "product_story"),
    ("get_scheduled_task", "task_id", "scheduled_task"),
    ("list_scheduled_task_runs", "task_id", "scheduled_task"),
    ("update_scheduled_task", "task_id", "scheduled_task"),
    ("set_scheduled_task_enabled", "task_id", "scheduled_task"),
    ("run_scheduled_task", "task_id", "scheduled_task"),
    ("delete_scheduled_task", "task_id", "scheduled_task"),
    ("create_scheduled_task", "workflow_id", "workflow"),
    ("update_scheduled_task", "workflow_id", "workflow"),
    ("get_proof_pack", "goal_loop_id", "goal_loop"),
    ("room_post", "room_id", "agent_room"),
    ("room_read", "room_id", "agent_room"),
    ("aws_s3_list_buckets", "account_id", "aws_account"),
    ("aws_s3_list_objects", "account_id", "aws_account"),
    ("aws_s3_preview", "account_id", "aws_account"),
    ("aws_sqs_list_queues", "account_id", "aws_account"),
    ("aws_sqs_peek", "account_id", "aws_account"),
    ("aws_sqs_send", "account_id", "aws_account"),
    ("aws_ec2_list_instances", "account_id", "aws_account"),
    ("aws_athena_list_tables", "account_id", "aws_account"),
    ("aws_athena_query", "account_id", "aws_account"),
    ("aws_athena_get_query", "account_id", "aws_account"),
    ("aws_eks_list_clusters", "account_id", "aws_account"),
    ("aws_logs_list_groups", "account_id", "aws_account"),
    ("aws_logs_filter", "account_id", "aws_account"),
    ("aws_logs_insights", "account_id", "aws_account"),
    ("aws_logs_get_insights", "account_id", "aws_account"),
    ("k8s_get_resources", "cluster_id", "k8s_cluster"),
    ("k8s_describe", "cluster_id", "k8s_cluster"),
    ("k8s_logs", "cluster_id", "k8s_cluster"),
    ("k8s_top", "cluster_id", "k8s_cluster"),
    ("k8s_health", "cluster_id", "k8s_cluster"),
    ("k8s_action", "cluster_id", "k8s_cluster"),
    ("k8s_pod_http", "cluster_id", "k8s_cluster"),
    ("k8s_pod_actions_list", "cluster_id", "k8s_cluster"),
];

/// Governed list tools served by the cross-workspace `agent_refs` directory:
/// omit `workspace_id` to list every workspace the caller can read (a pinned
/// token: its pin), each row annotated with its workspace.
pub const DIRECTORY_TOOLS: &[(&str, &str)] = &[
    ("list_workspaces", "workspace"),
    ("list_workflows", "workflow"),
    ("list_connections", "connection"),
    ("list_broker_clusters", "broker_cluster"),
    ("list_swarms", "swarm"),
    ("list_scheduled_tasks", "scheduled_task"),
    ("list_goal_loops", "goal_loop"),
    ("list_agent_rooms", "agent_room"),
    ("list_issue_accounts", "issue_account"),
    ("list_product_stories", "product_story"),
];

/// Tools that address ONE design artifact: the artifact's own workspace is
/// filled in (`mcp_outward::fill_design_workspace` (otto-server)).
pub const DESIGN_WS_TOOLS: &[&str] =
    &["design_get", "design_links", "design_assist", "design_link"];

/// Pin probes for id-only objects: `(tool, arg, route prefix, JSON pointer of
/// the object's workspace)`. Run for EVERY call from a workspace-pinned token
/// (whose pin must be checked against the object's REAL workspace) — the
/// probed workspace overwrites any caller-supplied `workspace_id`, which the
/// executor ignores for these tools.
pub const PIN_PROBES: &[(&str, &str, &str, &str)] = &[
    (
        "get_workflow_run",
        "run_id",
        "/api/v1/workflow-runs/",
        "/workspace_id",
    ),
    (
        "cancel_workflow_run",
        "run_id",
        "/api/v1/workflow-runs/",
        "/workspace_id",
    ),
    (
        "get_session",
        "session_id",
        "/api/v1/sessions/",
        "/workspace_id",
    ),
    (
        "wait_session",
        "session_id",
        "/api/v1/sessions/",
        "/workspace_id",
    ),
    (
        "send_message",
        "session_id",
        "/api/v1/sessions/",
        "/workspace_id",
    ),
    (
        "get_improvement_run",
        "run_id",
        "/api/v1/improvement/runs/",
        "/run/workspace_id",
    ),
];

/// Tools that may run under a workspace pin WITHOUT a workspace — they
/// address no workspace-owned object: global rows (AWS accounts, K8s
/// clusters, the product-story and design libraries), the caller's own issue
/// accounts, root-only usage, the skill catalogue, and the directory tools
/// (which the pin itself narrows).
pub fn pin_global(tool: &str) -> bool {
    // UI-control tools act on the CALLING SESSION, whose workspace the bridge
    // checks against the pin itself (`ui_bridge::run`).
    ui_commands::is_ui_tool(tool)
        || tool.starts_with("aws_")
        || tool.starts_with("k8s_")
        || DIRECTORY_TOOLS.iter().any(|(t, _)| *t == tool)
        || matches!(
            tool,
            "search_issues"
                | "get_issue"
                | "list_issue_transitions"
                | "comment_issue"
                | "transition_issue"
                | "search_confluence"
                | "get_confluence_page"
                | "list_confluence_page_comments"
                | "create_confluence_page"
                | "update_confluence_page"
                | "comment_confluence_page"
                | "get_product_story"
                | "get_usage_summary"
                | "list_bundled_skills"
        )
}

/// Tools that narrow to a workspace when given one: a pinned token that omits
/// it is narrowed to its pin (never widened to the whole library).
pub const PIN_NARROW_TOOLS: &[&str] = &[
    "design_list",
    "design_search",
    "list_design_projects",
    "ask_human_approval",
];

/// Tools whose target's workspace Otto cannot establish from the arguments
/// (no workspace argument, no name to resolve, no probe route) — a
/// workspace-pinned token is DENIED them ([`pin_verdict`] enforces it,
/// whatever `workspace_id` the caller sends). Listed so the classification
/// test forces a conscious choice for every new tool.
pub const PIN_UNVERIFIABLE: &[&str] = &[
    "create_work_item",
    "list_swarm_tasks",
    "list_findings",
    "get_finding",
    "approve_improvement_edit",
    "reject_improvement_edit",
    "rollback_improvement_edit",
    // The Assistant's memory is the owner's, across every workspace — a
    // token pinned to one workspace must not read or rewrite it.
    "assistant_remember",
    "assistant_forget",
    "assistant_recall",
];

/// The workspace-pin verdict on fully resolved arguments: `McpScope`'s own
/// check first; then, for a pinned token, a call that names no workspace is
/// allowed only for a [`pin_global`] tool. Pure — unit-tested.
pub fn pin_verdict(scope: &McpScope, tool: &str, args: &Value) -> Option<String> {
    let ws = args
        .get("workspace_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    if let Some(reason) = scope.deny_reason(tool, tool_is_mutating(tool), ws) {
        return Some(format!("token scope: {reason}"));
    }
    let pin = scope.workspace_id.as_deref().filter(|s| !s.is_empty())?;
    // Enforced, not a classification: these tools never consume a
    // `workspace_id`, so a caller echoing the pin back proves nothing.
    if PIN_UNVERIFIABLE.contains(&tool) {
        return Some(format!(
            "token scope: this token is scoped to workspace '{pin}', and '{tool}' addresses \
             an object whose workspace Otto cannot verify — use a token without a workspace \
             pin for it"
        ));
    }
    if ws.is_none() && !pin_global(tool) {
        return Some(format!(
            "token scope: this token is scoped to workspace '{pin}', and Otto cannot \
             establish which workspace this '{tool}' call touches — use a token without a \
             workspace pin for it"
        ));
    }
    None
}

/// Canonical argument spellings, before anything hashes them: the PR tools'
/// `number` / `pr_number` aliases collapse to the one each route reads, and a
/// stringified PR number becomes an integer. `None` when nothing changed.
/// Pure — unit-tested.
pub fn normalize_args(tool: &str, args: &Value) -> Option<Value> {
    let (want, alias) = match tool {
        "get_pr" | "comment_pr" | "get_pr_checks" | "get_pr_diff" | "merge_pr" => {
            ("number", "pr_number")
        }
        "start_pr_review" | "list_pr_reviews" => ("pr_number", "number"),
        _ => return None,
    };
    let obj = args.as_object()?;
    let value = obj.get(want).or_else(|| obj.get(alias))?;
    let n = i64_lenient(value)?;
    if obj.get(want) == Some(&json!(n)) && !obj.contains_key(alias) {
        return None;
    }
    let mut out = obj.clone();
    out.remove(alias);
    out.insert(want.to_string(), json!(n));
    Some(Value::Object(out))
}

/// A Confluence page id from a page id or URL: `…/pages/12345/Title`,
/// `…/pages/12345`, `…?pageId=12345`. `None` when nothing id-like is found.
/// Pure — unit-tested.
pub fn confluence_page_id(reference: &str) -> Option<String> {
    let r = reference.trim();
    if !r.is_empty() && r.bytes().all(|b| b.is_ascii_digit()) {
        return Some(r.to_string());
    }
    if let Some(i) = r.find("pageId=") {
        let digits: String = r[i + 7..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        if !digits.is_empty() {
            return Some(digits);
        }
    }
    if let Some(i) = r.find("/pages/") {
        let digits: String = r[i + 7..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        if !digits.is_empty() {
            return Some(digits);
        }
    }
    None
}

/// Pick a Jira transition by id, name, or target status name
/// (case-insensitive) from `GET …/transitions` (`[{id, name, to_status}]`).
/// Unknown / ambiguous → an error listing the issue's transitions. Pure.
pub fn match_transition(list: &Value, wanted: &str) -> Result<String, Error> {
    let rows = list.as_array().map(Vec::as_slice).unwrap_or(&[]);
    let field = |r: &Value, k: &str| r.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let id_of = |r: &Value| match r.get("id") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    };
    let listing = || {
        rows.iter()
            .map(|r| {
                format!(
                    "- {}  {} → {}",
                    id_of(r),
                    field(r, "name"),
                    field(r, "to_status")
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    for key in ["name", "to_status"] {
        let hits: Vec<&Value> = rows
            .iter()
            .filter(|r| field(r, key).eq_ignore_ascii_case(wanted.trim()))
            .collect();
        match hits.len() {
            0 => continue,
            1 => return Ok(id_of(hits[0])),
            _ => {
                return Err(Error::Conflict(format!(
                "transition '{wanted}' matches {} transitions — pass one id as transition_id:\n{}",
                hits.len(),
                listing()
            )))
            }
        }
    }
    Err(Error::NotFound(format!(
        "no transition named '{wanted}' is available for this issue. Available:\n{}",
        listing()
    )))
}

/// Pick the workspace an omitted-`workspace_id` vault call is scoped to: the
/// first one whose role satisfies the tool (Editor for mutating, Viewer for
/// reads), falling back to the first membership so the self-call's native RBAC
/// produces the honest 403 rather than an "unknown workspace" here.
pub fn pick_vault_workspace(
    rows: &[(otto_core::domain::Workspace, WorkspaceRole)],
    mutating: bool,
) -> Option<Id> {
    let need = if mutating {
        WorkspaceRole::Editor
    } else {
        WorkspaceRole::Viewer
    };
    rows.iter()
        .find(|(_, role)| *role >= need)
        .or_else(|| rows.first())
        .map(|(w, _)| w.id.clone())
}
