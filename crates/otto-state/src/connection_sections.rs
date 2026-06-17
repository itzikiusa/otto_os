//! Connection sections repository (user-defined groupings of connections).

use chrono::Utc;
use otto_core::domain::ConnectionSection;
use otto_core::{new_id, Id, Result};
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt, ts};

#[derive(Clone)]
pub struct ConnectionSectionsRepo {
    pool: SqlitePool,
}

fn row_to_section(r: &sqlx::sqlite::SqliteRow) -> Result<ConnectionSection> {
    Ok(ConnectionSection {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        parent_id: r.get("parent_id"),
        name: r.get("name"),
        position: r.get("position"),
        scope: r.get("scope"),
        created_by: r.get("created_by"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
    })
}

impl ConnectionSectionsRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(
        &self,
        ws: &Id,
        parent_id: Option<&str>,
        name: &str,
        scope: &str,
        created_by: &Id,
    ) -> Result<ConnectionSection> {
        let id = new_id();
        let now = fmt(Utc::now());
        // Position is scoped to the sibling group (same workspace + parent + scope).
        let pos: i64 = sqlx::query(
            "SELECT COALESCE(MAX(position) + 1, 0) AS p FROM connection_sections
             WHERE workspace_id = ? AND parent_id IS ? AND scope = ?",
        )
        .bind(ws)
        .bind(parent_id)
        .bind(scope)
        .fetch_one(&self.pool)
        .await
        .map_err(dberr("section position"))?
        .get("p");
        sqlx::query(
            "INSERT INTO connection_sections (id, workspace_id, parent_id, name, position, scope, created_by, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(ws)
        .bind(parent_id)
        .bind(name)
        .bind(pos)
        .bind(scope)
        .bind(created_by)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create section"))?;
        self.get(&id).await
    }

    /// Reparent a section (None = make it top-level). Callers must prevent
    /// cycles (moving a section under one of its own descendants).
    pub async fn reparent(&self, id: &Id, parent_id: Option<&str>) -> Result<ConnectionSection> {
        sqlx::query("UPDATE connection_sections SET parent_id = ? WHERE id = ?")
            .bind(parent_id)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("reparent section"))?;
        self.get(id).await
    }

    pub async fn get(&self, id: &Id) -> Result<ConnectionSection> {
        let r = sqlx::query("SELECT * FROM connection_sections WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("section"))?;
        row_to_section(&r)
    }

    pub async fn list_for_ws(&self, ws: &Id, scope: &str) -> Result<Vec<ConnectionSection>> {
        let rows = sqlx::query(
            "SELECT * FROM connection_sections WHERE workspace_id = ? AND scope = ?
             ORDER BY position, name",
        )
        .bind(ws)
        .bind(scope)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("sections"))?;
        rows.iter().map(row_to_section).collect()
    }

    /// Every section, across all workspaces and scopes — the single global tree
    /// shared by the Connections page and the DB Explorer.
    pub async fn list_all(&self) -> Result<Vec<ConnectionSection>> {
        let rows = sqlx::query("SELECT * FROM connection_sections ORDER BY position, name")
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("sections"))?;
        rows.iter().map(row_to_section).collect()
    }

    pub async fn rename(&self, id: &Id, name: &str) -> Result<ConnectionSection> {
        sqlx::query("UPDATE connection_sections SET name = ? WHERE id = ?")
            .bind(name)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("rename section"))?;
        self.get(id).await
    }

    /// Delete a section; its connections fall back to ungrouped (section_id = NULL).
    pub async fn delete(&self, id: &Id) -> Result<()> {
        sqlx::query("UPDATE connections SET section_id = NULL WHERE section_id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("clear section refs"))?;
        sqlx::query("DELETE FROM connection_sections WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete section"))?;
        Ok(())
    }

    /// Reassign positions 0..n in the given order (ids outside `ws` are ignored).
    pub async fn reorder(&self, ws: &Id, ids: &[Id]) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(dberr("reorder begin"))?;
        for (i, id) in ids.iter().enumerate() {
            sqlx::query(
                "UPDATE connection_sections SET position = ? WHERE id = ? AND workspace_id = ?",
            )
            .bind(i as i64)
            .bind(id)
            .bind(ws)
            .execute(&mut *tx)
            .await
            .map_err(dberr("reorder section"))?;
        }
        tx.commit().await.map_err(dberr("reorder commit"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn mem_pool() -> SqlitePool {
        // A single connection so the in-memory DB (private per connection) is
        // shared across migrate + queries.
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

    /// Insert a user + workspace so section FKs are satisfied; returns (ws, user).
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
    async fn create_list_reorder_rename_delete() {
        let pool = mem_pool().await;
        let (ws, user) = seed_ws(&pool).await;
        let repo = ConnectionSectionsRepo::new(pool.clone());

        let a = repo
            .create(&ws, None, "Platform", "connections", &user)
            .await
            .unwrap();
        let b = repo
            .create(&ws, None, "Staging", "connections", &user)
            .await
            .unwrap();
        assert_eq!(a.position, 0);
        assert_eq!(b.position, 1);
        assert_eq!(a.parent_id, None);
        assert_eq!(a.scope, "connections");

        // Sub-sections nest under a parent and have their own position scope.
        let aws = repo
            .create(&ws, Some(&a.id), "AWS", "connections", &user)
            .await
            .unwrap();
        let ams = repo
            .create(&ws, Some(&a.id), "AMS", "connections", &user)
            .await
            .unwrap();
        assert_eq!(aws.parent_id.as_deref(), Some(a.id.as_str()));
        assert_eq!(aws.position, 0);
        assert_eq!(ams.position, 0 + 1);

        // A "db"-scoped section is a separate tree: its top-level position
        // restarts at 0 and it never shows up in the "connections" listing.
        let db_top = repo.create(&ws, None, "Clusters", "db", &user).await.unwrap();
        assert_eq!(db_top.scope, "db");
        assert_eq!(db_top.position, 0);
        assert_eq!(repo.list_for_ws(&ws, "db").await.unwrap().len(), 1);

        // The global tree spans every scope (4 connections-scoped + 1 db).
        assert_eq!(repo.list_all().await.unwrap().len(), 5);

        let list = repo.list_for_ws(&ws, "connections").await.unwrap();
        assert_eq!(list.len(), 4);

        repo.reorder(&ws, &[b.id.clone(), a.id.clone()])
            .await
            .unwrap();
        assert_eq!(repo.get(&b.id).await.unwrap().position, 0);

        // Reparent AMS to be a child of AWS, then to top-level.
        let moved = repo.reparent(&ams.id, Some(&aws.id)).await.unwrap();
        assert_eq!(moved.parent_id.as_deref(), Some(aws.id.as_str()));
        let moved = repo.reparent(&ams.id, None).await.unwrap();
        assert_eq!(moved.parent_id, None);

        let renamed = repo.rename(&a.id, "Prod").await.unwrap();
        assert_eq!(renamed.name, "Prod");

        // Deleting "Prod" cascades to its child "AWS".
        repo.delete(&a.id).await.unwrap();
        let remaining = repo.list_for_ws(&ws, "connections").await.unwrap();
        let ids: Vec<&str> = remaining.iter().map(|s| s.id.as_str()).collect();
        assert!(!ids.contains(&a.id.as_str()));
        assert!(!ids.contains(&aws.id.as_str()));
        assert!(ids.contains(&b.id.as_str()));
        assert!(ids.contains(&ams.id.as_str())); // moved to top-level, survives
    }
}
