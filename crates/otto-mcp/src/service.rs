//! `McpService` — the control-plane engine: outbound-client construction (with
//! keychain secret resolution), server discovery + health, and the single
//! **governance pipeline** `invoke` every governed tool call funnels through.
//!
//! Pipeline order (design §5 + §14): resolve → server enabled/managed → allowlist
//! (deny-first) → per-tool permission → policy (most-restrictive-wins) → risk /
//! approval gate (hash-bound, single-use, approver≠requester, expiry) → dry-run
//! (pure simulation) → execute → **guaranteed** audit (fail-closed) → stats.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use chrono::{Duration as ChronoDuration, Utc};
use otto_core::redact::{redact_json, redact_text};
use otto_core::secrets::SecretStore;
use otto_core::{Error, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use otto_state::{
    DiscoveredTool, McpAllowlistRepo, McpApprovalRepo, McpCallLogRepo, McpPolicyRepo,
    McpRegistryRepo, McpServerDetail, McpTool, McpToolsRepo, NewApproval, NewCallLog, SettingsRepo,
    SqlitePool,
};

use crate::client::{McpClient, Transport};
use crate::policy::{self, Effect, PolicyCtx};
use crate::risk;

const MAX_ROWS: usize = 500;
const APPROVAL_TTL_MINS: i64 = 120;

/// The caller context for one governed invoke.
#[derive(Debug, Clone)]
pub struct InvokeCtx {
    pub workspace_id: Option<String>,
    pub dry_run: bool,
    pub caller_user_id: Option<String>,
    pub caller_kind: String, // ui|agent|agent_readonly|mcp_server|gateway
    pub direction: String,   // outbound|inbound
}

/// The terminal outcome of `invoke`.
#[derive(Debug)]
pub enum InvokeOutcome {
    Denied { reason: String },
    Pending { approval_id: String, title: String },
    DryRun { preview: Value },
    Executed { content: Value, is_error: bool },
}

#[derive(Clone)]
pub struct McpService {
    pool: SqlitePool,
    secrets: Arc<dyn SecretStore>,
}

impl McpService {
    pub fn new(pool: SqlitePool, secrets: Arc<dyn SecretStore>) -> Self {
        Self { pool, secrets }
    }

    pub fn registry(&self) -> McpRegistryRepo {
        McpRegistryRepo::new(self.pool.clone())
    }
    pub fn tools(&self) -> McpToolsRepo {
        McpToolsRepo::new(self.pool.clone())
    }
    pub fn allowlist(&self) -> McpAllowlistRepo {
        McpAllowlistRepo::new(self.pool.clone())
    }
    pub fn policies(&self) -> McpPolicyRepo {
        McpPolicyRepo::new(self.pool.clone())
    }
    pub fn call_log(&self) -> McpCallLogRepo {
        McpCallLogRepo::new(self.pool.clone())
    }
    pub fn approvals(&self) -> McpApprovalRepo {
        McpApprovalRepo::new(self.pool.clone())
    }

    /// The keychain ref for a server's secret blob.
    pub fn secret_ref(id: &str) -> String {
        format!("mcp-{id}")
    }

    /// Resolve the keychain secret blob `{env:{},headers:{}}` for a server.
    fn resolve_secrets(&self, server: &McpServerDetail) -> (BTreeMap<String, String>, BTreeMap<String, String>) {
        let mut env = BTreeMap::new();
        let mut headers = BTreeMap::new();
        if !server.has_secret {
            return (env, headers);
        }
        if let Ok(Some(blob)) = self.secrets.get(&Self::secret_ref(&server.id)) {
            if let Ok(v) = serde_json::from_str::<Value>(&blob) {
                if let Some(e) = v.get("env").and_then(Value::as_object) {
                    for (k, val) in e {
                        if let Some(s) = val.as_str() {
                            env.insert(k.clone(), s.to_string());
                        }
                    }
                }
                if let Some(h) = v.get("headers").and_then(Value::as_object) {
                    for (k, val) in h {
                        if let Some(s) = val.as_str() {
                            headers.insert(k.clone(), s.to_string());
                        }
                    }
                }
            }
        }
        (env, headers)
    }

    /// Build an outbound client for a server, overlaying keychain secrets onto the
    /// plaintext config.
    fn client_for(&self, server: &McpServerDetail) -> McpClient {
        let (secret_env, secret_headers) = self.resolve_secrets(server);
        match server.transport.as_str() {
            "http" => {
                let mut headers = server.headers.clone();
                headers.extend(secret_headers);
                McpClient::new(Transport::Http {
                    url: server.url.clone().unwrap_or_default(),
                    headers,
                })
            }
            _ => {
                let mut env: BTreeMap<String, String> = std::env::vars().collect();
                env.extend(server.env.clone());
                env.extend(secret_env);
                McpClient::new(Transport::Stdio {
                    command: server.command.clone(),
                    args: server.args.clone(),
                    env,
                })
            }
        }
    }

    // ---- discovery + health ----------------------------------------------

    /// Discover a server's tools, label their risk, and upsert the catalog.
    pub async fn discover(&self, server_id: &str) -> Result<Vec<McpTool>> {
        let server = self.registry().get(&server_id.to_string()).await?;
        let client = self.client_for(&server);
        let raw = client
            .list_tools()
            .await
            .map_err(|e| Error::Internal(format!("discover: {e}")))?;
        let discovered: Vec<DiscoveredTool> = raw
            .iter()
            .filter_map(|t| {
                let name = t.get("name").and_then(Value::as_str)?.to_string();
                let description = t.get("description").and_then(Value::as_str).map(str::to_string);
                let annotations = t.get("annotations").cloned().unwrap_or(json!({}));
                let labels = risk::label_tool(&name, description.as_deref(), &annotations);
                Some(DiscoveredTool {
                    name,
                    title: t.get("title").and_then(Value::as_str).map(str::to_string),
                    description,
                    input_schema: t.get("inputSchema").cloned().unwrap_or(json!({})),
                    annotations,
                    risk_label: labels.risk_label,
                    injection_risk: labels.injection_risk,
                    mutating: labels.mutating,
                    supports_dry_run: labels.supports_dry_run,
                })
            })
            .collect();
        self.tools().upsert_discovered(&server.id, &discovered).await?;
        self.registry()
            .set_tools_meta(&server.id, discovered.len() as i64)
            .await?;
        self.tools().list_for_server(&server.id).await
    }

    /// Probe a server's health (initialize round-trip), recording status+latency.
    pub async fn health_check(&self, server_id: &str) -> Result<McpServerDetail> {
        let server = self.registry().get(&server_id.to_string()).await?;
        if !server.enabled {
            self.registry().set_health(&server.id, "disabled", None, None).await?;
            return self.registry().get(&server.id).await;
        }
        let client = self.client_for(&server);
        let start = Instant::now();
        let res = client.health().await;
        let latency = start.elapsed().as_millis() as i64;
        match res {
            Ok(()) => {
                self.registry().set_health(&server.id, "healthy", Some(latency), None).await?;
            }
            Err(e) => {
                let err = redact_text(&e).value;
                self.registry().set_health(&server.id, "unhealthy", Some(latency), Some(&err)).await?;
            }
        }
        self.registry().get(&server.id).await
    }

    /// Best-effort health sweep across all managed servers (background tick).
    pub async fn health_sweep(&self) {
        let servers = match self.registry().list_all_managed().await {
            Ok(s) => s,
            Err(_) => return,
        };
        for s in servers {
            if s.enabled {
                let _ = self.health_check(&s.id).await;
            }
        }
        let _ = self.approvals().expire_stale().await;
    }

    // ---- the governance pipeline -----------------------------------------

    /// Run one governed tool call. Every terminal path writes exactly one audit
    /// row (fail-closed: an audit-insert failure aborts before execution).
    pub async fn invoke(
        &self,
        server_id: &str,
        tool_name: &str,
        args: &Value,
        ctx: &InvokeCtx,
    ) -> Result<InvokeOutcome> {
        let server = self.registry().get(&server_id.to_string()).await?;
        // Tool metadata (must be discovered to be governed).
        let tool = self.tools().get_by_name(&server.id, tool_name).await.ok();
        let (risk_label, injection_risk, mutating, tool_enabled, tool_require_approval) = match &tool {
            Some(t) => (
                t.risk_label.clone(),
                t.injection_risk.clone(),
                t.mutating,
                t.enabled,
                t.require_approval,
            ),
            // Unknown/undiscovered tool: fail closed (treat as dangerous + disabled).
            None => ("dangerous".into(), "high".into(), true, false, true),
        };

        // 0. server gate.
        if !server.enabled || !server.managed {
            return self
                .terminal_deny(&server, tool_name, args, &risk_label, &injection_risk, ctx,
                    "server is disabled or not managed").await;
        }
        // 1. allowlist (per workspace), deny wins.
        if let Some(ws) = ctx.workspace_id.as_deref() {
            match self.allowlist().resolve(&ws.to_string(), &server.id, tool_name).await? {
                Some(mode) if mode == "deny" => {
                    return self.terminal_deny(&server, tool_name, args, &risk_label, &injection_risk, ctx,
                        "workspace allowlist denies this tool").await;
                }
                Some(_) => {} // explicit allow
                None => {
                    if server.default_tool_access == "deny" {
                        return self.terminal_deny(&server, tool_name, args, &risk_label, &injection_risk, ctx,
                            "not in workspace allowlist (server default = deny)").await;
                    }
                }
            }
        }
        // 2. per-tool permission.
        if !tool_enabled {
            return self
                .terminal_deny(&server, tool_name, args, &risk_label, &injection_risk, ctx,
                    "tool is disabled (per-tool permission)").await;
        }
        // 3. policy-as-code (most-restrictive-wins).
        let rules = self
            .policies()
            .list_applicable(ctx.workspace_id.as_deref().unwrap_or(""))
            .await?;
        let pctx = PolicyCtx {
            server_id: &server.id,
            server_name: &server.name,
            tool: tool_name,
            risk_label: &risk_label,
            injection_risk: &injection_risk,
            mutating,
            direction: &ctx.direction,
            caller_kind: &ctx.caller_kind,
            workspace_id: ctx.workspace_id.as_deref(),
        };
        let effect = policy::evaluate(&rules, &pctx);
        if let Effect::Deny(reason) = &effect {
            return self
                .terminal_deny(&server, tool_name, args, &risk_label, &injection_risk, ctx,
                    &format!("policy denied: {reason}")).await;
        }
        let policy_dry_run = matches!(effect, Effect::RequireDryRun(_));
        let policy_approval = matches!(effect, Effect::RequireApproval(_));

        // 4. risk / approval gate. dry-run requests skip the approval *creation*
        //    (a preview executes nothing), but a policy require_dry_run still applies.
        let dangerous_default = risk_label == "dangerous" && self.require_approval_dangerous().await;
        let needs_approval = policy_approval || tool_require_approval || dangerous_default;
        let args_hash = canonical_hash(args);

        let mut approval_id_used: Option<String> = None;
        if needs_approval && !ctx.dry_run {
            match self
                .approvals()
                .find_usable(ctx.workspace_id.as_deref(), Some(&server.id), tool_name, &args_hash)
                .await?
            {
                Some(appr_id) => {
                    // Single-use: consume atomically; a lost race => already used.
                    if !self.approvals().consume(&appr_id).await? {
                        return self.terminal_deny(&server, tool_name, args, &risk_label, &injection_risk, ctx,
                            "approval was already used").await;
                    }
                    approval_id_used = Some(appr_id);
                }
                None => {
                    // Create a pending approval bound to the EXACT args.
                    let redacted = redact_json(args).value.to_string();
                    let expires = (Utc::now() + ChronoDuration::minutes(APPROVAL_TTL_MINS)).to_rfc3339();
                    let appr = self
                        .approvals()
                        .create(NewApproval {
                            workspace_id: ctx.workspace_id.clone(),
                            kind: "tool_call".into(),
                            server_id: Some(server.id.clone()),
                            server_name: Some(server.name.clone()),
                            tool: Some(tool_name.to_string()),
                            title: format!("{} → {}", server.name, tool_name),
                            detail: Some(format!(
                                "Approve {risk_label} MCP tool '{tool_name}' on server '{}'.",
                                server.name
                            )),
                            args_redacted_json: redacted,
                            args_hash: Some(args_hash.clone()),
                            risk_label: Some(risk_label.clone()),
                            requested_by: ctx.caller_user_id.clone(),
                            requested_by_kind: Some(ctx.caller_kind.clone()),
                            expires_at: Some(expires),
                        })
                        .await?;
                    self.audit_terminal(
                        &server, tool_name, args, &risk_label, &injection_risk, ctx,
                        "pending_approval", Some(&format!("awaiting approval for {risk_label} tool")),
                        Some(&appr.id),
                    )
                    .await?;
                    return Ok(InvokeOutcome::Pending { approval_id: appr.id, title: format!("{} → {}", server.name, tool_name) });
                }
            }
        }

        // 5. dry-run = pure simulation (never calls the tool). design §14 F4.
        if ctx.dry_run || policy_dry_run {
            let preview = json!({
                "executed": false,
                "mode": "preview",
                "would_call": { "server": server.name, "tool": tool_name, "arguments": redact_json(args).value },
                "note": "dry-run: arguments validated and target resolved; the tool was NOT executed",
            });
            self.audit_terminal(&server, tool_name, args, &risk_label, &injection_risk, ctx,
                "dry_run", None, approval_id_used.as_deref()).await?;
            return Ok(InvokeOutcome::DryRun { preview });
        }

        // 6. execute. Fail-closed audit: insert the row BEFORE running so an
        //    audit failure aborts the call; finalize with the outcome after.
        let decision = if approval_id_used.is_some() { "approved" } else { "allowed" };
        let audit_id = self
            .call_log()
            .insert(NewCallLog {
                workspace_id: ctx.workspace_id.clone(),
                server_id: Some(server.id.clone()),
                server_name: Some(server.name.clone()),
                tool: tool_name.to_string(),
                direction: ctx.direction.clone(),
                caller_user_id: ctx.caller_user_id.clone(),
                caller_kind: Some(ctx.caller_kind.clone()),
                args_redacted_json: redact_json(args).value.to_string(),
                decision: decision.into(),
                decision_reason: None,
                risk_label: Some(risk_label.clone()),
                injection_risk: Some(injection_risk.clone()),
                dry_run: false,
                ok: false, // finalized below
                error: None,
                latency_ms: None,
                bytes: None,
                rows: None,
                approval_id: approval_id_used.clone(),
            })
            .await?; // ← propagates: no audit row ⇒ no execution (fail-closed)

        let client = self.client_for(&server);
        let start = Instant::now();
        let res = client.call_tool(tool_name, args).await;
        let latency = start.elapsed().as_millis() as i64;
        match res {
            Ok(call) => {
                let mut rows = 0usize;
                let capped = cap_rows(call.content.clone(), &mut rows);
                let content = redact_json(&capped).value;
                self.call_log()
                    .finalize(&audit_id, !call.is_error, None, Some(latency), Some(call.bytes as i64), Some(rows as i64))
                    .await?;
                Ok(InvokeOutcome::Executed { content, is_error: call.is_error })
            }
            Err(e) => {
                let err = redact_text(&e).value;
                self.call_log()
                    .finalize(&audit_id, false, Some(&err), Some(latency), None, None)
                    .await?;
                Ok(InvokeOutcome::Executed {
                    content: json!({ "error": err }),
                    is_error: true,
                })
            }
        }
    }

    /// Read-only preview of the decision the pipeline would make (no side effects,
    /// no execution) — for the UI policy/evaluate view.
    pub async fn evaluate_preview(
        &self,
        server_id: &str,
        tool_name: &str,
        workspace_id: Option<&str>,
    ) -> Result<Value> {
        let server = self.registry().get(&server_id.to_string()).await?;
        let tool = self.tools().get_by_name(&server.id, tool_name).await.ok();
        let (risk_label, injection_risk, mutating) = match &tool {
            Some(t) => (t.risk_label.clone(), t.injection_risk.clone(), t.mutating),
            None => ("dangerous".into(), "high".into(), true),
        };
        let rules = self.policies().list_applicable(workspace_id.unwrap_or("")).await?;
        let pctx = PolicyCtx {
            server_id: &server.id,
            server_name: &server.name,
            tool: tool_name,
            risk_label: &risk_label,
            injection_risk: &injection_risk,
            mutating,
            direction: "outbound",
            caller_kind: "ui",
            workspace_id,
        };
        let effect = policy::evaluate(&rules, &pctx);
        let (decision, reason) = match effect {
            Effect::Allow => ("allow", None),
            Effect::Deny(r) => ("deny", Some(r)),
            Effect::RequireApproval(r) => ("require_approval", Some(r)),
            Effect::RequireDryRun(r) => ("require_dry_run", Some(r)),
        };
        Ok(json!({
            "server": server.name,
            "tool": tool_name,
            "risk_label": risk_label,
            "injection_risk": injection_risk,
            "policy_decision": decision,
            "reason": reason,
        }))
    }

    async fn require_approval_dangerous(&self) -> bool {
        SettingsRepo::new(self.pool.clone())
            .get("mcp_require_approval_dangerous")
            .await
            .ok()
            .flatten()
            .and_then(|v| v.as_bool())
            .unwrap_or(true)
    }

    // ---- audit helpers (every terminal path writes exactly one row) -------

    #[allow(clippy::too_many_arguments)]
    async fn terminal_deny(
        &self,
        server: &McpServerDetail,
        tool: &str,
        args: &Value,
        risk_label: &str,
        injection_risk: &str,
        ctx: &InvokeCtx,
        reason: &str,
    ) -> Result<InvokeOutcome> {
        self.audit_terminal(server, tool, args, risk_label, injection_risk, ctx, "denied", Some(reason), None)
            .await?;
        Ok(InvokeOutcome::Denied { reason: reason.to_string() })
    }

    #[allow(clippy::too_many_arguments)]
    async fn audit_terminal(
        &self,
        server: &McpServerDetail,
        tool: &str,
        args: &Value,
        risk_label: &str,
        injection_risk: &str,
        ctx: &InvokeCtx,
        decision: &str,
        reason: Option<&str>,
        approval_id: Option<&str>,
    ) -> Result<()> {
        self.call_log()
            .insert(NewCallLog {
                workspace_id: ctx.workspace_id.clone(),
                server_id: Some(server.id.clone()),
                server_name: Some(server.name.clone()),
                tool: tool.to_string(),
                direction: ctx.direction.clone(),
                caller_user_id: ctx.caller_user_id.clone(),
                caller_kind: Some(ctx.caller_kind.clone()),
                args_redacted_json: redact_json(args).value.to_string(),
                decision: decision.to_string(),
                decision_reason: reason.map(str::to_string),
                risk_label: Some(risk_label.to_string()),
                injection_risk: Some(injection_risk.to_string()),
                dry_run: decision == "dry_run",
                ok: decision == "dry_run", // denials/pending are not "ok" outcomes
                error: None,
                latency_ms: None,
                bytes: None,
                rows: None,
                approval_id: approval_id.map(str::to_string),
            })
            .await
            .map(|_| ())
    }
}

/// Canonical (sorted-key) JSON + sha256 hex. Binds an approval to exact args so a
/// post-approval argument swap is rejected by the gate.
pub fn canonical_hash(v: &Value) -> String {
    let mut hasher = Sha256::new();
    hasher.update(canonical_string(v).as_bytes());
    hex::encode(hasher.finalize())
}

fn canonical_string(v: &Value) -> String {
    match v {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let inner: Vec<String> = keys
                .into_iter()
                .map(|k| format!("{:?}:{}", k, canonical_string(&map[k])))
                .collect();
            format!("{{{}}}", inner.join(","))
        }
        Value::Array(items) => {
            let inner: Vec<String> = items.iter().map(canonical_string).collect();
            format!("[{}]", inner.join(","))
        }
        other => other.to_string(),
    }
}

/// Recursively cap every JSON array to `MAX_ROWS`, appending a marker; tracks the
/// largest array length seen (the audited row count).
fn cap_rows(v: Value, max_seen: &mut usize) -> Value {
    match v {
        Value::Array(items) => {
            let n = items.len();
            if n > *max_seen {
                *max_seen = n;
            }
            let truncated = n > MAX_ROWS;
            let mut out: Vec<Value> = items.into_iter().take(MAX_ROWS).map(|i| cap_rows(i, max_seen)).collect();
            if truncated {
                out.push(Value::String(format!("[otto: truncated — {n} items, showing first {MAX_ROWS}]")));
            }
            Value::Array(out)
        }
        Value::Object(map) => Value::Object(map.into_iter().map(|(k, val)| (k, cap_rows(val, max_seen))).collect()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_hash_is_key_order_independent() {
        let a = json!({"a":1,"b":[1,2,{"x":1,"y":2}]});
        let b = json!({"b":[1,2,{"y":2,"x":1}],"a":1});
        assert_eq!(canonical_hash(&a), canonical_hash(&b));
        let c = json!({"a":1,"b":[1,2,{"x":1,"y":3}]});
        assert_ne!(canonical_hash(&a), canonical_hash(&c));
    }
}
