//! The outward **executor** half that needs no server state: [`route_for`]
//! maps `(tool, args)` to the exact self-call against the daemon's own REST
//! API, and [`run_tool`] / [`upsert_request_preserving`] perform it with a
//! caller-supplied client + short-lived token (so each tool reuses its
//! endpoint's native RBAC). Pure argument helpers and the self-call error
//! shaping live here too.

use std::time::Duration;

use otto_core::Error;
use serde_json::{json, Value};

/// The governed envelope of a failed UI-control call. `code` is stable
/// (`pending_grant`, `no_ui_client`, `cancelled_by_user`, `invalid_args`,
/// `not_found`, `forbidden`, `timeout`, `failed`) and repeated inside
/// `content` so a client that only surfaces `content` still sees it.
/// `executed` is false when nothing reached a window.
pub fn ui_error_envelope(code: &str, message: &str) -> Value {
    let executed = !matches!(
        code,
        "pending_grant" | "no_ui_client" | "invalid_args" | "forbidden" | "not_found"
    );
    json!({"decision":"error","executed":executed,"is_error":true,"code":code,
           "reason":message,"content":{"error":message,"code":code}})
}

pub fn is_read_only_sql(stmt: &str) -> bool {
    let s = stmt.trim().trim_end_matches(';').trim();
    if s.contains(';') {
        return false; // single statement only — no batch tricks
    }
    let up = s.to_uppercase();
    up.starts_with("SELECT")
        || up.starts_with("SHOW")
        || up.starts_with("DESCRIBE")
        || up.starts_with("DESC ")
        || up.starts_with("EXPLAIN")
        || up.starts_with("WITH")
}

/// Wall-clock budget of one governed self-call. The flat 30 s cut off calls
/// whose route legitimately runs longer: a saved API request may itself wait
/// up to its `timeout_ms` (≤ 60 s), an automation runs several, pod logs shell
/// out to kubectl with a 60 s budget. Pure — unit-tested.
pub fn call_timeout(tool: &str, args: &Value) -> Duration {
    let secs = match tool {
        "api_execute" => {
            let ms = args
                .get("timeout_ms")
                .and_then(u64_lenient)
                .unwrap_or(30_000)
                .min(60_000);
            ms / 1000 + 15
        }
        "api_run_automation" => 180,
        "k8s_logs" | "k8s_action" | "aws_athena_query" | "aws_athena_get_query" => 75,
        // ≤30 s per pod request + a pod list + proxy start-up.
        "k8s_pod_http" => 90,
        "consume_broker_messages" | "run_workflow" | "start_pr_review" | "open_session" => 60,
        _ => 30,
    };
    Duration::from_secs(secs)
}

/// Fields of a saved API request that the PATCH route REPLACES wholesale when
/// omitted (only `auth` / `extras` are preserved server-side). An agent update
/// that sends just the field it means to change must not wipe the rest.
pub const UPSERT_PRESERVED: &[&str] = &[
    "collection_id",
    "headers",
    "query",
    "body_mode",
    "body",
    "ssh_connection_id",
];

/// Fill the fields an update omitted from the STORED request (read raw, as
/// the caller — never returned to the agent). Pure — unit-tested.
pub fn merge_stored_request(args: &Value, stored: &Value) -> Value {
    let mut out = args.clone();
    if let Some(o) = out.as_object_mut() {
        for k in UPSERT_PRESERVED {
            if !o.contains_key(*k) {
                if let Some(v) = stored.get(*k).filter(|v| !v.is_null()) {
                    o.insert((*k).to_string(), v.clone());
                }
            }
        }
    }
    out
}

/// `api_upsert_request`: on update, merge the stored request under the
/// agent's fields first (see [`UPSERT_PRESERVED`]); either way, return the
/// saved request in the masked agent shape — the route answers the raw row.
pub async fn upsert_request_preserving(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    args: &Value,
) -> Result<Value, Error> {
    let ws = arg_str(args, "workspace_id")?;
    let merged = match args
        .get("request_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        Some(id) => {
            let stored = self_get(
                client,
                token,
                &format!(
                    "{base}/api/v1/workspaces/{}/api-client/requests/{}",
                    seg(&ws),
                    seg(id)
                ),
            )
            .await?;
            merge_stored_request(args, &stored)
        }
        None => args.clone(),
    };
    let saved = run_tool(client, base, token, "api_upsert_request", &merged).await?;
    match saved.get("id").and_then(Value::as_str) {
        Some(id) => {
            self_get(
                client,
                token,
                &format!(
                    "{base}/api/v1/workspaces/{}/api-client/requests/{}?shape=agent",
                    seg(&ws),
                    seg(id)
                ),
            )
            .await
        }
        None => Ok(saved),
    }
}

pub fn arg_str(args: &Value, key: &str) -> Result<String, Error> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| Error::Invalid(format!("missing required string argument '{key}'")))
}

