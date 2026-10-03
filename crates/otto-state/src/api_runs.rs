//! Durable API automation reports. Callers must redact before saving.
use crate::DbPool;
use otto_core::api::{ApiAutomationRun, ApiRunStepResult};
use sqlx::Row;

#[derive(Clone)]
pub struct ApiRunsRepo(pub DbPool);

/// Most recent runs per automation kept by [`ApiRunsRepo::prune_runs`] when a
/// caller opts in — never applied by default (old run reports are user data).
pub const RUNS_KEEP_DEFAULT: i64 = 0;

impl ApiRunsRepo {
    /// Write `run` as-is (header and whatever steps it carries) — the start
    /// record (no steps yet) and [`Self::recover_interrupted`]'s legacy rows.
    pub async fn save(&self, run: &ApiAutomationRun) -> Result<(), sqlx::Error> {
        let json = serde_json::to_string(run).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
        sqlx::query("INSERT INTO api_automation_runs (id,workspace_id,automation_id,status,created_at,finished_at,record_json) VALUES (?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET status=excluded.status,finished_at=excluded.finished_at,record_json=excluded.record_json")
            .bind(&run.id).bind(&run.workspace_id).bind(&run.automation_id).bind(&run.status)
            .bind(&run.created_at).bind(&run.finished_at).bind(json).execute(&self.0).await?;
        Ok(())
    }

    /// Write the run HEADER only (status, error, snapshot, `passed`, counts):
    /// `record_json` carries no step results and the row is marked as keeping
    /// them in `api_automation_run_steps` (see [`Self::append_step`]). The
    /// in-memory `run` is left untouched.
    pub async fn save_header(&self, run: &ApiAutomationRun) -> Result<(), sqlx::Error> {
        let mut header = run.clone_header();
        header.snapshot = run.snapshot.clone();
        let json = serde_json::to_string(&header).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
        let total = run.report.steps.len() as i64;
        let passed = run.report.steps.iter().filter(|s| s.ok).count() as i64;
        sqlx::query("INSERT INTO api_automation_runs (id,workspace_id,automation_id,status,created_at,finished_at,record_json,steps_total,steps_passed,steps_in_table) VALUES (?,?,?,?,?,?,?,?,?,1) ON CONFLICT(id) DO UPDATE SET status=excluded.status,finished_at=excluded.finished_at,record_json=excluded.record_json,steps_total=excluded.steps_total,steps_passed=excluded.steps_passed,steps_in_table=1")
            .bind(&run.id).bind(&run.workspace_id).bind(&run.automation_id).bind(&run.status)
            .bind(&run.created_at).bind(&run.finished_at).bind(json).bind(total).bind(passed)
            .execute(&self.0).await?;
        Ok(())
    }

    /// Persist ONE completed step (O(1) per step — perf F2) and bump the run's
    /// counters, atomically. Returns the bytes written for the step row (a
    /// measurability hook for the O(n) guard test).
    pub async fn append_step(
        &self,
        run_id: &str,
        idx: usize,
        dataset_row: usize,
        step_result_id: &str,
        result: &ApiRunStepResult,
    ) -> Result<usize, sqlx::Error> {
        let json = serde_json::to_string(result).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
        let bytes = json.len();
        let mut tx = self.0.begin().await?;
        sqlx::query("INSERT INTO api_automation_run_steps (run_id,idx,dataset_row,step_result_id,result_json) VALUES (?,?,?,?,?)")
            .bind(run_id).bind(idx as i64).bind(dataset_row as i64).bind(step_result_id).bind(json)
            .execute(&mut *tx).await?;
        sqlx::query("UPDATE api_automation_runs SET steps_total = steps_total + 1, steps_passed = steps_passed + ?, steps_in_table = 1 WHERE id = ?")
            .bind(i64::from(result.ok)).bind(run_id)
            .execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(bytes)
    }

