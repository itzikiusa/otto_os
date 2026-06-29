//! `ottod mcp-tools` — the first-party Otto **read-only** MCP tool server
//! (Task B2b).
//!
//! Otto exposes a slice of its own data to an agent session as MCP tools. When
//! `otto_mcp_enabled` is on (default), `otto-sessions` injects an `otto` server
//! whose command is `ottod mcp-tools`, carrying a **per-session token** — in the
//! workspace `.mcp.json` for Claude, or a per-session creds file (`--config`) for
//! Codex. claude/codex launch that command and speak MCP to it over stdio; this
//! process answers `initialize` / `tools/list` / `tools/call`, calling back into
//! the running daemon on 127.0.0.1 with the per-session token (which authorizes as
//! the session's owner, so workspace RBAC applies).
//!
//! Beyond Otto's own data, the DB tools (`otto_list_connections`,
//! `otto_db_schema`/`_children`/`_object`, `otto_db_query`) expose the user's
//! database **connections**: schema introspection and **read-only** queries.
//!
//! Safety properties (every tool, no exceptions):
//! - **Read-only** — all upstream calls are `GET`s, or `POST`s to a hard-coded
//!   allow-list of **read-only-enforced** endpoints. The only query path,
//!   `…/db/mcp-query`, refuses any write/DDL server-side (`run_read_only`) before a
//!   driver runs, independent of the connection's write-guard. No tool here mutates.
//! - **Capped** — each upstream call has a wall-clock timeout; the response body
//!   is size-capped before parsing, and JSON arrays are row-capped.
//! - **Redacted** — every tool result is passed through `otto_core::redact` so
//!   tokens/PII never reach the agent transcript (the query path also masks cells
//!   server-side).
//! - **Audited** — every call appends a row to `mcp_tool_calls` (best-effort).
//!
//! The transport is newline-delimited JSON-RPC 2.0 (one JSON object per line on
//! stdin/stdout), which is the MCP stdio framing claude/codex use.

use std::time::Duration;

use otto_core::redact::redact_json;
use otto_state::{McpAuditRepo, NewMcpToolCall};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::config::Config;

/// Protocol version we advertise in `initialize`. Mirrors the spec revision the
/// bundled CLIs negotiate; clients echo their own and tolerate a match.
const PROTOCOL_VERSION: &str = "2024-11-05";

/// Per-call wall-clock timeout for an upstream daemon request.
const CALL_TIMEOUT: Duration = Duration::from_secs(20);
/// Hard cap on an upstream response body we will buffer + parse (1 MiB). Larger
/// bodies are rejected rather than streamed into an agent's context.
const MAX_BODY_BYTES: usize = 1024 * 1024;
/// Cap on the number of elements kept from any top-level / nested JSON array in
/// a tool result, so a huge schema/list can't blow the transcript. A truncation
/// marker is appended when the cap bites.
const MAX_ROWS: usize = 500;

/// Runtime context shared by all tool handlers.
struct Ctx {
    http: reqwest::Client,
    /// Base URL of the running daemon, e.g. `http://127.0.0.1:7700`.
    base: String,
    /// Per-session read-only bearer token (the `OTTO_MCP_TOKEN` env value).
    token: String,
    /// Calling agent session id (for audit + RBAC scoping). May be empty.
    session_id: Option<String>,
    /// Calling workspace id (for audit). May be empty.
    workspace_id: Option<String>,
    /// Audit sink. `None` when the DB can't be opened (audit degrades to logs).
    audit: Option<McpAuditRepo>,
}

