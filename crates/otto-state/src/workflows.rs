//! Workflows repository: workflow definitions + their run history.

use chrono::Utc;
use otto_core::workflows::{
    ActiveWorkflowRun, NodeRunState, NodeStatus, RunStatus, Workflow, WorkflowGraph, WorkflowRun,
    WorkflowVersion,
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
        instructions: r.try_get("instructions").unwrap_or_default(),
        graph: parse_graph(&r.get::<String, _>("graph_json"))?,
        created_by: r.get("created_by"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
        updated_at: ts(&r.get::<String, _>("updated_at"))?,
        version: r.try_get("version").unwrap_or(1),
        on_restart: r
            .try_get("on_restart")
            .unwrap_or_else(|_| otto_core::workflows::default_on_restart()),
    })
}

fn row_to_version(r: &sqlx::sqlite::SqliteRow) -> Result<WorkflowVersion> {
    Ok(WorkflowVersion {
        id: r.get("id"),
        workflow_id: r.get("workflow_id"),
        version: r.get("version"),
        name: r.get("name"),
        description: r.get("description"),
        instructions: r.try_get("instructions").unwrap_or_default(),
        graph: parse_graph(&r.get::<String, _>("graph_json"))?,
        note: r.get("note"),
        on_restart: r
            .try_get("on_restart")
            .unwrap_or_else(|_| otto_core::workflows::default_on_restart()),
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
    let approved_at: Option<String> = r.try_get("approved_at").ok().flatten();
    let waiting: i64 = r.try_get("waiting_approval").unwrap_or(0);
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
        rev: r.try_get("rev").unwrap_or(0),
        waiting_approval: waiting != 0,
        approval_node_id: r.try_get("approval_node_id").ok().flatten(),
        approved_by: r.try_get("approved_by").ok().flatten(),
        approval_note: r.try_get("approval_note").ok().flatten(),
        approved_at: approved_at.as_deref().map(ts).transpose()?,
        workflow_version: r.try_get("workflow_version").ok().flatten(),
        proof_pack_id: r.try_get("proof_pack_id").ok().flatten(),
        resume_attempts: r.try_get("resume_attempts").unwrap_or(0),
        // Derived at the API layer (routes/workflows.rs) from data_dir +
        // run id when the directory exists — never persisted.
        context_dir: None,
    })
}