    /// The whole run (every step).
    pub async fn get(&self, wid: &str, id: &str) -> Result<ApiAutomationRun, sqlx::Error> {
        self.get_from(wid, id, 0).await
    }

    /// Delta read for a running view: the header plus only the step results
    /// from index `after` on, and no `snapshot` — read straight off the step
    /// table's primary key rather than decoding every step and dropping most.
    pub async fn get_after(
        &self,
        wid: &str,
        id: &str,
        after: usize,
    ) -> Result<ApiAutomationRun, sqlx::Error> {
        let mut run = self.get_from(wid, id, after).await?;
        run.snapshot = serde_json::Value::Null;
        Ok(run)
    }

    async fn get_from(
        &self,
        wid: &str,
        id: &str,
        after: usize,
    ) -> Result<ApiAutomationRun, sqlx::Error> {
        let row = sqlx::query(
            "SELECT record_json, steps_in_table FROM api_automation_runs WHERE workspace_id=? AND id=?",
        )
        .bind(wid)
        .bind(id)
        .fetch_one(&self.0)
        .await?;
        let mut run = decode(&row)?;
        if row.try_get::<i64, _>("steps_in_table")? == 0 {
            // Legacy row: the steps are inside record_json.
            skip_steps(&mut run, after);
            return Ok(run);
        }
        let steps = sqlx::query(
            "SELECT dataset_row, step_result_id, result_json FROM api_automation_run_steps \
              WHERE run_id = ? AND idx >= ? ORDER BY idx",
        )
        .bind(id)
        .bind(after as i64)
        .fetch_all(&self.0)
        .await?;
        run.report.steps.clear();
        run.result_rows.clear();
        run.result_ids.clear();
        for s in &steps {
            let result: ApiRunStepResult = serde_json::from_str(s.try_get("result_json")?)
                .map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
            run.report.steps.push(result);
            run.result_rows
                .push(s.try_get::<i64, _>("dataset_row")? as usize);
            run.result_ids.push(s.try_get("step_result_id")?);
        }
        Ok(run)
    }

    /// Newest-first run LIST: headers with `steps_total` / `steps_passed`
    /// and NO step results (perf F2: it returned 50 full records). A legacy
    /// row's counts come from its inline steps, which are then dropped.
    pub async fn list(
        &self,
        wid: &str,
        automation: Option<&str>,
        before: Option<&str>,
    ) -> Result<Vec<ApiAutomationRun>, sqlx::Error> {
        let rows = sqlx::query("SELECT record_json, steps_total, steps_passed, steps_in_table FROM api_automation_runs WHERE workspace_id=? AND (? IS NULL OR automation_id=?) AND (? IS NULL OR id<?) ORDER BY id DESC LIMIT 50")
            .bind(wid).bind(automation).bind(automation).bind(before).bind(before).fetch_all(&self.0).await?;
        rows.iter()
            .map(|row| {
                let mut run = decode(row)?;
                let (total, passed) = if row.try_get::<i64, _>("steps_in_table")? == 0 {
                    let s = &run.report.steps;
                    (s.len(), s.iter().filter(|s| s.ok).count())
                } else {
                    (
                        row.try_get::<i64, _>("steps_total")? as usize,
                        row.try_get::<i64, _>("steps_passed")? as usize,
                    )
                };
                run.report.steps.clear();
                run.result_rows.clear();
                run.result_ids.clear();
                run.snapshot = serde_json::Value::Null;
                run.steps_total = Some(total);
                run.steps_passed = Some(passed);
                Ok(run)
            })
            .collect()
    }