impl Ctx {
    /// GET an `/api/v1` path with the bearer token, enforcing the call timeout
    /// and the body-size cap, returning parsed JSON. Read-only by construction —
    /// this is the ONLY upstream verb the tools use.
    async fn get_json(&self, path: &str) -> Result<Value, String> {
        let url = format!("{}/api/v1{}", self.base.trim_end_matches('/'), path);
        let resp = tokio::time::timeout(
            CALL_TIMEOUT,
            self.http
                .get(&url)
                .bearer_auth(&self.token)
                .header("X-Otto-Session", self.session_id.clone().unwrap_or_default())
                .send(),
        )
        .await
        .map_err(|_| format!("upstream timeout after {}s", CALL_TIMEOUT.as_secs()))?
        .map_err(|e| format!("request failed: {e}"))?;

        let status = resp.status();
        // Read the body with a size cap. reqwest's `bytes()` would buffer the
        // whole thing; we guard on Content-Length first, then re-check the actual
        // length (a server may omit/lie about the header).
        if let Some(len) = resp.content_length() {
            if len as usize > MAX_BODY_BYTES {
                return Err(format!(
                    "response too large ({len} bytes > {MAX_BODY_BYTES} cap)"
                ));
            }
        }
        let body = resp.bytes().await.map_err(|e| format!("read body: {e}"))?;
        if body.len() > MAX_BODY_BYTES {
            return Err(format!(
                "response too large ({} bytes > {MAX_BODY_BYTES} cap)",
                body.len()
            ));
        }
        if !status.is_success() {
            // Surface the daemon's error text (already small) but don't leak a
            // huge body; the status is the actionable part for the agent.
            let snippet = String::from_utf8_lossy(&body);
            let snippet = snippet.chars().take(300).collect::<String>();
            return Err(format!("daemon returned {status}: {snippet}"));
        }
        serde_json::from_slice(&body).map_err(|e| format!("parse json: {e}"))
    }

    /// POST an `/api/v1` path with the bearer token. Used by the governed gateway
    /// proxy AND by the read-only DB tools (`otto_db_children` / `otto_db_object` /
    /// `otto_db_query`), which post to **read-only-enforced** endpoints — the
    /// `…/db/mcp-query` route refuses any write/DDL server-side before a driver runs.
    /// Same body-size cap as [`Self::get_json`] so a large result can't blow the
    /// agent transcript.
    async fn post_json(&self, path: &str, body: &Value) -> Result<Value, String> {
        let url = format!("{}/api/v1{}", self.base.trim_end_matches('/'), path);
        let resp = tokio::time::timeout(
            CALL_TIMEOUT,
            self.http
                .post(&url)
                .bearer_auth(&self.token)
                .header("X-Otto-Session", self.session_id.clone().unwrap_or_default())
                .json(body)
                .send(),
        )
        .await
        .map_err(|_| "upstream timeout".to_string())?
        .map_err(|e| format!("request failed: {e}"))?;
        let status = resp.status();
        if let Some(len) = resp.content_length() {
            if len as usize > MAX_BODY_BYTES {
                return Err(format!("response too large ({len} bytes > {MAX_BODY_BYTES} cap)"));
            }
        }
        let bytes = resp.bytes().await.map_err(|e| format!("read body: {e}"))?;
        if bytes.len() > MAX_BODY_BYTES {
            return Err(format!(
                "response too large ({} bytes > {MAX_BODY_BYTES} cap)",
                bytes.len()
            ));
        }
        if !status.is_success() {
            let snippet = String::from_utf8_lossy(&bytes);
            return Err(format!(
                "daemon returned {status}: {}",
                snippet.chars().take(300).collect::<String>()
            ));
        }
        serde_json::from_slice(&bytes).map_err(|e| format!("parse json: {e}"))
    }

    /// The governed downstream tools the live-agent **gateway** exposes for this
    /// session's workspace, namespaced `mcp__<server>__<tool>`. Best-effort: an
    /// empty list (gateway off, no workspace, or the session user lacks MCP
    /// access) leaves the inward catalog unchanged.
    async fn gateway_tools(&self) -> Vec<Value> {
        let Some(ws) = &self.workspace_id else { return vec![] };
        match self.get_json(&format!("/mcp/gateway/tools?workspace_id={}", seg(ws))).await {
            Ok(v) => v.get("tools").and_then(Value::as_array).cloned().unwrap_or_default(),
            Err(_) => vec![],
        }
    }

    /// Append a best-effort audit row for one tool call. Failures are logged to
    /// stderr (never stdout — that's the protocol channel) and swallowed.
    async fn audit(&self, tool: &str, args: &Value, ok: bool, rows: Option<i64>) {
        let Some(audit) = &self.audit else {
            return;
        };
        // The arguments are redacted before persisting (defense-in-depth: a
        // caller could pass a secret-looking value).
        let args_json = redact_json(args).value.to_string();
        if let Err(e) = audit
            .record(NewMcpToolCall {
                workspace_id: self.workspace_id.clone(),
                session_id: self.session_id.clone(),
                tool: tool.to_string(),
                args_json,
                ok,
                rows,
            })
            .await
        {
            eprintln!("ottod mcp-tools: audit insert failed: {e}");
        }
    }
}