/// One URL path segment from an untrusted value: every byte outside the
/// unreserved set percent-encoded. A segment of only dots (`.` / `..`) would
/// still be normalized by the URL parser as a dot segment — and so would its
/// `%2E` form — moving the call to ANOTHER route (`send_message
/// {session_id:".."}` → `POST /api/v1/message`); those dots are encoded as
/// `%252E`, which the route decodes to a literal, unmatched `%2E` id.
/// Shared with the stdio bridge (`ottod mcp-tools`). Pure — unit-tested.
pub fn seg(s: &str) -> String {
    if !s.is_empty() && s.bytes().all(|b| b == b'.') {
        return "%252E".repeat(s.len());
    }
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Required-integer argument extractor (PR numbers etc.). Accepts a numeric
/// string too — some MCP clients stringify every argument, and `"52"` must not
/// read as "missing".
pub fn arg_i64(args: &Value, key: &str) -> Result<i64, Error> {
    args.get(key)
        .and_then(i64_lenient)
        .ok_or_else(|| Error::Invalid(format!("missing required integer argument '{key}'")))
}

/// A JSON number, or a string holding one (see [`arg_i64`]).
pub fn i64_lenient(v: &Value) -> Option<i64> {
    v.as_i64()
        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}

/// Unsigned counterpart of [`i64_lenient`] (limits, sizes, delays).
pub fn u64_lenient(v: &Value) -> Option<u64> {
    v.as_u64()
        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}

/// Render the optional query filters of the AWS/K8s console reads: for every
/// `(query_key, arg_name)` whose argument is a non-empty string, a number or a
/// bool, append `&key=value` (strings percent-encoded). Returns the string with
/// the leading `&` stripped so callers can place it right after `?`.
pub fn opt_query(args: &Value, pairs: &[(&str, &str)]) -> String {
    let mut q = String::new();
    for (key, arg) in pairs {
        match args.get(arg) {
            Some(Value::String(s)) if !s.is_empty() => q.push_str(&format!("&{key}={}", seg(s))),
            Some(Value::Number(n)) => q.push_str(&format!("&{key}={n}")),
            Some(Value::Bool(b)) => q.push_str(&format!("&{key}={b}")),
            _ => {}
        }
    }
    q.trim_start_matches('&').to_string()
}

/// The HTTP verb a tool's self-call uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

/// A resolved self-call: the verb, the `/api/v1/...` path (incl. query string),
/// and an optional JSON body. Built purely from `(tool, args)` by [`route_for`]
/// so the endpoint binding of every tool is unit-testable without a live server.
#[derive(Debug, Clone, PartialEq)]
pub struct SelfCall {
    pub method: Method,
    pub path: String,
    pub body: Option<Value>,
}

impl SelfCall {
    fn get(path: String) -> Self {
        Self {
            method: Method::Get,
            path,
            body: None,
        }
    }
    fn post(path: String, body: Value) -> Self {
        Self {
            method: Method::Post,
            path,
            body: Some(body),
        }
    }
    fn put(path: String, body: Value) -> Self {
        Self {
            method: Method::Put,
            path,
            body: Some(body),
        }
    }
    fn patch(path: String, body: Value) -> Self {
        Self {
            method: Method::Patch,
            path,
            body: Some(body),
        }
    }
    fn delete(path: String) -> Self {
        Self {
            method: Method::Delete,
            path,
            body: None,
        }
    }
}

/// Map an outward tool + its (validated) arguments to the exact self-call against
/// the daemon's own REST API. Pure: no I/O, no token — every tool reuses its
/// endpoint's native RBAC when the call is later executed as the user. `ask_human_approval`
/// is handled earlier (in `execute_otto_tool`) and never reaches here.
pub fn route_for(tool: &str, args: &Value) -> Result<SelfCall, Error> {
    Ok(match tool {
        // ---- Code & context ----
        "search_codebase" => {
            let ws = arg_str(args, "workspace_id")?;
            let q = arg_str(args, "query")?;
            let mut path = format!(
                "/api/v1/workspaces/{}/mcp/code-search?q={}",
                seg(&ws),
                seg(&q)
            );
            if let Some(p) = args.get("path").and_then(Value::as_str) {
                path.push_str(&format!("&path={}", seg(p)));
            }
            if let Some(m) = args.get("max_results").and_then(u64_lenient) {
                path.push_str(&format!("&max={m}"));
            }
            SelfCall::get(path)
        }
        "get_context_packet" => {
            let ws = arg_str(args, "workspace_id")?;
            SelfCall::post(
                format!("/api/v1/workspaces/{}/mcp/context-packet", seg(&ws)),
                args.clone(),
            )
        }
        "get_proof_pack" => {
            let ws = arg_str(args, "workspace_id")?;
            let mut path = format!("/api/v1/workspaces/{}/mcp/proof-pack?", seg(&ws));
            for k in ["repo_id", "branch", "goal_loop_id"] {
                if let Some(v) = args.get(k).and_then(Value::as_str) {
                    path.push_str(&format!("{k}={}&", seg(v)));
                }
            }
            SelfCall::get(path)
        }
        // ---- Database ----
        "query_db_readonly" => {
            let conn = arg_str(args, "connection_id")?;
            let stmt = arg_str(args, "statement")?;
            // F5: classify ourselves; reject writes/unknown/multi-statement
            // REGARDLESS of the connection's write-guard flag.
            if !is_read_only_sql(&stmt) {
                return Err(Error::Forbidden(
                    "otto.query_db_readonly only permits a single read-only statement (SELECT/SHOW/DESCRIBE/EXPLAIN/WITH)".into(),
                ));
            }
            // Routed through `/db/mcp-query` (not `/db/query`): that path adds the
            // engine-specific read-only classifier AND executes inside the
            // engine's read-only mode (read-only transaction / ClickHouse
            // `readonly`), with cell masking forced — so "read-only" holds even
            // if a statement slips past the classifier above.
            let body = json!({
                "statement": stmt,
                "max_rows": args.get("max_rows").and_then(u64_lenient).unwrap_or(200),
                "node": args.get("node").and_then(Value::as_str),
            });
            SelfCall::post(
                format!("/api/v1/connections/{}/db/mcp-query", seg(&conn)),
                body,
            )
        }
        "list_connections" => {
            let ws = arg_str(args, "workspace_id")?;
            SelfCall::get(format!("/api/v1/workspaces/{}/connections", seg(&ws)))
        }
        // ---- API Client ----
        "api_list" => {
            let ws = arg_str(args, "workspace_id")?;
            let query = opt_query(
                args,
                &[
                    ("q", "q"),
                    ("collection_id", "collection_id"),
                    ("kind", "kind"),
                ],
            );
            let mut path = format!("/api/v1/workspaces/{}/api-client/overview", seg(&ws));
            if !query.is_empty() {
                path.push('?');
                path.push_str(&query);
            }
            SelfCall::get(path)
        }
        "api_get_request" => {
            let ws = arg_str(args, "workspace_id")?;
            let request = arg_str(args, "request_id")?;
            SelfCall::get(format!(
                "/api/v1/workspaces/{}/api-client/requests/{}?shape=agent",
                seg(&ws),
                seg(&request)
            ))
        }
        "api_history" => {
            let ws = arg_str(args, "workspace_id")?;
            if let Some(id) = args
                .get("id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                SelfCall::get(format!(
                    "/api/v1/workspaces/{}/api-client/history/{}",
                    seg(&ws),
                    seg(id)
                ))
            } else {
                let query = opt_query(
                    args,
                    &[
                        ("limit", "limit"),
                        ("q", "q"),
                        ("status", "status"),
                        ("request_id", "request_id"),
                        ("source", "source"),
                    ],
                );
                let mut path = format!("/api/v1/workspaces/{}/api-client/history", seg(&ws));
                if !query.is_empty() {
                    path.push('?');
                    path.push_str(&query);
                }
                SelfCall::get(path)
            }
        }
        "api_execute" => {
            let ws = arg_str(args, "workspace_id")?;
            let request = arg_str(args, "request_id")?;
            if let Some(vars) = args.get("vars").and_then(Value::as_object) {
                for (key, value) in vars {
                    if value.as_str().is_some_and(|s| s.contains("{{")) {
                        return Err(Error::Invalid(format!(
                            "vars override '{key}' must not contain '{{{{' (no nested substitution)"
                        )));
                    }
                }
            }
            let mut body = json!({"shape": "agent"});
            for key in [
                "environment_id",
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
            SelfCall::post(
                format!(
                    "/api/v1/workspaces/{}/api-client/requests/{}/execute",
                    seg(&ws),
                    seg(&request)
                ),
                body,
            )
        }
        "api_upsert_request" => {
            let ws = arg_str(args, "workspace_id")?;
            let name = arg_str(args, "name")?;
            let method = arg_str(args, "method")?;
            let url = arg_str(args, "url")?;
            let mut body = json!({"name": name, "method": method, "url": url});
            for key in [
                "collection_id",
                "headers",
                "query",
                "body_mode",
                "body",
                "auth",
                "extras",
                "ssh_connection_id",
            ] {
                if let Some(value) = args.get(key) {
                    body[key] = value.clone();
                }
            }
            if let Some(request) = args
                .get("request_id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                SelfCall::patch(
                    format!(
                        "/api/v1/workspaces/{}/api-client/requests/{}",
                        seg(&ws),
                        seg(request)
                    ),
                    body,
                )
            } else {
                SelfCall::post(
                    format!("/api/v1/workspaces/{}/api-client/requests", seg(&ws)),
                    body,
                )
            }
        }
        "api_run_automation" => {
            let ws = arg_str(args, "workspace_id")?;
            let automation = arg_str(args, "automation_id")?;
            SelfCall::post(
                format!(
                    "/api/v1/workspaces/{}/api-client/automations/{}/run",
                    seg(&ws),
                    seg(&automation)
                ),
                json!({}),
            )
        }
        // ---- Git ----
        "open_pr_draft" => {
            let repo = arg_str(args, "repo_id")?;
            let base_branch = arg_str(args, "base")?;
            SelfCall::post(
                format!("/api/v1/repos/{}/pr/draft", seg(&repo)),
                json!({"base": base_branch}),
            )
        }
        "list_repos" => {
            // Every workspace the caller can read (the workspace-annotated
            // directory), unless narrowed to one. A pinned token never gets
            // here without its pin filled in (`fill_repo_ref`).
            let mut path = "/api/v1/git/repos/directory".to_string();
            if let Some(ws) = args
                .get("workspace_id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                path.push_str(&format!("?workspace_id={}", seg(ws)));
            }
            SelfCall::get(path)
        }
        "git_status" => {
            let repo = arg_str(args, "repo_id")?;
            SelfCall::get(format!("/api/v1/repos/{}/status", seg(&repo)))
        }
        "list_prs" => {
            let repo = arg_str(args, "repo_id")?;
            let q = opt_query(
                args,
                &[
                    ("state", "state"),
                    ("page", "page"),
                    ("per_page", "per_page"),
                ],
            );
            SelfCall::get(if q.is_empty() {
                format!("/api/v1/repos/{}/prs", seg(&repo))
            } else {
                format!("/api/v1/repos/{}/prs?{q}", seg(&repo))
            })
        }
        "list_pr_reviews" => {
            let repo = arg_str(args, "repo_id")?;
            let n = arg_i64(args, "pr_number")?;
            SelfCall::get(format!("/api/v1/repos/{}/prs/{}/reviews", seg(&repo), n))
        }
        "get_pr_checks" => {
            let repo = arg_str(args, "repo_id")?;
            let n = arg_i64(args, "number")?;
            SelfCall::get(format!("/api/v1/repos/{}/prs/{}/checks", seg(&repo), n))
        }
        "get_pr_diff" => {
            let repo = arg_str(args, "repo_id")?;
            let n = arg_i64(args, "number")?;
            // The route caps a whole-PR diff; these page it per file.
            let q = opt_query(
                args,
                &[
                    ("summary", "summary"),
                    ("path", "path"),
                    ("old_path", "old_path"),
                    ("full", "full"),
                ],
            );
            let base = format!("/api/v1/repos/{}/prs/{}/diff", seg(&repo), n);
            SelfCall::get(if q.is_empty() {
                base
            } else {
                format!("{base}?{q}")
            })
        }
        "merge_pr" => {
            let repo = arg_str(args, "repo_id")?;
            let n = arg_i64(args, "number")?;
            let mut body = json!({});
            if let Some(st) = args
                .get("strategy")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                body["strategy"] = json!(st);
            }
            if let Some(d) = args.get("delete_source_branch").and_then(Value::as_bool) {
                body["delete_source_branch"] = json!(d);
            }
            SelfCall::post(
                format!("/api/v1/repos/{}/prs/{}/merge", seg(&repo), n),
                body,
            )
        }
        "get_pr" => {
            let repo = arg_str(args, "repo_id")?;
            let n = arg_i64(args, "number")?;
            SelfCall::get(format!("/api/v1/repos/{}/prs/{}", seg(&repo), n))
        }
        "create_pr" => {
            let repo = arg_str(args, "repo_id")?;
            let mut body = json!({
                "title": arg_str(args, "title")?,
                "description": arg_str(args, "description")?,
                "source_branch": arg_str(args, "source_branch")?,
                "target_branch": arg_str(args, "target_branch")?,
            });
            if let Some(d) = args.get("draft").and_then(Value::as_bool) {
                body["draft"] = json!(d);
            }
            if let Some(r) = args.get("reviewers").filter(|v| v.is_array()) {
                body["reviewers"] = r.clone();
            }
            if let Some(p) = args
                .get("proof_pack_id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                body["proof_pack_id"] = json!(p);
            }
            SelfCall::post(format!("/api/v1/repos/{}/prs", seg(&repo)), body)
        }
        "comment_pr" => {
            let repo = arg_str(args, "repo_id")?;
            let n = arg_i64(args, "number")?;
            let body = json!({
                "body": arg_str(args, "body")?,
                "path": args.get("path").and_then(Value::as_str),
                "line": args.get("line").and_then(u64_lenient),
                "in_reply_to": args.get("in_reply_to").and_then(Value::as_str),
            });
            SelfCall::post(
                format!("/api/v1/repos/{}/prs/{}/comments", seg(&repo), n),
                body,
            )
        }
        "start_pr_review" => {
            let repo = arg_str(args, "repo_id")?;
            let n = arg_i64(args, "pr_number")?;
            let mut body = json!({});
            for k in ["context", "issue_key", "issue_account_id"] {
                if let Some(v) = args
                    .get(k)
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                {
                    body[k] = json!(v);
                }
            }
            SelfCall::post(
                format!("/api/v1/repos/{}/prs/{}/review", seg(&repo), n),
                body,
            )
        }
        // ---- Workflows ----
        "list_workflows" => {
            let ws = arg_str(args, "workspace_id")?;
            SelfCall::get(format!("/api/v1/workspaces/{}/workflows", seg(&ws)))
        }
        "get_workflow" => {
            let id = arg_str(args, "workflow_id")?;
            SelfCall::get(format!("/api/v1/workflows/{}", seg(&id)))
        }
        "list_workflow_runs" => {
            let id = arg_str(args, "workflow_id")?;
            // Lightweight rows by default: the full rows (node states, inputs,
            // outputs × 50) blow the result cap for a busy workflow.
            let summary = args.get("summary").and_then(Value::as_bool).unwrap_or(true);
            SelfCall::get(format!(
                "/api/v1/workflows/{}/runs?summary={summary}",
                seg(&id)
            ))
        }
        "get_workflow_run" => {
            let id = arg_str(args, "run_id")?;
            SelfCall::get(format!("/api/v1/workflow-runs/{}", seg(&id)))
        }
        "run_workflow" => {
            let id = arg_str(args, "workflow_id")?;
            let mut body = json!({});
            if let Some(v) = args.get("input") {
                body["input"] = v.clone();
            }
            if let Some(v) = args.get("start_node").and_then(Value::as_str) {
                body["start_node"] = json!(v);
            }
            // Per-run review-mode override; the route 400s on anything but
            // "fan_out" / "orchestrator".
            if let Some(v) = args.get("review_mode").and_then(Value::as_str) {
                body["review_mode"] = json!(v);
            }
            SelfCall::post(format!("/api/v1/workflows/{}/run", seg(&id)), body)
        }
        "cancel_workflow_run" => {
            let id = arg_str(args, "run_id")?;
            SelfCall::post(
                format!("/api/v1/workflow-runs/{}/cancel", seg(&id)),
                json!({}),
            )
        }
        // ---- Message brokers ----
        "list_broker_clusters" => {
            let ws = arg_str(args, "workspace_id")?;
            SelfCall::get(format!("/api/v1/workspaces/{}/brokers/clusters", seg(&ws)))
        }
        "list_broker_topics" => {
            let id = arg_str(args, "cluster_id")?;
            SelfCall::get(format!("/api/v1/brokers/clusters/{}/topics", seg(&id)))
        }
        "get_broker_topic" => {
            let id = arg_str(args, "cluster_id")?;
            let topic = arg_str(args, "topic")?;
            SelfCall::get(format!(
                "/api/v1/brokers/clusters/{}/topics/{}",
                seg(&id),
                seg(&topic)
            ))
        }
        "list_consumer_groups" => {
            let id = arg_str(args, "cluster_id")?;
            SelfCall::get(format!("/api/v1/brokers/clusters/{}/groups", seg(&id)))
        }
        "consume_broker_messages" => {
            let id = arg_str(args, "cluster_id")?;
            let topic = arg_str(args, "topic")?;
            let mut body = json!({});
            if let Some(p) = args.get("partition").and_then(i64_lenient) {
                body["partition"] = json!(p);
            }
            if let Some(l) = args.get("limit").and_then(u64_lenient) {
                body["limit"] = json!(l);
            }
            if let Some(f) = args.get("value_filter").and_then(Value::as_str) {
                body["value_filter"] = json!(f);
            }
            SelfCall::post(
                format!(
                    "/api/v1/brokers/clusters/{}/topics/{}/consume",
                    seg(&id),
                    seg(&topic)
                ),
                body,
            )
        }
        "produce_broker_message" => {
            let id = arg_str(args, "cluster_id")?;
            let topic = arg_str(args, "topic")?;
            let mut body = json!({ "value": arg_str(args, "value")? });
            if let Some(k) = args.get("key").and_then(Value::as_str) {
                body["key"] = json!(k);
            }
            if let Some(p) = args.get("partition").and_then(i64_lenient) {
                body["partition"] = json!(p);
            }
            if let Some(c) = args.get("confirm").and_then(Value::as_bool) {
                body["confirm"] = json!(c);
            }
            SelfCall::post(
                format!(
                    "/api/v1/brokers/clusters/{}/topics/{}/produce",
                    seg(&id),
                    seg(&topic)
                ),
                body,
            )
        }
        // ---- Issues (Jira / Confluence) ----
        "search_issues" => {
            let acc = arg_str(args, "account_id")?;
            let mut path = format!("/api/v1/issue/search?account_id={}", seg(&acc));
            if let Some(q) = args.get("query").and_then(Value::as_str) {
                path.push_str(&format!("&q={}", seg(q)));
            }
            if let Some(p) = args
                .get("project")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                path.push_str(&format!("&project={}", seg(p)));
            }
            if let Some(n) = args.get("start_at").and_then(u64_lenient) {
                path.push_str(&format!("&start_at={n}"));
            }
            SelfCall::get(path)
        }
        "list_issue_transitions" => {
            let acc = arg_str(args, "account_id")?;
            let key = arg_str(args, "key")?;
            SelfCall::get(format!(
                "/api/v1/issue/{}/{}/transitions",
                seg(&acc),
                seg(&key)
            ))
        }
        "get_issue" => {
            let acc = arg_str(args, "account_id")?;
            let key = arg_str(args, "key")?;
            SelfCall::get(format!("/api/v1/issue/{}/{}/full", seg(&acc), seg(&key)))
        }
        "search_confluence" => {
            let acc = arg_str(args, "account_id")?;
            let q = arg_str(args, "query")?;
            let mut path = format!(
                "/api/v1/issue/confluence/search?account_id={}&q={}",
                seg(&acc),
                seg(&q)
            );
            if let Some(s) = args
                .get("space")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                path.push_str(&format!("&space={}", seg(s)));
            }
            SelfCall::get(path)
        }
        "get_confluence_page" => {
            let acc = arg_str(args, "account_id")?;
            let pid = arg_str(args, "page_id")?;
            SelfCall::get(format!(
                "/api/v1/issue/confluence/pages/{}?account_id={}",
                seg(&pid),
                seg(&acc)
            ))
        }
        "list_confluence_page_comments" => {
            let acc = arg_str(args, "account_id")?;
            let pid = arg_str(args, "page_id")?;
            SelfCall::get(format!(
                "/api/v1/issue/confluence/pages/{}/comments?account_id={}",
                seg(&pid),
                seg(&acc)
            ))
        }
        "create_confluence_page" => {
            let acc = arg_str(args, "account_id")?;
            let mut body = json!({
                "space_key": arg_str(args, "space_key")?,
                "title": arg_str(args, "title")?,
            });
            for k in ["body_md", "body_html"] {
                if let Some(v) = args
                    .get(k)
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                {
                    body[k] = json!(v);
                }
            }
            if let Some(p) = args
                .get("parent_id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                body["parent_id"] = json!(p);
            }
            SelfCall::post(
                format!("/api/v1/issue/confluence/pages?account_id={}", seg(&acc)),
                body,
            )
        }
        "update_confluence_page" => {
            let acc = arg_str(args, "account_id")?;
            let pid = arg_str(args, "page_id")?;
            let mut body = json!({});
            for k in ["body_md", "body_html"] {
                if let Some(v) = args
                    .get(k)
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                {
                    body[k] = json!(v);
                }
            }
            if let Some(t) = args
                .get("title")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                body["title"] = json!(t);
            }
            if let Some(v) = args.get("base_version").and_then(i64_lenient) {
                body["base_version"] = json!(v);
            }
            SelfCall::put(
                format!(
                    "/api/v1/issue/confluence/pages/{}?account_id={}",
                    seg(&pid),
                    seg(&acc)
                ),
                body,
            )
        }
        "comment_confluence_page" => {
            let acc = arg_str(args, "account_id")?;
            let pid = arg_str(args, "page_id")?;
            let mut body = json!({});
            for k in ["body_md", "body_html"] {
                if let Some(v) = args
                    .get(k)
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                {
                    body[k] = json!(v);
                }
            }
            SelfCall::post(
                format!(
                    "/api/v1/issue/confluence/pages/{}/comments?account_id={}",
                    seg(&pid),
                    seg(&acc)
                ),
                body,
            )
        }
        "comment_issue" => {
            let acc = arg_str(args, "account_id")?;
            let key = arg_str(args, "key")?;
            let body = json!({ "body": arg_str(args, "body")? });
            SelfCall::post(
                format!("/api/v1/issue/{}/{}/comment", seg(&acc), seg(&key)),
                body,
            )
        }
        "transition_issue" => {
            let acc = arg_str(args, "account_id")?;
            let key = arg_str(args, "key")?;
            let body = json!({ "transition_id": arg_str(args, "transition_id")? });
            SelfCall::post(
                format!("/api/v1/issue/{}/{}/transitions", seg(&acc), seg(&key)),
                body,
            )
        }
        // ---- Swarm ----
        "list_swarms" => {
            let ws = arg_str(args, "workspace_id")?;
            SelfCall::get(format!("/api/v1/workspaces/{}/swarm/swarms", seg(&ws)))
        }
        "get_swarm" => {
            let id = arg_str(args, "swarm_id")?;
            SelfCall::get(format!("/api/v1/swarm/swarms/{}", seg(&id)))
        }
        "list_swarm_runs" => {
            let ws = arg_str(args, "workspace_id")?;
            let q = opt_query(args, &[("swarm_id", "swarm_id")]);
            SelfCall::get(if q.is_empty() {
                format!("/api/v1/workspaces/{}/swarm/runs", seg(&ws))
            } else {
                format!("/api/v1/workspaces/{}/swarm/runs?{q}", seg(&ws))
            })
        }
        "list_swarm_projects" => {
            let id = arg_str(args, "swarm_id")?;
            SelfCall::get(format!("/api/v1/swarm/swarms/{}/projects", seg(&id)))
        }
        "list_swarm_tasks" => {
            let id = arg_str(args, "project_id")?;
            SelfCall::get(format!("/api/v1/swarm/projects/{}/tasks", seg(&id)))
        }
        "get_swarm_board" => {
            let id = arg_str(args, "swarm_id")?;
            let q = opt_query(
                args,
                &[("project_id", "project_id"), ("task_id", "task_id")],
            );
            SelfCall::get(if q.is_empty() {
                format!("/api/v1/swarm/swarms/{}/board", seg(&id))
            } else {
                format!("/api/v1/swarm/swarms/{}/board?{q}", seg(&id))
            })
        }
        "post_swarm_board" => {
            let id = arg_str(args, "swarm_id")?;
            let mut body = json!({ "body": arg_str(args, "body")? });
            if let Some(p) = args.get("project_id").and_then(Value::as_str) {
                body["project_id"] = json!(p);
            }
            if let Some(t) = args.get("task_id").and_then(Value::as_str) {
                body["task_id"] = json!(t);
            }
            SelfCall::post(format!("/api/v1/swarm/swarms/{}/board", seg(&id)), body)
        }
        // ---- Memory / vault ----
        "list_memory" => {
            let ws = arg_str(args, "workspace_id")?;
            let mut q: Vec<String> = Vec::new();
            if let Some(c) = args
                .get("collection")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                q.push(format!("collection={}", seg(c)));
            }
            if let Some(s) = args
                .get("story_id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                q.push(format!("story_id={}", seg(s)));
            }
            let mut path = format!("/api/v1/workspaces/{}/memories", seg(&ws));
            if !q.is_empty() {
                path.push('?');
                path.push_str(&q.join("&"));
            }
            SelfCall::get(path)
        }
        "search_memory" => {
            let ws = arg_str(args, "workspace_id")?;
            // `k` defaults to 0 server-side (MemoryQuery), which would return nothing —
            // supply a useful default so a caller that omits it still gets hits.
            let k = args.get("k").and_then(u64_lenient).unwrap_or(20);
            let body = json!({ "text": arg_str(args, "query")?, "k": k });
            SelfCall::post(
                format!("/api/v1/workspaces/{}/memory/search", seg(&ws)),
                body,
            )
        }
        // ---- Vault v3 (docs home) ----
        "vault_list" => {
            let ws = arg_str(args, "workspace_id")?;
            SelfCall::get(format!("/api/v1/workspaces/{}/vault/vaults", seg(&ws)))
        }
        "vault_dir" => {
            let ws = arg_str(args, "workspace_id")?;
            let v = arg_i64(args, "vault_id")?;
            let path = args.get("path").and_then(Value::as_str).unwrap_or("");
            SelfCall::get(format!(
                "/api/v1/workspaces/{}/vault/vaults/{v}/dir?path={}",
                seg(&ws),
                seg(path)
            ))
        }
        "vault_read" => {
            let ws = arg_str(args, "workspace_id")?;
            let v = arg_i64(args, "vault_id")?;
            SelfCall::get(format!(
                "/api/v1/workspaces/{}/vault/vaults/{v}/note?path={}",
                seg(&ws),
                seg(&arg_str(args, "path")?)
            ))
        }
        "vault_search" => {
            let ws = arg_str(args, "workspace_id")?;
            let v = arg_i64(args, "vault_id")?;
            let limit = args.get("limit").and_then(u64_lenient).unwrap_or(20);
            let body = json!({ "query": arg_str(args, "query")?, "limit": limit });
            SelfCall::post(
                format!("/api/v1/workspaces/{}/vault/vaults/{v}/search", seg(&ws)),
                body,
            )
        }
        "vault_backlinks" => {
            let ws = arg_str(args, "workspace_id")?;
            let v = arg_i64(args, "vault_id")?;
            SelfCall::get(format!(
                "/api/v1/workspaces/{}/vault/vaults/{v}/backlinks?path={}",
                seg(&ws),
                seg(&arg_str(args, "path")?)
            ))
        }
        "vault_tags" => {
            let ws = arg_str(args, "workspace_id")?;
            let v = arg_i64(args, "vault_id")?;
            SelfCall::get(format!(
                "/api/v1/workspaces/{}/vault/vaults/{v}/tags",
                seg(&ws)
            ))
        }
        "vault_graph" => {
            let ws = arg_str(args, "workspace_id")?;
            let v = arg_i64(args, "vault_id")?;
            let mut path = format!("/api/v1/workspaces/{}/vault/vaults/{v}/graph", seg(&ws));
            let focus = args
                .get("path")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty());
            let mode = args
                .get("mode")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or(if focus.is_some() { "local" } else { "full" });
            path.push_str(&format!("?mode={}", seg(mode)));
            if let Some(f) = focus {
                path.push_str(&format!("&path={}", seg(f)));
            }
            if let Some(d) = args.get("depth").and_then(u64_lenient) {
                path.push_str(&format!("&depth={d}"));
            }
            SelfCall::get(path)
        }
        "vault_okf_validate" => {
            let ws = arg_str(args, "workspace_id")?;
            let v = arg_i64(args, "vault_id")?;
            SelfCall::post(
                format!(
                    "/api/v1/workspaces/{}/vault/vaults/{v}/okf/validate",
                    seg(&ws)
                ),
                json!({}),
            )
        }
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
            SelfCall::get(if q.is_empty() {
                "/api/v1/design/artifacts".to_string()
            } else {
                format!("/api/v1/design/artifacts?{q}")
            })
        }
        "list_design_projects" => {
            let q = opt_query(
                args,
                &[
                    ("workspace_id", "workspace_id"),
                    ("include_archived", "include_archived"),
                ],
            );
            SelfCall::get(if q.is_empty() {
                "/api/v1/design/projects".to_string()
            } else {
                format!("/api/v1/design/projects?{q}")
            })
        }
        "design_get" => {
            let id = arg_str(args, "artifact_id")?;
            let content = args
                .get("include_content")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let mut path = format!("/api/v1/design/artifacts/{}?content={content}", seg(&id));
            if let Some(v) = args
                .get("version")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                path.push_str(&format!("&version={}", seg(v)));
            }
            SelfCall::get(path)
        }
        "design_links" => {
            let id = arg_str(args, "artifact_id")?;
            let dir = args
                .get("dir")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or("both");
            SelfCall::get(format!(
                "/api/v1/design/artifacts/{}/links?dir={}",
                seg(&id),
                seg(dir)
            ))
        }
        "design_search" => {
            let query = arg_str(args, "query")?;
            let mut path = format!("/api/v1/design/search?q={}", seg(&query));
            let q = opt_query(
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
            );
            if !q.is_empty() {
                path.push('&');
                path.push_str(&q);
            }
            SelfCall::get(path)
        }
        "design_assist" => {
            let id = arg_str(args, "artifact_id")?;
            let mut body = json!({ "prompt": arg_str(args, "prompt")? });
            for k in ["mode", "references", "selection"] {
                if let Some(v) = args.get(k).filter(|v| !v.is_null()) {
                    body[k] = v.clone();
                }
            }
            SelfCall::post(
                format!("/api/v1/design/artifacts/{}/assist", seg(&id)),
                body,
            )
        }
        "design_link" => {
            let id = arg_str(args, "artifact_id")?;
            let mut body = json!({
                "rel": arg_str(args, "rel")?,
                "dst_kind": arg_str(args, "dst_kind")?,
                "dst_id": arg_str(args, "dst_id")?,
            });
            for k in ["dst_node", "src_node", "policy", "pinned_version_id"] {
                if let Some(v) = args
                    .get(k)
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                {
                    body[k] = json!(v);
                }
            }
            SelfCall::post(format!("/api/v1/design/artifacts/{}/links", seg(&id)), body)
        }
        "vault_write" => {
            let ws = arg_str(args, "workspace_id")?;
            let v = arg_i64(args, "vault_id")?;
            let mut body = json!({
                "path": arg_str(args, "path")?,
                "content": args.get("content").and_then(Value::as_str).unwrap_or(""),
            });
            if let Some(h) = args.get("if_hash").and_then(Value::as_str) {
                body["if_hash"] = json!(h);
            }
            SelfCall::put(
                format!("/api/v1/workspaces/{}/vault/vaults/{v}/note", seg(&ws)),
                body,
            )
        }
        "vault_write_file" => {
            let ws = arg_str(args, "workspace_id")?;
            let v = arg_i64(args, "vault_id")?;
            let mut body = json!({
                "path": arg_str(args, "path")?,
                "content": args.get("content").and_then(Value::as_str).unwrap_or(""),
            });
            if let Some(h) = args.get("if_hash").and_then(Value::as_str) {
                body["if_hash"] = json!(h);
            }
            SelfCall::put(
                format!("/api/v1/workspaces/{}/vault/vaults/{v}/file", seg(&ws)),
                body,
            )
        }
        "vault_rename" => {
            let ws = arg_str(args, "workspace_id")?;
            let v = arg_i64(args, "vault_id")?;
            let body = json!({ "from": arg_str(args, "from")?, "to": arg_str(args, "to")? });
            SelfCall::post(
                format!("/api/v1/workspaces/{}/vault/vaults/{v}/rename", seg(&ws)),
                body,
            )
        }
        "vault_delete" => {
            let ws = arg_str(args, "workspace_id")?;
            let v = arg_i64(args, "vault_id")?;
            SelfCall::delete(format!(
                "/api/v1/workspaces/{}/vault/vaults/{v}/note?path={}",
                seg(&ws),
                seg(&arg_str(args, "path")?)
            ))
        }
        // ---- Sessions ----
        "list_sessions" => {
            let ws = arg_str(args, "workspace_id")?;
            SelfCall::get(format!("/api/v1/workspaces/{}/sessions", seg(&ws)))
        }
        "get_session" => {
            let id = arg_str(args, "session_id")?;
            SelfCall::get(format!("/api/v1/sessions/{}", seg(&id)))
        }
        "broadcast_message" => {
            let ws = arg_str(args, "workspace_id")?;
            let body = json!({ "text": arg_str(args, "text")? });
            SelfCall::post(format!("/api/v1/workspaces/{}/broadcast", seg(&ws)), body)
        }
        "open_session" => {
            let ws = arg_str(args, "workspace_id")?;
            let mut body = json!({ "provider": arg_str(args, "provider")? });
            for k in ["title", "cwd", "model", "prompt"] {
                if let Some(v) = args
                    .get(k)
                    .and_then(Value::as_str)
                    .filter(|v| !v.is_empty())
                {
                    body[k] = json!(v);
                }
            }
            SelfCall::post(
                format!("/api/v1/workspaces/{}/sessions/open", seg(&ws)),
                body,
            )
        }
        "send_message" => {
            let id = arg_str(args, "session_id")?;
            let body = json!({ "text": arg_str(args, "text")? });
            SelfCall::post(format!("/api/v1/sessions/{}/message", seg(&id)), body)
        }
        "wait_session" => {
            let id = arg_str(args, "session_id")?;
            let mut path = format!("/api/v1/sessions/{}/wait?", seg(&id));
            let q = opt_query(
                args,
                &[("status", "status"), ("timeout_secs", "timeout_secs")],
            );
            path.push_str(&q);
            SelfCall::get(path)
        }
        // ---- Code review / findings ----
        "list_findings" => {
            let rid = arg_str(args, "review_id")?;
            SelfCall::get(format!("/api/v1/reviews/{}/findings", seg(&rid)))
        }
        "get_finding" => {
            let id = arg_str(args, "finding_id")?;
            SelfCall::get(format!("/api/v1/findings/{}", seg(&id)))
        }
        // ---- Product ----
        "list_product_stories" => {
            let ws = arg_str(args, "workspace_id")?;
            SelfCall::get(format!("/api/v1/workspaces/{}/product/stories", seg(&ws)))
        }
        "get_product_story" => {
            let id = arg_str(args, "story_id")?;
            SelfCall::get(format!("/api/v1/product/stories/{}", seg(&id)))
        }
        // ---- Channels ----
        "list_integrations" => {
            let ws = arg_str(args, "workspace_id")?;
            SelfCall::get(format!("/api/v1/workspaces/{}/integrations", seg(&ws)))
        }
        "test_integration" => {
            let ws = arg_str(args, "workspace_id")?;
            let ch = arg_str(args, "channel")?;
            SelfCall::post(
                format!(
                    "/api/v1/workspaces/{}/integrations/{}/test",
                    seg(&ws),
                    seg(&ch)
                ),
                json!({}),
            )
        }
        // ---- Usage ----
        "get_usage_summary" => {
            let mut q: Vec<String> = Vec::new();
            if let Some(d) = args.get("days").and_then(u64_lenient) {
                q.push(format!("days={d}"));
            }
            if let Some(o) = args.get("otto_only").and_then(Value::as_bool) {
                q.push(format!("otto_only={o}"));
            }
            let mut path = "/api/v1/usage/summary".to_string();
            if !q.is_empty() {
                path.push('?');
                path.push_str(&q.join("&"));
            }
            SelfCall::get(path)
        }
        // ---- Skills ----
        "list_bundled_skills" => SelfCall::get("/api/v1/library/bundled".to_string()),
        // ---- Self-improvement ----
        "get_self_improvement_config" => {
            let ws = arg_str(args, "workspace_id")?;
            SelfCall::get(format!("/api/v1/workspaces/{}/self-improvement", seg(&ws)))
        }
        "list_improvement_runs" => {
            let ws = arg_str(args, "workspace_id")?;
            SelfCall::get(format!("/api/v1/workspaces/{}/improvement/runs", seg(&ws)))
        }
        "get_improvement_run" => {
            let id = arg_str(args, "run_id")?;
            SelfCall::get(format!("/api/v1/improvement/runs/{}", seg(&id)))
        }
        "list_improvement_edits" => {
            let ws = arg_str(args, "workspace_id")?;
            let q = opt_query(args, &[("status", "status")]);
            SelfCall::get(if q.is_empty() {
                format!("/api/v1/workspaces/{}/improvement/edits", seg(&ws))
            } else {
                format!("/api/v1/workspaces/{}/improvement/edits?{q}", seg(&ws))
            })
        }
        "run_self_improvement" => {
            let ws = arg_str(args, "workspace_id")?;
            SelfCall::post(
                format!("/api/v1/workspaces/{}/self-improvement/run", seg(&ws)),
                json!({}),
            )
        }
        "approve_improvement_edit" => {
            let id = arg_str(args, "edit_id")?;
            SelfCall::post(
                format!("/api/v1/improvement/edits/{}/approve", seg(&id)),
                json!({}),
            )
        }
        "reject_improvement_edit" => {
            let id = arg_str(args, "edit_id")?;
            SelfCall::post(
                format!("/api/v1/improvement/edits/{}/reject", seg(&id)),
                json!({}),
            )
        }
        "rollback_improvement_edit" => {
            let id = arg_str(args, "edit_id")?;
            SelfCall::post(
                format!("/api/v1/improvement/edits/{}/rollback", seg(&id)),
                json!({}),
            )
        }
        // ---- Goal loop / swarm task / scheduled tasks ----
        "run_goal_loop" => {
            let ws = arg_str(args, "workspace_id")?;
            let mut body = args.clone();
            if let Some(obj) = body.as_object_mut() {
                obj.remove("workspace_id");
                obj.insert("autostart".into(), json!(true));
            }
            SelfCall::post(format!("/api/v1/workspaces/{}/goal-loops", seg(&ws)), body)
        }
        "create_work_item" => {
            let project = arg_str(args, "project_id")?;
            let body = json!({
                "title": arg_str(args, "title")?,
                "description": args.get("description").and_then(Value::as_str),
                "priority": args.get("priority").and_then(Value::as_str),
            });
            SelfCall::post(
                format!("/api/v1/swarm/projects/{}/tasks", seg(&project)),
                body,
            )
        }
        "room_post" => {
            let room = arg_str(args, "room_id")?;
            let text = arg_str(args, "text")?;
            let mut body = json!({"text": text});
            if let Some(sid) = args
                .get("session_id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                body["session_id"] = json!(sid);
            }
            SelfCall::post(format!("/api/v1/agent-rooms/{}/messages", seg(&room)), body)
        }
        "room_read" => {
            let room = arg_str(args, "room_id")?;
            // Same paging as the native `otto_room_read`: `after` forward,
            // `before` back, neither = the room's tail (it used to read from
            // the very first message).
            let cursor = |k: &str| {
                args.get(k)
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
            };
            let mut q = match (cursor("after"), cursor("before")) {
                (Some(after), _) => format!("&after={}", seg(after)),
                (None, Some(before)) => format!("&before={}", seg(before)),
                (None, None) => "&tail=true".to_string(),
            };
            let limit = args.get("limit").and_then(i64_lenient).unwrap_or(50);
            q.push_str(&format!("&limit={limit}"));
            if let Some(sid) = args
                .get("session_id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                q.push_str(&format!("&session_id={}", seg(sid)));
            }
            SelfCall::get(format!(
                "/api/v1/agent-rooms/{}/messages?{}",
                seg(&room),
                q.trim_start_matches('&')
            ))
        }
        // Otto Assistant memory: one agent-tool endpoint per verb; the body
        // carries the (bound) calling session so chips land in its thread.
        "assistant_remember" | "assistant_forget" | "assistant_recall" => {
            let verb = tool.strip_prefix("assistant_").unwrap_or(tool);
            let mut body = serde_json::Map::new();
            for k in ["text", "kind", "tags", "query", "k", "session_id"] {
                if let Some(v) = args.get(k).filter(|v| !v.is_null()) {
                    body.insert(k.to_string(), v.clone());
                }
            }
            if verb == "remember" {
                arg_str(args, "text")?;
            }
            if verb == "forget" {
                arg_str(args, "query")?;
            }
            SelfCall::post(
                format!("/api/v1/assistant/agent/{verb}"),
                Value::Object(body),
            )
        }
        "list_scheduled_tasks" => {
            let ws = arg_str(args, "workspace_id")?;
            SelfCall::get(format!("/api/v1/workspaces/{}/scheduled-tasks", seg(&ws)))
        }
        "get_scheduled_task" => {
            let id = arg_str(args, "task_id")?;
            SelfCall::get(format!("/api/v1/scheduled-tasks/{}", seg(&id)))
        }
        "list_scheduled_task_runs" => {
            let id = arg_str(args, "task_id")?;
            SelfCall::get(format!("/api/v1/scheduled-tasks/{}/runs", seg(&id)))
        }
        "create_scheduled_task" => {
            let ws = arg_str(args, "workspace_id")?;
            let mut body = args.clone();
            if let Some(o) = body.as_object_mut() {
                o.remove("workspace_id");
            }
            SelfCall::post(
                format!("/api/v1/workspaces/{}/scheduled-tasks", seg(&ws)),
                body,
            )
        }
        "update_scheduled_task" => {
            let id = arg_str(args, "task_id")?;
            let mut body = args.clone();
            if let Some(o) = body.as_object_mut() {
                o.remove("task_id");
                // Filled by the choke point (the task's own workspace, for the
                // pin + approval scope) — not an updatable field.
                o.remove("workspace_id");
            }
            SelfCall::patch(format!("/api/v1/scheduled-tasks/{}", seg(&id)), body)
        }
        "set_scheduled_task_enabled" => {
            let id = arg_str(args, "task_id")?;
            let enabled = args.get("enabled").and_then(Value::as_bool).unwrap_or(true);
            SelfCall::patch(
                format!("/api/v1/scheduled-tasks/{}", seg(&id)),
                json!({"enabled": enabled}),
            )
        }
        "run_scheduled_task" => {
            let id = arg_str(args, "task_id")?;
            SelfCall::post(
                format!("/api/v1/scheduled-tasks/{}/run", seg(&id)),
                json!({}),
            )
        }
        "delete_scheduled_task" => {
            let id = arg_str(args, "task_id")?;
            SelfCall::delete(format!("/api/v1/scheduled-tasks/{}", seg(&id)))
        }
        // ---- AWS console (docs/design/aws-k8s-consoles.md §2) ----
        "aws_list_accounts" => SelfCall::get("/api/v1/aws/accounts".into()),
        "aws_s3_list_buckets" => SelfCall::get(format!(
            "/api/v1/aws/accounts/{}/s3/buckets?{}",
            seg(&arg_str(args, "account_id")?),
            opt_query(args, &[("region", "region")])
        )),
        "aws_s3_list_objects" => SelfCall::get(format!(
            "/api/v1/aws/accounts/{}/s3/buckets/{}/objects?{}",
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
        )),
        "aws_s3_preview" => {
            let extra = opt_query(args, &[("max_bytes", "max_bytes"), ("region", "region")]);
            let mut path = format!(
                "/api/v1/aws/accounts/{}/s3/buckets/{}/preview?key={}",
                seg(&arg_str(args, "account_id")?),
                seg(&arg_str(args, "bucket")?),
                seg(&arg_str(args, "key")?)
            );
            if !extra.is_empty() {
                path.push('&');
                path.push_str(&extra);
            }
            SelfCall::get(path)
        }
        "aws_sqs_list_queues" => SelfCall::get(format!(
            "/api/v1/aws/accounts/{}/sqs/queues?{}",
            seg(&arg_str(args, "account_id")?),
            opt_query(args, &[("prefix", "prefix"), ("region", "region")])
        )),
        "aws_sqs_peek" => {
            // receive-message with visibility timeout pinned to 0 (nothing
            // hidden; the receive count still rises); `max` clamped to 1..10.
            let mut body = json!({"url": arg_str(args, "url")?, "visibility_timeout": 0});
            if let Some(max) = args.get("max").and_then(u64_lenient) {
                body["max"] = json!(max.clamp(1, 10));
            }
            SelfCall::post(
                format!(
                    "/api/v1/aws/accounts/{}/sqs/queues/peek?{}",
                    seg(&arg_str(args, "account_id")?),
                    opt_query(args, &[("region", "region")])
                ),
                body,
            )
        }
        "aws_sqs_send" => {
            let mut body = json!({"url": arg_str(args, "url")?, "body": arg_str(args, "body")?});
            if let Some(d) = args.get("delay_seconds").and_then(u64_lenient) {
                body["delay_seconds"] = json!(d);
            }
            for k in ["group_id", "dedup_id"] {
                if let Some(v) = args
                    .get(k)
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                {
                    body[k] = json!(v);
                }
            }
            if let Some(attrs) = args.get("message_attributes").filter(|v| v.is_object()) {
                body["message_attributes"] = attrs.clone();
            }
            SelfCall::post(
                format!(
                    "/api/v1/aws/accounts/{}/sqs/queues/send?{}",
                    seg(&arg_str(args, "account_id")?),
                    opt_query(args, &[("region", "region")])
                ),
                body,
            )
        }
        "aws_ec2_list_instances" => SelfCall::get(format!(
            "/api/v1/aws/accounts/{}/ec2/instances?{}",
            seg(&arg_str(args, "account_id")?),
            opt_query(
                args,
                &[("region", "region"), ("state", "state"), ("q", "q")]
            )
        )),
        "aws_athena_list_tables" => {
            let extra = opt_query(args, &[("catalog", "catalog"), ("region", "region")]);
            let mut path = format!(
                "/api/v1/aws/accounts/{}/athena/tables?database={}",
                seg(&arg_str(args, "account_id")?),
                seg(&arg_str(args, "database")?)
            );
            if !extra.is_empty() {
                path.push('&');
                path.push_str(&extra);
            }
            SelfCall::get(path)
        }
        "aws_athena_query" => {
            let mut body = json!({"sql": arg_str(args, "sql")?});
            for k in ["database", "workgroup", "output_location"] {
                if let Some(v) = args
                    .get(k)
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                {
                    body[k] = json!(v);
                }
            }
            SelfCall::post(
                format!(
                    "/api/v1/aws/accounts/{}/athena/query?{}",
                    seg(&arg_str(args, "account_id")?),
                    opt_query(args, &[("region", "region")])
                ),
                body,
            )
        }
        "aws_athena_get_query" => SelfCall::get(format!(
            "/api/v1/aws/accounts/{}/athena/query/{}?{}",
            seg(&arg_str(args, "account_id")?),
            seg(&arg_str(args, "query_execution_id")?),
            opt_query(
                args,
                &[("token", "token"), ("max", "max"), ("region", "region")]
            )
        )),
        "aws_eks_list_clusters" => SelfCall::get(format!(
            "/api/v1/aws/accounts/{}/eks/clusters?{}",
            seg(&arg_str(args, "account_id")?),
            opt_query(args, &[("region", "region")])
        )),
        "aws_logs_list_groups" => SelfCall::get(format!(
            "/api/v1/aws/accounts/{}/logs/groups?{}",
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
        )),
        "aws_logs_filter" => {
            let extra = opt_query(
                args,
                &[
                    ("pattern", "pattern"),
                    ("streams", "streams"),
                    ("start", "start"),
                    ("end", "end"),
                    ("token", "token"),
                    ("max", "max"),
                    ("region", "region"),
                ],
            );
            let mut path = format!(
                "/api/v1/aws/accounts/{}/logs/events?group={}",
                seg(&arg_str(args, "account_id")?),
                seg(&arg_str(args, "group")?)
            );
            if !extra.is_empty() {
                path.push('&');
                path.push_str(&extra);
            }
            SelfCall::get(path)
        }
        "aws_logs_insights" => {
            let mut body = json!({
                "groups": args.get("groups").cloned().unwrap_or(json!([])),
                "query": arg_str(args, "query")?,
                "start": args.get("start").and_then(Value::as_i64).unwrap_or(0),
                "end": args.get("end").and_then(Value::as_i64).unwrap_or(0),
            });
            if let Some(limit) = args.get("limit").and_then(Value::as_u64) {
                body["limit"] = json!(limit);
            }
            SelfCall::post(
                format!(
                    "/api/v1/aws/accounts/{}/logs/insights?{}",
                    seg(&arg_str(args, "account_id")?),
                    opt_query(args, &[("region", "region")])
                ),
                body,
            )
        }
        "aws_logs_get_insights" => SelfCall::get(format!(
            "/api/v1/aws/accounts/{}/logs/insights/{}?{}",
            seg(&arg_str(args, "account_id")?),
            seg(&arg_str(args, "query_id")?),
            opt_query(args, &[("region", "region")])
        )),
        // ---- Kubernetes console (§3) — `namespace` → `ns`; omitted ⇒ all (-A) ----
        "k8s_list_clusters" => SelfCall::get("/api/v1/k8s/clusters".into()),
        "k8s_get_resources" => {
            let extra = opt_query(args, &[("ns", "namespace"), ("label", "label"), ("q", "q")]);
            let mut path = format!(
                "/api/v1/k8s/clusters/{}/resources?kind={}",
                seg(&arg_str(args, "cluster_id")?),
                seg(&arg_str(args, "kind")?)
            );
            if !extra.is_empty() {
                path.push('&');
                path.push_str(&extra);
            }
            SelfCall::get(path)
        }
        // `ns` is omitted for cluster-scoped kinds (nodes, namespaces); the
        // route requires it for namespaced ones and says so.
        "k8s_describe" => {
            let mut path = format!(
                "/api/v1/k8s/clusters/{}/resource?kind={}&name={}",
                seg(&arg_str(args, "cluster_id")?),
                seg(&arg_str(args, "kind")?),
                seg(&arg_str(args, "name")?)
            );
            if let Some(ns) = args
                .get("namespace")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                path.push_str(&format!("&ns={}", seg(ns)));
            }
            SelfCall::get(path)
        }
        // text/plain route — `run_tool` wraps the body as `{text}`; `follow` is
        // deliberately never forwarded (a stream would hang the call).
        "k8s_logs" => SelfCall::get(format!(
            "/api/v1/k8s/clusters/{}/pods/{}/{}/logs?{}",
            seg(&arg_str(args, "cluster_id")?),
            seg(&arg_str(args, "namespace")?),
            seg(&arg_str(args, "pod")?),
            opt_query(
                args,
                &[
                    ("container", "container"),
                    ("tail", "tail"),
                    ("since", "since"),
                    ("previous", "previous"),
                    ("timestamps", "timestamps"),
                ]
            )
        )),
        "k8s_top" => SelfCall::get(format!(
            "/api/v1/k8s/clusters/{}/metrics?{}",
            seg(&arg_str(args, "cluster_id")?),
            opt_query(args, &[("ns", "namespace")])
        )),
        "k8s_health" => SelfCall::get(format!(
            "/api/v1/k8s/clusters/{}/monitor/health?{}",
            seg(&arg_str(args, "cluster_id")?),
            opt_query(args, &[("window", "window")])
        )),
        "k8s_action" => SelfCall::post(
            format!(
                "/api/v1/k8s/clusters/{}/actions",
                seg(&arg_str(args, "cluster_id")?)
            ),
            json!({
                "action": arg_str(args, "action")?,
                "kind": arg_str(args, "kind")?,
                "ns": arg_str(args, "namespace")?,
                "name": arg_str(args, "name")?,
                "params": args.get("params").cloned().unwrap_or(json!({})),
            }),
        ),
        "k8s_pod_actions_list" => SelfCall::get(format!(
            "/api/v1/k8s/clusters/{}/pod-actions?{}",
            seg(&arg_str(args, "cluster_id")?),
            opt_query(
                args,
                &[
                    ("namespace", "namespace"),
                    ("workload_kind", "workload_kind"),
                    ("workload", "workload")
                ]
            )
        )),
        "k8s_pod_http" => {
            let mut body = json!({
                "namespace": arg_str(args, "namespace")?,
                "port": args
                    .get("port")
                    .and_then(u64_lenient)
                    .ok_or_else(|| Error::Invalid("missing argument 'port'".into()))?,
                "method": arg_str(args, "method")?,
                "path": arg_str(args, "path")?,
            });
            // Forwarded verbatim: the route owns validation and the prod
            // confirm_name guard.
            for k in [
                "pod",
                "workload",
                "headers",
                "body",
                "timeout_ms",
                "max_concurrency",
                "confirm_name",
            ] {
                if let Some(v) = args.get(k).filter(|v| !v.is_null()) {
                    body[k] = v.clone();
                }
            }
            SelfCall::post(
                format!(
                    "/api/v1/k8s/clusters/{}/pod-http",
                    seg(&arg_str(args, "cluster_id")?)
                ),
                body,
            )
        }
        other => return Err(Error::Invalid(format!("unknown otto tool '{other}'"))),
    })
}

