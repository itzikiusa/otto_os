//! Native tool dispatch: [`run_tool`] and its per-domain parts, the pure
//! feature-read mapping [`read_route`], and the [`ROUTED_TWINS`] that call
//! the outward `route_for` (architecture-1-05). Moved out of `mcp_tools.rs`;
//! the routed twins replace their former native arms.

use super::*;

/// The new first-party feature read tools route through one pure mapping
/// ([`read_route`]); this is the set the dispatcher recognises before the
/// catch-all "unknown tool". All are reads (GET, or a read-only viewer POST).
pub(super) const FEATURE_READ_TOOLS: &[&str] = &[
    "otto_list_workflow_runs",
    "otto_get_workflow_run",
    "otto_list_broker_topics",
    "otto_search_issues",
    "otto_list_issue_transitions",
    "otto_search_memory",
    "otto_list_repos",
    "otto_list_prs",
    "otto_get_pr",
    "otto_list_sessions",
    "otto_get_session",
    "otto_wait_session",
    "otto_list_findings",
    "otto_usage_summary",
    "otto_list_improvement_runs",
    "otto_list_improvement_edits",
    "otto_vault_list",
    "otto_vault_dir",
    "otto_vault_read",
    "otto_vault_search",
    "otto_vault_backlinks",
    "otto_vault_tags",
    "otto_vault_graph",
    "otto_vault_okf_validate",
    // Design Hall reads (the global design library; find + cite earlier work).
    "design_list",
    "design_get",
    "design_links",
    "design_search",
    // AWS console reads. (`aws_sqs_peek` is NOT here: a receive-message bumps
    // each message's receive count — it can dead-letter — so it is Edit-gated
    // and dispatched by its own arm, like `aws_sqs_send`.)
    "aws_list_accounts",
    "aws_s3_list_buckets",
    "aws_s3_list_objects",
    "aws_s3_preview",
    "aws_sqs_list_queues",
    "aws_ec2_list_instances",
    "aws_athena_list_tables",
    "aws_athena_get_query",
    "aws_eks_list_clusters",
    "aws_logs_list_groups",
    "aws_logs_filter",
    "aws_logs_insights",
    "aws_logs_get_insights",
    // Kubernetes console reads (`k8s_logs` is text/plain and has its own arm).
    "k8s_list_clusters",
    "k8s_get_resources",
    "k8s_describe",
    "k8s_top",
    "k8s_health",
    "k8s_pod_actions_list",
];

/// How a [`ROUTED_TWINS`] call's arguments are bound to the calling session
/// before `route_for` — the native pre-step, after `resolve_native_refs`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum TwinScope {
    /// Nothing session-bound: the arguments go to `route_for` as given.
    Plain,
    /// `workspace_id` is the call's workspace: for a read the agent's
    /// (resolved) `workspace_id`, else this session's; for a write always this
    /// session's — the native writers never honoured the argument. No
    /// workspace at all errors as the native tools always have.
    Workspace,
    /// `session_id` is the CALLING session's own (from the spawn env, set by
    /// the daemon), never the agent's: the room routes resolve which personal
    /// agent is speaking from it and enforce membership. Session-less: none.
    CallingSession,
    /// `number` is the PR number, the native `pr_number` alias accepted.
    PrNumber,
}

/// How a routed twin's daemon answer is shaped for the agent — the native
/// post-step (before `finalize` caps and redacts it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum TwinReply {
    /// The daemon's answer as is.
    Raw,
    /// `{key: answer}`.
    Wrap(&'static str),
    /// `{"ok": true, key: answer}`.
    OkWith(&'static str),
    /// `{"ok": true, "trashed": <path>}` (a soft delete answers no body).
    Trashed,
}

/// Native twins (`otto_x` ⇄ `otto.x`) whose call IS the outward route:
/// `route_for` spells the route once (architecture-1-05). The session-bound
/// arguments are filled first ([`TwinScope`]), the answer is shaped after
/// ([`TwinReply`]). Reads also listed in `FEATURE_READ_TOOLS` go through
/// [`read_route`] (and its budgets); the rest through [`twin_call`].
///
/// Twins that stay native, and why (each keeps its own arm):
/// - the directory lists (`otto_list_workspaces`, `_workflows`,
///   `_goal_loops`, `_broker_clusters`, `_product_stories`,
///   `_agent_rooms`, `_issue_accounts`, `_connections`): served by the
///   cross-workspace `/refs/directory`, which the outward executor reaches
///   in-process (`otto_mcp::outward::refs::DIRECTORY_TOOLS`), not through a
///   `route_for` route; `otto_list_connections` also keeps only DB kinds;
/// - `otto_list_repos`: lists this session's workspace first
///   (`prefer_workspace_id`), which the outward route has no argument for;
/// - the API client (`otto_api_*`): resolves request / environment /
///   automation NAMES in this session's workspace and merges an upsert with
///   the stored request (and a pasted curl) before the write — `route_for`
///   takes canonical ids only, and `otto_api_execute` already runs through
///   the governed path.
pub(super) const ROUTED_TWINS: &[(&str, TwinScope, TwinReply)] = &[
    ("otto_get_session", TwinScope::Plain, TwinReply::Raw),
    ("otto_get_workflow_run", TwinScope::Plain, TwinReply::Raw),
    ("otto_list_findings", TwinScope::Plain, TwinReply::Raw),
    ("otto_list_prs", TwinScope::Plain, TwinReply::Raw),
    ("otto_list_workflow_runs", TwinScope::Plain, TwinReply::Raw),
    ("otto_list_broker_topics", TwinScope::Plain, TwinReply::Raw),
    ("otto_search_issues", TwinScope::Plain, TwinReply::Raw),
    (
        "otto_list_issue_transitions",
        TwinScope::Plain,
        TwinReply::Raw,
    ),
    ("otto_get_pr", TwinScope::PrNumber, TwinReply::Raw),
    ("otto_wait_session", TwinScope::Plain, TwinReply::Raw),
    ("otto_list_sessions", TwinScope::Workspace, TwinReply::Raw),
    (
        "otto_list_improvement_runs",
        TwinScope::Workspace,
        TwinReply::Raw,
    ),
    (
        "otto_list_improvement_edits",
        TwinScope::Workspace,
        TwinReply::Raw,
    ),
    ("otto_vault_list", TwinScope::Workspace, TwinReply::Raw),
    ("otto_vault_dir", TwinScope::Workspace, TwinReply::Raw),
    ("otto_vault_read", TwinScope::Workspace, TwinReply::Raw),
    ("otto_vault_search", TwinScope::Workspace, TwinReply::Raw),
    ("otto_vault_backlinks", TwinScope::Workspace, TwinReply::Raw),
    ("otto_vault_tags", TwinScope::Workspace, TwinReply::Raw),
    ("otto_vault_graph", TwinScope::Workspace, TwinReply::Raw),
    (
        "otto_vault_okf_validate",
        TwinScope::Workspace,
        TwinReply::Raw,
    ),
    // Vault v3 doc writers — Editor-gated by the daemon route; the delete is
    // a soft move into `.trash/`.
    (
        "otto_vault_write",
        TwinScope::Workspace,
        TwinReply::OkWith("meta"),
    ),
    (
        "otto_vault_write_file",
        TwinScope::Workspace,
        TwinReply::OkWith("file"),
    ),
    ("otto_vault_rename", TwinScope::Workspace, TwinReply::Raw),
    (
        "otto_vault_delete",
        TwinScope::Workspace,
        TwinReply::Trashed,
    ),
    // Personal-agent room tools. A session-less caller posts as the user.
    (
        "otto_room_post",
        TwinScope::CallingSession,
        TwinReply::Wrap("message"),
    ),
    (
        "otto_room_read",
        TwinScope::CallingSession,
        TwinReply::Wrap("messages"),
    ),
    ("otto_open_session", TwinScope::Workspace, TwinReply::Raw),
    ("otto_send_message", TwinScope::Plain, TwinReply::Raw),
    // PR comment — the one git WRITE on this surface. It lives here rather
    // than behind the gateway because the review workflow's whole product is
    // comments on a PR; the caller's own session token authorizes it.
    ("otto_comment_pr", TwinScope::PrNumber, TwinReply::Raw),
];

/// A routed twin's self-call: `/api/v1`-relative path, no-null body.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct TwinCall {
    pub(super) method: otto_server::mcp_outward::Method,
    pub(super) path: String,
    pub(super) body: Option<Value>,
    pub(super) reply: TwinReply,
}

