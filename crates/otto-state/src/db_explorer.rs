//! DB Explorer repository: saved queries, query history, dashboards, widgets.

use chrono::{DateTime, Utc};
use otto_core::{new_id, Id, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt, json, ts};

// --- Domain ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedQuery {
    pub id: Id,
    pub workspace_id: Id,
    pub connection_id: Option<Id>,
    pub name: String,
    pub statement: String,
    pub created_by: Id,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: Id,
    pub connection_id: Id,
    pub statement: String,
    pub ok: bool,
    pub duration_ms: i64,
    pub row_count: i64,
    pub error: Option<String>,
    /// Who ran this query. `None` for rows recorded before migration 0042
    /// (legacy single-user data). Non-admin filtered views only show rows
    /// where `user_id` matches the caller; legacy rows are invisible to them.
    pub user_id: Option<Id>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dashboard {
    pub id: Id,
    pub workspace_id: Id,
    pub name: String,
    /// JSON array of `{ widget_id, x, y, w, h }`.
    pub layout: Value,
    pub refresh_secs: Option<i64>,
    pub created_by: Id,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Widget {
    pub id: Id,
    pub workspace_id: Id,
    pub dashboard_id: Option<Id>,
    pub connection_id: Id,
    pub title: String,
    pub statement: String,
    /// "table" | "line" | "bar" | "area" | "pie" | "number".
    pub viz: String,
    /// JSON: `{ x?, y?[], category?, value? }` — which columns drive the chart.
    pub mapping: Value,
    pub options: Value,
    pub created_by: Id,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// --- Inputs ----------------------------------------------------------------

pub struct NewSavedQuery {
    pub workspace_id: Id,
    pub connection_id: Option<Id>,
    pub name: String,
    pub statement: String,
    pub created_by: Id,
}

pub struct NewWidget {
    pub workspace_id: Id,
    pub dashboard_id: Option<Id>,
    pub connection_id: Id,
    pub title: String,
    pub statement: String,
    pub viz: String,
    pub mapping: Value,
    pub options: Value,
    pub created_by: Id,
}

// --- Repo ------------------------------------------------------------------

#[derive(Clone)]
pub struct DbExplorerRepo {
    pool: SqlitePool,
}

fn row_to_saved(r: &sqlx::sqlite::SqliteRow) -> Result<SavedQuery> {
    Ok(SavedQuery {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        connection_id: r.get("connection_id"),
        name: r.get("name"),
        statement: r.get("statement"),
        created_by: r.get("created_by"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
    })
}

fn row_to_history(r: &sqlx::sqlite::SqliteRow) -> Result<HistoryEntry> {
    Ok(HistoryEntry {
        id: r.get("id"),
        connection_id: r.get("connection_id"),
        statement: r.get("statement"),
        ok: r.get::<i64, _>("ok") != 0,
        duration_ms: r.get("duration_ms"),
        row_count: r.get("row_count"),
        error: r.get("error"),
        user_id: r.get("user_id"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
    })
}

fn row_to_dashboard(r: &sqlx::sqlite::SqliteRow) -> Result<Dashboard> {
    Ok(Dashboard {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        name: r.get("name"),
        layout: json(&r.get::<String, _>("layout_json"))?,
        refresh_secs: r.get("refresh_secs"),
        created_by: r.get("created_by"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
        updated_at: ts(&r.get::<String, _>("updated_at"))?,
    })
}

fn row_to_widget(r: &sqlx::sqlite::SqliteRow) -> Result<Widget> {
    Ok(Widget {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        dashboard_id: r.get("dashboard_id"),
        connection_id: r.get("connection_id"),
        title: r.get("title"),
        statement: r.get("statement"),
        viz: r.get("viz"),
        mapping: json(&r.get::<String, _>("mapping_json"))?,
        options: json(&r.get::<String, _>("options_json"))?,
        created_by: r.get("created_by"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
        updated_at: ts(&r.get::<String, _>("updated_at"))?,
    })
}

impl DbExplorerRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    // -- Saved queries ------------------------------------------------------

    pub async fn create_saved(&self, q: NewSavedQuery) -> Result<SavedQuery> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO db_saved_queries (id, workspace_id, connection_id, name, statement,
                                           created_by, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&q.workspace_id)
        .bind(&q.connection_id)
        .bind(&q.name)
        .bind(&q.statement)
        .bind(&q.created_by)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create saved query"))?;
        self.get_saved(&id).await
    }

    pub async fn get_saved(&self, id: &Id) -> Result<SavedQuery> {
        let row = sqlx::query("SELECT * FROM db_saved_queries WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("get saved query"))?;
        row_to_saved(&row)
    }

    pub async fn delete_saved(&self, id: &Id) -> Result<()> {
        sqlx::query("DELETE FROM db_saved_queries WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete saved query"))?;
        Ok(())
    }

    // -- History ------------------------------------------------------------

    /// Insert a history row. `user_id` is the caller who executed the statement;
    /// it is recorded since migration 0042 and used to scope per-user history views.
    #[allow(clippy::too_many_arguments)]
    pub async fn add_history(
        &self,
        connection_id: &Id,
        user_id: &Id,
        statement: &str,
        ok: bool,
        duration_ms: i64,
        row_count: i64,
        error: Option<&str>,
    ) -> Result<()> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO db_query_history (id, connection_id, user_id, statement, ok, duration_ms,
                                           row_count, error, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(connection_id)
        .bind(user_id)
        .bind(statement)
        .bind(i64::from(ok))
        .bind(duration_ms)
        .bind(row_count)
        .bind(error)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("add history"))?;
        Ok(())
    }

    /// Return all history for a connection, unfiltered. For root / workspace-Admin use.
    pub async fn list_history(&self, connection_id: &Id, limit: i64) -> Result<Vec<HistoryEntry>> {
        let rows = sqlx::query(
            "SELECT * FROM db_query_history WHERE connection_id = ?
             ORDER BY created_at DESC LIMIT ?",
        )
        .bind(connection_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list history"))?;
        rows.iter().map(row_to_history).collect()
    }

    /// Return history for a connection scoped to a single user. Used for
    /// non-root callers: only shows rows where `user_id = caller_id`.
    ///
    /// Legacy rows with `user_id = NULL` (pre-0041) are excluded — they predate
    /// multi-user and cannot be attributed to any specific user.
    pub async fn list_history_for_user(
        &self,
        connection_id: &Id,
        user_id: &Id,
        limit: i64,
    ) -> Result<Vec<HistoryEntry>> {
        let rows = sqlx::query(
            "SELECT * FROM db_query_history WHERE connection_id = ? AND user_id = ?
             ORDER BY created_at DESC LIMIT ?",
        )
        .bind(connection_id)
        .bind(user_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list history for user"))?;
        rows.iter().map(row_to_history).collect()
    }

    /// Return all saved queries for a workspace, unfiltered. For root / workspace-Admin use.
    pub async fn list_saved(&self, ws: &Id) -> Result<Vec<SavedQuery>> {
        let rows = sqlx::query(
            "SELECT * FROM db_saved_queries WHERE workspace_id = ? ORDER BY created_at DESC",
        )
        .bind(ws)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list saved queries"))?;
        rows.iter().map(row_to_saved).collect()
    }

    /// Return saved queries for a workspace scoped to a single user. Used for
    /// non-root callers: only shows rows where `created_by = caller_id`.
    pub async fn list_saved_for_user(&self, ws: &Id, user_id: &Id) -> Result<Vec<SavedQuery>> {
        let rows = sqlx::query(
            "SELECT * FROM db_saved_queries WHERE workspace_id = ? AND created_by = ?
             ORDER BY created_at DESC",
        )
        .bind(ws)
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list saved queries for user"))?;
        rows.iter().map(row_to_saved).collect()
    }

    // -- Dashboards ---------------------------------------------------------

    /// All dashboards for a workspace — root / ws-Admin view.
    pub async fn list_dashboards(&self, ws: &Id) -> Result<Vec<Dashboard>> {
        let rows = sqlx::query("SELECT * FROM db_dashboards WHERE workspace_id = ? ORDER BY name")
            .bind(ws)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("list dashboards"))?;
        rows.iter().map(row_to_dashboard).collect()
    }

    /// Dashboards for a workspace scoped to a single user — non-admin view (#L13).
    pub async fn list_dashboards_for_user(&self, ws: &Id, user_id: &Id) -> Result<Vec<Dashboard>> {
        let rows = sqlx::query(
            "SELECT * FROM db_dashboards WHERE workspace_id = ? AND created_by = ? ORDER BY name",
        )
        .bind(ws)
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list dashboards for user"))?;
        rows.iter().map(row_to_dashboard).collect()
    }

    pub async fn get_dashboard(&self, id: &Id) -> Result<Dashboard> {
        let row = sqlx::query("SELECT * FROM db_dashboards WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("get dashboard"))?;
        row_to_dashboard(&row)
    }

    pub async fn create_dashboard(&self, ws: &Id, name: &str, created_by: &Id) -> Result<Dashboard> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO db_dashboards (id, workspace_id, name, layout_json, refresh_secs,
                                        created_by, created_at, updated_at)
             VALUES (?, ?, ?, '[]', NULL, ?, ?, ?)",
        )
        .bind(&id)
        .bind(ws)
        .bind(name)
        .bind(created_by)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create dashboard"))?;
        self.get_dashboard(&id).await
    }

    pub async fn update_dashboard(
        &self,
        id: &Id,
        name: Option<&str>,
        layout: Option<&Value>,
        refresh_secs: Option<Option<i64>>,
    ) -> Result<Dashboard> {
        let existing = self.get_dashboard(id).await?;
        let name = name.unwrap_or(&existing.name);
        let layout_json = match layout {
            Some(v) => v.to_string(),
            None => existing.layout.to_string(),
        };
        let refresh = match refresh_secs {
            Some(v) => v,
            None => existing.refresh_secs,
        };
        let now = fmt(Utc::now());
        sqlx::query(
            "UPDATE db_dashboards SET name = ?, layout_json = ?, refresh_secs = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(name)
        .bind(&layout_json)
        .bind(refresh)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("update dashboard"))?;
        self.get_dashboard(id).await
    }

    pub async fn delete_dashboard(&self, id: &Id) -> Result<()> {
        sqlx::query("DELETE FROM db_dashboards WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete dashboard"))?;
        Ok(())
    }

    // -- Widgets ------------------------------------------------------------

    /// All widgets for a workspace — root / ws-Admin view.
    pub async fn list_widgets(&self, ws: &Id) -> Result<Vec<Widget>> {
        let rows = sqlx::query("SELECT * FROM db_widgets WHERE workspace_id = ? ORDER BY created_at")
            .bind(ws)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("list widgets"))?;
        rows.iter().map(row_to_widget).collect()
    }

    /// Widgets for a workspace scoped to a single user — non-admin view (#L13).
    pub async fn list_widgets_for_user(&self, ws: &Id, user_id: &Id) -> Result<Vec<Widget>> {
        let rows = sqlx::query(
            "SELECT * FROM db_widgets WHERE workspace_id = ? AND created_by = ? ORDER BY created_at",
        )
        .bind(ws)
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list widgets for user"))?;
        rows.iter().map(row_to_widget).collect()
    }

    pub async fn get_widget(&self, id: &Id) -> Result<Widget> {
        let row = sqlx::query("SELECT * FROM db_widgets WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("get widget"))?;
        row_to_widget(&row)
    }

    pub async fn create_widget(&self, w: NewWidget) -> Result<Widget> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO db_widgets (id, workspace_id, dashboard_id, connection_id, title,
                                     statement, viz, mapping_json, options_json, created_by,
                                     created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&w.workspace_id)
        .bind(&w.dashboard_id)
        .bind(&w.connection_id)
        .bind(&w.title)
        .bind(&w.statement)
        .bind(&w.viz)
        .bind(w.mapping.to_string())
        .bind(w.options.to_string())
        .bind(&w.created_by)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create widget"))?;
        self.get_widget(&id).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn update_widget(
        &self,
        id: &Id,
        dashboard_id: Option<Option<&str>>,
        title: Option<&str>,
        statement: Option<&str>,
        viz: Option<&str>,
        mapping: Option<&Value>,
        options: Option<&Value>,
    ) -> Result<Widget> {
        let existing = self.get_widget(id).await?;
        let dash = match dashboard_id {
            Some(v) => v.map(str::to_string),
            None => existing.dashboard_id.clone(),
        };
        let title = title.unwrap_or(&existing.title);
        let statement = statement.unwrap_or(&existing.statement);
        let viz = viz.unwrap_or(&existing.viz);
        let mapping_json = mapping.map(|v| v.to_string()).unwrap_or_else(|| existing.mapping.to_string());
        let options_json = options.map(|v| v.to_string()).unwrap_or_else(|| existing.options.to_string());
        let now = fmt(Utc::now());
        sqlx::query(
            "UPDATE db_widgets SET dashboard_id = ?, title = ?, statement = ?, viz = ?,
                                   mapping_json = ?, options_json = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(&dash)
        .bind(title)
        .bind(statement)
        .bind(viz)
        .bind(&mapping_json)
        .bind(&options_json)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("update widget"))?;
        self.get_widget(id).await
    }

    pub async fn delete_widget(&self, id: &Id) -> Result<()> {
        sqlx::query("DELETE FROM db_widgets WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete widget"))?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests — Task 3.6: owner-scope DB history + saved queries (#L11–#L13)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::SqlitePool;

    /// Spin up an in-memory SQLite pool with all migrations applied.
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

    /// Seed a user row and return their id.
    async fn seed_user(pool: &SqlitePool, username: &str, is_root: bool) -> Id {
        let id = otto_core::new_id();
        let now = crate::convert::fmt(Utc::now());
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, display_name, is_root, disabled, created_at)
             VALUES (?, ?, ?, ?, ?, 0, ?)",
        )
        .bind(&id)
        .bind(username)
        .bind("hash")
        .bind(username)
        .bind(is_root as i64)
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();
        id
    }

    /// Seed a workspace and return its id.
    async fn seed_workspace(pool: &SqlitePool) -> Id {
        let ws_id = otto_core::new_id();
        let now = crate::convert::fmt(Utc::now());
        sqlx::query(
            "INSERT INTO workspaces (id, name, root_path, created_at) VALUES (?, ?, ?, ?)",
        )
        .bind(&ws_id)
        .bind("test-ws")
        .bind("/tmp/test-ws")
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();
        ws_id
    }

    /// Seed a connection and return its id. Needed so add_history can reference it.
    async fn seed_connection(pool: &SqlitePool, ws_id: &Id, created_by: &Id) -> Id {
        let conn_id = otto_core::new_id();
        let now = crate::convert::fmt(Utc::now());
        sqlx::query(
            "INSERT INTO connections (id, workspace_id, name, kind, params_json, created_by, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&conn_id)
        .bind(ws_id)
        .bind("test-conn")
        .bind("mysql")
        .bind("{}")
        .bind(created_by)
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();
        conn_id
    }

    // ── History isolation (#L11) ─────────────────────────────────────────────

    /// User A runs a query → user B's per-user history list does NOT include it.
    #[tokio::test]
    async fn history_user_a_invisible_to_user_b() {
        let pool = mem_pool().await;
        let repo = DbExplorerRepo::new(pool.clone());

        let user_a = seed_user(&pool, "alice", false).await;
        let user_b = seed_user(&pool, "bob", false).await;
        let ws_id = seed_workspace(&pool).await;
        let conn_id = seed_connection(&pool, &ws_id, &user_a).await;

        // A records a query.
        repo.add_history(&conn_id, &user_a, "SELECT 1", true, 10, 1, None)
            .await
            .unwrap();

        // B's per-user list is empty.
        let b_history = repo
            .list_history_for_user(&conn_id, &user_b, 100)
            .await
            .unwrap();
        assert!(
            b_history.is_empty(),
            "user B must not see user A's history; got {:?}",
            b_history.iter().map(|h| &h.statement).collect::<Vec<_>>()
        );
    }

    /// User B's own query appears in B's per-user history.
    #[tokio::test]
    async fn history_user_b_sees_own_rows() {
        let pool = mem_pool().await;
        let repo = DbExplorerRepo::new(pool.clone());

        let user_a = seed_user(&pool, "alice", false).await;
        let user_b = seed_user(&pool, "bob", false).await;
        let ws_id = seed_workspace(&pool).await;
        let conn_id = seed_connection(&pool, &ws_id, &user_a).await;

        repo.add_history(&conn_id, &user_a, "SELECT 1", true, 5, 1, None)
            .await
            .unwrap();
        repo.add_history(&conn_id, &user_b, "SELECT 2", true, 5, 1, None)
            .await
            .unwrap();

        let b_history = repo
            .list_history_for_user(&conn_id, &user_b, 100)
            .await
            .unwrap();
        assert_eq!(b_history.len(), 1, "B sees exactly their own entry");
        assert_eq!(b_history[0].statement, "SELECT 2");
    }

    /// The unfiltered `list_history` (root/admin view) sees all rows.
    #[tokio::test]
    async fn history_unfiltered_sees_all() {
        let pool = mem_pool().await;
        let repo = DbExplorerRepo::new(pool.clone());

        let user_a = seed_user(&pool, "alice", false).await;
        let user_b = seed_user(&pool, "bob", false).await;
        let ws_id = seed_workspace(&pool).await;
        let conn_id = seed_connection(&pool, &ws_id, &user_a).await;

        repo.add_history(&conn_id, &user_a, "SELECT 1", true, 5, 1, None)
            .await
            .unwrap();
        repo.add_history(&conn_id, &user_b, "SELECT 2", true, 5, 1, None)
            .await
            .unwrap();

        let all = repo.list_history(&conn_id, 100).await.unwrap();
        assert_eq!(all.len(), 2, "unfiltered view must return both entries");
    }

    /// `user_id` is stored in the row and visible via `list_history`.
    #[tokio::test]
    async fn history_user_id_is_recorded() {
        let pool = mem_pool().await;
        let repo = DbExplorerRepo::new(pool.clone());

        let user_a = seed_user(&pool, "alice", false).await;
        let ws_id = seed_workspace(&pool).await;
        let conn_id = seed_connection(&pool, &ws_id, &user_a).await;

        repo.add_history(&conn_id, &user_a, "SELECT 42", true, 7, 1, None)
            .await
            .unwrap();

        let all = repo.list_history(&conn_id, 100).await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(
            all[0].user_id.as_deref(),
            Some(user_a.as_str()),
            "user_id must be stored in the history row"
        );
    }

    // ── Saved-query isolation (#L12) ─────────────────────────────────────────

    /// A's saved query is invisible to B's per-user list.
    #[tokio::test]
    async fn saved_queries_user_a_invisible_to_user_b() {
        let pool = mem_pool().await;
        let repo = DbExplorerRepo::new(pool.clone());

        let user_a = seed_user(&pool, "alice", false).await;
        let user_b = seed_user(&pool, "bob", false).await;
        let ws_id = seed_workspace(&pool).await;

        // A creates a saved query.
        repo.create_saved(NewSavedQuery {
            workspace_id: ws_id.clone(),
            connection_id: None,
            name: "A query".into(),
            statement: "SELECT * FROM a".into(),
            created_by: user_a.clone(),
        })
        .await
        .unwrap();

        let b_saved = repo.list_saved_for_user(&ws_id, &user_b).await.unwrap();
        assert!(
            b_saved.is_empty(),
            "user B must not see user A's saved queries"
        );
    }

    /// B's saved query appears in B's per-user list.
    #[tokio::test]
    async fn saved_queries_user_b_sees_own() {
        let pool = mem_pool().await;
        let repo = DbExplorerRepo::new(pool.clone());

        let user_a = seed_user(&pool, "alice", false).await;
        let user_b = seed_user(&pool, "bob", false).await;
        let ws_id = seed_workspace(&pool).await;

        repo.create_saved(NewSavedQuery {
            workspace_id: ws_id.clone(),
            connection_id: None,
            name: "A query".into(),
            statement: "SELECT * FROM a".into(),
            created_by: user_a.clone(),
        })
        .await
        .unwrap();
        repo.create_saved(NewSavedQuery {
            workspace_id: ws_id.clone(),
            connection_id: None,
            name: "B query".into(),
            statement: "SELECT * FROM b".into(),
            created_by: user_b.clone(),
        })
        .await
        .unwrap();

        let b_saved = repo.list_saved_for_user(&ws_id, &user_b).await.unwrap();
        assert_eq!(b_saved.len(), 1);
        assert_eq!(b_saved[0].name, "B query");
    }

    /// Unfiltered `list_saved` (root/admin view) sees all rows.
    #[tokio::test]
    async fn saved_queries_unfiltered_sees_all() {
        let pool = mem_pool().await;
        let repo = DbExplorerRepo::new(pool.clone());

        let user_a = seed_user(&pool, "alice", false).await;
        let user_b = seed_user(&pool, "bob", false).await;
        let ws_id = seed_workspace(&pool).await;

        for (name, by) in [("A query", &user_a), ("B query", &user_b)] {
            repo.create_saved(NewSavedQuery {
                workspace_id: ws_id.clone(),
                connection_id: None,
                name: name.into(),
                statement: "SELECT 1".into(),
                created_by: by.clone(),
            })
            .await
            .unwrap();
        }

        let all = repo.list_saved(&ws_id).await.unwrap();
        assert_eq!(all.len(), 2, "unfiltered list must return both entries");
    }
}
