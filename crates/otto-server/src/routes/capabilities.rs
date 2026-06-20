//! Daemon capability & health registry + downloadable support bundle.
//!
//! Routes:
//!   GET /api/v1/capabilities    → Vec<ModuleCapability> (root only)
//!   GET /api/v1/support-bundle  → SupportBundle JSON (root only)
//!
//! Both routes are read-only aggregators: they gate with `require_root` (like
//! `/audit-log` and `/security-posture`) and never touch mutable state.
//!
//! ## Signal sources (all read-only)
//! - **providers / CLIs**: `ctx.manager.providers().names()` + `which` PATH check
//! - **MCP servers**: `McpServersRepo::list_for_ws` across all workspaces
//! - **Channels**: `IntegrationsRepo::list` across all workspaces
//! - **Git accounts**: `GitStore::list_all_accounts`
//! - **Issue accounts**: `IssuesRepo::list_all_accounts`
//! - **DB / broker connections**: `ConnectionsRepo::list_visible` + `BrokerClustersRepo::list_visible`
//! - **LSP servers**: `crate::lsp::servers::detect_all` (sync, sub-ms)
//! - **Settings redaction**: `otto_core::redact::redact_json` strips secrets before
//!   they leave the trust boundary

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::routing::get;
use axum::Json;
use axum::Router;
use otto_core::redact::redact_json;
use otto_state::{
    AuditRepo, BrokerClustersRepo, ConnectionsRepo, IntegrationsRepo, McpServersRepo, SettingsRepo,
};
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tokio::sync::Mutex;

use crate::auth::{require_root, CurrentUser};
use crate::error::ApiResult;
use crate::state::ServerCtx;

// ---------------------------------------------------------------------------
// DTOs (module-local; intentionally not added to otto-core's api.rs per spec)
// ---------------------------------------------------------------------------

/// A dependency signal: one external service, tool, or account this module
/// depends on, and whether it was detected as usable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityDep {
    /// Coarse kind: "provider" | "cli" | "lsp" | "mcp" | "channel" | "git"
    ///              | "issue" | "db" | "broker"
    pub kind: String,
    /// Human-readable name (provider/CLI name, account handle, connection name, …).
    pub name: String,
    /// Whether the dep is considered usable (on PATH / has token refs / configured).
    pub ok: bool,
    /// Optional detail — binary path, account provider, connection kind, …
    pub detail: Option<String>,
}

/// Status value for a capability module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityStatus {
    /// All required deps are present and usable.
    Ready,
    /// One or more deps are present but some expected dep is degraded.
    Degraded,
    /// Not set up at all (no accounts, no CLI, etc.).
    MissingSetup,
}

impl CapabilityStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Degraded => "degraded",
            Self::MissingSetup => "missing_setup",
        }
    }
}

/// One module's capability snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleCapability {
    /// Feature slug, e.g. "sessions", "git", "channels", "brokers", "lsp".
    pub feature: String,
    /// "ready" | "degraded" | "missing_setup"
    pub status: String,
    /// Human-readable reasons (e.g. "claude not found on PATH").
    pub reasons: Vec<String>,
    /// Actionable fix instructions matching the reasons.
    pub fixes: Vec<String>,
    /// Per-dep breakdown.
    pub deps: Vec<CapabilityDep>,
}

/// Support bundle: everything a user would attach to a bug report, with all
/// secrets stripped via `redact_json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupportBundle {
    /// Daemon version (CARGO_PKG_VERSION).
    pub version: String,
    /// All settings with secrets replaced by "[redacted]".
    pub settings: serde_json::Value,
    /// The capability snapshot (same as GET /capabilities).
    pub capabilities: Vec<ModuleCapability>,
    /// The 25 most-recent audit log entries (newest first).
    pub recent_audit: Vec<serde_json::Value>,
    /// Current SQLite migration version (number of applied migrations).
    pub migration_level: i64,
    /// Number of redaction hits applied to settings.
    pub redaction_hits: usize,
}

// ---------------------------------------------------------------------------
// Short-lived cache (5 s) so repeated page loads don't re-probe PATH
// ---------------------------------------------------------------------------

struct CacheEntry {
    value: Vec<ModuleCapability>,
    born: Instant,
}

static CACHE: OnceLock<Mutex<Option<CacheEntry>>> = OnceLock::new();

fn cache() -> &'static Mutex<Option<CacheEntry>> {
    CACHE.get_or_init(|| Mutex::new(None))
}

