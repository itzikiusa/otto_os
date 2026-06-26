//! Persistence for **Scheduled Tasks** (migration `0084_scheduled_tasks.sql`).
//!
//! Two tables: `scheduled_tasks` (the recurring definition) and
//! `scheduled_task_runs` (one row per execution, the report history). Pure storage —
//! the cadence/cursor logic and report I/O live in `otto_server`. `schedule` and
//! `destination` are JSON columns surfaced as `serde_json::Value`. The `last_run_at`
//! cursor is advanced by the scheduler on run completion via [`set_runtime`].
//!
//! [`set_runtime`]: ScheduledTasksRepo::set_runtime

use chrono::Utc;
use otto_core::domain::{ScheduledTask, ScheduledTaskRun};
use otto_core::{new_id, Result};
use serde_json::Value;
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt, json};

#[derive(Clone)]
pub struct ScheduledTasksRepo {
    pool: SqlitePool,
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
}

/// Fields for opening a run row (status starts `running`).
#[derive(Clone, Debug)]
pub struct NewRun {
    pub task_id: String,
    pub workspace_id: String,
    pub trigger: String,
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
        last_run_at: r.get("last_run_at"),
        last_status: r.get("last_status"),
        next_run_at: r.get("next_run_at"),
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
        created_at: r.get("created_at"),
    })
}