/// Tools whose route answers `text/plain` rather than JSON; [`run_tool`] wraps
/// the body as `{"text": …}` for them (today only the pod-logs route).
pub const TEXT_TOOLS: &[&str] = &["k8s_logs"];
/// Cap on the text handed back for a [`TEXT_TOOLS`] call (keeps the tail —
/// the newest log lines). The route's own non-follow cap is 5 MiB.
pub const MAX_TEXT_CHARS: usize = 256 * 1024;

/// Resolve `(tool, args)` to a self-call and execute it as the user. Thin wrapper
/// over the pure [`route_for`] so the routing of every tool is unit-tested.
pub async fn run_tool(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    tool: &str,
    args: &Value,
) -> Result<Value, Error> {
    let call = route_for(tool, args)?;
    let url = format!("{base}{}", call.path);
    let empty = json!({});
    let body = call.body.as_ref().unwrap_or(&empty);
    if TEXT_TOOLS.contains(&tool) && call.method == Method::Get {
        return self_get_text(client, token, &url).await;
    }
    match call.method {
        Method::Get => self_get(client, token, &url).await,
        Method::Post => self_post(client, token, &url, body).await,
        Method::Put => self_put(client, token, &url, body).await,
        Method::Patch => self_patch(client, token, &url, body).await,
        Method::Delete => self_delete(client, token, &url).await,
    }
}