/// The pre-step: `args` with the session-bound arguments of `scope` filled
/// in. `ws` is the call's workspace (see [`TwinScope::Workspace`]).
pub(super) fn twin_args(
    scope: TwinScope,
    args: &Value,
    ws: Option<&str>,
    session: Option<&str>,
) -> Result<Value, String> {
    let mut out = args.as_object().cloned().unwrap_or_default();
    match scope {
        TwinScope::Plain => {}
        TwinScope::Workspace => {
            let ws = ws
                .filter(|s| !s.is_empty())
                .ok_or("no workspace context (OTTO_WORKSPACE_ID unset)")?;
            out.insert("workspace_id".into(), json!(ws));
        }
        TwinScope::CallingSession => {
            out.remove("session_id");
            if let Some(sid) = session {
                out.insert("session_id".into(), json!(sid));
            }
        }
        TwinScope::PrNumber => {
            out.insert("number".into(), json!(arg_pr_number(args)?));
        }
    }
    Ok(Value::Object(out))
}

/// A `route_for` refusal in this surface's wording: an argument error reads
/// as the native tools' (`` `key` ``, no `invalid:` prefix).
fn twin_error(e: otto_core::Error) -> String {
    match e {
        otto_core::Error::Invalid(m) if m.starts_with("missing required") => m.replace('\'', "`"),
        otto_core::Error::Invalid(m) => m,
        other => other.to_string(),
    }
}

/// Every argument the twin's catalog schema types as a plain `string` is a
/// string when present: `route_for` reads them with `as_str` and would DROP a
/// mistyped one — an `if_hash: 7` would silently skip the vault's
/// optimistic-concurrency check — where the native tools always refused it.
fn check_string_args(short: &str, args: &Value) -> Result<(), String> {
    let full = format!("otto.{short}");
    let Some(spec) = GOVERNED_SPECS.iter().find(|s| s["name"] == full.as_str()) else {
        return Ok(());
    };
    let Some(props) = spec["inputSchema"]["properties"].as_object() else {
        return Ok(());
    };
    for (key, prop) in props {
        let mistyped = args
            .get(key)
            .is_some_and(|v| !v.is_null() && !v.is_string());
        if prop["type"] == "string" && mistyped {
            return Err(format!("argument `{key}` must be a string"));
        }
    }
    Ok(())
}

/// A [`ROUTED_TWINS`] call through `route_for` (pre-step applied), or `None`
/// when `name` is not routed. Pure — the dispatch table test pins it.
pub(super) fn twin_route(
    name: &str,
    args: &Value,
    ws: Option<&str>,
    session: Option<&str>,
) -> Option<Result<TwinCall, String>> {
    let &(_, scope, reply) = ROUTED_TWINS.iter().find(|(n, ..)| *n == name)?;
    let short = name.strip_prefix("otto_")?;
    Some(
        check_string_args(short, args)
            .and_then(|()| twin_args(scope, args, ws, session))
            .and_then(|args| {
                let call = otto_server::mcp_outward::route_for(short, &args).map_err(twin_error)?;
                let path = call
                    .path
                    .strip_prefix("/api/v1")
                    .ok_or_else(|| format!("{name}: the outward twin is not an /api/v1 route"))?
                    .to_string();
                // The native calls never sent an absent optional as `null`.
                let body = call.body.map(|mut b| {
                    if let Some(map) = b.as_object_mut() {
                        map.retain(|_, v| !v.is_null());
                    }
                    b
                });
                Ok(TwinCall {
                    method: call.method,
                    path,
                    body,
                    reply,
                })
            }),
    )
}

/// A routed READ (a `FEATURE_READ_TOOLS` twin) as a [`ReadCall`], with its
/// route budget (the `otto_wait_session` long-poll waits by design).
fn twin_read(name: &str, args: &Value, ws: Option<&str>) -> Option<Result<ReadCall, String>> {
    use otto_server::mcp_outward::Method;
    let call = twin_route(name, args, ws, None)?;
    Some(call.and_then(|c| {
        let read = match (c.method, c.body) {
            (Method::Get, _) => ReadCall::get(c.path),
            (Method::Post, body) => ReadCall::post(c.path, body.unwrap_or_else(|| json!({}))),
            _ => return Err(format!("{name}: the outward twin is not a read")),
        };
        Ok(if name == "otto_wait_session" {
            read.within(wait_session_budget(args))
        } else {
            read
        })
    }))
}

/// Whether `name` is a routed twin that [`twin_call`] dispatches (the routed
/// reads go through [`read_route`]).
pub(super) fn is_twin_call(name: &str) -> bool {
    !FEATURE_READ_TOOLS.contains(&name) && ROUTED_TWINS.iter().any(|(n, ..)| *n == name)
}

/// Run a routed twin that is not a plain read: the `route_for` self-call as
/// the session owner, then the native answer shape ([`TwinReply`]).
async fn twin_call(ctx: &Ctx, name: &str, args: &Value) -> Result<(Value, Option<i64>), String> {
    use otto_server::mcp_outward::Method;
    let call = twin_route(
        name,
        args,
        ctx.workspace_id.as_deref(),
        ctx.session_id.as_deref(),
    )
    .ok_or_else(|| format!("unknown tool `{name}`"))??;
    let body = call.body.unwrap_or_else(|| json!({}));
    let raw = match call.method {
        Method::Get => ctx.get_json(&call.path).await?,
        Method::Post => ctx.post_json(&call.path, &body).await?,
        Method::Put => ctx.put_json(&call.path, &body).await?,
        Method::Patch => ctx.patch_json(&call.path, &body).await?,
        Method::Delete => {
            ctx.delete_ok(&call.path).await?;
            Value::Null
        }
    };
    let mut out = serde_json::Map::new();
    let shaped = match call.reply {
        TwinReply::Raw => raw,
        TwinReply::Wrap(key) => {
            out.insert(key.into(), raw);
            Value::Object(out)
        }
        TwinReply::OkWith(key) => {
            out.insert("ok".into(), json!(true));
            out.insert(key.into(), raw);
            Value::Object(out)
        }
        TwinReply::Trashed => json!({ "ok": true, "trashed": args.get("path") }),
    };
    Ok(finalize(shaped))
}

