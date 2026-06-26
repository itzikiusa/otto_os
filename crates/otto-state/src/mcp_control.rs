//! MCP Control Plane persistence: the rich server registry (over the augmented
//! `mcp_servers` columns), the discovered tool catalog (`mcp_tools`), per-workspace
//! allowlists (`mcp_allowlist`), policy-as-code rules (`mcp_policies`), the
//! append-only governed-call audit ledger + per-tool stats (`mcp_call_log`), and
//! the approval queue (`mcp_approvals`).
//!
//! Security-relevant invariants enforced here:
//! - Secret env/header *values* never live in these rows — only their key names
//!   (`secret_env_keys`/`secret_header_keys`) and a keychain `secret_ref`. The
//!   wire structs serialize plaintext config + `has_secret`, never a secret.
//! - Approvals bind to `args_hash` (sha256 of the canonical full arguments) and
//!   are single-use (`consume` flips `status='consumed'` once), so an approved id
//!   cannot be replayed or used with swapped arguments.

use std::collections::BTreeMap;

use chrono::Utc;
use otto_core::{new_id, Id, Result};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, dberr_unique, fmt, json};

// ===========================================================================
// Wire / domain structs
// ===========================================================================

/// Full registry view of an MCP server (the augmented `mcp_servers` row). Secret
/// values are never included — `has_secret` + the `secret_*_keys` name lists are.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerDetail {
    pub id: Id,
    pub workspace_id: Id,
    pub name: String,
    pub transport: String, // 'stdio' | 'http'
    pub command: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>, // plaintext env only
    pub url: Option<String>,
    pub description: Option<String>,
    pub headers: BTreeMap<String, String>, // plaintext headers only
    pub secret_env_keys: Vec<String>,
    pub secret_header_keys: Vec<String>,
    pub has_secret: bool,
    pub injection_risk: String,
    pub managed: bool,
    pub default_tool_access: String, // 'allow' | 'deny'
    pub enabled: bool,
    pub health_status: String,
    pub health_checked_at: Option<String>,
    pub health_latency_ms: Option<i64>,
    pub health_error: Option<String>,
    pub tools_count: i64,
    pub tools_discovered_at: Option<String>,
    pub created_by: Id,
    pub created_at: String,
    pub updated_at: String,
}

/// Fields to create a control-plane server. Secret values are handed to the
/// keychain by the caller (the route layer) before this is built; only the key
/// names land in the row.
pub struct NewServerRow {
    pub workspace_id: Id,
    pub name: String,
    pub transport: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub url: Option<String>,
    pub description: Option<String>,
    pub headers: BTreeMap<String, String>,
    pub secret_ref: Option<String>,
    pub secret_env_keys: Vec<String>,
    pub secret_header_keys: Vec<String>,
    pub injection_risk: String,
    pub default_tool_access: String,
    pub enabled: bool,
    pub created_by: Id,
}

/// A discovered tool plus its governance metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTool {
    pub id: Id,
    pub server_id: Id,
    pub name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub input_schema: serde_json::Value,
    pub annotations: serde_json::Value,
    pub risk_label: String,
    pub injection_risk: String,
    pub mutating: bool,
    pub supports_dry_run: bool,
    pub enabled: bool,
    pub require_approval: bool,
    pub risk_overridden: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// A tool as freshly discovered from a server (pre-persistence).