pub async fn self_get(client: &reqwest::Client, token: &str, url: &str) -> Result<Value, Error> {
    let resp = client
        .get(url)
        .bearer_auth(token)
        .header("X-Otto-Agent", "mcp-outward")
        .send()
        .await
        .map_err(|e| Error::Upstream(format!("self-call: {e}")))?;
    parse_self(resp).await
}
pub async fn self_post(
    client: &reqwest::Client,
    token: &str,
    url: &str,
    body: &Value,
) -> Result<Value, Error> {
    let resp = client
        .post(url)
        .bearer_auth(token)
        .header("X-Otto-Agent", "mcp-outward")
        .json(body)
        .send()
        .await
        .map_err(|e| Error::Upstream(format!("self-call: {e}")))?;
    parse_self(resp).await
}
pub async fn self_put(
    client: &reqwest::Client,
    token: &str,
    url: &str,
    body: &Value,
) -> Result<Value, Error> {
    let resp = client
        .put(url)
        .bearer_auth(token)
        .header("X-Otto-Agent", "mcp-outward")
        .json(body)
        .send()
        .await
        .map_err(|e| Error::Upstream(format!("self-call: {e}")))?;
    parse_self(resp).await
}
pub async fn self_patch(
    client: &reqwest::Client,
    token: &str,
    url: &str,
    body: &Value,
) -> Result<Value, Error> {
    let resp = client
        .patch(url)
        .bearer_auth(token)
        .header("X-Otto-Agent", "mcp-outward")
        .json(body)
        .send()
        .await
        .map_err(|e| Error::Upstream(format!("self-call: {e}")))?;
    parse_self(resp).await
}
pub async fn self_delete(client: &reqwest::Client, token: &str, url: &str) -> Result<Value, Error> {
    let resp = client
        .delete(url)
        .bearer_auth(token)
        .header("X-Otto-Agent", "mcp-outward")
        .send()
        .await
        .map_err(|e| Error::Upstream(format!("self-call: {e}")))?;
    parse_self(resp).await
}
pub async fn parse_self(resp: reqwest::Response) -> Result<Value, Error> {
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(Error::Upstream(self_call_error(status, &text)));
    }
    Ok(parse_self_ok(&text))
}

