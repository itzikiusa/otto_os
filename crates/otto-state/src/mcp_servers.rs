//! User-managed MCP servers repository (per workspace).
//!
//! Rows here are merged into a workspace's `.mcp.json` (see `otto-sessions::mcp`)
//! when an agent session spawns — but only the *enabled* ones, and only on spawn.
//! Nothing is auto-enabled.

use std::collections::BTreeMap;

use chrono::Utc;
use otto_core::domain::McpServer;
use otto_core::{new_id, Id, Result};
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt, json, ts};

/// Fields for creating a server. `enabled` is the caller's choice (default off
/// at the route layer); the repo never forces it on.
pub struct NewMcpServer {
    pub workspace_id: Id,
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub enabled: bool,
    pub created_by: Id,
}

#[derive(Clone)]
pub struct McpServersRepo {
    pool: SqlitePool,
}

fn row_to_server(r: &sqlx::sqlite::SqliteRow) -> Result<McpServer> {
    let args: Vec<String> = serde_json::from_value(json(&r.get::<String, _>("args_json"))?)
        .unwrap_or_default();
    let env: BTreeMap<String, String> =
        serde_json::from_value(json(&r.get::<String, _>("env_json"))?).unwrap_or_default();
    Ok(McpServer {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        name: r.get("name"),
        command: r.get("command"),
        args,
        env,
        enabled: r.get::<i64, _>("enabled") != 0,
        created_by: r.get("created_by"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
        updated_at: ts(&r.get::<String, _>("updated_at"))?,
    })
}

impl McpServersRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, new: NewMcpServer) -> Result<McpServer> {
        let id = new_id();
        let now = fmt(Utc::now());
        let args = serde_json::to_string(&new.args).unwrap_or_else(|_| "[]".into());
        let env = serde_json::to_string(&new.env).unwrap_or_else(|_| "{}".into());
        sqlx::query(
            "INSERT INTO mcp_servers
               (id, workspace_id, name, command, args_json, env_json, enabled, created_by, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&new.workspace_id)
        .bind(&new.name)
        .bind(&new.command)
        .bind(&args)
        .bind(&env)
        .bind(new.enabled as i64)
        .bind(&new.created_by)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create mcp server"))?;
        self.get(&id).await
    }

    pub async fn get(&self, id: &Id) -> Result<McpServer> {
        let r = sqlx::query("SELECT * FROM mcp_servers WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("mcp server"))?;
        row_to_server(&r)
    }

    pub async fn list_for_ws(&self, ws: &Id) -> Result<Vec<McpServer>> {
        let rows = sqlx::query("SELECT * FROM mcp_servers WHERE workspace_id = ? ORDER BY name")
            .bind(ws)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("mcp servers"))?;
        rows.iter().map(row_to_server).collect()
    }

    /// Only the enabled servers for a workspace — the set merged into `.mcp.json`.
    pub async fn list_enabled(&self, ws: &Id) -> Result<Vec<McpServer>> {
        let rows = sqlx::query(
            "SELECT * FROM mcp_servers WHERE workspace_id = ? AND enabled = 1 ORDER BY name",
        )
        .bind(ws)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("mcp servers"))?;
        rows.iter().map(row_to_server).collect()
    }

    /// Partial update: only the `Some` fields are written; `updated_at` bumps.
    #[allow(clippy::too_many_arguments)]
    pub async fn update(
        &self,
        id: &Id,
        name: Option<&str>,
        command: Option<&str>,
        args: Option<&[String]>,
        env: Option<&BTreeMap<String, String>>,
        enabled: Option<bool>,
    ) -> Result<McpServer> {
        if let Some(v) = name {
            sqlx::query("UPDATE mcp_servers SET name = ? WHERE id = ?")
                .bind(v)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("update mcp server"))?;
        }
        if let Some(v) = command {
            sqlx::query("UPDATE mcp_servers SET command = ? WHERE id = ?")
                .bind(v)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("update mcp server"))?;
        }
        if let Some(v) = args {
            let s = serde_json::to_string(v).unwrap_or_else(|_| "[]".into());
            sqlx::query("UPDATE mcp_servers SET args_json = ? WHERE id = ?")
                .bind(&s)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("update mcp server"))?;
        }
        if let Some(v) = env {
            let s = serde_json::to_string(v).unwrap_or_else(|_| "{}".into());
            sqlx::query("UPDATE mcp_servers SET env_json = ? WHERE id = ?")
                .bind(&s)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("update mcp server"))?;
        }
        if let Some(v) = enabled {
            sqlx::query("UPDATE mcp_servers SET enabled = ? WHERE id = ?")
                .bind(v as i64)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("update mcp server"))?;
        }
        sqlx::query("UPDATE mcp_servers SET updated_at = ? WHERE id = ?")
            .bind(fmt(Utc::now()))
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("update mcp server"))?;
        self.get(id).await
    }

    pub async fn delete(&self, id: &Id) -> Result<()> {
        sqlx::query("DELETE FROM mcp_servers WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete mcp server"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn mem_pool() -> SqlitePool {
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!().run(&pool).await.unwrap();
        pool
    }

    async fn seed_ws(pool: &SqlitePool) -> (Id, Id) {
        let user = new_id();
        let ws = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, display_name, is_root, created_at)
             VALUES (?, ?, ?, ?, 0, ?)",
        )
        .bind(&user)
        .bind("u")
        .bind("x")
        .bind("U")
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO workspaces (id, name, root_path, created_at) VALUES (?, ?, ?, ?)")
            .bind(&ws)
            .bind("w")
            .bind("/tmp")
            .bind(&now)
            .execute(pool)
            .await
            .unwrap();
        (ws, user)
    }

    #[tokio::test]
    async fn create_list_update_enabled_delete() {
        let pool = mem_pool().await;
        let (ws, user) = seed_ws(&pool).await;
        let repo = McpServersRepo::new(pool.clone());

        let mut env = BTreeMap::new();
        env.insert("API_KEY".to_string(), "secret".to_string());
        let s = repo
            .create(NewMcpServer {
                workspace_id: ws.clone(),
                name: "linear".into(),
                command: "npx".into(),
                args: vec!["-y".into(), "@linear/mcp".into()],
                env: env.clone(),
                enabled: false,
                created_by: user.clone(),
            })
            .await
            .unwrap();
        assert_eq!(s.name, "linear");
        assert_eq!(s.args, vec!["-y".to_string(), "@linear/mcp".to_string()]);
        assert_eq!(s.env.get("API_KEY").map(String::as_str), Some("secret"));
        // Default off — never auto-enabled.
        assert!(!s.enabled);

        // Not in the enabled set until flipped on.
        assert!(repo.list_enabled(&ws).await.unwrap().is_empty());
        assert_eq!(repo.list_for_ws(&ws).await.unwrap().len(), 1);

        let s = repo
            .update(&s.id, None, None, None, None, Some(true))
            .await
            .unwrap();
        assert!(s.enabled);
        assert_eq!(repo.list_enabled(&ws).await.unwrap().len(), 1);

        // Rename + retarget args.
        let s = repo
            .update(
                &s.id,
                Some("linear-mcp"),
                None,
                Some(&["-y".into(), "@linear/mcp@latest".into()]),
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(s.name, "linear-mcp");
        assert_eq!(s.args.last().map(String::as_str), Some("@linear/mcp@latest"));

        repo.delete(&s.id).await.unwrap();
        assert!(repo.list_for_ws(&ws).await.unwrap().is_empty());
    }
}