#[derive(Debug, Clone)]
pub struct DiscoveredTool {
    pub name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub input_schema: serde_json::Value,
    pub annotations: serde_json::Value,
    pub risk_label: String,
    pub injection_risk: String,
    pub mutating: bool,
    pub supports_dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpAllowlistEntry {
    pub id: Id,
    pub workspace_id: Id,
    pub server_id: Id,
    pub tool_name: Option<String>,
    pub mode: String, // 'allow' | 'deny'
    pub created_by: Id,
    pub created_at: String,
}

pub struct NewAllowlistEntry {
    pub server_id: Id,
    pub tool_name: Option<String>,
    pub mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpPolicy {
    pub id: Id,
    pub workspace_id: Option<Id>,
    pub name: String,
    pub enabled: bool,
    pub priority: i64,
    #[serde(rename = "match")]
    pub match_json: serde_json::Value,
    pub effect: String,
    pub reason: Option<String>,
    pub created_by: Id,
    pub created_at: String,
    pub updated_at: String,
}

pub struct NewPolicy {
    pub workspace_id: Option<Id>,
    pub name: String,
    pub enabled: bool,
    pub priority: i64,
    pub match_json: serde_json::Value,
    pub effect: String,
    pub reason: Option<String>,
    pub created_by: Id,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpCallLogRow {
    pub id: Id,
    pub workspace_id: Option<String>,
    pub server_id: Option<String>,
    pub server_name: Option<String>,
    pub tool: String,
    pub direction: String,
    pub caller_user_id: Option<String>,
    pub caller_kind: Option<String>,
    pub args_redacted_json: String,
    pub decision: String,
    pub decision_reason: Option<String>,
    pub risk_label: Option<String>,
    pub injection_risk: Option<String>,
    pub dry_run: bool,
    pub ok: bool,
    pub error: Option<String>,
    pub latency_ms: Option<i64>,
    pub bytes: Option<i64>,
    pub rows: Option<i64>,
    pub approval_id: Option<String>,
    pub created_at: String,
}

/// Input for one audit row. `id`/`created_at` owned by the repo.
#[derive(Debug, Clone, Default)]
pub struct NewCallLog {
    pub workspace_id: Option<String>,
    pub server_id: Option<String>,
    pub server_name: Option<String>,
    pub tool: String,
    pub direction: String,
    pub caller_user_id: Option<String>,
    pub caller_kind: Option<String>,
    pub args_redacted_json: String,
    pub decision: String,
    pub decision_reason: Option<String>,
    pub risk_label: Option<String>,
    pub injection_risk: Option<String>,
    pub dry_run: bool,
    pub ok: bool,
    pub error: Option<String>,
    pub latency_ms: Option<i64>,
    pub bytes: Option<i64>,
    pub rows: Option<i64>,
    pub approval_id: Option<String>,
}

/// Filters for the audit list.
#[derive(Debug, Clone, Default)]
pub struct CallLogQuery {
    pub workspace_ids: Option<Vec<String>>, // None = all (root); Some = restrict
    pub server_id: Option<String>,
    pub tool: Option<String>,
    pub decision: Option<String>,
    pub limit: i64,
    pub offset: i64,
}

/// Per-tool aggregate stats (cost=bytes proxy / latency / error).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolStats {
    pub server_id: Option<String>,
    pub server_name: Option<String>,
    pub tool: String,
    pub calls: i64,
    pub errors: i64,
    pub error_rate: f64,
    pub avg_latency_ms: f64,
    pub max_latency_ms: i64,
    pub total_bytes: i64,
    pub avg_bytes: f64,
    pub last_called_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpApproval {
    pub id: Id,
    pub workspace_id: Option<String>,
    pub kind: String,
    pub server_id: Option<String>,
    pub server_name: Option<String>,
    pub tool: Option<String>,
    pub title: String,
    pub detail: Option<String>,
    pub args_redacted_json: String,
    pub risk_label: Option<String>,
    pub status: String,
    pub requested_by: Option<String>,
    pub requested_by_kind: Option<String>,
    pub decided_by: Option<String>,
    pub decision_note: Option<String>,
    pub created_at: String,
    pub decided_at: Option<String>,
    pub consumed_at: Option<String>,
    pub expires_at: Option<String>,
}

pub struct NewApproval {
    pub workspace_id: Option<String>,
    pub kind: String,
    pub server_id: Option<String>,
    pub server_name: Option<String>,
    pub tool: Option<String>,
    pub title: String,
    pub detail: Option<String>,
    pub args_redacted_json: String,
    pub args_hash: Option<String>,
    pub risk_label: Option<String>,
    pub requested_by: Option<String>,
    pub requested_by_kind: Option<String>,
    pub expires_at: Option<String>,
}

// ===========================================================================
// Registry repo
// ===========================================================================

fn parse_vec(s: &str) -> Vec<String> {
    serde_json::from_str(s).unwrap_or_default()
}
fn parse_map(s: &str) -> BTreeMap<String, String> {
    serde_json::from_str(s).unwrap_or_default()
}

fn row_to_detail(r: &sqlx::sqlite::SqliteRow) -> McpServerDetail {
    let secret_ref: Option<String> = r.get("secret_ref");
    McpServerDetail {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        name: r.get("name"),
        transport: r.get("transport"),
        command: r.get("command"),
        args: parse_vec(&r.get::<String, _>("args_json")),
        env: parse_map(&r.get::<String, _>("env_json")),
        url: r.get("url"),
        description: r.get("description"),
        headers: parse_map(&r.get::<String, _>("headers_json")),
        secret_env_keys: parse_vec(&r.get::<String, _>("secret_env_keys")),
        secret_header_keys: parse_vec(&r.get::<String, _>("secret_header_keys")),
        has_secret: secret_ref.is_some(),
        injection_risk: r.get("injection_risk"),
        managed: r.get::<i64, _>("managed") != 0,
        default_tool_access: r.get("default_tool_access"),
        enabled: r.get::<i64, _>("enabled") != 0,
        health_status: r.get("health_status"),
        health_checked_at: r.get("health_checked_at"),
        health_latency_ms: r.get("health_latency_ms"),
        health_error: r.get("health_error"),
        tools_count: r.get("tools_count"),
        tools_discovered_at: r.get("tools_discovered_at"),
        created_by: r.get("created_by"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

#[derive(Clone)]
pub struct McpRegistryRepo {
    pool: SqlitePool,
}

impl McpRegistryRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, n: NewServerRow) -> Result<McpServerDetail> {
        let id = new_id();
        let now = fmt(Utc::now());
        let args = serde_json::to_string(&n.args).unwrap_or_else(|_| "[]".into());
        let env = serde_json::to_string(&n.env).unwrap_or_else(|_| "{}".into());
        let headers = serde_json::to_string(&n.headers).unwrap_or_else(|_| "{}".into());
        let se_keys = serde_json::to_string(&n.secret_env_keys).unwrap_or_else(|_| "[]".into());
        let sh_keys = serde_json::to_string(&n.secret_header_keys).unwrap_or_else(|_| "[]".into());
        sqlx::query(
            "INSERT INTO mcp_servers
               (id, workspace_id, name, command, args_json, env_json, enabled, created_by,
                created_at, updated_at, transport, url, description, headers_json, secret_ref,
                secret_env_keys, secret_header_keys, injection_risk, managed, default_tool_access)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1, ?)",
        )
        .bind(&id)
        .bind(&n.workspace_id)
        .bind(&n.name)
        .bind(&n.command)
        .bind(&args)
        .bind(&env)
        .bind(n.enabled as i64)
        .bind(&n.created_by)
        .bind(&now)
        .bind(&now)
        .bind(&n.transport)
        .bind(&n.url)
        .bind(&n.description)
        .bind(&headers)
        .bind(&n.secret_ref)
        .bind(&se_keys)
        .bind(&sh_keys)
        .bind(&n.injection_risk)
        .bind(&n.default_tool_access)
        .execute(&self.pool)
        .await
        .map_err(dberr_unique(
            "create mcp server",
            "an MCP server with this name already exists in the workspace",
        ))?;
        self.get(&id).await
    }

    pub async fn get(&self, id: &Id) -> Result<McpServerDetail> {
        let r = sqlx::query("SELECT * FROM mcp_servers WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("mcp server"))?;
        Ok(row_to_detail(&r))
    }

    pub async fn list_for_ws(&self, ws: &Id) -> Result<Vec<McpServerDetail>> {
        let rows = sqlx::query("SELECT * FROM mcp_servers WHERE workspace_id = ? ORDER BY name")
            .bind(ws)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("mcp servers"))?;
        Ok(rows.iter().map(row_to_detail).collect())
    }

    /// All managed servers across all workspaces (for the background health sweep).
    pub async fn list_all_managed(&self) -> Result<Vec<McpServerDetail>> {
        let rows = sqlx::query("SELECT * FROM mcp_servers WHERE managed = 1 ORDER BY name")
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("mcp servers"))?;
        Ok(rows.iter().map(row_to_detail).collect())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn update(
        &self,
        id: &Id,
        name: Option<&str>,
        description: Option<&str>,
        command: Option<&str>,
        args: Option<&[String]>,
        env: Option<&BTreeMap<String, String>>,
        url: Option<&str>,
        headers: Option<&BTreeMap<String, String>>,
        injection_risk: Option<&str>,
        default_tool_access: Option<&str>,
        enabled: Option<bool>,
    ) -> Result<McpServerDetail> {
        macro_rules! set_str {
            ($col:literal, $val:expr) => {
                if let Some(v) = $val {
                    sqlx::query(concat!("UPDATE mcp_servers SET ", $col, " = ? WHERE id = ?"))
                        .bind(v)
                        .bind(id)
                        .execute(&self.pool)
                        .await
                        .map_err(dberr_unique(
                            "update mcp server",
                            "an MCP server with this name already exists in the workspace",
                        ))?;
                }
            };
        }
        set_str!("name", name);
        set_str!("description", description);
        set_str!("command", command);
        set_str!("url", url);
        set_str!("injection_risk", injection_risk);
        set_str!("default_tool_access", default_tool_access);
        if let Some(v) = args {
            let s = serde_json::to_string(v).unwrap_or_else(|_| "[]".into());
            sqlx::query("UPDATE mcp_servers SET args_json = ? WHERE id = ?")
                .bind(&s).bind(id).execute(&self.pool).await
                .map_err(dberr("update mcp server"))?;
        }
        if let Some(v) = env {
            let s = serde_json::to_string(v).unwrap_or_else(|_| "{}".into());
            sqlx::query("UPDATE mcp_servers SET env_json = ? WHERE id = ?")
                .bind(&s).bind(id).execute(&self.pool).await
                .map_err(dberr("update mcp server"))?;
        }
        if let Some(v) = headers {
            let s = serde_json::to_string(v).unwrap_or_else(|_| "{}".into());
            sqlx::query("UPDATE mcp_servers SET headers_json = ? WHERE id = ?")
                .bind(&s).bind(id).execute(&self.pool).await
                .map_err(dberr("update mcp server"))?;
        }
        if let Some(v) = enabled {
            sqlx::query("UPDATE mcp_servers SET enabled = ? WHERE id = ?")
                .bind(v as i64).bind(id).execute(&self.pool).await
                .map_err(dberr("update mcp server"))?;
        }
        sqlx::query("UPDATE mcp_servers SET updated_at = ? WHERE id = ?")
            .bind(fmt(Utc::now())).bind(id).execute(&self.pool).await
            .map_err(dberr("update mcp server"))?;
        self.get(id).await
    }

    /// Replace the keychain secret-key name lists + ref (after writing keychain).
    pub async fn set_secret_meta(
        &self,
        id: &Id,
        secret_ref: Option<&str>,
        env_keys: &[String],
        header_keys: &[String],
    ) -> Result<()> {
        let ek = serde_json::to_string(env_keys).unwrap_or_else(|_| "[]".into());
        let hk = serde_json::to_string(header_keys).unwrap_or_else(|_| "[]".into());
        sqlx::query(
            "UPDATE mcp_servers SET secret_ref = ?, secret_env_keys = ?, secret_header_keys = ?, updated_at = ? WHERE id = ?",
        )
        .bind(secret_ref).bind(&ek).bind(&hk).bind(fmt(Utc::now())).bind(id)
        .execute(&self.pool).await.map_err(dberr("update mcp secret meta"))?;
        Ok(())
    }

    pub async fn set_health(
        &self,
        id: &Id,
        status: &str,
        latency_ms: Option<i64>,
        error: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE mcp_servers SET health_status = ?, health_checked_at = ?, health_latency_ms = ?, health_error = ? WHERE id = ?",
        )
        .bind(status).bind(fmt(Utc::now())).bind(latency_ms).bind(error).bind(id)
        .execute(&self.pool).await.map_err(dberr("set mcp health"))?;
        Ok(())
    }