/// Recursively cap the number of elements in every JSON array to [`MAX_ROWS`],
/// appending a string marker element when truncation happens. Returns the capped
/// value and the largest array length seen (used as the audited `rows`).
fn cap_rows(v: Value, max_seen: &mut usize) -> Value {
    match v {
        Value::Array(items) => {
            let n = items.len();
            if n > *max_seen {
                *max_seen = n;
            }
            let truncated = n > MAX_ROWS;
            let mut out: Vec<Value> = items
                .into_iter()
                .take(MAX_ROWS)
                .map(|i| cap_rows(i, max_seen))
                .collect();
            if truncated {
                out.push(Value::String(format!(
                    "[otto: truncated — {n} items, showing first {MAX_ROWS}]"
                )));
            }
            Value::Array(out)
        }
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(k, val)| (k, cap_rows(val, max_seen)))
                .collect(),
        ),
        other => other,
    }
}

/// The static tool catalog returned by `tools/list`. Kept in one place so the
/// `tools/call` dispatch and the advertised schema can't drift.
fn tool_catalog() -> Value {
    json!({
        "tools": [
            {
                "name": "otto_list_connections",
                "description": "Read-only: list the database connections available to this session — id, name, kind, environment, read_only. Use this FIRST to discover connection ids, then call otto_db_schema / otto_db_query with a returned id. Only queryable DB kinds are listed (mysql, redis, mongodb, clickhouse).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "kind": { "type": "string", "description": "Optional filter to one kind: mysql | redis | mongodb | clickhouse." }
                    }
                }
            },
            {
                "name": "otto_db_schema",
                "description": "Read-only: the TOP of a connection's schema tree — databases (SQL/Mongo) or keyspaces (Redis). Returns structure only, no row data. Each node carries an `id` (a NodePath) you pass to otto_db_children / otto_db_object.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "connection_id": { "type": "string", "description": "Otto connection id (a DB-kind connection)." }
                    },
                    "required": ["connection_id"]
                }
            },
            {
                "name": "otto_db_children",
                "description": "Read-only: expand ONE node of a connection's schema tree (engine-agnostic, lazy). `path` is a NodePath from otto_db_schema/otto_db_children — e.g. 'db:shop' → its folders, 'db:shop/folder:tables' → its tables, 'db:shop/folder:tables/table:orders' → its columns. Redis: 'kdb:0' → key prefixes (optional `filter`).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "connection_id": { "type": "string", "description": "Otto connection id." },
                        "path": { "type": "string", "description": "NodePath of the node to expand, e.g. 'db:shop/folder:tables'." },
                        "filter": { "type": "string", "description": "Optional substring/prefix filter (e.g. a Redis key prefix)." }
                    },
                    "required": ["connection_id", "path"]
                }
            },
            {
                "name": "otto_db_object",
                "description": "Read-only: the FULL structure of one table/collection — columns (name + type), primary key, indexes, foreign keys, and the CREATE/DDL where the engine exposes it. `path` is the object's NodePath, e.g. 'db:shop/folder:tables/table:orders'.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "connection_id": { "type": "string", "description": "Otto connection id." },
                        "path": { "type": "string", "description": "NodePath of the table/collection." }
                    },
                    "required": ["connection_id", "path"]
                }
            },
            {
                "name": "otto_db_query",
                "description": "Run a READ-ONLY query against a connection and return rows {columns, rows, truncated}. SQL (mysql/clickhouse): a SELECT/SHOW/DESCRIBE/EXPLAIN/WITH statement. Redis: a read command per line (GET/HGETALL/SCAN/…). Mongo: a find/aggregate. Writes/DDL are REFUSED server-side. `database` scopes the active database (SQL/Mongo); Redis selects a keyspace via `node` 'kdb:N'. `max_rows` caps rows (server hard cap 200).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "connection_id": { "type": "string", "description": "Otto connection id." },
                        "statement": { "type": "string", "description": "The read-only statement / command." },
                        "database": { "type": "string", "description": "Optional active database to scope SQL/Mongo execution." },
                        "node": { "type": "string", "description": "Optional raw node context (e.g. 'kdb:0' for a Redis keyspace); overrides `database`." },
                        "max_rows": { "type": "integer", "description": "Optional row cap (clamped to 200)." }
                    },
                    "required": ["connection_id", "statement"]
                }
            },
            {
                "name": "otto_git_pr_review",
                "description": "Read-only: a pull request plus its review summary (state, reviewers, review comments) for an Otto-tracked repo.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "repo_id": { "type": "string", "description": "Otto repo id." },
                        "pr_number": { "type": "integer", "description": "Pull request number." }
                    },
                    "required": ["repo_id", "pr_number"]
                }
            },
            {
                "name": "otto_product_story",
                "description": "Read-only: a product story context bundle (the story record and its latest agent-ready inject context) by story id.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "story_id": { "type": "string", "description": "Otto product story id." }
                    },
                    "required": ["story_id"]
                }
            },
            {
                "name": "canvas_list_scenes",
                "description": "Read-only: list the Canvas Studio scenes (id/title/timestamps) in a workspace. To CREATE or EDIT a scene, use the otto-canvas skill's HTTP scripts (writes are not exposed as MCP tools).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "workspace_id": { "type": "string", "description": "Otto workspace id." }
                    },
                    "required": ["workspace_id"]
                }
            },
            {
                "name": "canvas_get_scene",
                "description": "Read-only: a Canvas Studio scene by id, including its full Scene JSON document (nodes/edges/slides).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "scene_id": { "type": "string", "description": "Otto canvas scene id." }
                    },
                    "required": ["scene_id"]
                }
            }
        ]
    })
}

