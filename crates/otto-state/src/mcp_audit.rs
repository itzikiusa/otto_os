//! Audit ledger for first-party Otto MCP tool calls (Task B2b).
//!
//! Every invocation of an `otto_*` tool exposed to an agent session through
//! `.mcp.json` appends one row via [`McpAuditRepo::record`]. Append-only — there
//! is no update or delete path. The `ottod mcp-tools` subprocess writes these
//! best-effort: a failed audit insert must never fail the tool call, so callers
//! log and swallow the error. `args_json` is redacted (`otto_core::redact`)
//! before it reaches here, so no raw secret is persisted.

use chrono::Utc;
use otto_core::{new_id, Result};
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt};

/// Input for [`McpAuditRepo::record`]. `id`/`created_at` are owned by the repo.
#[derive(Debug, Clone)]
pub struct NewMcpToolCall {
    /// The calling session's workspace, when known.
    pub workspace_id: Option<String>,
    /// The agent session that invoked the tool, when known.
    pub session_id: Option<String>,
    /// Tool name, e.g. `"otto_db_schema"`.
    pub tool: String,
    /// Redacted JSON of the call arguments.
    pub args_json: String,
    /// Whether the call succeeded.
    pub ok: bool,
    /// Row/item count returned (tool-specific; `None` when not meaningful).
    pub rows: Option<i64>,
}

/// One persisted audit row (read side, for a future governance view).
#[derive(Debug, Clone)]
pub struct McpToolCallRow {
    pub id: String,
    pub workspace_id: Option<String>,
    pub session_id: Option<String>,
    pub tool: String,
    pub args_json: String,
    pub ok: bool,
    pub rows: Option<i64>,
    pub created_at: String,
}

fn row_to_call(r: &sqlx::sqlite::SqliteRow) -> McpToolCallRow {
    McpToolCallRow {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        session_id: r.get("session_id"),
        tool: r.get("tool"),
        args_json: r.get("args_json"),
        ok: r.get::<i64, _>("ok") != 0,
        rows: r.get("rows"),
        created_at: r.get("created_at"),
    }
}

#[derive(Clone)]
pub struct McpAuditRepo {
    pool: SqlitePool,
}

impl McpAuditRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Append one tool-call audit row.
    pub async fn record(&self, e: NewMcpToolCall) -> Result<()> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO mcp_tool_calls
               (id, workspace_id, session_id, tool, args_json, ok, rows, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&e.workspace_id)
        .bind(&e.session_id)
        .bind(&e.tool)
        .bind(&e.args_json)
        .bind(e.ok as i64)
        .bind(e.rows)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("mcp tool audit insert"))?;
        Ok(())
    }

    /// Recent tool calls for a session (newest first, capped at `limit`).
    pub async fn recent_for_session(
        &self,
        session_id: &str,
        limit: i64,
    ) -> Result<Vec<McpToolCallRow>> {
        let rows = sqlx::query(
            "SELECT id, workspace_id, session_id, tool, args_json, ok, rows, created_at
             FROM mcp_tool_calls
             WHERE session_id = ?
             ORDER BY created_at DESC
             LIMIT ?",
        )
        .bind(session_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("mcp tool audit query"))?;
        Ok(rows.iter().map(row_to_call).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

    async fn mem_pool() -> SqlitePool {
        let opts = SqliteConnectOptions::new().in_memory(true).foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn record_and_read_back() {
        let pool = mem_pool().await;
        let repo = McpAuditRepo::new(pool.clone());
        repo.record(NewMcpToolCall {
            workspace_id: Some("ws1".into()),
            session_id: Some("sess1".into()),
            tool: "otto_db_schema".into(),
            args_json: r#"{"connection_id":"c1"}"#.into(),
            ok: true,
            rows: Some(7),
        })
        .await
        .unwrap();
        // A second call that failed (e.g. denied / not found).
        repo.record(NewMcpToolCall {
            workspace_id: Some("ws1".into()),
            session_id: Some("sess1".into()),
            tool: "otto_git_pr_review".into(),
            args_json: r#"{"repo_id":"r1","pr_number":9}"#.into(),
            ok: false,
            rows: None,
        })
        .await
        .unwrap();

        let recent = repo.recent_for_session("sess1", 10).await.unwrap();
        assert_eq!(recent.len(), 2);
        // Newest first: the failed git call is most recent.
        assert_eq!(recent[0].tool, "otto_git_pr_review");
        assert!(!recent[0].ok);
        assert_eq!(recent[0].rows, None);
        assert_eq!(recent[1].tool, "otto_db_schema");
        assert!(recent[1].ok);
        assert_eq!(recent[1].rows, Some(7));
    }
}
