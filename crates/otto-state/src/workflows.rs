//! Workflows repository: workflow definitions + their run history.

use chrono::Utc;
use otto_core::workflows::{NodeRunState, RunStatus, Workflow, WorkflowGraph, WorkflowRun};
use otto_core::{new_id, Error, Id, Result};
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt, ts};

#[derive(Clone)]
pub struct WorkflowsRepo {
    pool: SqlitePool,
}

fn parse_graph(s: &str) -> Result<WorkflowGraph> {
    serde_json::from_str(s).map_err(|e| Error::Internal(format!("bad workflow graph: {e}")))
}

fn row_to_workflow(r: &sqlx::sqlite::SqliteRow) -> Result<Workflow> {
    Ok(Workflow {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        name: r.get("name"),
        description: r.get("description"),
        graph: parse_graph(&r.get::<String, _>("graph_json"))?,
        created_by: r.get("created_by"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
        updated_at: ts(&r.get::<String, _>("updated_at"))?,
    })
}

fn row_to_run(r: &sqlx::sqlite::SqliteRow) -> Result<WorkflowRun> {
    let nodes: Vec<NodeRunState> = serde_json::from_str(&r.get::<String, _>("nodes_json"))
        .map_err(|e| Error::Internal(format!("bad run nodes: {e}")))?;
    let input: serde_json::Value = serde_json::from_str(&r.get::<String, _>("input_json"))
        .unwrap_or(serde_json::Value::Null);
    let finished: Option<String> = r.get("finished_at");
    Ok(WorkflowRun {
        id: r.get("id"),
        workflow_id: r.get("workflow_id"),
        workspace_id: r.get("workspace_id"),
        status: RunStatus::parse(&r.get::<String, _>("status"))
            .ok_or_else(|| Error::Internal("bad run status".into()))?,
        input,
        nodes,
        error: r.get("error"),
        started_at: ts(&r.get::<String, _>("started_at"))?,
        finished_at: finished.as_deref().map(ts).transpose()?,
    })
}

impl WorkflowsRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(
        &self,
        workspace_id: &Id,
        name: &str,
        description: &str,
        graph: &WorkflowGraph,
        created_by: &Id,
    ) -> Result<Workflow> {
        let id = new_id();
        let now = fmt(Utc::now());
        let graph_json =
            serde_json::to_string(graph).map_err(|e| Error::Internal(e.to_string()))?;
        sqlx::query(
            "INSERT INTO workflows (id, workspace_id, name, description, graph_json,
                                    created_by, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(workspace_id)
        .bind(name)
        .bind(description)
        .bind(&graph_json)
        .bind(created_by)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create workflow"))?;
        self.get(&id).await
    }

    pub async fn get(&self, id: &Id) -> Result<Workflow> {
        let r = sqlx::query("SELECT * FROM workflows WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("workflow"))?;
        row_to_workflow(&r)
    }

    pub async fn list(&self, ws: &Id) -> Result<Vec<Workflow>> {
        let rows = sqlx::query("SELECT * FROM workflows WHERE workspace_id = ? ORDER BY updated_at DESC")
            .bind(ws)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("workflows"))?;
        rows.iter().map(row_to_workflow).collect()
    }

    pub async fn update(
        &self,
        id: &Id,
        name: Option<&str>,
        description: Option<&str>,
        graph: Option<&WorkflowGraph>,
    ) -> Result<Workflow> {
        let now = fmt(Utc::now());
        if let Some(v) = name {
            sqlx::query("UPDATE workflows SET name = ?, updated_at = ? WHERE id = ?")
                .bind(v)
                .bind(&now)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("update workflow"))?;
        }
        if let Some(v) = description {
            sqlx::query("UPDATE workflows SET description = ?, updated_at = ? WHERE id = ?")
                .bind(v)
                .bind(&now)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("update workflow"))?;
        }
        if let Some(g) = graph {
            let graph_json =
                serde_json::to_string(g).map_err(|e| Error::Internal(e.to_string()))?;
            sqlx::query("UPDATE workflows SET graph_json = ?, updated_at = ? WHERE id = ?")
                .bind(&graph_json)
                .bind(&now)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("update workflow"))?;
        }
        self.get(id).await
    }

    pub async fn delete(&self, id: &Id) -> Result<()> {
        sqlx::query("DELETE FROM workflows WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete workflow"))?;
        Ok(())
    }

    // --- runs --------------------------------------------------------------

    pub async fn create_run(
        &self,
        workflow_id: &Id,
        workspace_id: &Id,
        input: &serde_json::Value,
    ) -> Result<WorkflowRun> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO workflow_runs (id, workflow_id, workspace_id, status, input_json,
                                        nodes_json, started_at)
             VALUES (?, ?, ?, 'pending', ?, '[]', ?)",
        )
        .bind(&id)
        .bind(workflow_id)
        .bind(workspace_id)
        .bind(input.to_string())
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create run"))?;
        self.get_run(&id).await
    }

    pub async fn get_run(&self, id: &Id) -> Result<WorkflowRun> {
        let r = sqlx::query("SELECT * FROM workflow_runs WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("run"))?;
        row_to_run(&r)
    }

    pub async fn list_runs(&self, workflow_id: &Id) -> Result<Vec<WorkflowRun>> {
        let rows = sqlx::query(
            "SELECT * FROM workflow_runs WHERE workflow_id = ? ORDER BY started_at DESC LIMIT 50",
        )
        .bind(workflow_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("runs"))?;
        rows.iter().map(row_to_run).collect()
    }

    // --- node output cache ------------------------------------------------

    /// Look up a cached node output by the composite natural key.
    /// Returns the stored JSON value when present; `None` on a miss.
    pub async fn get_cached_output(
        &self,
        workflow_id: &Id,
        node_id: &str,
        params_hash: &str,
        input_hash: &str,
    ) -> Option<serde_json::Value> {
        let row = sqlx::query(
            "SELECT output_json FROM workflow_node_cache
             WHERE workflow_id = ? AND node_id = ? AND params_hash = ? AND input_hash = ?",
        )
        .bind(workflow_id)
        .bind(node_id)
        .bind(params_hash)
        .bind(input_hash)
        .fetch_optional(&self.pool)
        .await
        .ok()??;
        let json_str: String = row.get("output_json");
        serde_json::from_str(&json_str).ok()
    }

    /// Upsert (insert-or-replace) a node output into the cache.
    pub async fn set_cached_output(
        &self,
        workflow_id: &Id,
        node_id: &str,
        params_hash: &str,
        input_hash: &str,
        output: &serde_json::Value,
    ) -> Result<()> {
        let id = new_id();
        let now = fmt(Utc::now());
        let output_json =
            serde_json::to_string(output).map_err(|e| Error::Internal(e.to_string()))?;
        sqlx::query(
            "INSERT INTO workflow_node_cache
                 (id, workflow_id, node_id, params_hash, input_hash, output_json, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(workflow_id, node_id, params_hash, input_hash)
             DO UPDATE SET output_json = excluded.output_json",
        )
        .bind(&id)
        .bind(workflow_id)
        .bind(node_id)
        .bind(params_hash)
        .bind(input_hash)
        .bind(&output_json)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("set node cache"))?;
        Ok(())
    }

    /// Persist run progress: status, the per-node states, optional error, and
    /// (when terminal) the finished timestamp.
    pub async fn update_run(
        &self,
        id: &Id,
        status: RunStatus,
        nodes: &[NodeRunState],
        error: Option<&str>,
        finished: bool,
    ) -> Result<()> {
        let nodes_json =
            serde_json::to_string(nodes).map_err(|e| Error::Internal(e.to_string()))?;
        let finished_at = if finished { Some(fmt(Utc::now())) } else { None };
        sqlx::query(
            "UPDATE workflow_runs
             SET status = ?, nodes_json = ?, error = ?,
                 finished_at = COALESCE(?, finished_at)
             WHERE id = ?",
        )
        .bind(status.as_str())
        .bind(&nodes_json)
        .bind(error)
        .bind(&finished_at)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("update run"))?;
        Ok(())
    }
}