    pub async fn set_tools_meta(&self, id: &Id, count: i64) -> Result<()> {
        sqlx::query(
            "UPDATE mcp_servers SET tools_count = ?, tools_discovered_at = ? WHERE id = ?",
        )
        .bind(count).bind(fmt(Utc::now())).bind(id)
        .execute(&self.pool).await.map_err(dberr("set mcp tools meta"))?;
        Ok(())
    }

    pub async fn delete(&self, id: &Id) -> Result<()> {
        sqlx::query("DELETE FROM mcp_servers WHERE id = ?")
            .bind(id).execute(&self.pool).await
            .map_err(dberr("delete mcp server"))?;
        Ok(())
    }
}

// ===========================================================================
// Tools catalog repo
// ===========================================================================

fn row_to_tool(r: &sqlx::sqlite::SqliteRow) -> Result<McpTool> {
    Ok(McpTool {
        id: r.get("id"),
        server_id: r.get("server_id"),
        name: r.get("name"),
        title: r.get("title"),
        description: r.get("description"),
        input_schema: json(&r.get::<String, _>("input_schema_json"))?,
        annotations: json(&r.get::<String, _>("annotations_json"))?,
        risk_label: r.get("risk_label"),
        injection_risk: r.get("injection_risk"),
        mutating: r.get::<i64, _>("mutating") != 0,
        supports_dry_run: r.get::<i64, _>("supports_dry_run") != 0,
        enabled: r.get::<i64, _>("enabled") != 0,
        require_approval: r.get::<i64, _>("require_approval") != 0,
        risk_overridden: r.get::<i64, _>("risk_overridden") != 0,
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    })
}

#[derive(Clone)]
pub struct McpToolsRepo {
    pool: SqlitePool,
}

impl McpToolsRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Upsert a freshly discovered catalog. New tools are inserted; existing ones
    /// have schema/description refreshed, but a human-pinned (`risk_overridden`)
    /// tool keeps its risk/injection labels. Tools no longer advertised are left
    /// in place (so their per-tool permission survives a transient discovery).
    pub async fn upsert_discovered(&self, server_id: &Id, tools: &[DiscoveredTool]) -> Result<()> {
        let now = fmt(Utc::now());
        for t in tools {
            let existing = sqlx::query(
                "SELECT id, risk_overridden FROM mcp_tools WHERE server_id = ? AND name = ?",
            )
            .bind(server_id)
            .bind(&t.name)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("lookup mcp tool"))?;
            let schema = serde_json::to_string(&t.input_schema).unwrap_or_else(|_| "{}".into());
            let annot = serde_json::to_string(&t.annotations).unwrap_or_else(|_| "{}".into());
            match existing {
                Some(row) => {
                    let id: String = row.get("id");
                    let overridden = row.get::<i64, _>("risk_overridden") != 0;
                    if overridden {
                        sqlx::query(
                            "UPDATE mcp_tools SET title=?, description=?, input_schema_json=?, annotations_json=?, mutating=?, supports_dry_run=?, updated_at=? WHERE id=?",
                        )
                        .bind(&t.title).bind(&t.description).bind(&schema).bind(&annot)
                        .bind(t.mutating as i64).bind(t.supports_dry_run as i64).bind(&now).bind(&id)
                        .execute(&self.pool).await.map_err(dberr("update mcp tool"))?;
                    } else {
                        sqlx::query(
                            "UPDATE mcp_tools SET title=?, description=?, input_schema_json=?, annotations_json=?, risk_label=?, injection_risk=?, mutating=?, supports_dry_run=?, updated_at=? WHERE id=?",
                        )
                        .bind(&t.title).bind(&t.description).bind(&schema).bind(&annot)
                        .bind(&t.risk_label).bind(&t.injection_risk).bind(t.mutating as i64)
                        .bind(t.supports_dry_run as i64).bind(&now).bind(&id)
                        .execute(&self.pool).await.map_err(dberr("update mcp tool"))?;
                    }
                }
                None => {
                    sqlx::query(
                        "INSERT INTO mcp_tools (id, server_id, name, title, description, input_schema_json, annotations_json, risk_label, injection_risk, mutating, supports_dry_run, enabled, require_approval, risk_overridden, created_at, updated_at)
                         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1, ?, 0, ?, ?)",
                    )
                    .bind(new_id()).bind(server_id).bind(&t.name).bind(&t.title).bind(&t.description)
                    .bind(&schema).bind(&annot).bind(&t.risk_label).bind(&t.injection_risk)
                    .bind(t.mutating as i64).bind(t.supports_dry_run as i64)
                    // Dangerous tools default to require_approval on first discovery.
                    .bind((t.risk_label == "dangerous") as i64)
                    .bind(&now).bind(&now)
                    .execute(&self.pool).await.map_err(dberr("insert mcp tool"))?;
                }
            }
        }
        Ok(())
    }

    pub async fn list_for_server(&self, server_id: &Id) -> Result<Vec<McpTool>> {
        let rows = sqlx::query("SELECT * FROM mcp_tools WHERE server_id = ? ORDER BY name")
            .bind(server_id)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("mcp tools"))?;
        rows.iter().map(row_to_tool).collect()
    }

    pub async fn get(&self, id: &Id) -> Result<McpTool> {
        let r = sqlx::query("SELECT * FROM mcp_tools WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("mcp tool"))?;
        row_to_tool(&r)
    }

    pub async fn get_by_name(&self, server_id: &Id, name: &str) -> Result<McpTool> {
        let r = sqlx::query("SELECT * FROM mcp_tools WHERE server_id = ? AND name = ?")
            .bind(server_id)
            .bind(name)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("mcp tool"))?;
        row_to_tool(&r)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn patch(
        &self,
        id: &Id,
        enabled: Option<bool>,
        require_approval: Option<bool>,
        risk_label: Option<&str>,
        injection_risk: Option<&str>,
    ) -> Result<McpTool> {
        if let Some(v) = enabled {
            sqlx::query("UPDATE mcp_tools SET enabled = ? WHERE id = ?")
                .bind(v as i64).bind(id).execute(&self.pool).await
                .map_err(dberr("patch mcp tool"))?;
        }
        if let Some(v) = require_approval {
            sqlx::query("UPDATE mcp_tools SET require_approval = ? WHERE id = ?")
                .bind(v as i64).bind(id).execute(&self.pool).await
                .map_err(dberr("patch mcp tool"))?;
        }
        // A human setting risk/injection pins it (risk_overridden=1) so rediscovery
        // never lowers it.
        if let Some(v) = risk_label {
            sqlx::query("UPDATE mcp_tools SET risk_label = ?, risk_overridden = 1 WHERE id = ?")
                .bind(v).bind(id).execute(&self.pool).await
                .map_err(dberr("patch mcp tool"))?;
        }
        if let Some(v) = injection_risk {
            sqlx::query("UPDATE mcp_tools SET injection_risk = ?, risk_overridden = 1 WHERE id = ?")
                .bind(v).bind(id).execute(&self.pool).await
                .map_err(dberr("patch mcp tool"))?;
        }
        sqlx::query("UPDATE mcp_tools SET updated_at = ? WHERE id = ?")
            .bind(fmt(Utc::now())).bind(id).execute(&self.pool).await
            .map_err(dberr("patch mcp tool"))?;
        self.get(id).await
    }
}