/// Extract a required string argument, erroring with a clear message if absent.
fn arg_str(args: &Value, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("missing required string argument `{key}`"))
}

/// Extract a required integer argument.
fn arg_i64(args: &Value, key: &str) -> Result<i64, String> {
    args.get(key)
        .and_then(|v| v.as_i64())
        .ok_or_else(|| format!("missing required integer argument `{key}`"))
}

/// Percent-encode a path segment so an id with `/` or spaces can't break out of
/// the intended route (defense-in-depth; ids are normally opaque tokens).
fn seg(s: &str) -> String {
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

/// Run one tool by name. Returns the capped+redacted result `Value` and the
/// audited row count, or an error string surfaced to the agent.
async fn run_tool(ctx: &Ctx, name: &str, args: &Value) -> Result<(Value, Option<i64>), String> {
    match name {
        "otto_list_connections" => {
            let Some(ws) = ctx.workspace_id.as_deref() else {
                return Err(
                    "no workspace context (OTTO_WORKSPACE_ID unset); cannot list connections".into(),
                );
            };
            let raw = ctx
                .get_json(&format!("/workspaces/{}/connections", seg(ws)))
                .await?;
            // Keep only queryable DB kinds, optionally one kind, and slim each row
            // so the agent sees ids/names/kinds without connection params/secrets.
            let kind_filter = args.get("kind").and_then(Value::as_str);
            let items: Vec<Value> = raw
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or(&[])
                .iter()
                .filter_map(|c| {
                    let kind = c.get("kind").and_then(Value::as_str).unwrap_or("");
                    if !matches!(kind, "mysql" | "redis" | "mongodb" | "clickhouse") {
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
            if let Some(f) = args.get("filter").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                body["filter"] = json!(f);
            }
            let raw = ctx
                .post_json(&format!("/connections/{}/db/schema/children", seg(&conn)), &body)
                .await?;
            Ok(finalize(json!({ "connection_id": conn, "path": path, "children": raw })))
        }
        "otto_db_object" => {
            let conn = arg_str(args, "connection_id")?;
            let path = arg_str(args, "path")?;
            let body = json!({ "path": path });
            let raw = ctx
                .post_json(&format!("/connections/{}/db/object", seg(&conn)), &body)
                .await?;
            Ok(finalize(json!({ "connection_id": conn, "path": path, "object": raw })))
        }
        "otto_db_query" => {
            let conn = arg_str(args, "connection_id")?;
            let statement = arg_str(args, "statement")?;
            let mut body = json!({ "statement": statement });
            // `node` (raw) wins over `database` (convenience → SQL/Mongo `db:<name>`).
            if let Some(node) = args.get("node").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                body["node"] = json!(node);
            } else if let Some(db) =
                args.get("database").and_then(Value::as_str).filter(|s| !s.is_empty())
            {
                body["node"] = json!(format!("db:{db}"));
            }
            if let Some(mr) = args.get("max_rows").and_then(Value::as_u64) {
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
        "canvas_list_scenes" => {
            let ws = arg_str(args, "workspace_id")?;
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
        other => Err(format!("unknown tool `{other}`")),
    }
}

/// Proxy a namespaced gateway tool (`mcp__<server>__<tool>`) through the control
/// plane's governed `/mcp/gateway/invoke`. The pipeline there does allowlist /
/// policy / approval / dry-run / execute / **audit**, so this path does NOT write
/// the first-party `mcp_tool_calls` row (the call is already on `mcp_call_log`).
/// Returns the governed envelope and whether it was an error/denial.
async fn gateway_call(ctx: &Ctx, namespaced: &str, args: &Value) -> Result<(Value, bool), String> {
    let Some(ws) = &ctx.workspace_id else {
        return Err("gateway: no workspace context".into());
    };
    let tools = ctx.gateway_tools().await;
    let entry = tools
        .iter()
        .find(|t| t.get("name").and_then(Value::as_str) == Some(namespaced))
        .ok_or_else(|| format!("unknown gateway tool `{namespaced}`"))?;
    let server_id = entry.get("server_id").and_then(Value::as_str).unwrap_or("");
    let tool = entry.get("tool").and_then(Value::as_str).unwrap_or("");
    let body = json!({
        "server_id": server_id,
        "tool": tool,
        "arguments": args,
        "workspace_id": ws,
        "session_id": ctx.session_id,
    });
    let v = ctx.post_json("/mcp/gateway/invoke", &body).await?;
    let is_error = matches!(v.get("decision").and_then(Value::as_str), Some("denied") | Some("error"))
        || v.get("is_error").and_then(Value::as_bool).unwrap_or(false);
    Ok((v, is_error))
}

/// Apply the row cap then redaction to a tool result, returning the cleaned
/// value and the audited row count (largest array length seen pre-cap).
fn finalize(v: Value) -> (Value, Option<i64>) {
    let mut max_seen = 0usize;
    let capped = cap_rows(v, &mut max_seen);
    let redacted = redact_json(&capped).value;
    (redacted, Some(max_seen as i64))
}

/// Build a JSON-RPC success result envelope.
fn rpc_ok(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

/// Build a JSON-RPC error envelope.
fn rpc_err(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message.into() } })
}

/// Wrap a tool result `Value` as MCP `tools/call` content (a single JSON text
/// block, pretty-printed). `is_error` flags a tool-level failure to the client.
fn tool_result(value: &Value, is_error: bool) -> Value {
    let text = serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string());
    json!({
        "content": [ { "type": "text", "text": text } ],
        "isError": is_error
    })
}

/// Handle a single decoded JSON-RPC request, returning the response to write
/// (or `None` for a notification, which gets no reply).
async fn handle(ctx: &Ctx, msg: Value) -> Option<Value> {
    let method = msg.get("method").and_then(|v| v.as_str()).unwrap_or("");
    let id = msg.get("id").cloned();
    // Notifications carry no `id` and MUST NOT be answered.
    let is_notification = id.is_none();

    match method {
        "initialize" => {
            let result = json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "otto", "version": env!("CARGO_PKG_VERSION") }
            });
            Some(rpc_ok(id.unwrap_or(Value::Null), result))
        }
        // Client tells us it's ready; no response.
        "notifications/initialized" | "initialized" => None,
        "ping" => Some(rpc_ok(id.unwrap_or(Value::Null), json!({}))),
        "tools/list" => {
            // The static first-party read-only catalog, plus — when the live-agent
            // gateway is enabled for this workspace — the governed downstream tools.
            let mut cat = tool_catalog();
            let gw = ctx.gateway_tools().await;
            if let Some(arr) = cat["tools"].as_array_mut() {
                for t in gw {
                    arr.push(json!({
                        "name": t["name"],
                        "description": t["description"],
                        "inputSchema": t["inputSchema"],
                    }));
                }
            }
            Some(rpc_ok(id.unwrap_or(Value::Null), cat))
        }
        "tools/call" => {
            let id = id.unwrap_or(Value::Null);
            let params = msg.get("params").cloned().unwrap_or(Value::Null);
            let name = params
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            // A namespaced `mcp__server__tool` is a governed downstream call —
            // route it through the control-plane gateway (which audits it itself).
            if name.starts_with("mcp__") {
                return Some(match gateway_call(ctx, &name, &args).await {
                    Ok((v, is_error)) => rpc_ok(id, tool_result(&v, is_error)),
                    Err(e) => rpc_ok(id, tool_result(&json!({ "error": e }), true)),
                });
            }
            match run_tool(ctx, &name, &args).await {
                Ok((value, rows)) => {
                    ctx.audit(&name, &args, true, rows).await;
                    Some(rpc_ok(id, tool_result(&value, false)))
                }
                Err(e) => {
                    ctx.audit(&name, &args, false, None).await;
                    // Tool-level errors are returned as a successful RPC with an
                    // error content block (per MCP), so the agent sees the reason
                    // rather than a transport failure.
                    Some(rpc_ok(
                        id,
                        tool_result(&json!({ "error": e }), true),
                    ))
                }
            }
        }
        _ if is_notification => None,
        _ => Some(rpc_err(
            id.unwrap_or(Value::Null),
            -32601,
            format!("method not found: {method}"),
        )),
    }
}

