//! Settings key/value repository (JSON values).

use otto_core::Result;
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, json};

/// Settings key for the first-party Otto MCP tool server opt-in (Task B2b).
///
/// The value is a JSON object keyed by workspace id, e.g. `{ "<ws>": true }`;
/// default OFF for every workspace (absent key / absent entry / `false`). A bare
/// scalar `true` is also honored as a global enable. When on for a workspace,
/// agent spawns there inject the read-only `otto` MCP server into `.mcp.json`.
/// Stored through the generic settings KV (`PUT /api/v1/settings`); this constant
/// is the single source of truth for the key name.
pub const OTTO_MCP_ENABLED_KEY: &str = "otto_mcp_enabled";

/// Read the per-workspace `otto_mcp_enabled` flag from a settings value, applying
/// the precedence rules documented on [`OTTO_MCP_ENABLED_KEY`]: a scalar `true`
/// is a global enable; an object is consulted per workspace; anything else is
/// `false`. Pure (no I/O) so it is trivially testable and reusable.
pub fn otto_mcp_enabled_for(value: Option<&serde_json::Value>, workspace_id: &str) -> bool {
    match value {
        Some(serde_json::Value::Bool(b)) => *b,
        Some(serde_json::Value::Object(map)) => {
            map.get(workspace_id).and_then(|v| v.as_bool()).unwrap_or(false)
        }
        _ => false,
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
        // Absent ⇒ off.
        assert!(!otto_mcp_enabled_for(None, "ws1"));
        // Scalar true ⇒ global on.
        assert!(otto_mcp_enabled_for(Some(&json!(true)), "ws1"));
        assert!(!otto_mcp_enabled_for(Some(&json!(false)), "ws1"));
        // Per-workspace object.
        let map = json!({ "ws1": true, "ws2": false });
        assert!(otto_mcp_enabled_for(Some(&map), "ws1"));
        assert!(!otto_mcp_enabled_for(Some(&map), "ws2"));
        // Unknown workspace ⇒ off.
        assert!(!otto_mcp_enabled_for(Some(&map), "ws3"));
        // Wrong shape ⇒ off (fail closed).
        assert!(!otto_mcp_enabled_for(Some(&json!("yes")), "ws1"));
    }
}