    /// Opt-in retention: keep only the newest `keep` FINISHED runs of
    /// `automation` (`keep <= 0` keeps everything — the default, see
    /// [`RUNS_KEEP_DEFAULT`]). Their step rows go with them.
    pub async fn prune_runs(
        &self,
        wid: &str,
        automation: &str,
        keep: i64,
    ) -> Result<u64, sqlx::Error> {
        if keep <= 0 {
            return Ok(0);
        }
        let mut tx = self.0.begin().await?;
        sqlx::query(
            "DELETE FROM api_automation_run_steps WHERE run_id IN ( \
               SELECT id FROM api_automation_runs WHERE workspace_id = ? AND automation_id = ? \
                  AND status <> 'running' ORDER BY id DESC LIMIT -1 OFFSET ?)",
        )
        .bind(wid)
        .bind(automation)
        .bind(keep)
        .execute(&mut *tx)
        .await?;
        let deleted = sqlx::query(
            "DELETE FROM api_automation_runs WHERE id IN ( \
               SELECT id FROM api_automation_runs WHERE workspace_id = ? AND automation_id = ? \
                  AND status <> 'running' ORDER BY id DESC LIMIT -1 OFFSET ?)",
        )
        .bind(wid)
        .bind(automation)
        .bind(keep)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        tx.commit().await?;
        Ok(deleted)
    }

    pub async fn recover_interrupted(&self) -> Result<(), sqlx::Error> {
        let rows =
            sqlx::query("SELECT record_json FROM api_automation_runs WHERE status='running'")
                .fetch_all(&self.0)
                .await?;
        for row in rows {
            // Header-level edit: a new-style row's record_json has no steps
            // and keeps none; a legacy row keeps its inline steps.
            let mut run = decode(&row)?;
            run.status = "interrupted".into();
            run.finished_at = Some(chrono::Utc::now().to_rfc3339());
            run.error = Some("Daemon stopped during this run; completed steps are retained. Requests are not replayed automatically.".into());
            self.save(&run).await?;
        }
        Ok(())
    }
}

trait CloneHeader {
    fn clone_header(&self) -> ApiAutomationRun;
}
impl CloneHeader for ApiAutomationRun {
    /// A copy without the (potentially large) step vectors or snapshot.
    fn clone_header(&self) -> ApiAutomationRun {
        ApiAutomationRun {
            id: self.id.clone(),
            workspace_id: self.workspace_id.clone(),
            automation_id: self.automation_id.clone(),
            environment_id: self.environment_id.clone(),
            created_by: self.created_by.clone(),
            status: self.status.clone(),
            created_at: self.created_at.clone(),
            finished_at: self.finished_at.clone(),
            stop_on_failure: self.stop_on_failure,
            dataset_rows: self.dataset_rows,
            snapshot: serde_json::Value::Null,
            report: otto_core::api::ApiRunResult {
                automation_id: self.report.automation_id.clone(),
                steps: Vec::new(),
                passed: self.report.passed,
            },
            result_rows: Vec::new(),
            result_ids: Vec::new(),
            error: self.error.clone(),
            steps_total: None,
            steps_passed: None,
        }
    }
}

fn decode(row: &sqlx::sqlite::SqliteRow) -> Result<ApiAutomationRun, sqlx::Error> {
    serde_json::from_str(row.try_get("record_json")?).map_err(|e| sqlx::Error::Decode(Box::new(e)))
}

