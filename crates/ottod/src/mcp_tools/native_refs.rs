//! Friendly references on the native tools: the cross-workspace directory
//! list tools, the `(tool, arg, kind)` references resolved through
//! `/refs/resolve`, and the repo references resolved through
//! `/git/repos/resolve`. Moved out of `mcp_tools.rs` verbatim
//! (architecture-1-05).

use super::*;

/// Native list tools served by the daemon's cross-workspace directory
/// (`GET /refs/directory`, `otto_server::agent_refs`): every workspace the
/// session owner can read, each row annotated with its workspace, this
/// session's workspace first. `otto_list_connections` also uses it (then
/// keeps only the queryable DB kinds).
pub(super) const NATIVE_DIRECTORY_TOOLS: &[(&str, &str)] = &[
    ("otto_list_workspaces", "workspace"),
    ("otto_list_workflows", "workflow"),
    ("otto_list_goal_loops", "goal_loop"),
    ("otto_list_broker_clusters", "broker_cluster"),
    ("otto_list_swarms", "swarm"),
    ("otto_list_product_stories", "product_story"),
    ("otto_list_agent_rooms", "agent_room"),
    ("otto_list_issue_accounts", "issue_account"),
];

/// Native `(tool, argument, kind)` whose argument is a friendly reference —
/// the id, or a name / title / Jira key / label — resolved by the daemon
/// (`GET /refs/resolve`, as the session owner) before the call. An omitted
/// issue account resolves to the owner's only account.
pub(super) const NATIVE_REF_ARGS: &[(&str, &str, &str)] = &[
    ("otto_db_schema", "connection_id", "connection"),
    ("otto_db_children", "connection_id", "connection"),
    ("otto_db_object", "connection_id", "connection"),
    ("otto_db_query", "connection_id", "connection"),
    ("otto_list_broker_topics", "cluster_id", "broker_cluster"),
    ("otto_search_issues", "account_id", "issue_account"),
    ("otto_list_issue_transitions", "account_id", "issue_account"),
    ("otto_list_workflow_runs", "workflow_id", "workflow"),
    ("swarm_list_projects", "swarm_id", "swarm"),
    ("swarm_utilization", "swarm_id", "swarm"),
    ("otto_product_story", "story_id", "product_story"),
    ("otto_vault_dir", "vault_id", "vault"),
    ("otto_vault_read", "vault_id", "vault"),
    ("otto_vault_search", "vault_id", "vault"),
    ("otto_vault_backlinks", "vault_id", "vault"),
    ("otto_vault_tags", "vault_id", "vault"),
    ("otto_vault_graph", "vault_id", "vault"),
    ("otto_vault_okf_validate", "vault_id", "vault"),
    ("otto_vault_write", "vault_id", "vault"),
    ("otto_vault_write_file", "vault_id", "vault"),
    ("otto_vault_rename", "vault_id", "vault"),
    ("otto_vault_delete", "vault_id", "vault"),
    ("design_get", "artifact_id", "design_artifact"),
    ("design_links", "artifact_id", "design_artifact"),
    ("canvas_get_scene", "scene_id", "canvas_scene"),
    ("canvas_update_scene", "scene_id", "canvas_scene"),
    ("otto_room_post", "room_id", "agent_room"),
    ("otto_room_read", "room_id", "agent_room"),
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

/// The `/refs/directory` query for a native list tool. Pure — unit-tested.
pub(super) fn directory_path(kind: &str, args: &Value, session_ws: Option<&str>) -> String {
    let mut path = format!("/refs/directory?kind={}", seg(kind));
    path.push_str(&opt_query(args, &[("workspace_id", "workspace_id")]));
    if let Some(ws) = session_ws.filter(|s| !s.is_empty()) {
        path.push_str(&format!("&prefer_workspace_id={}", seg(ws)));
    }
    path
}

/// The references a native call needs resolved: `(arg, kind, reference)` —
/// a present value that is not an id, a workspace NAME, or an omitted issue
/// account (the owner's sole account). Pure — unit-tested.
pub(super) fn native_ref_plan(
    name: &str,
    args: &Value,
) -> Vec<(&'static str, &'static str, Option<String>)> {
    let text = |k: &str| -> Option<String> {
        match args.get(k)? {
            Value::String(v) if !v.trim().is_empty() => Some(v.trim().to_string()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        }
    };
    let mut plan = Vec::new();
    if let Some(ws) = text("workspace_id").filter(|w| !otto_server::agent_refs::looks_like_id(w)) {
        plan.push(("workspace_id", "workspace", Some(ws)));
    }
    for (tool, arg, kind) in NATIVE_REF_ARGS {
        if *tool != name {
            continue;
        }
        match text(arg) {
            Some(v) if !otto_server::agent_refs::looks_like_id(&v) => {
                plan.push((*arg, *kind, Some(v)))
            }
            None if *kind == "issue_account" => plan.push((*arg, *kind, None)),
            _ => {}
        }
    }
    plan
}

/// Resolve every friendly reference a native call carries (see
/// [`native_ref_plan`]) through `GET /refs/resolve`, returning the arguments
/// with canonical ids. An unknown / ambiguous reference errors with the
/// daemon's candidate listing.
pub(super) async fn resolve_native_refs(
    ctx: &Ctx,
    name: &str,
    args: &Value,
) -> Result<Option<Value>, String> {
    let plan = native_ref_plan(name, args);
    if plan.is_empty() {
        return Ok(None);
    }
    let mut out = args.clone();
    if !out.is_object() {
        out = json!({});
    }
    for (arg, kind, reference) in plan {
        let mut path = format!("/refs/resolve?kind={}&arg={}", seg(kind), seg(arg));
        if let Some(r) = &reference {
            path.push_str(&format!("&ref={}", seg(r)));
        }
        if let Some(ws) = ctx.workspace_id.as_deref().filter(|s| !s.is_empty()) {
            path.push_str(&format!("&prefer_workspace_id={}", seg(ws)));
        }
        let v = match ctx.get_json(&path).await {
            Ok(v) => v,
            // The daemon could not be reached at all: pass the reference
            // through untouched so the call's own argument checks — and then
            // its own request — report the real problem. A daemon ANSWER
            // (unknown / ambiguous reference) always surfaces.
            Err(e) if e.starts_with("request failed") || e.starts_with("upstream timeout") => {
                return Ok(None);
            }
            Err(e) => return Err(e),
        };
        let id = v["id"]
            .as_str()
            .ok_or_else(|| format!("{arg}: resolution returned no id"))?;
        out[arg] = if kind == "vault" {
            id.parse::<i64>()
                .map(|n| json!(n))
                .unwrap_or_else(|_| json!(id))
        } else {
            json!(id)
        };
    }
    Ok(Some(out))
}

/// Native tools whose `repo_id` is a friendly reference: resolved by the
/// daemon (`GET /git/repos/resolve`) before the call — id, name, local path or
/// remote, across EVERY workspace the session owner can read, falling back to
/// the session's own cwd when omitted. The governed `otto.*` git tools get the
/// same resolution server-side in the invoke choke point.
pub(super) const NATIVE_REPO_REF_TOOLS: &[&str] = &[
    "otto_git_pr_review",
    "otto_list_prs",
    "otto_get_pr",
    "otto_comment_pr",
];

/// The `/git/repos/resolve` query for a tool call's `repo_id` (+ optional
/// `workspace_id` filter). Pure, so the binding is unit-tested.
pub(super) fn resolve_repo_path(args: &Value) -> String {
    let q = opt_query(
        args,
        &[("ref", "repo_id"), ("workspace_id", "workspace_id")],
    );
    let q = q.trim_start_matches('&');
    if q.is_empty() {
        "/git/repos/resolve".to_string()
    } else {
        format!("/git/repos/resolve?{q}")
    }
}

/// Resolve `args.repo_id` (a friendly reference, or absent) to the canonical
/// Otto repo id via the daemon, returning the arguments with `repo_id`
/// rewritten. The daemon authorizes as the session owner (Git:View +
/// workspace Viewer), so a reference only ever lands on a repo the owner could
/// already list; an unknown/ambiguous reference errors with the candidates.
pub(super) async fn resolve_repo_arg(ctx: &Ctx, args: &Value) -> Result<Value, String> {
    let v = ctx.get_json(&resolve_repo_path(args)).await?;
    let id = v["repo"]["id"]
        .as_str()
        .ok_or("repo resolution returned no repo")?
        .to_string();
    let mut out = args.clone();
    out["repo_id"] = json!(id);
    Ok(out)
}