const CACHE_TTL: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Synchronous PATH check (no subprocess). Returns the resolved path or None.
fn which_sync(name: &str) -> Option<String> {
    let path_var = std::env::var("PATH").unwrap_or_default();
    for dir in path_var.split(':') {
        let candidate = std::path::Path::new(dir).join(name);
        if candidate.is_file() {
            return Some(candidate.to_string_lossy().into_owned());
        }
    }
    None
}

/// Async PATH check with a tight timeout (for providers whose binary may be a
/// shell alias or wrapper; we just confirm it exists on disk).
async fn which_async(name: &str) -> bool {
    // First: fast synchronous check (no subprocess needed for disk presence).
    if which_sync(name).is_some() {
        return true;
    }
    // Fallback: `which` subprocess (catches shell builtins / PATH wrappers).
    matches!(
        tokio::time::timeout(
            Duration::from_secs(2),
            Command::new("which").arg(name).output(),
        )
        .await,
        Ok(Ok(o)) if o.status.success()
    )
}

fn status_from_deps(deps: &[CapabilityDep]) -> CapabilityStatus {
    if deps.iter().all(|d| d.ok) {
        CapabilityStatus::Ready
    } else if deps.iter().any(|d| d.ok) {
        CapabilityStatus::Degraded
    } else {
        CapabilityStatus::MissingSetup
    }
}

// ---------------------------------------------------------------------------
// Per-feature capability builders
// ---------------------------------------------------------------------------

/// **Sessions / agent providers** — each registered provider name checked via PATH.
async fn cap_sessions(ctx: &ServerCtx) -> ModuleCapability {
    let names = ctx.manager.providers().names();
    let mut deps = Vec::new();
    let mut reasons = Vec::new();
    let mut fixes = Vec::new();

    if names.is_empty() {
        reasons.push("No agent providers are registered.".into());
        fixes.push("Add a provider in Settings → Providers.".into());
    }

    for name in &names {
        let binary = ctx.manager.providers().program_for(name);
        let binary = binary.as_deref().unwrap_or(name.as_str());
        let found = which_async(binary).await;
        let path = which_sync(binary).map(|p| p.to_string());
        if !found {
            reasons.push(format!("Agent CLI '{name}' ({binary}) not found on PATH."));
            fixes.push(format!("Install '{name}' or update its PATH in Settings → Providers."));
        }
        deps.push(CapabilityDep {
            kind: "provider".into(),
            name: name.clone(),
            ok: found,
            detail: path,
        });
    }

    let status = if deps.is_empty() {
        CapabilityStatus::MissingSetup
    } else {
        status_from_deps(&deps)
    };

    ModuleCapability {
        feature: "sessions".into(),
        status: status.as_str().into(),
        reasons,
        fixes,
        deps,
    }
}

/// **LSP language servers** — sync PATH check only (no subprocess).
fn cap_lsp() -> ModuleCapability {
    let resolved = crate::lsp::servers::detect_all();
    let mut reasons = Vec::new();
    let mut fixes = Vec::new();
    let mut deps: Vec<CapabilityDep> = Vec::new();

    for r in &resolved {
        if !r.available {
            reasons.push(format!("LSP server for '{}' not found on PATH.", r.lang));
            if let Some(cmd) = &r.install_command {
                fixes.push(format!("Run: {cmd}"));
            } else {
                fixes.push(format!(
                    "Install a language server for '{}' and add it to PATH.",
                    r.lang
                ));
            }
        }
        deps.push(CapabilityDep {
            kind: "lsp".into(),
            name: r.lang.clone(),
            ok: r.available,
            detail: if r.available { Some(r.command.clone()) } else { None },
        });
    }

    let status = if deps.is_empty() {
        CapabilityStatus::MissingSetup
    } else if reasons.is_empty() {
        CapabilityStatus::Ready
    } else if deps.iter().any(|d| d.ok) {
        CapabilityStatus::Degraded
    } else {
        CapabilityStatus::MissingSetup
    };

    ModuleCapability {
        feature: "lsp".into(),
        status: status.as_str().into(),
        reasons,
        fixes,
        deps,
    }
}

