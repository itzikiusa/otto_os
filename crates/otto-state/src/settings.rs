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

/// Settings key for the model that drafts PR titles/descriptions and commit
/// messages (`POST /repos/{id}/pr/draft`, `…/draft-commit-message`).
///
/// These turns are a narrow, mechanical job — read a diff, emit a title and a
/// bullet list — and they block a modal the user is sitting in front of, so the
/// default is the FASTEST model rather than the user's default agent model.
/// Drafting used to inherit the default (Opus) and take minutes. The value is a
/// bare string (`"haiku"`, `"sonnet"`, a full model id…); anything empty or
/// non-string falls back to [`PR_DRAFT_MODEL_DEFAULT`]. Stored through the
/// generic settings KV (`PUT /api/v1/settings`).
pub const PR_DRAFT_MODEL_KEY: &str = "pr_draft_model";

/// Default drafting model — fast, and more than capable of a title + bullets.
pub const PR_DRAFT_MODEL_DEFAULT: &str = "haiku";

/// Resolve the drafting model from a settings value, per
/// [`PR_DRAFT_MODEL_KEY`]. Pure (no I/O) so it is trivially testable.
pub fn pr_draft_model_from(value: Option<&serde_json::Value>) -> String {
    value
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(PR_DRAFT_MODEL_DEFAULT)
        .to_string()
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

    pub async fn delete(&self, key: &str) -> Result<()> {
        sqlx::query("DELETE FROM settings WHERE key = ?")
            .bind(key)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete setting"))?;
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

    #[test]
    fn pr_draft_model_falls_back_to_the_fast_default() {
        // Unset / blank / wrong shape must NEVER fall through to the user's
        // default agent model — inheriting a reasoning model is what made the
        // draft dialog sit for minutes.
        assert_eq!(pr_draft_model_from(None), PR_DRAFT_MODEL_DEFAULT);
        assert_eq!(pr_draft_model_from(Some(&json!(""))), PR_DRAFT_MODEL_DEFAULT);
        assert_eq!(pr_draft_model_from(Some(&json!("   "))), PR_DRAFT_MODEL_DEFAULT);
        assert_eq!(pr_draft_model_from(Some(&json!(true))), PR_DRAFT_MODEL_DEFAULT);
        // An explicit choice wins, whitespace-trimmed.
        assert_eq!(pr_draft_model_from(Some(&json!("sonnet"))), "sonnet");
        assert_eq!(pr_draft_model_from(Some(&json!("  opus  "))), "opus");
    }
}
