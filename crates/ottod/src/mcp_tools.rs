//! `ottod mcp-tools` — the first-party Otto MCP tool server (Task B2b).
//! Nearly every tool here is **read-only**; the exceptions are
//! `canvas_create_scene` / `canvas_update_scene` (Task B5), the Vault v3
//! doc writers `otto_vault_write` / `otto_vault_rename` / `otto_vault_delete`,
//! the swarm board tools `swarm_create_task` / `swarm_update_task` /
//! `swarm_run_task` / `swarm_stop_run` (the manager's utilization levers), and
//! `browser_navigate` (Task 6, opens a reader-mode browser tab) — which call
//! the normal governed HTTP endpoints AS THE SESSION OWNER — the
//! same workspace-role check (`Editor`) a human gets, no more. Canvas is meant
//! to be agent-drawable, the vault is the agents' documentation home
//! (delete is a soft move to `.trash/`), the swarm board is how a manager
//! agent keeps its team utilized, and a reader tab is the same one-URL "open
//! a tab" action the Browser module UI does; every other tool stays strictly
//! read-only (see "Safety properties" below).
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
//! Safety properties:
//! - **Read-only, with named exceptions** — all upstream calls are `GET`s, or
//!   `POST`s to a hard-coded allow-list of **read-only-enforced** endpoints. The
//!   DB query path, `…/db/mcp-query`, refuses any write/DDL server-side
//!   (`run_read_only`) before a driver runs, independent of the connection's
//!   write-guard; the other read POSTs are `…/memory/search`, the vault
//!   `…/vault/vaults/{id}/search` / `…/okf/validate`, and `…/browser/summarize`
//!   (Editor-gated but doesn't persist anything — the summarize session is
//!   ephemeral and never saved; see `routes/browser.rs`). `canvas_create_scene`/
//!   `canvas_update_scene`, `otto_vault_write`/`otto_vault_rename`/
//!   `otto_vault_delete`, and `browser_navigate` are the ONLY tools that
//!   mutate persisted state: they hit the normal governed HTTP routes, which
//!   apply the same `WorkspaceRole::Editor` gate a human caller hits — the
//!   token can only do what the session's owner is already allowed to do
//!   (and vault delete only trashes, never destroys).
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
    /// Per-session bearer token (the `OTTO_MCP_TOKEN` env value). Authorizes as
    /// the session's owner against the governed `/api/v1` routes — read-only
    /// for every tool except `canvas_create_scene`/`canvas_update_scene`, which
    /// it also authorizes to write (see the module doc's "Safety properties").
    token: String,
    /// Calling agent session id (for audit + RBAC scoping). May be empty.
    session_id: Option<String>,
    /// Calling workspace id (for audit). May be empty.
    workspace_id: Option<String>,
    /// Session metadata source. Review sessions use this to receive a
    /// read-only Vault catalog and a dispatcher-level mutation deny.
    source: Option<String>,
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

    /// PUT an `/api/v1` path with the bearer token. Used ONLY by
    /// `canvas_update_scene` — the one mutating call site that isn't a POST.
    /// Same size cap / error handling as [`Self::post_json`].
    async fn put_json(&self, path: &str, body: &Value) -> Result<Value, String> {
        let url = format!("{}/api/v1{}", self.base.trim_end_matches('/'), path);
        let resp = tokio::time::timeout(
            CALL_TIMEOUT,
            self.http
                .put(&url)
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

    /// PATCH an `/api/v1` path with the bearer token. Used by the swarm write
    /// tools (`swarm_update_task`) — same governed Editor-gated routes the UI
    /// hits; same timeout/size handling as [`Self::put_json`].
    async fn patch_json(&self, path: &str, body: &Value) -> Result<Value, String> {
        let url = format!("{}/api/v1{}", self.base.trim_end_matches('/'), path);
        let resp = tokio::time::timeout(
            CALL_TIMEOUT,
            self.http
                .patch(&url)
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

    /// DELETE an `/api/v1` path with the bearer token. Used ONLY by
    /// `otto_vault_delete` — a soft delete (the daemon moves the note into the
    /// vault's `.trash/`, never destroying files). Same error handling as
    /// [`Self::post_json`]; tolerates an empty (204) body.
    async fn delete_ok(&self, path: &str) -> Result<(), String> {
        let url = format!("{}/api/v1{}", self.base.trim_end_matches('/'), path);
        let resp = tokio::time::timeout(
            CALL_TIMEOUT,
            self.http
                .delete(&url)
                .bearer_auth(&self.token)
                .header("X-Otto-Session", self.session_id.clone().unwrap_or_default())
                .send(),
        )
        .await
        .map_err(|_| "upstream timeout".to_string())?
        .map_err(|e| format!("request failed: {e}"))?;
        let status = resp.status();
        if status.is_success() {
            return Ok(());
        }
        let bytes = resp.bytes().await.unwrap_or_default();
        let snippet = String::from_utf8_lossy(&bytes);
        Err(format!(
            "daemon returned {status}: {}",
            snippet.chars().take(300).collect::<String>()
        ))
    }

    /// The governed downstream tools the live-agent **gateway** exposes for this
    /// session's workspace, namespaced `mcp__<server>__<tool>`. Best-effort: an
    /// empty list (gateway off, no workspace, or the session user lacks MCP
    /// access) leaves the inward catalog unchanged.
    async fn gateway_tools(&self) -> Vec<Value> {
        let Some(ws) = &self.workspace_id else {
            eprintln!(
                "ottod mcp-tools: gateway tools skipped — session has no workspace_id; \
                 only the static read-only catalog is advertised"
            );
            return vec![];
        };
        match self.get_json(&format!("/mcp/gateway/tools?workspace_id={}", seg(ws))).await {
            Ok(v) => v.get("tools").and_then(Value::as_array).cloned().unwrap_or_default(),
            // Swallowing this silently made the advertised tool list vary between
            // otherwise-identical runs with no trace: when the gateway answered,
            // agents saw the downstream write tools; when it errored they saw a
            // read-only catalog and correctly reported the tool "missing". Same
            // code, same token, different answer — and nothing to grep for. Still
            // best-effort (the static catalog stands on its own), but never quiet.
            Err(e) => {
                eprintln!(
                    "ottod mcp-tools: gateway tools unavailable for workspace {ws}: {e} — \
                     advertising the static read-only catalog only; write tools \
                     (e.g. comment_pr) will NOT appear on this stdio surface"
                );
                vec![]
            }
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
                "description": "Read-only: list the Canvas Studio scenes (id/title/timestamps) in a workspace. Use canvas_create_scene to draw a new one.",
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
                "description": "Read-only: a Canvas Studio scene by id, including its full Scene JSON document (nodes/edges/slides). Use canvas_update_scene to edit it.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "scene_id": { "type": "string", "description": "Otto canvas scene id." }
                    },
                    "required": ["scene_id"]
                }
            },
            {
                "name": "canvas_create_scene",
                "description": "Create a Canvas scene (format: mermaid | d2 | excalidraw) and reference it to this session — it appears in the session's Canvas panel and the Canvas module. Uses this session's workspace (OTTO_WORKSPACE_ID). `source` is the mermaid/D2 diagram text (omit for excalidraw or to start blank).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "title": { "type": "string", "description": "Scene title." },
                        "format": { "type": "string", "description": "mermaid (default) | d2 | excalidraw." },
                        "source": { "type": "string", "description": "Mermaid/D2 diagram source text." },
                        "section": { "type": "string", "description": "Optional folder path to group the scene under, e.g. \"Platform/Staging\"." }
                    },
                    "required": ["title"]
                }
            },
            {
                "name": "canvas_update_scene",
                "description": "Replace a Canvas scene's diagram source (mermaid/D2 text) with new content, preserving its format. Use canvas_get_scene first to read the current source.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "scene_id": { "type": "string", "description": "Otto canvas scene id." },
                        "source": { "type": "string", "description": "The new mermaid/D2 diagram source text." }
                    },
                    "required": ["scene_id", "source"]
                }
            },
            {
                "name": "otto_list_workflows",
                "description": "Read-only: list this session's workspace's workflows (visual node-graph automations) — id, name, status.",
                "inputSchema": { "type": "object", "properties": {} }
            },
            {
                "name": "otto_get_workflow_run",
                "description": "Read-only: a workflow run's status, per-node step states and outputs, by run id.",
                "inputSchema": { "type": "object", "properties": { "run_id": { "type": "string", "description": "Workflow run id." } }, "required": ["run_id"] }
            },
            {
                "name": "otto_list_broker_clusters",
                "description": "Read-only: list this workspace's message-broker clusters (Kafka).",
                "inputSchema": { "type": "object", "properties": {} }
            },
            {
                "name": "otto_list_broker_topics",
                "description": "Read-only: list the topics of a broker cluster.",
                "inputSchema": { "type": "object", "properties": { "cluster_id": { "type": "string", "description": "Broker cluster id." } }, "required": ["cluster_id"] }
            },
            {
                "name": "otto_search_issues",
                "description": "Read-only: search Jira issues for an issue account. `query` is JQL (empty → most recent). Optional `project`.",
                "inputSchema": { "type": "object", "properties": { "account_id": { "type": "string" }, "query": { "type": "string" }, "project": { "type": "string" } }, "required": ["account_id"] }
            },
            {
                "name": "otto_list_swarms",
                "description": "Read-only: list this workspace's agent swarms.",
                "inputSchema": { "type": "object", "properties": {} }
            },
            {
                "name": "swarm_list_projects",
                "description": "Read-only: list a swarm's projects (id, name, repo, goal). Project ids feed swarm_list_tasks / swarm_create_task.",
                "inputSchema": { "type": "object", "properties": { "swarm_id": { "type": "string" } }, "required": ["swarm_id"] }
            },
            {
                "name": "swarm_list_tasks",
                "description": "Read-only: list a project's board tasks (id, title, status, assignee, priority, depends_on).",
                "inputSchema": { "type": "object", "properties": { "project_id": { "type": "string" } }, "required": ["project_id"] }
            },
            {
                "name": "swarm_utilization",
                "description": "Read-only: a swarm's board-utilization snapshot — parallel cap vs live runs, ready vs open tasks by status, and which agents are busy/idle. Use it to spot wasted capacity before dispatching.",
                "inputSchema": { "type": "object", "properties": { "swarm_id": { "type": "string" } }, "required": ["swarm_id"] }
            },
            {
                "name": "swarm_create_task",
                "description": "Create a task on a project board (Editor-gated). Unassigned tasks are auto-assigned to the best-fitting agent when scheduled. Managers: prefer editing the plan over piling on new tasks.",
                "inputSchema": { "type": "object", "properties": { "project_id": { "type": "string" }, "title": { "type": "string" }, "description": { "type": "string" }, "assignee_agent_id": { "type": "string" }, "priority": { "type": "string", "description": "low|medium|high|urgent" } }, "required": ["project_id", "title"] }
            },
            {
                "name": "swarm_update_task",
                "description": "Update a board task (Editor-gated): status (backlog|todo|in_progress|in_review|blocked|done|cancelled), assignee_agent_id, priority, title, description. Use to unblock, reassign, reprioritize or close stale items.",
                "inputSchema": { "type": "object", "properties": { "task_id": { "type": "string" }, "status": { "type": "string" }, "assignee_agent_id": { "type": "string" }, "priority": { "type": "string" }, "title": { "type": "string" }, "description": { "type": "string" } }, "required": ["task_id"] }
            },
            {
                "name": "swarm_run_task",
                "description": "Dispatch a board task now (Editor-gated): creates a run and launches the assignee's agent session immediately instead of waiting for the coordinator tick.",
                "inputSchema": { "type": "object", "properties": { "task_id": { "type": "string" } }, "required": ["task_id"] }
            },
            {
                "name": "swarm_stop_run",
                "description": "Stop an in-flight swarm run (Editor-gated): cancels the turn and marks the run stopped. Use on wedged or duplicate dispatches.",
                "inputSchema": { "type": "object", "properties": { "run_id": { "type": "string" } }, "required": ["run_id"] }
            },
            {
                "name": "otto_search_memory",
                "description": "Read-only: keyword (FTS) search of this workspace's agent memories for a free-text query; returns the top hits.",
                "inputSchema": { "type": "object", "properties": { "query": { "type": "string" }, "k": { "type": "integer" } }, "required": ["query"] }
            },
            {
                "name": "otto_list_repos",
                "description": "Read-only: list this workspace's git repositories (id, name, branch, remote).",
                "inputSchema": { "type": "object", "properties": {} }
            },
            {
                "name": "otto_list_sessions",
                "description": "Read-only: list this workspace's agent/terminal sessions (your own unless you are an admin).",
                "inputSchema": { "type": "object", "properties": {} }
            },
            {
                "name": "otto_list_product_stories",
                "description": "Read-only: list this workspace's product stories.",
                "inputSchema": { "type": "object", "properties": {} }
            },
            {
                "name": "otto_list_findings",
                "description": "Read-only: list a code review's findings (with workflow state) by review id.",
                "inputSchema": { "type": "object", "properties": { "review_id": { "type": "string" } }, "required": ["review_id"] }
            },
            {
                "name": "otto_list_prs",
                "description": "Read-only: list a repo's pull requests (number, title, state, source/destination branches, author, url). Use this to find the PR number for otto_get_pr / otto_comment_pr.",
                "inputSchema": { "type": "object", "properties": { "repo_id": { "type": "string" } }, "required": ["repo_id"] }
            },
            {
                "name": "otto_get_pr",
                "description": "Read-only: one pull request plus its existing review comments (id, body, path, line, thread_id). Call this BEFORE commenting so you can dedupe and reply in-thread instead of posting duplicates.",
                "inputSchema": { "type": "object", "properties": { "repo_id": { "type": "string" }, "pr_number": { "type": "integer" } }, "required": ["repo_id", "pr_number"] }
            },
            {
                "name": "otto_comment_pr",
                "description": "Post a comment on a pull request. Pass `path` (and optionally `line`) to anchor it INLINE on a file in the diff; omit both for a PR-level comment; pass `in_reply_to` with an existing comment id to reply in that thread. Mutating and outward-facing — the comment is visible to everyone on the PR.",
                "inputSchema": { "type": "object", "properties": {
                    "repo_id": { "type": "string" },
                    "pr_number": { "type": "integer" },
                    "body": { "type": "string", "description": "Markdown comment body." },
                    "path": { "type": "string", "description": "Repo-relative file path to anchor the comment to." },
                    "line": { "type": "integer", "description": "1-indexed line in `path` to anchor to." },
                    "in_reply_to": { "type": "string", "description": "Existing comment id to reply to." }
                }, "required": ["repo_id", "pr_number", "body"] }
            },
            {
                "name": "otto_usage_summary",
                "description": "Read-only: token-usage rollups by provider/day/session/feature (root-only endpoint; non-root callers get a clean error). Optional `days` (default 30).",
                "inputSchema": { "type": "object", "properties": { "days": { "type": "integer" } } }
            },
            {
                "name": "otto_list_improvement_runs",
                "description": "Read-only: list this workspace's self-improvement runs (status + summary).",
                "inputSchema": { "type": "object", "properties": {} }
            },
            {
                "name": "otto_list_improvement_edits",
                "description": "Read-only: list this workspace's self-improvement edit suggestions (pending/applied) with their status.",
                "inputSchema": { "type": "object", "properties": {} }
            },
            {
                "name": "otto_vault_list",
                "description": "Read-only: list this workspace's markdown doc vaults (id, name, root_path, okf, note/link counts, scan state). Vault ids feed every other otto_vault_* tool.",
                "inputSchema": { "type": "object", "properties": {} }
            },
            {
                "name": "otto_vault_dir",
                "description": "Read-only: one level of a vault's folder tree — subfolders (with child counts), notes and attachments. Empty `path` = vault root. Read `index.md` first in OKF vaults (progressive disclosure).",
                "inputSchema": { "type": "object", "properties": { "vault_id": { "type": "integer" }, "path": { "type": "string", "description": "Vault-relative folder path; empty for root." } }, "required": ["vault_id"] }
            },
            {
                "name": "otto_vault_read",
                "description": "Read-only: a note's raw markdown + indexed metadata (frontmatter, tags, aliases, headings, word count, hash for optimistic writes) + outgoing links (resolved).",
                "inputSchema": { "type": "object", "properties": { "vault_id": { "type": "integer" }, "path": { "type": "string", "description": "Vault-relative note path incl. .md" } }, "required": ["vault_id", "path"] }
            },
            {
                "name": "otto_vault_search",
                "description": "Read-only: full-text (FTS5) search over a vault's notes with snippets; supports tag:/path:/type: operators inside the query.",
                "inputSchema": { "type": "object", "properties": { "vault_id": { "type": "integer" }, "query": { "type": "string" }, "limit": { "type": "integer" } }, "required": ["vault_id", "query"] }
            },
            {
                "name": "otto_vault_backlinks",
                "description": "Read-only: notes that link TO a given note (linked mentions), each with a context snippet.",
                "inputSchema": { "type": "object", "properties": { "vault_id": { "type": "integer" }, "path": { "type": "string" } }, "required": ["vault_id", "path"] }
            },
            {
                "name": "otto_vault_tags",
                "description": "Read-only: every tag in a vault with its note count.",
                "inputSchema": { "type": "object", "properties": { "vault_id": { "type": "integer" } }, "required": ["vault_id"] }
            },
            {
                "name": "otto_vault_graph",
                "description": "Read-only: the vault link graph in a compact form (parallel node arrays + flat [src,dst,...] edge index pairs). Defaults to the LOCAL neighborhood of `path` (depth 1-3); mode=full returns the whole graph (edge-budgeted).",
                "inputSchema": { "type": "object", "properties": { "vault_id": { "type": "integer" }, "mode": { "type": "string", "description": "local (default when path given) | full" }, "path": { "type": "string" }, "depth": { "type": "integer" } }, "required": ["vault_id"] }
            },
            {
                "name": "otto_vault_okf_validate",
                "description": "Read-only: deterministic OKF v0.1 conformance report for a vault — errors E1 (no/unparseable frontmatter), E2 (missing type), E3 (reserved-file structure); warnings W1-W5. Never eyeball conformance: run this.",
                "inputSchema": { "type": "object", "properties": { "vault_id": { "type": "integer" } }, "required": ["vault_id"] }
            },
            {
                "name": "otto_vault_write",
                "description": "Create or update a markdown note in a doc vault (Editor-gated; parent folders auto-created). Pass `if_hash` from otto_vault_read for optimistic concurrency (\"\" = must-not-exist). Prefer OKF format: YAML frontmatter with `type` (+ one-sentence description), markdown links, then otto_vault_okf_validate.",
                "inputSchema": { "type": "object", "properties": { "vault_id": { "type": "integer" }, "path": { "type": "string", "description": "Vault-relative note path incl. .md" }, "content": { "type": "string" }, "if_hash": { "type": "string" } }, "required": ["vault_id", "path", "content"] }
            },
            {
                "name": "otto_vault_write_file",
                "description": "Create or update a guarded UTF-8 documentation artifact (Editor-gated): OpenAPI YAML, JSON, D2, Mermaid, text, or CSV. Parent folders are auto-created; pass `if_hash` for optimistic concurrency. Markdown must use otto_vault_write.",
                "inputSchema": { "type": "object", "properties": { "vault_id": { "type": "integer" }, "path": { "type": "string", "description": "Vault-relative path ending .yaml/.yml/.json/.d2/.mmd/.txt/.csv" }, "content": { "type": "string" }, "if_hash": { "type": "string" } }, "required": ["vault_id", "path", "content"] }
            },
            {
                "name": "otto_vault_rename",
                "description": "Rename/move a note or folder (Editor-gated). Every referencing wikilink/markdown link across the vault is rewritten on disk; returns links_updated.",
                "inputSchema": { "type": "object", "properties": { "vault_id": { "type": "integer" }, "from": { "type": "string" }, "to": { "type": "string" } }, "required": ["vault_id", "from", "to"] }
            },
            {
                "name": "otto_vault_delete",
                "description": "Soft-delete a note (Editor-gated): moves it into the vault's .trash/ folder — never destroys files. Prefer an OKF **Deprecation** log entry over deletion for knowledge that aged out.",
                "inputSchema": { "type": "object", "properties": { "vault_id": { "type": "integer" }, "path": { "type": "string" } }, "required": ["vault_id", "path"] }
            },
            {
                "name": "browser_navigate",
                "description": "Open a reader-mode browser tab on `url` (fetches it, netguard-checked — loopback/private/metadata addresses are refused) and return its fetched title. The tab appears in this workspace's Browser module.",
                "inputSchema": { "type": "object", "properties": { "url": { "type": "string" } }, "required": ["url"] }
            },
            {
                "name": "browser_page",
                "description": "Read-only: fetch a URL (netguard-checked) and return its extracted markdown, title, and which engine rendered it. `degraded:true` means the plain-fetch fallback ran (no JS execution).",
                "inputSchema": { "type": "object", "properties": { "url": { "type": "string" } }, "required": ["url"] }
            },
            {
                "name": "browser_query",
                "description": "Read-only: fetch a URL (netguard-checked, same as browser_page) and return every node matching a CSS `selector` — outer HTML and text content per match.",
                "inputSchema": { "type": "object", "properties": { "url": { "type": "string" }, "selector": { "type": "string" } }, "required": ["url", "selector"] }
            },
            {
                "name": "browser_summarize",
                "description": "Fetch a URL (netguard-checked) and run one short-lived agent turn to summarize its markdown (capped at 30k chars) for a developer notebook.",
                "inputSchema": { "type": "object", "properties": { "url": { "type": "string" } }, "required": ["url"] }
            },
            {
                "name": "browser_login",
                "description": "Sign in to a stored site credential for `domain` (a Site Credential the user created and explicitly marked \"allow agent use\" — otherwise this is refused). The daemon resolves the credential server-side, drives the fill+submit over CDP, and returns only whether it worked — the password never enters this tool's arguments, result, or the audit log.",
                "inputSchema": { "type": "object", "properties": { "domain": { "type": "string" } }, "required": ["domain"] }
            },
            {
                "name": "otto_room_post",
                "description": "Personal agents: post a message (max 16KB) into an agent room this agent is a member of. The message is persisted and shown to the user live — rooms are the only agent-to-agent channel. Your session identity determines which agent is speaking.",
                "inputSchema": { "type": "object", "properties": { "room_id": { "type": "string" }, "text": { "type": "string" } }, "required": ["room_id", "text"] }
            },
            {
                "name": "otto_room_read",
                "description": "Personal agents: read messages from an agent room this agent is a member of, oldest first. Pass `after` (the last message id you saw) to page forward.",
                "inputSchema": { "type": "object", "properties": { "room_id": { "type": "string" }, "after": { "type": "string" }, "limit": { "type": "integer" } }, "required": ["room_id"] }
            }
        ]
    })
}

