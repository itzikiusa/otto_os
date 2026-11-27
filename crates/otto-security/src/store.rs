use chrono::Utc;
use otto_core::{Error, Result};
use otto_state::DbPool;
use serde::{de::DeserializeOwned, Serialize};

use crate::{ProjectScans, ScanDetail, ScanSettings, ScanStatus, ScanSummary};

pub(crate) fn now() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub(crate) fn encode<T: Serialize>(value: &T) -> Result<String> {
    serde_json::to_string(value).map_err(|e| Error::Internal(e.to_string()))
}

pub(crate) fn decode<T: DeserializeOwned>(json: &str) -> Result<T> {
    serde_json::from_str(json).map_err(|e| Error::Internal(e.to_string()))
}

pub(crate) fn db_error(error: sqlx::Error) -> Error {
    Error::Internal(format!("security scan storage: {error}"))
}

#[derive(sqlx::FromRow)]
struct Row {
    id: String,
    status: String,
    created_at: String,
    started_at: Option<String>,
    finished_at: Option<String>,
    engine: Option<String>,
    metrics: Option<String>,
    error: Option<String>,
    count: u32,
    engines: String,
}

impl TryFrom<Row> for ScanSummary {
    type Error = Error;

    fn try_from(row: Row) -> Result<Self> {
        let status = match row.status.as_str() {
            "queued" => ScanStatus::Queued,
            "running" => ScanStatus::Running,
            "completed" => ScanStatus::Completed,
            "failed" => ScanStatus::Failed,
            "cancelled" => ScanStatus::Cancelled,
            _ => return Err(Error::Internal("unknown security scan status".into())),
        };
        Ok(Self {
            id: row.id,
            status,
            created_at: row.created_at,
            started_at: row.started_at,
            finished_at: row.finished_at,
            engine: row.engine.as_deref().map(decode).transpose()?,
            metrics: row.metrics.as_deref().map(decode).transpose()?,
            error: row.error,
            count: row.count,
            engines: decode(&row.engines)?,
        })
    }
}

macro_rules! summary_query {
    ($prefix:literal, $suffix:literal) => {
        concat!(
            $prefix,
            "id, status, created_at, started_at, finished_at, engine, metrics, error, count, engines",
            $suffix,
        )
    };
}

pub(crate) async fn recover(pool: &DbPool) -> Result<()> {
    sqlx::query(
        "UPDATE ide_security_scans SET status = 'failed', finished_at = ?, error = ?          WHERE status IN ('queued', 'running')",
    )
    .bind(now())
    .bind("Otto restarted before the scan finished. Start a new scan to retry.")
    .execute(pool)
    .await
    .map_err(db_error)?;
    Ok(())
}

pub(crate) async fn settings(pool: &DbPool, project: &str) -> Result<ScanSettings> {
    sqlx::query_scalar::<_, String>(
        "SELECT settings FROM ide_security_settings WHERE project_id = ?",
    )
    .bind(project)
    .fetch_optional(pool)
    .await
    .map_err(db_error)?
    .as_deref()
    .map(decode)
    .transpose()
    .map(Option::unwrap_or_default)
}

pub(crate) async fn save_settings(
    pool: &DbPool,
    project: &str,
    settings: &ScanSettings,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO ide_security_settings (project_id, settings) VALUES (?, ?)          ON CONFLICT(project_id) DO UPDATE SET settings = excluded.settings",
    )
    .bind(project)
    .bind(encode(settings)?)
    .execute(pool)
    .await
    .map_err(db_error)?;
    Ok(())
}

pub(crate) async fn list(pool: &DbPool, project: &str) -> Result<ProjectScans> {
    let query = summary_query!(
        "SELECT ",
        " FROM ide_security_scans WHERE project_id = ? ORDER BY created_at DESC, id DESC LIMIT 30"
    );
    let scans = sqlx::query_as::<_, Row>(query)
        .bind(project)
        .fetch_all(pool)
        .await
        .map_err(db_error)?
        .into_iter()
        .map(ScanSummary::try_from)
        .collect::<Result<_>>()?;
    Ok(ProjectScans {
        settings: settings(pool, project).await?,
        engine: None,
        scans,
        opengrep: None,
        schedule: crate::schedule::get(pool, project).await?,
        baseline: crate::preferences::baseline(pool, project).await?,
    })
}