/// Cap on a self-call error MESSAGE handed back to the agent. The message is
/// often the actionable part (a reference that did not resolve lists the
/// candidates, a validation error names the bad field), so it is kept whole up
/// to this bound — the old 400-char raw snippet cut candidate lists mid-row.
pub const MAX_ERROR_MESSAGE_CHARS: usize = 4000;

/// The agent-facing text of a non-2xx self-call: `"<status>: <message>"`, the
/// `message` (or a module's `error`) of a JSON problem body when there is one
/// (≤ [`MAX_ERROR_MESSAGE_CHARS`]), else a short raw snippet so an HTML/huge
/// body never floods the transcript. Pure, so it is unit-tested.
pub fn self_call_error(status: reqwest::StatusCode, body: &str) -> String {
    let message = serde_json::from_str::<Value>(body).ok().and_then(|v| {
        v.get("message")
            .or_else(|| v.get("error"))
            .and_then(Value::as_str)
            .map(str::to_string)
    });
    match message {
        Some(m) => format!(
            "{status}: {}",
            m.chars().take(MAX_ERROR_MESSAGE_CHARS).collect::<String>()
        ),
        None => format!("{status}: {}", body.chars().take(400).collect::<String>()),
    }
}

/// A 2xx self-call body as JSON. An empty body (`204 No Content` — a Jira
/// transition, a delete) is `{"ok": true}`, not `null`, so the agent reads a
/// success as one.
pub fn parse_self_ok(body: &str) -> Value {
    if body.trim().is_empty() {
        return json!({ "ok": true });
    }
    serde_json::from_str(body).unwrap_or(Value::Null)
}
/// GET a `text/plain` route (pod logs) and wrap it as `{text, truncated}`,
/// keeping the newest [`MAX_TEXT_CHARS`] — `parse_self` would turn a non-JSON
/// body into `null`.
pub async fn self_get_text(
    client: &reqwest::Client,
    token: &str,
    url: &str,
) -> Result<Value, Error> {
    let resp = client
        .get(url)
        .bearer_auth(token)
        .header("X-Otto-Agent", "mcp-outward")
        .send()
        .await
        .map_err(|e| Error::Upstream(format!("self-call: {e}")))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(Error::Upstream(self_call_error(status, &text)));
    }
    let n = text.chars().count();
    let (text, truncated) = if n > MAX_TEXT_CHARS {
        let start = text
            .char_indices()
            .nth(n - MAX_TEXT_CHARS)
            .map(|(i, _)| i)
            .unwrap_or(text.len());
        (text[start..].to_string(), true)
    } else {
        (text, false)
    };
    Ok(json!({"text": text, "truncated": truncated}))
}