const VAULT_MUTATION_TOOLS: [&str; 4] = [
    "otto_vault_write",
    "otto_vault_write_file",
    "otto_vault_rename",
    "otto_vault_delete",
];

const VAULT_REVIEW_READ_TOOLS: [(&str, &str); 8] = [
    ("otto_vault_list", "otto.vault_list"),
    ("otto_vault_dir", "otto.vault_dir"),
    ("otto_vault_read", "otto.vault_read"),
    ("otto_vault_search", "otto.vault_search"),
    ("otto_vault_backlinks", "otto.vault_backlinks"),
    ("otto_vault_tags", "otto.vault_tags"),
    ("otto_vault_graph", "otto.vault_graph"),
    ("otto_vault_okf_validate", "otto.vault_okf_validate"),
];

fn is_vault_docs_reviewer(source: Option<&str>) -> bool {
    source == Some("vault-docs-review")
}

fn tool_catalog_for_source(source: Option<&str>) -> Value {
    let mut catalog = tool_catalog();
    if is_vault_docs_reviewer(source) {
        if let Some(tools) = catalog["tools"].as_array_mut() {
            tools.retain(|tool| {
                tool["name"]
                    .as_str()
                    .is_some_and(|name| {
                        VAULT_REVIEW_READ_TOOLS
                            .iter()
                            .any(|(internal, _)| *internal == name)
                    })
            });
        }
    }
    catalog
}