pub(crate) async fn detail(pool: &DbPool, project: &str, id: &str) -> Result<ScanDetail> {
    #[derive(sqlx::FromRow)]
    struct Detail {
        #[sqlx(flatten)]
        row: Row,
        findings: String,
    }
    let query = summary_query!(
        "SELECT ",
        ", findings FROM ide_security_scans WHERE project_id = ? AND id = ?"
    );
    let row = sqlx::query_as::<_, Detail>(query)
        .bind(project)
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(db_error)?
        .ok_or_else(|| Error::NotFound("security scan".into()))?;
    let scan: ScanSummary = row.row.try_into()?;
    let annotations = crate::preferences::annotations(pool, project, &scan).await?;
    Ok(ScanDetail {
        scan,
        findings: decode(&row.findings)?,
        annotations,
    })
}

pub(crate) async fn insert(pool: &DbPool, project: &str) -> Result<ScanSummary> {
    let id = otto_core::new_id().to_string();
    let query = summary_query!(
        "INSERT INTO ide_security_scans (id, project_id, status, created_at) VALUES (?, ?, 'queued', ?) RETURNING ",
        ""
    );
    sqlx::query_as::<_, Row>(query)
        .bind(id)
        .bind(project)
        .bind(now())
        .fetch_one(pool)
        .await
        .map_err(|e| {
            if e.as_database_error()
                .is_some_and(|e| e.is_unique_violation())
            {
                Error::Conflict("This project already has a scan queued or running.".into())
            } else {
                db_error(e)
            }
        })?
        .try_into()
}

pub(crate) async fn start(pool: &DbPool, id: &str) -> Result<bool> {
    let updated = sqlx::query(
        "UPDATE ide_security_scans SET status = 'running', started_at = ?          WHERE id = ? AND status = 'queued'",
    )
    .bind(now())
    .bind(id)
    .execute(pool)
    .await
    .map_err(db_error)?;
    Ok(updated.rows_affected() == 1)
}

pub(crate) async fn finish(
    pool: &DbPool,
    id: &str,
    result: Result<(Vec<crate::EngineRun>, Vec<crate::Finding>)>,
) -> Result<()> {
    let (status, engine, metrics, error, count, findings, engines) = match result {
        Ok((engines, findings)) => {
            let primary = engines
                .first()
                .map(|run| encode(&run.version))
                .transpose()?;
            let mut metrics = crate::ScanMetrics::default();
            for run in &engines {
                metrics.wall_ms += run.metrics.wall_ms;
                metrics.peak_cpu_percent =
                    metrics.peak_cpu_percent.max(run.metrics.peak_cpu_percent);
                metrics.peak_rss_bytes = metrics.peak_rss_bytes.max(run.metrics.peak_rss_bytes);
            }
            (
                "completed",
                primary,
                Some(encode(&metrics)?),
                None,
                findings.len(),
                encode(&findings)?,
                encode(&engines)?,
            )
        }
        Err(error) => (
            "failed",
            None,
            None,
            Some(error.public_message()),
            0,
            "[]".into(),
            "[]".into(),
        ),
    };
    sqlx::query(
        "UPDATE ide_security_scans          SET status = ?, finished_at = ?, engine = ?, metrics = ?, error = ?, count = ?, findings = ?, engines = ?          WHERE id = ? AND status IN ('queued', 'running')",
    )
    .bind(status)
    .bind(now())
    .bind(engine)
    .bind(metrics)
    .bind(error)
    .bind(count as u32)
    .bind(findings)
    .bind(engines)
    .bind(id)
    .execute(pool)
    .await
    .map_err(db_error)?;
    Ok(())
}

