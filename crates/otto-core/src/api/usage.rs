//! Usage budgets (spend caps, opt-in enforcement).
//!
//! Split out of `api.rs` by area (re-exported from there, so every path is
//! still `otto_core::api::*`). Mirrored by `ui/src/lib/api/types/core.ts`.
use super::*;

// ---------------------------------------------------------------------------
// Usage budgets (spend caps, opt-in enforcement)
// ---------------------------------------------------------------------------

/// Per-workspace spend cap (USD over the budget window).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkspaceBudget {
    pub workspace_id: Id,
    /// Cap in USD for the window. `0` (or absent) = no cap for this workspace.
    pub monthly_usd: f64,
}

/// Per-provider spend cap (USD over the budget window).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderBudget {
    pub provider: String,
    pub monthly_usd: f64,
}

/// The full usage-budget configuration, persisted under the `usage_budgets`
/// settings key. Enforcement is **opt-in**: `enforce` defaults to `false`, so a
/// budget never blocks or even warns until a root user turns it on.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageBudgetConfig {
    /// Master opt-in flag. When `false`, budgets are purely informational and
    /// the daemon's budget check is a no-op.
    #[serde(default)]
    pub enforce: bool,
    /// When `true` (and `enforce` is on) an exceeded budget is a hard block;
    /// when `false` it only warns prominently. Default `false` (warn-only).
    #[serde(default)]
    pub block_on_exceed: bool,
    /// Window the caps apply to, in days (approximates a calendar month).
    /// Default 30, clamped 1..=3650 at the route.
    #[serde(default)]
    pub window_days: u32,
    #[serde(default)]
    pub workspaces: Vec<WorkspaceBudget>,
    #[serde(default)]
    pub providers: Vec<ProviderBudget>,
}

/// One budget vs. its current spend, as surfaced in the Usage UI.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BudgetStatusRow {
    /// "workspace" or "provider".
    pub scope: String,
    /// The workspace id or provider name.
    pub key: String,
    /// Human label (workspace name, or the provider name) when resolvable.
    #[serde(default)]
    pub label: Option<String>,
    pub limit_usd: f64,
    pub spent_usd: f64,
    /// `spent / limit` (0 when no limit).
    pub used_fraction: f64,
    /// True once spend crosses the warn threshold (80% of the cap).
    pub warning: bool,
    /// True once spend meets/exceeds the cap.
    pub exceeded: bool,
}

/// `GET /api/v1/usage/budgets` response: the config plus its live status rows.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageBudgetStatus {
    pub config: UsageBudgetConfig,
    /// Window actually used to compute spend (after clamping/defaults).
    pub window_days: u32,
    pub rows: Vec<BudgetStatusRow>,
}

/// `PATCH /api/v1/mcp-servers/{id}` — partial update; absent fields are kept.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateMcpServerReq {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Option<Vec<String>>,
    #[serde(default)]
    pub env: Option<std::collections::BTreeMap<String, String>>,
    /// WRITE-ONLY: when present, replaces the server's secret env set (values
    /// to the Keychain, key names to the row). `None` keeps existing secrets.
    #[serde(default)]
    pub secret_env: Option<std::collections::BTreeMap<String, String>>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

/// `POST /api/v1/connections/{id}/test`
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TestConnectionResp {
    pub ok: bool,
    pub latency_ms: Option<u64>,
    pub message: String,
    /// True when the built command unavoidably exposes the secret in argv
    /// (clickhouse-client) — UI shows a warning banner.
    pub warn_argv: bool,
    /// Set when the SSH private key file used by this connection has insecure
    /// (group/other-readable) permissions — OpenSSH may refuse it. Carries a
    /// human-readable message including the exact `chmod 600 <path>` fix. This
    /// warning is independent of whether the test itself succeeded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warn_key_perms: Option<String>,
    /// What to change when `ok` is false and the failure is a recognised one
    /// (auth rejected, host key changed, DNS, refused, timed out, …) — e.g.
    /// "Check the VPN, firewall / security group …". Absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

/// Body of `POST /connections/unsaved/test` — probe an SSH profile's form
/// values before they are saved (nothing is persisted).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestUnsavedConnectionReq {
    /// Authorizes the caller (workspace Editor); the probe itself is global.
    pub workspace_id: Id,
    /// Only `ssh` — database kinds use `/connections/unsaved/db/test`.
    pub kind: ConnectionKind,
    #[serde(default)]
    pub params: serde_json::Value,
}

/// Connection responses use the domain type (secret_ref is opaque).
pub type ConnectionResp = Connection;