/// Map the compatibility names exposed by the in-session MCP bridge onto the
/// governed outward tools. Workspace identity is injected from the session;
/// it is also pinned in the persisted token scope and checked server-side.
fn reviewer_read_invoke(name: &str, args: &Value, workspace_id: &str) -> Option<(String, Value)> {
    let outward = VAULT_REVIEW_READ_TOOLS
        .iter()
        .find_map(|(internal, outward)| (*internal == name).then_some(*outward))?;
    let mut mapped = args.as_object().cloned().unwrap_or_default();
    mapped.insert(
        "workspace_id".to_string(),
        Value::String(workspace_id.to_string()),
    );
    Some((outward.to_string(), Value::Object(mapped)))
}

async fn run_reviewer_read(ctx: &Ctx, name: &str, args: &Value) -> Result<Value, String> {
    let workspace = ctx
        .workspace_id
        .as_deref()
        .ok_or("no workspace context (OTTO_WORKSPACE_ID unset)")?;
    let (tool, arguments) = reviewer_read_invoke(name, args, workspace)
        .ok_or_else(|| format!("tool `{name}` is outside the documentation reviewer scope"))?;
    let envelope = ctx
        .post_json(
            "/mcp/otto-tools/invoke",
            &json!({"tool": tool, "arguments": arguments}),
        )
        .await?;
    if envelope.get("decision").and_then(Value::as_str) != Some("allowed")
        || envelope.get("executed").and_then(Value::as_bool) != Some(true)
    {
        return Err(envelope
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("governed reviewer read was denied")
            .to_string());
    }
    Ok(envelope.get("content").cloned().unwrap_or(Value::Null))
}