// ===========================================================================
// Allowlist repo
// ===========================================================================

#[derive(Clone)]
pub struct McpAllowlistRepo {
    pool: SqlitePool,
}

impl McpAllowlistRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn list_for_ws(&self, ws: &Id) -> Result<Vec<McpAllowlistEntry>> {
        let rows = sqlx::query("SELECT * FROM mcp_allowlist WHERE workspace_id = ?")
            .bind(ws)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("mcp allowlist"))?;
        Ok(rows
            .iter()
            .map(|r| McpAllowlistEntry {
                id: r.get("id"),
                workspace_id: r.get("workspace_id"),
                server_id: r.get("server_id"),
                tool_name: r.get("tool_name"),
                mode: r.get("mode"),
                created_by: r.get("created_by"),
                created_at: r.get("created_at"),
            })
            .collect())
    }

    /// Replace the entire allowlist for a workspace (bulk set from the UI).
    pub async fn replace_for_ws(
        &self,
        ws: &Id,
        entries: &[NewAllowlistEntry],
        created_by: &Id,
    ) -> Result<()> {
        let now = fmt(Utc::now());
        let mut tx = self.pool.begin().await.map_err(dberr("allowlist tx"))?;
        sqlx::query("DELETE FROM mcp_allowlist WHERE workspace_id = ?")
            .bind(ws)
            .execute(&mut *tx)
            .await
            .map_err(dberr("clear allowlist"))?;
        for e in entries {
            sqlx::query(
                "INSERT INTO mcp_allowlist (id, workspace_id, server_id, tool_name, mode, created_by, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(new_id()).bind(ws).bind(&e.server_id).bind(&e.tool_name)
            .bind(&e.mode).bind(created_by).bind(&now)
            .execute(&mut *tx)
            .await
            .map_err(dberr("insert allowlist"))?;
        }
        tx.commit().await.map_err(dberr("commit allowlist"))?;
        Ok(())
    }

    /// Resolve the effective allow/deny for (ws, server, tool). A `deny` (tool- or
    /// server-scoped) wins; else a matching `allow`; else `None` (caller falls back
    /// to the server's `default_tool_access`). Returns the matched mode.
    pub async fn resolve(
        &self,
        ws: &Id,
        server_id: &Id,
        tool: &str,
    ) -> Result<Option<String>> {
        let rows = sqlx::query(
            "SELECT tool_name, mode FROM mcp_allowlist WHERE workspace_id = ? AND server_id = ?",
        )
        .bind(ws)
        .bind(server_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("resolve allowlist"))?;
        let mut allow = false;
        for r in &rows {
            let t: Option<String> = r.get("tool_name");
            let mode: String = r.get("mode");
            let matches = t.as_deref().map(|n| n == tool).unwrap_or(true); // None = whole server
            if matches {
                if mode == "deny" {
                    return Ok(Some("deny".into())); // deny wins immediately
                }
                if mode == "allow" {
                    allow = true;
                }
            }
        }
        Ok(if allow { Some("allow".into()) } else { None })
    }
}

