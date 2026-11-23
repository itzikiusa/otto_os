//! Persistence pass-throughs for saved queries, query history, dashboards,
//! and dashboard widgets.

use otto_core::{Error, Id, Result};
use otto_state::{Dashboard, HistoryEntry, NewSavedQuery, NewWidget, SavedQuery, Widget};
use serde_json::Value;

use super::{ensure_read_only_request, DbViewerService};
use crate::types::{Engine, QueryRequest, QueryResult};

impl DbViewerService {
    /// All saved queries for a workspace — root / ws-Admin view.
    pub async fn list_saved(&self, ws: &Id) -> Result<Vec<SavedQuery>> {
        self.repo.list_saved(ws).await
    }

    /// Saved queries for a workspace scoped to a single user — non-admin view.
    pub async fn list_saved_for_user(&self, ws: &Id, user_id: &Id) -> Result<Vec<SavedQuery>> {
        let mut visible = Vec::new();
        for query in self.repo.list_saved_for_user(ws, user_id).await? {
            if let Some(conn) = &query.connection_id {
                if self
                    .authorize(conn, user_id, None, "db_query")
                    .await
                    .is_err()
                {
                    continue;
                }
            }
            visible.push(query);
        }
        Ok(visible)
    }

    pub async fn create_saved(&self, q: NewSavedQuery) -> Result<SavedQuery> {
        if let Some(conn) = &q.connection_id {
            self.authorize(conn, &q.created_by, None, "db_query")
                .await?;
        }
        self.repo.create_saved(q).await
    }

    pub async fn get_saved(&self, id: &Id) -> Result<SavedQuery> {
        self.repo.get_saved(id).await
    }

    pub async fn update_saved(
        &self,
        id: &Id,
        name: Option<&str>,
        statement: Option<&str>,
    ) -> Result<SavedQuery> {
        self.repo.update_saved(id, name, statement).await
    }

    pub async fn delete_saved(&self, id: &Id) -> Result<()> {
        self.repo.delete_saved(id).await
    }

    /// All history for a connection — root / ws-Admin view.
    pub async fn list_history(&self, conn_id: &Id, limit: i64) -> Result<Vec<HistoryEntry>> {
        self.repo.list_history(conn_id, limit).await
    }

    /// One history row with its full statement, scoped like the list: it must
    /// belong to `conn_id`, and a non-root caller may only read their own rows.
    pub async fn get_history(
        &self,
        conn_id: &Id,
        user_id: &Id,
        entry_id: &Id,
        is_root: bool,
    ) -> Result<HistoryEntry> {
        self.authorize(conn_id, user_id, None, "db_query").await?;
        let entry = self.repo.get_history(entry_id).await?;
        let visible =
            &entry.connection_id == conn_id && (is_root || entry.user_id.as_ref() == Some(user_id));
        if !visible {
            return Err(Error::NotFound(format!("history entry {entry_id}")));
        }
        Ok(entry)
    }

    /// History for a connection scoped to a single user — non-admin view.
    pub async fn list_history_for_user(
        &self,
        conn_id: &Id,
        user_id: &Id,
        limit: i64,
    ) -> Result<Vec<HistoryEntry>> {
        self.authorize(conn_id, user_id, None, "db_query").await?;
        self.repo
            .list_history_for_user(conn_id, user_id, limit)
            .await
    }

    /// All dashboards for a workspace — root / ws-Admin view.
    pub async fn list_dashboards(&self, ws: &Id) -> Result<Vec<Dashboard>> {
        self.repo.list_dashboards(ws).await
    }

    /// Dashboards for a workspace scoped to a single user — non-admin view (#L13).
    pub async fn list_dashboards_for_user(&self, ws: &Id, user_id: &Id) -> Result<Vec<Dashboard>> {
        self.repo.list_dashboards_for_user(ws, user_id).await
    }

    pub async fn get_dashboard(&self, id: &Id) -> Result<Dashboard> {
        self.repo.get_dashboard(id).await
    }

    pub async fn create_dashboard(&self, ws: &Id, name: &str, by: &Id) -> Result<Dashboard> {
        self.repo.create_dashboard(ws, name, by).await
    }

    pub async fn update_dashboard(
        &self,
        id: &Id,
        name: Option<&str>,
        layout: Option<&Value>,
        refresh_secs: Option<Option<i64>>,
    ) -> Result<Dashboard> {
        self.repo
            .update_dashboard(id, name, layout, refresh_secs)
            .await
    }