/// Map a feature read tool + its arguments to the upstream daemon read. Pure: no
/// I/O. Workspace-scoped tools use the session's workspace (`ws`); others take an
/// explicit id argument. Every path is a GET or the read-only `/memory/search`
/// viewer POST — no tool here mutates.
pub(super) fn read_route(name: &str, args: &Value, ws: Option<&str>) -> Result<ReadCall, String> {
    if let Some(call) = twin_read(name, args, ws) {
        return call;
    }
    let ws_req = || {
        ws.filter(|s| !s.is_empty())
            .ok_or_else(|| "no workspace context (OTTO_WORKSPACE_ID unset)".to_string())
    };
    Ok(match name {
        "otto_search_memory" => {
            // `k` defaults to 0 server-side (MemoryQuery) → no hits; supply a useful default.
            let k = args.get("k").and_then(u64_lenient).unwrap_or(20);
            let body = json!({ "text": arg_str(args, "query")?, "k": k });
            ReadCall::post(
                format!("/workspaces/{}/memory/search", seg(ws_req()?)),
                body,
            )
        }
        // Every workspace the caller can read, this session's first (the
        // daemon also infers "current" from the session token); an explicit
        // `workspace_id` narrows it.
        "otto_list_repos" => {
            let mut q = opt_query(args, &[("workspace_id", "workspace_id")]);
            if let Some(ws) = ws.filter(|s| !s.is_empty()) {
                q.push_str(&format!("&prefer_workspace_id={}", seg(ws)));
            }
            let q = q.trim_start_matches('&');
            ReadCall::get(if q.is_empty() {
                "/git/repos/directory".to_string()
            } else {
                format!("/git/repos/directory?{q}")
            })
        }
        "otto_usage_summary" => {
            let q = opt_query(args, &[("days", "days"), ("otto_only", "otto_only")]);
            let q = q.trim_start_matches('&');
            ReadCall::get(if q.is_empty() {
                "/usage/summary".to_string()
            } else {
                format!("/usage/summary?{q}")
            })
        }
        // ---- AWS console (docs/design/aws-k8s-consoles.md §2). Accounts are
        // global rows (not workspace-scoped), so no `ws` here; `?region=` is the
        // per-call override every service route accepts (§1 "Auth injection").
        "aws_list_accounts" => ReadCall::get("/aws/accounts".to_string()),
        "aws_s3_list_buckets" => ReadCall::get(format!(
            "/aws/accounts/{}/s3/buckets?{}",
            seg(&arg_str(args, "account_id")?),
            opt_query(args, &[("region", "region")]).trim_start_matches('&')
        )),
        "aws_s3_list_objects" => ReadCall::get(format!(
            "/aws/accounts/{}/s3/buckets/{}/objects?{}",
            seg(&arg_str(args, "account_id")?),
            seg(&arg_str(args, "bucket")?),
            opt_query(
                args,
                &[
                    ("prefix", "prefix"),
                    ("token", "token"),
                    ("max", "max"),
                    ("region", "region")
                ]
            )
            .trim_start_matches('&')
        )),
        "aws_s3_preview" => ReadCall::get(format!(
            "/aws/accounts/{}/s3/buckets/{}/preview?key={}{}",
            seg(&arg_str(args, "account_id")?),
            seg(&arg_str(args, "bucket")?),
            seg(&arg_str(args, "key")?),
            opt_query(args, &[("max_bytes", "max_bytes"), ("region", "region")])
        )),
        "aws_sqs_list_queues" => ReadCall::get(format!(
            "/aws/accounts/{}/sqs/queues?{}",
            seg(&arg_str(args, "account_id")?),
            opt_query(args, &[("prefix", "prefix"), ("region", "region")]).trim_start_matches('&')
        )),
        "aws_sqs_peek" => {
            // `receive-message --visibility-timeout 0`: nothing is hidden or
            // deleted, but the receive count rises (Edit-gated server-side).
            let mut body = json!({ "url": arg_str(args, "url")?, "visibility_timeout": 0 });
            if let Some(max) = args.get("max").and_then(u64_lenient) {
                body["max"] = json!(max.clamp(1, 10));
            }
            ReadCall::post(
                format!(
                    "/aws/accounts/{}/sqs/queues/peek?{}",
                    seg(&arg_str(args, "account_id")?),
                    opt_query(args, &[("region", "region")]).trim_start_matches('&')
                ),
                body,
            )
        }
        "aws_ec2_list_instances" => ReadCall::get(format!(
            "/aws/accounts/{}/ec2/instances?{}",
            seg(&arg_str(args, "account_id")?),
            opt_query(
                args,
                &[("region", "region"), ("state", "state"), ("q", "q")]
            )
            .trim_start_matches('&')
        )),
        "aws_athena_list_tables" => ReadCall::get(format!(
            "/aws/accounts/{}/athena/tables?database={}{}",
            seg(&arg_str(args, "account_id")?),
            seg(&arg_str(args, "database")?),
            opt_query(args, &[("catalog", "catalog"), ("region", "region")])
        )),
        "aws_athena_get_query" => ReadCall::get(format!(
            "/aws/accounts/{}/athena/query/{}?{}",
            seg(&arg_str(args, "account_id")?),
            seg(&arg_str(args, "query_execution_id")?),
            opt_query(
                args,
                &[("token", "token"), ("max", "max"), ("region", "region")]
            )
            .trim_start_matches('&')
        )),
        "aws_eks_list_clusters" => ReadCall::get(format!(
            "/aws/accounts/{}/eks/clusters?{}",
            seg(&arg_str(args, "account_id")?),
            opt_query(args, &[("region", "region")]).trim_start_matches('&')
        )),
        "aws_logs_list_groups" => ReadCall::get(format!(
            "/aws/accounts/{}/logs/groups?{}",
            seg(&arg_str(args, "account_id")?),
            opt_query(
                args,
                &[
                    ("prefix", "prefix"),
                    ("token", "token"),
                    ("max", "max"),
                    ("region", "region")
                ]
            )
            .trim_start_matches('&')
        )),
        "aws_logs_filter" => ReadCall::get(format!(
            "/aws/accounts/{}/logs/events?group={}{}",
            seg(&arg_str(args, "account_id")?),
            seg(&arg_str(args, "group")?),
            opt_query(
                args,
                &[
                    ("pattern", "pattern"),
                    ("streams", "streams"),
                    ("start", "start"),
                    ("end", "end"),
                    ("token", "token"),
                    ("max", "max"),
                    ("region", "region")
                ]
            )
        )),
        "aws_logs_insights" => {
            // Read-only POST: `logs start-query` reads (bills per GB scanned);
            // the policy table grades every `/logs/*` route Aws:View.
            let mut body = json!({
                "groups": args.get("groups").cloned().unwrap_or(json!([])),
                "query": arg_str(args, "query")?,
                "start": args.get("start").and_then(|v| v.as_i64()).unwrap_or(0),
                "end": args.get("end").and_then(|v| v.as_i64()).unwrap_or(0),
            });
            if let Some(limit) = args.get("limit").and_then(u64_lenient) {
                body["limit"] = json!(limit);
            }
            ReadCall::post(
                format!(
                    "/aws/accounts/{}/logs/insights?{}",
                    seg(&arg_str(args, "account_id")?),
                    opt_query(args, &[("region", "region")]).trim_start_matches('&')
                ),
                body,
            )
        }
        "aws_logs_get_insights" => ReadCall::get(format!(
            "/aws/accounts/{}/logs/insights/{}?{}",
            seg(&arg_str(args, "account_id")?),
            seg(&arg_str(args, "query_id")?),
            opt_query(args, &[("region", "region")]).trim_start_matches('&')
        )),
        // ---- Kubernetes console (§3). `namespace` → `ns` query param; omitted
        // ⇒ the route's all-namespaces default (`-A`).
        "k8s_list_clusters" => ReadCall::get("/k8s/clusters".to_string()),
        "k8s_get_resources" => ReadCall::get(format!(
            "/k8s/clusters/{}/resources?kind={}{}",
            seg(&arg_str(args, "cluster_id")?),
            seg(&arg_str(args, "kind")?),
            opt_query(args, &[("ns", "namespace"), ("label", "label"), ("q", "q")])
        )),
        // `ns` omitted for cluster-scoped kinds (nodes, namespaces).
        "k8s_describe" => ReadCall::get(format!(
            "/k8s/clusters/{}/resource?kind={}&name={}{}",
            seg(&arg_str(args, "cluster_id")?),
            seg(&arg_str(args, "kind")?),
            seg(&arg_str(args, "name")?),
            opt_query(args, &[("ns", "namespace")])
        )),
        "k8s_health" => ReadCall::get(format!(
            "/k8s/clusters/{}/monitor/health?{}",
            seg(&arg_str(args, "cluster_id")?),
            opt_query(args, &[("window", "window")]).trim_start_matches('&')
        )),
        "k8s_pod_actions_list" => ReadCall::get(format!(
            "/k8s/clusters/{}/pod-actions?{}",
            seg(&arg_str(args, "cluster_id")?),
            opt_query(
                args,
                &[
                    ("namespace", "namespace"),
                    ("workload_kind", "workload_kind"),
                    ("workload", "workload")
                ]
            )
            .trim_start_matches('&')
        )),
        "k8s_top" => ReadCall::get(format!(
            "/k8s/clusters/{}/metrics?{}",
            seg(&arg_str(args, "cluster_id")?),
            opt_query(args, &[("ns", "namespace")]).trim_start_matches('&')
        )),
        // ---- Design Hall (global library; `workspace_id` only narrows) ----
        "design_list" => {
            let q = opt_query(
                args,
                &[
                    ("workspace_id", "workspace_id"),
                    ("project_id", "project_id"),
                    ("studio", "studio"),
                    ("format", "format"),
                    ("status", "status"),
                    ("story_id", "story_id"),
                    ("limit", "limit"),
                    ("cursor", "cursor"),
                ],
            );
            let q = q.trim_start_matches('&');
            ReadCall::get(if q.is_empty() {
                "/design/artifacts".to_string()
            } else {
                format!("/design/artifacts?{q}")
            })
        }
        "design_get" => {
            let content = args
                .get("include_content")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            ReadCall::get(format!(
                "/design/artifacts/{}?content={content}{}",
                seg(&arg_str(args, "artifact_id")?),
                opt_query(args, &[("version", "version")])
            ))
        }
        "design_links" => ReadCall::get(format!(
            "/design/artifacts/{}/links?dir={}",
            seg(&arg_str(args, "artifact_id")?),
            seg(args
                .get("dir")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or("both"))
        )),
        "design_search" => ReadCall::get(format!(
            "/design/search?q={}{}",
            seg(&arg_str(args, "query")?),
            opt_query(
                args,
                &[
                    ("workspace_id", "workspace_id"),
                    ("studio", "studio"),
                    ("format", "format"),
                    ("status", "status"),
                    ("story_id", "story_id"),
                    ("project_id", "project_id"),
                    ("limit", "limit"),
                ],
            )
        )),
        other => return Err(format!("unknown feature read tool `{other}`")),
    })
}