/// **MCP servers** — whether any are configured across all workspaces.
async fn cap_mcp(ctx: &ServerCtx) -> ModuleCapability {
    let workspaces = ctx.workspaces.list_all().await.unwrap_or_default();
    let mcp_repo = McpServersRepo::new(ctx.pool.clone());
    let mut deps: Vec<CapabilityDep> = Vec::new();
    let mut reasons = Vec::new();
    let mut fixes = Vec::new();

    for ws in &workspaces {
        let servers = mcp_repo.list_for_ws(&ws.id).await.unwrap_or_default();
        for s in servers {
            // An MCP server is "ok" if it's enabled. We don't spawn it to
            // verify liveness here — that would be expensive.
            deps.push(CapabilityDep {
                kind: "mcp".into(),
                name: s.name.clone(),
                ok: s.enabled,
                detail: Some(format!("workspace:{}", ws.name)),
            });
            if !s.enabled {
                reasons.push(format!("MCP server '{}' is configured but disabled.", s.name));
                fixes.push(format!(
                    "Enable '{}' in Settings → MCP Servers.",
                    s.name
                ));
            }
        }
    }

    let status = if deps.is_empty() {
        // No MCP servers configured — that's fine, it's an optional feature.
        CapabilityStatus::MissingSetup
    } else {
        status_from_deps(&deps)
    };

    ModuleCapability {
        feature: "mcp".into(),
        status: status.as_str().into(),
        reasons,
        fixes,
        deps,
    }
}

/// **Channels** (Slack / Telegram) — configured & enabled integrations.
async fn cap_channels(ctx: &ServerCtx) -> ModuleCapability {
    let workspaces = ctx.workspaces.list_all().await.unwrap_or_default();
    let integrations_repo = IntegrationsRepo::new(ctx.pool.clone());
    let mut deps: Vec<CapabilityDep> = Vec::new();
    let mut reasons = Vec::new();
    let mut fixes = Vec::new();

    for ws in &workspaces {
        let integrations = integrations_repo.list(&ws.id).await.unwrap_or_default();
        for integ in integrations {
            // An integration is "ok" if it's enabled AND has its required token refs.
            let ok = integ.enabled
                && integ.has_bot_token
                && (integ.channel.as_str() != "slack" || integ.has_app_token);
            let channel_label = format!("{}/{}", integ.channel.as_str(), ws.name);

            if !ok {
                if !integ.has_bot_token {
                    reasons.push(format!(
                        "Channel '{}' in workspace '{}' is missing a bot token.",
                        integ.channel.as_str(),
                        ws.name
                    ));
                    fixes.push(format!(
                        "Add a bot token for '{}' in Settings → Channels.",
                        integ.channel.as_str()
                    ));
                } else if !integ.enabled {
                    reasons.push(format!(
                        "Channel '{}' in workspace '{}' is disabled.",
                        integ.channel.as_str(),
                        ws.name
                    ));
                    fixes.push(format!(
                        "Enable '{}' in Settings → Channels.",
                        integ.channel.as_str()
                    ));
                }
            }

            deps.push(CapabilityDep {
                kind: "channel".into(),
                name: channel_label,
                ok,
                detail: Some(integ.channel.as_str().to_string()),
            });
        }
    }

    let status = if deps.is_empty() {
        CapabilityStatus::MissingSetup
    } else {
        status_from_deps(&deps)
    };

    ModuleCapability {
        feature: "channels".into(),
        status: status.as_str().into(),
        reasons,
        fixes,
        deps,
    }
}

/// **Git** — configured git provider accounts.
async fn cap_git(ctx: &ServerCtx) -> ModuleCapability {
    let accounts = ctx.git_store.list_all_accounts().await.unwrap_or_default();
    let mut deps: Vec<CapabilityDep> = Vec::new();
    let mut reasons = Vec::new();
    let mut fixes = Vec::new();

    if accounts.is_empty() {
        reasons.push("No git accounts are configured.".into());
        fixes.push("Add a GitHub or Bitbucket account in Settings → Git.".into());
    }

    for acct in &accounts {
        // Accounts are "ok" when they exist — liveness not re-verified here
        // (the stored token ref indicates a credential was saved).
        let has_token = !acct.token_ref.is_empty();
        if !has_token {
            reasons.push(format!(
                "Git account '{}' ({:?}) is missing a token.",
                acct.username,
                acct.provider
            ));
            fixes.push(format!(
                "Re-authenticate '{}' in Settings → Git.",
                acct.username
            ));
        }
        deps.push(CapabilityDep {
            kind: "git".into(),
            name: acct.username.clone(),
            ok: has_token,
            detail: Some(format!("{:?}", acct.provider)),
        });
    }

    let status = if deps.is_empty() {
        CapabilityStatus::MissingSetup
    } else {
        status_from_deps(&deps)
    };

    ModuleCapability {
        feature: "git".into(),
        status: status.as_str().into(),
        reasons,
        fixes,
        deps,
    }
}