impl ScheduledTasksRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    // -- Tasks ---------------------------------------------------------------

    pub async fn create(&self, t: NewScheduledTask) -> Result<ScheduledTask> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO scheduled_tasks (id, workspace_id, name, kind, prompt, skill, provider, \
             model, cwd, schedule_json, destination_json, enabled, created_by, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
        .bind(&t.created_by)
        .bind(&now)
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
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("update scheduled task"))?;
        self.get(id).await
    }

    /// Advance the scheduler cursor + display fields after a run completes.
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

    pub async fn delete(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM scheduled_tasks WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete scheduled task"))?;
        Ok(())
    }

    // -- Runs ----------------------------------------------------------------

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

    #[allow(clippy::too_many_arguments)]
    pub async fn finish_run(
        &self,
        run_id: &str,
        status: &str,
        summary: &str,
        report_path: Option<&str>,
        report_rel: Option<&str>,
        delivered: bool,
        delivery_error: Option<&str>,
        error: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE scheduled_task_runs SET status = ?, summary = ?, report_path = ?, \
             report_rel = ?, delivered = ?, delivery_error = ?, error = ?, finished_at = ? \
             WHERE id = ?",
        )
        .bind(status)
        .bind(summary)
        .bind(report_path)
        .bind(report_rel)
        .bind(delivered as i64)
        .bind(delivery_error)
        .bind(error)
        .bind(fmt(Utc::now()))
        .bind(run_id)
        .execute(&self.pool)
        .await
        .map_err(dberr("finish scheduled task run"))?;
        Ok(())
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

    /// Delete all but the most-recent `keep` runs for a task. Returns the
    /// `report_path`s of deleted rows so the caller can unlink the report files.
    pub async fn prune_runs(&self, task_id: &str, keep: i64) -> Result<Vec<String>> {
        let rows = sqlx::query(
            "SELECT id, report_path FROM scheduled_task_runs WHERE task_id = ? \
             ORDER BY started_at DESC LIMIT -1 OFFSET ?",
        )
        .bind(task_id)
        .bind(keep.max(0))
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("select prunable runs"))?;
        let mut paths = Vec::new();
        for r in &rows {
            let id: String = r.get("id");
            if let Some(p) = r.get::<Option<String>, _>("report_path") {
                paths.push(p);
            }
            let _ = sqlx::query("DELETE FROM scheduled_task_runs WHERE id = ?")
                .bind(&id)
                .execute(&self.pool)
                .await
                .map_err(dberr("prune scheduled task run"))?;
        }
        Ok(paths)
    }

    /// Mark every still-`running` run as `error` — called once at scheduler start
    /// to clear zombie rows left by a daemon restart. Returns the count.
    pub async fn reap_running(&self) -> Result<u64> {
        let res = sqlx::query(
            "UPDATE scheduled_task_runs SET status = 'error', \
             error = 'interrupted by daemon restart', finished_at = ? WHERE status = 'running'",
        )
        .bind(fmt(Utc::now()))
        .execute(&self.pool)
        .await
        .map_err(dberr("reap running scheduled task runs"))?;
        Ok(res.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    async fn pool() -> SqlitePool {
        crate::db::test_pool().await
    }

    async fn seed_ws(pool: &SqlitePool, id: &str) {
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
            workspace_id: ws.into(),
            name: name.into(),
            kind: "agent_prompt".into(),
            prompt: "do the thing".into(),
            skill: None,
            provider: "claude".into(),
            model: "".into(),
            cwd: "".into(),
            schedule: json!({"cadence":"interval","every_min":60}),
            destination: json!({"type":"none"}),
            enabled: true,
            created_by: Some("u1".into()),
        }
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
        repo.set_runtime(&t.id, Some("2026-06-26T10:00:00+00:00"), "ok", Some("2026-06-26T11:00:00+00:00"))
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
            .create_run(NewRun { task_id: t.id.clone(), workspace_id: "ws1".into(), trigger: "manual".into() })
            .await
            .unwrap();
        assert_eq!(run.status, "running");
        repo.finish_run(&run.id, "ok", "Reviewed: 1", Some("/x/r.md"), Some("t/reports/r.md"), true, None, None)
            .await
            .unwrap();
        let got = repo.get_run(&run.id).await.unwrap();
        assert_eq!(got.status, "ok");
        assert_eq!(got.summary, "Reviewed: 1");
        assert!(got.delivered);
        let runs = repo.list_runs(&t.id, 10).await.unwrap();
        assert_eq!(runs.len(), 1);
    }

    #[tokio::test]
    async fn prune_keeps_recent_and_returns_paths() {
        let p = pool().await;
        seed_ws(&p, "ws1").await;
        let repo = ScheduledTasksRepo::new(p.clone());
        let t = repo.create(new_task("ws1", "t")).await.unwrap();
        for i in 0..5 {
            let r = repo
                .create_run(NewRun { task_id: t.id.clone(), workspace_id: "ws1".into(), trigger: "schedule".into() })
                .await
                .unwrap();
            repo.finish_run(&r.id, "ok", "", Some(&format!("/x/{i}.md")), None, false, None, None)
                .await
                .unwrap();
        }
        let deleted = repo.prune_runs(&t.id, 2).await.unwrap();
        assert_eq!(deleted.len(), 3);
        assert_eq!(repo.list_runs(&t.id, 100).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn reap_flips_running_to_error() {
        let p = pool().await;
        seed_ws(&p, "ws1").await;
        let repo = ScheduledTasksRepo::new(p.clone());
        let t = repo.create(new_task("ws1", "t")).await.unwrap();
        let r = repo
            .create_run(NewRun { task_id: t.id.clone(), workspace_id: "ws1".into(), trigger: "schedule".into() })
            .await
            .unwrap();
        let n = repo.reap_running().await.unwrap();
        assert_eq!(n, 1);
        assert_eq!(repo.get_run(&r.id).await.unwrap().status, "error");
    }

    #[tokio::test]
    async fn delete_cascades_runs() {
        let p = pool().await;
        seed_ws(&p, "ws1").await;
        let repo = ScheduledTasksRepo::new(p.clone());
        let t = repo.create(new_task("ws1", "t")).await.unwrap();
        repo.create_run(NewRun { task_id: t.id.clone(), workspace_id: "ws1".into(), trigger: "manual".into() })
            .await
            .unwrap();
        repo.delete(&t.id).await.unwrap();
        assert!(repo.get(&t.id).await.is_err());
        assert_eq!(repo.list_runs(&t.id, 10).await.unwrap().len(), 0);
    }
}
