//! Persistence for session **name themes**.
//!
//! Built-in themes live in the daemon (`otto_sessions::names`); this repo stores
//! only the user-owned bits:
//!   * `name_themes`       — a user's CUSTOM ordered name lists (family names, …).
//!   * `name_theme_active` — which theme each user picked for auto-naming.

use chrono::Utc;
use otto_core::{new_id, Error, Id, Result};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt, ts};

/// A user-defined custom name theme: an ordered list of plain names.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomTheme {
    pub id: Id,
    pub owner_id: Id,
    pub label: String,
    /// Ordered name list (e.g. `["Dad","Mom","Sister"]`).
    pub names: Vec<String>,
    pub created_at: chrono::DateTime<Utc>,
}

#[derive(Clone)]
pub struct NameThemesRepo {
    pool: SqlitePool,
}

fn row_to_theme(r: &sqlx::sqlite::SqliteRow) -> Result<CustomTheme> {
    let names: Vec<String> =
        serde_json::from_str(&r.get::<String, _>("names_json")).unwrap_or_default();
    Ok(CustomTheme {
        id: r.get("id"),
        owner_id: r.get("owner_id"),
        label: r.get("label"),
        names,
        created_at: ts(&r.get::<String, _>("created_at"))?,
    })
}

impl NameThemesRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// All custom themes owned by `owner`, newest-first.
    pub async fn list_for_owner(&self, owner: &Id) -> Result<Vec<CustomTheme>> {
        let rows = sqlx::query(
            "SELECT * FROM name_themes WHERE owner_id = ? ORDER BY created_at DESC",
        )
        .bind(owner)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list name themes"))?;
        rows.iter().map(row_to_theme).collect()
    }

    /// Fetch one custom theme by id (any owner).
    pub async fn get(&self, id: &Id) -> Result<CustomTheme> {
        let row = sqlx::query("SELECT * FROM name_themes WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("get name theme"))?;
        row_to_theme(&row)
    }

    /// Create a custom theme. `names` is stored verbatim (order preserved); the
    /// allocator skips empty entries at use time.
    pub async fn create(&self, owner: &Id, label: &str, names: &[String]) -> Result<CustomTheme> {
        let id = new_id();
        let now = fmt(Utc::now());
        let names_json = serde_json::to_string(names)
            .map_err(|e| Error::Internal(format!("serialize names: {e}")))?;
        sqlx::query(
            "INSERT INTO name_themes (id, owner_id, label, names_json, created_at)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(owner)
        .bind(label)
        .bind(&names_json)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create name theme"))?;
        self.get(&id).await
    }

    /// Replace a custom theme's label/names (owner-scoped). `Error::NotFound`
    /// when the row doesn't exist or isn't owned by `owner`.
    pub async fn update(
        &self,
        id: &Id,
        owner: &Id,
        label: &str,
        names: &[String],
    ) -> Result<CustomTheme> {
        let names_json = serde_json::to_string(names)
            .map_err(|e| Error::Internal(format!("serialize names: {e}")))?;
        let res = sqlx::query(
            "UPDATE name_themes SET label = ?, names_json = ? WHERE id = ? AND owner_id = ?",
        )
        .bind(label)
        .bind(&names_json)
        .bind(id)
        .bind(owner)
        .execute(&self.pool)
        .await
        .map_err(dberr("update name theme"))?;
        if res.rows_affected() == 0 {
            return Err(Error::NotFound(format!("name theme {id}")));
        }
        self.get(id).await
    }

    /// Delete a custom theme (owner-scoped). `Error::NotFound` when absent or not
    /// owned by `owner`.
    pub async fn delete(&self, id: &Id, owner: &Id) -> Result<()> {
        let res = sqlx::query("DELETE FROM name_themes WHERE id = ? AND owner_id = ?")
            .bind(id)
            .bind(owner)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete name theme"))?;
        if res.rows_affected() == 0 {
            return Err(Error::NotFound(format!("name theme {id}")));
        }
        Ok(())
    }

    /// The user's active theme id, or `None` when unset (caller applies default).
    pub async fn active(&self, user: &Id) -> Result<Option<String>> {
        let row = sqlx::query("SELECT theme_id FROM name_theme_active WHERE user_id = ?")
            .bind(user)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("get active name theme"))?;
        Ok(row.map(|r| r.get::<String, _>("theme_id")))
    }

    /// Set the user's active theme id (upsert).
    pub async fn set_active(&self, user: &Id, theme_id: &str) -> Result<()> {
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO name_theme_active (user_id, theme_id, updated_at) VALUES (?, ?, ?)
             ON CONFLICT (user_id) DO UPDATE SET theme_id = excluded.theme_id, updated_at = excluded.updated_at",
        )
        .bind(user)
        .bind(theme_id)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("set active name theme"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn mem_pool() -> SqlitePool {
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(false);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!().run(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn custom_theme_crud() {
        let pool = mem_pool().await;
        let repo = NameThemesRepo::new(pool);
        let uid: Id = "u1".into();

        let t = repo
            .create(&uid, "Family", &["Dad".into(), "Mom".into()])
            .await
            .unwrap();
        assert_eq!(t.label, "Family");
        assert_eq!(t.names, vec!["Dad", "Mom"]);

        let list = repo.list_for_owner(&uid).await.unwrap();
        assert_eq!(list.len(), 1);

        let upd = repo
            .update(&t.id, &uid, "Family", &["Dad".into(), "Mom".into(), "Sis".into()])
            .await
            .unwrap();
        assert_eq!(upd.names.len(), 3);

        // wrong owner can't delete
        assert!(matches!(
            repo.delete(&t.id, &"other".into()).await,
            Err(Error::NotFound(_))
        ));
        repo.delete(&t.id, &uid).await.unwrap();
        assert!(repo.list_for_owner(&uid).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn active_theme_upsert() {
        let pool = mem_pool().await;
        let repo = NameThemesRepo::new(pool);
        let uid: Id = "u1".into();

        assert_eq!(repo.active(&uid).await.unwrap(), None);
        repo.set_active(&uid, "footballers").await.unwrap();
        assert_eq!(repo.active(&uid).await.unwrap().as_deref(), Some("footballers"));
        repo.set_active(&uid, "none").await.unwrap();
        assert_eq!(repo.active(&uid).await.unwrap().as_deref(), Some("none"));
    }
}