/// **Issue trackers** (Jira / Confluence) — configured issue accounts.
async fn cap_issues(ctx: &ServerCtx) -> ModuleCapability {
    let accounts = ctx.issues_store.list_all_accounts().await.unwrap_or_default();
    let mut deps: Vec<CapabilityDep> = Vec::new();
    let mut reasons = Vec::new();
    let mut fixes = Vec::new();

    if accounts.is_empty() {
        reasons.push("No issue-tracker accounts are configured.".into());
        fixes.push("Add a Jira or Confluence account in Settings → Integrations.".into());
    }

    for acct in &accounts {
        let has_token = !acct.token_ref.is_empty();
        if !has_token {
            reasons.push(format!(
                "Issue account '{}' ({:?}) is missing a token.",
                acct.base_url,
                acct.provider
            ));
            fixes.push(format!(
                "Re-authenticate '{}' in Settings → Integrations.",
                acct.base_url
            ));
        }
        deps.push(CapabilityDep {
            kind: "issue".into(),
            name: acct.base_url.clone(),
            ok: has_token,
            detail: Some(format!("{:?}", acct.provider)),
        });
    }

    let status = if deps.is_empty() {
        CapabilityStatus::MissingSetup
    } else {
        status_from_deps(&deps)
    };

    ModuleCapability {
        feature: "issues".into(),
        status: status.as_str().into(),
        reasons,
        fixes,
        deps,
    }
}

/// **DB connections** — configured database / SSH connection profiles.
async fn cap_db(ctx: &ServerCtx) -> ModuleCapability {
    let workspaces = ctx.workspaces.list_all().await.unwrap_or_default();
    let conn_repo = ConnectionsRepo::new(ctx.pool.clone());
    let mut deps: Vec<CapabilityDep> = Vec::new();
    let mut reasons = Vec::new();
    let mut fixes = Vec::new();

    for ws in &workspaces {
        let conns = conn_repo.list_visible(&ws.id).await.unwrap_or_default();
        for c in conns {
            // Connections are "ok" when they exist (we don't ping each one).
            // Flag those missing a secret ref when the kind typically needs one.
            let needs_secret = !matches!(
                c.kind,
                otto_core::domain::ConnectionKind::Custom
            );
            let ok = !needs_secret || c.secret_ref.as_deref().is_some_and(|s| !s.is_empty());
            if !ok {
                reasons.push(format!(
                    "Connection '{}' ({}) may be missing credentials.",
                    c.name,
                    c.kind.as_str()
                ));
                fixes.push(format!(
                    "Edit '{}' in Connections and add credentials.",
                    c.name
                ));
            }
            deps.push(CapabilityDep {
                kind: "db".into(),
                name: c.name.clone(),
                ok,
                detail: Some(c.kind.as_str().to_string()),
            });
        }
    }

    let status = if deps.is_empty() {
        CapabilityStatus::MissingSetup
    } else {
        status_from_deps(&deps)
    };

    ModuleCapability {
        feature: "db".into(),
        status: status.as_str().into(),
        reasons,
        fixes,
        deps,
    }
}

/// **Message brokers** (Kafka) — configured broker cluster profiles.
async fn cap_brokers(ctx: &ServerCtx) -> ModuleCapability {
    let workspaces = ctx.workspaces.list_all().await.unwrap_or_default();
    let broker_repo = BrokerClustersRepo::new(ctx.pool.clone());
    let mut deps: Vec<CapabilityDep> = Vec::new();
    let mut reasons = Vec::new();
    let mut fixes = Vec::new();

    for ws in &workspaces {
        let clusters = broker_repo.list_visible(&ws.id).await.unwrap_or_default();
        for c in clusters {
            // A cluster is "ok" when it has at least one broker address configured.
            let ok = !c.bootstrap_servers.trim().is_empty();
            if !ok {
                reasons.push(format!(
                    "Broker cluster '{}' has no bootstrap servers configured.",
                    c.name
                ));
                fixes.push(format!(
                    "Edit '{}' in Brokers and add broker addresses.",
                    c.name
                ));
            }
            deps.push(CapabilityDep {
                kind: "broker".into(),
                name: c.name.clone(),
                ok,
                detail: Some(c.security_protocol.clone()),
            });
        }
    }

    let status = if deps.is_empty() {
        CapabilityStatus::MissingSetup
    } else {
        status_from_deps(&deps)
    };

    ModuleCapability {
        feature: "brokers".into(),
        status: status.as_str().into(),
        reasons,
        fixes,
        deps,
    }
}

