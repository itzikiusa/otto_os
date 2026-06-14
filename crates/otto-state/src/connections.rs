//! Connections repository.

use chrono::Utc;
use otto_core::domain::{Connection, ConnectionKind};
use otto_core::{new_id, Error, Id, Result};
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt, json, ts};

#[derive(Clone)]
pub struct ConnectionsRepo {
    pool: SqlitePool,
}

pub struct NewConnection {
    pub workspace_id: Option<Id>,
    pub name: String,
    pub kind: ConnectionKind,
    pub params: serde_json::Value,
    pub secret_ref: Option<String>,
    pub first_command: Option<String>,
    pub section_id: Option<Id>,
    pub created_by: Id,
}

fn row_to_connection(r: &sqlx::sqlite::SqliteRow) -> Result<Connection> {
    Ok(Connection {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        name: r.get("name"),
        kind: ConnectionKind::parse(&r.get::<String, _>("kind"))
            .ok_or_else(|| Error::Internal("bad connection kind".into()))?,
        params: json(&r.get::<String, _>("params_json"))?,
        secret_ref: r.get("secret_ref"),
        first_command: r.get("first_command"),
        section_id: r.get("section_id"),
        created_by: r.get("created_by"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
    })
}

impl ConnectionsRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, c: NewConnection) -> Result<Connection> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO connections (id, workspace_id, name, kind, params_json, secret_ref,
                                      first_command, section_id, created_by, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&c.workspace_id)
        .bind(&c.name)
        .bind(c.kind.as_str())
        .bind(c.params.to_string())
        .bind(&c.secret_ref)
        .bind(&c.first_command)
        .bind(&c.section_id)
        .bind(&c.created_by)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create connection"))?;
        self.get(&id).await
    }

    pub async fn get(&self, id: &Id) -> Result<Connection> {
        let r = sqlx::query("SELECT * FROM connections WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("connection"))?;
        row_to_connection(&r)
    }

    /// Connections visible to a workspace: its own plus global (NULL workspace).
    pub async fn list_visible(&self, ws: &Id) -> Result<Vec<Connection>> {
        let rows = sqlx::query(
            "SELECT * FROM connections WHERE workspace_id = ? OR workspace_id IS NULL ORDER BY name",
        )
        .bind(ws)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("connections"))?;
        rows.iter().map(row_to_connection).collect()
    }

    pub async fn update(
        &self,
        id: &Id,
        name: Option<&str>,
        params: Option<&serde_json::Value>,
        secret_ref: Option<Option<&str>>,
        first_command: Option<Option<&str>>,
        section_id: Option<Option<&str>>,
    ) -> Result<Connection> {
        if let Some(v) = name {
            sqlx::query("UPDATE connections SET name = ? WHERE id = ?")
                .bind(v)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("update connection"))?;
        }
        if let Some(v) = params {
            sqlx::query("UPDATE connections SET params_json = ? WHERE id = ?")
                .bind(v.to_string())
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("update connection"))?;
        }
        if let Some(v) = secret_ref {
            sqlx::query("UPDATE connections SET secret_ref = ? WHERE id = ?")
                .bind(v)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("update connection"))?;
        }
        if let Some(v) = first_command {
            sqlx::query("UPDATE connections SET first_command = ? WHERE id = ?")
                .bind(v)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("update connection"))?;
        }
        if let Some(v) = section_id {
            sqlx::query("UPDATE connections SET section_id = ? WHERE id = ?")
                .bind(v)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("update connection"))?;
        }
        self.get(id).await
    }

    pub async fn delete(&self, id: &Id) -> Result<()> {
        sqlx::query("DELETE FROM connections WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete connection"))?;
        Ok(())
    }
}