// ===========================================================================
// Policy repo
// ===========================================================================

fn row_to_policy(r: &sqlx::sqlite::SqliteRow) -> Result<McpPolicy> {
    Ok(McpPolicy {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        name: r.get("name"),
        enabled: r.get::<i64, _>("enabled") != 0,
        priority: r.get("priority"),
        match_json: json(&r.get::<String, _>("match_json"))?,
        effect: r.get("effect"),
        reason: r.get("reason"),
        created_by: r.get("created_by"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    })
}

#[derive(Clone)]
pub struct McpPolicyRepo {
    pool: SqlitePool,
}

impl McpPolicyRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, n: NewPolicy) -> Result<McpPolicy> {
        let id = new_id();
        let now = fmt(Utc::now());
        let m = serde_json::to_string(&n.match_json).unwrap_or_else(|_| "{}".into());
        sqlx::query(
            "INSERT INTO mcp_policies (id, workspace_id, name, enabled, priority, match_json, effect, reason, created_by, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id).bind(&n.workspace_id).bind(&n.name).bind(n.enabled as i64)
        .bind(n.priority).bind(&m).bind(&n.effect).bind(&n.reason).bind(&n.created_by)
        .bind(&now).bind(&now)
        .execute(&self.pool).await.map_err(dberr("create policy"))?;
        self.get(&id).await
    }

    pub async fn get(&self, id: &Id) -> Result<McpPolicy> {
        let r = sqlx::query("SELECT * FROM mcp_policies WHERE id = ?")
            .bind(id).fetch_one(&self.pool).await.map_err(dberr("policy"))?;
        row_to_policy(&r)
    }

    /// Global policies (`workspace_id IS NULL`) plus, when `ws` is given, that
    /// workspace's rules. Ordered by priority for display.
    pub async fn list(&self, ws: Option<&Id>) -> Result<Vec<McpPolicy>> {
        let rows = match ws {
            Some(w) => sqlx::query(
                "SELECT * FROM mcp_policies WHERE workspace_id IS NULL OR workspace_id = ? ORDER BY priority, created_at",
            )
            .bind(w)
            .fetch_all(&self.pool)
            .await,
            None => sqlx::query("SELECT * FROM mcp_policies ORDER BY priority, created_at")
                .fetch_all(&self.pool)
                .await,
        }
        .map_err(dberr("policies"))?;
        rows.iter().map(row_to_policy).collect()
    }

    /// Rules that apply to a workspace's evaluation: global + that ws.
    pub async fn list_applicable(&self, ws: &str) -> Result<Vec<McpPolicy>> {
        let rows = sqlx::query(
            "SELECT * FROM mcp_policies WHERE enabled = 1 AND (workspace_id IS NULL OR workspace_id = ?) ORDER BY priority, created_at",
        )
        .bind(ws)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("policies"))?;
        rows.iter().map(row_to_policy).collect()
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn update(
        &self,
        id: &Id,
        name: Option<&str>,
        enabled: Option<bool>,
        priority: Option<i64>,
        match_json: Option<&serde_json::Value>,
        effect: Option<&str>,
        reason: Option<&str>,
    ) -> Result<McpPolicy> {
        if let Some(v) = name {
            sqlx::query("UPDATE mcp_policies SET name = ? WHERE id = ?").bind(v).bind(id)
                .execute(&self.pool).await.map_err(dberr("update policy"))?;
        }
        if let Some(v) = enabled {
            sqlx::query("UPDATE mcp_policies SET enabled = ? WHERE id = ?").bind(v as i64).bind(id)
                .execute(&self.pool).await.map_err(dberr("update policy"))?;
        }
        if let Some(v) = priority {
            sqlx::query("UPDATE mcp_policies SET priority = ? WHERE id = ?").bind(v).bind(id)
                .execute(&self.pool).await.map_err(dberr("update policy"))?;
        }
        if let Some(v) = match_json {
            let s = serde_json::to_string(v).unwrap_or_else(|_| "{}".into());
            sqlx::query("UPDATE mcp_policies SET match_json = ? WHERE id = ?").bind(&s).bind(id)
                .execute(&self.pool).await.map_err(dberr("update policy"))?;
        }
        if let Some(v) = effect {
            sqlx::query("UPDATE mcp_policies SET effect = ? WHERE id = ?").bind(v).bind(id)
                .execute(&self.pool).await.map_err(dberr("update policy"))?;
        }
        if let Some(v) = reason {
            sqlx::query("UPDATE mcp_policies SET reason = ? WHERE id = ?").bind(v).bind(id)
                .execute(&self.pool).await.map_err(dberr("update policy"))?;
        }
        sqlx::query("UPDATE mcp_policies SET updated_at = ? WHERE id = ?")
            .bind(fmt(Utc::now())).bind(id).execute(&self.pool).await
            .map_err(dberr("update policy"))?;
        self.get(id).await
    }