// ---------------------------------------------------------------------------
// Aggregate
// ---------------------------------------------------------------------------

async fn build_capabilities(ctx: &ServerCtx) -> Vec<ModuleCapability> {
    // Fan-out: the async builders run sequentially here to avoid holding
    // ctx across a `join!` macro boundary (ctx is &, not Arc inside the fut).
    // Sub-ms for config-presence checks; no live TCP probes.
    let sessions = cap_sessions(ctx).await;
    let lsp = cap_lsp();
    let mcp = cap_mcp(ctx).await;
    let channels = cap_channels(ctx).await;
    let git = cap_git(ctx).await;
    let issues = cap_issues(ctx).await;
    let db = cap_db(ctx).await;
    let brokers = cap_brokers(ctx).await;

    vec![sessions, lsp, mcp, channels, git, issues, db, brokers]
}

// ---------------------------------------------------------------------------
// Route handlers
// ---------------------------------------------------------------------------

/// `GET /api/v1/capabilities` (root only)
///
/// Returns a snapshot of every Otto module's health: what deps it needs,
/// which are present, and how to fix missing ones. Cached for 5 s to avoid
/// repeated PATH scans on successive page loads.
pub async fn get_capabilities(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<ModuleCapability>>> {
    require_root(&user)?;

    let mut guard = cache().lock().await;
    if let Some(entry) = guard.as_ref() {
        if entry.born.elapsed() < CACHE_TTL {
            return Ok(Json(entry.value.clone()));
        }
    }

    let caps = build_capabilities(&ctx).await;
    *guard = Some(CacheEntry { value: caps.clone(), born: Instant::now() });
    Ok(Json(caps))
}

/// `GET /api/v1/support-bundle` (root only)
///
/// Returns a single JSON object the user can attach to a bug report:
/// - daemon version + migration level
/// - all daemon settings with secrets stripped by `redact_json`
/// - a full capability snapshot (same as GET /capabilities)
/// - the 25 most-recent audit log entries
///
/// Everything sensitive is passed through `otto_core::redact::redact_json`
/// before it leaves the trust boundary.
pub async fn get_support_bundle(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<SupportBundle>> {
    require_root(&user)?;

    // ---- settings (redacted) -----------------------------------------------
    let settings_raw = SettingsRepo::new(ctx.pool.clone())
        .all()
        .await
        .unwrap_or_default();
    let settings_value = serde_json::Value::Object(settings_raw);
    let redacted = redact_json(&settings_value);
    let redaction_hits: usize = redacted.hits.iter().map(|h| h.count).sum();

    // ---- capabilities snapshot ---------------------------------------------
    let capabilities = build_capabilities(&ctx).await;

    // ---- audit log (last 25, newest first) ---------------------------------
    let audit_repo = AuditRepo::new(ctx.pool.clone());
    let audit_entries = audit_repo
        .list(&otto_core::api::AuditLogQuery {
            limit: Some(25),
            ..Default::default()
        })
        .await
        .unwrap_or_default();
    let recent_audit: Vec<serde_json::Value> = audit_entries
        .into_iter()
        .map(|e| serde_json::to_value(e).unwrap_or(serde_json::Value::Null))
        .collect();

    // ---- migration level ---------------------------------------------------
    // Count of applied migrations as a proxy for schema version.
    let migration_level: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM _sqlx_migrations WHERE success = TRUE",
    )
    .fetch_one(&ctx.pool)
    .await
    .unwrap_or(0);

    Ok(Json(SupportBundle {
        version: ctx.version.clone(),
        settings: redacted.value,
        capabilities,
        recent_audit,
        migration_level,
        redaction_hits,
    }))
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Capability + support-bundle routes: mounted as api_extras in `module_routers`.
pub fn capabilities_routes() -> Router<ServerCtx> {
    Router::new()
        .route("/capabilities", get(get_capabilities))
        .route("/support-bundle", get(get_support_bundle))
}
