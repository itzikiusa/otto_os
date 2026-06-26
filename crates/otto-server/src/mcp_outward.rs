//! "Otto as an MCP server" — the OUTWARD surface. External agents (Claude Code,
//! Copilot, …) connect to `ottod mcp-server` over stdio with a **restricted**
//! `kind='mcp'` token and call the eight `otto.*` tools. Every call funnels
//! through `POST /mcp/otto-tools/invoke` (the only route that token may reach —
//! see feature_guard, design §14 F1), which governs (enabled? allowlisted?
//! dangerous→approval?), audits (`mcp_call_log`, direction='inbound'), then
//! executes the capability **as the token's user** by self-calling the real
//! endpoint with a short-lived ephemeral token — so each tool reuses its
//! endpoint's native RBAC (no privilege escalation). It also hosts the live-agent
//! **gateway** (`/mcp/gateway/*`).

use std::time::Duration;

use axum::extract::{Query, State};
use axum::Json;
use otto_core::{Error, Id};
use otto_mcp::{canonical_hash, InvokeCtx};
use otto_rbac::AuthRepo;
use otto_state::{NewApproval, NewCallLog, SettingsRepo};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::CurrentUser;
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

const DEFAULT_ENABLED: &[&str] = &[
    "get_context_packet",
    "get_proof_pack",
    "ask_human_approval",
    // Scheduled-tasks reads are safe to expose by default so an agent can inspect
    // existing jobs; the write tools below stay off until an admin enables them.
    "list_scheduled_tasks",
    "list_scheduled_task_runs",
];
const DANGEROUS: &[&str] = &[
    "run_goal_loop",
    "create_work_item",
    // Creating/altering/running a recurring autonomous job that triggers agents and
    // posts to an external destination is approval-gated (off by default).
    "create_scheduled_task",
    "update_scheduled_task",
    "delete_scheduled_task",
    "run_scheduled_task",
    "set_scheduled_task_enabled",
];
const MAX_WAIT_SECS: u64 = 30;

