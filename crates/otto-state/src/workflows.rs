//! Workflows repository: workflow definitions + their run history.

use chrono::Utc;
use otto_core::workflows::{
    NodeRunState, RunStatus, Workflow, WorkflowGraph, WorkflowRun, WorkflowVersion,
};
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
        version: r.try_get("version").unwrap_or(1),
    })
}

fn row_to_version(r: &sqlx::sqlite::SqliteRow) -> Result<WorkflowVersion> {
    Ok(WorkflowVersion {
        id: r.get("id"),
        workflow_id: r.get("workflow_id"),
        version: r.get("version"),
        name: r.get("name"),
        description: r.get("description"),
        graph: parse_graph(&r.get::<String, _>("graph_json"))?,
        note: r.get("note"),
        created_by: r.get("created_by"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
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
        workflow_version: r.try_get("workflow_version").ok().flatten(),
        proof_pack_id: r.try_get("proof_pack_id").ok().flatten(),
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
        // Snapshot the initial version so every workflow has a v1 in history.
        self.snapshot_version(&id, 1, name, description, graph, "initial", created_by)
            .await?;
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

    // --- versioning -------------------------------------------------------

    /// Insert a version-history snapshot of a workflow's graph. Idempotent on the
    /// `(workflow_id, version)` unique key (re-snapshotting the same version is a
    /// no-op rather than an error).
    #[allow(clippy::too_many_arguments)]
    pub async fn snapshot_version(
        &self,
        workflow_id: &Id,
        version: i64,
        name: &str,
        description: &str,
        graph: &WorkflowGraph,
        note: &str,
        created_by: &Id,
    ) -> Result<()> {
        let id = new_id();
        let now = fmt(Utc::now());
        let graph_json =
            serde_json::to_string(graph).map_err(|e| Error::Internal(e.to_string()))?;
        sqlx::query(
            "INSERT INTO workflow_versions
                 (id, workflow_id, version, name, description, graph_json, note,
                  created_by, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(workflow_id, version) DO NOTHING",
        )
        .bind(&id)
        .bind(workflow_id)
        .bind(version)
        .bind(name)
        .bind(description)
        .bind(&graph_json)
        .bind(note)
        .bind(created_by)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("snapshot version"))?;
        Ok(())
    }

    /// All versions of a workflow, newest first.
    pub async fn list_versions(&self, workflow_id: &Id) -> Result<Vec<WorkflowVersion>> {
        let rows = sqlx::query(
            "SELECT * FROM workflow_versions WHERE workflow_id = ? ORDER BY version DESC",
        )
        .bind(workflow_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list versions"))?;
        rows.iter().map(row_to_version).collect()
    }

    /// A single version of a workflow, or `None` if it does not exist.
    pub async fn get_version(
        &self,
        workflow_id: &Id,
        version: i64,
    ) -> Result<Option<WorkflowVersion>> {
        let row = sqlx::query(
            "SELECT * FROM workflow_versions WHERE workflow_id = ? AND version = ?",
        )
        .bind(workflow_id)
        .bind(version)
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("get version"))?;
        row.map(|r| row_to_version(&r)).transpose()
    }

    /// The workflow's current version counter.
    pub async fn current_version(&self, workflow_id: &Id) -> Result<i64> {
        let row = sqlx::query("SELECT version FROM workflows WHERE id = ?")
            .bind(workflow_id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("current version"))?;
        Ok(row.try_get("version").unwrap_or(1))
    }

    /// Atomically bump the workflow's version counter, returning the new value.
    pub async fn bump_version(&self, workflow_id: &Id) -> Result<i64> {
        sqlx::query("UPDATE workflows SET version = version + 1 WHERE id = ?")
            .bind(workflow_id)
            .execute(&self.pool)
            .await
            .map_err(dberr("bump version"))?;
        self.current_version(workflow_id).await
    }

    /// Record which workflow version a run executed.
    pub async fn set_run_version(&self, run_id: &Id, version: i64) -> Result<()> {
        sqlx::query("UPDATE workflow_runs SET workflow_version = ? WHERE id = ?")
            .bind(version)
            .bind(run_id)
            .execute(&self.pool)
            .await
            .map_err(dberr("set run version"))?;
        Ok(())
    }

    /// Link a run to the Proof Pack assembled for it.
    pub async fn set_run_proof_pack(&self, run_id: &Id, proof_pack_id: &str) -> Result<()> {
        sqlx::query("UPDATE workflow_runs SET proof_pack_id = ? WHERE id = ?")
            .bind(proof_pack_id)
            .bind(run_id)
            .execute(&self.pool)
            .await
            .map_err(dberr("set run proof pack"))?;
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
    async fn versioning_snapshot_bump_restore_roundtrip() {
        let pool = mem_pool().await;
        let repo = WorkflowsRepo::new(pool);
        let g0 = WorkflowGraph::default();

        let wf = repo
            .create(&"ws1".into(), "WF", "desc", &g0, &"u1".into())
            .await
            .unwrap();
        assert_eq!(wf.version, 1, "new workflow starts at version 1");

        // create() snapshots v1.
        let versions = repo.list_versions(&wf.id).await.unwrap();
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].version, 1);
        assert_eq!(versions[0].note, "initial");

        // A graph-changing update bumps to v2 + snapshots it.
        let g2 = serde_json::from_value::<WorkflowGraph>(serde_json::json!({
            "nodes": [{"id":"a","kind":"manual_trigger"}], "edges": []
        }))
        .unwrap();
        let v = repo.bump_version(&wf.id).await.unwrap();
        assert_eq!(v, 2);
        repo.snapshot_version(&wf.id, v, "WF", "desc", &g2, "edited graph", &"u1".into())
            .await
            .unwrap();
        assert_eq!(repo.current_version(&wf.id).await.unwrap(), 2);

        let versions = repo.list_versions(&wf.id).await.unwrap();
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].version, 2, "newest first");

        let got = repo.get_version(&wf.id, 2).await.unwrap().unwrap();
        assert_eq!(got.graph.nodes.len(), 1);
        assert!(repo.get_version(&wf.id, 99).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn run_records_version_and_proof_pack() {
        let pool = mem_pool().await;
        let repo = WorkflowsRepo::new(pool);
        let wf = repo
            .create(&"ws1".into(), "WF", "", &WorkflowGraph::default(), &"u1".into())
            .await
            .unwrap();

        let run = repo
            .create_run(&wf.id, &"ws1".into(), &serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(run.workflow_version, None);
        assert_eq!(run.proof_pack_id, None);

        repo.set_run_version(&run.id, 1).await.unwrap();
        repo.set_run_proof_pack(&run.id, "pack-123").await.unwrap();

        let run = repo.get_run(&run.id).await.unwrap();
        assert_eq!(run.workflow_version, Some(1));
        assert_eq!(run.proof_pack_id.as_deref(), Some("pack-123"));
    }
}
