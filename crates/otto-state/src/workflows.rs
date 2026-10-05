//! Workflows repository: workflow definitions + their run history.

use crate::DbPool;
use chrono::Utc;
use otto_core::workflows::{
    ActiveWorkflowRun, NodeRunState, NodeStatus, RunStatus, Workflow, WorkflowCheckpoint,
    WorkflowGraph, WorkflowRun, WorkflowVersion, WorkflowVersionSummary,
};
use otto_core::{new_id, Error, Id, Result};
use sqlx::Row;

use crate::convert::{dberr, fmt, ts};

/// Narrow view of a run for projections ([`WorkflowsRepo::run_head`]).
#[derive(Debug, Clone, PartialEq)]
pub struct RunHead {
    pub id: Id,
    pub workflow_id: Id,
    pub workspace_id: Id,
    pub status: String,
    pub error: Option<String>,
    /// Entries in `nodes_json`, when asked for.
    pub node_count: Option<u32>,
    /// `None` when the workflow row is gone.
    pub workflow_name: Option<String>,
}

#[derive(Clone)]
pub struct WorkflowsRepo {
    pool: DbPool,
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
    let input: serde_json::Value =
        serde_json::from_str(&r.get::<String, _>("input_json")).unwrap_or(serde_json::Value::Null);
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
        checkpoints: vec![],
        error: r.get("error"),
        started_at: ts(&r.get::<String, _>("started_at"))?,
        finished_at: finished.as_deref().map(ts).transpose()?,
        rev: r.try_get("rev").unwrap_or(0),
        waiting_approval: waiting != 0,
        approval_node_id: r.try_get("approval_node_id").ok().flatten(),
        approved_by: r.try_get("approved_by").ok().flatten(),
        created_by: r.try_get("created_by").ok().flatten(),
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
    // Only the per-node status is needed. `nodes_json` here is the small
    // `progress_json` projection whenever it is published (it carries the same
    // `status` per node, logs/output stripped); the full body is the fallback.
    #[derive(serde::Deserialize)]
    struct NodeStatusOnly {
        status: NodeStatus,
    }
    let nodes: Vec<NodeStatusOnly> =
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
    pub async fn checkpoint(
        &self,
        run_id: &Id,
        node_id: &str,
    ) -> Result<Option<WorkflowCheckpoint>> {
        let json: Option<String> = sqlx::query_scalar(
            "SELECT checkpoint_json FROM workflow_checkpoints WHERE run_id = ? AND node_id = ?",
        )
        .bind(run_id)
        .bind(node_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("read loop checkpoint"))?;
        json.map(|json| {
            serde_json::from_str(&json)
                .map_err(|e| Error::Internal(format!("invalid loop checkpoint: {e}")))
        })
        .transpose()
    }