/// Static catalog of the eight outward tools.
pub fn otto_tool_specs() -> Vec<Value> {
    vec![
        json!({"name":"otto.search_codebase","mutating":false,
            "description":"Search a workspace's code for a literal query; returns file:line matches. Read-only, confined to the workspace root.",
            "inputSchema":{"type":"object","required":["workspace_id","query"],"properties":{
                "workspace_id":{"type":"string"},"query":{"type":"string"},
                "path":{"type":"string","description":"optional sub-path within the workspace"},
                "max_results":{"type":"integer"}}}}),
        json!({"name":"otto.get_context_packet","mutating":false,
            "description":"Assemble a code-grounded context packet for a workspace: metadata + the most relevant code excerpts for a query.",
            "inputSchema":{"type":"object","required":["workspace_id"],"properties":{
                "workspace_id":{"type":"string"},"query":{"type":"string"},"story_id":{"type":"string"}}}}),
        json!({"name":"otto.run_goal_loop","mutating":true,
            "description":"Create and start a bounded goal loop (Plan→Execute→Evaluate→Digest). Pass a goal-loop spec. DANGEROUS: spawns autonomous agents — approval-gated.",
            "inputSchema":{"type":"object","required":["workspace_id","name","repo_path","definition","limits","config"],"properties":{
                "workspace_id":{"type":"string"},"name":{"type":"string"},"repo_path":{"type":"string"},
                "definition":{"type":"object"},"limits":{"type":"object"},"config":{"type":"object"}}}}),
        json!({"name":"otto.create_work_item","mutating":true,
            "description":"Create a work item (a Swarm task) under a project. DANGEROUS: mutates project state — approval-gated.",
            "inputSchema":{"type":"object","required":["project_id","title"],"properties":{
                "project_id":{"type":"string"},"title":{"type":"string"},
                "description":{"type":"string"},"priority":{"type":"string"}}}}),
        json!({"name":"otto.query_db_readonly","mutating":false,
            "description":"Run a READ-ONLY SQL query against an Otto DB connection. Writes/DDL and multi-statement input are rejected server-side regardless of the connection's guard.",
            "inputSchema":{"type":"object","required":["connection_id","statement"],"properties":{
                "connection_id":{"type":"string"},"statement":{"type":"string"},"max_rows":{"type":"integer"}}}}),
        json!({"name":"otto.open_pr_draft","mutating":false,
            "description":"Draft a PR title + description from a repo's diff vs a base branch. Drafts text only — does NOT open/publish a PR.",
            "inputSchema":{"type":"object","required":["repo_id","base"],"properties":{
                "repo_id":{"type":"string"},"base":{"type":"string"}}}}),
        json!({"name":"otto.get_proof_pack","mutating":false,
            "description":"Assemble an evidence bundle for a target: git status/recent-commits/diffstat for a repo and a goal loop's machine-checked acceptance criteria.",
            "inputSchema":{"type":"object","required":["workspace_id"],"properties":{
                "workspace_id":{"type":"string"},"repo_id":{"type":"string"},
                "branch":{"type":"string"},"goal_loop_id":{"type":"string"}}}}),
        json!({"name":"otto.ask_human_approval","mutating":false,
            "description":"Request a human's approval for an action and (optionally) wait for the decision. Creates a pending item in the MCP approval queue.",
            "inputSchema":{"type":"object","required":["title"],"properties":{
                "workspace_id":{"type":"string"},"title":{"type":"string"},
                "detail":{"type":"string"},"wait_seconds":{"type":"integer"}}}}),
        // ---- Scheduled Tasks ----
        json!({"name":"otto.list_scheduled_tasks","mutating":false,
            "description":"List a workspace's scheduled tasks (recurring agent jobs). Read-only.",
            "inputSchema":{"type":"object","required":["workspace_id"],"properties":{
                "workspace_id":{"type":"string"}}}}),
        json!({"name":"otto.list_scheduled_task_runs","mutating":false,
            "description":"List the recent run history (status + summary) of a scheduled task. Read-only.",
            "inputSchema":{"type":"object","required":["task_id"],"properties":{
                "task_id":{"type":"string"}}}}),
        json!({"name":"otto.create_scheduled_task","mutating":true,
            "description":"Create a scheduled task: a recurring job that runs an agent (or hands off to a workflow) on a cadence, writes a Markdown report, and delivers it to a destination. DANGEROUS: an autonomous recurring capability — approval-gated. `schedule` = {cadence:'interval'|'daily'|'weekly'|'cron', every_min, at:'HH:MM', weekday, expr:'<5-field cron>'} interpreted in `timezone` (IANA). `provider` = claude|codex|agy|shell|<custom>. `kind` = 'agent_prompt'|'workflow' (workflow requires workflow_id). `sandbox` = 'none'|'worktree'. `max_retries` 0..5. `notify_on_change` only delivers when the report changes. `attach_proof` builds a proof pack. `destination` = {type:'none'|'slack'|'telegram'|'email'|'webhook', ...}.",
            "inputSchema":{"type":"object","required":["workspace_id","name","prompt"],"properties":{
                "workspace_id":{"type":"string"},"name":{"type":"string"},"prompt":{"type":"string"},
                "kind":{"type":"string"},"provider":{"type":"string"},"model":{"type":"string"},
                "schedule":{"type":"object"},"destination":{"type":"object"},"timezone":{"type":"string"},
                "workflow_id":{"type":"string"},"sandbox":{"type":"string"},"max_retries":{"type":"integer"},
                "notify_on_change":{"type":"boolean"},"attach_proof":{"type":"boolean"},
                "cwd":{"type":"string"},"skill":{"type":"string"},"enabled":{"type":"boolean"}}}}),
        json!({"name":"otto.update_scheduled_task","mutating":true,
            "description":"Update a scheduled task's fields (name/prompt/schedule/destination/provider/timezone/sandbox/max_retries/notify_on_change/attach_proof/workflow_id/skill/enabled). DANGEROUS — approval-gated.",
            "inputSchema":{"type":"object","required":["task_id"],"properties":{
                "task_id":{"type":"string"},"name":{"type":"string"},"prompt":{"type":"string"},
                "provider":{"type":"string"},"schedule":{"type":"object"},"destination":{"type":"object"},
                "timezone":{"type":"string"},"workflow_id":{"type":"string"},"sandbox":{"type":"string"},
                "max_retries":{"type":"integer"},"notify_on_change":{"type":"boolean"},
                "attach_proof":{"type":"boolean"},"skill":{"type":"string"},"enabled":{"type":"boolean"}}}}),
        json!({"name":"otto.set_scheduled_task_enabled","mutating":true,
            "description":"Enable or disable a scheduled task. DANGEROUS — approval-gated.",
            "inputSchema":{"type":"object","required":["task_id","enabled"],"properties":{
                "task_id":{"type":"string"},"enabled":{"type":"boolean"}}}}),
        json!({"name":"otto.run_scheduled_task","mutating":true,
            "description":"Run a scheduled task once now (does not change its schedule). Returns the run. DANGEROUS — approval-gated.",
            "inputSchema":{"type":"object","required":["task_id"],"properties":{
                "task_id":{"type":"string"}}}}),
        json!({"name":"otto.delete_scheduled_task","mutating":true,
            "description":"Delete a scheduled task and its run history. DANGEROUS — approval-gated.",
            "inputSchema":{"type":"object","required":["task_id"],"properties":{
                "task_id":{"type":"string"}}}}),
    ]
}