/// Resolved credentials + routing for the tools — from env (the Claude path) or a
/// per-session creds file (the Codex path).
struct Creds {
    token: String,
    base: Option<String>,
    session_id: Option<String>,
    workspace_id: Option<String>,
}

/// Find the creds-file path: `--config <path>` / `--config=<path>` in `args`, else
/// the `OTTO_MCP_CONFIG` env. Pure over `args` for testability.
fn config_path_in(args: &[String]) -> Option<String> {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--config" {
            return it.next().cloned();
        }
        if let Some(p) = a.strip_prefix("--config=") {
            return Some(p.to_string());
        }
    }
    std::env::var("OTTO_MCP_CONFIG").ok().filter(|s| !s.is_empty())
}

/// Parse a per-session creds JSON document (`{token, base?, session_id?,
/// workspace_id?}`). Pure for testability; errors if the `token` is missing.
fn parse_creds(body: &str) -> Result<Creds, String> {
    let v: Value = serde_json::from_str(body).map_err(|e| format!("parse creds: {e}"))?;
    let token = v.get("token").and_then(Value::as_str).unwrap_or("").to_string();
    if token.is_empty() {
        return Err("creds file has no `token`".into());
    }
    let s = |k: &str| {
        v.get(k)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    Ok(Creds {
        token,
        base: s("base"),
        session_id: s("session_id"),
        workspace_id: s("workspace_id"),
    })
}

/// Resolve credentials: env first (Claude — `OTTO_MCP_TOKEN` & friends), then a
/// per-session creds file (Codex — `--config <path>` / `OTTO_MCP_CONFIG`).
fn load_creds(args: &[String]) -> Result<Creds, String> {
    if let Ok(token) = std::env::var("OTTO_MCP_TOKEN") {
        if !token.is_empty() {
            let s = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
            return Ok(Creds {
                token,
                base: s("OTTO_MCP_BASE"),
                session_id: s("OTTO_SESSION_ID"),
                workspace_id: s("OTTO_WORKSPACE_ID"),
            });
        }
    }
    let path = config_path_in(args).ok_or_else(|| {
        "no OTTO_MCP_TOKEN and no --config/OTTO_MCP_CONFIG creds file (the first-party \
         tools require a per-session token)"
            .to_string()
    })?;
    let body = std::fs::read_to_string(&path).map_err(|e| format!("read creds {path}: {e}"))?;
    parse_creds(&body)
}

/// Entry point for `ottod mcp-tools`. Reads JSON-RPC lines on stdin, writes
/// responses on stdout, until EOF.
pub async fn run() -> Result<(), String> {
    // Credentials + routing come from the env the session manager injected (the
    // Claude path: `OTTO_MCP_TOKEN`/`OTTO_MCP_BASE`/`OTTO_SESSION_ID`/
    // `OTTO_WORKSPACE_ID`), OR — when the token env is absent (the Codex path,
    // which can't carry per-session env through `-c` cleanly) — from a per-session
    // creds file named by `--config <path>` / `OTTO_MCP_CONFIG`.
    let args: Vec<String> = std::env::args().collect();
    let creds = load_creds(&args)?;
    let token = creds.token;
    let base = creds.base.unwrap_or_else(|| {
        let cfg = Config::load();
        format!("http://127.0.0.1:{}", cfg.port)
    });
    let session_id = creds.session_id;
    let workspace_id = creds.workspace_id;

    // Open the same SQLite DB the daemon uses, for the audit ledger. Best-effort:
    // if it can't be opened the tools still run, audit just degrades to stderr.
    let audit = match otto_state::open(&Config::load().db_path()).await {
        Ok(pool) => Some(McpAuditRepo::new(pool)),
        Err(e) => {
            eprintln!("ottod mcp-tools: audit DB unavailable ({e}); audit disabled");
            None
        }
    };

    let http = reqwest::Client::builder()
        .timeout(CALL_TIMEOUT)
        .build()
        .map_err(|e| format!("build http client: {e}"))?;

    let ctx = Ctx {
        http,
        base,
        token,
        session_id,
        workspace_id,
        audit,
    };

    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin);
    let mut stdout = tokio::io::stdout();
    let mut line = String::new();

    loop {
        line.clear();
        let n = reader
            .read_line(&mut line)
            .await
            .map_err(|e| format!("read stdin: {e}"))?;
        if n == 0 {
            break; // EOF: the client closed the pipe.
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                // Malformed line: emit a parse-error response with null id.
                let resp = rpc_err(Value::Null, -32700, format!("parse error: {e}"));
                write_line(&mut stdout, &resp).await?;
                continue;
            }
        };
        if let Some(resp) = handle(&ctx, msg).await {
            write_line(&mut stdout, &resp).await?;
        }
    }
    Ok(())
}