/// The new first-party feature read tools route through one pure mapping
/// ([`read_route`]); this is the set the dispatcher recognises before the
/// catch-all "unknown tool". All are reads (GET, or a read-only viewer POST).
const FEATURE_READ_TOOLS: &[&str] = &[
    "otto_list_workflows",
    "otto_get_workflow_run",
    "otto_list_broker_clusters",
    "otto_list_broker_topics",
    "otto_search_issues",
    "otto_list_swarms",
    "swarm_list_projects",
    "swarm_list_tasks",
    "swarm_utilization",
    "otto_search_memory",
    "otto_list_repos",
    "otto_list_prs",
    "otto_get_pr",
    "otto_list_sessions",
    "otto_list_product_stories",
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
];

/// A resolved upstream read call: GET (or a read-only viewer POST), the `/api/v1`-
/// relative path, and an optional JSON body. Built purely from `(name, args, ws)`
/// by [`read_route`] so each feature read's endpoint binding is unit-tested.
#[derive(Debug, Clone, PartialEq)]
struct ReadCall {
    post: bool,
    path: String,
    body: Option<Value>,
}

impl ReadCall {
    fn get(path: String) -> Self {
        Self { post: false, path, body: None }
    }
    fn post(path: String, body: Value) -> Self {
        Self { post: true, path, body: Some(body) }
    }
}

