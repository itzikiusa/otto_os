//! `ottod mcp-tools` — the first-party Otto MCP tool server (Task B2b).
//! Nearly every tool here is **read-only**; the exceptions are
//! `canvas_create_scene` / `canvas_update_scene` (Task B5), the Vault v3
//! doc writers `otto_vault_write` / `otto_vault_rename` / `otto_vault_delete`,
//! the swarm board tools `swarm_create_task` / `swarm_update_task` /
//! `swarm_run_task` / `swarm_stop_run` (the manager's utilization levers),
//! `browser_navigate` (Task 6, opens a reader-mode browser tab), `workbench_write`
//! (writes the owner's Workbench scratch file as a new history revision), and the three
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
//! `otto_create_pr` still waits on a human approval — unless an explicit
//! auto-approve rule (MCP → Otto server → Auto-approve; global, this
//! workspace, or this session) covers it, in which case it runs at once and is
//! audited `auto_approved` (the envelope's `auto_approved_by` names the rule,
//! passed through to the agent unchanged). Native tools win by name
//! (see `governed_tools_for`), so what the control plane shows as enabled is
//! what a session can call, with no second hand-maintained list to drift.
//!
//! The Design Hall WRITES (`otto_design_assist` — start an agent turn that
//! commits a version, `otto_design_link` — file an explicit link) are
//! deliberately NOT native: they exist only as that governed bridge, so they
//! stay approval-gated (DANGEROUS) unless an auto-approve rule covers them, and no
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
//!   ephemeral and never saved; see `routes/browser.rs`).
//!   `canvas_create_scene`/`canvas_update_scene`, `otto_vault_write`/
//!   `otto_vault_rename`/`otto_vault_delete`, `browser_navigate`, and the
//!   cloud-console writers `aws_athena_query`/`aws_sqs_send`/`aws_sqs_peek`
//!   (a receive bumps each message's receive count — it can dead-letter)/`k8s_action`, plus
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
//! - **Audited** — every call appends a row to `mcp_tool_calls` (best-effort),
//!   through the daemon's `POST /mcp/tool-calls` — the bridge holds no
//!   database connection of its own.
//!
//! The transport is newline-delimited JSON-RPC 2.0 (one JSON object per line on
//! stdin/stdout), which is the MCP stdio framing claude/codex use.

use std::time::Duration;

use otto_core::redact::redact_json;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Instant;

use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};

use crate::config::Config;

// Per-route upstream budgets and the pure read-call descriptor:
// `mcp_tools/budgets.rs`.
mod budgets;
// Session credentials (env / Codex creds file): `mcp_tools/creds.rs`.
mod creds;
use creds::load_creds;
#[cfg(test)]
use creds::{config_path_in, parse_creds};
// MCP profiles — the list/dispatch filter hook: `mcp_tools/profile.rs`.
mod profile;
use budgets::{wait_session_budget, ReadCall, SUMMARIZE_CALL_TIMEOUT};
// Twin schemas generated from the control-plane catalog: `mcp_tools/listing.rs`.
mod listing;

/// Protocol version we advertise in `initialize`. Mirrors the spec revision the
/// bundled CLIs negotiate; clients echo their own and tolerate a match.
const PROTOCOL_VERSION: &str = "2024-11-05";

/// Per-call wall-clock timeout for an upstream daemon request (shared with the
/// spawner's client-side cap: `otto_core::mcp_budgets`).
const CALL_TIMEOUT: Duration = otto_core::mcp_budgets::CALL_TIMEOUT;
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
const TEXT_CALL_TIMEOUT: Duration = otto_core::mcp_budgets::TEXT_CALL_TIMEOUT;

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
    /// The session's MCP profile ids (`OTTO_MCP_PROFILES`); `None` ⇒ no
    /// profile filter (see `mcp_tools/profile.rs`).
    profiles: Option<String>,
    /// Whether tool calls are audited (`POST /mcp/tool-calls`; off in tests
    /// that never reach a daemon).
    audit: bool,
    /// In-flight audit posts. They run off the reply path and are drained
    /// when stdin closes so the last calls are still recorded.
    audit_tasks: std::sync::Mutex<tokio::task::JoinSet<()>>,
    /// The control-plane enable list (+ this session's UI grant), cached for
    /// [`ENABLED_CACHE_TTL`]: every governed call and `tools/list` used to
    /// fetch the full `/mcp/otto-server` status (~100 KB) for it. A stale
    /// entry can never widen access — `governed_invoke` re-checks the enable
    /// list itself, and the UI bridge the grant.
    enabled_cache: tokio::sync::Mutex<Option<(Instant, EnabledInfo)>>,
    /// The gateway's downstream tool list, cached for [`GATEWAY_CACHE_TTL`]
    /// (refetched once on a miss). The daemon re-authorizes every gateway call.
    gateway_cache: tokio::sync::Mutex<Option<(Instant, Vec<Value>)>>,
    /// Whether the last `tools/list` advertised the `otto_ui_*` tools (the
    /// session held the UI grant). When the grant flips, the bridge emits
    /// `notifications/tools/list_changed` so the client re-lists.
    advertised_ui: std::sync::atomic::AtomicBool,
    /// The single stdout writer's channel, for server-initiated
    /// notifications. Taken (dropped) at shutdown so the writer can finish.
    notify: std::sync::Mutex<Option<tokio::sync::mpsc::UnboundedSender<Value>>>,
}

/// What `GET /mcp/otto-server/enabled` answers, as the bridge keeps it.
#[derive(Clone, Debug, Default)]
struct EnabledInfo {
    /// Full `otto.*` names enabled in the control plane.
    names: Vec<String>,
    /// The calling session holds the per-session "Allow UI control" grant.
    ui_granted: bool,
    /// The session's resolved MCP profile filter (`None`: no profile asked).
    filter: Option<otto_server::mcp_profiles::ToolFilter>,
}

/// Wall-clock cap on one audit append (`POST /mcp/tool-calls`).
const AUDIT_POST_TIMEOUT: Duration = Duration::from_secs(5);
/// How long the control-plane enable list (+ UI grant) is reused.
const ENABLED_CACHE_TTL: Duration = Duration::from_secs(5);
/// How long the gateway's downstream tool list is reused.
const GATEWAY_CACHE_TTL: Duration = Duration::from_secs(30);
/// Concurrent requests the bridge runs at once. Claude/Codex issue parallel
/// `tools/call`s; one slow call (a 15 s approval wait, a 60 s workflow run)
/// must not queue every other call — and `ping` — behind it.
const MAX_CONCURRENT_CALLS: usize = 8;
/// The one stub advertised instead of the ~90 `otto_ui_*` tools while the
/// session lacks the UI grant: calling it asks the human (it runs the
/// read-only `otto.ui_state`, which raises the grant request).
const UI_REQUEST_CONTROL_TOOL: &str = "otto_ui_request_control";

impl Ctx {
    /// GET an `/api/v1` path with the bearer token, enforcing the call timeout
    /// and the body-size cap, returning parsed JSON. Read-only by construction —
    /// this is the ONLY upstream verb the tools use.
    async fn get_json(&self, path: &str) -> Result<Value, String> {
        self.get_json_within(path, CALL_TIMEOUT).await
    }

