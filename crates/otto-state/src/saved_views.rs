//! Persistence for Mission Control saved work-queue views (B4).
//!
//! Each row is one user-defined named filter on the 6-bucket view.
//! Users create them with `POST /workspaces/{id}/mission/views`,
//! list them with `GET /workspaces/{id}/mission/views`, and delete
//! them with `DELETE /mission-views/{id}`.

use chrono::Utc;
use otto_core::{new_id, Error, Id, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt, ts};

// ---------------------------------------------------------------------------
// Domain type
// ---------------------------------------------------------------------------

/// A user-defined named filter saved against a workspace.
///
/// `filter_json` is opaque to the daemon — the UI interprets it.  Typical
/// examples: `{"bucket":"needs_you"}`, `{"bucket":"failed","max_age_secs":3600}`,
/// `{"min_cost_usd":5.0}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedView {
    pub id: Id,
    pub user_id: Id,
    pub workspace_id: Id,
    pub name: String,
    /// Freeform JSON filter descriptor; interpreted entirely by the client.
    pub filter: Value,
    pub created_at: chrono::DateTime<Utc>,
}

/// Payload for `POST /workspaces/{id}/mission/views`.
#[derive(Debug, Clone, Deserialize)]
pub struct NewSavedView {
    pub name: String,
    #[serde(default)]
    pub filter: Value,
}

// ---------------------------------------------------------------------------
// Repository
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct SavedViewsRepo {
    pool: SqlitePool,
}

fn row_to_view(r: &sqlx::sqlite::SqliteRow) -> Result<SavedView> {
    let filter: Value = serde_json::from_str(&r.get::<String, _>("filter_json"))
        .unwrap_or(Value::Object(serde_json::Map::new()));
    Ok(SavedView {
        id: r.get("id"),
        user_id: r.get("user_id"),
        workspace_id: r.get("workspace_id"),
        name: r.get("name"),
        filter,
        created_at: ts(&r.get::<String, _>("created_at"))?,
    })
}

impl SavedViewsRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// List all saved views for the given user in the given workspace,
    /// newest-first.
    pub async fn list(&self, workspace_id: &Id, user_id: &Id) -> Result<Vec<SavedView>> {
        let rows = sqlx::query(
            "SELECT * FROM saved_views WHERE workspace_id = ? AND user_id = ?
             ORDER BY created_at DESC",
        )
        .bind(workspace_id)
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list saved views"))?;

        rows.iter().map(row_to_view).collect()
    }

    /// Create a new saved view; returns the persisted row.
    pub async fn create(
        &self,
        workspace_id: &Id,
        user_id: &Id,
        req: NewSavedView,
    ) -> Result<SavedView> {
        let id = new_id();
        let now = fmt(Utc::now());
        let filter_json = serde_json::to_string(&req.filter)
            .map_err(|e| Error::Internal(format!("serialize filter: {e}")))?;

        sqlx::query(
            "INSERT INTO saved_views (id, user_id, workspace_id, name, filter_json, created_at)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(user_id)
        .bind(workspace_id)
        .bind(&req.name)
        .bind(&filter_json)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create saved view"))?;

        self.get(&id).await
    }

    /// Fetch a single saved view by id.
    pub async fn get(&self, id: &Id) -> Result<SavedView> {
        let row = sqlx::query("SELECT * FROM saved_views WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("get saved view"))?;
        row_to_view(&row)
    }

    /// Delete a saved view.  The caller should verify ownership before calling.
    /// Returns `Error::NotFound` when the row does not exist (already gone is
    /// idempotent at the HTTP layer — the handler returns 204 either way).
    pub async fn delete(&self, id: &Id) -> Result<()> {
        let res = sqlx::query("DELETE FROM saved_views WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete saved view"))?;
        if res.rows_affected() == 0 {
            return Err(Error::NotFound(format!("saved view {id}")));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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
    async fn create_list_delete() {
        let pool = mem_pool().await;
        let repo = SavedViewsRepo::new(pool);

        let ws: Id = "ws1".into();
        let uid: Id = "u1".into();

        // create
        let v = repo
            .create(
                &ws,
                &uid,
                NewSavedView {
                    name: "Waiting on me".into(),
                    filter: serde_json::json!({"bucket": "needs_you"}),
                },
            )
            .await
            .unwrap();
        assert_eq!(v.name, "Waiting on me");
        assert_eq!(v.filter["bucket"], "needs_you");

        // list
        let list = repo.list(&ws, &uid).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, v.id);

        // delete
        repo.delete(&v.id).await.unwrap();
        let list = repo.list(&ws, &uid).await.unwrap();
        assert!(list.is_empty());

        // double-delete is NotFound
        assert!(matches!(repo.delete(&v.id).await, Err(Error::NotFound(_))));
    }

    #[tokio::test]
    async fn list_isolated_by_workspace() {
        let pool = mem_pool().await;
        let repo = SavedViewsRepo::new(pool);

        let uid: Id = "u1".into();
        repo.create(
            &"ws1".into(),
            &uid,
            NewSavedView { name: "A".into(), filter: Value::Null },
        )
        .await
        .unwrap();
        repo.create(
            &"ws2".into(),
            &uid,
            NewSavedView { name: "B".into(), filter: Value::Null },
        )
        .await
        .unwrap();

        let list = repo.list(&"ws1".into(), &uid).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "A");
    }
}