    pub async fn delete_dashboard(&self, id: &Id) -> Result<()> {
        self.repo.delete_dashboard(id).await
    }

    /// All widgets for a workspace — root / ws-Admin view.
    pub async fn list_widgets(&self, ws: &Id) -> Result<Vec<Widget>> {
        self.repo.list_widgets(ws).await
    }

    /// Widgets for a workspace scoped to a single user — non-admin view (#L13).
    pub async fn list_widgets_for_user(&self, ws: &Id, user_id: &Id) -> Result<Vec<Widget>> {
        let mut visible = Vec::new();
        for widget in self.repo.list_widgets_for_user(ws, user_id).await? {
            if self
                .authorize(&widget.connection_id, user_id, None, "db_query")
                .await
                .is_ok()
            {
                visible.push(widget);
            }
        }
        Ok(visible)
    }

    pub async fn get_widget(&self, id: &Id) -> Result<Widget> {
        self.repo.get_widget(id).await
    }

    pub async fn create_widget(&self, w: NewWidget) -> Result<Widget> {
        self.authorize(&w.connection_id, &w.created_by, None, "db_query")
            .await?;
        self.ensure_widget_statement_reads(&w.connection_id, &w.statement)
            .await?;
        if let Some(dash) = w.dashboard_id.as_deref() {
            self.ensure_dashboard_in_workspace(&w.workspace_id, dash)
                .await?;
        }
        self.repo.create_widget(w).await
    }

    /// A widget may only be filed under a dashboard of its OWN workspace — an
    /// unknown or foreign id would leave it invisible on every screen (no FK
    /// guards `db_widgets.dashboard_id`).
    async fn ensure_dashboard_in_workspace(&self, ws: &Id, dashboard_id: &str) -> Result<()> {
        match self.repo.dashboard_workspace(dashboard_id).await? {
            Some(owner) if &owner == ws => Ok(()),
            _ => Err(Error::Invalid(format!(
                "dashboard '{dashboard_id}' does not exist in this workspace"
            ))),
        }
    }

    /// Widgets re-run unattended on every refresh, so only a statement the
    /// read-only classifier accepts may be saved as one (S16-08) — a widget
    /// holding `UPDATE counters …` would otherwise re-execute the write every
    /// `refresh_secs`. [`Self::run_widget`] enforces it again at run time
    /// (with the engine's native read-only mode) for widgets saved earlier.
    async fn ensure_widget_statement_reads(&self, conn_id: &Id, statement: &str) -> Result<()> {
        let conn = self.connections.get(conn_id).await?;
        let Some(engine) = Engine::from_kind(conn.kind) else {
            return Err(Error::Invalid(format!(
                "connection '{}' is not a queryable database (kind {:?})",
                conn.name, conn.kind
            )));
        };
        ensure_widget_statement(engine, statement)
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
        if statement.is_some() || matches!(dashboard_id, Some(Some(_))) {
            let widget = self.repo.get_widget(id).await?;
            if let Some(statement) = statement {
                self.ensure_widget_statement_reads(&widget.connection_id, statement)
                    .await?;
            }
            if let Some(Some(dash)) = dashboard_id {
                self.ensure_dashboard_in_workspace(&widget.workspace_id, dash)
                    .await?;
            }
        }
        self.repo
            .update_widget(id, dashboard_id, title, statement, viz, mapping, options)
            .await
    }

    pub async fn delete_widget(&self, id: &Id) -> Result<()> {
        self.repo.delete_widget(id).await
    }

    /// Run a widget's query for rendering.
    ///
    /// `user_id` is threaded through to history recording (since migration 0042).
    pub async fn run_widget(&self, id: &Id, user_id: &Id) -> Result<QueryResult> {
        let widget = self.repo.get_widget(id).await?;
        let req = widget_request(widget.statement);
        self.run(&widget.connection_id, user_id, &req).await
    }
}

/// The request a dashboard widget runs with. An unattended re-run must never
/// write (S16-08): `read_only` classifies the statement first and then runs it
/// in the engine's native read-only mode, on ANY connection.
pub(super) fn widget_request(statement: String) -> QueryRequest {
    QueryRequest {
        statement,
        max_rows: Some(5000),
        read_only: true,
        ..Default::default()
    }
}

/// Save-time gate for a widget statement (create / update).
pub(super) fn ensure_widget_statement(engine: Engine, statement: &str) -> Result<()> {
    ensure_read_only_request(engine, statement)
        .map_err(|e| Error::Invalid(format!("widgets must be read-only queries — {e}")))
}