    /// [`Self::get_json`] with an explicit wall-clock budget — for reads whose
    /// route legitimately blocks longer than [`CALL_TIMEOUT`] (the
    /// `otto_wait_session` long-poll, see [`wait_session_budget`]).
    async fn get_json_within(&self, path: &str, budget: Duration) -> Result<Value, String> {
        let url = format!("{}/api/v1{}", self.base.trim_end_matches('/'), path);
        let resp = tokio::time::timeout(
            budget,
            self.http
                .get(&url)
                // Per-request: the client has no default timeout, so every
                // request states its own budget (body read included).
                .timeout(budget)
                .bearer_auth(&self.token)
                .header(
                    "X-Otto-Session",
                    self.session_id.clone().unwrap_or_default(),
                )
                .send(),
        )
        .await
        .map_err(|_| format!("upstream timeout after {}s", budget.as_secs()))?
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
                .timeout(CALL_TIMEOUT)
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
                .timeout(CALL_TIMEOUT)
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

    /// DELETE an `/api/v1` path with the bearer token. Used by
    /// `otto_vault_delete` — a soft delete (the daemon moves the note into the
    /// vault's `.trash/`, never destroying files) — and by `browser_navigate`
    /// to remove the tab IT just created when that tab's fetch failed. Same
    /// error handling as [`Self::post_json`]; tolerates an empty (204) body.
    async fn delete_ok(&self, path: &str) -> Result<(), String> {
        let url = format!("{}/api/v1{}", self.base.trim_end_matches('/'), path);
        let resp = tokio::time::timeout(
            CALL_TIMEOUT,
            self.http
                .delete(&url)
                .timeout(CALL_TIMEOUT)
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
                // The 60 s kubectl budget needs more than CALL_TIMEOUT.
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
    /// access) leaves the inward catalog unchanged. Cached for
    /// [`GATEWAY_CACHE_TTL`]; `fresh` bypasses the cache (a call naming a tool
    /// the cached list lacks).
    async fn gateway_tools(&self, fresh: bool) -> Vec<Value> {
        let mut cache = self.gateway_cache.lock().await;
        if !fresh {
            if let Some((at, tools)) = cache.as_ref() {
                if at.elapsed() < GATEWAY_CACHE_TTL {
                    return tools.clone();
                }
            }
        }
        let tools = self.fetch_gateway_tools().await;
        *cache = Some((Instant::now(), tools.clone()));
        tools
    }

    async fn fetch_gateway_tools(&self) -> Vec<Value> {
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

    /// The full `otto.*` names the operator has ENABLED in the control plane,
    /// plus this session's UI grant (`GET /mcp/otto-server/enabled`, MCP
    /// View), cached for [`ENABLED_CACHE_TTL`]; `fresh` skips the cache.
    /// `None` when the daemon can't answer — then no governed tool is
    /// advertised or callable, and the reason is on stderr, so "the control
    /// plane says X but the session can't see it" is always explainable from
    /// the daemon's log.
    async fn enabled_info(&self, fresh: bool) -> Option<EnabledInfo> {
        let mut cache = self.enabled_cache.lock().await;
        if !fresh {
            if let Some((at, info)) = cache.as_ref() {
                if at.elapsed() < ENABLED_CACHE_TTL {
                    return Some(info.clone());
                }
            }
        }
        match self.fetch_enabled_info().await {
            Ok(info) => {
                *cache = Some((Instant::now(), info.clone()));
                Some(info)
            }
            Err(e) => {
                eprintln!(
                    "ottod mcp-tools: control-plane tool list unavailable: {e} — governed \
                     otto.* tools (e.g. otto_create_pr) will NOT appear on this stdio surface"
                );
                None
            }
        }
    }

    /// Fold a grant flag learned from a UI tool's reply into the cached
    /// enabled info, so the next `tools/list` agrees with the re-list
    /// notification without a refetch. No cache yet ⇒ nothing to correct.
    async fn note_ui_grant(&self, granted: bool) {
        if let Some((_, info)) = self.enabled_cache.lock().await.as_mut() {
            info.ui_granted = granted;
        }
    }

    async fn fetch_enabled_info(&self) -> Result<EnabledInfo, String> {
        match self
            .get_json(&profile::enabled_path(self.profiles.as_deref()))
            .await
        {
            Ok(v) => Ok(EnabledInfo {
                names: v["enabled"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|n| n.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default(),
                ui_granted: v["ui_granted"].as_bool().unwrap_or(false),
                filter: profile::parse(&v),
            }),
            // A daemon older than this bridge has no light route: fall back to
            // the full status (no grant field there — UI tools stay hidden
            // behind the request stub, which still works).
            Err(e) if e.contains("404") => {
                let v = self.get_json("/mcp/otto-server").await?;
                Ok(EnabledInfo {
                    names: v["tools"]
                        .as_array()
                        .map(|tools| {
                            tools
                                .iter()
                                .filter(|t| t["enabled"].as_bool().unwrap_or(false))
                                .filter_map(|t| t["name"].as_str().map(str::to_string))
                                .collect()
                        })
                        .unwrap_or_default(),
                    ui_granted: false,
                    filter: None,
                })
            }
            Err(e) => Err(e),
        }
    }

    /// Send a server-initiated message (a notification) through the single
    /// stdout writer. A no-op before the loop starts or after shutdown.
    fn send_notification(&self, msg: Value) {
        if let Some(tx) = self
            .notify
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
        {
            let _ = tx.send(msg);
        }
    }

    /// When the session's UI grant no longer matches what the last
    /// `tools/list` advertised, tell the client to re-list
    /// (`notifications/tools/list_changed`). `info` is the latest answer.
    fn sync_ui_listing(&self, info: &EnabledInfo) {
        self.sync_ui_grant(info.ui_granted);
    }

    /// [`Self::sync_ui_listing`] from a bare grant flag (a UI tool's reply
    /// carries it — perf2/10-mcp R4).
    fn sync_ui_grant(&self, granted: bool) {
        use std::sync::atomic::Ordering;
        let was = self.advertised_ui.load(Ordering::Relaxed);
        if was != granted
            && self
                .advertised_ui
                .compare_exchange(was, granted, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
        {
            self.send_notification(
                json!({ "jsonrpc": "2.0", "method": "notifications/tools/list_changed" }),
            );
        }
    }

    /// Append a best-effort audit row for one tool call, through the daemon
    /// (`POST /mcp/tool-calls`, which stamps this session and its workspace —
    /// perf2/10-mcp R7: the bridge used to hold its own writer connection to
    /// the live DB, one per agent session). Failures are logged to stderr
    /// (never stdout — that's the protocol channel) and swallowed.
    async fn audit(&self, tool: &str, args: &Value, ok: bool, rows: Option<i64>) {
        if !self.audit {
            return;
        }
        // The arguments are redacted before they leave (defense-in-depth: a
        // caller could pass a secret-looking value); the daemon redacts again
        // and the repo caps long string values.
        let body = json!({
            "tool": tool,
            "arguments": redact_json(args).value,
            "ok": ok,
            "rows": rows,
        });
        let req = self
            .http
            .post(format!(
                "{}/api/v1/mcp/tool-calls",
                self.base.trim_end_matches('/')
            ))
            .timeout(AUDIT_POST_TIMEOUT)
            .bearer_auth(&self.token)
            .header(
                "X-Otto-Session",
                self.session_id.clone().unwrap_or_default(),
            )
            .json(&body);
        let mut tasks = self.audit_tasks.lock().unwrap_or_else(|p| p.into_inner());
        // Reap finished posts so the set stays small in a long session.
        while tasks.try_join_next().is_some() {}
        tasks.spawn(async move {
            match req.send().await {
                Ok(r) if r.status().is_success() => {}
                Ok(r) => eprintln!("ottod mcp-tools: audit append refused: {}", r.status()),
                Err(e) => eprintln!("ottod mcp-tools: audit append failed: {e}"),
            }
        });
    }

    /// Wait (bounded) for in-flight audit posts — called when stdin closes.
    async fn drain_audit(&self) {
        let mut tasks =
            std::mem::take(&mut *self.audit_tasks.lock().unwrap_or_else(|p| p.into_inner()));
        let _ = tokio::time::timeout(std::time::Duration::from_secs(6), async {
            while tasks.join_next().await.is_some() {}
        })
        .await;
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

// The native catalog, assistant tools, references and dispatch live in
// `mcp_tools/{catalog,assistant,native_refs,dispatch}.rs`.
mod assistant;
use assistant::*;
mod catalog;
use catalog::*;
mod dispatch;
use dispatch::*;
mod native_refs;
use native_refs::*;

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

/// Append `&key=<value>` for every `(key, arg)` whose argument is a non-empty
/// string or a number, percent-encoding string values. Optional query filters
/// for the AWS/K8s reads (`?region=`, `?prefix=`, `?ns=` …) all follow this
/// shape; booleans render `true`/`false`, objects as their JSON text.
fn opt_query(args: &Value, pairs: &[(&str, &str)]) -> String {
    let mut q = String::new();
    for (key, arg) in pairs {
        match args.get(arg) {
            Some(Value::String(s)) if !s.is_empty() => q.push_str(&format!("&{key}={}", seg(s))),
            Some(Value::Number(n)) => q.push_str(&format!("&{key}={n}")),
            Some(Value::Bool(b)) => q.push_str(&format!("&{key}={b}")),
            Some(v @ Value::Object(_)) => q.push_str(&format!("&{key}={}", seg(&v.to_string()))),
            _ => {}
        }
    }
    q
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
/// Which saved API-client object an [`api_object_id`] reference names: the
/// reference argument, the `overview?kind=` set, its array, and the label.
struct ApiRef {
    arg: &'static str,
    kind: &'static str,
    label: &'static str,
}

impl ApiRef {
    const REQUEST: ApiRef = ApiRef {
        arg: "request_id",
        kind: "requests",
        label: "request",
    };
    const AUTOMATION: ApiRef = ApiRef {
        arg: "automation_id",
        kind: "automations",
        label: "automation",
    };
}

/// A saved API-client object's id from the catalog's id-OR-name reference
/// (`request_id` / `automation_id`, architecture-1-05) or, when `legacy_name`,
/// the old native-only `name`. A name is matched in THIS session's workspace
/// (`{base}/overview?kind=…&q=`), as the native tools always did; a reference
/// that looks like an id, or that names nothing, is used as the id verbatim
/// (the route then answers 404 for an unknown one).
async fn api_object_id(
    ctx: &Ctx,
    base: &str,
    args: &Value,
    r: ApiRef,
    legacy_name: bool,
) -> Result<String, String> {
    let missing = || format!("pass `{}` (its id or unique name)", r.arg);
    let reference = match arg_optional_string(args, r.arg)?.filter(|s| !s.trim().is_empty()) {
        Some(id) if otto_server::agent_refs::looks_like_id(&id) => return Ok(id),
        Some(reference) => reference,
        None if legacy_name => arg_optional_string(args, "name")?
            .filter(|n| !n.is_empty())
            .ok_or_else(missing)?,
        None => return Err(missing()),
    };
    let overview = ctx
        .get_json(&format!(
            "{base}/overview?kind={}&q={}",
            r.kind,
            seg(&reference)
        ))
        .await?;
    let items = overview
        .get(r.kind)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("daemon response missing {} array", r.kind))?;
    match pick_by_name(items, &reference, r.label) {
        Ok(item) => item
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| format!("matched {} has no id", r.label)),
        Err(e) if e == format!("no {} named '{reference}'", r.label) => Ok(reference),
        Err(e) => Err(e),
    }
}

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
    // The catalog's `extras` replaces the stored extras (as the governed
    // executor does); the legacy native `docs_md` merges into them.
    let passed_extras = args.get("extras").filter(|e| e.is_object());
    if let Some(extras) = passed_extras {
        merged.insert("extras".to_string(), extras.clone());
    }
    if let Some(docs_md) = args.get("docs_md") {
        let mut extras = passed_extras
            .or_else(|| stored.and_then(|s| s.get("extras")))
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        extras.entry("v".to_string()).or_insert_with(|| json!(1));
        extras.insert("docs_md".to_string(), docs_md.clone());
        merged.insert("extras".to_string(), Value::Object(extras));
    }
    Value::Object(merged)
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

/// The PR number of `otto_get_pr` / `otto_comment_pr`: `number` (the outward
/// catalog's name) or its alias `pr_number`, the same pair the governed
/// `otto.get_pr` / `otto.comment_pr` accept.
fn arg_pr_number(args: &Value) -> Result<u64, String> {
    if args.get("number").is_some() {
        arg_u64(args, "number")
    } else if args.get("pr_number").is_some() {
        arg_u64(args, "pr_number")
    } else {
        Err("missing required integer argument `number` (or `pr_number`)".into())
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

// Path segments are encoded by the ONE shared `seg` (otto-mcp): an id with
// `/`, spaces or a whole-segment `..` can't break out of the intended route.
use otto_server::mcp_outward::seg;

/// Proxy a namespaced gateway tool (`mcp__<server>__<tool>`) through the control
/// plane's governed `/mcp/gateway/invoke`. The pipeline there does allowlist /
/// policy / approval / dry-run / execute / **audit**, so this path does NOT write
/// the first-party `mcp_tool_calls` row (the call is already on `mcp_call_log`).
/// Returns the governed envelope and whether it was an error/denial.
async fn gateway_call(ctx: &Ctx, namespaced: &str, args: &Value) -> Result<(Value, bool), String> {
    let Some(ws) = &ctx.workspace_id else {
        return Err("gateway: no workspace context".into());
    };
    let find = |tools: &[Value]| {
        tools
            .iter()
            .find(|t| t.get("name").and_then(Value::as_str) == Some(namespaced))
            .cloned()
    };
    // The cached list first; a miss refetches once (a server/tool added or
    // re-enabled since). The daemon re-authorizes the call either way.
    let entry = match find(&ctx.gateway_tools(false).await) {
        Some(e) => e,
        None => find(&ctx.gateway_tools(true).await)
            .ok_or_else(|| format!("unknown gateway tool `{namespaced}`"))?,
    };
    let server_id = entry.get("server_id").and_then(Value::as_str).unwrap_or("");
    let tool = entry.get("tool").and_then(Value::as_str).unwrap_or("");
    let body = json!({
        "server_id": server_id,
        "tool": tool,
        "arguments": args,
        "workspace_id": ws,
        "session_id": ctx.session_id,
    });
    // The daemon may queue, cold-open (spawn + initialize) and then run the
    // downstream call, each up to otto-mcp's `OP_TIMEOUT`: the default 20 s
    // gave up first, dropping the handler mid-call (integrations-4-01).
    let v = ctx
        .post_json_within(
            "/mcp/gateway/invoke",
            &body,
            otto_mcp::client::GATEWAY_CALL_TIMEOUT,
        )
        .await?;
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
    let info = ctx.enabled_info(false).await.ok_or_else(|| {
        format!("`{name}` is unavailable: the Otto MCP server tool list could not be read from the daemon")
    })?;
    let enabled = &info.names;
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
            Duration::from_secs(GOVERNED_WAIT_SECS) + otto_core::mcp_budgets::GOVERNED_RUN_BUDGET,
        )
        .await?;
    let is_error = matches!(
        v.get("decision").and_then(Value::as_str),
        Some("denied") | Some("error")
    ) || v.get("is_error").and_then(Value::as_bool).unwrap_or(false);
    // A UI tool called by name may just have obtained (or lost) the grant:
    // re-list if the advertised catalog is now wrong. The daemon puts the
    // post-call grant in the reply (`ui_granted`, R4) — no second request;
    // only a daemon older than that field costs a fresh enabled read.
    if is_ui_spec_name(&full) {
        match v.get("ui_granted").and_then(Value::as_bool) {
            Some(granted) => {
                ctx.note_ui_grant(granted).await;
                ctx.sync_ui_grant(granted);
            }
            None => {
                if let Some(fresh) = ctx.enabled_info(true).await {
                    ctx.sync_ui_listing(&fresh);
                }
            }
        }
    }
    Ok((v, is_error))
}

/// Run one of this surface's NATIVE irreversible writers
/// (`otto_mcp::outward::NATIVE_SESSION_WRITERS`: `k8s_action`,
/// `k8s_pod_http`, `aws_sqs_send`, `api_execute`, `api_run_automation`)
/// through the governed choke point instead of straight at its daemon route:
/// the same approval gate, IRREVERSIBLE guardrail (an auto-approve rule needs
/// `allow_irreversible`) and audit as `otto.<tool>` — the agent filling in
/// `confirm_name` itself is no human approval. `tool` is the governed short
/// name; `args` its governed arguments. Executed → the tool's own result;
/// pending → the envelope (approval id, "resubmit after it is approved");
/// denied / failed → an error.
async fn governed_native_write(
    ctx: &Ctx,
    tool: &str,
    args: &Value,
    run_budget: Duration,
) -> Result<Value, String> {
    let body = json!({
        "tool": format!("otto.{tool}"),
        "arguments": args,
        "wait_seconds": GOVERNED_WAIT_SECS,
    });
    let v = ctx
        .post_json_within(
            "/mcp/otto-tools/invoke",
            &body,
            Duration::from_secs(GOVERNED_WAIT_SECS) + run_budget,
        )
        .await?;
    governed_native_result(tool, v)
}

/// The native result of a governed envelope (see [`governed_native_write`]).
/// Pure — unit-tested.
fn governed_native_result(tool: &str, v: Value) -> Result<Value, String> {
    let executed = v.get("executed").and_then(Value::as_bool) == Some(true);
    let is_error = v.get("is_error").and_then(Value::as_bool) == Some(true);
    match v.get("decision").and_then(Value::as_str) {
        _ if executed && !is_error => Ok(v.get("content").cloned().unwrap_or(Value::Null)),
        Some("pending_approval") | Some("dry_run") => Ok(v),
        Some("denied") => Err(format!(
            "{tool} denied: {}",
            v.get("reason")
                .and_then(Value::as_str)
                .unwrap_or("refused by policy")
        )),
        _ => Err(v
            .pointer("/content/error")
            .and_then(Value::as_str)
            .map_or_else(|| format!("{tool} failed: {v}"), str::to_string)),
    }
}

/// [`UI_REQUEST_CONTROL_TOOL`]: run the read-only `otto.ui_state` through the
/// governed path — the UI bridge raises the owner's "Allow UI control" prompt
/// and waits briefly for the decision — then re-read the grant and, when it
/// now holds, emit `notifications/tools/list_changed` so the client lists the
/// `otto_ui_*` tools. The envelope tells the agent what happened.
async fn ui_request_control(ctx: &Ctx) -> Result<(Value, bool), String> {
    let (v, is_error) = governed_call(ctx, "otto_ui_state", &json!({})).await?;
    // `governed_call` already synced the listing from the reply's grant (or,
    // on an older daemon, from a fresh read that refreshed the cache) — no
    // second request here (R4).
    let granted = match v.get("ui_granted").and_then(Value::as_bool) {
        Some(g) => g,
        None => ctx
            .enabled_info(false)
            .await
            .is_some_and(|info| info.ui_granted),
    };
    let note = if granted {
        "UI control is allowed for this session: the otto_ui_* tools are now in your tool list \
         (re-list tools if your client did not refresh)."
    } else {
        "UI control is not allowed (yet): the person has to click Allow in Otto. Ask again later \
         only if the task still needs the Otto UI."
    };
    Ok((
        json!({ "ui_control_granted": granted, "note": note, "result": v }),
        is_error && !granted,
    ))
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
    // Notifications carry no `id` and MUST NOT be answered — nor executed:
    // a notification-form `tools/call` would run a tool (possibly a writer)
    // whose result nobody receives.
    let id = msg.get("id").cloned();
    id.as_ref()?;

    match method {
        "initialize" => {
            let result = json!({
                "protocolVersion": PROTOCOL_VERSION,
                // `listChanged`: the bridge emits `notifications/tools/list_changed`
                // when the session's UI grant flips (the otto_ui_* tools appear
                // or disappear).
                "capabilities": { "tools": { "listChanged": true } },
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
            // The gateway list and the enable list are independent daemon
            // reads: fetch them concurrently (each is cached briefly).
            let (gw, enabled): (_, Option<EnabledInfo>) = if reviewer {
                (Vec::new(), None)
            } else {
                tokio::join!(ctx.gateway_tools(false), ctx.enabled_info(false))
            };
            // …plus the control-plane-enabled otto.* tools this surface doesn't
            // serve natively (what the operator ticked under MCP → Otto server),
            // UI-control tools only once this session holds the UI grant.
            let filter = enabled.as_ref().and_then(|i| i.filter.clone());
            let governed = match enabled {
                Some(info) => {
                    ctx.advertised_ui
                        .store(info.ui_granted, std::sync::atomic::Ordering::Relaxed);
                    governed_tools_for_session(&info)
                }
                None => Vec::new(),
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
                scope_listing_for_source(arr, ctx.source.as_deref());
                if let Some(f) = &filter {
                    f.retain_listed(arr);
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
            if let Some(refused) = profile::refuse(ctx, &name, &args).await {
                return Some(rpc_ok(id, refused));
            }
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
            // The UI-control request stub: ask for the grant through the
            // read-only `otto.ui_state` (which raises the human's Allow prompt
            // and waits briefly), then re-list if the grant now holds.
            if name == UI_REQUEST_CONTROL_TOOL {
                return Some(match ui_request_control(ctx).await {
                    Ok((v, is_error)) => rpc_ok(id, tool_result(&v, is_error)),
                    Err(e) => rpc_ok(id, tool_result(&json!({ "error": e }), true)),
                });
            }
            // A control-plane otto.* tool with no native twin — governed proxy
            // (enabled-check, approval, audit all happen daemon-side).
            if governed_spec_ref(&name).is_some() {
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
        _ => Some(rpc_err(
            id.unwrap_or(Value::Null),
            -32601,
            format!("method not found: {method}"),
        )),
    }
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
    let profiles = creds.profiles;

    // Audit rows go through the daemon (`POST /mcp/tool-calls`): the bridge
    // opens no database connection at all (R7; before, every agent session's
    // bridge held a writer on the live DB just for these inserts).
    let audit = true;

    // No client-wide timeout: each request sets its own (`.timeout(budget)`),
    // so a long-poll or an agent-turn route is not cut off at CALL_TIMEOUT
    // underneath its per-route budget (integrations-3-01).
    let http = reqwest::Client::builder()
        .build()
        .map_err(|e| format!("build http client: {e}"))?;

    let mut ctx = Ctx::new(http, base, token, session_id, workspace_id, source, audit);
    ctx.profiles = profiles;
    let ctx = Arc::new(ctx);
    serve(ctx, BufReader::new(tokio::io::stdin()), tokio::io::stdout()).await
}

impl Ctx {
    fn new(
        http: reqwest::Client,
        base: String,
        token: String,
        session_id: Option<String>,
        workspace_id: Option<String>,
        source: Option<String>,
        audit: bool,
    ) -> Self {
        Ctx {
            http,
            base,
            token,
            session_id,
            workspace_id,
            source,
            profiles: None,
            audit,
            audit_tasks: Default::default(),
            enabled_cache: Default::default(),
            gateway_cache: Default::default(),
            advertised_ui: Default::default(),
            notify: Default::default(),
        }
    }
}

/// Whether a request is answered inline by the read loop (no daemon I/O, or
/// it must complete before anything else runs) instead of on a task.
fn answered_inline(msg: &Value) -> bool {
    matches!(
        msg.get("method").and_then(Value::as_str),
        Some("initialize" | "ping" | "notifications/initialized" | "initialized")
    )
}

/// The longest request line the bridge buffers (8 MiB). A huge or newline-
/// free input used to grow `read_line`'s buffer without limit; an over-long
/// line is discarded up to its newline and answered with -32700.
const MAX_LINE_BYTES: usize = 8 * 1024 * 1024;

/// What [`read_bounded_line`] read.
#[derive(Debug, PartialEq)]
enum BoundedLine {
    /// A whole line (newline included) is in the buffer.
    Line,
    /// The line was over the cap — consumed and dropped; the buffer is empty.
    TooLong,
    /// End of input before any byte.
    Eof,
}

/// `read_line` with a cap: buffers at most `max` bytes of one line, and
/// past that keeps CONSUMING (without storing) up to the newline, so memory
/// stays bounded whatever the peer sends.
async fn read_bounded_line<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    buf: &mut Vec<u8>,
    max: usize,
) -> std::io::Result<BoundedLine> {
    buf.clear();
    let (mut any, mut too_long) = (false, false);
    loop {
        let avail = reader.fill_buf().await?;
        if avail.is_empty() {
            return Ok(match (any, too_long) {
                (false, _) => BoundedLine::Eof,
                (true, true) => BoundedLine::TooLong,
                (true, false) => BoundedLine::Line,
            });
        }
        any = true;
        let (used, done) = match avail.iter().position(|&b| b == b'\n') {
            Some(i) => (i + 1, true),
            None => (avail.len(), false),
        };
        if !too_long {
            if buf.len() + used > max {
                too_long = true;
                buf.clear();
                buf.shrink_to_fit();
            } else {
                buf.extend_from_slice(&avail[..used]);
            }
        }
        reader.consume(used);
        if done {
            return Ok(if too_long {
                BoundedLine::TooLong
            } else {
                BoundedLine::Line
            });
        }
    }
}

/// The JSON-RPC loop: read newline-delimited requests from `reader`, answer
/// them on `writer`, until EOF.
///
/// Requests run CONCURRENTLY (up to [`MAX_CONCURRENT_CALLS`]) — Claude and
/// Codex send parallel `tools/call`s and match replies by JSON-RPC id, so a
/// slow call no longer queues every other call (and `ping`) behind it. ONE
/// writer task owns `writer`: each reply is a whole line, never interleaved.
/// `initialize` / `ping` / the `initialized` notification are answered inline.
/// At EOF in-flight calls get a bounded drain (6 s), then the audit drain.
async fn serve<R, W>(ctx: Arc<Ctx>, mut reader: R, writer: W) -> Result<(), String>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Value>();
    *ctx.notify.lock().unwrap_or_else(|p| p.into_inner()) = Some(tx.clone());
    let writer_task = tokio::spawn(async move {
        let mut writer = writer;
        while let Some(msg) = rx.recv().await {
            if let Err(e) = write_line(&mut writer, &msg).await {
                eprintln!("ottod mcp-tools: {e}");
                break;
            }
        }
    });
    let permits = Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_CALLS));
    let mut inflight = tokio::task::JoinSet::new();
    let mut buf = Vec::new();
    let result = loop {
        match read_bounded_line(&mut reader, &mut buf, MAX_LINE_BYTES).await {
            Ok(BoundedLine::Line) => {}
            Ok(BoundedLine::Eof) => break Ok(()), // the client closed the pipe.
            Ok(BoundedLine::TooLong) => {
                let _ = tx.send(rpc_err(
                    Value::Null,
                    -32700,
                    format!("parse error: request line exceeds {MAX_LINE_BYTES} bytes"),
                ));
                continue;
            }
            Err(e) => break Err(format!("read stdin: {e}")),
        }
        while inflight.try_join_next().is_some() {}
        let Ok(line) = std::str::from_utf8(&buf) else {
            let _ = tx.send(rpc_err(Value::Null, -32700, "parse error: invalid UTF-8"));
            continue;
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                // Malformed line: emit a parse-error response with null id.
                let _ = tx.send(rpc_err(Value::Null, -32700, format!("parse error: {e}")));
                continue;
            }
        };
        if answered_inline(&msg) {
            if let Some(resp) = handle(&ctx, msg).await {
                let _ = tx.send(resp);
            }
            continue;
        }
        // Backpressure: with MAX_CONCURRENT_CALLS in flight, stop reading
        // until one finishes.
        let Ok(permit) = permits.clone().acquire_owned().await else {
            break Ok(());
        };
        let (ctx, tx) = (ctx.clone(), tx.clone());
        inflight.spawn(async move {
            if let Some(resp) = handle(&ctx, msg).await {
                let _ = tx.send(resp);
            }
            drop(permit);
        });
    };
    let _ = tokio::time::timeout(Duration::from_secs(6), async {
        while inflight.join_next().await.is_some() {}
    })
    .await;
    // Close the writer's channel (the loop's sender, the tasks' clones are
    // gone, and the notification sender) so it flushes and ends.
    drop(tx);
    ctx.notify.lock().unwrap_or_else(|p| p.into_inner()).take();
    let _ = tokio::time::timeout(Duration::from_secs(2), writer_task).await;
    ctx.drain_audit().await;
    result
}

/// Serialize one JSON-RPC message and write it as a single newline-terminated
/// line, flushing so the client sees it immediately.
async fn write_line<W: AsyncWrite + Unpin>(stdout: &mut W, value: &Value) -> Result<(), String> {
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

    // Catalog-shape tests (twin schema parity, byte budget): `tests/catalog_tests.rs`.
    mod catalog_tests;
    // Per-route upstream budgets (the wait_session long-poll): `tests/budget_tests.rs`.
    mod budget_tests;
    // MCP profiles: group coverage + list/dispatch filtering: `tests/profile_tests.rs`.
    mod profile_tests;
    // architecture-1-05 characterization: tools/list + twin dispatch snapshots.
    mod twin_tests;
    // architecture-1-05 rules: listing parity, routed-twin pre-step: `tests/routing_tests.rs`.
    mod routing_tests;

    /// S5-20: an over-long line is dropped (to its newline) and the next one
    /// still reads; memory never holds more than the cap.
    #[tokio::test]
    async fn bounded_lines_drop_an_oversized_line_and_continue() {
        let input = format!("{}\n{{\"ok\":1}}\ntail", "x".repeat(100));
        let mut r = BufReader::with_capacity(16, input.as_bytes());
        let mut buf = Vec::new();
        assert_eq!(
            read_bounded_line(&mut r, &mut buf, 32).await.unwrap(),
            BoundedLine::TooLong
        );
        assert!(buf.capacity() <= 32);
        assert_eq!(
            read_bounded_line(&mut r, &mut buf, 32).await.unwrap(),
            BoundedLine::Line
        );
        assert_eq!(buf, b"{\"ok\":1}\n");
        assert_eq!(
            read_bounded_line(&mut r, &mut buf, 32).await.unwrap(),
            BoundedLine::Line
        );
        assert_eq!(buf, b"tail");
        assert_eq!(
            read_bounded_line(&mut r, &mut buf, 32).await.unwrap(),
            BoundedLine::Eof
        );
    }

    /// S5-07: a native irreversible writer's governed envelope maps back to
    /// the native result — executed → the tool's content; pending → the
    /// envelope (no error, the agent resubmits); denied / failed → an error.
    #[test]
    fn governed_native_results_map_back() {
        let ok = governed_native_result(
            "k8s_action",
            json!({"decision":"allowed","executed":true,"content":{"ok":true}}),
        );
        assert_eq!(ok.unwrap(), json!({"ok":true}));
        let pending = json!({"decision":"pending_approval","executed":false,"approval_id":"a1"});
        assert_eq!(
            governed_native_result("k8s_action", pending.clone()).unwrap(),
            pending
        );
        let denied = governed_native_result(
            "aws_sqs_send",
            json!({"decision":"denied","executed":false,"reason":"human denied the request"}),
        )
        .unwrap_err();
        assert!(denied.contains("human denied"), "{denied}");
        let failed = governed_native_result(
            "api_execute",
            json!({"decision":"error","executed":true,"is_error":true,"content":{"error":"404"}}),
        )
        .unwrap_err();
        assert_eq!(failed, "404");
    }

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

    /// `otto_room_read` opens on the room's tail and pages both ways (the
    /// outward `room_read` route), with the CALLING session bound.
    #[test]
    fn room_read_opens_on_the_tail_and_pages_both_ways() {
        let path = |args: Value, session: Option<&str>| {
            twin_route("otto_room_read", &args, None, session)
                .expect("routed")
                .expect("valid")
                .path
        };
        let room = "/agent-rooms/r1/messages?";
        // No cursor: the newest messages, with a bounded default page.
        assert_eq!(
            path(json!({"room_id":"r1"}), None),
            format!("{room}tail=true&limit=50")
        );
        assert_eq!(
            path(json!({"room_id":"r1","limit":10}), None),
            format!("{room}tail=true&limit=10")
        );
        // `after` pages forward (and wins over `before`); `before` pages back.
        assert_eq!(
            path(json!({"room_id":"r1","after":"m9","before":"m1"}), None),
            format!("{room}after=m9&limit=50")
        );
        assert_eq!(
            path(json!({"room_id":"r1","before":"m1"}), None),
            format!("{room}before=m1&limit=50")
        );
        // Blank cursors are ignored, not sent as `after=`.
        assert_eq!(
            path(json!({"room_id":"r1","after":"  "}), None),
            format!("{room}tail=true&limit=50")
        );
        // The calling session is bound; an agent-supplied one never is.
        assert_eq!(
            path(json!({"room_id":"r1","session_id":"other"}), Some("s1")),
            format!("{room}tail=true&limit=50&session_id=s1")
        );
        assert_eq!(
            path(json!({"room_id":"r1","session_id":"other"}), None),
            format!("{room}tail=true&limit=50")
        );
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

    /// S5-10: a notification-form `tools/call` (no `id`) is neither executed
    /// nor answered — not even the argument check runs.
    #[tokio::test]
    async fn a_notification_tools_call_is_not_executed_or_answered() {
        let ctx = test_ctx();
        let resp = handle(
            &ctx,
            json!({ "jsonrpc": "2.0", "method": "tools/call",
                    "params": { "name": "otto_api_execute", "arguments": {} } }),
        )
        .await;
        assert!(resp.is_none(), "no reply to a notification: {resp:?}");
        let resp = handle(&ctx, json!({ "jsonrpc": "2.0", "method": "ping" })).await;
        assert!(resp.is_none());
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
    fn catalog_lists_the_workbench_tools() {
        let cat = tool_catalog();
        let tools = cat["tools"].as_array().unwrap();
        for t in ["workbench_list", "workbench_get", "workbench_write"] {
            let spec = tools
                .iter()
                .find(|x| x["name"] == t)
                .unwrap_or_else(|| panic!("catalog missing {t}"));
            let d = spec["description"].as_str().unwrap();
            if t == "workbench_write" {
                assert!(d.starts_with("MUTATING"), "{t} must flag MUTATING");
            } else {
                assert!(d.starts_with("Read-only"), "{t} must be read-only");
            }
        }
    }

    #[test]
    fn workbench_history_catalog_exposes_bounded_paging() {
        let catalog = tool_catalog();
        let spec = catalog["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == "workbench_get")
            .unwrap();
        let props = &spec["inputSchema"]["properties"];
        assert_eq!(props["history_limit"]["type"], "integer");
        assert_eq!(props["history_limit"]["default"], 100);
        assert_eq!(props["history_limit"]["maximum"], 200);
        assert_eq!(props["before_seq"]["type"], "integer");
        assert_eq!(props["revision"]["type"], "integer");
    }

    #[tokio::test]
    async fn workbench_history_handler_forwards_cursor_and_preserves_direct_revision() {
        use axum::extract::OriginalUri;
        let requests = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let captured = requests.clone();
        // A fallback avoids adding fictitious routes to the route-inventory scanner.
        let app = axum::Router::new().fallback(move |OriginalUri(uri): OriginalUri| {
            let captured = captured.clone();
            async move {
                captured.lock().unwrap().push(uri.to_string());
                if uri.path().ends_with("/revisions") {
                    let query: std::collections::HashMap<String, String> =
                        reqwest::Url::parse(&format!("http://fixture{uri}"))
                            .unwrap()
                            .query_pairs()
                            .into_owned()
                            .collect();
                    let before = query
                        .get("before_seq")
                        .and_then(|s| s.parse::<i64>().ok())
                        .unwrap_or(1000);
                    let rows = if before == 2 {
                        json!([{ "seq": 1 }])
                    } else {
                        json!([{ "seq": before - 1 }, { "seq": before - 2 }])
                    };
                    axum::Json(rows)
                } else {
                    axum::Json(json!({"id": "doc", "seq": 1, "content": "oldest"}))
                }
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut ctx = test_ctx();
        ctx.base = format!("http://{addr}");
        let result = run_tool(
            &ctx,
            "workbench_get",
            &json!({
                "doc_id": "doc", "revision": 1, "history": true,
                "history_limit": 2, "before_seq": 500
            }),
        )
        .await;
        let short = run_tool(
            &ctx,
            "workbench_get",
            &json!({
                "doc_id": "doc", "history": true, "history_limit": 2, "before_seq": 2
            }),
        )
        .await;
        server.abort();
        let (result, _) = result.unwrap();
        let (short, _) = short.unwrap();
        let requests = requests.lock().unwrap();
        assert!(
            requests[0].ends_with("/revisions/1"),
            "direct lookup must stay addressable"
        );
        let url = reqwest::Url::parse(&format!("http://fixture{}", requests[1])).unwrap();
        let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(query.get("limit").map(String::as_str), Some("2"));
        assert_eq!(query.get("before_seq").map(String::as_str), Some("500"));
        assert_eq!(result["content"], "oldest");
        assert_eq!(result["revisions"].as_array().unwrap().len(), 2);
        assert_eq!(result["next_before_seq"], 498);
        assert_eq!(short["revisions"], json!([{ "seq": 1 }]));
        assert!(short["next_before_seq"].is_null());
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
                body: None,
                budget: CALL_TIMEOUT,
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
        // Routed twins (`route_for`, `/api/v1` stripped once): same URLs.
        assert_eq!(
            read_route(
                "otto_list_prs",
                &json!({"repo_id":"a b","state":"open","per_page":5}),
                ws
            )
            .unwrap(),
            ReadCall {
                post: false,
                path: "/repos/a%20b/prs?state=open&per_page=5".into(),
                body: None,
                budget: CALL_TIMEOUT,
            }
        );
        assert_eq!(
            read_route("otto_list_prs", &json!({"repo_id":"r1"}), ws)
                .unwrap()
                .path,
            "/repos/r1/prs"
        );
        assert!(read_route("otto_list_prs", &json!({}), ws)
            .unwrap_err()
            .contains("repo_id"));
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
            "aws_logs_list_groups",
            "aws_logs_filter",
            "aws_logs_insights",
            "aws_logs_get_insights",
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
        for w in ["aws_athena_query", "aws_sqs_send", "aws_sqs_peek"] {
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
            "k8s_pod_http",
            "k8s_pod_actions_list",
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
        // Peek POSTs with visibility_timeout pinned to 0 and
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
            read_route(
                "aws_logs_filter",
                &json!({"account_id":"a1","group":"/aws/eks/prod/cluster","pattern":"ERROR","start":1000}),
                None
            )
            .unwrap()
            .path,
            "/aws/accounts/a1/logs/events?group=%2Faws%2Feks%2Fprod%2Fcluster&pattern=ERROR&start=1000"
        );
        let insights = read_route(
            "aws_logs_insights",
            &json!({"account_id":"a1","groups":["/g"],"query":"fields @message","start":1,"end":2,"region":"eu-west-1"}),
            None,
        )
        .unwrap();
        assert!(insights.post);
        assert_eq!(
            insights.path,
            "/aws/accounts/a1/logs/insights?region=eu-west-1"
        );
        assert_eq!(insights.body.unwrap()["groups"], json!(["/g"]));
        assert_eq!(
            read_route(
                "aws_logs_get_insights",
                &json!({"account_id":"a1","query_id":"q-1"}),
                None
            )
            .unwrap()
            .path,
            "/aws/accounts/a1/logs/insights/q-1?"
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
            assert!(
                assistant.contains(&tool.to_string()),
                "assistant catalog missing {tool}"
            );
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
            assert!(
                governed_tool_for_stdio_name(&format!("otto_{n}")).is_none(),
                "{n}"
            );
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

    // --- Performance guards (perf/10-mcp F1/F2/F3/F9) -------------------------
    //
    // A mock daemon on an ephemeral port answers the three routes the bridge
    // calls for governed tools, counting hits, so the tests can assert both
    // behaviour (concurrency, grant-aware listing, list_changed) and cost
    // (one enable-list read per cache window, catalog bytes).

    #[derive(Default)]
    struct MockDaemon {
        enabled_hits: std::sync::atomic::AtomicUsize,
        status_hits: std::sync::atomic::AtomicUsize,
        gateway_hits: std::sync::atomic::AtomicUsize,
        ui_granted: std::sync::atomic::AtomicBool,
        /// Answer UI tools like a daemon older than R4 (no `ui_granted`).
        legacy_reply: std::sync::atomic::AtomicBool,
        /// `POST /mcp/tool-calls` bodies (the bridge's audit appends).
        audits: std::sync::Mutex<Vec<Value>>,
        /// Full names the mock reports enabled.
        enabled: std::sync::Mutex<Vec<String>>,
    }

    /// Every governed spec enabled — the default-ish worst case for size.
    fn all_governed_names() -> Vec<String> {
        GOVERNED_SPECS
            .iter()
            .filter_map(|s| s["name"].as_str().map(str::to_string))
            .collect()
    }

    async fn mock_daemon(state: Arc<MockDaemon>) -> String {
        use axum::routing::{get, post};
        use std::sync::atomic::Ordering;
        let app = axum::Router::new()
            .route(
                "/mcp/otto-server/enabled",
                get(
                    |axum::extract::State(m): axum::extract::State<Arc<MockDaemon>>| async move {
                        m.enabled_hits.fetch_add(1, Ordering::SeqCst);
                        axum::Json(json!({
                            "enabled": *m.enabled.lock().unwrap(),
                            "outward_enabled": false,
                            "ui_granted": m.ui_granted.load(Ordering::SeqCst),
                        }))
                    },
                ),
            )
            .route(
                "/mcp/otto-server",
                get(
                    |axum::extract::State(m): axum::extract::State<Arc<MockDaemon>>| async move {
                        m.status_hits.fetch_add(1, Ordering::SeqCst);
                        axum::Json(json!({ "tools": [] }))
                    },
                ),
            )
            .route(
                "/mcp/gateway/tools",
                get(
                    |axum::extract::State(m): axum::extract::State<Arc<MockDaemon>>| async move {
                        m.gateway_hits.fetch_add(1, Ordering::SeqCst);
                        axum::Json(json!({ "tools": [] }))
                    },
                ),
            )
            .route(
                "/mcp/otto-tools/invoke",
                post(
                    |axum::extract::State(m): axum::extract::State<Arc<MockDaemon>>,
                     axum::Json(body): axum::Json<Value>| async move {
                        let tool = body["tool"].as_str().unwrap_or_default().to_string();
                        // The "slow" tool: an approval wait / long workflow.
                        if tool == "otto.create_pr" {
                            tokio::time::sleep(Duration::from_millis(800)).await;
                        }
                        // Asking for UI control: the human clicks Allow.
                        if tool == "otto.ui_state" {
                            m.ui_granted.store(true, Ordering::SeqCst);
                        }
                        let mut env = json!({
                            "decision": "allowed", "executed": true,
                            "content": { "tool": tool }
                        });
                        // R4: a UI tool's reply carries the post-call grant.
                        if tool.starts_with("otto.ui_") && !m.legacy_reply.load(Ordering::SeqCst) {
                            env["ui_granted"] = json!(m.ui_granted.load(Ordering::SeqCst));
                        }
                        axum::Json(env)
                    },
                ),
            )
            .route(
                "/mcp/tool-calls",
                post(
                    |axum::extract::State(m): axum::extract::State<Arc<MockDaemon>>,
                     axum::Json(body): axum::Json<Value>| async move {
                        m.audits.lock().unwrap().push(body);
                        axum::http::StatusCode::NO_CONTENT
                    },
                ),
            )
            .with_state(state);
        // Nested so the route literals above carry the same paths the
        // daemon registers (the route-inventory / policy-coverage scanners
        // read every `.route` literal in the workspace).
        let app = axum::Router::new().nest("/api/v1", app);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{addr}")
    }

    fn ctx_at(base: &str) -> Ctx {
        let mut ctx = test_ctx();
        ctx.base = base.to_string();
        // No workspace: the gateway list is skipped without a daemon call.
        ctx.workspace_id = None;
        ctx
    }

    /// Drive `serve` over in-memory pipes: write `requests` (one JSON per
    /// line), collect every line written back until `expect` messages arrived.
    async fn drive(ctx: Ctx, requests: &[Value], expect: usize) -> Vec<Value> {
        use tokio::io::AsyncWriteExt as _;
        let (mut client_w, server_r) = tokio::io::duplex(1 << 20);
        let (server_w, client_r) = tokio::io::duplex(1 << 22);
        let server = tokio::spawn(serve(Arc::new(ctx), BufReader::new(server_r), server_w));
        for r in requests {
            let mut line = serde_json::to_vec(r).unwrap();
            line.push(b'\n');
            client_w.write_all(&line).await.unwrap();
        }
        let mut lines = BufReader::new(client_r).lines();
        let mut out = Vec::new();
        while out.len() < expect {
            let line = tokio::time::timeout(Duration::from_secs(10), lines.next_line())
                .await
                .expect("bridge reply within 10 s")
                .unwrap()
                .expect("bridge closed early");
            out.push(serde_json::from_str::<Value>(&line).unwrap());
        }
        drop(client_w); // EOF → the loop drains and returns.
        tokio::time::timeout(Duration::from_secs(10), server)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        out
    }

    fn call(id: i64, name: &str) -> Value {
        json!({"jsonrpc":"2.0","id":id,"method":"tools/call",
               "params":{"name":name,"arguments":{}}})
    }

    #[tokio::test]
    async fn a_slow_call_no_longer_blocks_the_next_one_or_ping() {
        let mock = Arc::new(MockDaemon::default());
        *mock.enabled.lock().unwrap() = all_governed_names();
        let base = mock_daemon(mock.clone()).await;
        // #1 is slow (800 ms upstream); #2 (a fast governed read) and the
        // ping are sent right behind it and must be answered FIRST.
        let replies = drive(
            ctx_at(&base),
            &[
                call(1, "otto_create_pr"),
                call(2, "otto_list_workflows"),
                json!({"jsonrpc":"2.0","id":3,"method":"ping"}),
            ],
            3,
        )
        .await;
        let order: Vec<i64> = replies.iter().map(|r| r["id"].as_i64().unwrap()).collect();
        assert_eq!(
            order.last(),
            Some(&1),
            "the slow call finishes last: {order:?}"
        );
        assert!(order.contains(&2) && order.contains(&3), "{order:?}");
        // Every reply is a whole, well-formed JSON-RPC line (one writer).
        for r in &replies {
            assert_eq!(r["jsonrpc"], json!("2.0"));
        }
        // Two governed calls inside one cache window: ONE enable-list read,
        // and never the full status.
        assert_eq!(
            mock.enabled_hits.load(std::sync::atomic::Ordering::SeqCst),
            1
        );
        assert_eq!(
            mock.status_hits.load(std::sync::atomic::Ordering::SeqCst),
            0
        );
    }

    #[tokio::test]
    async fn concurrency_is_capped() {
        // More slow calls than permits: all still answered, and the cap held
        // (the semaphore never hands out more than MAX_CONCURRENT_CALLS).
        let mock = Arc::new(MockDaemon::default());
        *mock.enabled.lock().unwrap() = all_governed_names();
        let base = mock_daemon(mock.clone()).await;
        let n = MAX_CONCURRENT_CALLS + 2;
        let reqs: Vec<Value> = (0..n as i64).map(|i| call(i, "otto_create_pr")).collect();
        let started = Instant::now();
        let replies = drive(ctx_at(&base), &reqs, n).await;
        assert_eq!(replies.len(), n);
        let took = started.elapsed();
        // Parallel, but in two waves (8 + 2) of ~800 ms — not ten in a row
        // (8 s) and not all at once (one wave).
        assert!(
            took >= Duration::from_millis(1500),
            "cap not applied: {took:?}"
        );
        assert!(
            took < Duration::from_secs(5),
            "calls ran serially: {took:?}"
        );
    }

    #[tokio::test]
    async fn ui_tools_are_listed_only_with_the_grant_and_a_grant_relists() {
        let mock = Arc::new(MockDaemon::default());
        *mock.enabled.lock().unwrap() = all_governed_names();
        let base = mock_daemon(mock.clone()).await;
        let list = json!({"jsonrpc":"2.0","id":1,"method":"tools/list"});
        // Ungranted: no otto_ui_* tool, exactly one request stub. Then the stub
        // is called (the mock "human" allows), which must emit list_changed,
        // and the next tools/list carries the UI tools.
        let replies = drive(ctx_at(&base), std::slice::from_ref(&list), 1).await;
        let names = |r: &Value| -> Vec<String> {
            r["result"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| t["name"].as_str().unwrap().to_string())
                .collect()
        };
        let before = names(&replies[0]);
        assert!(!before
            .iter()
            .any(|n| n.starts_with("otto_ui_") && n != UI_REQUEST_CONTROL_TOOL));
        assert_eq!(
            before
                .iter()
                .filter(|n| *n == UI_REQUEST_CONTROL_TOOL)
                .count(),
            1
        );

        // A fresh bridge that advertised the ungranted catalog (the default).
        let replies = drive(
            ctx_at(&base),
            &[call(2, UI_REQUEST_CONTROL_TOOL)],
            2, // list_changed notification + stub reply
        )
        .await;
        assert!(
            replies
                .iter()
                .any(|r| r["method"] == json!("notifications/tools/list_changed")),
            "a grant must re-list: {replies:?}"
        );
        let stub = replies.iter().find(|r| r["id"] == json!(2)).unwrap();
        assert!(stub["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("\"ui_control_granted\": true"));
        // R4: the grant came back IN the reply — the stub call cost one
        // enable read (the governed call's cached check), not three (+ a
        // fresh re-read in governed_call and another in the stub).
        assert_eq!(
            mock.enabled_hits.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "one read for the first tools/list, one for the stub call"
        );
        let after = drive(ctx_at(&base), &[list], 1).await;
        let after = names(&after[0]);
        assert!(after.iter().any(|n| n == "otto_ui_state"));
        assert!(!after.iter().any(|n| n == UI_REQUEST_CONTROL_TOOL));
    }

    /// R4 fallback: a daemon older than the reply's `ui_granted` field still
    /// gets a correct re-list, through a fresh enable read.
    #[tokio::test]
    async fn a_grant_relists_against_a_daemon_without_the_reply_field() {
        let mock = Arc::new(MockDaemon::default());
        *mock.enabled.lock().unwrap() = all_governed_names();
        mock.legacy_reply
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let base = mock_daemon(mock.clone()).await;
        let replies = drive(ctx_at(&base), &[call(2, UI_REQUEST_CONTROL_TOOL)], 2).await;
        assert!(replies
            .iter()
            .any(|r| r["method"] == json!("notifications/tools/list_changed")));
        let stub = replies.iter().find(|r| r["id"] == json!(2)).unwrap();
        assert!(stub["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("\"ui_control_granted\": true"));
        // The cached check + ONE fresh re-read (the stub reuses it).
        assert_eq!(
            mock.enabled_hits.load(std::sync::atomic::Ordering::SeqCst),
            2
        );
    }

    /// R7: a native tool call's audit row goes to the daemon
    /// (`POST /mcp/tool-calls`), redacted, and is drained at EOF — the bridge
    /// opens no database connection.
    #[tokio::test]
    async fn native_tool_calls_are_audited_through_the_daemon() {
        let mock = Arc::new(MockDaemon::default());
        let base = mock_daemon(mock.clone()).await;
        let mut ctx = ctx_at(&base);
        ctx.audit = true;
        let req = json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
            "params":{"name":"otto_list_workspaces","arguments":{"password":"hunter2"}}});
        let _ = drive(ctx, &[req], 1).await;
        let audits = mock.audits.lock().unwrap().clone();
        assert_eq!(audits.len(), 1, "{audits:?}");
        assert_eq!(audits[0]["tool"], json!("otto_list_workspaces"));
        // The mock has no /workspaces route: the call failed and says so.
        assert_eq!(audits[0]["ok"], json!(false));
        assert!(
            !audits[0].to_string().contains("hunter2"),
            "arguments are redacted before they leave: {}",
            audits[0]
        );
    }

    /// See [`tools_list_catalog_stays_within_its_byte_budget`]. 79_748 at
    /// perf2/10-mcp R2 (shorter reference texts; granted 131_940); 83_418 at
    /// perf/10-mcp (granted 135_610); 83_318 after architecture-1-05 aligned
    /// the native twins (id-or-name vault_id, PR `number` alias, room refs —
    /// missing text, not padding); 85_412 with otto_create_issue and its
    /// discovery reads (list_issue_projects / _types); 84_834 with
    /// otto_query_db (review sessions: 59_688).
    const TOOLS_LIST_BYTE_BUDGET: usize = 86_000;
    /// A review-type session (R2 per-source scoping): 56_830; 60_266 with those.
    const NARROW_TOOLS_LIST_BYTE_BUDGET: usize = 61_000;

    /// A Ctx pointing at an unreachable base; used by the no-upstream tests above
    /// (which never actually call out). Audit disabled.
    fn test_ctx() -> Ctx {
        Ctx::new(
            reqwest::Client::new(),
            "http://127.0.0.1:9".to_string(),
            "test-token".to_string(),
            Some("sess-test".into()),
            Some("ws-test".into()),
            None,
            false,
        )
    }
}