async fn enabled_tools(ctx: &ServerCtx) -> Vec<String> {
    SettingsRepo::new(ctx.pool.clone())
        .get("mcp_otto_server_tools")
        .await
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_value::<Vec<String>>(v).ok())
        .unwrap_or_else(|| DEFAULT_ENABLED.iter().map(|s| s.to_string()).collect())
}

async fn outward_enabled(ctx: &ServerCtx) -> bool {
    SettingsRepo::new(ctx.pool.clone())
        .get("mcp_otto_server_enabled")
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

async fn require_approval_dangerous(ctx: &ServerCtx) -> bool {
    SettingsRepo::new(ctx.pool.clone())
        .get("mcp_require_approval_dangerous")
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

/// Build the human-facing approval detail. For scheduled-task create/update it
/// surfaces the prompt + cadence + destination so the approver knows exactly what
/// recurring autonomous capability they are granting (security review fix).
fn dangerous_detail(tool: &str, args: &Value) -> String {
    let short = tool.strip_prefix("otto.").unwrap_or(tool);
    match short {
        "create_scheduled_task" | "update_scheduled_task" => {
            let name = args.get("name").and_then(Value::as_str).unwrap_or("(unnamed)");
            let sched = args.get("schedule");
            let cadence = sched
                .and_then(|s| s.get("cadence"))
                .and_then(Value::as_str)
                .unwrap_or("interval");
            let cad = match sched.and_then(|s| s.get("every_min")).and_then(Value::as_i64) {
                Some(m) => format!("{cadence} (every {m} min)"),
                None => cadence.to_string(),
            };
            let dest = args
                .get("destination")
                .and_then(|d| d.get("type"))
                .and_then(Value::as_str)
                .unwrap_or("none");
            let prompt: String = args
                .get("prompt")
                .and_then(Value::as_str)
                .unwrap_or("")
                .chars()
                .take(160)
                .collect();
            format!(
                "Recurring agent job '{name}' — cadence: {cad}; destination: {dest}; prompt: {prompt}"
            )
        }
        _ => format!("External agent requests the dangerous tool '{tool}'."),
    }
}

// ===========================================================================
// POST /mcp/otto-tools/invoke  (the governed choke point for the 8 tools)
// ===========================================================================

#[derive(Deserialize)]
pub struct OttoInvokeReq {
    pub tool: String,
    #[serde(default)]
    pub arguments: Value,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub wait_seconds: Option<u64>,
}

pub async fn otto_tools_invoke(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<OttoInvokeReq>,
) -> ApiResult<Json<Value>> {
    let short = req.tool.strip_prefix("otto.").unwrap_or(&req.tool).to_string();
    let mut audit = NewCallLog {
        tool: req.tool.clone(),
        direction: "inbound".into(),
        server_name: Some("otto".into()),
        caller_user_id: Some(user.id.clone()),
        caller_kind: Some("mcp_server".into()),
        args_redacted_json: otto_core::redact::redact_json(&req.arguments).value.to_string(),
        ..Default::default()
    };

    if !outward_enabled(&ctx).await {
        return Ok(Json(deny_audit(&ctx, &mut audit, "the Otto MCP server is disabled").await));
    }
    if !enabled_tools(&ctx).await.contains(&short) {
        return Ok(Json(deny_audit(&ctx, &mut audit, "this tool is not enabled on the Otto MCP server").await));
    }

    let dangerous = DANGEROUS.contains(&short.as_str());
    let needs_approval = dangerous && require_approval_dangerous(&ctx).await;
    let args_hash = canonical_hash(&req.arguments);
    let ws = req.arguments.get("workspace_id").and_then(Value::as_str).map(str::to_string);

    if needs_approval && !req.dry_run {
        match ctx
            .mcp
            .approvals()
            .find_usable(ws.as_deref(), None, &req.tool, &args_hash)
            .await
            .map_err(ApiError)?
        {
            Some(appr_id) => {
                if !ctx.mcp.approvals().consume(&appr_id).await.map_err(ApiError)? {
                    return Ok(Json(deny_audit(&ctx, &mut audit, "approval already used").await));
                }
                audit.approval_id = Some(appr_id);
            }
            None => {
                let appr = ctx
                    .mcp
                    .approvals()
                    .create(NewApproval {
                        workspace_id: ws.clone(),
                        kind: "tool_call".into(),
                        server_id: None,
                        server_name: Some("otto".into()),
                        tool: Some(req.tool.clone()),
                        title: format!("otto MCP server → {}", req.tool),
                        detail: Some(dangerous_detail(&req.tool, &req.arguments)),
                        args_redacted_json: audit.args_redacted_json.clone(),
                        args_hash: Some(args_hash.clone()),
                        risk_label: Some("dangerous".into()),
                        requested_by: Some(user.id.clone()),
                        requested_by_kind: Some("mcp_server".into()),
                        expires_at: Some((chrono::Utc::now() + chrono::Duration::minutes(120)).to_rfc3339()),
                    })
                    .await
                    .map_err(ApiError)?;
                match wait_for_decision(&ctx, &appr.id, req.wait_seconds).await {
                    Some(true) => {
                        let _ = ctx.mcp.approvals().consume(&appr.id).await;
                        audit.approval_id = Some(appr.id.clone());
                    }
                    Some(false) => {
                        audit.decision = "denied".into();
                        audit.decision_reason = Some("human denied the request".into());
                        let _ = ctx.mcp.call_log().insert(audit).await;
                        return Ok(Json(json!({"decision":"denied","executed":false,"reason":"human denied the request"})));
                    }
                    None => {
                        audit.decision = "pending_approval".into();
                        audit.approval_id = Some(appr.id.clone());
                        let _ = ctx.mcp.call_log().insert(audit).await;
                        return Ok(Json(json!({"decision":"pending_approval","executed":false,
                            "approval_id":appr.id,"reason":"awaiting human approval — resubmit after it is approved"})));
                    }
                }
            }
        }
    }

    if req.dry_run {
        audit.decision = "dry_run".into();
        audit.dry_run = true;
        audit.ok = true;
        let _ = ctx.mcp.call_log().insert(audit).await;
        return Ok(Json(json!({"decision":"dry_run","executed":false,"dry_run":true,
            "preview":{"tool":req.tool,"arguments":otto_core::redact::redact_json(&req.arguments).value,
                       "note":"dry-run: the tool was NOT executed"}})));
    }

    // Fail-closed audit: insert before executing.
    audit.decision = if audit.approval_id.is_some() { "approved".into() } else { "allowed".into() };
    let audit_id = ctx.mcp.call_log().insert(audit).await.map_err(ApiError)?;

    let started = std::time::Instant::now();
    let result = execute_otto_tool(&ctx, &user, &short, &req.arguments).await;
    let latency = started.elapsed().as_millis() as i64;
    match result {
        Ok(value) => {
            let bytes = serde_json::to_vec(&value).map(|v| v.len() as i64).unwrap_or(0);
            let _ = ctx.mcp.call_log().finalize(&audit_id, true, None, Some(latency), Some(bytes), None).await;
            Ok(Json(json!({"decision":"allowed","executed":true,"content":value})))
        }
        Err(e) => {
            let err = otto_core::redact::redact_text(&e.to_string()).value;
            let _ = ctx.mcp.call_log().finalize(&audit_id, false, Some(&err), Some(latency), None, None).await;
            Ok(Json(json!({"decision":"error","executed":true,"is_error":true,"content":{"error":err}})))
        }
    }
}

async fn deny_audit(ctx: &ServerCtx, audit: &mut NewCallLog, reason: &str) -> Value {
    audit.decision = "denied".into();
    audit.decision_reason = Some(reason.to_string());
    let _ = ctx.mcp.call_log().insert(audit.clone()).await;
    json!({"decision":"denied","executed":false,"reason":reason})
}

/// Poll an approval up to a bounded wait. `Some(true)`=approved, `Some(false)`=denied,
/// `None`=still pending after the wait (caller resubmits later).
async fn wait_for_decision(ctx: &ServerCtx, approval_id: &str, wait_seconds: Option<u64>) -> Option<bool> {
    let budget = wait_seconds.unwrap_or(0).min(MAX_WAIT_SECS);
    let mut waited = 0u64;
    loop {
        if let Ok(a) = ctx.mcp.approvals().get(&approval_id.to_string()).await {
            match a.status.as_str() {
                "approved" => return Some(true),
                "denied" | "expired" | "cancelled" => return Some(false),
                _ => {}
            }
        }
        if waited >= budget {
            return None;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
        waited += 1;
    }
}

// ===========================================================================
// The executor — runs each tool AS the user via an ephemeral self-call.
// ===========================================================================

fn is_read_only_sql(stmt: &str) -> bool {
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

async fn execute_otto_tool(
    ctx: &ServerCtx,
    user: &otto_core::domain::User,
    tool: &str,
    args: &Value,
) -> Result<Value, Error> {
    if tool == "ask_human_approval" {
        return ask_human_approval(ctx, user, args).await;
    }
    // Mint a short-lived ephemeral token so the self-call reuses the target
    // endpoint's native RBAC; revoke it on the way out.
    let (token, _) = AuthRepo::new(ctx.pool.clone())
        .issue_api_token(&user.id, Some("mcp-otto-exec"))
        .await?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| Error::Internal(format!("http client: {e}")))?;
    let base = ctx.base_url.trim_end_matches('/').to_string();
    let result = run_tool(&client, &base, &token, tool, args).await;
    let _ = AuthRepo::new(ctx.pool.clone()).revoke(&token).await;
    result
}

fn arg_str(args: &Value, key: &str) -> Result<String, Error> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| Error::Invalid(format!("missing required string argument '{key}'")))
}

fn seg(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

async fn run_tool(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    tool: &str,
    args: &Value,
) -> Result<Value, Error> {
    match tool {
        "search_codebase" => {
            let ws = arg_str(args, "workspace_id")?;
            let q = arg_str(args, "query")?;
            let mut url = format!("{base}/api/v1/workspaces/{}/mcp/code-search?q={}", seg(&ws), seg(&q));
            if let Some(p) = args.get("path").and_then(Value::as_str) {
                url.push_str(&format!("&path={}", seg(p)));
            }
            if let Some(m) = args.get("max_results").and_then(Value::as_u64) {
                url.push_str(&format!("&max={m}"));
            }
            self_get(client, token, &url).await
        }
        "get_context_packet" => {
            let ws = arg_str(args, "workspace_id")?;
            self_post(client, token, &format!("{base}/api/v1/workspaces/{}/mcp/context-packet", seg(&ws)), args).await
        }
        "get_proof_pack" => {
            let ws = arg_str(args, "workspace_id")?;
            let mut url = format!("{base}/api/v1/workspaces/{}/mcp/proof-pack?", seg(&ws));
            for k in ["repo_id", "branch", "goal_loop_id"] {
                if let Some(v) = args.get(k).and_then(Value::as_str) {
                    url.push_str(&format!("{k}={}&", seg(v)));
                }
            }
            self_get(client, token, &url).await
        }
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
            let body = json!({
                "statement": stmt,
                "max_rows": args.get("max_rows").and_then(Value::as_u64).unwrap_or(200),
                "confirm_write": false, // forced — never honored from the caller
            });
            self_post(client, token, &format!("{base}/api/v1/connections/{}/db/query", seg(&conn)), &body).await
        }
        "open_pr_draft" => {
            let repo = arg_str(args, "repo_id")?;
            let base_branch = arg_str(args, "base")?;
            self_post(client, token, &format!("{base}/api/v1/repos/{}/pr/draft", seg(&repo)), &json!({"base": base_branch})).await
        }
        "run_goal_loop" => {
            let ws = arg_str(args, "workspace_id")?;
            let mut body = args.clone();
            if let Some(obj) = body.as_object_mut() {
                obj.remove("workspace_id");
                obj.insert("autostart".into(), json!(true));
            }
            self_post(client, token, &format!("{base}/api/v1/workspaces/{}/goal-loops", seg(&ws)), &body).await
        }
        "create_work_item" => {
            let project = arg_str(args, "project_id")?;
            let body = json!({
                "title": arg_str(args, "title")?,
                "description": args.get("description").and_then(Value::as_str),
                "priority": args.get("priority").and_then(Value::as_str),
            });
            self_post(client, token, &format!("{base}/api/v1/swarm/projects/{}/tasks", seg(&project)), &body).await
        }
        "list_scheduled_tasks" => {
            let ws = arg_str(args, "workspace_id")?;
            self_get(client, token, &format!("{base}/api/v1/workspaces/{}/scheduled-tasks", seg(&ws))).await
        }
        "list_scheduled_task_runs" => {
            let id = arg_str(args, "task_id")?;
            self_get(client, token, &format!("{base}/api/v1/scheduled-tasks/{}/runs", seg(&id))).await
        }
        "create_scheduled_task" => {
            let ws = arg_str(args, "workspace_id")?;
            let mut body = args.clone();
            if let Some(o) = body.as_object_mut() {
                o.remove("workspace_id");
            }
            self_post(client, token, &format!("{base}/api/v1/workspaces/{}/scheduled-tasks", seg(&ws)), &body).await
        }
        "update_scheduled_task" => {
            let id = arg_str(args, "task_id")?;
            let mut body = args.clone();
            if let Some(o) = body.as_object_mut() {
                o.remove("task_id");
            }
            self_patch(client, token, &format!("{base}/api/v1/scheduled-tasks/{}", seg(&id)), &body).await
        }
        "set_scheduled_task_enabled" => {
            let id = arg_str(args, "task_id")?;
            let enabled = args.get("enabled").and_then(Value::as_bool).unwrap_or(true);
            self_patch(client, token, &format!("{base}/api/v1/scheduled-tasks/{}", seg(&id)), &json!({"enabled": enabled})).await
        }
        "run_scheduled_task" => {
            let id = arg_str(args, "task_id")?;
            self_post(client, token, &format!("{base}/api/v1/scheduled-tasks/{}/run", seg(&id)), &json!({})).await
        }
        "delete_scheduled_task" => {
            let id = arg_str(args, "task_id")?;
            self_delete(client, token, &format!("{base}/api/v1/scheduled-tasks/{}", seg(&id))).await
        }
        other => Err(Error::Invalid(format!("unknown otto tool '{other}'"))),
    }
}

async fn self_get(client: &reqwest::Client, token: &str, url: &str) -> Result<Value, Error> {
    let resp = client.get(url).bearer_auth(token).send().await
        .map_err(|e| Error::Upstream(format!("self-call: {e}")))?;
    parse_self(resp).await
}
async fn self_post(client: &reqwest::Client, token: &str, url: &str, body: &Value) -> Result<Value, Error> {
    let resp = client.post(url).bearer_auth(token).json(body).send().await
        .map_err(|e| Error::Upstream(format!("self-call: {e}")))?;
    parse_self(resp).await
}
async fn self_patch(client: &reqwest::Client, token: &str, url: &str, body: &Value) -> Result<Value, Error> {
    let resp = client.patch(url).bearer_auth(token).json(body).send().await
        .map_err(|e| Error::Upstream(format!("self-call: {e}")))?;
    parse_self(resp).await
}
async fn self_delete(client: &reqwest::Client, token: &str, url: &str) -> Result<Value, Error> {
    let resp = client.delete(url).bearer_auth(token).send().await
        .map_err(|e| Error::Upstream(format!("self-call: {e}")))?;
    parse_self(resp).await
}
async fn parse_self(resp: reqwest::Response) -> Result<Value, Error> {
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        let snippet: String = text.chars().take(400).collect();
        return Err(Error::Upstream(format!("{status}: {snippet}")));
    }
    Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
}

