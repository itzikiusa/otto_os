//! Persistence for **Scheduled Tasks** (migration `0084_scheduled_tasks.sql`).
//!
//! Two tables: `scheduled_tasks` (the recurring definition) and
//! `scheduled_task_runs` (one row per execution, the report history). Pure storage —
//! the cadence/cursor logic and report I/O live in `otto_server`. `schedule` and
//! `destination` are JSON columns surfaced as `serde_json::Value`. The `last_run_at`
//! cursor is advanced by the scheduler on run completion via [`settle_generation`].
//!
//! [`settle_generation`]: ScheduledTasksRepo::settle_generation

use crate::DbPool;
use chrono::Utc;
use otto_core::domain::{ScheduledTask, ScheduledTaskRun};
use otto_core::{new_id, Result};
use serde_json::Value;
use sqlx::Row;

use crate::convert::{dberr, fmt, json};

#[derive(Clone)]
pub struct ScheduledTasksRepo {
    pool: DbPool,
}

/// Fields for creating a task. `schedule`/`destination` default to `{}`.
#[derive(Clone, Debug)]
pub struct NewScheduledTask {
    pub workspace_id: String,
    pub name: String,
    pub kind: String,
    pub prompt: String,
    pub skill: Option<String>,
    pub provider: String,
    pub model: String,
    pub cwd: String,
    pub schedule: Value,
    pub destination: Value,
    pub enabled: bool,
    pub created_by: Option<String>,
    // v2
    pub timezone: String,
    pub workflow_id: Option<String>,
    pub sandbox: String,
    pub max_retries: i64,
    pub notify_on_change: bool,
    pub attach_proof: bool,
}

impl NewScheduledTask {
    /// Construct with v2 fields at their backward-compatible defaults.
    pub fn defaults(workspace_id: String, name: String) -> Self {
        Self {
            workspace_id,
            name,
            kind: "agent_prompt".into(),
            prompt: String::new(),
            skill: None,
            provider: "claude".into(),
            model: String::new(),
            cwd: String::new(),
            schedule: Value::Null,
            destination: Value::Null,
            enabled: true,
            created_by: None,
            timezone: "UTC".into(),
            workflow_id: None,
            sandbox: "none".into(),
            max_retries: 0,
            notify_on_change: false,
            attach_proof: false,
        }
    }
}

/// Partial update — every `Some` field is written (`None` leaves it unchanged).
#[derive(Clone, Debug, Default)]
pub struct ScheduledTaskPatch {
    pub name: Option<String>,
    pub prompt: Option<String>,
    pub skill: Option<Option<String>>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub cwd: Option<String>,
    pub schedule: Option<Value>,
    pub destination: Option<Value>,
    pub enabled: Option<bool>,
    // v2
    pub timezone: Option<String>,
    pub workflow_id: Option<Option<String>>,
    pub sandbox: Option<String>,
    pub max_retries: Option<i64>,
    pub notify_on_change: Option<bool>,
    pub attach_proof: Option<bool>,
}

/// Fields for opening a run row (status starts `running`).
#[derive(Clone, Debug)]
pub struct NewRun {
    pub task_id: String,
    pub workspace_id: String,
    pub trigger: String,
}

// Kept out of the HTTP run DTO. ScheduledTask deliberately skips its internal
// epochs in serde, so persist those explicitly instead of recovering them as 0.
#[derive(serde::Serialize, serde::Deserialize)]
struct AdmittedTaskSnapshot {
    version: u8,
    schedule_generation: i64,
    admission_generation: i64,
    task: ScheduledTask,
}

/// Terminal state for a run — the engine fills this once the agent/workflow
/// completes (success or failure) and delivery has been attempted.
#[derive(Clone, Debug, Default)]
pub struct FinishRun {
    pub status: String,
    pub summary: String,
    pub report_path: Option<String>,
    pub report_rel: Option<String>,
    pub delivered: bool,
    pub delivery_error: Option<String>,
    pub error: Option<String>,
    pub session_id: Option<String>,
    pub report_hash: Option<String>,
    pub proof_pack_id: Option<String>,
    pub attempts: i64,
    pub skipped_delivery: bool,
    pub workflow_run_id: Option<String>,
}

// --- Row mapping -----------------------------------------------------------