    pub async fn checkpoints(&self, run_id: &Id) -> Result<Vec<WorkflowCheckpoint>> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT checkpoint_json FROM workflow_checkpoints WHERE run_id = ? ORDER BY node_id",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("read loop checkpoints"))?;
        rows.into_iter()
            .map(|(json,)| {
                serde_json::from_str(&json)
                    .map_err(|e| Error::Internal(format!("invalid loop checkpoint: {e}")))
            })
            .collect()
    }

    pub async fn save_checkpoint(
        &self,
        run_id: &Id,
        checkpoint: &WorkflowCheckpoint,
    ) -> Result<()> {
        let json = serde_json::to_string(checkpoint).map_err(|e| Error::Internal(e.to_string()))?;
        let projection = crate::workflow_progress::checkpoint_projection(checkpoint)?;
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(dberr("begin checkpoint write"))?;
        sqlx::query("INSERT INTO workflow_checkpoints (run_id, node_id, checkpoint_json) VALUES (?, ?, ?) ON CONFLICT(run_id, node_id) DO UPDATE SET checkpoint_json = excluded.checkpoint_json")
            .bind(run_id).bind(&checkpoint.node_id).bind(json).execute(&mut *tx).await.map_err(dberr("save loop checkpoint"))?;
        crate::workflow_progress::publish_checkpoint(
            &mut tx,
            run_id,
            &checkpoint.node_id,
            &projection,
        )
        .await?;
        tx.commit()
            .await
            .map_err(dberr("commit checkpoint write"))?;
        Ok(())
    }

    /// Atomically acknowledge a retry, persist its scope and reset only selected
    /// failed inner attempts. A competing retry cannot reset an active run.
    pub async fn prepare_retry(
        &self,
        run_id: &Id,
        node_ids: &[String],
        replay: bool,
        scope: &otto_core::workflows::RunScope,
    ) -> Result<()> {
        let scope_json =
            serde_json::to_string(scope).map_err(|e| Error::Internal(e.to_string()))?;
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(dberr("begin workflow retry"))?;
        let changed = sqlx::query("UPDATE workflow_runs SET status = 'pending', finished_at = NULL, error = NULL, resume_scope_json = ?, waiting_approval = 0, approval_node_id = NULL, resume_attempts = 0, rev = rev + 1, checkpoint_generation = checkpoint_generation + 1 WHERE id = ? AND status IN ('success','error','canceled')")
            .bind(scope_json).bind(run_id).execute(&mut *tx).await.map_err(dberr("prepare workflow retry"))?.rows_affected();
        if changed == 0 {
            return Err(Error::Conflict("run is still active".into()));
        }
        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT checkpoint_json FROM workflow_checkpoints WHERE run_id = ?")
                .bind(run_id)
                .fetch_all(&mut *tx)
                .await
                .map_err(dberr("read loop checkpoints"))?;
        for (json,) in rows {
            let mut cp: WorkflowCheckpoint =
                serde_json::from_str(&json).map_err(|e| Error::Internal(e.to_string()))?;
            if !node_ids
                .iter()
                .any(|id| cp.node_id == *id || cp.node_id.starts_with(&format!("{id}#")))
            {
                continue;
            }
            if replay {
                sqlx::query("DELETE FROM workflow_checkpoints WHERE run_id = ? AND node_id = ?")
                    .bind(run_id)
                    .bind(&cp.node_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(dberr("reset loop checkpoint"))?;
            } else if matches!(cp.status, NodeStatus::Error | NodeStatus::Running) {
                cp.status = NodeStatus::Pending;
                cp.attempts = 0;
                cp.error = None;
                let json =
                    serde_json::to_string(&cp).map_err(|e| Error::Internal(e.to_string()))?;
                sqlx::query("UPDATE workflow_checkpoints SET checkpoint_json = ? WHERE run_id = ? AND node_id = ?")
                    .bind(json).bind(run_id).bind(&cp.node_id).execute(&mut *tx).await.map_err(dberr("reset loop checkpoint"))?;
                let projection = crate::workflow_progress::checkpoint_projection(&cp)?;
                crate::workflow_progress::publish_checkpoint(
                    &mut tx,
                    run_id,
                    &cp.node_id,
                    &projection,
                )
                .await?;
            }
        }
        tx.commit().await.map_err(dberr("commit workflow retry"))?;
        Ok(())
    }

    pub fn new(pool: impl Into<DbPool>) -> Self {
        let pool: DbPool = pool.into();
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
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(dberr("begin create workflow"))?;
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
        .execute(&mut *tx)
        .await
        .map_err(dberr("create workflow"))?;
        Self::snapshot_live(&mut tx, &id, "initial", Some(created_by), None).await?;
        let row = sqlx::query("SELECT * FROM workflows WHERE id=?")
            .bind(&id)
            .fetch_one(&mut *tx)
            .await
            .map_err(dberr("created workflow"))?;
        let wf = row_to_workflow(&row)?;
        tx.commit().await.map_err(dberr("commit create workflow"))?;
        Ok(wf)
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
        let rows =
            sqlx::query("SELECT * FROM workflows WHERE workspace_id = ? ORDER BY updated_at DESC")
                .bind(ws)
                .fetch_all(&self.pool)
                .await
                .map_err(dberr("workflows"))?;
        rows.iter().map(row_to_workflow).collect()
    }

    /// Resolve a workflow by name within ONE workspace, case-insensitively
    /// (most recently updated wins a tie). Chat triggers resolve through this
    /// with the receiving integration's workspace: a member of workspace A's
    /// channel must never start workspace B's workflow — running as B's
    /// creator, posting A's channel content into B's run (S3-03).
    pub async fn find_by_name(&self, name: &str, workspace_id: &Id) -> Result<Option<Workflow>> {
        let row = sqlx::query(
            "SELECT * FROM workflows WHERE name = ? COLLATE NOCASE AND workspace_id = ?
             ORDER BY updated_at DESC LIMIT 1",
        )
        .bind(name.trim())
        .bind(workspace_id)
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
        self.publish(
            id,
            name,
            description,
            instructions,
            graph,
            on_restart,
            "edited",
            None,
            None,
        )
        .await
    }

    /// Publish a patch and its immutable definition together. Name/description
    /// edits alone keep the existing version, matching the API's prior behavior.
    /// Restore may record historical labels while preserving the live labels.
    #[allow(clippy::too_many_arguments)]
    pub async fn publish(
        &self,
        id: &Id,
        name: Option<&str>,
        description: Option<&str>,
        instructions: Option<&str>,
        graph: Option<&WorkflowGraph>,
        on_restart: Option<&str>,
        note: &str,
        actor: Option<&Id>,
        snapshot_labels: Option<(&str, &str)>,
    ) -> Result<Workflow> {
        let versioned = instructions.is_some() || graph.is_some() || on_restart.is_some();
        let graph_json = graph
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| Error::Internal(e.to_string()))?;
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(dberr("begin workflow publication"))?;
        // UPDATE is the first statement, acquiring the write lock before any
        // row/version read. A competing save or run sees only committed pairs.
        let row = sqlx::query("UPDATE workflows SET name=COALESCE(?,name), description=COALESCE(?,description), instructions=COALESCE(?,instructions), graph_json=COALESCE(?,graph_json), on_restart=COALESCE(?,on_restart), version=version+?, updated_at=? WHERE id=? RETURNING *")
            .bind(name).bind(description).bind(instructions).bind(graph_json).bind(on_restart)
            .bind(i64::from(versioned)).bind(fmt(Utc::now())).bind(id)
            .fetch_one(&mut *tx).await.map_err(dberr("publish workflow"))?;
        let wf = row_to_workflow(&row)?;
        if versioned {
            Self::snapshot_live(&mut tx, id, note, actor, snapshot_labels).await?;
        }
        tx.commit()
            .await
            .map_err(dberr("commit workflow publication"))?;
        Ok(wf)
    }

    async fn snapshot_live(
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        id: &Id,
        note: &str,
        actor: Option<&Id>,
        labels: Option<(&str, &str)>,
    ) -> Result<()> {
        // No conflict-ignore: an existing version is an invariant violation and
        // must roll the entire publication back, not silently lose this edit.
        sqlx::query("INSERT INTO workflow_versions(id,workflow_id,version,name,description,instructions,graph_json,note,on_restart,created_by,created_at) SELECT ?,id,version,COALESCE(?,name),COALESCE(?,description),instructions,graph_json,?,on_restart,COALESCE(?,created_by),? FROM workflows WHERE id=?")
            .bind(new_id()).bind(labels.map(|v|v.0)).bind(labels.map(|v|v.1)).bind(note).bind(actor).bind(fmt(Utc::now())).bind(id)
            .execute(&mut **tx).await.map_err(dberr("snapshot published workflow"))?;
        Ok(())
    }

    /// Delete a workflow — refused (`Conflict`) while it has a live
    /// (`pending`/`running`) run. `ON DELETE CASCADE` would otherwise remove
    /// the run row out from under its engine driver, which then could no
    /// longer be canceled (404) and kept spawning sessions invisibly (S17-01).
    /// The live-run check and the DELETE are one statement, so a run admitted
    /// concurrently either blocks the delete or is never admitted (its INSERT
    /// sees no parent row).
    pub async fn delete(&self, id: &Id) -> Result<()> {
        let done = sqlx::query(
            "DELETE FROM workflows WHERE id = ?
             AND NOT EXISTS (SELECT 1 FROM workflow_runs
                             WHERE workflow_id = ? AND status IN ('pending','running'))",
        )
        .bind(id)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("delete workflow"))?;
        if done.rows_affected() == 0 {
            let live: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM workflow_runs
                 WHERE workflow_id = ? AND status IN ('pending','running')",
            )
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("delete workflow live runs"))?;
            if live > 0 {
                return Err(Error::Conflict(format!(
                    "this workflow has {live} active run(s) — cancel them before deleting it"
                )));
            }
        }
        Ok(())
    }

    // --- runs --------------------------------------------------------------

    /// `created_by` = the user who started the run (the engine acts as them
    /// when spawning agent sessions); `None` for trigger / schedule / chat runs.
    pub async fn create_run(
        &self,
        workflow_id: &Id,
        workspace_id: &Id,
        input: &serde_json::Value,
        created_by: Option<&Id>,
    ) -> Result<WorkflowRun> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(dberr("begin workflow run"))?;
        let run = Self::insert_run(&mut tx, workflow_id, workspace_id, input, created_by).await?;
        tx.commit().await.map_err(dberr("commit workflow run"))?;
        Ok(run)
    }

    /// Admit a run only while the workflow has no live (`pending`/`running`)
    /// run — the check and the insert share one `BEGIN IMMEDIATE` write
    /// transaction, so two admitters (a workflow-kind scheduled task, an event
    /// trigger, the schedule-trigger claim) can never both pass the check and
    /// stack concurrent runs. `Ok(None)` = a run is already in flight.
    pub async fn admit_run_if_idle(
        &self,
        workflow_id: &Id,
        workspace_id: &Id,
        input: &serde_json::Value,
        created_by: Option<&Id>,
    ) -> Result<Option<WorkflowRun>> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(dberr("begin idle workflow admission"))?;
        let busy: Option<String> = sqlx::query_scalar(
            "SELECT id FROM workflow_runs
             WHERE workflow_id = ? AND status IN ('pending','running') LIMIT 1",
        )
        .bind(workflow_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(dberr("idle admission check"))?;
        if busy.is_some() {
            return Ok(None);
        }
        let run = Self::insert_run(&mut tx, workflow_id, workspace_id, input, created_by).await?;
        tx.commit()
            .await
            .map_err(dberr("commit idle workflow admission"))?;
        Ok(Some(run))
    }

    /// Shared transactional insertion for ordinary run admission and atomic
    /// trigger claims. The caller commits its own surrounding transaction.
    pub(crate) async fn insert_run(
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        workflow_id: &Id,
        workspace_id: &Id,
        input: &serde_json::Value,
        created_by: Option<&Id>,
    ) -> Result<WorkflowRun> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO workflow_runs (id, workflow_id, workspace_id, status, input_json,
                                        nodes_json, started_at, created_by, workflow_version)
             VALUES (?, ?, ?, 'pending', ?, '[]', ?, ?, (SELECT version FROM workflows WHERE id = ?))",
        )
        .bind(&id)
        .bind(workflow_id)
        .bind(workspace_id)
        .bind(input.to_string())
        .bind(&now)
        .bind(created_by)
        .bind(workflow_id)
        .execute(&mut **tx)
        .await
        .map_err(dberr("create run"))?;
        crate::workflow_progress::publish_nodes(tx, &id, "[]").await?;
        let row = sqlx::query("SELECT * FROM workflow_runs WHERE id=?")
            .bind(&id)
            .fetch_one(&mut **tx)
            .await
            .map_err(dberr("created workflow run"))?;
        row_to_run(&row)
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

    /// The few fields the work-graph projector needs about a run, WITHOUT
    /// parsing its `nodes_json` / `input_json` in Rust (r3-07-02: a running
    /// node's progress write fired a full `get_run` + `get` of the graph up to
    /// ~4×/s, O(run JSON) each). `node_count` uses SQLite's `json_array_length`
    /// only when `with_node_count` is set — the live path already knows it
    /// from the event. `None` when the run is gone.
    pub async fn run_head(&self, run_id: &Id, with_node_count: bool) -> Result<Option<RunHead>> {
        let sql = if with_node_count {
            "SELECT r.id, r.workflow_id, r.workspace_id, r.status, r.error, \
                    json_array_length(r.nodes_json) AS node_count, w.name AS workflow_name \
             FROM workflow_runs r LEFT JOIN workflows w ON w.id = r.workflow_id WHERE r.id = ?"
        } else {
            "SELECT r.id, r.workflow_id, r.workspace_id, r.status, r.error, \
                    NULL AS node_count, w.name AS workflow_name \
             FROM workflow_runs r LEFT JOIN workflows w ON w.id = r.workflow_id WHERE r.id = ?"
        };
        let row = sqlx::query(sql)
            .bind(run_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("run head"))?;
        Ok(row.map(|r| RunHead {
            id: r.get("id"),
            workflow_id: r.get("workflow_id"),
            workspace_id: r.get("workspace_id"),
            status: r.get("status"),
            error: r.get("error"),
            node_count: r
                .get::<Option<i64>, _>("node_count")
                .map(|n| n.max(0) as u32),
            workflow_name: r.get("workflow_name"),
        }))
    }

    /// Ids of a workflow's newest `limit` runs (reconcile sweeps) — no row
    /// bodies, unlike [`Self::list_runs`].
    pub async fn recent_run_ids(&self, workflow_id: &Id, limit: i64) -> Result<Vec<Id>> {
        sqlx::query_scalar(
            "SELECT id FROM workflow_runs WHERE workflow_id = ? ORDER BY started_at DESC LIMIT ?",
        )
        .bind(workflow_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("recent run ids"))
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

    pub async fn run_scope(&self, id: &Id) -> Result<otto_core::workflows::RunScope> {
        let value: Option<String> =
            sqlx::query_scalar("SELECT resume_scope_json FROM workflow_runs WHERE id = ?")
                .bind(id)
                .fetch_one(&self.pool)
                .await
                .map_err(dberr("read run scope"))?;
        value
            .map(|json| {
                serde_json::from_str(&json)
                    .map_err(|e| Error::Internal(format!("invalid run scope: {e}")))
            })
            .unwrap_or_else(|| Ok(Default::default()))
    }

    /// Re-queue an interrupted run for a restart resume: back to `pending`
    /// with the reconciler's adjusted node states + re-entry scope, the
    /// resume counter bumped, and any stale approval pause cleared (the
    /// re-entered approval node re-parks itself). The engine's normal
    /// Pending→Running transition then owns the lifecycle. Bumps + returns
    /// `rev` for the announcing WS event.
    ///
    /// `count_attempt` is false for a run that was parked at a human approval:
    /// re-entering an approval has no side effects, so a run waiting a day for
    /// sign-off used to exhaust its resume budget on the third reboot/deploy.
    pub async fn prepare_resume(
        &self,
        id: &Id,
        nodes: &[NodeRunState],
        scope_json: &str,
        count_attempt: bool,
    ) -> Result<i64> {
        let nodes_json =
            serde_json::to_string(nodes).map_err(|e| Error::Internal(e.to_string()))?;
        let rev: i64 = sqlx::query_scalar(
            "UPDATE workflow_runs
             SET status = 'pending', nodes_json = ?, progress_json = NULL,
                 resume_scope_json = ?,
                 interrupted_at = ?, resume_attempts = resume_attempts + ?,
                 error = NULL, finished_at = NULL,
                 waiting_approval = 0, approval_node_id = NULL,
                 rev = rev + 1
             WHERE id = ?
             RETURNING rev",
        )
        .bind(&nodes_json)
        .bind(scope_json)
        .bind(fmt(Utc::now()))
        .bind(count_attempt as i64)
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
                    r.started_at, COALESCE(r.progress_json, r.nodes_json) AS nodes_json,
                    r.waiting_approval, w.name AS workflow_name
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

    /// The live (`pending`/`running`) run a chat message controls: its input
    /// carries this `channel`/`chat` (and the message's workspace as origin or
    /// owner). An exact `thread` match wins, else the newest channel/chat
    /// match. Ids only, filtered in SQL — the old scan `SELECT *`-loaded every
    /// active run's 50–200 KB `nodes_json` per inbound message (S3-09).
    pub async fn find_active_run_for_chat(
        &self,
        workspace_id: &str,
        channel: &str,
        chat: &str,
        thread: Option<&str>,
    ) -> Result<Option<Id>> {
        sqlx::query_scalar::<_, String>(
            "SELECT id FROM workflow_runs
             WHERE status IN ('pending','running')
               AND json_valid(input_json)
               AND json_extract(input_json, '$.channel') = ?
               AND json_extract(input_json, '$.chat') = ?
               AND (json_extract(input_json, '$.origin_workspace_id') = ? OR workspace_id = ?)
             ORDER BY (json_extract(input_json, '$.thread') IS ?) DESC, started_at DESC
             LIMIT 1",
        )
        .bind(channel)
        .bind(chat)
        .bind(workspace_id)
        .bind(workspace_id)
        .bind(thread)
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("active run for chat"))
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

    /// Import an explicit history row. Duplicate version keys fail rather than
    /// hiding conflicting definitions. Live edits must use `publish` instead.
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
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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

    /// Bounded history pages. Summary queries never select/parse definition bodies.
    pub async fn version_summaries(
        &self,
        workflow_id: &Id,
        before: Option<i64>,
        limit: i64,
    ) -> Result<Vec<WorkflowVersionSummary>> {
        let rows = sqlx::query("SELECT id,workflow_id,version,note,created_by,created_at FROM workflow_versions WHERE workflow_id=? AND version<? ORDER BY version DESC LIMIT ?")
            .bind(workflow_id).bind(before.unwrap_or(i64::MAX)).bind(limit.clamp(1,100))
            .fetch_all(&self.pool).await.map_err(dberr("workflow version summaries"))?;
        rows.iter()
            .map(|r| {
                Ok(WorkflowVersionSummary {
                    id: r.get("id"),
                    workflow_id: r.get("workflow_id"),
                    version: r.get("version"),
                    note: r.get("note"),
                    created_by: r.get("created_by"),
                    created_at: ts(&r.get::<String, _>("created_at"))?,
                })
            })
            .collect()
    }

    pub async fn version_page(
        &self,
        workflow_id: &Id,
        before: Option<i64>,
        limit: i64,
    ) -> Result<Vec<WorkflowVersion>> {
        let rows = sqlx::query("SELECT * FROM workflow_versions WHERE workflow_id=? AND version<? ORDER BY version DESC LIMIT ?")
            .bind(workflow_id).bind(before.unwrap_or(i64::MAX)).bind(limit.clamp(1,100))
            .fetch_all(&self.pool).await.map_err(dberr("workflow version page"))?;
        rows.iter().map(row_to_version).collect()
    }

    /// A single version of a workflow, or `None` if it does not exist.
    pub async fn get_version(
        &self,
        workflow_id: &Id,
        version: i64,
    ) -> Result<Option<WorkflowVersion>> {
        let row =
            sqlx::query("SELECT * FROM workflow_versions WHERE workflow_id = ? AND version = ?")
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

    /// Load the immutable definition this run started with. Legacy rows without
    /// a version use the current definition; a missing recorded snapshot fails
    /// closed instead of silently running different instructions.
    pub async fn definition_for_run(&self, run: &WorkflowRun) -> Result<Workflow> {
        let mut wf = self.get(&run.workflow_id).await?;
        if let Some(version) = run.workflow_version {
            let snapshot = self
                .get_version(&run.workflow_id, version)
                .await?
                .ok_or_else(|| {
                    Error::NotFound(format!(
                        "workflow version {version} for run {} is missing",
                        run.id
                    ))
                })?;
            wf.name = snapshot.name;
            wf.description = snapshot.description;
            wf.instructions = snapshot.instructions;
            wf.graph = snapshot.graph;
            wf.on_restart = snapshot.on_restart;
            wf.version = snapshot.version;
        }
        Ok(wf)
    }

    /// Record which workflow version a run executed.
    pub async fn set_run_version(&self, run_id: &Id, version: i64) -> Result<()> {
        sqlx::query("UPDATE workflow_runs SET workflow_version = COALESCE(workflow_version, ?) WHERE id = ?")
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
        self.write_run(id, None, status, nodes, error, finished)
            .await?
            .ok_or_else(|| Error::NotFound(format!("workflow run {id}")))
    }

    /// [`update_run`] as a compare-and-set on the lifecycle: writes only while
    /// the row's status is one of `expected`, returning `None` (nothing
    /// written) otherwise. Every engine lifecycle transition goes through
    /// this so a late writer can never undo a newer decision — e.g. the
    /// Pending→Running start must not resurrect a run the user canceled while
    /// it was provisioning worktrees, and a finishing run must not overwrite a
    /// cancel that landed after its last node.
    pub async fn update_run_if(
        &self,
        id: &Id,
        expected: &[RunStatus],
        status: RunStatus,
        nodes: &[NodeRunState],
        error: Option<&str>,
        finished: bool,
    ) -> Result<Option<i64>> {
        self.write_run(id, Some(expected), status, nodes, error, finished)
            .await
    }

    async fn write_run(
        &self,
        id: &Id,
        expected: Option<&[RunStatus]>,
        status: RunStatus,
        nodes: &[NodeRunState],
        error: Option<&str>,
        finished: bool,
    ) -> Result<Option<i64>> {
        // Serialize + project off the runtime through the SAME memoized path
        // as `update_run_progress` (perf W10): every node start/finish and
        // status transition lands here, and re-projecting a big run inline
        // was tens of ms on a tokio worker.
        let (run, owned) = (id.to_string(), nodes.to_vec());
        let (nodes_json, projection) = tokio::task::spawn_blocking(move || {
            crate::workflow_progress::progress_write(&run, &owned)
        })
        .await
        .map_err(|e| Error::Internal(format!("workflow run write: {e}")))??;
        let finished_at = if finished {
            Some(fmt(Utc::now()))
        } else {
            None
        };
        // The status guard is built from `RunStatus::as_str` — fixed internal
        // literals, never caller text.
        let guard = match expected {
            None => String::new(),
            Some(list) => format!(
                " AND status IN ({})",
                list.iter()
                    .map(|s| format!("'{}'", s.as_str()))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        };
        // A terminal write (finished) also clears the persisted re-entry scope
        // — it only ever describes an IN-FLIGHT (re-)entry — and any approval
        // pause: a canceled/failed run must not keep a live "waiting for
        // approval" banner whose Approve button "resumes" a dead run, nor a
        // stale `approval_node_id` that a later retry's restart-resume would
        // mistake for its re-entry point.
        // The body AND its read projection in ONE statement (r3-07-03): the
        // 0130 trigger used to NULL the projection right after, and the
        // republish rewrote the row a third time — 3 full-row rewrites (the
        // overflow chain included) per write. Migration 0145 limits that
        // trigger to raw writes that don't bump `rev`.
        let sql = format!(
            "UPDATE workflow_runs
             SET status = ?, nodes_json = ?, progress_json = ?, error = ?,
                 finished_at = COALESCE(?, finished_at),
                 resume_scope_json = CASE WHEN ? IS NULL
                                          THEN resume_scope_json ELSE NULL END,
                 waiting_approval = CASE WHEN ? IS NULL
                                         THEN waiting_approval ELSE 0 END,
                 approval_node_id = CASE WHEN ? IS NULL
                                         THEN approval_node_id ELSE NULL END,
                 rev = rev + 1
             WHERE id = ?{guard}
             RETURNING rev"
        );
        let rev: Option<i64> = sqlx::query_scalar(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(status.as_str())
            .bind(&nodes_json)
            .bind(&projection)
            .bind(error)
            .bind(&finished_at)
            .bind(&finished_at)
            .bind(&finished_at)
            .bind(&finished_at)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("update run"))?;
        // `None`: guard not met (or no such row) — nothing was written.
        Ok(rev)
    }

    /// Request a cancel: flip an in-flight (`pending`/`running`) run to
    /// `canceled` WITHOUT touching `nodes_json`. The API/chat cancel used to
    /// write back the node snapshot it had read, so a run the engine finished
    /// between that read and the write got its final node states replaced by
    /// the stale snapshot (a dead step "running" forever) and its success
    /// overwritten. The engine's cancel poll owns the node states: it stops
    /// the in-flight node, marks the rest skipped and re-writes the run.
    /// Returns the bumped `rev`, or `None` when the run had already settled.
    pub async fn request_cancel(&self, id: &Id) -> Result<Option<i64>> {
        sqlx::query_scalar(
            "UPDATE workflow_runs
             SET status = 'canceled', error = 'canceled',
                 finished_at = ?,
                 resume_scope_json = NULL,
                 waiting_approval = 0, approval_node_id = NULL,
                 rev = rev + 1
             WHERE id = ? AND status IN ('pending','running')
             RETURNING rev",
        )
        .bind(fmt(Utc::now()))
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("cancel run"))
        .inspect(|rev| {
            if rev.is_some() {
                announce_cancel(id);
            }
        })
    }

    /// Lifecycle-only read of a run (perf W1): `(status, waiting_approval)`
    /// without loading or parsing `nodes_json` (live runs carry 50–200 KB).
    /// `None` when the run is gone.
    /// The run's original `input` alone (perf N5: the notify node read the
    /// whole run — `nodes_json` included — just for this). `Null` when the
    /// row is gone or the JSON is unreadable, like the old full read's
    /// fallback.
    pub async fn run_input(&self, id: &Id) -> Result<serde_json::Value> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT input_json FROM workflow_runs WHERE id = ?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await
                .map_err(dberr("run input"))?;
        Ok(row
            .and_then(|(j,)| serde_json::from_str(&j).ok())
            .unwrap_or(serde_json::Value::Null))
    }

    /// The user who started the run (`None` for trigger/schedule runs or a
    /// missing row) — one column, for the run start (perf N5).
    pub async fn run_created_by(&self, id: &Id) -> Result<Option<Id>> {
        let row: Option<(Option<String>,)> =
            sqlx::query_as("SELECT created_by FROM workflow_runs WHERE id = ?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await
                .map_err(dberr("run created_by"))?;
        Ok(row.and_then(|(c,)| c))
    }

    pub async fn run_status(&self, id: &Id) -> Result<Option<(RunStatus, bool)>> {
        let row: Option<(String, i64)> = sqlx::query_as(
            "SELECT status, COALESCE(waiting_approval, 0) FROM workflow_runs WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("run status"))?;
        Ok(row.and_then(|(s, w)| RunStatus::parse(&s).map(|st| (st, w != 0))))
    }

    /// True when the run's status is `canceled` — or the row is gone (its
    /// workflow was deleted): a driver whose run no longer exists must stop
    /// rather than keep spawning sessions nobody can see or cancel (S17-01).
    /// A transient DB error still reads as not canceled.
    pub async fn is_canceled(&self, id: &Id) -> bool {
        matches!(
            self.run_status(id).await,
            Ok(Some((RunStatus::Canceled, _)) | None)
        )
    }

    /// Re-open a FINISHED run for a retry-a-step re-entry: back to `pending`
    /// with `finished_at`/`error` cleared, so the engine's normal
    /// Pending→Running transition + finalize stamp a fresh lifecycle. Guarded
    /// against live runs — the engine loop owning a running run must never be
    /// raced by a second one.
    pub async fn reopen_run(&self, id: &Id) -> Result<()> {
        let n = sqlx::query(
            "UPDATE workflow_runs SET status = 'pending', finished_at = NULL, error = NULL,
             waiting_approval = 0, approval_node_id = NULL, resume_attempts = 0,
             rev = rev + 1, checkpoint_generation = checkpoint_generation + 1
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

    /// Record a decision only for the pending run/gate snapshot read here.
    /// Product gates require the full node-body version actually displayed by
    /// the client. A revision CAS binds the subsequent write to that exact
    /// read, so a reject/retry or gate replacement cannot reuse an old click.
    pub async fn record_approval(
        &self,
        id: &Id,
        node_id: &str,
        approved_by: Option<&Id>,
        note: &str,
        expected_detail_version: Option<&str>,
    ) -> Result<i64> {
        let row = sqlx::query(
            "SELECT rev, status, waiting_approval, approval_node_id,
                (SELECT value FROM json_each(nodes_json)
                 WHERE json_extract(value, '$.node_id') = ? LIMIT 1) AS node_json
             FROM workflow_runs WHERE id = ?",
        )
        .bind(node_id)
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .map_err(dberr("read pending approval"))?;
        if row.get::<String, _>("status") != "running" {
            return Err(Error::Conflict(
                "only a running run can be approved or rejected".into(),
            ));
        }
        if row.get::<i64, _>("waiting_approval") == 0 {
            return Err(Error::Invalid(
                "run is not currently waiting for approval".into(),
            ));
        }
        if row.get::<Option<String>, _>("approval_node_id").as_deref() != Some(node_id) {
            return Err(Error::Invalid(
                "approval node_id does not match the pending gate".into(),
            ));
        }
        let node = row
            .get::<Option<String>, _>("node_json")
            .map(|raw| serde_json::from_str::<serde_json::Value>(&raw))
            .transpose()
            .map_err(|e| Error::Internal(format!("approval node JSON: {e}")))?;
        let product_preview = node
            .as_ref()
            .and_then(|node| node.pointer("/output/publication_preview"))
            .is_some_and(|preview| !preview.is_null());
        if product_preview || expected_detail_version.is_some() {
            // Same Value serialization and SHA-256 as workflow_progress::detail.
            let version = node
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .map_err(|e| Error::Internal(format!("approval node version: {e}")))?
                .map(|json| otto_core::proof::content_sha256(&json));
            if expected_detail_version.is_none() || expected_detail_version != version.as_deref() {
                return Err(Error::Conflict("The publication preview changed or was not reviewed. Reload and review it before deciding.".into()));
            }
        }
        let revision: i64 = row.get("rev");
        sqlx::query_scalar(
            "UPDATE workflow_runs SET waiting_approval = 0, approved_by = ?,
                approval_note = ?, approved_at = ?, rev = rev + 1
             WHERE id = ? AND rev = ? AND status = 'running'
                AND waiting_approval = 1 AND approval_node_id = ?
             RETURNING rev",
        )
        .bind(approved_by)
        .bind(note)
        .bind(fmt(Utc::now()))
        .bind(id)
        .bind(revision)
        .bind(node_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("record approval"))?
        .ok_or_else(|| {
            Error::Conflict(
                "The pending approval changed. Reload and review it before deciding.".into(),
            )
        })
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
        // Serialize + project off the runtime: on a big run this is tens of ms
        // per write (the clone that moves it there is a fraction of that).
        let (run, nodes) = (id.to_string(), nodes.to_vec());
        let (nodes_json, projection) = tokio::task::spawn_blocking(move || {
            crate::workflow_progress::progress_write(&run, &nodes)
        })
        .await
        .map_err(|e| Error::Internal(format!("workflow progress write: {e}")))??;
        // One statement, one row rewrite: body + projection + rev together
        // (r3-07-03 — it was three rewrites; see `write_run`).
        let rev: i64 = sqlx::query_scalar(
            "UPDATE workflow_runs
             SET nodes_json = ?, progress_json = ?, rev = rev + 1
             WHERE id = ?
             RETURNING rev",
        )
        .bind(&nodes_json)
        .bind(&projection)
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .map_err(dberr("update run progress"))?;
        Ok(rev)
    }
}

/// Process-wide cancel announcements (perf W1): [`WorkflowsRepo::request_cancel`]
/// publishes the run id here when it flipped a run to `canceled`, so the
/// engine's running node wakes AT ONCE instead of re-reading the run row on a
/// timer. Every cancel path (API, chat, scheduled task) goes through
/// `request_cancel`, so none needs wiring. Receivers see every run's cancels
/// (they are rare) and filter by id; a lagged receiver must re-check status.
fn cancel_bus() -> &'static tokio::sync::broadcast::Sender<Id> {
    static BUS: std::sync::OnceLock<tokio::sync::broadcast::Sender<Id>> =
        std::sync::OnceLock::new();
    BUS.get_or_init(|| tokio::sync::broadcast::channel(64).0)
}

fn announce_cancel(id: &Id) {
    // No receiver (nothing running) is fine.
    let _ = cancel_bus().send(id.clone());
}

/// Subscribe to cancel announcements (see [`cancel_bus`]).
pub fn subscribe_cancels() -> tokio::sync::broadcast::Receiver<Id> {
    cancel_bus().subscribe()
}

/// Run-history retention defaults (08-workflows R1): per workflow, keep the
/// newest [`RUN_RETENTION_KEEP`] terminal runs AND every run younger than
/// [`RUN_RETENTION_DAYS`]; only a run outside both is pruned.
pub const RUN_RETENTION_KEEP: i64 = 200;
pub const RUN_RETENTION_DAYS: i64 = 30;
/// Rows deleted per write transaction by [`WorkflowsRepo::prune_runs`].
const RUN_PRUNE_BATCH: usize = 100;

impl WorkflowsRepo {
    /// Retention sweep for `workflow_runs` (+ their `workflow_checkpoints`).
    ///
    /// A run is deleted only when ALL hold: it is terminal (`success` /
    /// `error` / `canceled`) and not parked on an approval; it is outside its
    /// workflow's newest `keep` terminal runs; it started before
    /// `now - older_than_days`; no Proof Pack references it (the run's
    /// `proof_pack_id` or a `proof_packs` row of kind `workflow_run`); and no
    /// scheduled-task run links to it (`scheduled_task_runs.workflow_run_id`).
    /// Candidates are found by one read, then deleted in
    /// [`RUN_PRUNE_BATCH`]-row transactions. Returns the deleted run ids so
    /// the caller can remove each `workflow-context/<id>` dir.
    pub async fn prune_runs(&self, keep: i64, older_than_days: i64) -> Result<Vec<Id>> {
        let keep = keep.max(1);
        let cutoff = fmt(Utc::now() - chrono::Duration::days(older_than_days.max(1)));
        let ids: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM (
                 SELECT r.id, r.started_at, r.proof_pack_id,
                        ROW_NUMBER() OVER (PARTITION BY r.workflow_id
                                           ORDER BY r.started_at DESC, r.id DESC) AS rn
                 FROM workflow_runs r
                 WHERE r.status IN ('success','error','canceled')
                   AND COALESCE(r.waiting_approval, 0) = 0
             ) t
             WHERE t.rn > ? AND t.started_at < ? AND t.proof_pack_id IS NULL
               AND NOT EXISTS (SELECT 1 FROM proof_packs p
                               WHERE p.work_item_kind = 'workflow_run' AND p.work_item_id = t.id)
               AND NOT EXISTS (SELECT 1 FROM scheduled_task_runs s
                               WHERE s.workflow_run_id = t.id)",
        )
        .bind(keep)
        .bind(&cutoff)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("prune runs: scan"))?;
        let mut deleted = Vec::with_capacity(ids.len());
        for chunk in ids.chunks(RUN_PRUNE_BATCH) {
            let marks = vec!["?"; chunk.len()].join(",");
            let mut tx = self
                .pool
                .begin()
                .await
                .map_err(dberr("prune runs: begin"))?;
            // Re-check terminal status under the write lock: a run retried
            // between the scan and here is live again and must survive.
            let q = format!(
                "DELETE FROM workflow_runs WHERE id IN ({marks}) \
                 AND status IN ('success','error','canceled') RETURNING id"
            );
            let mut del = sqlx::query_scalar::<_, String>(sqlx::AssertSqlSafe(q.as_str()));
            for id in chunk {
                del = del.bind(id);
            }
            let gone = del
                .fetch_all(&mut *tx)
                .await
                .map_err(dberr("prune runs: delete"))?;
            // Checkpoints go by FK cascade on a real DB; delete explicitly
            // too so a connection without `foreign_keys` leaves no orphans.
            if !gone.is_empty() {
                let marks = vec!["?"; gone.len()].join(",");
                let q = format!("DELETE FROM workflow_checkpoints WHERE run_id IN ({marks})");
                let mut del = sqlx::query(sqlx::AssertSqlSafe(q.as_str()));
                for id in &gone {
                    del = del.bind(id);
                }
                del.execute(&mut *tx)
                    .await
                    .map_err(dberr("prune runs: checkpoints"))?;
            }
            tx.commit().await.map_err(dberr("prune runs: commit"))?;
            deleted.extend(gone);
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        Ok(deleted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn mem_pool() -> DbPool {
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(false);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!().run(&pool).await.unwrap();
        pool.into()
    }

    /// Competing real publications and run admission share the SQLite boundary.
    #[tokio::test]
    async fn review4_interleaved_publication_matches_live_and_run_definition() {
        for second in ["saved Y", "initial restored"] {
            let repo = WorkflowsRepo::new(mem_pool().await);
            let wf = repo
                .create(
                    &"ws".into(),
                    "WF",
                    "description",
                    "initial restored",
                    &WorkflowGraph::default(),
                    &"u".into(),
                )
                .await
                .unwrap();
            let (a, b, run) = tokio::join!(
                repo.publish(
                    &wf.id,
                    None,
                    None,
                    Some("saved X"),
                    None,
                    Some("fail"),
                    "save",
                    None,
                    None
                ),
                repo.publish(
                    &wf.id,
                    None,
                    None,
                    Some(second),
                    None,
                    Some("resume"),
                    "save or restore",
                    None,
                    None
                ),
                repo.create_run(&wf.id, &wf.workspace_id, &serde_json::Value::Null, None),
            );
            let (a, b, run) = (a.unwrap(), b.unwrap(), run.unwrap());
            assert_ne!(a.version, b.version);
            for saved in [a, b, repo.get(&wf.id).await.unwrap()] {
                let snapshot = repo
                    .get_version(&wf.id, saved.version)
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(saved.instructions, snapshot.instructions);
                assert_eq!(saved.on_restart, snapshot.on_restart);
                assert_eq!(
                    serde_json::to_value(saved.graph).unwrap(),
                    serde_json::to_value(snapshot.graph).unwrap()
                );
            }
            let pinned = repo.definition_for_run(&run).await.unwrap();
            assert_eq!(pinned.version, run.workflow_version.unwrap());
            assert_eq!(repo.list_versions(&wf.id).await.unwrap().len(), 3);
            let next = repo
                .create_run(&wf.id, &wf.workspace_id, &serde_json::Value::Null, None)
                .await
                .unwrap();
            assert_eq!(
                repo.definition_for_run(&next).await.unwrap().instructions,
                repo.get(&wf.id).await.unwrap().instructions
            );
        }
    }

    #[tokio::test]
    async fn review4_snapshot_conflict_rolls_back_the_complete_publication() {
        let repo = WorkflowsRepo::new(mem_pool().await);
        let wf = repo
            .create(
                &"ws".into(),
                "WF",
                "",
                "original",
                &WorkflowGraph::default(),
                &"u".into(),
            )
            .await
            .unwrap();
        repo.snapshot_version(
            &wf.id,
            2,
            "imported",
            "",
            "imported",
            &wf.graph,
            "import",
            "resume",
            &"u".into(),
        )
        .await
        .unwrap();
        assert!(repo
            .publish(
                &wf.id,
                Some("lost name"),
                None,
                Some("lost instructions"),
                None,
                None,
                "conflict",
                None,
                None
            )
            .await
            .is_err());
        let live = repo.get(&wf.id).await.unwrap();
        assert_eq!(live.version, 1);
        assert_eq!(live.name, "WF");
        assert_eq!(live.instructions, "original");
    }

    #[tokio::test]
    async fn retry_scope_is_atomic_and_active_retry_cannot_reset_checkpoints() {
        let pool = mem_pool().await;
        let repo = WorkflowsRepo::new(pool.clone());
        let wf = repo
            .create(
                &"ws1".into(),
                "WF",
                "",
                "",
                &WorkflowGraph::default(),
                &"u1".into(),
            )
            .await
            .unwrap();
        let run = repo
            .create_run(&wf.id, &wf.workspace_id, &serde_json::Value::Null, None)
            .await
            .unwrap();
        repo.update_run(&run.id, RunStatus::Error, &[], Some("failed"), true)
            .await
            .unwrap();
        let cp = WorkflowCheckpoint {
            node_id: "loop#1.0".into(),
            loop_id: "loop".into(),
            iteration: 1,
            step_index: 0,
            kind: "http_request".into(),
            name: "write".into(),
            status: NodeStatus::Success,
            attempts: 1,
            input: serde_json::Value::Null,
            output: Some(serde_json::json!({"saved": true})),
            error: None,
            logs: vec![],
            updated_at: Utc::now(),
        };
        repo.save_checkpoint(&run.id, &cp).await.unwrap();
        let scope = otto_core::workflows::RunScope {
            start_node: Some("loop".into()),
            only_node: true,
            ..Default::default()
        };
        repo.prepare_retry(&run.id, &["loop".into()], false, &scope)
            .await
            .unwrap();
        assert_eq!(
            repo.get_run(&run.id).await.unwrap().status,
            RunStatus::Pending
        );
        assert_eq!(
            repo.run_scope(&run.id).await.unwrap().start_node.as_deref(),
            Some("loop")
        );
        assert!(repo
            .prepare_retry(&run.id, &["loop".into()], true, &scope)
            .await
            .is_err());
        assert_eq!(
            repo.checkpoint(&run.id, &cp.node_id)
                .await
                .unwrap()
                .unwrap()
                .output,
            cp.output
        );
        repo.update_run(&run.id, RunStatus::Error, &[], None, true)
            .await
            .unwrap();
        sqlx::query("UPDATE workflow_checkpoints SET checkpoint_json = 'invalid'")
            .execute(&pool)
            .await
            .unwrap();
        assert!(repo
            .prepare_retry(&run.id, &["loop".into()], false, &scope)
            .await
            .is_err());
        assert_eq!(
            repo.get_run(&run.id).await.unwrap().status,
            RunStatus::Error,
            "failed checkpoint reset rolls back reopening"
        );
    }

    #[tokio::test]
    async fn queued_run_pins_the_definition_before_execution() {
        let repo = WorkflowsRepo::new(mem_pool().await);
        let wf = repo
            .create(
                &"ws1".into(),
                "Original",
                "",
                "original instruction",
                &WorkflowGraph::default(),
                &"u1".into(),
            )
            .await
            .unwrap();
        let run = repo
            .create_run(&wf.id, &wf.workspace_id, &serde_json::json!({}), None)
            .await
            .unwrap();
        assert_eq!(run.workflow_version, Some(1));
        repo.update(
            &wf.id,
            Some("Edited"),
            None,
            Some("different instructions"),
            None,
            None,
        )
        .await
        .unwrap();
        let pinned = repo.definition_for_run(&run).await.unwrap();
        assert_eq!(pinned.name, "Original");
        assert_eq!(pinned.instructions, "original instruction");
        assert_eq!(pinned.version, 1);
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
        let edited = repo
            .publish(
                &wf.id,
                None,
                None,
                None,
                Some(&g2),
                None,
                "edited graph",
                Some(&"u1".into()),
                None,
            )
            .await
            .unwrap();
        assert_eq!(edited.version, 2);
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
        let wf = repo
            .create(&"ws1".into(), "WF", "", "", &g, &"u1".into())
            .await
            .unwrap();
        let run = repo
            .create_run(&wf.id, &wf.workspace_id, &serde_json::Value::Null, None)
            .await
            .unwrap();

        // Live run → Conflict (the engine loop owns it).
        repo.update_run(&run.id, RunStatus::Running, &[], None, false)
            .await
            .unwrap();
        assert!(repo.reopen_run(&run.id).await.is_err());

        // Finished (error) run → reopens to pending with error/finished_at cleared.
        repo.update_run(&run.id, RunStatus::Error, &[], Some("boom"), true)
            .await
            .unwrap();
        repo.reopen_run(&run.id).await.unwrap();
        let r = repo.get_run(&run.id).await.unwrap();
        assert_eq!(r.status, RunStatus::Pending);
        assert!(r.error.is_none());
        assert!(r.finished_at.is_none());
    }

    #[tokio::test]
    async fn resume_budget_skips_approval_reentry_and_resets_on_retry() {
        let pool = mem_pool().await;
        let repo = WorkflowsRepo::new(pool);
        let g = WorkflowGraph::default();
        let wf = repo
            .create(&"ws1".into(), "WF", "", "", &g, &"u1".into())
            .await
            .unwrap();
        let run = repo
            .create_run(&wf.id, &wf.workspace_id, &serde_json::Value::Null, None)
            .await
            .unwrap();
        // A restart while parked at an approval doesn't spend the budget…
        repo.prepare_resume(&run.id, &[], "{}", false)
            .await
            .unwrap();
        assert_eq!(repo.get_run(&run.id).await.unwrap().resume_attempts, 0);
        // …a restart mid-step does.
        repo.prepare_resume(&run.id, &[], "{}", true).await.unwrap();
        repo.prepare_resume(&run.id, &[], "{}", true).await.unwrap();
        assert_eq!(repo.get_run(&run.id).await.unwrap().resume_attempts, 2);
        // A user retry is a fresh lifecycle with a fresh budget.
        repo.update_run(&run.id, RunStatus::Error, &[], Some("boom"), true)
            .await
            .unwrap();
        repo.prepare_retry(&run.id, &[], false, &Default::default())
            .await
            .unwrap();
        assert_eq!(repo.get_run(&run.id).await.unwrap().resume_attempts, 0);
        repo.prepare_resume(&run.id, &[], "{}", true).await.unwrap();
        repo.update_run(&run.id, RunStatus::Error, &[], Some("boom"), true)
            .await
            .unwrap();
        repo.reopen_run(&run.id).await.unwrap();
        assert_eq!(repo.get_run(&run.id).await.unwrap().resume_attempts, 0);
    }

    #[tokio::test]
    async fn startup_reap_fails_started_runs_but_keeps_the_queue() {
        let pool = mem_pool().await;
        let repo = WorkflowsRepo::new(pool);
        let g = WorkflowGraph::default();
        let wf = repo
            .create(&"ws1".into(), "WF", "", "", &g, &"u1".into())
            .await
            .unwrap();
        let mk = || repo.create_run(&wf.id, &wf.workspace_id, &serde_json::Value::Null, None);

        // r1: EXECUTING when the daemon died → must fail.
        let r1 = mk().await.unwrap();
        repo.update_run(&r1.id, RunStatus::Running, &[], None, false)
            .await
            .unwrap();
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
            review_ids: Vec::new(),
            activity: None,
        };
        repo.update_run(&r3.id, RunStatus::Error, &[node], Some("boom"), true)
            .await
            .unwrap();
        repo.reopen_run(&r3.id).await.unwrap();

        assert_eq!(repo.fail_interrupted_runs("interrupted").await.unwrap(), 2);
        assert_eq!(repo.get_run(&r1.id).await.unwrap().status, RunStatus::Error);
        assert_eq!(repo.get_run(&r3.id).await.unwrap().status, RunStatus::Error);
        // The queued run is untouched and is exactly what re-enqueues.
        assert_eq!(
            repo.get_run(&r2.id).await.unwrap().status,
            RunStatus::Pending
        );
        assert_eq!(repo.queued_run_ids().await.unwrap(), vec![r2.id.clone()]);
    }

    #[tokio::test]
    async fn create_run_records_the_starter() {
        let pool = mem_pool().await;
        let repo = WorkflowsRepo::new(pool);
        let g = WorkflowGraph::default();
        let wf = repo
            .create(&"ws1".into(), "WF", "", "", &g, &"u1".into())
            .await
            .unwrap();
        let by = "user-b".to_string();
        let r = repo
            .create_run(
                &wf.id,
                &wf.workspace_id,
                &serde_json::Value::Null,
                Some(&by),
            )
            .await
            .unwrap();
        assert_eq!(r.created_by.as_deref(), Some("user-b"));
        assert_eq!(
            repo.get_run(&r.id).await.unwrap().created_by.as_deref(),
            Some("user-b")
        );
        let t = repo
            .create_run(&wf.id, &wf.workspace_id, &serde_json::Value::Null, None)
            .await
            .unwrap();
        assert!(t.created_by.is_none(), "trigger runs carry no starter");
        // Perf N5: the single-column reads agree with the full row and read
        // neither nodes_json nor the whole row.
        let probe = repo.pool.statement_probe();
        probe.reset();
        assert_eq!(
            repo.run_created_by(&r.id).await.unwrap().as_deref(),
            Some("user-b")
        );
        assert!(repo.run_created_by(&t.id).await.unwrap().is_none());
        assert!(repo.run_created_by(&"gone".into()).await.unwrap().is_none());
        let with_input = repo
            .create_run(
                &wf.id,
                &wf.workspace_id,
                &serde_json::json!({"jira_ticket": "OT-1"}),
                None,
            )
            .await
            .unwrap();
        probe.reset();
        assert_eq!(
            repo.run_input(&with_input.id).await.unwrap(),
            serde_json::json!({"jira_ticket": "OT-1"})
        );
        assert_eq!(
            repo.run_input(&"gone".into()).await.unwrap(),
            serde_json::Value::Null
        );
        let stmts = probe.take();
        assert!(
            stmts
                .iter()
                .all(|q| !q.contains("SELECT *") && !q.contains("nodes_json")),
            "{stmts:?}"
        );
    }

    #[tokio::test]
    async fn queued_run_ids_are_fifo_by_creation() {
        let pool = mem_pool().await;
        let repo = WorkflowsRepo::new(pool);
        let g = WorkflowGraph::default();
        let wf = repo
            .create(&"ws1".into(), "WF", "", "", &g, &"u1".into())
            .await
            .unwrap();
        let a = repo
            .create_run(&wf.id, &wf.workspace_id, &serde_json::Value::Null, None)
            .await
            .unwrap();
        let b = repo
            .create_run(&wf.id, &wf.workspace_id, &serde_json::Value::Null, None)
            .await
            .unwrap();
        let c = repo
            .create_run(&wf.id, &wf.workspace_id, &serde_json::Value::Null, None)
            .await
            .unwrap();
        // A run that already started is not queued.
        repo.update_run(&b.id, RunStatus::Running, &[], None, false)
            .await
            .unwrap();
        // Same-timestamp rows tiebreak on id; ULIDs are creation-ordered, so
        // FIFO order holds either way.
        assert_eq!(
            repo.queued_run_ids().await.unwrap(),
            vec![a.id.clone(), c.id.clone()]
        );
    }

    #[tokio::test]
    async fn find_by_name_is_scoped_to_the_workspace() {
        let pool = mem_pool().await;
        let repo = WorkflowsRepo::new(pool);
        let g = WorkflowGraph::default();
        let a = repo
            .create(&"wsA".into(), "Write tests", "", "", &g, &"u".into())
            .await
            .unwrap();
        let b = repo
            .create(&"wsB".into(), "Write tests", "", "", &g, &"u".into())
            .await
            .unwrap();

        // S3-03: another workspace's workflow is never resolved.
        assert!(repo
            .find_by_name("write TESTS", &"wsC".into())
            .await
            .unwrap()
            .is_none());
        // Case-insensitive, and each workspace gets its own.
        for (ws, want) in [("wsA", &a.id), ("wsB", &b.id)] {
            let got = repo
                .find_by_name("write TESTS", &ws.into())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(&got.id, want);
        }
        assert!(repo
            .find_by_name("nope", &"wsA".into())
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn instructions_round_trip_and_version_snapshot() {
        let pool = mem_pool().await;
        let repo = WorkflowsRepo::new(pool);
        let ws: Id = "ws1".into();
        let wf = repo
            .create(
                &ws,
                "generic",
                "d",
                "FOLLOW THE RULES",
                &WorkflowGraph {
                    nodes: vec![],
                    edges: vec![],
                },
                &"u1".into(),
            )
            .await
            .unwrap();
        assert_eq!(wf.instructions, "FOLLOW THE RULES");
        let up = repo
            .update(&wf.id, None, None, Some("v2 rules"), None, None)
            .await
            .unwrap();
        assert_eq!(up.instructions, "v2 rules");
        let versions = repo.list_versions(&wf.id).await.unwrap();
        assert_eq!(versions.last().unwrap().instructions, "FOLLOW THE RULES"); // v1 snapshot
    }

    #[tokio::test]
    async fn run_records_version_and_proof_pack() {
        let pool = mem_pool().await;
        let repo = WorkflowsRepo::new(pool);
        let wf = repo
            .create(
                &"ws1".into(),
                "WF",
                "",
                "",
                &WorkflowGraph::default(),
                &"u1".into(),
            )
            .await
            .unwrap();

        let run = repo
            .create_run(&wf.id, &"ws1".into(), &serde_json::json!({}), None)
            .await
            .unwrap();
        assert_eq!(run.workflow_version, Some(1));
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
            .create(
                &"ws1".into(),
                "WF",
                "",
                "",
                &WorkflowGraph::default(),
                &"u1".into(),
            )
            .await
            .unwrap();
        let run = repo
            .create_run(&wf.id, &"ws1".into(), &serde_json::json!({}), None)
            .await
            .unwrap();
        assert_eq!(run.rev, 0, "fresh run starts at rev 0");

        // Every progress write returns the next rev, and get_run round-trips it.
        let r1 = repo
            .update_run(&run.id, RunStatus::Running, &[], None, false)
            .await
            .unwrap();
        let r2 = repo
            .update_run(&run.id, RunStatus::Running, &[], None, false)
            .await
            .unwrap();
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
            .create(
                &"ws1".into(),
                "WF",
                "",
                "",
                &WorkflowGraph::default(),
                &"u1".into(),
            )
            .await
            .unwrap();
        let run = repo
            .create_run(&wf.id, &"ws1".into(), &serde_json::json!({}), None)
            .await
            .unwrap();

        // Engine marks the run Running, then the API cancels it (terminal write).
        repo.update_run(&run.id, RunStatus::Running, &[], None, false)
            .await
            .unwrap();
        repo.update_run(&run.id, RunStatus::Canceled, &[], Some("canceled"), true)
            .await
            .unwrap();

        // A routine progress save lands AFTER the cancel. It must bump rev but
        // leave the status Canceled — the old `update_run(.., Running, ..)`
        // would have clobbered it back to Running (the bug this fixes).
        let rev = repo.update_run_progress(&run.id, &[]).await.unwrap();
        let got = repo.get_run(&run.id).await.unwrap();
        assert_eq!(
            got.status,
            RunStatus::Canceled,
            "progress write must not resurrect a canceled run"
        );
        assert_eq!(
            (rev, got.rev),
            (3, 3),
            "progress write still bumps + round-trips the monotonic rev"
        );
    }

    fn node(id: &str, status: &str) -> NodeRunState {
        serde_json::from_value(serde_json::json!({ "node_id": id, "status": status })).unwrap()
    }

    /// Perf W1/W12: the engine's cancel checks read the lifecycle only — no
    /// `SELECT *` / `nodes_json` on `workflow_runs` — and a cancel is
    /// announced on the bus so a running node wakes without polling.
    #[tokio::test]
    async fn status_reads_never_load_the_run_body_and_cancel_is_announced() {
        let pool = mem_pool().await;
        let repo = WorkflowsRepo::new(pool.clone());
        let wf = repo
            .create(
                &"ws1".into(),
                "WF",
                "",
                "",
                &WorkflowGraph::default(),
                &"u1".into(),
            )
            .await
            .unwrap();
        let run = repo
            .create_run(&wf.id, &"ws1".into(), &serde_json::json!({}), None)
            .await
            .unwrap();
        let mut bus = subscribe_cancels();
        let probe = pool.statement_probe();
        probe.reset();
        // A simulated 10 s of a running node: 5 cancel-poll safety reads.
        for _ in 0..5 {
            assert_eq!(
                repo.run_status(&run.id).await.unwrap(),
                Some((RunStatus::Pending, false))
            );
            assert!(!repo.is_canceled(&run.id).await);
        }
        let stmts = probe.take();
        assert_eq!(stmts.len(), 10, "one statement per check: {stmts:?}");
        assert!(
            stmts
                .iter()
                .all(|q| !q.contains("SELECT *") && !q.contains("nodes_json")),
            "status checks must not read the run body: {stmts:?}"
        );
        // The bus is process-wide: other tests' cancels may be on it too.
        let ours = |bus: &mut tokio::sync::broadcast::Receiver<Id>| {
            std::iter::from_fn(|| bus.try_recv().ok()).any(|id| id == run.id)
        };
        assert!(repo.request_cancel(&run.id).await.unwrap().is_some());
        assert!(ours(&mut bus), "cancel announced");
        assert!(repo.is_canceled(&run.id).await);
        // A no-op cancel (already settled) announces nothing.
        assert!(repo.request_cancel(&run.id).await.unwrap().is_none());
        assert!(!ours(&mut bus));
        assert_eq!(repo.run_status(&"gone".into()).await.unwrap(), None);
    }

    /// Perf W12 budget: serializing + projecting a big run (500 nodes,
    /// ~200 KB) — what every `update_run` / `update_run_progress` does off the
    /// runtime. `progress_write` is memoized per node, so a steady-state write
    /// (one node changed) must stay well under the cold full projection.
    /// Timing-sensitive, so `#[ignore]`d in the per-PR run; the nightly bench
    /// runs it in release (`.github/workflows/nightly-bench.yml`):
    /// `cargo test --release -p otto-state --lib big_run_write_budget -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn big_run_write_budget() {
        let mut nodes: Vec<NodeRunState> = (0..500)
            .map(|i| {
                serde_json::from_value(serde_json::json!({
                    "node_id": format!("n{i}"),
                    "status": "success",
                    "logs": (0..4).map(|l| format!("{i}:{l} {}", "x".repeat(80))).collect::<Vec<_>>(),
                    "output": { "text": "y".repeat(40) },
                }))
                .unwrap()
            })
            .collect();
        let size = serde_json::to_string(&nodes).unwrap().len();
        assert!(size > 150_000, "fixture is a big run: {size} B");
        let run = "big-run-write-budget";
        let t = std::time::Instant::now();
        crate::workflow_progress::progress_write(run, &nodes).unwrap();
        let cold = t.elapsed();
        let mut warm = std::time::Duration::ZERO;
        for tick in 0..20 {
            nodes[tick].logs.push(format!("tick {tick}"));
            let t = std::time::Instant::now();
            crate::workflow_progress::progress_write(run, &nodes).unwrap();
            warm = warm.max(t.elapsed());
        }
        let t = std::time::Instant::now();
        crate::workflow_progress::nodes_projection(&nodes).unwrap();
        let full = t.elapsed();
        // Greppable for the nightly bench summary (.github/workflows/nightly-bench.yml).
        eprintln!(
            "WORKFLOW_BIG_RUN size={size}B cold={cold:?} warm_max={warm:?} unmemoized={full:?}"
        );
        // `OTTO_BENCH_WRITE_MS` overrides the ceiling (nightly runs release).
        let budget = std::env::var("OTTO_BENCH_WRITE_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(if cfg!(debug_assertions) { 150 } else { 15 });
        assert!(
            warm < std::time::Duration::from_millis(budget),
            "steady-state progress write {warm:?} over {budget} ms"
        );
    }

    #[tokio::test]
    async fn lifecycle_cas_never_undoes_a_cancel() {
        let pool = mem_pool().await;
        let repo = WorkflowsRepo::new(pool);
        let wf = repo
            .create(
                &"ws1".into(),
                "WF",
                "",
                "",
                &WorkflowGraph::default(),
                &"u1".into(),
            )
            .await
            .unwrap();
        let run = repo
            .create_run(&wf.id, &"ws1".into(), &serde_json::json!({}), None)
            .await
            .unwrap();

        // Cancel lands while the engine is still provisioning (run pending)…
        assert!(repo.request_cancel(&run.id).await.unwrap().is_some());
        // …so the engine's Pending→Running start must NOT apply.
        let started = repo
            .update_run_if(
                &run.id,
                &[RunStatus::Pending],
                RunStatus::Running,
                &[node("n1", "pending")],
                None,
                false,
            )
            .await
            .unwrap();
        assert!(started.is_none(), "start must not resurrect a canceled run");
        let got = repo.get_run(&run.id).await.unwrap();
        assert_eq!(got.status, RunStatus::Canceled);
        assert!(got.finished_at.is_some());
        // A late Success finalize is refused too.
        let fin = repo
            .update_run_if(
                &run.id,
                &[RunStatus::Running],
                RunStatus::Success,
                &[],
                None,
                true,
            )
            .await
            .unwrap();
        assert!(fin.is_none());
        assert_eq!(
            repo.get_run(&run.id).await.unwrap().status,
            RunStatus::Canceled
        );
        // A second cancel on a settled run is a no-op.
        assert!(repo.request_cancel(&run.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn request_cancel_keeps_the_engines_node_states() {
        let pool = mem_pool().await;
        let repo = WorkflowsRepo::new(pool.clone());
        let wf = repo
            .create(
                &"ws1".into(),
                "WF",
                "",
                "",
                &WorkflowGraph::default(),
                &"u1".into(),
            )
            .await
            .unwrap();
        let run = repo
            .create_run(&wf.id, &"ws1".into(), &serde_json::json!({}), None)
            .await
            .unwrap();
        repo.update_run(
            &run.id,
            RunStatus::Running,
            &[node("n1", "success"), node("n2", "running")],
            None,
            false,
        )
        .await
        .unwrap();
        // Parked at an approval when the cancel lands.
        sqlx::query(
            "UPDATE workflow_runs SET waiting_approval = 1, approval_node_id = 'n2' WHERE id = ?",
        )
        .bind(&run.id)
        .execute(&pool)
        .await
        .unwrap();
        repo.request_cancel(&run.id).await.unwrap().unwrap();
        let got = repo.get_run(&run.id).await.unwrap();
        assert_eq!(got.status, RunStatus::Canceled);
        assert_eq!(got.nodes.len(), 2, "cancel must not rewrite nodes_json");
        assert!(!got.waiting_approval, "no phantom approval banner");
        assert!(got.approval_node_id.is_none());
    }

    #[tokio::test]
    async fn terminal_update_clears_a_stale_approval_pause() {
        let pool = mem_pool().await;
        let repo = WorkflowsRepo::new(pool.clone());
        let wf = repo
            .create(
                &"ws1".into(),
                "WF",
                "",
                "",
                &WorkflowGraph::default(),
                &"u1".into(),
            )
            .await
            .unwrap();
        let run = repo
            .create_run(&wf.id, &"ws1".into(), &serde_json::json!({}), None)
            .await
            .unwrap();
        sqlx::query(
            "UPDATE workflow_runs SET status = 'running', waiting_approval = 1, approval_node_id = 'gate' WHERE id = ?",
        )
        .bind(&run.id)
        .execute(&pool)
        .await
        .unwrap();
        // A progress (non-terminal) write leaves the pause alone…
        repo.update_run(&run.id, RunStatus::Running, &[], None, false)
            .await
            .unwrap();
        assert!(repo.get_run(&run.id).await.unwrap().waiting_approval);
        // …a terminal one clears it.
        repo.update_run(&run.id, RunStatus::Error, &[], Some("boom"), true)
            .await
            .unwrap();
        let got = repo.get_run(&run.id).await.unwrap();
        assert!(!got.waiting_approval);
        assert!(got.approval_node_id.is_none());
    }

    #[tokio::test]
    async fn prune_runs_keeps_newest_young_active_and_referenced() {
        let pool = mem_pool().await;
        let repo = WorkflowsRepo::new(pool.clone());
        let wf = repo
            .create(
                &"ws1".into(),
                "WF",
                "",
                "",
                &WorkflowGraph::default(),
                &"u1".into(),
            )
            .await
            .unwrap();
        // 6 old terminal runs (started 40+ days ago, newest first by index).
        let mut ids = Vec::new();
        for i in 0..6 {
            let run = repo
                .create_run(&wf.id, &wf.workspace_id, &serde_json::Value::Null, None)
                .await
                .unwrap();
            repo.update_run(&run.id, RunStatus::Success, &[], None, true)
                .await
                .unwrap();
            sqlx::query("UPDATE workflow_runs SET started_at = ? WHERE id = ?")
                .bind(fmt(Utc::now() - chrono::Duration::days(40 + i)))
                .bind(&run.id)
                .execute(&pool)
                .await
                .unwrap();
            ids.push(run.id);
        }
        // A young terminal run, an old running run, an old proof-linked run
        // and an old scheduled-task-linked run: all must survive.
        let young = repo
            .create_run(&wf.id, &wf.workspace_id, &serde_json::Value::Null, None)
            .await
            .unwrap();
        repo.update_run(&young.id, RunStatus::Error, &[], Some("x"), true)
            .await
            .unwrap();
        let live = repo
            .create_run(&wf.id, &wf.workspace_id, &serde_json::Value::Null, None)
            .await
            .unwrap();
        sqlx::query("UPDATE workflow_runs SET status='running', started_at = ? WHERE id = ?")
            .bind(fmt(Utc::now() - chrono::Duration::days(90)))
            .bind(&live.id)
            .execute(&pool)
            .await
            .unwrap();
        repo.set_run_proof_pack(&ids[4], "pack1").await.unwrap();
        sqlx::query(
            "INSERT INTO scheduled_task_runs (id, task_id, workspace_id, status, started_at, \
             workflow_run_id, created_at) VALUES ('str1', 't1', 'ws1', 'ok', ?1, ?2, ?1)",
        )
        .bind(fmt(Utc::now()))
        .bind(&ids[5])
        .execute(&pool)
        .await
        .unwrap();
        // keep=3 → the 3 newest terminal runs (young + ids[0..2]) stay.
        let gone = repo.prune_runs(3, 30).await.unwrap();
        let mut gone_sorted = gone.clone();
        gone_sorted.sort();
        let mut want = vec![ids[2].clone(), ids[3].clone()];
        want.sort();
        assert_eq!(gone_sorted, want);
        for keep in [&young.id, &live.id, &ids[0], &ids[1], &ids[4], &ids[5]] {
            assert!(repo.get_run(keep).await.is_ok(), "{keep} must survive");
        }
        // Idempotent.
        assert!(repo.prune_runs(3, 30).await.unwrap().is_empty());
    }

    /// Finding 6: concurrent admitters (a workflow-kind scheduled task racing
    /// an event trigger) get exactly ONE run; the check-then-insert is one
    /// write transaction. A finished run frees the slot again.
    #[tokio::test]
    async fn admit_run_if_idle_admits_exactly_one_concurrent_run() {
        let dir = tempfile::tempdir().unwrap();
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(dir.path().join("wf.db"))
            .create_if_missing(true)
            .foreign_keys(false)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .busy_timeout(std::time::Duration::from_secs(10));
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!().run(&pool).await.unwrap();
        let repo = WorkflowsRepo::new(DbPool::from(pool));
        let wf = repo
            .create(
                &"ws".into(),
                "WF",
                "description",
                "",
                &WorkflowGraph::default(),
                &"u".into(),
            )
            .await
            .unwrap();
        let input = serde_json::json!({});
        let admits = futures_util::future::join_all(
            (0..6).map(|_| repo.admit_run_if_idle(&wf.id, &wf.workspace_id, &input, None)),
        )
        .await;
        let admitted: Vec<_> = admits.into_iter().filter_map(|r| r.unwrap()).collect();
        assert_eq!(admitted.len(), 1, "exactly one concurrent admission");
        assert!(repo.has_active_run(&wf.id).await.unwrap());
        assert!(repo
            .admit_run_if_idle(&wf.id, &wf.workspace_id, &input, None)
            .await
            .unwrap()
            .is_none());
        repo.update_run(&admitted[0].id, RunStatus::Success, &[], None, true)
            .await
            .unwrap();
        assert!(repo
            .admit_run_if_idle(&wf.id, &wf.workspace_id, &input, None)
            .await
            .unwrap()
            .is_some());
    }

    /// S17-01: deleting a workflow with a live run is refused (the cascade
    /// would orphan its driver); once the run settles the delete goes through,
    /// and a driver whose row is gone reads as canceled so it stops.
    #[tokio::test]
    async fn delete_refuses_live_runs_and_missing_run_reads_canceled() {
        let pool = mem_pool().await;
        let repo = WorkflowsRepo::new(pool.clone());
        let wf = repo
            .create(
                &"ws1".into(),
                "WF",
                "",
                "",
                &WorkflowGraph::default(),
                &"u1".into(),
            )
            .await
            .unwrap();
        let run = repo
            .create_run(&wf.id, &"ws1".into(), &serde_json::json!({}), None)
            .await
            .unwrap();
        let err = repo.delete(&wf.id).await.unwrap_err();
        assert!(matches!(err, Error::Conflict(_)), "got {err:?}");
        assert!(repo.get(&wf.id).await.is_ok(), "workflow must survive");
        sqlx::query("UPDATE workflow_runs SET status='success' WHERE id=?")
            .bind(&run.id)
            .execute(&pool)
            .await
            .unwrap();
        repo.delete(&wf.id).await.unwrap();
        assert!(repo.get(&wf.id).await.is_err());
        // The test pool runs without foreign keys; do the cascade by hand.
        sqlx::query("DELETE FROM workflow_runs WHERE id=?")
            .bind(&run.id)
            .execute(&pool)
            .await
            .unwrap();
        assert!(
            repo.is_canceled(&run.id).await,
            "a run whose row is gone must read as canceled"
        );
    }

    /// S3-09: chat control resolves the run by SQL on the input keys — exact
    /// thread first, then the newest channel/chat match; settled runs and
    /// other chats never match.
    #[tokio::test]
    async fn find_active_run_for_chat_prefers_exact_thread() {
        let pool = mem_pool().await;
        let repo = WorkflowsRepo::new(pool.clone());
        let wf = repo
            .create(
                &"ws1".into(),
                "WF",
                "",
                "",
                &WorkflowGraph::default(),
                &"u1".into(),
            )
            .await
            .unwrap();
        let mk = |thread: Option<&str>, chat: &str| serde_json::json!({"channel":"slack","chat":chat,"thread":thread,"origin_workspace_id":"ws1"});
        let threaded = repo
            .create_run(&wf.id, &"ws1".into(), &mk(Some("t1"), "C1"), None)
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let newer = repo
            .create_run(&wf.id, &"ws1".into(), &mk(None, "C1"), None)
            .await
            .unwrap();
        let _other = repo
            .create_run(&wf.id, &"ws1".into(), &mk(Some("t1"), "C2"), None)
            .await
            .unwrap();
        let find = |t: Option<&'static str>| {
            let repo = repo.clone();
            async move {
                repo.find_active_run_for_chat("ws1", "slack", "C1", t)
                    .await
                    .unwrap()
            }
        };
        assert_eq!(find(Some("t1")).await, Some(threaded.id.clone()));
        assert_eq!(find(None).await, Some(newer.id.clone()));
        assert_eq!(
            find(Some("t9")).await,
            Some(newer.id.clone()),
            "newest fallback"
        );
        assert_eq!(
            repo.find_active_run_for_chat("wsX", "slack", "C1", None)
                .await
                .unwrap(),
            None,
            "another workspace's message never controls the run"
        );
        sqlx::query("UPDATE workflow_runs SET status='success' WHERE workflow_id=?")
            .bind(&wf.id)
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(find(Some("t1")).await, None, "settled runs never match");
    }
}