    pub async fn delete(&self, id: &Id) -> Result<()> {
        sqlx::query("DELETE FROM mcp_policies WHERE id = ?")
            .bind(id).execute(&self.pool).await.map_err(dberr("delete policy"))?;
        Ok(())
    }
}

// ===========================================================================
// Call log (audit) repo + stats
// ===========================================================================

fn row_to_call_log(r: &sqlx::sqlite::SqliteRow) -> McpCallLogRow {
    McpCallLogRow {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        server_id: r.get("server_id"),
        server_name: r.get("server_name"),
        tool: r.get("tool"),
        direction: r.get("direction"),
        caller_user_id: r.get("caller_user_id"),
        caller_kind: r.get("caller_kind"),
        args_redacted_json: r.get("args_redacted_json"),
        decision: r.get("decision"),
        decision_reason: r.get("decision_reason"),
        risk_label: r.get("risk_label"),
        injection_risk: r.get("injection_risk"),
        dry_run: r.get::<i64, _>("dry_run") != 0,
        ok: r.get::<i64, _>("ok") != 0,
        error: r.get("error"),
        latency_ms: r.get("latency_ms"),
        bytes: r.get("bytes"),
        rows: r.get("rows"),
        approval_id: r.get("approval_id"),
        created_at: r.get("created_at"),
    }
}

#[derive(Clone)]
pub struct McpCallLogRepo {
    pool: SqlitePool,
}

impl McpCallLogRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn insert(&self, e: NewCallLog) -> Result<String> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO mcp_call_log (id, workspace_id, server_id, server_name, tool, direction, caller_user_id, caller_kind, args_redacted_json, decision, decision_reason, risk_label, injection_risk, dry_run, ok, error, latency_ms, bytes, rows, approval_id, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id).bind(&e.workspace_id).bind(&e.server_id).bind(&e.server_name)
        .bind(&e.tool).bind(&e.direction).bind(&e.caller_user_id).bind(&e.caller_kind)
        .bind(&e.args_redacted_json).bind(&e.decision).bind(&e.decision_reason)
        .bind(&e.risk_label).bind(&e.injection_risk).bind(e.dry_run as i64).bind(e.ok as i64)
        .bind(&e.error).bind(e.latency_ms).bind(e.bytes).bind(e.rows).bind(&e.approval_id)
        .bind(&now)
        .execute(&self.pool).await.map_err(dberr("insert call log"))?;
        Ok(id)
    }

    /// Update an audit row with the execution outcome. Used by the invoke
    /// pipeline's fail-closed pattern: the row is inserted *before* the tool
    /// runs (so an audit-insert failure aborts the call), then finalized here.
    #[allow(clippy::too_many_arguments)]
    pub async fn finalize(
        &self,
        id: &str,
        ok: bool,
        error: Option<&str>,
        latency_ms: Option<i64>,
        bytes: Option<i64>,
        rows: Option<i64>,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE mcp_call_log SET ok = ?, error = ?, latency_ms = ?, bytes = ?, rows = ? WHERE id = ?",
        )
        .bind(ok as i64)
        .bind(error)
        .bind(latency_ms)
        .bind(bytes)
        .bind(rows)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("finalize call log"))?;
        Ok(())
    }

    pub async fn list(&self, q: &CallLogQuery) -> Result<Vec<McpCallLogRow>> {
        // Build a parameterized query honoring the workspace restriction (so a
        // non-root caller only sees logs for workspaces they can access).
        let mut sql = String::from("SELECT * FROM mcp_call_log WHERE 1=1");
        if let Some(ids) = &q.workspace_ids {
            if ids.is_empty() {
                return Ok(vec![]); // no accessible workspaces => nothing
            }
            let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            sql.push_str(&format!(
                " AND (workspace_id IN ({placeholders}) OR workspace_id IS NULL)"
            ));
        }
        if q.server_id.is_some() {
            sql.push_str(" AND server_id = ?");
        }
        if q.tool.is_some() {
            sql.push_str(" AND tool = ?");
        }
        if q.decision.is_some() {
            sql.push_str(" AND decision = ?");
        }
        sql.push_str(" ORDER BY created_at DESC LIMIT ? OFFSET ?");
        let mut query = sqlx::query(&sql);
        if let Some(ids) = &q.workspace_ids {
            for id in ids {
                query = query.bind(id);
            }
        }
        if let Some(v) = &q.server_id {
            query = query.bind(v);
        }
        if let Some(v) = &q.tool {
            query = query.bind(v);
        }
        if let Some(v) = &q.decision {
            query = query.bind(v);
        }
        let limit = if q.limit <= 0 { 200 } else { q.limit.min(1000) };
        query = query.bind(limit).bind(q.offset.max(0));
        let rows = query.fetch_all(&self.pool).await.map_err(dberr("call log list"))?;
        Ok(rows.iter().map(row_to_call_log).collect())
    }

    /// Per-tool aggregates over the executed (non-denied) calls.
    pub async fn stats(&self, workspace_ids: Option<&[String]>) -> Result<Vec<McpToolStats>> {
        let mut sql = String::from(
            "SELECT server_id, server_name, tool,
                    COUNT(*) AS calls,
                    SUM(CASE WHEN ok = 0 THEN 1 ELSE 0 END) AS errors,
                    AVG(COALESCE(latency_ms,0)) AS avg_latency,
                    MAX(COALESCE(latency_ms,0)) AS max_latency,
                    SUM(COALESCE(bytes,0)) AS total_bytes,
                    AVG(COALESCE(bytes,0)) AS avg_bytes,
                    MAX(created_at) AS last_called
             FROM mcp_call_log
             WHERE decision != 'denied' AND decision != 'pending_approval'",
        );
        if let Some(ids) = workspace_ids {
            if ids.is_empty() {
                return Ok(vec![]);
            }
            let ph = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            sql.push_str(&format!(" AND (workspace_id IN ({ph}) OR workspace_id IS NULL)"));
        }
        sql.push_str(" GROUP BY server_id, tool ORDER BY calls DESC");
        let mut query = sqlx::query(&sql);
        if let Some(ids) = workspace_ids {
            for id in ids {
                query = query.bind(id);
            }
        }
        let rows = query.fetch_all(&self.pool).await.map_err(dberr("call log stats"))?;
        Ok(rows
            .iter()
            .map(|r| {
                let calls: i64 = r.get("calls");
                let errors: i64 = r.get("errors");
                McpToolStats {
                    server_id: r.get("server_id"),
                    server_name: r.get("server_name"),
                    tool: r.get("tool"),
                    calls,
                    errors,
                    error_rate: if calls > 0 { errors as f64 / calls as f64 } else { 0.0 },
                    avg_latency_ms: r.get::<f64, _>("avg_latency"),
                    max_latency_ms: r.get::<i64, _>("max_latency"),
                    total_bytes: r.get::<i64, _>("total_bytes"),
                    avg_bytes: r.get::<f64, _>("avg_bytes"),
                    last_called_at: r.get("last_called"),
                }
            })
            .collect())
    }
}