pub(crate) async fn cancel(pool: &DbPool, project: &str, id: &str) -> Result<ScanSummary> {
    sqlx::query(
        "UPDATE ide_security_scans SET status = 'cancelled', finished_at = ?          WHERE id = ? AND project_id = ? AND status IN ('queued', 'running')",
    )
    .bind(now())
    .bind(id)
    .bind(project)
    .execute(pool)
    .await
    .map_err(db_error)?;
    let query = summary_query!(
        "SELECT ",
        " FROM ide_security_scans WHERE project_id = ? AND id = ?"
    );
    sqlx::query_as::<_, Row>(query)
        .bind(project)
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(db_error)?
        .ok_or_else(|| Error::NotFound("security scan".into()))?
        .try_into()
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn database() -> DbPool {
        let pool = otto_state::db::test_pool().await;
        sqlx::query("INSERT INTO ide_named_projects (id) VALUES ('project'), ('other')")
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn isolates_projects_and_only_allows_one_queued_or_running_scan_per_project() {
        let pool = database().await;
        let scan = insert(&pool, "project").await.unwrap();
        assert!(matches!(
            insert(&pool, "project").await,
            Err(Error::Conflict(_))
        ));
        assert!(matches!(
            detail(&pool, "other", &scan.id).await,
            Err(Error::NotFound(_))
        ));
        assert!(matches!(
            cancel(&pool, "other", &scan.id).await,
            Err(Error::NotFound(_))
        ));
        assert_eq!(
            detail(&pool, "project", &scan.id)
                .await
                .unwrap()
                .scan
                .status,
            ScanStatus::Queued
        );
        assert!(start(&pool, &scan.id).await.unwrap());
        assert_eq!(
            cancel(&pool, "project", &scan.id).await.unwrap().status,
            ScanStatus::Cancelled
        );
        finish(&pool, &scan.id, Err(Error::Internal("late worker".into())))
            .await
            .unwrap();
        assert_eq!(
            detail(&pool, "project", &scan.id)
                .await
                .unwrap()
                .scan
                .status,
            ScanStatus::Cancelled
        );
        assert!(insert(&pool, "project").await.is_ok());
    }

    #[tokio::test]
    async fn recovery_ends_orphaned_scans_and_preserves_history() {
        let pool = database().await;
        let interrupted = insert(&pool, "project").await.unwrap();
        recover(&pool).await.unwrap();
        let status = list(&pool, "project").await.unwrap();
        assert_eq!(status.scans.len(), 1);
        assert_eq!(status.scans[0].id, interrupted.id);
        assert_eq!(status.scans[0].status, ScanStatus::Failed);
        assert!(insert(&pool, "project").await.is_ok());
    }

    #[tokio::test]
    async fn a_busy_mac_holds_the_global_queue_until_that_scan_is_cancelled() {
        use std::sync::Arc;
        use std::time::Duration;

        let pool = database().await;
        let temp = tempfile::tempdir().unwrap();
        let scans = Arc::new(crate::SecurityScans::new(temp.path()));
        let waiting = Arc::new(tokio::sync::Notify::new());
        let busy = waiting.clone();
        let first = scans
            .enqueue(
                pool.clone(),
                "project".into(),
                || async { panic!("a busy scan must not resolve its roots") },
                move || {
                    busy.notify_one();
                    async { true }
                },
            )
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(3), waiting.notified())
            .await
            .unwrap();

        let second = scans
            .enqueue(
                pool.clone(),
                "other".into(),
                || async { Err(Error::Invalid("Project folder was removed.".into())) },
                || async { false },
            )
            .await
            .unwrap();
        assert_eq!(
            detail(&pool, "other", &second.id)
                .await
                .unwrap()
                .scan
                .status,
            ScanStatus::Queued
        );
        scans.cancel(&pool, "project", &first.id).await.unwrap();

        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let scan = detail(&pool, "other", &second.id).await.unwrap().scan;
                if scan.status == ScanStatus::Failed {
                    assert_eq!(
                        scan.error.as_deref(),
                        Some("invalid: Project folder was removed.")
                    );
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
    }
}
