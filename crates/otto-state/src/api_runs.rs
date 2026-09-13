//! Durable API automation reports. Callers must redact before saving.
use otto_core::api::ApiAutomationRun;
use sqlx::{Row, SqlitePool};

#[derive(Clone)]
pub struct ApiRunsRepo(pub SqlitePool);
impl ApiRunsRepo {
    pub async fn save(&self, run: &ApiAutomationRun) -> Result<(), sqlx::Error> {
        let json = serde_json::to_string(run).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
        sqlx::query("INSERT INTO api_automation_runs (id,workspace_id,automation_id,status,created_at,finished_at,record_json) VALUES (?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET status=excluded.status,finished_at=excluded.finished_at,record_json=excluded.record_json")
            .bind(&run.id).bind(&run.workspace_id).bind(&run.automation_id).bind(&run.status)
            .bind(&run.created_at).bind(&run.finished_at).bind(json).execute(&self.0).await?;
        Ok(())
    }
    pub async fn get(&self, wid: &str, id: &str) -> Result<ApiAutomationRun, sqlx::Error> {
        let row = sqlx::query(
            "SELECT record_json FROM api_automation_runs WHERE workspace_id=? AND id=?",
        )
        .bind(wid)
        .bind(id)
        .fetch_one(&self.0)
        .await?;
        decode(&row)
    }
    pub async fn list(
        &self,
        wid: &str,
        automation: Option<&str>,
        before: Option<&str>,
    ) -> Result<Vec<ApiAutomationRun>, sqlx::Error> {
        let rows = sqlx::query("SELECT record_json FROM api_automation_runs WHERE workspace_id=? AND (? IS NULL OR automation_id=?) AND (? IS NULL OR id<?) ORDER BY id DESC LIMIT 50")
            .bind(wid).bind(automation).bind(automation).bind(before).bind(before).fetch_all(&self.0).await?;
        rows.iter().map(decode).collect()
    }
    pub async fn recover_interrupted(&self) -> Result<(), sqlx::Error> {
        let rows =
            sqlx::query("SELECT record_json FROM api_automation_runs WHERE status='running'")
                .fetch_all(&self.0)
                .await?;
        for row in rows {
            let mut run = decode(&row)?;
            run.status = "interrupted".into();
            run.finished_at = Some(chrono::Utc::now().to_rfc3339());
            run.error = Some("Daemon stopped during this run; completed steps are retained. Requests are not replayed automatically.".into());
            self.save(&run).await?;
        }
        Ok(())
    }
}
fn decode(row: &sqlx::sqlite::SqliteRow) -> Result<ApiAutomationRun, sqlx::Error> {
    serde_json::from_str(row.try_get("record_json")?).map_err(|e| sqlx::Error::Decode(Box::new(e)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_core::api::ApiRunResult;
    #[tokio::test]
    async fn reports_are_workspace_scoped_and_restart_retains_completed_steps() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
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
}