/// Drop the first `after` step results (the three vectors stay aligned).
fn skip_steps(run: &mut ApiAutomationRun, after: usize) {
    if after == 0 {
        return;
    }
    let n = after.min(run.report.steps.len());
    run.report.steps.drain(..n);
    let n = after.min(run.result_rows.len());
    run.result_rows.drain(..n);
    let n = after.min(run.result_ids.len());
    run.result_ids.drain(..n);
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_core::api::ApiRunResult;
    #[tokio::test]
    async fn reports_are_workspace_scoped_and_restart_retains_completed_steps() {
        let pool = DbPool::connect("sqlite::memory:").await.unwrap();
        sqlx::query("CREATE TABLE workspaces(id TEXT PRIMARY KEY)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO workspaces VALUES ('a'),('b')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::raw_sql(include_str!("../migrations/0128_api_automation_runs.sql"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::raw_sql(include_str!(
            "../migrations/0157_api_automation_run_steps.sql"
        ))
        .execute(&pool)
        .await
        .unwrap();
        let repo = ApiRunsRepo(pool.clone());
        let mut run = ApiAutomationRun {
            id: "run".into(),
            workspace_id: "a".into(),
            automation_id: "auto".into(),
            environment_id: None,
            created_by: "u".into(),
            status: "running".into(),
            created_at: chrono::Utc::now().to_rfc3339(),
            finished_at: None,
            stop_on_failure: false,
            dataset_rows: 2,
            snapshot: serde_json::json!([]),
            report: ApiRunResult {
                automation_id: "auto".into(),
                steps: vec![],
                passed: false,
            },
            result_rows: vec![],
            result_ids: vec![],
            error: None,
            steps_total: None,
            steps_passed: None,
        };
        run.report.steps.push(otto_core::api::ApiRunStepResult {
            request_id: "request".into(),
            name: "Completed request".into(),
            status: Some(200),
            duration_ms: 5,
            ok: true,
            assertions: serde_json::json!([]),
            error: None,
        });
        run.result_rows.push(0);
        repo.save(&run).await.unwrap();
        assert!(repo.get("b", "run").await.is_err());
        assert!(repo.list("b", None, None).await.unwrap().is_empty());
        repo.recover_interrupted().await.unwrap();
        let saved = repo.get("a", "run").await.unwrap();
        assert_eq!(saved.status, "interrupted");
        assert_eq!(saved.report.steps.len(), 1);
        assert_eq!(saved.result_rows, vec![0]);
        assert!(saved.finished_at.is_some());
        assert_eq!(repo.list("a", Some("auto"), None).await.unwrap().len(), 1);
        assert!(repo
            .list("a", Some("different"), None)
            .await
            .unwrap()
            .is_empty());
    }

    async fn pool() -> DbPool {
        let pool = DbPool::connect("sqlite::memory:").await.unwrap();
        sqlx::query("CREATE TABLE workspaces(id TEXT PRIMARY KEY)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO workspaces VALUES ('a')")
            .execute(&pool)
            .await
            .unwrap();
        for sql in [
            include_str!("../migrations/0128_api_automation_runs.sql"),
            include_str!("../migrations/0157_api_automation_run_steps.sql"),
        ] {
            sqlx::raw_sql(sql).execute(&pool).await.unwrap();
        }
        pool
    }

    fn header(id: &str) -> ApiAutomationRun {
        ApiAutomationRun {
            id: id.into(),
            workspace_id: "a".into(),
            automation_id: "auto".into(),
            environment_id: None,
            created_by: "u".into(),
            status: "running".into(),
            created_at: chrono::Utc::now().to_rfc3339(),
            finished_at: None,
            stop_on_failure: false,
            dataset_rows: 1,
            snapshot: serde_json::json!([{"request_id": "r"}]),
            report: ApiRunResult {
                automation_id: "auto".into(),
                steps: vec![],
                passed: false,
            },
            result_rows: vec![],
            result_ids: vec![],
            error: None,
            steps_total: None,
            steps_passed: None,
        }
    }

    fn step(i: usize) -> otto_core::api::ApiRunStepResult {
        otto_core::api::ApiRunStepResult {
            request_id: "r".into(),
            name: format!("step {i}"),
            status: Some(200),
            duration_ms: 1,
            ok: !i.is_multiple_of(3),
            assertions: serde_json::json!([{"ok": true, "detail": "x".repeat(800)}]),
            error: None,
        }
    }

    /// Perf guard (F2/F11): a 1000-step run writes O(n) bytes — each step row
    /// is written once and the header never grows with the steps — and the
    /// delta read / list return only what they promise.
    #[tokio::test]
    async fn thousand_step_run_writes_linear_bytes_and_reads_deltas() {
        let repo = ApiRunsRepo(pool().await);
        let mut run = header("run1");
        repo.save_header(&run).await.unwrap();
        let mut step_bytes = 0usize;
        let mut max_step = 0usize;
        for i in 0..1000 {
            let s = step(i);
            let n = repo
                .append_step(&run.id, i, i % 2, &format!("s{i}"), &s)
                .await
                .unwrap();
            step_bytes += n;
            max_step = max_step.max(n);
            run.report.steps.push(s);
            run.result_rows.push(i % 2);
            run.result_ids.push(format!("s{i}"));
        }
        assert!(step_bytes <= 1000 * max_step, "each step written once");
        let header_len: i64 = sqlx::query_scalar(
            "SELECT length(record_json) FROM api_automation_runs WHERE id='run1'",
        )
        .fetch_one(&repo.0)
        .await
        .unwrap();
        assert!(header_len < 2048, "header carries no steps: {header_len}");
        run.status = "failed".into();
        repo.save_header(&run).await.unwrap();

        let full = repo.get("a", "run1").await.unwrap();
        assert_eq!(full.report.steps.len(), 1000);
        assert_eq!(full.result_ids[999], "s999");
        assert_eq!(full.result_rows[3], 1);
        assert_eq!(full.status, "failed");
        assert!(!full.snapshot.is_null());

        let delta = repo.get_after("a", "run1", 998).await.unwrap();
        assert_eq!(delta.report.steps.len(), 2);
        assert_eq!(delta.report.steps[0].name, "step 998");
        assert_eq!(
            delta.result_ids,
            vec!["s998".to_string(), "s999".to_string()]
        );
        assert!(delta.snapshot.is_null());

        let listed = repo.list("a", None, None).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert!(listed[0].report.steps.is_empty());
        assert_eq!(listed[0].steps_total, Some(1000));
        assert_eq!(listed[0].steps_passed, Some(666));
    }

    /// A row written before 0166 (steps inline in record_json) still reads
    /// whole, deltas, and lists with counts.
    #[tokio::test]
    async fn legacy_inline_runs_stay_readable() {
        let repo = ApiRunsRepo(pool().await);
        let mut run = header("old");
        for i in 0..4 {
            run.report.steps.push(step(i));
            run.result_rows.push(0);
            run.result_ids.push(format!("s{i}"));
        }
        repo.save(&run).await.unwrap();
        assert_eq!(repo.get("a", "old").await.unwrap().report.steps.len(), 4);
        let delta = repo.get_after("a", "old", 3).await.unwrap();
        assert_eq!(delta.report.steps.len(), 1);
        assert_eq!(delta.result_ids, vec!["s3".to_string()]);
        let listed = repo.list("a", None, None).await.unwrap();
        assert_eq!(listed[0].steps_total, Some(4));
        assert_eq!(listed[0].steps_passed, Some(2));
        assert!(listed[0].report.steps.is_empty());
    }

    #[tokio::test]
    async fn prune_runs_is_opt_in_and_keeps_the_newest_finished() {
        let repo = ApiRunsRepo(pool().await);
        for i in 0..5 {
            let mut run = header(&format!("r{i}"));
            run.status = if i == 0 { "running" } else { "passed" }.into();
            repo.save_header(&run).await.unwrap();
            repo.append_step(&run.id, 0, 0, "s", &step(1))
                .await
                .unwrap();
        }
        assert_eq!(
            repo.prune_runs("a", "auto", RUNS_KEEP_DEFAULT)
                .await
                .unwrap(),
            0
        );
        assert_eq!(repo.prune_runs("a", "auto", 2).await.unwrap(), 2);
        let mut left: Vec<String> = repo
            .list("a", None, None)
            .await
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect();
        left.sort();
        assert_eq!(left, vec!["r0", "r3", "r4"], "running run never pruned");
        let orphan_steps: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM api_automation_run_steps WHERE run_id IN ('r1','r2')",
        )
        .fetch_one(&repo.0)
        .await
        .unwrap();
        assert_eq!(orphan_steps, 0);
    }
}