async fn ask_human_approval(ctx: &ServerCtx, user: &otto_core::domain::User, args: &Value) -> Result<Value, Error> {
    let title = arg_str(args, "title")?;
    let ws = args.get("workspace_id").and_then(Value::as_str).map(str::to_string);
    let detail = args.get("detail").and_then(Value::as_str).map(str::to_string);
    let appr = ctx
        .mcp
        .approvals()
        .create(NewApproval {
            workspace_id: ws,
            kind: "human_ask".into(),
            server_id: None,
            server_name: Some("otto".into()),
            tool: Some("otto.ask_human_approval".into()),
            title,
            detail,
            args_redacted_json: otto_core::redact::redact_json(args).value.to_string(),
            args_hash: None,
            risk_label: None,
            requested_by: Some(user.id.clone()),
            requested_by_kind: Some("mcp_server".into()),
            expires_at: Some((chrono::Utc::now() + chrono::Duration::hours(24)).to_rfc3339()),
        })
        .await?;
    let wait = args.get("wait_seconds").and_then(Value::as_u64);
    let decided = wait_for_decision(ctx, &appr.id, wait).await;
    Ok(json!({
        "approval_id": appr.id,
        "status": match decided { Some(true) => "approved", Some(false) => "denied", None => "pending" },
        "note": "poll the MCP approvals queue, or pass wait_seconds (≤30) to block briefly",
    }))
}