/// Run one tool by name. Returns the capped+redacted result `Value` and the
/// audited row count, or an error string surfaced to the agent.
pub(super) async fn run_tool(
    ctx: &Ctx,
    name: &str,
    args: &Value,
) -> Result<(Value, Option<i64>), String> {
    // Git tools: turn a friendly repo reference (or none → this session's
    // repo) into the canonical id before any arm builds a `/repos/{id}` path.
    let resolved;
    let args = if NATIVE_REPO_REF_TOOLS.contains(&name) {
        resolved = resolve_repo_arg(ctx, args).await?;
        &resolved
    } else {
        args
    };
    // Every other friendly reference (a connection / cluster / account /
    // vault / workflow NAME, a workspace name, an omitted sole issue account)
    // → canonical ids, via the daemon's `/refs/resolve` as the session owner.
    let refs = resolve_native_refs(ctx, name, args).await?;
    let args = refs.as_ref().unwrap_or(args);
    match name {
        // API-client reads and writes are thin wrappers over the masked,
        // workspace-scoped daemon routes (`run_api_tool`).
        "otto_api_list"
        | "otto_api_get_request"
        | "otto_api_history"
        | "otto_api_execute"
        | "otto_api_upsert_request"
        | "otto_api_run_automation" => run_api_tool(ctx, name, args).await,
        // Database Explorer reads, PR reviews and product stories (`run_data_tool`).
        "otto_list_connections"
        | "otto_db_schema"
        | "otto_db_children"
        | "otto_db_object"
        | "otto_db_query"
        | "otto_git_pr_review"
        | "otto_product_story" => run_data_tool(ctx, name, args).await,
        // Canvas scenes and the workbench (`run_canvas_tool`).
        "canvas_list_scenes"
        | "canvas_get_scene"
        | "canvas_create_scene"
        | "canvas_update_scene"
        | "workbench_get"
        | "workbench_list"
        | "workbench_write" => run_canvas_tool(ctx, name, args).await,
        // Browser tools — all call the governed browser routes with the
        // session's own token (netguard + RBAC enforced there, same as the
        // Browser module UI). `browser_navigate` is the only write: it opens
        // a reader-mode tab (a genuine new `browser_tabs` row), so it's an
        // explicit arm rather than a `FEATURE_READ_TOOLS` GET/POST mapping.
        "browser_navigate" | "browser_page" | "browser_query" | "browser_summarize"
        | "browser_marks" | "browser_login" => run_browser_tool(ctx, name, args).await,
        // Native twins whose call IS the outward route (`ROUTED_TWINS`).
        name if is_twin_call(name) => twin_call(ctx, name, args).await,
        // ---- AWS / Kubernetes console (docs/design/aws-k8s-consoles.md §6).
        // `k8s_logs` is the one text/plain read; the three writers below hit the
        // per-feature Edit-gated console routes as the session owner.
        "k8s_logs" => {
            let (text, truncated) = ctx.get_text(&k8s_logs_path(args)?).await?;
            let lines = text.lines().count() as i64;
            let (v, _) = finalize(json!({ "text": text, "truncated": truncated }));
            Ok((v, Some(lines)))
        }
        "aws_sqs_peek" => {
            // Edit-gated server-side: the receive bumps the receive count.
            let call = read_route("aws_sqs_peek", args, None)?;
            let raw = ctx
                .post_json(&call.path, call.body.as_ref().unwrap_or(&json!({})))
                .await?;
            Ok(finalize(raw))
        }
        "aws_athena_query" => {
            let acc = arg_str(args, "account_id")?;
            let mut body = json!({ "sql": arg_str(args, "sql")? });
            for k in ["database", "workgroup", "output_location"] {
                if let Some(v) = arg_optional_string(args, k)?.filter(|s| !s.is_empty()) {
                    body[k] = json!(v);
                }
            }
            let raw = ctx
                .post_json(
                    &format!(
                        "/aws/accounts/{}/athena/query?{}",
                        seg(&acc),
                        opt_query(args, &[("region", "region")]).trim_start_matches('&')
                    ),
                    &body,
                )
                .await?;
            Ok(finalize(raw))
        }
        "aws_sqs_send" => {
            // Required arguments checked here for a fast, clear error; the
            // send runs through the governed path (approval + guardrail +
            // audit), whose executor builds the same route body.
            for k in ["account_id", "url", "body"] {
                arg_str(args, k)?;
            }
            Ok(finalize(
                governed_native_write(ctx, "aws_sqs_send", args, Duration::from_secs(30)).await?,
            ))
        }
        "k8s_action" => {
            for k in ["cluster_id", "action", "kind", "namespace", "name"] {
                arg_str(args, k)?;
            }
            // The governed executor forwards `params` verbatim: the route owns
            // the confirm_name / replicas / prune / revision semantics.
            Ok(finalize(
                governed_native_write(ctx, "k8s_action", args, Duration::from_secs(75)).await?,
            ))
        }
        "k8s_pod_http" => {
            for k in ["cluster_id", "namespace", "method", "path"] {
                arg_str(args, k)?;
            }
            args.get("port")
                .and_then(u64_lenient)
                .ok_or("missing argument 'port'")?;
            // The governed executor forwards the rest verbatim: the route owns
            // validation and the prod confirm_name guard.
            Ok(finalize(
                governed_native_write(ctx, "k8s_pod_http", args, Duration::from_secs(90)).await?,
            ))
        }
        name if NATIVE_DIRECTORY_TOOLS.iter().any(|(t, _)| *t == name) => {
            let kind = NATIVE_DIRECTORY_TOOLS
                .iter()
                .find(|(t, _)| *t == name)
                .map(|(_, k)| *k)
                .unwrap_or_default();
            let raw = ctx
                .get_json(&directory_path(kind, args, ctx.workspace_id.as_deref()))
                .await?;
            Ok(finalize(raw))
        }
        name if FEATURE_READ_TOOLS.contains(&name) => {
            // An explicit (already resolved) `workspace_id` picks another
            // workspace for the workspace-scoped reads; default: this session's.
            let ws = args
                .get("workspace_id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .or(ctx.workspace_id.as_deref());
            let call = read_route(name, args, ws)?;
            let raw = if call.post {
                ctx.post_json_within(
                    &call.path,
                    call.body.as_ref().unwrap_or(&json!({})),
                    call.budget,
                )
                .await?
            } else {
                ctx.get_json_within(&call.path, call.budget).await?
            };
            Ok(finalize(raw))
        }
        other => Err(format!("unknown tool `{other}`")),
    }
}

/// The API-client tools of [`run_tool`] (split out to keep it under the
/// function-size ratchet).
async fn run_api_tool(ctx: &Ctx, name: &str, args: &Value) -> Result<(Value, Option<i64>), String> {
    match name {
        "otto_api_list" => {
            let base = api_base(ctx)?;
            let query = opt_query(
                args,
                &[
                    ("q", "q"),
                    ("collection_id", "collection_id"),
                    ("kind", "kind"),
                ],
            );
            let path = if query.is_empty() {
                format!("{base}/overview")
            } else {
                format!("{base}/overview?{}", query.trim_start_matches('&'))
            };
            Ok(finalize(ctx.get_json(&path).await?))
        }
        "otto_api_get_request" => {
            let base = api_base(ctx)?;
            let request_id = api_object_id(ctx, &base, args, ApiRef::REQUEST, true).await?;
            let raw = ctx
                .get_json(&format!("{base}/requests/{}?shape=agent", seg(&request_id)))
                .await?;
            Ok(finalize(raw))
        }
        "otto_api_history" => {
            let base = api_base(ctx)?;
            if let Some(id) = arg_optional_string(args, "id")?.filter(|id| !id.is_empty()) {
                let raw = ctx
                    .get_json(&format!("{base}/history/{}", seg(&id)))
                    .await?;
                return Ok(finalize(raw));
            }
            let limit = if args.get("limit").is_some() {
                arg_u64(args, "limit")?.clamp(1, 100)
            } else {
                25
            };
            // The scalar summary projection (method/url/status/duration/
            // source) — the list never needs bodies; `id` returns one whole
            // entry (perf F9: up to 100 × 64 KB responses into the context).
            let mut path = format!("{base}/history/summaries?limit={limit}");
            // The `request_id` filter takes an id or a unique name too.
            let resolved;
            let args = if arg_optional_string(args, "request_id")?.is_some_and(|r| !r.is_empty()) {
                let mut a = args.clone();
                a["request_id"] =
                    json!(api_object_id(ctx, &base, args, ApiRef::REQUEST, false).await?);
                resolved = a;
                &resolved
            } else {
                args
            };
            path.push_str(&opt_query(
                args,
                &[
                    ("q", "q"),
                    ("status", "status"),
                    ("request_id", "request_id"),
                    ("source", "source"),
                ],
            ));
            Ok(finalize(ctx.get_json(&path).await?))
        }
        "otto_api_execute" => {
            let base = api_base(ctx)?;
            if let Some(vars) = args.get("vars") {
                check_override_vars(vars)?;
            }
            let request_id = api_object_id(ctx, &base, args, ApiRef::REQUEST, true).await?;

            let mut body = json!({
                "workspace_id": ctx.workspace_id.clone(),
                "request_id": request_id,
            });
            // `environment_id` (the catalog's name; id or name) — the legacy
            // native `environment` still works.
            let environment = match arg_optional_string(args, "environment_id")?
                .filter(|environment| !environment.is_empty())
            {
                Some(e) => Some(e),
                None => arg_optional_string(args, "environment")?,
            };
            if let Some(environment) = environment.filter(|environment| !environment.is_empty()) {
                let environments = ctx.get_json(&format!("{base}/environments")).await?;
                let environments = environments
                    .as_array()
                    .ok_or("daemon response missing environments array")?;
                let environment_id = if let Some(found) = environments.iter().find(|item| {
                    item.get("id").and_then(Value::as_str) == Some(environment.as_str())
                }) {
                    found
                        .get("id")
                        .and_then(Value::as_str)
                        .ok_or("matched environment has no id")?
                        .to_string()
                } else {
                    let overview = ctx
                        .get_json(&format!(
                            "{base}/overview?kind=environments&q={}",
                            seg(&environment)
                        ))
                        .await?;
                    let environments = overview
                        .get("environments")
                        .and_then(Value::as_array)
                        .ok_or("daemon response missing environments array")?;
                    pick_by_name(environments, &environment, "environment")?
                        .get("id")
                        .and_then(Value::as_str)
                        .ok_or("matched environment has no id")?
                        .to_string()
                };
                body["environment_id"] = json!(environment_id);
            }
            for key in [
                "vars",
                "timeout_ms",
                "confirm",
                "confirm_new_host",
                "decode_jwt",
            ] {
                if let Some(value) = args.get(key) {
                    body[key] = value.clone();
                }
            }
            // The request itself may wait up to its `timeout_ms` (≤ 60 s).
            let budget_ms = args
                .get("timeout_ms")
                .and_then(u64_lenient)
                .unwrap_or(30_000)
                .min(60_000);
            // Resolved here (names → ids in this session's workspace), executed
            // through the governed path: approval + guardrail + audit.
            Ok(finalize(
                governed_native_write(
                    ctx,
                    "api_execute",
                    &body,
                    Duration::from_millis(budget_ms) + Duration::from_secs(15),
                )
                .await?,
            ))
        }
        "otto_api_upsert_request" => {
            let base = api_base(ctx)?;
            // `request_id` is the id or unique name to update; `name` is the
            // request's (new) name, never a lookup key here.
            let update_id =
                match arg_optional_string(args, "request_id")?.filter(|id| !id.is_empty()) {
                    Some(_) => Some(api_object_id(ctx, &base, args, ApiRef::REQUEST, false).await?),
                    None => None,
                };
            // Update: the stored row (raw, as the owner — only ever sent back
            // in the PATCH, never to the agent) supplies every omitted field.
            let stored = match &update_id {
                Some(id) => Some(
                    ctx.get_json(&format!("{base}/requests/{}", seg(id)))
                        .await?,
                ),
                None => {
                    arg_str(args, "name")?;
                    None
                }
            };
            let parsed_curl =
                match arg_optional_string(args, "curl")?.filter(|curl| !curl.is_empty()) {
                    Some(curl) => Some(
                        ctx.post_json("/api-client/import-curl", &json!({ "curl": curl }))
                            .await?,
                    ),
                    None => None,
                };

            let mut effective_args = args.clone();
            if args.get("collection_id").is_none() {
                if let Some(collection_name) =
                    arg_optional_string(args, "collection_name")?.filter(|name| !name.is_empty())
                {
                    let overview = ctx
                        .get_json(&format!(
                            "{base}/overview?kind=requests&q={}",
                            seg(&collection_name)
                        ))
                        .await?;
                    let collections = overview
                        .get("collections")
                        .and_then(Value::as_array)
                        .ok_or("daemon response missing collections array")?;
                    let collection_id =
                        match pick_by_name(collections, &collection_name, "collection") {
                            Ok(collection) => collection
                                .get("id")
                                .and_then(Value::as_str)
                                .ok_or("matched collection has no id")?
                                .to_string(),
                            Err(error)
                                if error == format!("no collection named '{collection_name}'") =>
                            {
                                let collection = ctx
                                    .post_json(
                                        &format!("{base}/collections"),
                                        &json!({ "name": collection_name }),
                                    )
                                    .await?;
                                collection
                                    .get("id")
                                    .and_then(Value::as_str)
                                    .ok_or("created collection has no id")?
                                    .to_string()
                            }
                            Err(error) => return Err(error),
                        };
                    effective_args["collection_id"] = json!(collection_id);
                }
            }
            let body = merge_upsert(&effective_args, parsed_curl.as_ref(), stored.as_ref());
            let saved = if let Some(request_id) = update_id {
                ctx.patch_json(&format!("{base}/requests/{}", seg(&request_id)), &body)
                    .await?
            } else {
                ctx.post_json(&format!("{base}/requests"), &body).await?
            };
            let request_id = saved
                .get("id")
                .and_then(Value::as_str)
                .ok_or("saved request has no id")?;
            let raw = ctx
                .get_json(&format!("{base}/requests/{}?shape=agent", seg(request_id)))
                .await?;
            Ok(finalize(raw))
        }
        "otto_api_run_automation" => {
            let base = api_base(ctx)?;
            let automation_id = api_object_id(ctx, &base, args, ApiRef::AUTOMATION, true).await?;
            Ok(finalize(
                governed_native_write(
                    ctx,
                    "api_run_automation",
                    &json!({"workspace_id": ctx.workspace_id.clone(),
                            "automation_id": automation_id}),
                    Duration::from_secs(180),
                )
                .await?,
            ))
        }
        other => Err(format!("unknown tool `{other}`")),
    }
}

/// The Database Explorer reads, PR-review and product-story tools of
/// [`run_tool`].
async fn run_data_tool(
    ctx: &Ctx,
    name: &str,
    args: &Value,
) -> Result<(Value, Option<i64>), String> {
    match name {
        "otto_list_connections" => {
            // Every workspace the owner can read (a connection is registered
            // in ONE workspace, not necessarily this session's), this
            // session's first — the daemon's cross-workspace directory.
            let raw = ctx
                .get_json(&directory_path(
                    "connection",
                    args,
                    ctx.workspace_id.as_deref(),
                ))
                .await?;
            // Keep only queryable DB kinds, optionally one kind, and slim each row
            // so the agent sees ids/names/kinds without connection params/secrets.
            let kind_filter = args.get("kind").and_then(Value::as_str);
            let items: Vec<Value> = raw["items"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or(&[])
                .iter()
                .filter_map(|c| {
                    let kind = c.get("kind").and_then(Value::as_str).unwrap_or("");
                    if !matches!(
                        kind,
                        "mysql" | "postgres" | "redis" | "mongodb" | "clickhouse"
                    ) {
                        return None;
                    }
                    if kind_filter.is_some_and(|kf| kf != kind) {
                        return None;
                    }
                    Some(json!({
                        "id": c.get("id").cloned().unwrap_or(Value::Null),
                        "name": c.get("name").cloned().unwrap_or(Value::Null),
                        "kind": kind,
                        "environment": c.get("environment").cloned().unwrap_or(Value::Null),
                        "read_only": c.get("read_only").cloned().unwrap_or(Value::Null),
                        "workspace_id": c.get("workspace_id").cloned().unwrap_or(Value::Null),
                        "workspace_name": c.get("workspace_name").cloned().unwrap_or(Value::Null),
                    }))
                })
                .collect();
            Ok(finalize(json!({ "connections": items })))
        }
        "otto_db_schema" => {
            let conn = arg_str(args, "connection_id")?;
            let raw = ctx
                .get_json(&format!("/connections/{}/db/schema", seg(&conn)))
                .await?;
            Ok(finalize(json!({ "connection_id": conn, "schema": raw })))
        }
        "otto_db_children" => {
            let conn = arg_str(args, "connection_id")?;
            let path = arg_str(args, "path")?;
            let mut body = json!({ "path": path });
            if let Some(f) = args
                .get("filter")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                body["filter"] = json!(f);
            }
            let raw = ctx
                .post_json(
                    &format!("/connections/{}/db/schema/children", seg(&conn)),
                    &body,
                )
                .await?;
            Ok(finalize(
                json!({ "connection_id": conn, "path": path, "children": raw }),
            ))
        }
        "otto_db_object" => {
            let conn = arg_str(args, "connection_id")?;
            let path = arg_str(args, "path")?;
            let body = json!({ "path": path });
            let raw = ctx
                .post_json(&format!("/connections/{}/db/object", seg(&conn)), &body)
                .await?;
            Ok(finalize(
                json!({ "connection_id": conn, "path": path, "object": raw }),
            ))
        }
        "otto_db_query" => {
            let conn = arg_str(args, "connection_id")?;
            let statement = arg_str(args, "statement")?;
            let mut body = json!({ "statement": statement });
            // `node` (raw) wins over `database`. The active-DB `node` is a PLAIN
            // name for SQL/Mongo (e.g. "shopdb" → `USE shopdb`); Redis selects a
            // keyspace via a raw `node` like "kdb:0".
            if let Some(node) = args
                .get("node")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                body["node"] = json!(node);
            } else if let Some(db) = args
                .get("database")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                body["node"] = json!(db);
            }
            if let Some(mr) = args.get("max_rows").and_then(u64_lenient) {
                body["max_rows"] = json!(mr);
            }
            // POSTs to the read-only-enforced endpoint: any write/DDL is refused
            // server-side (see otto-dbviewer `run_read_only`) before a driver runs.
            let raw = ctx
                .post_json(&format!("/connections/{}/db/mcp-query", seg(&conn)), &body)
                .await?;
            Ok(finalize(json!({ "connection_id": conn, "result": raw })))
        }
        "otto_git_pr_review" => {
            let repo = arg_str(args, "repo_id")?;
            let number = arg_i64(args, "pr_number")?;
            // The PR record and its reviews are two read endpoints; bundle them.
            let pr = ctx
                .get_json(&format!("/repos/{}/prs/{}", seg(&repo), number))
                .await?;
            // Reviews are best-effort: a repo/provider without review data should
            // still return the PR. A failure here yields an empty review list.
            let reviews = ctx
                .get_json(&format!("/repos/{}/prs/{}/reviews", seg(&repo), number))
                .await
                .unwrap_or(Value::Array(vec![]));
            Ok(finalize(json!({
                "repo_id": repo,
                "pr_number": number,
                "pull_request": pr,
                "reviews": reviews,
            })))
        }
        "otto_product_story" => {
            let story = arg_str(args, "story_id")?;
            let story_rec = ctx
                .get_json(&format!("/product/stories/{}", seg(&story)))
                .await?;
            // The agent-ready inject bundle is optional context; tolerate absence.
            let inject = ctx
                .get_json(&format!("/product/stories/{}/inject", seg(&story)))
                .await
                .unwrap_or(Value::Null);
            Ok(finalize(json!({
                "story_id": story,
                "story": story_rec,
                "inject": inject,
            })))
        }
        other => Err(format!("unknown tool `{other}`")),
    }
}

