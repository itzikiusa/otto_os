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

    pub async fn add_history(
        &self,
        connection_id: &Id,
        statement: &str,
        ok: bool,
        duration_ms: i64,
        row_count: i64,
        error: Option<&str>,
    ) -> Result<()> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO db_query_history (id, connection_id, statement, ok, duration_ms,
                                           row_count, error, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(connection_id)
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

    // -- Dashboards ---------------------------------------------------------

    pub async fn list_dashboards(&self, ws: &Id) -> Result<Vec<Dashboard>> {
        let rows = sqlx::query("SELECT * FROM db_dashboards WHERE workspace_id = ? ORDER BY name")
            .bind(ws)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("list dashboards"))?;
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

    pub async fn list_widgets(&self, ws: &Id) -> Result<Vec<Widget>> {
        let rows = sqlx::query("SELECT * FROM db_widgets WHERE workspace_id = ? ORDER BY created_at")
            .bind(ws)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("list widgets"))?;
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