// ===========================================================================
// GET / PATCH /mcp/otto-server  (status + config + token mint)
// ===========================================================================

pub async fn otto_server_status(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Value>> {
    let enabled = outward_enabled(&ctx).await;
    let on = enabled_tools(&ctx).await;
    let tools: Vec<Value> = otto_tool_specs()
        .into_iter()
        .map(|t| {
            let name = t["name"].as_str().unwrap_or("").to_string();
            let short = name.strip_prefix("otto.").unwrap_or(&name).to_string();
            json!({
                "name": name,
                "description": t["description"],
                "mutating": t["mutating"],
                "enabled": on.contains(&short),
            })
        })
        .collect();
    let prefix = AuthRepo::new(ctx.pool.clone()).mcp_token_prefix(&user.id).await.map_err(ApiError)?;
    Ok(Json(json!({
        "enabled": enabled,
        "tools": tools,
        "has_token": prefix.is_some(),
        "token_prefix": prefix,
    })))
}

#[derive(Deserialize)]
pub struct OttoServerConfigReq {
    pub enabled: Option<bool>,
    pub tools: Option<Vec<String>>,
    #[serde(default)]
    pub rotate_token: bool,
}

pub async fn otto_server_config(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<OttoServerConfigReq>,
) -> ApiResult<Json<Value>> {
    let settings = SettingsRepo::new(ctx.pool.clone());
    if let Some(en) = req.enabled {
        settings.put("mcp_otto_server_enabled", &json!(en)).await.map_err(ApiError)?;
    }
    if let Some(tools) = &req.tools {
        let known: Vec<String> = otto_tool_specs()
            .iter()
            .filter_map(|t| t["name"].as_str().map(|n| n.strip_prefix("otto.").unwrap_or(n).to_string()))
            .collect();
        for t in tools {
            if !known.contains(t) {
                return Err(ApiError(Error::Invalid(format!("unknown otto tool '{t}'"))));
            }
        }
        settings.put("mcp_otto_server_tools", &json!(tools)).await.map_err(ApiError)?;
    }
    let mut minted: Option<String> = None;
    if req.rotate_token {
        let repo = AuthRepo::new(ctx.pool.clone());
        repo.revoke_mcp_tokens(&user.id).await.map_err(ApiError)?;
        minted = Some(repo.issue_mcp_token(&user.id, Some("otto-mcp-server")).await.map_err(ApiError)?);
        ctx.audit(otto_state::NewAuditEntry {
            user_id: Some(user.id.clone()),
            action: "mcp.otto_server.token_mint".into(),
            target: None,
            detail: None,
            ip: None,
        })
        .await;
    }
    let mut status = otto_server_status(State(ctx), CurrentUser(user)).await?;
    if let Some(tok) = minted {
        status.0["token"] = json!(tok);
    }
    Ok(status)
}

// ===========================================================================
// Gateway — governs LIVE-AGENT downstream calls through the same pipeline.
// ===========================================================================

#[derive(Deserialize)]
pub struct GatewayToolsQuery {
    pub workspace_id: String,
}

/// `GET /mcp/gateway/tools?workspace_id=` — the governed downstream tools for a
/// workspace, namespaced `mcp__<server>__<tool>`. The inward `ottod mcp-tools`
/// surfaces these to the agent and proxies each call through `/mcp/gateway/invoke`.
pub async fn gateway_tools(
    State(ctx): State<ServerCtx>,
    CurrentUser(_user): CurrentUser,
    Query(q): Query<GatewayToolsQuery>,
) -> ApiResult<Json<Value>> {
    let servers = ctx.mcp.registry().list_for_ws(&q.workspace_id).await.map_err(ApiError)?;
    let mut tools: Vec<Value> = Vec::new();
    for s in servers.into_iter().filter(|s| s.enabled && s.managed) {
        for t in ctx.mcp.tools().list_for_server(&s.id).await.map_err(ApiError)?.into_iter().filter(|t| t.enabled) {
            tools.push(json!({
                "name": format!("mcp__{}__{}", s.name, t.name),
                "server_id": s.id,
                "server_name": s.name,
                "tool": t.name,
                "description": t.description,
                "inputSchema": t.input_schema,
                "risk_label": t.risk_label,
            }));
        }
    }
    Ok(Json(json!({ "tools": tools })))
}

#[derive(Deserialize)]
pub struct GatewayInvokeReq {
    pub server_id: Id,
    pub tool: String,
    #[serde(default)]
    pub arguments: Value,
    #[serde(default)]
    pub dry_run: bool,
    pub workspace_id: String,
    #[serde(default)]
    pub session_id: Option<String>,
}

/// `POST /mcp/gateway/invoke` — run a downstream call through the SAME governance
/// pipeline (allowlist→policy→approval→dry-run→execute→audit), tagged
/// `caller_kind='gateway'`. This is what puts the control plane in the path of a
/// live agent's every downstream MCP call.
pub async fn gateway_invoke(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<GatewayInvokeReq>,
) -> ApiResult<Json<Value>> {
    let _ = &req.session_id;
    let ictx = InvokeCtx {
        workspace_id: Some(req.workspace_id.clone()),
        dry_run: req.dry_run,
        caller_user_id: Some(user.id.clone()),
        caller_kind: "gateway".into(),
        direction: "outbound".into(),
    };
    let outcome = ctx.mcp.invoke(&req.server_id, &req.tool, &req.arguments, &ictx).await.map_err(ApiError)?;
    let resp = otto_mcp::outcome_to_resp(outcome);
    if resp.decision == "pending_approval" {
        let _ = ctx.events.send(otto_core::event::Event::Notice {
            level: "warn".into(),
            title: "MCP approval needed".into(),
            body: format!("A governed MCP tool '{}' is awaiting approval.", req.tool),
        });
    }
    Ok(Json(serde_json::to_value(resp).unwrap_or(Value::Null)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec_names() -> Vec<String> {
        otto_tool_specs()
            .iter()
            .filter_map(|t| t["name"].as_str().map(String::from))
            .collect()
    }

    #[test]
    fn scheduled_task_tools_are_registered() {
        let names = spec_names();
        for n in [
            "otto.list_scheduled_tasks",
            "otto.list_scheduled_task_runs",
            "otto.create_scheduled_task",
            "otto.update_scheduled_task",
            "otto.set_scheduled_task_enabled",
            "otto.run_scheduled_task",
            "otto.delete_scheduled_task",
        ] {
            assert!(names.contains(&n.to_string()), "missing spec {n}");
        }
    }

    #[test]
    fn write_tools_are_dangerous_reads_are_default_enabled() {
        for w in [
            "create_scheduled_task",
            "update_scheduled_task",
            "delete_scheduled_task",
            "run_scheduled_task",
            "set_scheduled_task_enabled",
        ] {
            assert!(DANGEROUS.contains(&w), "{w} must be DANGEROUS");
            assert!(!DEFAULT_ENABLED.contains(&w), "{w} must be off by default");
        }
        assert!(DEFAULT_ENABLED.contains(&"list_scheduled_tasks"));
        assert!(DEFAULT_ENABLED.contains(&"list_scheduled_task_runs"));
    }

    #[test]
    fn create_tool_is_marked_mutating() {
        let specs = otto_tool_specs();
        let create = specs
            .iter()
            .find(|t| t["name"] == "otto.create_scheduled_task")
            .unwrap();
        assert_eq!(create["mutating"], serde_json::json!(true));
    }

    #[test]
    fn dangerous_detail_surfaces_cadence_and_destination() {
        let args = serde_json::json!({
            "name": "Nightly",
            "schedule": {"cadence": "interval", "every_min": 60},
            "destination": {"type": "slack"},
            "prompt": "do the thing"
        });
        let d = dangerous_detail("otto.create_scheduled_task", &args);
        assert!(d.contains("Nightly"));
        assert!(d.contains("every 60 min"));
        assert!(d.contains("slack"));
        assert!(d.contains("do the thing"));
    }
}
