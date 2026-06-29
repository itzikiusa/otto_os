//! Settings key/value repository (JSON values).

use otto_core::Result;
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, json};

/// Settings key for the first-party Otto MCP tool server (the `otto` server that
/// exposes Otto's read-only tools — including the DB connection tools — to an
/// agent session).
///
/// **Default ON for every workspace.** The `otto` MCP server is attached to every
/// agent session unless the user explicitly turns it off. The value is normally a
/// bare scalar (`true`/`false`) written by the settings toggle. A JSON object keyed
/// by workspace id (`{ "<ws>": false }`) is also honored for per-workspace
/// overrides; an unlisted workspace falls back to the default-ON. When on for a
/// workspace, agent spawns there inject the `otto` MCP server (into `.mcp.json` for
/// Claude, via `-c` overrides for Codex). Stored through the generic settings KV
/// (`PUT /api/v1/settings`); this constant is the single source of truth for the key.
pub const OTTO_MCP_ENABLED_KEY: &str = "otto_mcp_enabled";

/// Read the per-workspace `otto_mcp_enabled` flag from a settings value, applying
/// the precedence rules documented on [`OTTO_MCP_ENABLED_KEY`]: a scalar bool is
/// the global toggle; an object is consulted per workspace (explicit entry wins,
/// unlisted falls back to the default); **everything else — including an absent
/// value — is ON.** The server is opt-out, not opt-in. Pure (no I/O) so it is
/// trivially testable and reusable.
pub fn otto_mcp_enabled_for(value: Option<&serde_json::Value>, workspace_id: &str) -> bool {
    match value {
        Some(serde_json::Value::Bool(b)) => *b,
        Some(serde_json::Value::Object(map)) => {
            // Explicit per-workspace entry wins; unlisted ⇒ default ON.
            map.get(workspace_id).and_then(|v| v.as_bool()).unwrap_or(true)
        }
        // Absent / malformed ⇒ default ON (attach to every session).
        _ => true,
    }
}

#[derive(Clone)]
pub struct SettingsRepo {
    pool: SqlitePool,
}

impl SettingsRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn get(&self, key: &str) -> Result<Option<serde_json::Value>> {
        let row = sqlx::query("SELECT value_json FROM settings WHERE key = ?")
            .bind(key)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("setting"))?;
        row.map(|r| json(&r.get::<String, _>("value_json")))
            .transpose()
    }

    pub async fn put(&self, key: &str, value: &serde_json::Value) -> Result<()> {
        sqlx::query(
            "INSERT INTO settings (key, value_json) VALUES (?, ?)
             ON CONFLICT (key) DO UPDATE SET value_json = excluded.value_json",
        )
        .bind(key)
        .bind(value.to_string())
        .execute(&self.pool)
        .await
        .map_err(dberr("put setting"))?;
        Ok(())
    }

    pub async fn all(&self) -> Result<serde_json::Map<String, serde_json::Value>> {
        let rows = sqlx::query("SELECT key, value_json FROM settings")
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("settings"))?;
        let mut map = serde_json::Map::new();
        for r in rows {
            map.insert(
                r.get::<String, _>("key"),
                json(&r.get::<String, _>("value_json"))?,
            );
        }
        Ok(map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn otto_mcp_enabled_precedence() {
        // Absent ⇒ ON (the server is attached to every session by default).
        assert!(otto_mcp_enabled_for(None, "ws1"));
        // Scalar toggles the global default.
        assert!(otto_mcp_enabled_for(Some(&json!(true)), "ws1"));
        assert!(!otto_mcp_enabled_for(Some(&json!(false)), "ws1"));
        // Per-workspace object: an explicit entry wins; an unlisted workspace
        // falls back to the default-ON.
        let map = json!({ "ws1": true, "ws2": false });
        assert!(otto_mcp_enabled_for(Some(&map), "ws1"));
        assert!(!otto_mcp_enabled_for(Some(&map), "ws2"));
        // Unlisted workspace ⇒ ON (default).
        assert!(otto_mcp_enabled_for(Some(&map), "ws3"));
        // Wrong shape ⇒ ON (best-effort attach; the toggle is the off switch).
        assert!(otto_mcp_enabled_for(Some(&json!("yes")), "ws1"));
    }
}