// ===========================================================================
// Approval queue repo
// ===========================================================================

fn row_to_approval(r: &sqlx::sqlite::SqliteRow) -> McpApproval {
    McpApproval {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        kind: r.get("kind"),
        server_id: r.get("server_id"),
        server_name: r.get("server_name"),
        tool: r.get("tool"),
        title: r.get("title"),
        detail: r.get("detail"),
        args_redacted_json: r.get("args_redacted_json"),
        risk_label: r.get("risk_label"),
        status: r.get("status"),
        requested_by: r.get("requested_by"),
        requested_by_kind: r.get("requested_by_kind"),
        decided_by: r.get("decided_by"),
        decision_note: r.get("decision_note"),
        created_at: r.get("created_at"),
        decided_at: r.get("decided_at"),
        consumed_at: r.get("consumed_at"),
        expires_at: r.get("expires_at"),
    }
}

#[derive(Clone)]
pub struct McpApprovalRepo {
    pool: SqlitePool,
}

impl McpApprovalRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, n: NewApproval) -> Result<McpApproval> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO mcp_approvals (id, workspace_id, kind, server_id, server_name, tool, title, detail, args_redacted_json, args_hash, risk_label, status, requested_by, requested_by_kind, created_at, expires_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'pending', ?, ?, ?, ?)",
        )
        .bind(&id).bind(&n.workspace_id).bind(&n.kind).bind(&n.server_id).bind(&n.server_name)
        .bind(&n.tool).bind(&n.title).bind(&n.detail).bind(&n.args_redacted_json).bind(&n.args_hash)
        .bind(&n.risk_label).bind(&n.requested_by).bind(&n.requested_by_kind).bind(&now)
        .bind(&n.expires_at)
        .execute(&self.pool).await.map_err(dberr("create approval"))?;
        self.get(&id).await
    }

    pub async fn get(&self, id: &Id) -> Result<McpApproval> {
        let r = sqlx::query("SELECT * FROM mcp_approvals WHERE id = ?")
            .bind(id).fetch_one(&self.pool).await.map_err(dberr("approval"))?;
        Ok(row_to_approval(&r))
    }

    pub async fn list(
        &self,
        workspace_ids: Option<&[String]>,
        status: Option<&str>,
        limit: i64,
    ) -> Result<Vec<McpApproval>> {
        let mut sql = String::from("SELECT * FROM mcp_approvals WHERE 1=1");
        if let Some(ids) = workspace_ids {
            if ids.is_empty() {
                return Ok(vec![]);
            }
            let ph = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            sql.push_str(&format!(" AND (workspace_id IN ({ph}) OR workspace_id IS NULL)"));
        }
        if status.is_some() {
            sql.push_str(" AND status = ?");
        }
        sql.push_str(" ORDER BY created_at DESC LIMIT ?");
        let mut query = sqlx::query(&sql);
        if let Some(ids) = workspace_ids {
            for id in ids {
                query = query.bind(id);
            }
        }
        if let Some(s) = status {
            query = query.bind(s);
        }
        query = query.bind(if limit <= 0 { 200 } else { limit.min(1000) });
        let rows = query.fetch_all(&self.pool).await.map_err(dberr("approvals"))?;
        Ok(rows.iter().map(row_to_approval).collect())
    }

    /// Decide an approval. Enforces: still pending, and approver != requester
    /// (separation of duties). Returns the updated row.
    pub async fn decide(
        &self,
        id: &Id,
        approved: bool,
        decided_by: &str,
        note: Option<&str>,
    ) -> Result<McpApproval> {
        let cur = self.get(id).await?;
        if cur.status != "pending" {
            return Err(otto_core::Error::Conflict(format!(
                "approval is already {}",
                cur.status
            )));
        }
        if cur.requested_by.as_deref() == Some(decided_by) {
            return Err(otto_core::Error::Invalid(
                "the requester cannot approve their own request (separation of duties)".into(),
            ));
        }
        let status = if approved { "approved" } else { "denied" };
        sqlx::query(
            "UPDATE mcp_approvals SET status = ?, decided_by = ?, decision_note = ?, decided_at = ? WHERE id = ?",
        )
        .bind(status).bind(decided_by).bind(note).bind(fmt(Utc::now())).bind(id)
        .execute(&self.pool).await.map_err(dberr("decide approval"))?;
        self.get(id).await
    }

    /// Find an approved-and-unconsumed approval that binds to exactly this call:
    /// same (server, tool, workspace), matching `args_hash`, not expired. Used by
    /// the invoke gate. Returns the approval id if a usable one exists.
    pub async fn find_usable(
        &self,
        workspace_id: Option<&str>,
        server_id: Option<&str>,
        tool: &str,
        args_hash: &str,
    ) -> Result<Option<String>> {
        let now = fmt(Utc::now());
        let r = sqlx::query(
            "SELECT id FROM mcp_approvals
             WHERE status = 'approved' AND consumed_at IS NULL
               AND tool = ? AND args_hash = ?
               AND (server_id IS ? OR server_id = ?)
               AND (workspace_id IS ? OR workspace_id = ?)
               AND (expires_at IS NULL OR expires_at > ?)
             ORDER BY decided_at DESC LIMIT 1",
        )
        .bind(tool)
        .bind(args_hash)
        .bind(server_id)
        .bind(server_id)
        .bind(workspace_id)
        .bind(workspace_id)
        .bind(&now)
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("find approval"))?;
        Ok(r.map(|row| row.get::<String, _>("id")))
    }

    /// Mark an approval consumed (single-use). Returns true if it transitioned
    /// from `approved` to `consumed` (false if it was already consumed — a replay).
    pub async fn consume(&self, id: &str) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE mcp_approvals SET status = 'consumed', consumed_at = ? WHERE id = ? AND status = 'approved' AND consumed_at IS NULL",
        )
        .bind(fmt(Utc::now()))
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("consume approval"))?;
        Ok(res.rows_affected() == 1)
    }

    /// Expire pending approvals past their `expires_at` (best-effort housekeeping).
    pub async fn expire_stale(&self) -> Result<u64> {
        let now = fmt(Utc::now());
        let res = sqlx::query(
            "UPDATE mcp_approvals SET status = 'expired' WHERE status = 'pending' AND expires_at IS NOT NULL AND expires_at <= ?",
        )
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("expire approvals"))?;
        Ok(res.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

    async fn mem_pool() -> SqlitePool {
        let opts = SqliteConnectOptions::new().in_memory(true).foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    async fn seed(pool: &SqlitePool) -> (Id, Id) {
        let user = new_id();
        let ws = new_id();
        let now = fmt(Utc::now());
        sqlx::query("INSERT INTO users (id, username, password_hash, display_name, is_root, created_at) VALUES (?, 'u', 'x', 'U', 0, ?)")
            .bind(&user).bind(&now).execute(pool).await.unwrap();
        sqlx::query("INSERT INTO workspaces (id, name, root_path, created_at) VALUES (?, 'w', '/tmp', ?)")
            .bind(&ws).bind(&now).execute(pool).await.unwrap();
        (ws, user)
    }

    async fn mk_server(pool: &SqlitePool, ws: &Id, user: &Id) -> McpServerDetail {
        let repo = McpRegistryRepo::new(pool.clone());
        repo.create(NewServerRow {
            workspace_id: ws.clone(),
            name: "test".into(),
            transport: "stdio".into(),
            command: "echo".into(),
            args: vec!["hi".into()],
            env: BTreeMap::new(),
            url: None,
            description: None,
            headers: BTreeMap::new(),
            secret_ref: None,
            secret_env_keys: vec![],
            secret_header_keys: vec![],
            injection_risk: "low".into(),
            default_tool_access: "allow".into(),
            enabled: true,
            created_by: user.clone(),
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn registry_create_and_augmented_cols() {
        let pool = mem_pool().await;
        let (ws, user) = seed(&pool).await;
        let s = mk_server(&pool, &ws, &user).await;
        assert_eq!(s.transport, "stdio");
        assert_eq!(s.health_status, "unknown");
        assert!(s.managed);
        assert!(!s.has_secret);
        // Legacy reader still maps the row fine (additive columns ignored).
        let legacy = crate::mcp_servers::McpServersRepo::new(pool.clone())
            .get(&s.id)
            .await
            .unwrap();
        assert_eq!(legacy.name, "test");
    }

    #[tokio::test]
    async fn allowlist_deny_beats_allow() {
        let pool = mem_pool().await;
        let (ws, user) = seed(&pool).await;
        let s = mk_server(&pool, &ws, &user).await;
        let repo = McpAllowlistRepo::new(pool.clone());
        repo.replace_for_ws(
            &ws,
            &[
                NewAllowlistEntry { server_id: s.id.clone(), tool_name: None, mode: "allow".into() },
                NewAllowlistEntry { server_id: s.id.clone(), tool_name: Some("danger".into()), mode: "deny".into() },
            ],
            &user,
        )
        .await
        .unwrap();
        assert_eq!(repo.resolve(&ws, &s.id, "safe").await.unwrap().as_deref(), Some("allow"));
        assert_eq!(repo.resolve(&ws, &s.id, "danger").await.unwrap().as_deref(), Some("deny"));
    }

    #[tokio::test]
    async fn approval_single_use_and_args_binding() {
        let pool = mem_pool().await;
        let (ws, _user) = seed(&pool).await;
        let repo = McpApprovalRepo::new(pool.clone());
        let a = repo
            .create(NewApproval {
                workspace_id: Some(ws.clone()),
                kind: "tool_call".into(),
                server_id: Some("srv1".into()),
                server_name: Some("srv".into()),
                tool: Some("delete_thing".into()),
                title: "delete_thing".into(),
                detail: None,
                args_redacted_json: "{}".into(),
                args_hash: Some("HASH_A".into()),
                risk_label: Some("dangerous".into()),
                requested_by: Some("requester".into()),
                requested_by_kind: Some("agent".into()),
                expires_at: None,
            })
            .await
            .unwrap();

        // Requester cannot self-approve.
        assert!(repo.decide(&a.id, true, "requester", None).await.is_err());
        // A different user approves.
        let decided = repo.decide(&a.id, true, "approver", Some("ok")).await.unwrap();
        assert_eq!(decided.status, "approved");

        // Gate finds it only for the exact (tool, args_hash, server, ws).
        let found = repo
            .find_usable(Some(&ws), Some("srv1"), "delete_thing", "HASH_A")
            .await
            .unwrap();
        assert!(found.is_some());
        // Swapped args => not usable.
        assert!(repo
            .find_usable(Some(&ws), Some("srv1"), "delete_thing", "HASH_B")
            .await
            .unwrap()
            .is_none());

        // Single-use: first consume succeeds, replay fails.
        assert!(repo.consume(&a.id).await.unwrap());
        assert!(!repo.consume(&a.id).await.unwrap());
        // After consume the gate no longer finds it.
        assert!(repo
            .find_usable(Some(&ws), Some("srv1"), "delete_thing", "HASH_A")
            .await
            .unwrap()
            .is_none());
    }
}