/// The canvas and workbench tools of [`run_tool`].
async fn run_canvas_tool(
    ctx: &Ctx,
    name: &str,
    args: &Value,
) -> Result<(Value, Option<i64>), String> {
    match name {
        "canvas_list_scenes" => {
            let ws = match arg_optional_string(args, "workspace_id")?.filter(|s| !s.is_empty()) {
                Some(ws) => ws,
                None => ctx
                    .workspace_id
                    .clone()
                    .ok_or("no workspace context (OTTO_WORKSPACE_ID unset); pass `workspace_id`")?,
            };
            let scenes = ctx
                .get_json(&format!("/workspaces/{}/canvas/scenes", seg(&ws)))
                .await?;
            Ok(finalize(json!({ "workspace_id": ws, "scenes": scenes })))
        }
        "canvas_get_scene" => {
            let scene = arg_str(args, "scene_id")?;
            let raw = ctx
                .get_json(&format!("/canvas/scenes/{}", seg(&scene)))
                .await?;
            Ok(finalize(json!({ "scene_id": scene, "scene": raw })))
        }
        "canvas_create_scene" => {
            let Some(ws) = ctx.workspace_id.clone() else {
                return Err(
                    "no workspace context (OTTO_WORKSPACE_ID unset); cannot create a scene".into(),
                );
            };
            let title = arg_str(args, "title")?;
            let format = args
                .get("format")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or("mermaid");
            if !matches!(format, "mermaid" | "d2" | "excalidraw") {
                return Err(format!(
                    "invalid format `{format}` — must be one of: mermaid | d2 | excalidraw"
                ));
            }
            let source = match args
                .get("source")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                Some(s) => s.to_string(),
                None if format == "excalidraw" => {
                    json!({ "type": "excalidraw", "version": 2, "source": "otto", "elements": [] })
                        .to_string()
                }
                None => String::new(),
            };
            let mut body = json!({
                "title": title,
                "doc": { "type": "otto-canvas", "version": 1, "format": format, "source": source },
            });
            if let Some(section) = args
                .get("section")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                body["section"] = json!(section);
            }
            let created = ctx
                .post_json(&format!("/workspaces/{}/canvas/scenes", seg(&ws)), &body)
                .await?;
            let scene_id = created
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| "daemon did not return a scene id".to_string())?
                .to_string();

            // Reference the new scene to this session — best-effort: a session-less
            // caller (e.g. `canvas/assist/preview`-style testing) still gets the
            // created scene back even if the ref-attach can't run.
            if let Some(sid) = ctx.session_id.clone() {
                let _ = ctx
                    .post_json(
                        &format!("/sessions/{}/canvas-refs", seg(&sid)),
                        &json!({ "scene_id": scene_id }),
                    )
                    .await;
            }

            Ok(finalize(
                json!({ "scene_id": scene_id, "workspace_id": ws }),
            ))
        }
        "canvas_update_scene" => {
            let scene_id = arg_str(args, "scene_id")?;
            let source = arg_str(args, "source")?;

            let existing = ctx
                .get_json(&format!("/canvas/scenes/{}", seg(&scene_id)))
                .await?;
            let existing_doc: Value = existing
                .get("doc_json")
                .and_then(Value::as_str)
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or(json!({}));
            let format = existing_doc
                .get("format")
                .and_then(Value::as_str)
                .unwrap_or("mermaid")
                .to_string();

            let mut new_doc = json!({
                "type": "otto-canvas",
                "version": 1,
                "format": format,
                "source": source,
            });
            if let Some(sketch) = existing_doc.get("sketch") {
                new_doc["sketch"] = sketch.clone();
            }

            // `?summary=true`: the list row comes back, not the whole scene —
            // an echo of a big Excalidraw doc tripped the 1 MiB body cap and
            // reported a failure for a write that had succeeded (SD-22).
            ctx.put_json(
                &format!("/canvas/scenes/{}?summary=true", seg(&scene_id)),
                &json!({ "doc": new_doc }),
            )
            .await?;

            Ok(finalize(json!({ "ok": true, "format": format })))
        }
        // Workbench: the session owner's scratch files (owner-scoped routes).
        // `workbench_write` is Editor-gated by the daemon route and always
        // lands as a checkpoint revision, so an agent write never folds into
        // (or hides) the user's own autosave burst.
        "workbench_list" | "workbench_get" | "workbench_write" => {
            let ws = ctx
                .workspace_id
                .clone()
                .filter(|s| !s.is_empty())
                .ok_or("no workspace context (OTTO_WORKSPACE_ID unset)")?;
            let base = format!("/workspaces/{}/workbench/docs", seg(&ws));
            let find_by_name = |docs: &Value, want: &str| -> Option<String> {
                docs.as_array()?
                    .iter()
                    .find(|d| d["name"].as_str() == Some(want))
                    .and_then(|d| d["id"].as_str().map(str::to_string))
            };
            match name {
                "workbench_list" => {
                    let trash = args.get("trash").and_then(Value::as_bool).unwrap_or(false);
                    let mut docs = ctx
                        .get_json(&if trash {
                            format!("{base}?trash=true")
                        } else {
                            base.clone()
                        })
                        .await?;
                    if let (Some(q), Some(arr)) = (
                        arg_optional_string(args, "query")?.filter(|q| !q.is_empty()),
                        docs.as_array_mut(),
                    ) {
                        let q = q.to_lowercase();
                        arr.retain(|d| {
                            d["name"]
                                .as_str()
                                .is_some_and(|n| n.to_lowercase().contains(&q))
                        });
                    }
                    Ok(finalize(json!({ "workspace_id": ws, "docs": docs })))
                }
                "workbench_get" => {
                    let id = match arg_optional_string(args, "doc_id")?.filter(|s| !s.is_empty()) {
                        Some(id) => id,
                        None => {
                            let want = arg_str(args, "name")?;
                            let docs = ctx.get_json(&base).await?;
                            find_by_name(&docs, &want)
                                .ok_or_else(|| format!("no workbench file named `{want}`"))?
                        }
                    };
                    let doc_path = format!("{base}/{}", seg(&id));
                    let mut out = match args.get("revision").and_then(Value::as_i64) {
                        Some(seq) => ctx.get_json(&format!("{doc_path}/revisions/{seq}")).await?,
                        None => ctx.get_json(&doc_path).await?,
                    };
                    if args
                        .get("history")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                    {
                        let limit = match args.get("history_limit") {
                            None => 100,
                            Some(value) => value
                                .as_i64()
                                .ok_or("history_limit must be an integer")?
                                .clamp(1, 200),
                        };
                        let before = args
                            .get("before_seq")
                            .map(|value| value.as_i64().ok_or("before_seq must be an integer"))
                            .transpose()?;
                        let mut path = format!("{doc_path}/revisions?limit={limit}");
                        if let Some(before) = before {
                            path.push_str(&format!("&before_seq={before}"));
                        }
                        let revisions = ctx.get_json(&path).await?;
                        let rows = revisions
                            .as_array()
                            .ok_or("invalid workbench history response")?;
                        // A full page offers a continuation hint, not proof of
                        // more rows; the final request can legitimately be empty.
                        out["next_before_seq"] = if rows.len() == limit as usize {
                            rows.last()
                                .and_then(|row| row.get("seq"))
                                .cloned()
                                .unwrap_or(Value::Null)
                        } else {
                            Value::Null
                        };
                        out["revisions"] = revisions;
                    }
                    Ok(finalize(out))
                }
                _ => {
                    let content = arg_string_allow_empty(args, "content")?;
                    let language = arg_optional_string(args, "language")?.filter(|s| !s.is_empty());
                    let mut id = arg_optional_string(args, "doc_id")?.filter(|s| !s.is_empty());
                    let doc_name = arg_optional_string(args, "name")?.filter(|s| !s.is_empty());
                    if id.is_none() {
                        let want = doc_name.clone().ok_or("pass `doc_id` or `name`")?;
                        let docs = ctx.get_json(&base).await?;
                        id = find_by_name(&docs, &want);
                    }
                    match id {
                        Some(id) => {
                            let mut body = json!({ "content": content, "checkpoint": true });
                            if let Some(l) = language {
                                body["language"] = json!(l);
                            }
                            let doc = ctx
                                .patch_json(&format!("{base}/{}", seg(&id)), &body)
                                .await?;
                            Ok(finalize(
                                json!({ "ok": true, "created": false, "doc": doc }),
                            ))
                        }
                        None => {
                            let mut body = json!({
                                "name": doc_name.unwrap_or_default(),
                                "content": content, "scope": arg_optional_string(args, "scope")?.filter(|s| !s.is_empty()),
                            });
                            if let Some(l) = language {
                                body["language"] = json!(l);
                            }
                            let mut doc = ctx.post_json(&base, &body).await?;
                            // Don't echo the content back into the transcript.
                            if let Some(o) = doc.as_object_mut() {
                                o.remove("content");
                            }
                            Ok(finalize(json!({ "ok": true, "created": true, "doc": doc })))
                        }
                    }
                }
            }
        }
        other => Err(format!("unknown tool `{other}`")),
    }
}