/// Map a feature read tool + its arguments to the upstream daemon read. Pure: no
/// I/O. Workspace-scoped tools use the session's workspace (`ws`); others take an
/// explicit id argument. Every path is a GET or the read-only `/memory/search`
/// viewer POST — no tool here mutates.
fn read_route(name: &str, args: &Value, ws: Option<&str>) -> Result<ReadCall, String> {
    let ws_req = || {
        ws.filter(|s| !s.is_empty())
            .ok_or_else(|| "no workspace context (OTTO_WORKSPACE_ID unset)".to_string())
    };
    Ok(match name {
        "otto_list_workflows" => ReadCall::get(format!("/workspaces/{}/workflows", seg(ws_req()?))),
        "otto_get_workflow_run" => {
            ReadCall::get(format!("/workflow-runs/{}", seg(&arg_str(args, "run_id")?)))
        }
        "otto_list_broker_clusters" => {
            ReadCall::get(format!("/workspaces/{}/brokers/clusters", seg(ws_req()?)))
        }
        "otto_list_broker_topics" => {
            ReadCall::get(format!("/brokers/clusters/{}/topics", seg(&arg_str(args, "cluster_id")?)))
        }
        "otto_search_issues" => {
            let acc = arg_str(args, "account_id")?;
            let mut path = format!("/issue/search?account_id={}", seg(&acc));
            if let Some(q) = args.get("query").and_then(Value::as_str) {
                path.push_str(&format!("&q={}", seg(q)));
            }
            if let Some(p) = args.get("project").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                path.push_str(&format!("&project={}", seg(p)));
            }
            ReadCall::get(path)
        }
        "otto_list_swarms" => ReadCall::get(format!("/workspaces/{}/swarm/swarms", seg(ws_req()?))),
        "swarm_list_projects" => {
            ReadCall::get(format!("/swarm/swarms/{}/projects", seg(&arg_str(args, "swarm_id")?)))
        }
        "swarm_list_tasks" => {
            ReadCall::get(format!("/swarm/projects/{}/tasks", seg(&arg_str(args, "project_id")?)))
        }
        "swarm_utilization" => {
            ReadCall::get(format!("/swarm/swarms/{}/utilization", seg(&arg_str(args, "swarm_id")?)))
        }
        "otto_search_memory" => {
            // `k` defaults to 0 server-side (MemoryQuery) → no hits; supply a useful default.
            let k = args.get("k").and_then(Value::as_u64).unwrap_or(20);
            let body = json!({ "text": arg_str(args, "query")?, "k": k });
            ReadCall::post(format!("/workspaces/{}/memory/search", seg(ws_req()?)), body)
        }
        "otto_list_repos" => ReadCall::get(format!("/workspaces/{}/repos", seg(ws_req()?))),
        "otto_list_sessions" => ReadCall::get(format!("/workspaces/{}/sessions", seg(ws_req()?))),
        "otto_list_product_stories" => {
            ReadCall::get(format!("/workspaces/{}/product/stories", seg(ws_req()?)))
        }
        "otto_list_findings" => {
            ReadCall::get(format!("/reviews/{}/findings", seg(&arg_str(args, "review_id")?)))
        }
        "otto_list_prs" => {
            ReadCall::get(format!("/repos/{}/prs", seg(&arg_str(args, "repo_id")?)))
        }
        "otto_get_pr" => ReadCall::get(format!(
            "/repos/{}/prs/{}",
            seg(&arg_str(args, "repo_id")?),
            arg_u64(args, "pr_number")?
        )),
        "otto_usage_summary" => {
            let mut path = "/usage/summary".to_string();
            if let Some(d) = args.get("days").and_then(Value::as_u64) {
                path.push_str(&format!("?days={d}"));
            }
            ReadCall::get(path)
        }
        "otto_list_improvement_runs" => {
            ReadCall::get(format!("/workspaces/{}/improvement/runs", seg(ws_req()?)))
        }
        "otto_list_improvement_edits" => {
            ReadCall::get(format!("/workspaces/{}/improvement/edits", seg(ws_req()?)))
        }
        "otto_vault_list" => ReadCall::get(format!("/workspaces/{}/vault/vaults", seg(ws_req()?))),
        "otto_vault_dir" => {
            let v = arg_i64(args, "vault_id")?;
            let path = args.get("path").and_then(Value::as_str).unwrap_or("");
            ReadCall::get(format!("/workspaces/{}/vault/vaults/{v}/dir?path={}", seg(ws_req()?), seg(path)))
        }
        "otto_vault_read" => {
            let v = arg_i64(args, "vault_id")?;
            ReadCall::get(format!(
                "/workspaces/{}/vault/vaults/{v}/note?path={}",
                seg(ws_req()?),
                seg(&arg_str(args, "path")?)
            ))
        }
        "otto_vault_search" => {
            let v = arg_i64(args, "vault_id")?;
            let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(20);
            let body = json!({ "query": arg_str(args, "query")?, "limit": limit });
            ReadCall::post(format!("/workspaces/{}/vault/vaults/{v}/search", seg(ws_req()?)), body)
        }
        "otto_vault_backlinks" => {
            let v = arg_i64(args, "vault_id")?;
            ReadCall::get(format!(
                "/workspaces/{}/vault/vaults/{v}/backlinks?path={}",
                seg(ws_req()?),
                seg(&arg_str(args, "path")?)
            ))
        }
        "otto_vault_tags" => {
            let v = arg_i64(args, "vault_id")?;
            ReadCall::get(format!("/workspaces/{}/vault/vaults/{v}/tags", seg(ws_req()?)))
        }
        "otto_vault_graph" => {
            let v = arg_i64(args, "vault_id")?;
            let mut path = format!("/workspaces/{}/vault/vaults/{v}/graph", seg(ws_req()?));
            let focus = args.get("path").and_then(Value::as_str).filter(|s| !s.is_empty());
            let mode = args
                .get("mode")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or(if focus.is_some() { "local" } else { "full" });
            path.push_str(&format!("?mode={}", seg(mode)));
            if let Some(f) = focus {
                path.push_str(&format!("&path={}", seg(f)));
            }
            if let Some(d) = args.get("depth").and_then(Value::as_u64) {
                path.push_str(&format!("&depth={d}"));
            }
            ReadCall::get(path)
        }
        "otto_vault_okf_validate" => {
            let v = arg_i64(args, "vault_id")?;
            ReadCall::post(
                format!("/workspaces/{}/vault/vaults/{v}/okf/validate", seg(ws_req()?)),
                json!({}),
            )
        }
        other => return Err(format!("unknown feature read tool `{other}`")),
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

/// Required unsigned integer. Accepts a JSON number or a numeric string — some
/// MCP clients stringify every argument, and a PR number arriving as `"52"`
/// should not read as "missing".
fn arg_u64(args: &Value, key: &str) -> Result<u64, String> {
    match args.get(key) {
        Some(Value::Number(n)) => n
            .as_u64()
            .ok_or_else(|| format!("argument `{key}` must be a non-negative integer")),
        Some(Value::String(s)) => s
            .trim()
            .parse::<u64>()
            .map_err(|_| format!("argument `{key}` must be a non-negative integer")),
        Some(_) => Err(format!("argument `{key}` must be a number")),
        None => Err(format!("missing required integer argument `{key}`")),
    }
}

/// Required string that may be explicitly empty (valid for file content).
fn arg_string_allow_empty(args: &Value, key: &str) -> Result<String, String> {
    match args.get(key) {
        Some(Value::String(value)) => Ok(value.clone()),
        Some(_) => Err(format!("argument `{key}` must be a string")),
        None => Err(format!("missing required string argument `{key}`")),
    }
}

fn arg_optional_string(args: &Value, key: &str) -> Result<Option<String>, String> {
    match args.get(key) {
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(format!("argument `{key}` must be a string")),
        None => Ok(None),
    }
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
        // Personal-agent room tools: the calling session's id (from the spawn
        // env, set by the daemon) is injected so the server can resolve which
        // personal agent is speaking via the session's `meta.personal_agent`
        // and enforce room membership. A session-less caller posts as the user.
        "otto_room_post" => {
            let room = arg_str(args, "room_id")?;
            let text = arg_str(args, "text")?;
            let mut body = json!({ "text": text });
            if let Some(sid) = ctx.session_id.clone() {
                body["session_id"] = json!(sid);
            }
            let raw = ctx
                .post_json(&format!("/agent-rooms/{}/messages", seg(&room)), &body)
                .await?;
            Ok(finalize(json!({ "message": raw })))
        }
        "otto_room_read" => {
            let room = arg_str(args, "room_id")?;
            let mut q = String::new();
            if let Some(after) = args.get("after").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                q.push_str(&format!("&after={}", seg(after)));
            }
            if let Some(limit) = args.get("limit").and_then(Value::as_i64) {
                q.push_str(&format!("&limit={limit}"));
            }
            if let Some(sid) = ctx.session_id.clone() {
                q.push_str(&format!("&session_id={}", seg(&sid)));
            }
            let raw = ctx
                .get_json(&format!(
                    "/agent-rooms/{}/messages?{}",
                    seg(&room),
                    q.trim_start_matches('&')
                ))
                .await?;
            Ok(finalize(json!({ "messages": raw })))
        }
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
            // `node` (raw) wins over `database`. The active-DB `node` is a PLAIN
            // name for SQL/Mongo (e.g. "shopdb" → `USE shopdb`); Redis selects a
            // keyspace via a raw `node` like "kdb:0".
            if let Some(node) = args.get("node").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                body["node"] = json!(node);
            } else if let Some(db) =
                args.get("database").and_then(Value::as_str).filter(|s| !s.is_empty())
            {
                body["node"] = json!(db);
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
            let source = match args.get("source").and_then(Value::as_str).filter(|s| !s.is_empty()) {
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
            if let Some(section) = args.get("section").and_then(Value::as_str).filter(|s| !s.is_empty()) {
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

            Ok(finalize(json!({ "scene_id": scene_id, "workspace_id": ws })))
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

            ctx.put_json(&format!("/canvas/scenes/{}", seg(&scene_id)), &json!({ "doc": new_doc }))
                .await?;

            Ok(finalize(json!({ "ok": true, "format": format })))
        }
        // Vault v3 doc writers — Editor-gated by the daemon route; the delete is
        // a soft move into `.trash/`.
        "swarm_create_task" => {
            let pid = arg_str(args, "project_id")?;
            let mut body = json!({ "title": arg_str(args, "title")? });
            for k in ["description", "assignee_agent_id", "priority"] {
                if let Some(v) = args.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()) {
                    body[k] = json!(v);
                }
            }
            let task = ctx.post_json(&format!("/swarm/projects/{}/tasks", seg(&pid)), &body).await?;
            Ok(finalize(json!({ "ok": true, "task": task })))
        }
        "swarm_update_task" => {
            let tid = arg_str(args, "task_id")?;
            let mut body = json!({});
            for k in ["status", "assignee_agent_id", "priority", "title", "description"] {
                if let Some(v) = args.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()) {
                    body[k] = json!(v);
                }
            }
            if body.as_object().is_some_and(|o| o.is_empty()) {
                return Err("nothing to update — pass at least one of status/assignee_agent_id/priority/title/description".into());
            }
            let task = ctx.patch_json(&format!("/swarm/tasks/{}", seg(&tid)), &body).await?;
            Ok(finalize(json!({ "ok": true, "task": task })))
        }
        "swarm_run_task" => {
            let tid = arg_str(args, "task_id")?;
            let run = ctx.post_json(&format!("/swarm/tasks/{}/run", seg(&tid)), &json!({})).await?;
            Ok(finalize(json!({ "ok": true, "run": run })))
        }
        "swarm_stop_run" => {
            let rid = arg_str(args, "run_id")?;
            let run = ctx.post_json(&format!("/swarm/runs/{}/stop", seg(&rid)), &json!({})).await?;
            Ok(finalize(json!({ "ok": true, "run": run })))
        }
        "otto_vault_write" => {
            let ws = ctx.workspace_id.clone().ok_or("no workspace context (OTTO_WORKSPACE_ID unset)")?;
            let v = arg_i64(args, "vault_id")?;
            let mut body = json!({
                "path": arg_str(args, "path")?,
                "content": arg_string_allow_empty(args, "content")?,
            });
            if let Some(h) = arg_optional_string(args, "if_hash")? {
                body["if_hash"] = json!(h);
            }
            let meta = ctx
                .put_json(&format!("/workspaces/{}/vault/vaults/{v}/note", seg(&ws)), &body)
                .await?;
            Ok(finalize(json!({ "ok": true, "meta": meta })))
        }
        "otto_vault_write_file" => {
            let ws = ctx.workspace_id.clone().ok_or("no workspace context (OTTO_WORKSPACE_ID unset)")?;
            let v = arg_i64(args, "vault_id")?;
            let mut body = json!({
                "path": arg_str(args, "path")?,
                "content": arg_string_allow_empty(args, "content")?,
            });
            if let Some(h) = arg_optional_string(args, "if_hash")? {
                body["if_hash"] = json!(h);
            }
            let file = ctx
                .put_json(&format!("/workspaces/{}/vault/vaults/{v}/file", seg(&ws)), &body)
                .await?;
            Ok(finalize(json!({ "ok": true, "file": file })))
        }
        "otto_vault_rename" => {
            let ws = ctx.workspace_id.clone().ok_or("no workspace context (OTTO_WORKSPACE_ID unset)")?;
            let v = arg_i64(args, "vault_id")?;
            let body = json!({ "from": arg_str(args, "from")?, "to": arg_str(args, "to")? });
            let res = ctx
                .post_json(&format!("/workspaces/{}/vault/vaults/{v}/rename", seg(&ws)), &body)
                .await?;
            Ok(finalize(res))
        }
        "otto_vault_delete" => {
            let ws = ctx.workspace_id.clone().ok_or("no workspace context (OTTO_WORKSPACE_ID unset)")?;
            let v = arg_i64(args, "vault_id")?;
            ctx.delete_ok(&format!(
                "/workspaces/{}/vault/vaults/{v}/note?path={}",
                seg(&ws),
                seg(&arg_str(args, "path")?)
            ))
            .await?;
            Ok(finalize(json!({ "ok": true, "trashed": args.get("path") })))
        }
        // Browser tools — all call the governed browser routes with the
        // session's own token (netguard + RBAC enforced there, same as the
        // Browser module UI). `browser_navigate` is the only write: it opens
        // a reader-mode tab (a genuine new `browser_tabs` row), so it's an
        // explicit arm rather than a `FEATURE_READ_TOOLS` GET/POST mapping.
        "browser_navigate" => {
            let ws = ctx.workspace_id.clone().ok_or("no workspace context (OTTO_WORKSPACE_ID unset)")?;
            let url = arg_str(args, "url")?;
            let tab = ctx
                .post_json(&format!("/workspaces/{}/browser/tabs", seg(&ws)), &json!({ "url": url }))
                .await?;
            let tab_id = tab
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| "daemon did not return a tab id".to_string())?
                .to_string();
            // The tab is created in mode:"reader" by construction (see
            // `otto_state::browser::BrowserTabsRepo::create`), so this PATCH
            // runs the fetch pipeline and adopts the fetched page's title.
            let updated = ctx
                .patch_json(&format!("/browser/tabs/{}", seg(&tab_id)), &json!({ "url": url }))
                .await?;
            let title = updated.get("title").cloned().unwrap_or(Value::String(String::new()));
            Ok(finalize(json!({ "ok": true, "title": title })))
        }
        "browser_page" => {
            let Some(ws) = ctx.workspace_id.as_deref() else {
                return Err("no workspace context (OTTO_WORKSPACE_ID unset); cannot fetch a page".into());
            };
            let url = arg_str(args, "url")?;
            let raw = ctx
                .get_json(&format!("/workspaces/{}/browser/page?url={}", seg(ws), seg(&url)))
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
                return Err("no workspace context (OTTO_WORKSPACE_ID unset); cannot query a page".into());
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
                return Err("no workspace context (OTTO_WORKSPACE_ID unset); cannot summarize a page".into());
            };
            let url = arg_str(args, "url")?;
            let raw = ctx
                .post_json(&format!("/workspaces/{}/browser/summarize", seg(ws)), &json!({ "url": url }))
                .await?;
            Ok(finalize(raw))
        }
        // The daemon resolves the credential (`allow_agent_use` required,
        // else a typed 403) and drives the fill+submit itself — this arm only
        // ever sends/receives `domain`/`logged_in`/`engine`; the password
        // never reaches this process at all. `finalize`'s `redact_json` pass
        // is defense-in-depth on top of that, not the only guard.
        "browser_login" => {
            let Some(ws) = ctx.workspace_id.as_deref() else {
                return Err("no workspace context (OTTO_WORKSPACE_ID unset); cannot sign in".into());
            };
            let domain = arg_str(args, "domain")?;
            let raw = ctx
                .post_json(&format!("/workspaces/{}/browser/login", seg(ws)), &json!({ "domain": domain }))
                .await?;
            Ok(finalize(raw))
        }
        // First-party feature reads (workflows / brokers / issues / swarm / vault /
        // repos / sessions / product / findings / usage / self-improvement). All
        // route through the pure `read_route` and post-process identically: the raw
        // upstream value is capped + redacted by `finalize`.
        // PR comment — the one WRITE on this surface. It lives here rather than
        // behind the gateway because the review workflow's whole product is
        // comments on a PR, and routing it through a gateway that silently
        // returns an empty tool list on any error left agents reporting
        // "comment_pr does not exist" and the PR at zero comments. Same daemon
        // route the Otto UI uses; the caller's own session token authorizes it,
        // and the call is audited like every other.
        "otto_comment_pr" => {
            let repo_id = arg_str(args, "repo_id")?;
            let number = arg_u64(args, "pr_number")?;
            let body = arg_str(args, "body")?;
            let mut payload = json!({ "body": body });
            // path/line anchor the comment inline; in_reply_to threads it. All
            // optional — omitting them posts a PR-level comment.
            if let Some(path) = arg_optional_string(args, "path")? {
                payload["path"] = json!(path);
            }
            if let Some(line) = args.get("line").and_then(Value::as_u64) {
                payload["line"] = json!(line);
            }
            if let Some(reply) = arg_optional_string(args, "in_reply_to")? {
                payload["in_reply_to"] = json!(reply);
            }
            let raw = ctx
                .post_json(
                    &format!("/repos/{}/prs/{number}/comments", seg(&repo_id)),
                    &payload,
                )
                .await?;
            Ok(finalize(raw))
        }
        name if FEATURE_READ_TOOLS.contains(&name) => {
            let call = read_route(name, args, ctx.workspace_id.as_deref())?;
            let raw = if call.post {
                ctx.post_json(&call.path, call.body.as_ref().unwrap_or(&json!({})))
                    .await?
            } else {
                ctx.get_json(&call.path).await?
            };
            Ok(finalize(raw))
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
            let mut cat = tool_catalog_for_source(ctx.source.as_deref());
            let gw = if is_vault_docs_reviewer(ctx.source.as_deref()) {
                Vec::new()
            } else {
                ctx.gateway_tools().await
            };
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
            if is_vault_docs_reviewer(ctx.source.as_deref())
                && VAULT_MUTATION_TOOLS.contains(&name.as_str())
            {
                return Some(rpc_ok(
                    id,
                    tool_result(
                        &json!({ "error": "vault mutation tools are disabled for documentation review sessions" }),
                        true,
                    ),
                ));
            }
            if is_vault_docs_reviewer(ctx.source.as_deref()) {
                return Some(match run_reviewer_read(ctx, &name, &args).await {
                    Ok(value) => {
                        ctx.audit(&name, &args, true, None).await;
                        rpc_ok(id, tool_result(&value, false))
                    }
                    Err(error) => {
                        ctx.audit(&name, &args, false, None).await;
                        rpc_ok(id, tool_result(&json!({"error": error}), true))
                    }
                });
            }
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
    source: Option<String>,
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
        source: s("source"),
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
                source: s("OTTO_SESSION_SOURCE"),
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
    let source = creds.source;

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
        source,
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
            r#"{"token":"t-1","base":"http://127.0.0.1:7700","session_id":"s-1","workspace_id":"ws-1","source":"vault-docs-review"}"#,
        )
        .unwrap();
        assert_eq!(c.token, "t-1");
        assert_eq!(c.base.as_deref(), Some("http://127.0.0.1:7700"));
        assert_eq!(c.session_id.as_deref(), Some("s-1"));
        assert_eq!(c.workspace_id.as_deref(), Some("ws-1"));
        assert_eq!(c.source.as_deref(), Some("vault-docs-review"));
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

    #[test]
    fn catalog_lists_feature_read_tools() {
        let cat = tool_catalog();
        let names: Vec<&str> = cat["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        for t in FEATURE_READ_TOOLS {
            assert!(names.contains(t), "catalog missing feature read tool {t}");
        }
        // Every advertised tool carries an object inputSchema.
        for tool in cat["tools"].as_array().unwrap() {
            assert_eq!(
                tool["inputSchema"]["type"],
                json!("object"),
                "tool {} has no object inputSchema",
                tool["name"]
            );
        }
    }

    #[test]
    fn catalog_lists_the_canvas_write_tools() {
        let cat = tool_catalog();
        let names: Vec<&str> = cat["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"canvas_create_scene"), "catalog missing canvas_create_scene");
        assert!(names.contains(&"canvas_update_scene"), "catalog missing canvas_update_scene");
    }

    #[test]
    fn catalog_lists_the_browser_tools() {
        let cat = tool_catalog();
        let names: Vec<&str> = cat["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        for t in [
            "browser_navigate",
            "browser_page",
            "browser_query",
            "browser_summarize",
            "browser_login",
        ] {
            assert!(names.contains(&t), "catalog missing {t}");
        }
    }

    #[tokio::test]
    async fn browser_navigate_errors_without_url() {
        let ctx = test_ctx();
        let resp = handle(
            &ctx,
            json!({ "jsonrpc": "2.0", "id": 20, "method": "tools/call",
                    "params": { "name": "browser_navigate", "arguments": {} } }),
        )
        .await
        .unwrap();
        assert_eq!(resp["result"]["isError"], json!(true));
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("url"), "got: {text}");
    }

    #[tokio::test]
    async fn browser_navigate_errors_without_workspace() {
        let mut ctx = test_ctx();
        ctx.workspace_id = None;
        let resp = handle(
            &ctx,
            json!({ "jsonrpc": "2.0", "id": 21, "method": "tools/call",
                    "params": { "name": "browser_navigate", "arguments": { "url": "https://a.io" } } }),
        )
        .await
        .unwrap();
        assert_eq!(resp["result"]["isError"], json!(true));
        assert!(resp["result"]["content"][0]["text"].as_str().unwrap().contains("workspace"));
    }

    #[tokio::test]
    async fn browser_page_errors_without_url() {
        let ctx = test_ctx();
        let resp = handle(
            &ctx,
            json!({ "jsonrpc": "2.0", "id": 22, "method": "tools/call",
                    "params": { "name": "browser_page", "arguments": {} } }),
        )
        .await
        .unwrap();
        assert_eq!(resp["result"]["isError"], json!(true));
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("url"), "got: {text}");
    }

    #[tokio::test]
    async fn browser_page_errors_without_workspace() {
        let mut ctx = test_ctx();
        ctx.workspace_id = None;
        let resp = handle(
            &ctx,
            json!({ "jsonrpc": "2.0", "id": 23, "method": "tools/call",
                    "params": { "name": "browser_page", "arguments": { "url": "https://a.io" } } }),
        )
        .await
        .unwrap();
        assert_eq!(resp["result"]["isError"], json!(true));
        assert!(resp["result"]["content"][0]["text"].as_str().unwrap().contains("workspace"));
    }

    #[tokio::test]
    async fn browser_query_errors_without_selector() {
        let ctx = test_ctx();
        let resp = handle(
            &ctx,
            json!({ "jsonrpc": "2.0", "id": 24, "method": "tools/call",
                    "params": { "name": "browser_query", "arguments": { "url": "https://a.io" } } }),
        )
        .await
        .unwrap();
        assert_eq!(resp["result"]["isError"], json!(true));
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("selector"), "got: {text}");
    }

    #[tokio::test]
    async fn browser_summarize_errors_without_url() {
        let ctx = test_ctx();
        let resp = handle(
            &ctx,
            json!({ "jsonrpc": "2.0", "id": 25, "method": "tools/call",
                    "params": { "name": "browser_summarize", "arguments": {} } }),
        )
        .await
        .unwrap();
        assert_eq!(resp["result"]["isError"], json!(true));
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("url"), "got: {text}");
    }

    #[tokio::test]
    async fn browser_login_errors_without_domain() {
        let ctx = test_ctx();
        let resp = handle(
            &ctx,
            json!({ "jsonrpc": "2.0", "id": 26, "method": "tools/call",
                    "params": { "name": "browser_login", "arguments": {} } }),
        )
        .await
        .unwrap();
        assert_eq!(resp["result"]["isError"], json!(true));
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("domain"), "got: {text}");
    }

    #[tokio::test]
    async fn browser_login_errors_without_workspace() {
        let mut ctx = test_ctx();
        ctx.workspace_id = None;
        let resp = handle(
            &ctx,
            json!({ "jsonrpc": "2.0", "id": 27, "method": "tools/call",
                    "params": { "name": "browser_login", "arguments": { "domain": "example.com" } } }),
        )
        .await
        .unwrap();
        assert_eq!(resp["result"]["isError"], json!(true));
        assert!(resp["result"]["content"][0]["text"].as_str().unwrap().contains("workspace"));
    }

    #[test]
    fn catalog_lists_the_vault_tools() {
        let cat = tool_catalog();
        let names: Vec<&str> = cat["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        for t in [
            "otto_vault_list",
            "otto_vault_dir",
            "otto_vault_read",
            "otto_vault_search",
            "otto_vault_backlinks",
            "otto_vault_tags",
            "otto_vault_graph",
            "otto_vault_okf_validate",
            "otto_vault_write",
            "otto_vault_write_file",
            "otto_vault_rename",
            "otto_vault_delete",
        ] {
            assert!(names.contains(&t), "catalog missing {t}");
        }

        let write_file = cat["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == "otto_vault_write_file")
            .unwrap();
        assert_eq!(
            write_file["inputSchema"]["required"],
            json!(["vault_id", "path", "content"])
        );
        assert_eq!(write_file["inputSchema"]["properties"]["content"]["type"], json!("string"));
    }

    #[test]
    fn vault_docs_review_source_gets_read_only_vault_catalog() {
        let cat = tool_catalog_for_source(Some("vault-docs-review"));
        let names: Vec<&str> = cat["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        for read in ["otto_vault_list", "otto_vault_read", "otto_vault_search"] {
            assert!(names.contains(&read), "reviewer catalog missing {read}");
        }
        for mutation in [
            "otto_vault_write",
            "otto_vault_write_file",
            "otto_vault_rename",
            "otto_vault_delete",
        ] {
            assert!(!names.contains(&mutation), "reviewer catalog leaked {mutation}");
        }

        let normal = tool_catalog_for_source(Some("vault-docs"));
        assert!(normal["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["name"] == "otto_vault_write"));
    }

    #[test]
    fn reviewer_read_bridge_maps_only_vault_reads_to_governed_tools() {
        let (tool, args) = reviewer_read_invoke(
            "otto_vault_read",
            &json!({"vault_id": 7, "path": "api/widgets.md"}),
            "workspace-1",
        )
        .expect("vault read must be routed through the governed MCP choke point");
        assert_eq!(tool, "otto.vault_read");
        assert_eq!(args["workspace_id"], "workspace-1");
        assert_eq!(args["vault_id"], 7);
        assert_eq!(args["path"], "api/widgets.md");
        assert!(reviewer_read_invoke(
            "otto_vault_write",
            &json!({"vault_id": 7, "path": "x.md", "content": "x"}),
            "workspace-1",
        )
        .is_none());
    }

    #[tokio::test]
    async fn vault_docs_review_source_cannot_call_hidden_vault_mutations() {
        let mut ctx = test_ctx();
        ctx.source = Some("vault-docs-review".into());
        let response = handle(
            &ctx,
            json!({"jsonrpc":"2.0","id":31,"method":"tools/call","params":{
                "name":"otto_vault_write","arguments":{"vault_id":1,"path":"x.md","content":"x"}
            }}),
        )
        .await
        .unwrap();
        assert_eq!(response["result"]["isError"], json!(true));
        assert!(response["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("disabled for documentation review"));
    }

    #[tokio::test]
    async fn vault_write_file_requires_string_content_but_allows_explicit_empty() {
        let ctx = test_ctx();
        for arguments in [
            json!({ "vault_id": 1, "path": "api.json" }),
            json!({ "vault_id": 1, "path": "api.json", "content": 7 }),
        ] {
            let resp = handle(
                &ctx,
                json!({ "jsonrpc": "2.0", "id": 21, "method": "tools/call",
                        "params": { "name": "otto_vault_write_file", "arguments": arguments } }),
            )
            .await
            .unwrap();
            assert_eq!(resp["result"]["isError"], json!(true));
            let text = resp["result"]["content"][0]["text"].as_str().unwrap();
            assert!(text.contains("content"), "got: {text}");
        }

        let resp = handle(
            &ctx,
            json!({ "jsonrpc": "2.0", "id": 22, "method": "tools/call",
                    "params": { "name": "otto_vault_write_file", "arguments": {
                        "vault_id": 1, "path": "api.json", "content": "{}", "if_hash": 7
                    } } }),
        )
        .await
        .unwrap();
        assert_eq!(resp["result"]["isError"], json!(true));
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("if_hash"), "got: {text}");

        let err = run_tool(
            &ctx,
            "otto_vault_write_file",
            &json!({ "vault_id": 1, "path": "api.json", "content": "" }),
        )
        .await
        .unwrap_err();
        assert!(!err.contains("content"), "explicit empty content must pass argument validation: {err}");
    }

    #[tokio::test]
    async fn canvas_create_scene_errors_without_title() {
        let ctx = test_ctx();
        let resp = handle(
            &ctx,
            json!({ "jsonrpc": "2.0", "id": 11, "method": "tools/call",
                    "params": { "name": "canvas_create_scene", "arguments": {} } }),
        )
        .await
        .unwrap();
        assert_eq!(resp["result"]["isError"], json!(true));
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("title"), "got: {text}");
    }

    #[tokio::test]
    async fn canvas_create_scene_errors_on_bad_format() {
        let ctx = test_ctx();
        let resp = handle(
            &ctx,
            json!({ "jsonrpc": "2.0", "id": 12, "method": "tools/call",
                    "params": { "name": "canvas_create_scene",
                                 "arguments": { "title": "T", "format": "svg" } } }),
        )
        .await
        .unwrap();
        assert_eq!(resp["result"]["isError"], json!(true));
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("format"), "got: {text}");
    }

    #[tokio::test]
    async fn canvas_create_scene_errors_without_workspace() {
        let mut ctx = test_ctx();
        ctx.workspace_id = None;
        let resp = handle(
            &ctx,
            json!({ "jsonrpc": "2.0", "id": 13, "method": "tools/call",
                    "params": { "name": "canvas_create_scene", "arguments": { "title": "T" } } }),
        )
        .await
        .unwrap();
        assert_eq!(resp["result"]["isError"], json!(true));
        assert!(resp["result"]["content"][0]["text"].as_str().unwrap().contains("workspace"));
    }

    #[tokio::test]
    async fn canvas_update_scene_errors_without_source() {
        let ctx = test_ctx();
        let resp = handle(
            &ctx,
            json!({ "jsonrpc": "2.0", "id": 14, "method": "tools/call",
                    "params": { "name": "canvas_update_scene", "arguments": { "scene_id": "s1" } } }),
        )
        .await
        .unwrap();
        assert_eq!(resp["result"]["isError"], json!(true));
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("source"), "got: {text}");
    }

    #[test]
    fn read_route_maps_workspace_and_arg_tools() {
        let ws = Some("ws1");
        assert_eq!(
            read_route("otto_list_workflows", &json!({}), ws).unwrap(),
            ReadCall { post: false, path: "/workspaces/ws1/workflows".into(), body: None }
        );
        assert_eq!(
            read_route("otto_list_broker_clusters", &json!({}), ws).unwrap().path,
            "/workspaces/ws1/brokers/clusters"
        );
        assert_eq!(read_route("otto_get_workflow_run", &json!({"run_id":"r1"}), ws).unwrap().path, "/workflow-runs/r1");
        assert_eq!(read_route("otto_list_broker_topics", &json!({"cluster_id":"c1"}), ws).unwrap().path, "/brokers/clusters/c1/topics");
        assert_eq!(read_route("otto_list_findings", &json!({"review_id":"rv1"}), ws).unwrap().path, "/reviews/rv1/findings");
        let c = read_route("otto_search_memory", &json!({"query":"db","k":3}), ws).unwrap();
        assert!(c.post);
        assert_eq!(c.path, "/workspaces/ws1/memory/search");
        assert_eq!(c.body.unwrap(), json!({"text":"db","k":3}));
        assert_eq!(read_route("otto_usage_summary", &json!({"days":7}), ws).unwrap().path, "/usage/summary?days=7");
        let c = read_route("otto_search_issues", &json!({"account_id":"a1","query":"a = b"}), ws).unwrap();
        assert!(c.path.starts_with("/issue/search?account_id=a1"));
        assert!(c.path.contains("&q=a%20%3D%20b"), "got {}", c.path);
        assert_eq!(read_route("otto_list_improvement_edits", &json!({}), ws).unwrap().path, "/workspaces/ws1/improvement/edits");
        assert_eq!(read_route("otto_vault_list", &json!({}), ws).unwrap().path, "/workspaces/ws1/vault/vaults");
        assert_eq!(
            read_route("otto_vault_read", &json!({"vault_id":3,"path":"services/auth api.md"}), ws).unwrap().path,
            "/workspaces/ws1/vault/vaults/3/note?path=services%2Fauth%20api.md"
        );
        let c = read_route("otto_vault_search", &json!({"vault_id":3,"query":"jwt"}), ws).unwrap();
        assert!(c.post);
        assert_eq!(c.path, "/workspaces/ws1/vault/vaults/3/search");
        assert_eq!(c.body.unwrap()["limit"], json!(20));
        // Graph defaults: local when a focus path is given, full otherwise.
        assert!(read_route("otto_vault_graph", &json!({"vault_id":3,"path":"a.md"}), ws).unwrap().path.contains("mode=local"));
        assert!(read_route("otto_vault_graph", &json!({"vault_id":3}), ws).unwrap().path.contains("mode=full"));
        let c = read_route("otto_vault_okf_validate", &json!({"vault_id":3}), ws).unwrap();
        assert!(c.post);
        // Swarm board reads: explicit-id tools, no workspace needed.
        assert_eq!(read_route("swarm_list_projects", &json!({"swarm_id":"s1"}), ws).unwrap().path, "/swarm/swarms/s1/projects");
        assert_eq!(read_route("swarm_list_tasks", &json!({"project_id":"p1"}), ws).unwrap().path, "/swarm/projects/p1/tasks");
        assert_eq!(read_route("swarm_utilization", &json!({"swarm_id":"s1"}), ws).unwrap().path, "/swarm/swarms/s1/utilization");
    }

    #[test]
    fn catalog_lists_the_swarm_board_tools() {
        let tools = tool_catalog();
        let names: Vec<&str> = tools["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        for t in ["swarm_create_task", "swarm_update_task", "swarm_run_task", "swarm_stop_run"] {
            assert!(names.contains(&t), "catalog missing swarm write tool {t}");
        }
    }

    #[test]
    fn read_route_errors_without_workspace_or_required_arg() {
        assert!(read_route("otto_list_workflows", &json!({}), None).is_err());
        assert!(read_route("otto_get_workflow_run", &json!({}), Some("ws1")).is_err());
        assert!(read_route("otto_search_issues", &json!({}), Some("ws1")).is_err());
        assert!(read_route("nope", &json!({}), Some("ws1")).is_err());
    }

    #[tokio::test]
    async fn feature_read_errors_without_workspace_context() {
        let mut ctx = test_ctx();
        ctx.workspace_id = None;
        let resp = handle(
            &ctx,
            json!({ "jsonrpc": "2.0", "id": 7, "method": "tools/call",
                    "params": { "name": "otto_list_workflows", "arguments": {} } }),
        )
        .await
        .unwrap();
        assert_eq!(resp["result"]["isError"], json!(true));
        assert!(resp["result"]["content"][0]["text"].as_str().unwrap().contains("workspace"));
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
            source: None,
            audit: None,
        }
    }
}
