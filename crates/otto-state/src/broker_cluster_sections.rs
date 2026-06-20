//! Broker cluster sections repository (user-defined groupings of Kafka cluster
//! profiles within a workspace). Mirrors `connection_sections`, minus the
//! scope column (brokers have a single tree per workspace). Returns a plain row;
//! the `otto-brokers` service maps it to its own domain type.

use chrono::{DateTime, Utc};
use otto_core::{new_id, Id, Result};
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt, ts};

/// A persisted broker-cluster section row.
#[derive(Debug, Clone)]
pub struct BrokerClusterSectionRow {
    pub id: Id,
    pub workspace_id: Id,
    pub parent_id: Option<Id>,
    pub name: String,
    pub position: i64,
    pub created_by: Id,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone)]
pub struct BrokerClusterSectionsRepo {
    pool: SqlitePool,
}

fn row_to_section(r: &sqlx::sqlite::SqliteRow) -> Result<BrokerClusterSectionRow> {
    Ok(BrokerClusterSectionRow {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        parent_id: r.get("parent_id"),
        name: r.get("name"),
        position: r.get("position"),
        created_by: r.get("created_by"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
    })
}

impl BrokerClusterSectionsRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(
        &self,
        ws: &Id,
        parent_id: Option<&str>,
        name: &str,
        created_by: &Id,
    ) -> Result<BrokerClusterSectionRow> {
        let id = new_id();
        let now = fmt(Utc::now());
        // Position is scoped to the sibling group (same workspace + parent).
        let pos: i64 = sqlx::query(
            "SELECT COALESCE(MAX(position) + 1, 0) AS p FROM broker_cluster_sections
             WHERE workspace_id = ? AND parent_id IS ?",
        )
        .bind(ws)
        .bind(parent_id)
        .fetch_one(&self.pool)
        .await
        .map_err(dberr("section position"))?
        .get("p");
        sqlx::query(
            "INSERT INTO broker_cluster_sections (id, workspace_id, parent_id, name, position, created_by, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(ws)
        .bind(parent_id)
        .bind(name)
        .bind(pos)
        .bind(created_by)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create section"))?;
        self.get(&id).await
    }

    pub async fn get(&self, id: &Id) -> Result<BrokerClusterSectionRow> {
        let r = sqlx::query("SELECT * FROM broker_cluster_sections WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("section"))?;
        row_to_section(&r)
    }

    pub async fn list_for_ws(&self, ws: &Id) -> Result<Vec<BrokerClusterSectionRow>> {
        let rows = sqlx::query(
            "SELECT * FROM broker_cluster_sections WHERE workspace_id = ?
             ORDER BY position, name",
        )
        .bind(ws)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("sections"))?;
        rows.iter().map(row_to_section).collect()
    }

    pub async fn rename(&self, id: &Id, name: &str) -> Result<BrokerClusterSectionRow> {
        sqlx::query("UPDATE broker_cluster_sections SET name = ? WHERE id = ?")
            .bind(name)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("rename section"))?;
        self.get(id).await
    }

    /// Reparent a section (None = top-level). Callers must prevent cycles
    /// (moving a section under one of its own descendants).
    pub async fn reparent(
        &self,
        id: &Id,
        parent_id: Option<&str>,
    ) -> Result<BrokerClusterSectionRow> {
        sqlx::query("UPDATE broker_cluster_sections SET parent_id = ? WHERE id = ?")
            .bind(parent_id)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("reparent section"))?;
        self.get(id).await
    }

    /// Delete a section; its clusters fall back to ungrouped (section_id = NULL).
    /// Descendant sections cascade via the FK.
    pub async fn delete(&self, id: &Id) -> Result<()> {
        sqlx::query("UPDATE broker_clusters SET section_id = NULL WHERE section_id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("clear section refs"))?;
        sqlx::query("DELETE FROM broker_cluster_sections WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete section"))?;
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
    async fn create_list_rename_reparent_delete() {
        let pool = mem_pool().await;
        let (ws, user) = seed_ws(&pool).await;
        let repo = BrokerClusterSectionsRepo::new(pool.clone());

        let a = repo.create(&ws, None, "Platform", &user).await.unwrap();
        let b = repo.create(&ws, None, "Staging", &user).await.unwrap();
        assert_eq!(a.position, 0);
        assert_eq!(b.position, 1);

        let aws = repo.create(&ws, Some(&a.id), "AWS", &user).await.unwrap();
        assert_eq!(aws.parent_id.as_deref(), Some(a.id.as_str()));
        assert_eq!(aws.position, 0);

        assert_eq!(repo.list_for_ws(&ws).await.unwrap().len(), 3);

        let renamed = repo.rename(&a.id, "Prod").await.unwrap();
        assert_eq!(renamed.name, "Prod");

        let moved = repo.reparent(&b.id, Some(&a.id)).await.unwrap();
        assert_eq!(moved.parent_id.as_deref(), Some(a.id.as_str()));

        // Deleting "Prod" cascades to its children (AWS + moved Staging).
        repo.delete(&a.id).await.unwrap();
        assert_eq!(repo.list_for_ws(&ws).await.unwrap().len(), 0);
    }
}
