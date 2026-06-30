//! Persistence for Vault v2 **remote backend** configuration — per-workspace
//! Qdrant / SurrealDB / Ollama wiring.
//!
//! Only non-secret connection config lives here (URL, role, enabled, status).
//! Secrets (Qdrant API key, SurrealDB user/password) are stored in the Keychain
//! by reference and resolved at use time, never persisted in SQLite.

use chrono::Utc;
use otto_core::{new_id, Result};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt};

/// One configured remote backend for a workspace.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VaultBackend {
    pub id: String,
    pub workspace_id: String,
    /// `qdrant` | `surreal` | `ollama`.
    pub kind: String,
    pub enabled: bool,
    pub url: String,
    /// Which layer this serves: `vector` | `graph` | `embed`.
    pub role: String,
    #[serde(default)]
    pub config_json: String,
    /// `unknown` | `ok` | `error` | `installing`.
    pub status: String,
    pub message: Option<String>,
    pub updated_at: String,
}

#[derive(Clone)]
pub struct VaultBackendsRepo {
    pool: SqlitePool,
}

impl VaultBackendsRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn list(&self, ws: &str) -> Result<Vec<VaultBackend>> {
        let rows = sqlx::query("SELECT * FROM vault_backends WHERE workspace_id = ? ORDER BY kind ASC")
            .bind(ws)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("vault_backends.list"))?;
        Ok(rows.iter().map(row_to_backend).collect())
    }

    /// All enabled backends across every workspace — for boot-time registration.
    pub async fn all_enabled(&self) -> Result<Vec<VaultBackend>> {
        let rows = sqlx::query("SELECT * FROM vault_backends WHERE enabled = 1")
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("vault_backends.all_enabled"))?;
        Ok(rows.iter().map(row_to_backend).collect())
    }

    pub async fn get(&self, ws: &str, kind: &str) -> Result<Option<VaultBackend>> {
        let r = sqlx::query("SELECT * FROM vault_backends WHERE workspace_id = ? AND kind = ?")
            .bind(ws)
            .bind(kind)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("vault_backends.get"))?;
        Ok(r.as_ref().map(row_to_backend))
    }

    /// Upsert the (ws, kind) backend config.
    pub async fn upsert(
        &self,
        ws: &str,
        kind: &str,
        enabled: bool,
        url: &str,
        role: &str,
        config_json: &str,
    ) -> Result<VaultBackend> {
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO vault_backends (id,workspace_id,kind,enabled,url,role,config_json,status,updated_at) \
             VALUES (?,?,?,?,?,?,?,'unknown',?) \
             ON CONFLICT(workspace_id,kind) DO UPDATE SET \
               enabled=excluded.enabled, url=excluded.url, role=excluded.role, \
               config_json=excluded.config_json, updated_at=excluded.updated_at",
        )
        .bind(new_id())
        .bind(ws)
        .bind(kind)
        .bind(enabled)
        .bind(url)
        .bind(role)
        .bind(config_json)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("vault_backends.upsert"))?;
        Ok(self.get(ws, kind).await?.expect("just upserted"))
    }

    /// Update only the live status + message (health checks / install progress).
    pub async fn set_status(&self, ws: &str, kind: &str, status: &str, message: Option<&str>) -> Result<()> {
        let now = fmt(Utc::now());
        sqlx::query("UPDATE vault_backends SET status=?, message=?, updated_at=? WHERE workspace_id=? AND kind=?")
            .bind(status)
            .bind(message)
            .bind(&now)
            .bind(ws)
            .bind(kind)
            .execute(&self.pool)
            .await
            .map_err(dberr("vault_backends.set_status"))?;
        Ok(())
    }
}

fn row_to_backend(r: &sqlx::sqlite::SqliteRow) -> VaultBackend {
    VaultBackend {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        kind: r.get("kind"),
        enabled: r.get::<i64, _>("enabled") != 0,
        url: r.get("url"),
        role: r.get("role"),
        config_json: r.get("config_json"),
        status: r.get("status"),
        message: r.get("message"),
        updated_at: r.get("updated_at"),
    }
}
