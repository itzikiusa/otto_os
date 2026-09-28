//! `McpService` — the control-plane engine: outbound-client construction (with
//! keychain secret resolution), server discovery + health, and the single
//! **governance pipeline** `invoke` every governed tool call funnels through.
//!
//! Pipeline order (design §5 + §14): resolve → server enabled/managed → allowlist
//! (deny-first) → per-tool permission → policy (most-restrictive-wins) → risk /
//! approval gate (hash-bound, single-use, approver≠requester, expiry) → dry-run
//! (pure simulation) → execute → **guaranteed** audit (fail-closed) → stats.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{Duration as ChronoDuration, Utc};
use otto_core::redact::{redact_json, redact_text};
use otto_core::secrets::SecretStore;
use otto_core::{Error, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use otto_state::{
    DbPool, DiscoveredTool, McpAllowlistRepo, McpApprovalRepo, McpCallLogRepo, McpPolicyRepo,
    McpRegistryRepo, McpServerDetail, McpTool, McpToolsRepo, NewApproval, NewCallLog, SettingsRepo,
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

/// Pooled outbound clients (SE-14): one per server, reused while its effective
/// config (transport + plaintext config + keychain secrets) hashes the same.
/// An entry idle past [`CLIENT_IDLE_TTL`] is dropped — which kills a parked
/// stdio child — on the next checkout or health sweep.
const CLIENT_IDLE_TTL: Duration = Duration::from_secs(5 * 60);
const CLIENT_POOL_CAP: usize = 32;
/// The background sweep re-probes a STDIO server only if something used it
/// this recently (r3-08-05: probing = spawning the server, e.g. an `npx`
/// package, for every enabled server every 5 min whether or not anything uses
/// MCP). HTTP servers are always probed (a request, no process). Unused stdio
/// servers keep their last health until used or probed from the UI.
const STDIO_SWEEP_RECENT: Duration = Duration::from_secs(6 * 3600);

struct PooledClient {
    config_hash: String,
    client: Arc<McpClient>,
    last_used: Instant,
}

#[derive(Clone)]
pub struct McpService {
    pool: DbPool,
    secrets: Arc<dyn SecretStore>,
    clients: Arc<Mutex<HashMap<String, PooledClient>>>,
    /// server id → last time a governed op checked out its client. Outlives
    /// the pool entry (reaped after 5 min idle); decides which stdio servers
    /// the sweep still probes.
    last_use: Arc<Mutex<HashMap<String, Instant>>>,
}

/// What the background sweep does for one enabled server.
#[derive(Debug, PartialEq, Eq)]
enum SweepAction {
    /// A request (HTTP) or a server in recent use with no live session.
    Probe,
    /// A stdio server with a parked, initialized session: a `tools/list` on it
    /// (no spawn) — the live connection is reused.
    PingLive,
    /// A stdio server nothing used recently: no process is started.
    Skip,
}

fn sweep_action(
    transport: &str,
    has_live_session: bool,
    used_ago: Option<Duration>,
) -> SweepAction {
    if transport == "http" {
        return SweepAction::Probe;
    }
    match used_ago {
        Some(ago) if ago < STDIO_SWEEP_RECENT => {
            if has_live_session {
                SweepAction::PingLive
            } else {
                SweepAction::Probe
            }
        }
        _ => SweepAction::Skip,
    }
}

impl McpService {
    pub fn new(pool: impl Into<DbPool>, secrets: Arc<dyn SecretStore>) -> Self {
        let pool: DbPool = pool.into();
        Self {
            pool,
            secrets,
            clients: Arc::new(Mutex::new(HashMap::new())),
            last_use: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Number of pooled clients (diagnostics / tests).
    pub fn pooled_clients(&self) -> usize {
        self.clients.lock().map(|m| m.len()).unwrap_or(0)
    }

    /// Drop a server's pooled client now (config edit, disable, delete).
    pub fn evict_client(&self, server_id: &str) {
        if let Ok(mut m) = self.clients.lock() {
            m.remove(server_id);
        }
    }

    /// Drop pooled clients idle past the TTL (their stdio children die with them).
    pub fn reap_idle_clients(&self) {
        if let Ok(mut m) = self.clients.lock() {
            m.retain(|_, c| c.last_used.elapsed() < CLIENT_IDLE_TTL);
        }
    }

    /// Current identity and resource policy are checked again at execution, so
    /// API, gateway and agent callers share the same authorization boundary.
    pub async fn resource_allowed(
        &self,
        server: &otto_state::McpServerDetail,
        user: &otto_core::domain::User,
        operation: &str,
        child: Option<&str>,
    ) -> Result<bool> {
        use otto_core::access::{AccessMode, ResourceKind, ResourceRef};
        self.registry().get(&server.id).await?;
        let policy = otto_state::ResourceAccessRepo::new(self.pool.clone())
            .get_live_policy(ResourceKind::McpServer, &server.id)
            .await?;
        if policy.mode == AccessMode::Legacy {
            return Ok(!user.disabled);
        }
        let feature = otto_state::GrantsRepo::new(self.pool.clone())
            .capability_of(user, otto_core::domain::Feature::Mcp)
            .await?;
        if feature < otto_core::domain::Capability::View {
            return Ok(false);
        }
        if otto_state::WorkspacesRepo::new(self.pool.clone())
            .role_of(user, &server.workspace_id)
            .await?
            .is_none()
        {
            return Ok(false);
        }
        let access = otto_rbac::ResourceAccess::new(self.pool.clone());
        let resource = ResourceRef {
            kind: ResourceKind::McpServer,
            id: server.id.clone(),
            child: child.map(str::to_string),
        };
        if operation != "discover" && !access.evaluate(user, &resource, "discover").await?.allowed {
            return Ok(false);
        }
        Ok(access.evaluate(user, &resource, operation).await?.allowed)
    }

    pub async fn visible_tools(
        &self,
        server: &otto_state::McpServerDetail,
        user: &otto_core::domain::User,
    ) -> Result<Vec<otto_state::McpTool>> {
        let mut visible = Vec::new();
        for tool in self.tools().list_for_server(&server.id).await? {
            if self
                .resource_allowed(server, user, "discover", Some(&tool.name))
                .await?
                && (self
                    .resource_allowed(server, user, "invoke", Some(&tool.name))
                    .await?
                    || self
                        .resource_allowed(server, user, "configure", Some(&tool.name))
                        .await?)
            {
                visible.push(tool);
            }
        }
        Ok(visible)
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
    async fn resolve_secrets(
        &self,
        server: &McpServerDetail,
    ) -> (BTreeMap<String, String>, BTreeMap<String, String>) {
        let mut env = BTreeMap::new();
        let mut headers = BTreeMap::new();
        if !server.has_secret {
            return (env, headers);
        }
        // Every governed invoke + health check lands here: cache hit inline,
        // a keychain miss on the blocking pool (never a stalled worker).
        if let Ok(Some(blob)) =
            otto_core::secrets::get_async(&self.secrets, &Self::secret_ref(&server.id)).await
        {
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

    /// The pooled outbound client for a server (SE-14): reused while the
    /// server's effective config is unchanged, so governed calls stop paying a
    /// spawn + `initialize` each (1–3 s for an `npx` server). A config or
    /// secret change hashes differently and replaces the entry.
    async fn client_for(&self, server: &McpServerDetail) -> Arc<McpClient> {
        if let Ok(mut m) = self.last_use.lock() {
            m.insert(server.id.clone(), Instant::now());
        }
        let (secret_env, secret_headers) = self.resolve_secrets(server).await;
        let hash = client_config_hash(server, &secret_env, &secret_headers);
        if let Ok(mut m) = self.clients.lock() {
            m.retain(|_, c| c.last_used.elapsed() < CLIENT_IDLE_TTL);
            if let Some(entry) = m.get_mut(&server.id) {
                if entry.config_hash == hash {
                    entry.last_used = Instant::now();
                    return entry.client.clone();
                }
            }
        }
        let client = Arc::new(Self::build_client(server, secret_env, secret_headers));
        if let Ok(mut m) = self.clients.lock() {
            if m.len() >= CLIENT_POOL_CAP && !m.contains_key(&server.id) {
                // Evict the least recently used entry.
                if let Some(oldest) = m
                    .iter()
                    .min_by_key(|(_, c)| c.last_used)
                    .map(|(k, _)| k.clone())
                {
                    m.remove(&oldest);
                }
            }
            m.insert(
                server.id.clone(),
                PooledClient {
                    config_hash: hash,
                    client: client.clone(),
                    last_used: Instant::now(),
                },
            );
        }
        client
    }

    /// Build an outbound client for a server, overlaying keychain secrets onto the
    /// plaintext config.
    fn build_client(
        server: &McpServerDetail,
        secret_env: BTreeMap<String, String>,
        secret_headers: BTreeMap<String, String>,
    ) -> McpClient {
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
        let client = self.client_for(&server).await;
        let raw = client
            .list_tools()
            .await
            .map_err(|e| Error::Internal(format!("discover: {e}")))?;
        let discovered: Vec<DiscoveredTool> = raw
            .iter()
            .filter_map(|t| {
                let name = t.get("name").and_then(Value::as_str)?.to_string();
                let description = t
                    .get("description")
                    .and_then(Value::as_str)
                    .map(str::to_string);
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
        self.tools()
            .upsert_discovered(&server.id, &discovered)
            .await?;
        self.registry()
            .set_tools_meta(&server.id, discovered.len() as i64)
            .await?;
        self.tools().list_for_server(&server.id).await
    }

    /// Probe a server's health (initialize round-trip), recording status+latency.
    pub async fn health_check(&self, server_id: &str) -> Result<McpServerDetail> {
        let server = self.registry().get(&server_id.to_string()).await?;
        if !server.enabled {
            self.registry()
                .set_health(&server.id, "disabled", None, None)
                .await?;
            return self.registry().get(&server.id).await;
        }
        // Health is a one-shot probe that the server can START; it never
        // touches (or parks) the pooled session.
        let (secret_env, secret_headers) = self.resolve_secrets(&server).await;
        let client = Self::build_client(&server, secret_env, secret_headers);
        let start = Instant::now();
        let res = client.health().await;
        let latency = start.elapsed().as_millis() as i64;
        match res {
            Ok(()) => {
                self.registry()
                    .set_health(&server.id, "healthy", Some(latency), None)
                    .await?;
            }
            Err(e) => {
                let err = redact_text(&e).value;
                self.registry()
                    .set_health(&server.id, "unhealthy", Some(latency), Some(&err))
                    .await?;
            }
        }
        self.registry().get(&server.id).await
    }

    /// Best-effort health sweep across all managed servers (background tick).
    ///
    /// Lazy for stdio servers (r3-08-05): only servers used in the last
    /// [`STDIO_SWEEP_RECENT`] are checked, over their parked session when one is
    /// live. HTTP servers are probed every sweep. An explicit
    /// [`health_check`](Self::health_check) (the UI's Health button) always
    /// probes. Governed calls also record health as they succeed or fail.
    pub async fn health_sweep(&self) {
        self.reap_idle_clients();
        let servers = match self.registry().list_all_managed().await {
            Ok(s) => s,
            Err(_) => return,
        };
        for s in servers {
            if !s.enabled {
                continue;
            }
            let used_ago = self
                .last_use
                .lock()
                .ok()
                .and_then(|m| m.get(&s.id).map(|t| t.elapsed()));
            let live = self.live_client(&s.id);
            let action = sweep_action(
                &s.transport,
                live.as_ref().is_some_and(|c| c.has_live_session()),
                used_ago,
            );
            match (action, live) {
                (SweepAction::Skip, _) => {}
                (SweepAction::PingLive, Some(client)) => {
                    let start = Instant::now();
                    let res = client.list_tools().await.map(|_| ());
                    let latency = start.elapsed().as_millis() as i64;
                    let _ = self.record_health(&s, res, Some(latency)).await;
                }
                _ => {
                    let _ = self.health_check(&s.id).await;
                }
            }
        }
        let _ = self.approvals().expire_stale().await;
    }

    /// The pooled client for `server_id` when one is parked and not idle-expired
    /// (no checkout: the sweep must not keep an idle session alive).
    fn live_client(&self, server_id: &str) -> Option<Arc<McpClient>> {
        let m = self.clients.lock().ok()?;
        m.get(server_id)
            .filter(|c| c.last_used.elapsed() < CLIENT_IDLE_TTL)
            .map(|c| c.client.clone())
    }

    /// Write a health outcome. `latency` = `None` (a governed call's outcome)
    /// writes only when the status flips, so busy servers don't rewrite their
    /// row per call.
    async fn record_health(
        &self,
        server: &McpServerDetail,
        res: std::result::Result<(), String>,
        latency: Option<i64>,
    ) -> Result<()> {
        let (status, err) = match &res {
            Ok(()) => ("healthy", None),
            Err(e) => ("unhealthy", Some(redact_text(e).value)),
        };
        if latency.is_none() && server.health_status == status {
            return Ok(());
        }
        self.registry()
            .set_health(&server.id, status, latency, err.as_deref())
            .await
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
        let (risk_label, injection_risk, mutating, tool_enabled, tool_require_approval) =
            match &tool {
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

        let access_policy = otto_state::ResourceAccessRepo::new(self.pool.clone())
            .get_policy(otto_core::access::ResourceKind::McpServer, &server.id)
            .await?;
        if access_policy.mode == otto_core::access::AccessMode::Enforced {
            let permitted = match &ctx.caller_user_id {
                Some(uid) => match otto_state::UsersRepo::new(self.pool.clone()).get(uid).await {
                    Ok(user) => {
                        self.resource_allowed(&server, &user, "invoke", Some(tool_name))
                            .await?
                    }
                    Err(otto_core::Error::NotFound(_)) => false,
                    Err(e) => return Err(e),
                },
                None => false,
            };
            if !permitted {
                return self
                    .terminal_deny(
                        &server,
                        tool_name,
                        args,
                        &risk_label,
                        &injection_risk,
                        ctx,
                        "resource access denied",
                    )
                    .await;
            }
        }
        // 0. server gate.
        if !server.enabled
            || (!server.managed && access_policy.mode == otto_core::access::AccessMode::Legacy)
        {
            return self
                .terminal_deny(
                    &server,
                    tool_name,
                    args,
                    &risk_label,
                    &injection_risk,
                    ctx,
                    "server is disabled or not managed",
                )
                .await;
        }
        // 1. allowlist (per workspace), deny wins.
        if let Some(ws) = ctx.workspace_id.as_deref() {
            match self
                .allowlist()
                .resolve(&ws.to_string(), &server.id, tool_name)
                .await?
            {
                Some(mode) if mode == "deny" => {
                    return self
                        .terminal_deny(
                            &server,
                            tool_name,
                            args,
                            &risk_label,
                            &injection_risk,
                            ctx,
                            "workspace allowlist denies this tool",
                        )
                        .await;
                }
                Some(_) => {} // explicit allow
                None => {
                    if server.default_tool_access == "deny" {
                        return self
                            .terminal_deny(
                                &server,
                                tool_name,
                                args,
                                &risk_label,
                                &injection_risk,
                                ctx,
                                "not in workspace allowlist (server default = deny)",
                            )
                            .await;
                    }
                }
            }
        }
        // 2. per-tool permission.
        if !tool_enabled {
            return self
                .terminal_deny(
                    &server,
                    tool_name,
                    args,
                    &risk_label,
                    &injection_risk,
                    ctx,
                    "tool is disabled (per-tool permission)",
                )
                .await;
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
                .terminal_deny(
                    &server,
                    tool_name,
                    args,
                    &risk_label,
                    &injection_risk,
                    ctx,
                    &format!("policy denied: {reason}"),
                )
                .await;
        }
        let policy_dry_run = matches!(effect, Effect::RequireDryRun(_));
        let policy_approval = matches!(effect, Effect::RequireApproval(_));

        // 4. risk / approval gate. dry-run requests skip the approval *creation*
        //    (a preview executes nothing), but a policy require_dry_run still applies.
        let dangerous_default =
            risk_label == "dangerous" && self.require_approval_dangerous().await;
        let needs_approval = policy_approval || tool_require_approval || dangerous_default;
        let args_hash = canonical_hash(args);

        let mut approval_id_used: Option<String> = None;
        if needs_approval && !ctx.dry_run {
            match self
                .approvals()
                .find_usable(
                    ctx.workspace_id.as_deref(),
                    Some(&server.id),
                    tool_name,
                    &args_hash,
                )
                .await?
            {
                Some(appr_id) => {
                    let approval = self.approvals().get(&appr_id).await?;
                    if approval.requested_by != ctx.caller_user_id {
                        return self
                            .terminal_deny(
                                &server,
                                tool_name,
                                args,
                                &risk_label,
                                &injection_risk,
                                ctx,
                                "approval belongs to another caller",
                            )
                            .await;
                    }
                    // Single-use: consume atomically; a lost race => already used.
                    if !self.approvals().consume(&appr_id).await? {
                        return self
                            .terminal_deny(
                                &server,
                                tool_name,
                                args,
                                &risk_label,
                                &injection_risk,
                                ctx,
                                "approval was already used",
                            )
                            .await;
                    }
                    approval_id_used = Some(appr_id);
                }
                None => {
                    // Create a pending approval bound to the EXACT args.
                    let redacted = redact_json(args).value.to_string();
                    let expires =
                        (Utc::now() + ChronoDuration::minutes(APPROVAL_TTL_MINS)).to_rfc3339();
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
                        &server,
                        tool_name,
                        args,
                        &risk_label,
                        &injection_risk,
                        ctx,
                        "pending_approval",
                        Some(&format!("awaiting approval for {risk_label} tool")),
                        Some(&appr.id),
                    )
                    .await?;
                    return Ok(InvokeOutcome::Pending {
                        approval_id: appr.id,
                        title: format!("{} → {}", server.name, tool_name),
                    });
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
            self.audit_terminal(
                &server,
                tool_name,
                args,
                &risk_label,
                &injection_risk,
                ctx,
                "dry_run",
                None,
                approval_id_used.as_deref(),
            )
            .await?;
            return Ok(InvokeOutcome::DryRun { preview });
        }

        // 6. execute. Fail-closed audit: insert the row BEFORE running so an
        //    audit failure aborts the call; finalize with the outcome after.
        let decision = if approval_id_used.is_some() {
            "approved"
        } else {
            "allowed"
        };
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

        let client = self.client_for(&server).await;
        let start = Instant::now();
        let latest = otto_state::ResourceAccessRepo::new(self.pool.clone())
            .get_policy(otto_core::access::ResourceKind::McpServer, &server.id)
            .await?;
        if latest.mode == otto_core::access::AccessMode::Enforced {
            let permitted = match &ctx.caller_user_id {
                Some(id) => match otto_state::UsersRepo::new(self.pool.clone()).get(id).await {
                    Ok(user) => {
                        self.resource_allowed(&server, &user, "invoke", Some(tool_name))
                            .await?
                    }
                    Err(_) => false,
                },
                None => false,
            };
            if !permitted {
                return self
                    .terminal_deny(
                        &server,
                        tool_name,
                        args,
                        &risk_label,
                        &injection_risk,
                        ctx,
                        "resource access revoked before execution",
                    )
                    .await;
            }
        }
        self.registry().get(&server.id).await?;
        let res = client.call_tool(tool_name, args).await;
        let latency = start.elapsed().as_millis() as i64;
        // Health on use: a transport failure marks the server unhealthy, any
        // answer (even a tool-level error) healthy — written on change only.
        let outcome = res.as_ref().map(|_| ()).map_err(Clone::clone);
        let _ = self.record_health(&server, outcome, None).await;
        match res {
            Ok(call) => {
                let mut rows = 0usize;
                let capped = cap_rows(call.content.clone(), &mut rows);
                let content = redact_json(&capped).value;
                self.call_log()
                    .finalize(
                        &audit_id,
                        !call.is_error,
                        None,
                        Some(latency),
                        Some(call.bytes as i64),
                        Some(rows as i64),
                    )
                    .await?;
                Ok(InvokeOutcome::Executed {
                    content,
                    is_error: call.is_error,
                })
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
        let rules = self
            .policies()
            .list_applicable(workspace_id.unwrap_or(""))
            .await?;
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
        self.audit_terminal(
            server,
            tool,
            args,
            risk_label,
            injection_risk,
            ctx,
            "denied",
            Some(reason),
            None,
        )
        .await?;
        Ok(InvokeOutcome::Denied {
            reason: reason.to_string(),
        })
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

/// Hash of everything that shapes an outbound client: transport, command,
/// args, plaintext env/url/headers and the resolved secrets. Secrets are
/// hashed (never stored) so a rotated key replaces the pooled client.
fn client_config_hash(
    server: &McpServerDetail,
    secret_env: &BTreeMap<String, String>,
    secret_headers: &BTreeMap<String, String>,
) -> String {
    let mut h = Sha256::new();
    let mut put = |tag: &str, v: &str| {
        h.update(tag.as_bytes());
        h.update((v.len() as u64).to_le_bytes());
        h.update(v.as_bytes());
    };
    put("transport", &server.transport);
    put("command", &server.command);
    for a in &server.args {
        put("arg", a);
    }
    put("url", server.url.as_deref().unwrap_or(""));
    for (k, v) in &server.env {
        put("env.k", k);
        put("env.v", v);
    }
    for (k, v) in &server.headers {
        put("hdr.k", k);
        put("hdr.v", v);
    }
    for (k, v) in secret_env {
        put("senv.k", k);
        put("senv.v", v);
    }
    for (k, v) in secret_headers {
        put("shdr.k", k);
        put("shdr.v", v);
    }
    hex::encode(h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sweep_probes_http_and_only_recently_used_stdio() {
        let min = Duration::from_secs(60);
        assert_eq!(sweep_action("http", false, None), SweepAction::Probe);
        assert_eq!(sweep_action("stdio", false, None), SweepAction::Skip);
        assert_eq!(
            sweep_action("stdio", false, Some(STDIO_SWEEP_RECENT + min)),
            SweepAction::Skip
        );
        assert_eq!(sweep_action("stdio", false, Some(min)), SweepAction::Probe);
        assert_eq!(
            sweep_action("stdio", true, Some(min)),
            SweepAction::PingLive
        );
    }

    #[test]
    fn canonical_hash_is_key_order_independent() {
        let a = json!({"a":1,"b":[1,2,{"x":1,"y":2}]});
        let b = json!({"b":[1,2,{"y":2,"x":1}],"a":1});
        assert_eq!(canonical_hash(&a), canonical_hash(&b));
        let c = json!({"a":1,"b":[1,2,{"x":1,"y":3}]});
        assert_ne!(canonical_hash(&a), canonical_hash(&c));
    }
}