fn row_to_active_run(r: &sqlx::sqlite::SqliteRow) -> Result<ActiveWorkflowRun> {
    let nodes: Vec<NodeRunState> =
        serde_json::from_str(&r.get::<String, _>("nodes_json")).unwrap_or_default();
    let nodes_total = nodes.len() as u32;
    let nodes_done = nodes
        .iter()
        .filter(|n| matches!(n.status, NodeStatus::Success | NodeStatus::Skipped))
        .count() as u32;
    let waiting: i64 = r.try_get("waiting_approval").unwrap_or(0);
    Ok(ActiveWorkflowRun {
        run_id: r.get("run_id"),
        workflow_id: r.get("workflow_id"),
        workspace_id: r.get("workspace_id"),
        workflow_name: r.get("workflow_name"),
        status: RunStatus::parse(&r.get::<String, _>("status"))
            .ok_or_else(|| Error::Internal("bad run status".into()))?,
        started_at: ts(&r.get::<String, _>("started_at"))?,
        nodes_total,
        nodes_done,
        waiting_approval: waiting != 0,
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
        instructions: &str,
        graph: &WorkflowGraph,
        created_by: &Id,
    ) -> Result<Workflow> {
        let id = new_id();
        let now = fmt(Utc::now());
        let graph_json =
            serde_json::to_string(graph).map_err(|e| Error::Internal(e.to_string()))?;
        sqlx::query(
            "INSERT INTO workflows (id, workspace_id, name, description, instructions, graph_json,
                                    created_by, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(workspace_id)
        .bind(name)
        .bind(description)
        .bind(instructions)
        .bind(&graph_json)
        .bind(created_by)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create workflow"))?;
        // Snapshot the initial version so every workflow has a v1 in history.
        self.snapshot_version(
            &id,
            1,
            name,
            description,
            instructions,
            graph,
            "initial",
            &otto_core::workflows::default_on_restart(),
            created_by,
        )
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

    /// Resolve a workflow by name **globally** (across all workspaces),
    /// case-insensitively. Workflows are a global library; a chat trigger that
    /// names a workflow finds it regardless of which workspace's integration
    /// received the message. Ties prefer `prefer_ws` (the message's workspace),
    /// then the most recently updated.
    pub async fn find_by_name(&self, name: &str, prefer_ws: &Id) -> Result<Option<Workflow>> {
        let row = sqlx::query(
            "SELECT * FROM workflows WHERE name = ? COLLATE NOCASE
             ORDER BY (workspace_id = ?) DESC, updated_at DESC LIMIT 1",
        )
        .bind(name.trim())
        .bind(prefer_ws)
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("find workflow by name"))?;
        row.map(|r| row_to_workflow(&r)).transpose()
    }

    pub async fn update(
        &self,
        id: &Id,
        name: Option<&str>,
        description: Option<&str>,
        instructions: Option<&str>,
        graph: Option<&WorkflowGraph>,
        on_restart: Option<&str>,
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
        if let Some(v) = instructions {
            sqlx::query("UPDATE workflows SET instructions = ?, updated_at = ? WHERE id = ?")
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
        if let Some(v) = on_restart {
            // The route validates the value; the column's CHECK is the backstop.
            sqlx::query("UPDATE workflows SET on_restart = ?, updated_at = ? WHERE id = ?")
                .bind(v)
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

    /// True when the workflow already has a pending/running run. The trigger
    /// schedulers use this as an overlap guard: a schedule tick or event storm
    /// must not stack concurrent runs of the same workflow (each provisioning
    /// its own worktrees).
    pub async fn has_active_run(&self, workflow_id: &Id) -> Result<bool> {
        let row: Option<String> = sqlx::query_scalar(
            "SELECT id FROM workflow_runs
             WHERE workflow_id = ? AND status IN ('pending','running') LIMIT 1",
        )
        .bind(workflow_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("has active run"))?;
        Ok(row.is_some())
    }

    /// Startup reconciliation: fail every run a dead daemon left EXECUTING
    /// (`running`), plus any `pending` run that already carries node progress —
    /// a reopened retry-a-step (re-running it blind would replay finished
    /// steps' side effects). FRESH `pending` rows (nodes_json still `[]`) are
    /// left untouched: they are the persistent run QUEUE, re-enqueued by
    /// `workflow_engine::resume_queued_runs`. Since 0108 this bulk hard-fail
    /// is only the fallback — `workflow_engine::reconcile_interrupted_runs`
    /// resumes runs per-row where the workflow's `on_restart` policy allows —
    /// but it remains the semantics for `on_restart = 'fail'`. Returns the
    /// rows updated.
    pub async fn fail_interrupted_runs(&self, error: &str) -> Result<u64> {
        let res = sqlx::query(
            "UPDATE workflow_runs
             SET status = 'error', error = ?, finished_at = COALESCE(finished_at, ?),
                 resume_scope_json = NULL, rev = rev + 1
             WHERE status = 'running' OR (status = 'pending' AND nodes_json != '[]')",
        )
        .bind(error)
        .bind(fmt(Utc::now()))
        .execute(&self.pool)
        .await
        .map_err(dberr("fail interrupted runs"))?;
        Ok(res.rows_affected())
    }

    /// The runs a dead daemon left in flight — EXECUTING (`running`) rows plus
    /// `pending` rows that already carry node progress (a reopened
    /// retry-a-step or a run the reconciler itself re-queued). Each row comes
    /// with its persisted `resume_scope_json` so the startup reconciler can
    /// re-enter with the exact scope the dead process was running. Oldest
    /// first, mirroring the queue's FIFO order.
    pub async fn interrupted_runs(&self) -> Result<Vec<(WorkflowRun, Option<String>)>> {
        let rows = sqlx::query(
            "SELECT * FROM workflow_runs
             WHERE status = 'running' OR (status = 'pending' AND nodes_json != '[]')
             ORDER BY started_at ASC, id ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("interrupted runs"))?;
        rows.iter()
            .map(|r| {
                let scope: Option<String> = r.try_get("resume_scope_json").ok().flatten();
                Ok((row_to_run(r)?, scope))
            })
            .collect()
    }

    /// Persist (or clear) the run's re-entry scope. Written by `spawn_run` the
    /// moment a scoped re-entry launches, so a daemon restart mid-retry can
    /// resume with the same scope instead of failing the run.
    pub async fn set_run_resume_scope(&self, id: &Id, scope_json: Option<&str>) -> Result<()> {
        sqlx::query("UPDATE workflow_runs SET resume_scope_json = ? WHERE id = ?")
            .bind(scope_json)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("set run resume scope"))?;
        Ok(())
    }

    /// Re-queue an interrupted run for a restart resume: back to `pending`
    /// with the reconciler's adjusted node states + re-entry scope, the
    /// resume counter bumped, and any stale approval pause cleared (the
    /// re-entered approval node re-parks itself). The engine's normal
    /// Pending→Running transition then owns the lifecycle. Bumps + returns
    /// `rev` for the announcing WS event.
    pub async fn prepare_resume(
        &self,
        id: &Id,
        nodes: &[NodeRunState],
        scope_json: &str,
    ) -> Result<i64> {
        let nodes_json =
            serde_json::to_string(nodes).map_err(|e| Error::Internal(e.to_string()))?;
        let rev: i64 = sqlx::query_scalar(
            "UPDATE workflow_runs
             SET status = 'pending', nodes_json = ?, resume_scope_json = ?,
                 interrupted_at = ?, resume_attempts = resume_attempts + 1,
                 error = NULL, finished_at = NULL,
                 waiting_approval = 0, approval_node_id = NULL,
                 rev = rev + 1
             WHERE id = ?
             RETURNING rev",
        )
        .bind(&nodes_json)
        .bind(scope_json)
        .bind(fmt(Utc::now()))
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .map_err(dberr("prepare resume"))?;
        Ok(rev)
    }

    /// Ids of QUEUED runs — fresh `pending`, no node progress — oldest first:
    /// the FIFO order `resume_queued_runs` re-enqueues them in after a restart.
    pub async fn queued_run_ids(&self) -> Result<Vec<Id>> {
        sqlx::query_scalar::<_, String>(
            "SELECT id FROM workflow_runs
             WHERE status = 'pending' AND nodes_json = '[]'
             ORDER BY started_at ASC, id ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("queued run ids"))
    }

    /// In-flight runs (pending|running) across a workspace, newest first, joined
    /// with their workflow name and with per-run step progress pre-computed.
    /// Backs the "Running" sidebar list (`GET /workspaces/{wid}/workflow-runs/active`).
    pub async fn list_active_runs(&self, workspace_id: &Id) -> Result<Vec<ActiveWorkflowRun>> {
        let rows = sqlx::query(
            "SELECT r.id AS run_id, r.workflow_id, r.workspace_id, r.status,
                    r.started_at, r.nodes_json, r.waiting_approval, w.name AS workflow_name
             FROM workflow_runs r
             JOIN workflows w ON w.id = r.workflow_id
             WHERE r.workspace_id = ? AND r.status IN ('pending','running')
             ORDER BY r.started_at DESC",
        )
        .bind(workspace_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("active runs"))?;
        rows.iter().map(row_to_active_run).collect()
    }

    /// Run ids of ALL in-flight runs (pending|running) across every workspace,
    /// newest first (bounded). Workflows are global, so a chat reply's origin
    /// workspace can differ from the run's own workspace — the caller matches a
    /// run to a thread by its input's channel/chat/thread. Backs chat control
    /// (`status`/`skip`/`abort`) of a running workflow from its thread.
    pub async fn list_active_run_ids_global(&self) -> Result<Vec<Id>> {
        let rows = sqlx::query_scalar::<_, String>(
            "SELECT id FROM workflow_runs
             WHERE status IN ('pending','running')
             ORDER BY started_at DESC LIMIT 200",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("active run ids"))?;
        Ok(rows)
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
        instructions: &str,
        graph: &WorkflowGraph,
        note: &str,
        on_restart: &str,
        created_by: &Id,
    ) -> Result<()> {
        let id = new_id();
        let now = fmt(Utc::now());
        let graph_json =
            serde_json::to_string(graph).map_err(|e| Error::Internal(e.to_string()))?;
        sqlx::query(
            "INSERT INTO workflow_versions
                 (id, workflow_id, version, name, description, instructions, graph_json, note,
                  on_restart, created_by, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(workflow_id, version) DO NOTHING",
        )
        .bind(&id)
        .bind(workflow_id)
        .bind(version)
        .bind(name)
        .bind(description)
        .bind(instructions)
        .bind(&graph_json)
        .bind(note)
        .bind(on_restart)
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
    /// (when terminal) the finished timestamp. Bumps and returns the run's
    /// monotonic `rev` so callers can stamp the change's WS event with it.
    pub async fn update_run(
        &self,
        id: &Id,
        status: RunStatus,
        nodes: &[NodeRunState],
        error: Option<&str>,
        finished: bool,
    ) -> Result<i64> {
        let nodes_json =
            serde_json::to_string(nodes).map_err(|e| Error::Internal(e.to_string()))?;
        let finished_at = if finished { Some(fmt(Utc::now())) } else { None };
        let rev: i64 = sqlx::query_scalar(
            // A terminal write (finished) also clears the persisted re-entry
            // scope — it only ever describes an IN-FLIGHT (re-)entry.
            "UPDATE workflow_runs
             SET status = ?, nodes_json = ?, error = ?,
                 finished_at = COALESCE(?, finished_at),
                 resume_scope_json = CASE WHEN ? IS NULL
                                          THEN resume_scope_json ELSE NULL END,
                 rev = rev + 1
             WHERE id = ?
             RETURNING rev",
        )
        .bind(status.as_str())
        .bind(&nodes_json)
        .bind(error)
        .bind(&finished_at)
        .bind(&finished_at)
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .map_err(dberr("update run"))?;
        Ok(rev)
    }

    /// Re-open a FINISHED run for a retry-a-step re-entry: back to `pending`
    /// with `finished_at`/`error` cleared, so the engine's normal
    /// Pending→Running transition + finalize stamp a fresh lifecycle. Guarded
    /// against live runs — the engine loop owning a running run must never be
    /// raced by a second one.
    pub async fn reopen_run(&self, id: &Id) -> Result<()> {
        let n = sqlx::query(
            "UPDATE workflow_runs SET status = 'pending', finished_at = NULL, error = NULL,
             rev = rev + 1
             WHERE id = ? AND status IN ('success','error','canceled')",
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("reopen run"))?
        .rows_affected();
        if n == 0 {
            return Err(Error::Conflict("run is still active".into()));
        }
        Ok(())
    }

    /// Persist per-node progress WITHOUT touching the run's lifecycle `status`
    /// or `finished_at`. The engine calls this for its routine in-loop progress
    /// writes so a concurrent Cancel (the API flips `status` to Canceled) is
    /// never silently resurrected back to Running by a progress save — the old
    /// `update_run(.., Running, ..)` did a bare `SET status = ?` and clobbered
    /// it, so a canceled run "came back" on the next node. Bumps + returns the
    /// monotonic `rev` like [`update_run`], so callers can still stamp the WS
    /// event with it.
    pub async fn update_run_progress(&self, id: &Id, nodes: &[NodeRunState]) -> Result<i64> {
        let nodes_json =
            serde_json::to_string(nodes).map_err(|e| Error::Internal(e.to_string()))?;
        let rev: i64 = sqlx::query_scalar(
            "UPDATE workflow_runs
             SET nodes_json = ?, rev = rev + 1
             WHERE id = ?
             RETURNING rev",
        )
        .bind(&nodes_json)
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .map_err(dberr("update run progress"))?;
        Ok(rev)
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
            .create(&"ws1".into(), "WF", "desc", "", &g0, &"u1".into())
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
        repo.snapshot_version(&wf.id, v, "WF", "desc", "", &g2, "edited graph", "resume", &"u1".into())
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
    async fn reopen_run_only_reopens_finished_runs() {
        let pool = mem_pool().await;
        let repo = WorkflowsRepo::new(pool);
        let g = WorkflowGraph::default();
        let wf = repo.create(&"ws1".into(), "WF", "", "", &g, &"u1".into()).await.unwrap();
        let run = repo
            .create_run(&wf.id, &wf.workspace_id, &serde_json::Value::Null)
            .await
            .unwrap();

        // Live run → Conflict (the engine loop owns it).
        repo.update_run(&run.id, RunStatus::Running, &[], None, false).await.unwrap();
        assert!(repo.reopen_run(&run.id).await.is_err());

        // Finished (error) run → reopens to pending with error/finished_at cleared.
        repo.update_run(&run.id, RunStatus::Error, &[], Some("boom"), true).await.unwrap();
        repo.reopen_run(&run.id).await.unwrap();
        let r = repo.get_run(&run.id).await.unwrap();
        assert_eq!(r.status, RunStatus::Pending);
        assert!(r.error.is_none());
        assert!(r.finished_at.is_none());
    }

    #[tokio::test]
    async fn startup_reap_fails_started_runs_but_keeps_the_queue() {
        let pool = mem_pool().await;
        let repo = WorkflowsRepo::new(pool);
        let g = WorkflowGraph::default();
        let wf = repo.create(&"ws1".into(), "WF", "", "", &g, &"u1".into()).await.unwrap();
        let mk = || repo.create_run(&wf.id, &wf.workspace_id, &serde_json::Value::Null);

        // r1: EXECUTING when the daemon died → must fail.
        let r1 = mk().await.unwrap();
        repo.update_run(&r1.id, RunStatus::Running, &[], None, false).await.unwrap();
        // r2: fresh pending (queued behind the run gate) → must SURVIVE.
        let r2 = mk().await.unwrap();
        // r3: a reopened retry-a-step (pending but carrying node progress) —
        // its retry scope lived only in the dead process's memory → must fail.
        let r3 = mk().await.unwrap();
        let node = NodeRunState {
            node_id: "a".into(),
            status: NodeStatus::Error,
            output: None,
            error: Some("boom".into()),
            logs: vec![],
            started_at: None,
            duration_ms: None,
            attempts: None,
            sessions: vec![],
        };
        repo.update_run(&r3.id, RunStatus::Error, &[node], Some("boom"), true).await.unwrap();
        repo.reopen_run(&r3.id).await.unwrap();

        assert_eq!(repo.fail_interrupted_runs("interrupted").await.unwrap(), 2);
        assert_eq!(repo.get_run(&r1.id).await.unwrap().status, RunStatus::Error);
        assert_eq!(repo.get_run(&r3.id).await.unwrap().status, RunStatus::Error);
        // The queued run is untouched and is exactly what re-enqueues.
        assert_eq!(repo.get_run(&r2.id).await.unwrap().status, RunStatus::Pending);
        assert_eq!(repo.queued_run_ids().await.unwrap(), vec![r2.id.clone()]);
    }

    #[tokio::test]
    async fn queued_run_ids_are_fifo_by_creation() {
        let pool = mem_pool().await;
        let repo = WorkflowsRepo::new(pool);
        let g = WorkflowGraph::default();
        let wf = repo.create(&"ws1".into(), "WF", "", "", &g, &"u1".into()).await.unwrap();
        let a = repo.create_run(&wf.id, &wf.workspace_id, &serde_json::Value::Null).await.unwrap();
        let b = repo.create_run(&wf.id, &wf.workspace_id, &serde_json::Value::Null).await.unwrap();
        let c = repo.create_run(&wf.id, &wf.workspace_id, &serde_json::Value::Null).await.unwrap();
        // A run that already started is not queued.
        repo.update_run(&b.id, RunStatus::Running, &[], None, false).await.unwrap();
        // Same-timestamp rows tiebreak on id; ULIDs are creation-ordered, so
        // FIFO order holds either way.
        assert_eq!(repo.queued_run_ids().await.unwrap(), vec![a.id.clone(), c.id.clone()]);
    }

    #[tokio::test]
    async fn find_by_name_is_global_and_prefers_workspace() {
        let pool = mem_pool().await;
        let repo = WorkflowsRepo::new(pool);
        let g = WorkflowGraph::default();
        let a = repo.create(&"wsA".into(), "Write tests", "", "", &g, &"u".into()).await.unwrap();
        let b = repo.create(&"wsB".into(), "Write tests", "", "", &g, &"u".into()).await.unwrap();

        // Global resolution finds it from a third workspace; case-insensitive.
        let any = repo.find_by_name("write TESTS", &"wsC".into()).await.unwrap();
        assert!(any.is_some(), "resolves across all workspaces");

        // Ties prefer the requested workspace.
        assert_eq!(repo.find_by_name("Write tests", &"wsA".into()).await.unwrap().unwrap().id, a.id);
        assert_eq!(repo.find_by_name("Write tests", &"wsB".into()).await.unwrap().unwrap().id, b.id);
        assert!(repo.find_by_name("nope", &"wsA".into()).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn instructions_round_trip_and_version_snapshot() {
        let pool = mem_pool().await;
        let repo = WorkflowsRepo::new(pool);
        let ws: Id = "ws1".into();
        let wf = repo
            .create(&ws, "generic", "d", "FOLLOW THE RULES", &WorkflowGraph { nodes: vec![], edges: vec![] }, &"u1".into())
            .await
            .unwrap();
        assert_eq!(wf.instructions, "FOLLOW THE RULES");
        let up = repo.update(&wf.id, None, None, Some("v2 rules"), None, None).await.unwrap();
        assert_eq!(up.instructions, "v2 rules");
        let versions = repo.list_versions(&wf.id).await.unwrap();
        assert_eq!(versions.last().unwrap().instructions, "FOLLOW THE RULES"); // v1 snapshot
    }

    #[tokio::test]
    async fn run_records_version_and_proof_pack() {
        let pool = mem_pool().await;
        let repo = WorkflowsRepo::new(pool);
        let wf = repo
            .create(&"ws1".into(), "WF", "", "", &WorkflowGraph::default(), &"u1".into())
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

    #[tokio::test]
    async fn update_run_bumps_a_monotonic_rev() {
        let pool = mem_pool().await;
        let repo = WorkflowsRepo::new(pool);
        let wf = repo
            .create(&"ws1".into(), "WF", "", "", &WorkflowGraph::default(), &"u1".into())
            .await
            .unwrap();
        let run = repo
            .create_run(&wf.id, &"ws1".into(), &serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(run.rev, 0, "fresh run starts at rev 0");

        // Every progress write returns the next rev, and get_run round-trips it.
        let r1 = repo.update_run(&run.id, RunStatus::Running, &[], None, false).await.unwrap();
        let r2 = repo.update_run(&run.id, RunStatus::Running, &[], None, false).await.unwrap();
        let r3 = repo
            .update_run(&run.id, RunStatus::Success, &[], None, true)
            .await
            .unwrap();
        assert_eq!((r1, r2, r3), (1, 2, 3), "rev increments per write");
        let got = repo.get_run(&run.id).await.unwrap();
        assert_eq!(got.rev, 3);
        assert_eq!(got.status, RunStatus::Success);
    }

    #[tokio::test]
    async fn update_run_progress_never_resurrects_a_canceled_run() {
        let pool = mem_pool().await;
        let repo = WorkflowsRepo::new(pool);
        let wf = repo
            .create(&"ws1".into(), "WF", "", "", &WorkflowGraph::default(), &"u1".into())
            .await
            .unwrap();
        let run = repo
            .create_run(&wf.id, &"ws1".into(), &serde_json::json!({}))
            .await
            .unwrap();

        // Engine marks the run Running, then the API cancels it (terminal write).
        repo.update_run(&run.id, RunStatus::Running, &[], None, false).await.unwrap();
        repo.update_run(&run.id, RunStatus::Canceled, &[], Some("canceled"), true)
            .await
            .unwrap();

        // A routine progress save lands AFTER the cancel. It must bump rev but
        // leave the status Canceled — the old `update_run(.., Running, ..)`
        // would have clobbered it back to Running (the bug this fixes).
        let rev = repo.update_run_progress(&run.id, &[]).await.unwrap();
        let got = repo.get_run(&run.id).await.unwrap();
        assert_eq!(got.status, RunStatus::Canceled, "progress write must not resurrect a canceled run");
        assert_eq!((rev, got.rev), (3, 3), "progress write still bumps + round-trips the monotonic rev");
    }
}