/// The browser tools of [`run_tool`]: all call the governed browser routes
/// with the session's own token (netguard + RBAC enforced there).
async fn run_browser_tool(
    ctx: &Ctx,
    name: &str,
    args: &Value,
) -> Result<(Value, Option<i64>), String> {
    match name {
        "browser_navigate" => {
            let ws = ctx
                .workspace_id
                .clone()
                .ok_or("no workspace context (OTTO_WORKSPACE_ID unset)")?;
            let url = arg_str(args, "url")?;
            let tab = ctx
                .post_json(
                    &format!("/workspaces/{}/browser/tabs", seg(&ws)),
                    &json!({ "url": url }),
                )
                .await?;
            let tab_id = tab
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| "daemon did not return a tab id".to_string())?
                .to_string();
            // The tab is created in mode:"reader" by construction (see
            // `otto_state::browser::BrowserTabsRepo::create`), so this PATCH
            // runs the fetch pipeline and adopts the fetched page's title.
            let updated = match ctx
                .patch_json(
                    &format!("/browser/tabs/{}", seg(&tab_id)),
                    &json!({ "url": url }),
                )
                .await
            {
                Ok(updated) => updated,
                Err(error) => {
                    // A refused / unreachable URL must not leave a blank tab
                    // in the person's Browser for every failed agent attempt.
                    let _ = ctx
                        .delete_ok(&format!("/browser/tabs/{}", seg(&tab_id)))
                        .await;
                    return Err(error);
                }
            };
            let title = updated
                .get("title")
                .cloned()
                .unwrap_or(Value::String(String::new()));
            Ok(finalize(json!({ "ok": true, "title": title })))
        }
        "browser_page" => {
            let Some(ws) = ctx.workspace_id.as_deref() else {
                return Err(
                    "no workspace context (OTTO_WORKSPACE_ID unset); cannot fetch a page".into(),
                );
            };
            let url = arg_str(args, "url")?;
            let raw = ctx
                .get_json(&format!(
                    "/workspaces/{}/browser/page?url={}&include_html=0",
                    seg(ws),
                    seg(&url)
                ))
                .await?;
            // Drop `url` (the caller already has it) and `html` (raw markup —
            // large, and `markdown` is the extracted content agents want) so
            // the result stays small; keep the rest verbatim.
            Ok(finalize(json!({
                "markdown": raw.get("markdown").cloned().unwrap_or(Value::Null),
                "title": raw.get("title").cloned().unwrap_or(Value::Null),
                "engine": raw.get("engine").cloned().unwrap_or(Value::Null),
                "degraded": raw.get("degraded").cloned().unwrap_or(Value::Null),
            })))
        }
        "browser_query" => {
            let Some(ws) = ctx.workspace_id.as_deref() else {
                return Err(
                    "no workspace context (OTTO_WORKSPACE_ID unset); cannot query a page".into(),
                );
            };
            let url = arg_str(args, "url")?;
            let selector = arg_str(args, "selector")?;
            let raw = ctx
                .get_json(&format!(
                    "/workspaces/{}/browser/query?url={}&selector={}",
                    seg(ws),
                    seg(&url),
                    seg(&selector)
                ))
                .await?;
            Ok(finalize(raw))
        }
        "browser_summarize" => {
            let Some(ws) = ctx.workspace_id.as_deref() else {
                return Err(
                    "no workspace context (OTTO_WORKSPACE_ID unset); cannot summarize a page"
                        .into(),
                );
            };
            let url = arg_str(args, "url")?;
            // A whole agent turn: CALL_TIMEOUT would drop the request (and
            // so kill the summarizing agent) mid-turn.
            let raw = ctx
                .post_json_within(
                    &format!("/workspaces/{}/browser/summarize", seg(ws)),
                    &json!({ "url": url }),
                    SUMMARIZE_CALL_TIMEOUT,
                )
                .await?;
            Ok(finalize(raw))
        }
        // Read-only list of the user's marks — the Browser page's ask bar
        // also inlines the current page's marks into the turn it submits,
        // so this is the agent's own way to look them up later (a follow-up
        // question, or a mark made after the ask).
        "browser_marks" => {
            let Some(ws) = ctx.workspace_id.as_deref() else {
                return Err(
                    "no workspace context (OTTO_WORKSPACE_ID unset); cannot list marks".into(),
                );
            };
            let url = args
                .get("url")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|u| !u.is_empty());
            let path = match url {
                Some(u) => format!("/workspaces/{}/browser/annotations?url={}", seg(ws), seg(u)),
                None => format!("/workspaces/{}/browser/annotations", seg(ws)),
            };
            let raw = ctx.get_json(&path).await?;
            // Trim each row to what an agent acts on; the excerpt is raw page
            // HTML and can be large, so cap it per mark.
            let marks: Vec<Value> = raw
                .as_array()
                .map(|rows| {
                    rows.iter()
                        .map(|a| {
                            let excerpt: String = a
                                .get("excerpt")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .chars()
                                .take(1_000)
                                .collect();
                            json!({
                                "id": a.get("id").cloned().unwrap_or(Value::Null),
                                "url": a.get("url").cloned().unwrap_or(Value::Null),
                                "selector": a.get("selector").cloned().unwrap_or(Value::Null),
                                "text": a.get("text").cloned().unwrap_or(Value::Null),
                                "excerpt": excerpt,
                                "comment": a.get("comment").cloned().unwrap_or(Value::Null),
                                "created_at": a.get("created_at").cloned().unwrap_or(Value::Null),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            Ok(finalize(json!({ "marks": marks })))
        }
        // The daemon resolves the credential (`allow_agent_use` required,
        // else a typed 403) and drives the fill+submit itself — this arm only
        // ever sends/receives `domain`/`logged_in`/`engine`; the password
        // never reaches this process at all. `finalize`'s `redact_json` pass
        // is defense-in-depth on top of that, not the only guard.
        "browser_login" => {
            let Some(ws) = ctx.workspace_id.as_deref() else {
                return Err(
                    "no workspace context (OTTO_WORKSPACE_ID unset); cannot sign in".into(),
                );
            };
            let domain = arg_str(args, "domain")?;
            let raw = ctx
                .post_json(
                    &format!("/workspaces/{}/browser/login", seg(ws)),
                    &json!({ "domain": domain }),
                )
                .await?;
            Ok(finalize(raw))
        }
        other => Err(format!("unknown tool `{other}`")),
    }
}