fn row_to_task(r: &sqlx::sqlite::SqliteRow) -> Result<ScheduledTask> {
    let sched_raw: String = r.get("schedule_json");
    let dest_raw: String = r.get("destination_json");
    Ok(ScheduledTask {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        name: r.get("name"),
        kind: r.get("kind"),
        prompt: r.get("prompt"),
        skill: r.get("skill"),
        provider: r.get("provider"),
        model: r.get("model"),
        cwd: r.get("cwd"),
        schedule: json(&sched_raw).unwrap_or(Value::Null),
        destination: json(&dest_raw).unwrap_or(Value::Null),
        enabled: r.get::<i64, _>("enabled") != 0,
        timezone: r.get("timezone"),
        workflow_id: r.get("workflow_id"),
        sandbox: r.get("sandbox"),
        max_retries: r.get("max_retries"),
        notify_on_change: r.get::<i64, _>("notify_on_change") != 0,
        attach_proof: r.get::<i64, _>("attach_proof") != 0,
        schedule_generation: r.get("schedule_generation"),
        admission_generation: r.get("admission_generation"),
        last_run_at: r.get("last_run_at"),
        last_status: r.get("last_status"),
        next_run_at: r.get("next_run_at"),
        armed_at: r.get("armed_at"),
        created_by: r.get("created_by"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    })
}

fn row_to_run(r: &sqlx::sqlite::SqliteRow) -> Result<ScheduledTaskRun> {
    Ok(ScheduledTaskRun {
        id: r.get("id"),
        task_id: r.get("task_id"),
        workspace_id: r.get("workspace_id"),
        status: r.get("status"),
        trigger: r.get("trigger"),
        started_at: r.get("started_at"),
        finished_at: r.get("finished_at"),
        summary: r.get("summary"),
        report_path: r.get("report_path"),
        report_rel: r.get("report_rel"),
        delivered: r.get::<i64, _>("delivered") != 0,
        delivery_error: r.get("delivery_error"),
        error: r.get("error"),
        session_id: r.get("session_id"),
        report_hash: r.get("report_hash"),
        proof_pack_id: r.get("proof_pack_id"),
        attempts: r.get("attempts"),
        skipped_delivery: r.get::<i64, _>("skipped_delivery") != 0,
        workflow_run_id: r.get("workflow_run_id"),
        created_at: r.get("created_at"),
    })
}

impl ScheduledTasksRepo {
    pub fn new(pool: impl Into<DbPool>) -> Self {
        let pool: DbPool = pool.into();
        Self { pool }
    }

    // -- Tasks ---------------------------------------------------------------

    pub async fn create(&self, t: NewScheduledTask) -> Result<ScheduledTask> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO scheduled_tasks (id, workspace_id, name, kind, prompt, skill, provider, \
             model, cwd, schedule_json, destination_json, enabled, timezone, workflow_id, sandbox, \
             max_retries, notify_on_change, attach_proof, created_by, created_at, updated_at, \
             armed_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&t.workspace_id)
        .bind(&t.name)
        .bind(&t.kind)
        .bind(&t.prompt)
        .bind(&t.skill)
        .bind(&t.provider)
        .bind(&t.model)
        .bind(&t.cwd)
        .bind(t.schedule.to_string())
        .bind(t.destination.to_string())
        .bind(t.enabled as i64)
        .bind(&t.timezone)
        .bind(&t.workflow_id)
        .bind(&t.sandbox)
        .bind(t.max_retries)
        .bind(t.notify_on_change as i64)
        .bind(t.attach_proof as i64)
        .bind(&t.created_by)
        .bind(&now)
        .bind(&now)
        // Armed at creation: the first fire is the next one after now.
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create scheduled task"))?;
        self.get(&id).await
    }

    pub async fn get(&self, id: &str) -> Result<ScheduledTask> {
        let row = sqlx::query("SELECT * FROM scheduled_tasks WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("scheduled task not found"))?;
        row_to_task(&row)
    }

    pub async fn list_by_workspace(&self, ws: &str) -> Result<Vec<ScheduledTask>> {
        let rows = sqlx::query(
            "SELECT * FROM scheduled_tasks WHERE workspace_id = ? ORDER BY created_at DESC",
        )
        .bind(ws)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list scheduled tasks"))?;
        rows.iter().map(row_to_task).collect()
    }

    /// All enabled tasks across every workspace — the scheduler's tick query.
    pub async fn list_enabled(&self) -> Result<Vec<ScheduledTask>> {
        let rows = sqlx::query("SELECT * FROM scheduled_tasks WHERE enabled = 1")
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("list enabled scheduled tasks"))?;
        rows.iter().map(row_to_task).collect()
    }

    pub async fn update(&self, id: &str, p: ScheduledTaskPatch) -> Result<ScheduledTask> {
        self.update_with_next_run(id, p, |_| None).await
    }

    /// Apply an edit, occurrence generation, rearming and the next-fire display
    /// atomically. Cadence interpretation stays in the caller's synchronous
    /// calculator; it sees the complete new definition under the write lock.
    pub async fn update_with_next_run<F>(
        &self,
        id: &str,
        p: ScheduledTaskPatch,
        next_run: F,
    ) -> Result<ScheduledTask>
    where
        F: FnOnce(&ScheduledTask) -> Option<String>,
    {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(dberr("begin scheduled task edit"))?;
        let row = sqlx::query("SELECT * FROM scheduled_tasks WHERE id=?")
            .bind(id)
            .fetch_one(&mut *tx)
            .await
            .map_err(dberr("scheduled task for edit"))?;
        let current = row_to_task(&row)?;
        let timing_changed = p.schedule.as_ref().is_some_and(|v| v != &current.schedule)
            || p.timezone.as_ref().is_some_and(|v| v != &current.timezone);
        let enabled = p.enabled.unwrap_or(current.enabled);
        let rearm = timing_changed || (!current.enabled && enabled);
        // Pause/resume controls eligibility, not the identity of an occurrence
        // already running. Its completion must still consume that same once.
        let generation_changed = timing_changed;
        let admission_changed = timing_changed || enabled != current.enabled;
        let once = p
            .schedule
            .as_ref()
            .unwrap_or(&current.schedule)
            .get("cadence")
            .and_then(Value::as_str)
            == Some("once");
        let reset_once = timing_changed && once;
        let now = fmt(Utc::now());
        sqlx::query(
            "UPDATE scheduled_tasks SET \
               name = COALESCE(?, name), \
               prompt = COALESCE(?, prompt), \
               skill = CASE WHEN ? THEN ? ELSE skill END, \
               provider = COALESCE(?, provider), \
               model = COALESCE(?, model), \
               cwd = COALESCE(?, cwd), \
               schedule_json = COALESCE(?, schedule_json), \
               destination_json = COALESCE(?, destination_json), \
               enabled = COALESCE(?, enabled), \
               timezone = COALESCE(?, timezone), \
               workflow_id = CASE WHEN ? THEN ? ELSE workflow_id END, \
               sandbox = COALESCE(?, sandbox), \
               max_retries = COALESCE(?, max_retries), \
               notify_on_change = COALESCE(?, notify_on_change), \
               attach_proof = COALESCE(?, attach_proof), \
               updated_at = ? \
             WHERE id = ?",
        )
        .bind(p.name)
        .bind(p.prompt)
        // skill is Option<Option<String>>: outer Some => set (possibly to NULL).
        .bind(p.skill.is_some())
        .bind(p.skill.flatten())
        .bind(p.provider)
        .bind(p.model)
        .bind(p.cwd)
        .bind(p.schedule.map(|v| v.to_string()))
        .bind(p.destination.map(|v| v.to_string()))
        .bind(p.enabled.map(|b| b as i64))
        .bind(p.timezone)
        .bind(p.workflow_id.is_some())
        .bind(p.workflow_id.flatten())
        .bind(p.sandbox)
        .bind(p.max_retries)
        .bind(p.notify_on_change.map(|b| b as i64))
        .bind(p.attach_proof.map(|b| b as i64))
        .bind(&now)
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(dberr("update scheduled task"))?;
        let row = sqlx::query("SELECT * FROM scheduled_tasks WHERE id=?")
            .bind(id)
            .fetch_one(&mut *tx)
            .await
            .map_err(dberr("updated scheduled task"))?;
        let updated = row_to_task(&row)?;
        let next = if rearm {
            next_run(&updated)
        } else {
            updated.next_run_at.clone()
        };
        let row=sqlx::query("UPDATE scheduled_tasks SET schedule_generation=schedule_generation+?, admission_generation=admission_generation+?, armed_at=CASE WHEN ? THEN ? ELSE armed_at END, last_run_at=CASE WHEN ? THEN NULL ELSE last_run_at END, next_run_at=? WHERE id=? RETURNING *")
            .bind(i64::from(generation_changed)).bind(i64::from(admission_changed)).bind(rearm).bind(&now).bind(reset_once).bind(next).bind(id)
            .fetch_one(&mut *tx).await.map_err(dberr("rearm edited scheduled task"))?;
        let result = row_to_task(&row)?;
        tx.commit()
            .await
            .map_err(dberr("commit scheduled task edit"))?;
        Ok(result)
    }

    /// Old runs still finish their independent history row, but can only advance
    /// the occurrence whose generation was captured with their task definition.
    pub async fn settle_generation(
        &self,
        id: &str,
        generation: i64,
        last_run_at: Option<&str>,
        status: &str,
        next_run_at: Option<&str>,
    ) -> Result<bool> {
        let changed=sqlx::query("UPDATE scheduled_tasks SET last_run_at=COALESCE(?,last_run_at),last_status=?,next_run_at=?,updated_at=? WHERE id=? AND schedule_generation=?")
            .bind(last_run_at).bind(status).bind(next_run_at).bind(fmt(Utc::now())).bind(id).bind(generation)
            .execute(&self.pool).await.map_err(dberr("settle scheduled occurrence"))?.rows_affected();
        Ok(changed > 0)
    }

    /// Legacy fixture helper. Production completions must compare generations.
    #[cfg(test)]
    pub async fn set_runtime(
        &self,
        id: &str,
        last_run_at: Option<&str>,
        last_status: &str,
        next_run_at: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE scheduled_tasks SET last_run_at = COALESCE(?, last_run_at), \
             last_status = ?, next_run_at = ?, updated_at = ? WHERE id = ?",
        )
        .bind(last_run_at)
        .bind(last_status)
        .bind(next_run_at)
        .bind(fmt(Utc::now()))
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("set scheduled task runtime"))?;
        Ok(())
    }

    /// Record a manual run's outcome as the task's `last_status` without
    /// touching the scheduler cursor or `next_run_at`.
    pub async fn set_last_status(&self, id: &str, last_status: &str) -> Result<()> {
        sqlx::query("UPDATE scheduled_tasks SET last_status = ?, updated_at = ? WHERE id = ?")
            .bind(last_status)
            .bind(fmt(Utc::now()))
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("set scheduled task last status"))?;
        Ok(())
    }

    /// Re-arm a task's schedule at `at` (see `ScheduledTask::armed_at`);
    /// `reset_once` also forgets a fired `once` (its cursor is the fired flag).
    #[cfg(test)]
    pub async fn rearm(&self, id: &str, at: &str, reset_once: bool) -> Result<()> {
        sqlx::query(
            "UPDATE scheduled_tasks SET armed_at = ?, schedule_generation = schedule_generation + 1, \
             last_run_at = CASE WHEN ? THEN NULL ELSE last_run_at END WHERE id = ?",
        )
        .bind(at)
        .bind(reset_once)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("rearm scheduled task"))?;
        Ok(())
    }

    pub async fn delete(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM scheduled_tasks WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete scheduled task"))?;
        Ok(())
    }

    // -- Runs ----------------------------------------------------------------

    /// Claim a captured dispatch and open its history row under one write lock.
    /// Scheduled snapshots expire on timing/eligibility changes or another
    /// admission. Manual runs may explicitly run disabled tasks. Neither kind
    /// overlaps a running task, and insertion failure rolls back the claim.
    /// Content-only edits preserve eligibility: execution uses the definition
    /// captured by the caller, including its prompt and delivery destination.
    pub async fn admit_run(
        &self,
        captured: &ScheduledTask,
        trigger: &str,
    ) -> Result<ScheduledTaskRun> {
        let snapshot = serde_json::to_string(&AdmittedTaskSnapshot {
            version: 1,
            schedule_generation: captured.schedule_generation,
            admission_generation: captured.admission_generation,
            task: captured.clone(),
        })
        .map_err(|e| otto_core::Error::Internal(format!("encode admission snapshot: {e}")))?;
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(dberr("begin scheduled task admission"))?;
        let claimed = sqlx::query(
            "UPDATE scheduled_tasks SET admission_generation=admission_generation+1 \
             WHERE id=? AND workspace_id=? \
             AND (? != 'schedule' OR (enabled=1 AND admission_generation=? \
                  AND schedule_generation=? AND last_run_at IS ?)) \
             AND NOT EXISTS (SELECT 1 FROM scheduled_task_runs \
                             WHERE task_id=scheduled_tasks.id AND status='running')",
        )
        .bind(&captured.id)
        .bind(&captured.workspace_id)
        .bind(trigger)
        .bind(captured.admission_generation)
        .bind(captured.schedule_generation)
        .bind(&captured.last_run_at)
        .execute(&mut *tx)
        .await
        .map_err(dberr("claim scheduled task admission"))?
        .rows_affected();
        if claimed == 0 {
            return Err(otto_core::Error::Conflict(
                "task dispatch is no longer eligible or a run is already in progress".into(),
            ));
        }
        let id = new_id();
        let now = fmt(Utc::now());
        let row = sqlx::query(
            "INSERT INTO scheduled_task_runs (id, task_id, workspace_id, status, trigger, \
             started_at, summary, delivered, created_at, admitted_task_json) \
             VALUES (?, ?, ?, 'running', ?, ?, '', 0, ?, ?) RETURNING *",
        )
        .bind(&id)
        .bind(&captured.id)
        .bind(&captured.workspace_id)
        .bind(trigger)
        .bind(&now)
        .bind(&now)
        .bind(snapshot)
        .fetch_one(&mut *tx)
        .await
        .map_err(dberr("insert admitted scheduled run"))?;
        let run = row_to_run(&row)?;
        tx.commit()
            .await
            .map_err(dberr("commit scheduled task admission"))?;
        Ok(run)
    }

    /// The immutable definition admitted with this run. Missing legacy data,
    /// invalid versions and malformed snapshots are errors: callers must not
    /// substitute the editable task's current schedule or delivery destination.
    pub async fn get_run_task_snapshot(&self, run_id: &str) -> Result<ScheduledTask> {
        let row = sqlx::query(
            "SELECT task_id, workspace_id, admitted_task_json FROM scheduled_task_runs WHERE id=?",
        )
        .bind(run_id)
        .fetch_one(&self.pool)
        .await
        .map_err(dberr("read admission snapshot"))?;
        let raw: Option<String> = row.get("admitted_task_json");
        let raw = raw.ok_or_else(|| {
            otto_core::Error::Internal(
                "admission snapshot is unavailable for this legacy run".into(),
            )
        })?;
        let snapshot: AdmittedTaskSnapshot = serde_json::from_str(&raw)
            .map_err(|_| otto_core::Error::Internal("admission snapshot is malformed".into()))?;
        if snapshot.version != 1
            || snapshot.schedule_generation < 0
            || snapshot.admission_generation < 0
            || snapshot.task.id != row.get::<String, _>("task_id")
            || snapshot.task.workspace_id != row.get::<String, _>("workspace_id")
        {
            return Err(otto_core::Error::Internal(
                "admission snapshot has an unsupported version or invalid ownership".into(),
            ));
        }
        let mut task = snapshot.task;
        task.schedule_generation = snapshot.schedule_generation;
        task.admission_generation = snapshot.admission_generation;
        Ok(task)
    }

    /// Raw history insertion for import/fixtures. Engine dispatch uses `admit_run`.
    pub async fn create_run(&self, r: NewRun) -> Result<ScheduledTaskRun> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO scheduled_task_runs (id, task_id, workspace_id, status, trigger, \
             started_at, summary, delivered, created_at) \
             VALUES (?, ?, ?, 'running', ?, ?, '', 0, ?)",
        )
        .bind(&id)
        .bind(&r.task_id)
        .bind(&r.workspace_id)
        .bind(&r.trigger)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create scheduled task run"))?;
        self.get_run(&id).await
    }

    /// Settle a run. A `None` session id keeps the one [`Self::set_run_session`]
    /// recorded when the agent session opened — a failed run is exactly when the
    /// user needs to open that session to see why.
    pub async fn finish_run(&self, run_id: &str, f: FinishRun) -> Result<()> {
        sqlx::query(
            "UPDATE scheduled_task_runs SET status = ?, summary = ?, report_path = ?, \
             report_rel = ?, delivered = ?, delivery_error = ?, error = ?, \
             session_id = COALESCE(?, session_id), \
             report_hash = ?, proof_pack_id = ?, attempts = ?, skipped_delivery = ?, \
             workflow_run_id = COALESCE(?, workflow_run_id), finished_at = ? WHERE id = ?",
        )
        .bind(&f.status)
        .bind(&f.summary)
        .bind(&f.report_path)
        .bind(&f.report_rel)
        .bind(f.delivered as i64)
        .bind(&f.delivery_error)
        .bind(&f.error)
        .bind(&f.session_id)
        .bind(&f.report_hash)
        .bind(&f.proof_pack_id)
        .bind(f.attempts.max(1))
        .bind(f.skipped_delivery as i64)
        .bind(&f.workflow_run_id)
        .bind(fmt(Utc::now()))
        .bind(run_id)
        .execute(&self.pool)
        .await
        .map_err(dberr("finish scheduled task run"))?;
        Ok(())
    }

    /// Persist the live session id as soon as the run's agent session is created,
    /// so the UI can Open it while the run is still in flight.
    pub async fn set_run_session(&self, run_id: &str, session_id: &str) -> Result<()> {
        sqlx::query("UPDATE scheduled_task_runs SET session_id = ? WHERE id = ?")
            .bind(session_id)
            .bind(run_id)
            .execute(&self.pool)
            .await
            .map_err(dberr("set run session"))?;
        Ok(())
    }

    /// Admit and link a child in one write transaction. Cancellation takes the
    /// same lock, so it either fences admission or observes and cancels the child.
    pub async fn admit_workflow_handoff(
        &self,
        run_id: &str,
        workflow_id: &otto_core::Id,
        workspace_id: &otto_core::Id,
        input: &Value,
    ) -> Result<Option<otto_core::workflows::WorkflowRun>> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(dberr("begin scheduled workflow handoff"))?;
        let eligible: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM scheduled_task_runs WHERE id = ? AND workspace_id = ? AND status = 'running' AND workflow_run_id IS NULL)")
            .bind(run_id).bind(workspace_id).fetch_one(&mut *tx).await
            .map_err(dberr("check scheduled workflow parent"))?;
        if !eligible {
            return Err(otto_core::Error::Conflict(
                "scheduled run is stopped or already handed off".into(),
            ));
        }
        let busy: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM workflow_runs WHERE workflow_id = ? AND status IN ('pending','running'))")
            .bind(workflow_id).fetch_one(&mut *tx).await
            .map_err(dberr("check scheduled workflow overlap"))?;
        if busy {
            return Ok(None);
        }
        let child =
            crate::WorkflowsRepo::insert_run(&mut tx, workflow_id, workspace_id, input, None)
                .await?;
        sqlx::query("UPDATE scheduled_task_runs SET workflow_run_id = ? WHERE id = ?")
            .bind(&child.id)
            .bind(run_id)
            .execute(&mut *tx)
            .await
            .map_err(dberr("link scheduled workflow child"))?;
        tx.commit()
            .await
            .map_err(dberr("commit scheduled workflow handoff"))?;
        Ok(Some(child))
    }

    /// Cancel the parent and its admitted workflow together. A crash after
    /// this commit cannot leave a pending child for boot recovery to launch.
    pub async fn cancel_handoff(&self, run_id: &str) -> Result<ScheduledTaskRun> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(dberr("begin scheduled workflow cancellation"))?;
        sqlx::query("UPDATE scheduled_task_runs SET status = 'canceled', finished_at = ? WHERE id = ? AND status = 'running'")
            .bind(fmt(Utc::now())).bind(run_id).execute(&mut *tx).await
            .map_err(dberr("fence scheduled workflow parent"))?;
        let row = sqlx::query("SELECT * FROM scheduled_task_runs WHERE id = ?")
            .bind(run_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(dberr("read canceled scheduled run"))?;
        let run = row_to_run(&row)?;
        let mut canceled_child = None;
        if run.status == "canceled" {
            if let Some(child) = &run.workflow_run_id {
                if crate::WorkflowsRepo::cancel_in_transaction(&mut tx, child)
                    .await?
                    .is_some()
                {
                    canceled_child = Some(child.clone());
                }
            }
        }
        tx.commit()
            .await
            .map_err(dberr("commit scheduled workflow cancellation"))?;
        if let Some(child) = canceled_child {
            crate::workflows::announce_cancel(&child);
        }
        Ok(run)
    }

    /// Link the workflow run a hand-off launched as soon as it exists (the run
    /// row can open it while it runs; a Stop cancels it).
    pub async fn set_run_workflow_run(&self, run_id: &str, workflow_run_id: &str) -> Result<()> {
        sqlx::query("UPDATE scheduled_task_runs SET workflow_run_id = ? WHERE id = ?")
            .bind(workflow_run_id)
            .bind(run_id)
            .execute(&self.pool)
            .await
            .map_err(dberr("set run workflow run"))?;
        Ok(())
    }

    /// An unchanged report suppresses delivery only when the latest successful
    /// run delivered (or already inherited a delivered baseline) to the same
    /// destination without error. Inspect the latest row first: filtering out a
    /// failed delivery would incorrectly fall back to an older matching report.
    pub async fn last_ok_report_hash(
        &self,
        task_id: &str,
        exclude_run: &str,
        destination: &serde_json::Value,
    ) -> Result<Option<String>> {
        let row = sqlx::query(
            "SELECT report_hash, delivery_error, delivered, skipped_delivery, admitted_task_json \
             FROM scheduled_task_runs WHERE task_id = ? AND status = 'ok' \
             AND id != ? AND report_hash IS NOT NULL ORDER BY started_at DESC, id DESC LIMIT 1",
        )
        .bind(task_id)
        .bind(exclude_run)
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("last ok report hash"))?;
        let Some(row) = row else {
            return Ok(None);
        };
        if row.get::<Option<String>, _>("delivery_error").is_some()
            || !(row.get::<bool, _>("delivered") || row.get::<bool, _>("skipped_delivery"))
        {
            return Ok(None);
        }
        let snapshot = row
            .get::<Option<String>, _>("admitted_task_json")
            .and_then(|raw| serde_json::from_str::<AdmittedTaskSnapshot>(&raw).ok());
        // Legacy rows have no destination snapshot: deliver once conservatively.
        if snapshot
            .as_ref()
            .filter(|s| s.version == 1)
            .map(|s| &s.task.destination)
            != Some(destination)
        {
            return Ok(None);
        }
        Ok(row.get::<Option<String>, _>("report_hash"))
    }

    pub async fn get_run(&self, run_id: &str) -> Result<ScheduledTaskRun> {
        let row = sqlx::query("SELECT * FROM scheduled_task_runs WHERE id = ?")
            .bind(run_id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("scheduled task run not found"))?;
        row_to_run(&row)
    }

    pub async fn list_runs(&self, task_id: &str, limit: i64) -> Result<Vec<ScheduledTaskRun>> {
        let rows = sqlx::query(
            "SELECT * FROM scheduled_task_runs WHERE task_id = ? ORDER BY started_at DESC LIMIT ?",
        )
        .bind(task_id)
        .bind(limit.max(1))
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list scheduled task runs"))?;
        rows.iter().map(row_to_run).collect()
    }

    /// Delete all but the most-recent `keep` runs for a task. Only return report
    /// paths no remaining run owns: historical runs can share timestamp paths.
    pub async fn prune_runs(&self, task_id: &str, keep: i64) -> Result<Vec<String>> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(dberr("begin scheduled run pruning"))?;
        let rows = sqlx::query(
            "DELETE FROM scheduled_task_runs WHERE id IN \
             (SELECT id FROM scheduled_task_runs WHERE task_id=? \
              ORDER BY started_at DESC, id DESC LIMIT -1 OFFSET ?) RETURNING report_path",
        )
        .bind(task_id)
        .bind(keep.max(0))
        .fetch_all(&mut *tx)
        .await
        .map_err(dberr("prune scheduled task runs"))?;
        let candidates: std::collections::BTreeSet<String> = rows
            .iter()
            .filter_map(|r| r.get::<Option<String>, _>("report_path"))
            .collect();
        let mut paths = Vec::new();
        for path in candidates {
            let referenced: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM scheduled_task_runs WHERE report_path=?)",
            )
            .bind(&path)
            .fetch_one(&mut *tx)
            .await
            .map_err(dberr("check retained report ownership"))?;
            if !referenced {
                paths.push(path);
            }
        }
        tx.commit()
            .await
            .map_err(dberr("commit scheduled run pruning"))?;
        Ok(paths)
    }

    /// Mark every still-`running` run as `error` — called once at scheduler start
    /// to clear zombie rows left by a daemon restart. Returns the count.
    ///
    /// A workflow hand-off (`workflow_run_id` set) is NOT reaped: the workflow
    /// itself survives the restart (boot recovery resumes or settles it), so
    /// the engine re-attaches a waiter instead and records its REAL outcome
    /// (S3-303) — see [`Self::list_running_workflow_handoffs`].
    pub async fn reap_running(&self) -> Result<u64> {
        let res = sqlx::query(
            "UPDATE scheduled_task_runs SET status = 'error', \
             error = 'interrupted by daemon restart', finished_at = ? \
             WHERE status = 'running' AND workflow_run_id IS NULL",
        )
        .bind(fmt(Utc::now()))
        .execute(&self.pool)
        .await
        .map_err(dberr("reap running scheduled task runs"))?;
        Ok(res.rows_affected())
    }

    /// `running` rows that handed off to a workflow run — left alone by
    /// [`Self::reap_running`] so boot can re-attach their waiters (S3-303).
    pub async fn list_running_workflow_handoffs(&self) -> Result<Vec<ScheduledTaskRun>> {
        let rows = sqlx::query(
            "SELECT * FROM scheduled_task_runs WHERE status = 'running' \
             AND workflow_run_id IS NOT NULL ORDER BY started_at",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list running workflow hand-offs"))?;
        rows.iter().map(row_to_run).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    async fn pool() -> DbPool {
        crate::db::test_pool().await
    }

    async fn seed_ws(pool: &DbPool, id: &str) {
        let now = fmt(Utc::now());
        sqlx::query("INSERT INTO workspaces (id, name, root_path, created_at) VALUES (?, ?, ?, ?)")
            .bind(id)
            .bind("ws")
            .bind("/tmp/ws")
            .bind(&now)
            .execute(pool)
            .await
            .unwrap();
    }

    fn new_task(ws: &str, name: &str) -> NewScheduledTask {
        NewScheduledTask {
            prompt: "do the thing".into(),
            schedule: json!({"cadence":"interval","every_min":60}),
            destination: json!({"type":"none"}),
            created_by: Some("u1".into()),
            ..NewScheduledTask::defaults(ws.into(), name.into())
        }
    }

    async fn workflow_parent() -> (
        DbPool,
        ScheduledTasksRepo,
        ScheduledTaskRun,
        otto_core::workflows::Workflow,
    ) {
        let p = pool().await;
        seed_ws(&p, "handoff-ws").await;
        let repo = ScheduledTasksRepo::new(p.clone());
        let task = repo
            .create(NewScheduledTask::defaults(
                "handoff-ws".into(),
                "handoff".into(),
            ))
            .await
            .unwrap();
        let run = repo.admit_run(&task, "manual").await.unwrap();
        let user = crate::UsersRepo::new(p.clone())
            .create("handoff-user", "hash", "User", false)
            .await
            .unwrap();
        let wf = crate::WorkflowsRepo::new(p.clone())
            .create(
                &task.workspace_id,
                "child",
                "",
                "",
                &Default::default(),
                &user.id,
            )
            .await
            .unwrap();
        (p, repo, run, wf)
    }

    #[tokio::test]
    async fn recovery_handoff_link_failure_leaves_no_orphan_child() {
        let (p, repo, parent, wf) = workflow_parent().await;
        sqlx::query("CREATE TRIGGER reject_handoff BEFORE UPDATE OF workflow_run_id ON scheduled_task_runs BEGIN SELECT RAISE(ABORT, 'link failed'); END").execute(&p).await.unwrap();
        assert!(repo
            .admit_workflow_handoff(&parent.id, &wf.id, &wf.workspace_id, &Value::Null)
            .await
            .is_err());
        let children: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM workflow_runs")
            .fetch_one(&p)
            .await
            .unwrap();
        assert_eq!(
            children, 0,
            "failed parent linkage must roll back child admission"
        );
    }

    #[tokio::test]
    async fn recovery_handoff_cancel_fences_late_admission() {
        let (p, repo, parent, wf) = workflow_parent().await;
        repo.cancel_handoff(&parent.id).await.unwrap();
        assert!(repo
            .admit_workflow_handoff(&parent.id, &wf.id, &wf.workspace_id, &Value::Null)
            .await
            .is_err());
        let children: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM workflow_runs")
            .fetch_one(&p)
            .await
            .unwrap();
        assert_eq!(children, 0);
    }

    #[tokio::test]
    async fn recovery_handoff_cancel_observes_admitted_child() {
        let (p, repo, parent, wf) = workflow_parent().await;
        let child = repo
            .admit_workflow_handoff(&parent.id, &wf.id, &wf.workspace_id, &Value::Null)
            .await
            .unwrap()
            .unwrap();
        let stopped = repo.cancel_handoff(&parent.id).await.unwrap();
        assert_eq!(stopped.status, "canceled");
        assert_eq!(stopped.workflow_run_id.as_deref(), Some(child.id.as_str()));
        let child = crate::WorkflowsRepo::new(p)
            .get_run(&child.id)
            .await
            .unwrap();
        assert_eq!(
            child.status,
            otto_core::workflows::RunStatus::Canceled,
            "restart must not revive a stopped parent child"
        );
    }

    #[tokio::test]
    async fn recovery_handoff_admission_racing_cancel_never_leaves_live_child() {
        let (p, repo, parent, wf) = workflow_parent().await;
        let (admitted, canceled) = tokio::join!(
            repo.admit_workflow_handoff(&parent.id, &wf.id, &wf.workspace_id, &Value::Null),
            repo.cancel_handoff(&parent.id),
        );
        assert_eq!(canceled.unwrap().status, "canceled");
        let live: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM workflow_runs WHERE status IN ('pending','running')",
        )
        .fetch_one(&p)
        .await
        .unwrap();
        assert_eq!(live, 0);
        if let Ok(Some(child)) = admitted {
            assert_eq!(
                repo.get_run(&parent.id).await.unwrap().workflow_run_id,
                Some(child.id)
            );
        }
    }

    #[tokio::test]
    async fn review5_scheduled_admission_insert_failure_rolls_back_claim() {
        let p = pool().await;
        seed_ws(&p, "rollback-ws").await;
        let repo = ScheduledTasksRepo::new(p.clone());
        let captured = repo
            .create(NewScheduledTask::defaults(
                "rollback-ws".into(),
                "rollback".into(),
            ))
            .await
            .unwrap();
        sqlx::query("CREATE TRIGGER reject_scheduled_admission BEFORE INSERT ON scheduled_task_runs BEGIN SELECT RAISE(ABORT, 'injected insertion failure'); END")
            .execute(&p).await.unwrap();
        assert!(repo.admit_run(&captured, "schedule").await.is_err());
        let after = repo.get(&captured.id).await.unwrap();
        assert_eq!(after.admission_generation, captured.admission_generation);
        assert_eq!(after.schedule_generation, captured.schedule_generation);
        assert_eq!(after.last_run_at, captured.last_run_at);
        assert!(repo.list_runs(&captured.id, 10).await.unwrap().is_empty());
        sqlx::query("DROP TRIGGER reject_scheduled_admission")
            .execute(&p)
            .await
            .unwrap();
        repo.admit_run(&captured, "schedule").await.unwrap();
    }

    #[tokio::test]
    async fn review5_scheduled_admission_consumed_snapshot_cannot_replay_before_settlement() {
        let p = pool().await;
        seed_ws(&p, "replay-ws").await;
        let repo = ScheduledTasksRepo::new(p);
        let captured = repo
            .create(NewScheduledTask::defaults(
                "replay-ws".into(),
                "replay".into(),
            ))
            .await
            .unwrap();
        let run = repo.admit_run(&captured, "schedule").await.unwrap();
        repo.finish_run(
            &run.id,
            FinishRun {
                status: "ok".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        // Engine history and cadence settlement are separate operations. A
        // captured scan cannot replay in the gap, even without a running row.
        assert!(repo.admit_run(&captured, "schedule").await.is_err());
        assert_eq!(repo.list_runs(&captured.id, 10).await.unwrap().len(), 1);
        let after = repo.get(&captured.id).await.unwrap();
        assert_eq!(after.last_run_at, captured.last_run_at);
        assert_eq!(after.schedule_generation, captured.schedule_generation);
    }

    /// Perf W12 budget: the scheduled-task scheduler's per-minute scan is ONE
    /// statement for any number of enabled tasks.
    #[tokio::test]
    async fn tick_scan_is_one_query_for_any_n() {
        let p = pool().await;
        seed_ws(&p, "ws1").await;
        let repo = ScheduledTasksRepo::new(p.clone());
        for i in 0..8 {
            repo.create(new_task("ws1", &format!("t{i}")))
                .await
                .unwrap();
        }
        let probe = p.statement_probe();
        probe.reset();
        assert_eq!(repo.list_enabled().await.unwrap().len(), 8);
        assert_eq!(probe.take().len(), 1);
    }

    #[tokio::test]
    async fn create_get_list() {
        let p = pool().await;
        seed_ws(&p, "ws1").await;
        let repo = ScheduledTasksRepo::new(p.clone());
        let t = repo.create(new_task("ws1", "nightly")).await.unwrap();
        assert_eq!(t.name, "nightly");
        assert_eq!(t.schedule["every_min"], 60);
        let got = repo.get(&t.id).await.unwrap();
        assert_eq!(got.id, t.id);
        let list = repo.list_by_workspace("ws1").await.unwrap();
        assert_eq!(list.len(), 1);
    }

    #[tokio::test]
    async fn list_enabled_excludes_disabled() {
        let p = pool().await;
        seed_ws(&p, "ws1").await;
        let repo = ScheduledTasksRepo::new(p.clone());
        let on = repo.create(new_task("ws1", "on")).await.unwrap();
        let mut off = new_task("ws1", "off");
        off.enabled = false;
        repo.create(off).await.unwrap();
        let enabled = repo.list_enabled().await.unwrap();
        assert_eq!(enabled.len(), 1);
        assert_eq!(enabled[0].id, on.id);
    }

    #[tokio::test]
    async fn update_changes_fields_and_clears_skill() {
        let p = pool().await;
        seed_ws(&p, "ws1").await;
        let repo = ScheduledTasksRepo::new(p.clone());
        let mut nt = new_task("ws1", "t");
        nt.skill = Some("db-mysql".into());
        let t = repo.create(nt).await.unwrap();
        assert_eq!(t.skill.as_deref(), Some("db-mysql"));
        let upd = repo
            .update(
                &t.id,
                ScheduledTaskPatch {
                    name: Some("renamed".into()),
                    enabled: Some(false),
                    skill: Some(None), // explicit clear
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(upd.name, "renamed");
        assert!(!upd.enabled);
        assert_eq!(upd.skill, None);
    }

    #[tokio::test]
    async fn set_runtime_persists_cursor() {
        let p = pool().await;
        seed_ws(&p, "ws1").await;
        let repo = ScheduledTasksRepo::new(p.clone());
        let t = repo.create(new_task("ws1", "t")).await.unwrap();
        repo.set_runtime(
            &t.id,
            Some("2026-06-26T10:00:00+00:00"),
            "ok",
            Some("2026-06-26T11:00:00+00:00"),
        )
        .await
        .unwrap();
        let got = repo.get(&t.id).await.unwrap();
        assert_eq!(got.last_status.as_deref(), Some("ok"));
        assert!(got.last_run_at.is_some());
        assert!(got.next_run_at.is_some());
    }

    #[tokio::test]
    async fn runs_create_finish_list() {
        let p = pool().await;
        seed_ws(&p, "ws1").await;
        let repo = ScheduledTasksRepo::new(p.clone());
        let t = repo.create(new_task("ws1", "t")).await.unwrap();
        let run = repo
            .create_run(NewRun {
                task_id: t.id.clone(),
                workspace_id: "ws1".into(),
                trigger: "manual".into(),
            })
            .await
            .unwrap();
        assert_eq!(run.status, "running");
        repo.finish_run(
            &run.id,
            FinishRun {
                status: "ok".into(),
                summary: "Reviewed: 1".into(),
                report_path: Some("/x/r.md".into()),
                report_rel: Some("t/reports/r.md".into()),
                delivered: true,
                session_id: Some("sess-1".into()),
                report_hash: Some("abc123".into()),
                attempts: 2,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let got = repo.get_run(&run.id).await.unwrap();
        assert_eq!(got.status, "ok");
        assert_eq!(got.summary, "Reviewed: 1");
        assert!(got.delivered);
        assert_eq!(got.session_id.as_deref(), Some("sess-1"));
        assert_eq!(got.attempts, 2);
        // A legacy create_run row has no destination snapshot, so re-deliver once.
        let h = repo
            .last_ok_report_hash(&t.id, "other-run", &t.destination)
            .await
            .unwrap();
        assert_eq!(h, None);
        let runs = repo.list_runs(&t.id, 10).await.unwrap();
        assert_eq!(runs.len(), 1);
    }

    #[tokio::test]
    async fn notification_baseline_does_not_hide_failed_delivery() {
        let p = pool().await;
        seed_ws(&p, "ws1").await;
        let repo = ScheduledTasksRepo::new(p.clone());
        let task = repo
            .create(new_task("ws1", "delivery retry"))
            .await
            .unwrap();
        for (delivered, skipped, error, expected) in [
            (true, false, None, Some("same-report")),
            (false, false, Some("fixture send failure"), None),
            (true, false, Some("fixture attachment failure"), None),
            (true, false, None, Some("same-report")),
            (false, true, None, Some("same-report")),
            (false, false, None, None),
        ] {
            let run = repo.admit_run(&task, "manual").await.unwrap();
            repo.finish_run(
                &run.id,
                FinishRun {
                    status: "ok".into(),
                    delivered,
                    skipped_delivery: skipped,
                    delivery_error: error.map(str::to_owned),
                    report_hash: Some("same-report".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
            assert_eq!(
                repo.last_ok_report_hash(&task.id, "next-run", &task.destination)
                    .await
                    .unwrap()
                    .as_deref(),
                expected
            );
        }
    }

    #[tokio::test]
    async fn notification_baseline_is_bound_to_the_admitted_destination() {
        let p = pool().await;
        seed_ws(&p, "ws1").await;
        let repo = ScheduledTasksRepo::new(p);
        let mut input = new_task("ws1", "destination ownership");
        input.destination = json!({"type":"email", "to":"first@example.invalid"});
        let task = repo.create(input).await.unwrap();
        let run = repo.admit_run(&task, "manual").await.unwrap();
        repo.finish_run(
            &run.id,
            FinishRun {
                status: "ok".into(),
                delivered: true,
                report_hash: Some("unchanged".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            repo.last_ok_report_hash(&task.id, "next", &task.destination)
                .await
                .unwrap()
                .as_deref(),
            Some("unchanged")
        );
        let changed = json!({"type":"email", "to":"second@example.invalid"});
        assert_eq!(
            repo.last_ok_report_hash(&task.id, "next", &changed)
                .await
                .unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn prune_keeps_recent_and_returns_paths() {
        let p = pool().await;
        seed_ws(&p, "ws1").await;
        let repo = ScheduledTasksRepo::new(p.clone());
        let t = repo.create(new_task("ws1", "t")).await.unwrap();
        for i in 0..5 {
            let r = repo
                .create_run(NewRun {
                    task_id: t.id.clone(),
                    workspace_id: "ws1".into(),
                    trigger: "schedule".into(),
                })
                .await
                .unwrap();
            repo.finish_run(
                &r.id,
                FinishRun {
                    status: "ok".into(),
                    report_path: Some(format!("/x/{i}.md")),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        }
        let deleted = repo.prune_runs(&t.id, 2).await.unwrap();
        assert_eq!(deleted.len(), 3);
        assert_eq!(repo.list_runs(&t.id, 100).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn quality_report_ownership_lookup_does_not_scan_run_history() {
        let p = pool().await;
        let rows = sqlx::query(
            "EXPLAIN QUERY PLAN SELECT EXISTS(SELECT 1 FROM scheduled_task_runs WHERE report_path=?)",
        )
        .bind("/tmp/retained-report.md")
        .fetch_all(&p)
        .await
        .unwrap();
        let plan: Vec<String> = rows.iter().map(|row| row.get("detail")).collect();
        assert!(
            plan.iter()
                .any(|step| step.contains("SEARCH scheduled_task_runs")),
            "each pruned report must use an indexed ownership lookup: {plan:?}"
        );
    }

    #[tokio::test]
    async fn quality_admission_snapshot_preserves_nonzero_epochs_and_original_content() {
        let p = pool().await;
        seed_ws(&p, "ws1").await;
        let repo = ScheduledTasksRepo::new(p.clone());
        let initial = repo.create(new_task("ws1", "original")).await.unwrap();
        let captured = repo
            .update(
                &initial.id,
                ScheduledTaskPatch {
                    schedule: Some(json!({"cadence":"once", "run_at":"2000-01-01T10:00:00Z"})),
                    destination: Some(json!({"type":"email", "to":"original@example.invalid"})),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(captured.schedule_generation > 0);
        assert!(captured.admission_generation > 0);
        let run = repo.admit_run(&captured, "schedule").await.unwrap();
        repo.update(
            &captured.id,
            ScheduledTaskPatch {
                name: Some("edited".into()),
                prompt: Some("edited prompt".into()),
                destination: Some(json!({"type":"none"})),
                attach_proof: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        // Older SELECT * readers and finishing writes ignore the new column.
        let read = repo.get_run(&run.id).await.unwrap();
        assert_eq!(read.task_id, captured.id);
        assert!(serde_json::to_value(&read)
            .unwrap()
            .get("admitted_task_json")
            .is_none());
        repo.finish_run(
            &run.id,
            FinishRun {
                status: "ok".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let recovered = ScheduledTasksRepo::new(p)
            .get_run_task_snapshot(&run.id)
            .await
            .unwrap();
        assert_eq!(recovered.schedule_generation, captured.schedule_generation);
        assert_eq!(
            recovered.admission_generation,
            captured.admission_generation
        );
        assert_eq!(
            serde_json::to_value(&recovered).unwrap(),
            serde_json::to_value(&captured).unwrap()
        );
    }

    #[tokio::test]
    async fn quality_old_insert_read_finish_paths_remain_compatible_with_snapshot_schema() {
        let p = pool().await;
        seed_ws(&p, "ws1").await;
        let repo = ScheduledTasksRepo::new(p.clone());
        let task = repo.create(new_task("ws1", "legacy")).await.unwrap();
        // create_run retains the previous build's exact column list.
        let run = repo
            .create_run(NewRun {
                task_id: task.id,
                workspace_id: "ws1".into(),
                trigger: "schedule".into(),
            })
            .await
            .unwrap();
        let snapshot: Option<String> =
            sqlx::query_scalar("SELECT admitted_task_json FROM scheduled_task_runs WHERE id=?")
                .bind(&run.id)
                .fetch_one(&p)
                .await
                .unwrap();
        assert!(snapshot.is_none());
        assert!(repo
            .get_run_task_snapshot(&run.id)
            .await
            .unwrap_err()
            .to_string()
            .contains("legacy"));
        repo.finish_run(
            &run.id,
            FinishRun {
                status: "ok".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(repo.get_run(&run.id).await.unwrap().status, "ok");
    }

    #[tokio::test]
    async fn quality_invalid_admission_snapshots_are_rejected_without_current_task_fallback() {
        let p = pool().await;
        seed_ws(&p, "ws1").await;
        let repo = ScheduledTasksRepo::new(p.clone());
        let task = repo.create(new_task("ws1", "snapshot")).await.unwrap();
        let run = repo.admit_run(&task, "schedule").await.unwrap();
        let valid: String =
            sqlx::query_scalar("SELECT admitted_task_json FROM scheduled_task_runs WHERE id=?")
                .bind(&run.id)
                .fetch_one(&p)
                .await
                .unwrap();
        let value: Value = serde_json::from_str(&valid).unwrap();
        let mut invalid = vec!["{".to_string(), "{}".to_string(), "null".to_string()];
        for (key, replacement) in [
            ("version", json!(2)),
            ("schedule_generation", json!(-1)),
            ("admission_generation", json!(-1)),
        ] {
            let mut v = value.clone();
            v[key] = replacement;
            invalid.push(v.to_string());
        }
        for key in ["id", "workspace_id"] {
            let mut v = value.clone();
            v["task"][key] = json!("unrelated");
            invalid.push(v.to_string());
        }
        let mut missing_epoch = value;
        missing_epoch
            .as_object_mut()
            .unwrap()
            .remove("schedule_generation");
        invalid.push(missing_epoch.to_string());
        for malformed in invalid {
            sqlx::query("UPDATE scheduled_task_runs SET admitted_task_json=? WHERE id=?")
                .bind(malformed)
                .bind(&run.id)
                .execute(&p)
                .await
                .unwrap();
            assert!(repo.get_run_task_snapshot(&run.id).await.is_err());
            assert!(repo.get(&task.id).await.unwrap().last_run_at.is_none());
        }
    }

    #[tokio::test]
    async fn quality_prune_preserves_reports_still_owned_by_retained_runs() {
        let p = pool().await;
        seed_ws(&p, "ws1").await;
        let repo = ScheduledTasksRepo::new(p.clone());
        let task = repo.create(new_task("ws1", "shared report")).await.unwrap();
        for day in [1, 2] {
            let run = repo
                .create_run(NewRun {
                    task_id: task.id.clone(),
                    workspace_id: "ws1".into(),
                    trigger: "manual".into(),
                })
                .await
                .unwrap();
            repo.finish_run(
                &run.id,
                FinishRun {
                    status: "ok".into(),
                    report_path: Some("/legacy/shared.md".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
            sqlx::query("UPDATE scheduled_task_runs SET started_at=? WHERE id=?")
                .bind(format!("2026-10-0{day}T00:00:00Z"))
                .bind(&run.id)
                .execute(&p)
                .await
                .unwrap();
        }
        assert!(
            repo.prune_runs(&task.id, 1).await.unwrap().is_empty(),
            "a retained historical run still owns this colliding path"
        );
        assert_eq!(repo.list_runs(&task.id, 10).await.unwrap().len(), 1);
        assert_eq!(
            repo.prune_runs(&task.id, 0).await.unwrap(),
            vec!["/legacy/shared.md"]
        );
    }

    #[tokio::test]
    async fn reap_flips_running_to_error() {
        let p = pool().await;
        seed_ws(&p, "ws1").await;
        let repo = ScheduledTasksRepo::new(p.clone());
        let t = repo.create(new_task("ws1", "t")).await.unwrap();
        let r = repo
            .create_run(NewRun {
                task_id: t.id.clone(),
                workspace_id: "ws1".into(),
                trigger: "schedule".into(),
            })
            .await
            .unwrap();
        // S3-303: a workflow hand-off survives the reap (its waiter is
        // re-attached at boot instead).
        let handoff = repo
            .create_run(NewRun {
                task_id: t.id.clone(),
                workspace_id: "ws1".into(),
                trigger: "manual".into(),
            })
            .await
            .unwrap();
        repo.set_run_workflow_run(&handoff.id, "wf-run-1")
            .await
            .unwrap();
        let n = repo.reap_running().await.unwrap();
        assert_eq!(n, 1);
        assert_eq!(repo.get_run(&r.id).await.unwrap().status, "error");
        assert_eq!(repo.get_run(&handoff.id).await.unwrap().status, "running");
        let live = repo.list_running_workflow_handoffs().await.unwrap();
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].id, handoff.id);
        assert_eq!(live[0].workflow_run_id.as_deref(), Some("wf-run-1"));
    }

    #[tokio::test]
    async fn tasks_are_armed_at_creation_and_rearmable() {
        let p = pool().await;
        seed_ws(&p, "ws1").await;
        let repo = ScheduledTasksRepo::new(p.clone());
        let t = repo.create(new_task("ws1", "t")).await.unwrap();
        assert_eq!(t.armed_at.as_deref(), Some(t.created_at.as_str()));
        repo.set_runtime(&t.id, Some("2026-09-01T10:00:00+00:00"), "ok", None)
            .await
            .unwrap();
        repo.rearm(&t.id, "2026-09-02T00:00:00+00:00", false)
            .await
            .unwrap();
        let got = repo.get(&t.id).await.unwrap();
        assert_eq!(got.armed_at.as_deref(), Some("2026-09-02T00:00:00+00:00"));
        assert_eq!(
            got.last_run_at.as_deref(),
            Some("2026-09-01T10:00:00+00:00")
        );
        repo.rearm(&t.id, "2026-09-03T00:00:00+00:00", true)
            .await
            .unwrap();
        assert!(repo.get(&t.id).await.unwrap().last_run_at.is_none());
        // A manual run's status shows on the task; the cursor stays put.
        repo.set_last_status(&t.id, "error").await.unwrap();
        let got = repo.get(&t.id).await.unwrap();
        assert_eq!(got.last_status.as_deref(), Some("error"));
        assert!(got.last_run_at.is_none());
    }

    #[tokio::test]
    async fn finish_without_session_keeps_the_live_one() {
        let p = pool().await;
        seed_ws(&p, "ws1").await;
        let repo = ScheduledTasksRepo::new(p.clone());
        let t = repo.create(new_task("ws1", "t")).await.unwrap();
        let r = repo
            .create_run(NewRun {
                task_id: t.id.clone(),
                workspace_id: "ws1".into(),
                trigger: "manual".into(),
            })
            .await
            .unwrap();
        repo.set_run_session(&r.id, "sess-1").await.unwrap();
        repo.finish_run(
            &r.id,
            FinishRun {
                status: "error".into(),
                error: Some("agent run failed".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let got = repo.get_run(&r.id).await.unwrap();
        assert_eq!(got.status, "error");
        assert_eq!(got.session_id.as_deref(), Some("sess-1"));
        // An explicit session id still wins.
        repo.finish_run(
            &r.id,
            FinishRun {
                status: "ok".into(),
                session_id: Some("sess-2".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let got = repo.get_run(&r.id).await.unwrap();
        assert_eq!(got.session_id.as_deref(), Some("sess-2"));
    }

    #[tokio::test]
    async fn delete_cascades_runs() {
        let p = pool().await;
        seed_ws(&p, "ws1").await;
        let repo = ScheduledTasksRepo::new(p.clone());
        let t = repo.create(new_task("ws1", "t")).await.unwrap();
        repo.create_run(NewRun {
            task_id: t.id.clone(),
            workspace_id: "ws1".into(),
            trigger: "manual".into(),
        })
        .await
        .unwrap();
        repo.delete(&t.id).await.unwrap();
        assert!(repo.get(&t.id).await.is_err());
        assert_eq!(repo.list_runs(&t.id, 10).await.unwrap().len(), 0);
    }
}