/// Serialize one JSON-RPC message and write it as a single newline-terminated
/// line, flushing so the client sees it immediately.
async fn write_line(
    stdout: &mut tokio::io::Stdout,
    value: &Value,
) -> Result<(), String> {
    let mut buf = serde_json::to_vec(value).map_err(|e| format!("encode response: {e}"))?;
    buf.push(b'\n');
    stdout
        .write_all(&buf)
        .await
        .map_err(|e| format!("write stdout: {e}"))?;
    stdout.flush().await.map_err(|e| format!("flush stdout: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cap_rows_truncates_and_marks() {
        let big: Vec<Value> = (0..(MAX_ROWS + 50)).map(|i| json!(i)).collect();
        let mut max = 0;
        let out = cap_rows(json!({ "items": big }), &mut max);
        let arr = out["items"].as_array().unwrap();
        // MAX_ROWS kept + 1 truncation marker.
        assert_eq!(arr.len(), MAX_ROWS + 1);
        assert_eq!(max, MAX_ROWS + 50);
        assert!(arr.last().unwrap().as_str().unwrap().contains("truncated"));
    }

    #[test]
    fn cap_rows_passes_small_arrays() {
        let mut max = 0;
        let out = cap_rows(json!({ "a": [1, 2, 3] }), &mut max);
        assert_eq!(out["a"].as_array().unwrap().len(), 3);
        assert_eq!(max, 3);
    }

    #[test]
    fn finalize_redacts_secrets_in_result() {
        // A value carrying a sensitive key must come back redacted.
        let (v, _rows) = finalize(json!({ "rows": [ { "password": "hunter2", "name": "alice" } ] }));
        assert_eq!(v["rows"][0]["password"], json!("[redacted]"));
        assert_eq!(v["rows"][0]["name"], json!("alice"));
    }

    #[test]
    fn tool_catalog_lists_the_three_priority_tools() {
        let cat = tool_catalog();
        let names: Vec<&str> = cat["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"otto_db_schema"));
        assert!(names.contains(&"otto_git_pr_review"));
        assert!(names.contains(&"otto_product_story"));
    }

    #[test]
    fn tool_catalog_lists_the_connection_db_tools() {
        let cat = tool_catalog();
        let names: Vec<&str> = cat["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        for t in [
            "otto_list_connections",
            "otto_db_schema",
            "otto_db_children",
            "otto_db_object",
            "otto_db_query",
        ] {
            assert!(names.contains(&t), "catalog is missing {t}");
        }
        // Every advertised tool must carry an inputSchema object.
        for tool in cat["tools"].as_array().unwrap() {
            assert!(
                tool["inputSchema"]["type"] == json!("object"),
                "tool {} has no object inputSchema",
                tool["name"]
            );
        }
    }

    #[tokio::test]
    async fn list_connections_errors_without_workspace() {
        // No OTTO_WORKSPACE_ID context ⇒ a clear tool error, no upstream call.
        let mut ctx = test_ctx();
        ctx.workspace_id = None;
        let resp = handle(
            &ctx,
            json!({ "jsonrpc": "2.0", "id": 5, "method": "tools/call",
                    "params": { "name": "otto_list_connections", "arguments": {} } }),
        )
        .await
        .unwrap();
        assert_eq!(resp["result"]["isError"], json!(true));
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("workspace"), "got: {text}");
    }

    #[test]
    fn config_path_in_reads_flag_and_eq_forms() {
        let a: Vec<String> = ["ottod", "mcp-tools", "--config", "/tmp/c.json"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(config_path_in(&a).as_deref(), Some("/tmp/c.json"));
        let b: Vec<String> = ["ottod", "mcp-tools", "--config=/tmp/x.json"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(config_path_in(&b).as_deref(), Some("/tmp/x.json"));
    }

    #[test]
    fn parse_creds_reads_fields_and_requires_token() {
        let c = parse_creds(
            r#"{"token":"t-1","base":"http://127.0.0.1:7700","session_id":"s-1","workspace_id":"ws-1"}"#,
        )
        .unwrap();
        assert_eq!(c.token, "t-1");
        assert_eq!(c.base.as_deref(), Some("http://127.0.0.1:7700"));
        assert_eq!(c.session_id.as_deref(), Some("s-1"));
        assert_eq!(c.workspace_id.as_deref(), Some("ws-1"));
        // A token-less document is rejected.
        assert!(parse_creds(r#"{"base":"x"}"#).is_err());
    }

    #[tokio::test]
    async fn initialize_returns_protocol_and_serverinfo() {
        let ctx = test_ctx();
        let resp = handle(&ctx, json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize" }))
            .await
            .unwrap();
        assert_eq!(resp["result"]["protocolVersion"], json!(PROTOCOL_VERSION));
        assert_eq!(resp["result"]["serverInfo"]["name"], json!("otto"));
    }

    #[tokio::test]
    async fn initialized_notification_gets_no_reply() {
        let ctx = test_ctx();
        let resp = handle(
            &ctx,
            json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        )
        .await;
        assert!(resp.is_none(), "a notification must not be answered");
    }

    #[tokio::test]
    async fn tools_list_is_answered() {
        let ctx = test_ctx();
        let resp = handle(&ctx, json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }))
            .await
            .unwrap();
        assert!(resp["result"]["tools"].is_array());
    }

    #[tokio::test]
    async fn unknown_method_is_method_not_found() {
        let ctx = test_ctx();
        let resp = handle(&ctx, json!({ "jsonrpc": "2.0", "id": 9, "method": "frobnicate" }))
            .await
            .unwrap();
        assert_eq!(resp["error"]["code"], json!(-32601));
    }

    #[tokio::test]
    async fn unknown_tool_returns_error_content() {
        let ctx = test_ctx();
        let resp = handle(
            &ctx,
            json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call",
                    "params": { "name": "nope", "arguments": {} } }),
        )
        .await
        .unwrap();
        // Tool-level error: RPC success, isError true, message mentions the tool.
        assert_eq!(resp["result"]["isError"], json!(true));
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("nope"));
    }

    /// A Ctx pointing at an unreachable base; used by the no-upstream tests above
    /// (which never actually call out). Audit disabled.
    fn test_ctx() -> Ctx {
        Ctx {
            http: reqwest::Client::new(),
            base: "http://127.0.0.1:9".to_string(),
            token: "test-token".to_string(),
            session_id: Some("sess-test".into()),
            workspace_id: Some("ws-test".into()),
            audit: None,
        }
    }
}
