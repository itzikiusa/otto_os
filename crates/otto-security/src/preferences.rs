use std::collections::BTreeMap;

use otto_core::{Error, Result};
use otto_state::DbPool;

use crate::store::{db_error, decode};
use crate::{Finding, FindingAnnotations, ScanStatus, ScanSummary};

pub async fn baseline(pool: &DbPool, project: &str) -> Result<Option<String>> {
    sqlx::query_scalar("SELECT scan_id FROM ide_security_baselines WHERE project_id = ?")
        .bind(project)
        .fetch_optional(pool)
        .await
        .map_err(db_error)
}

pub async fn set_baseline(pool: &DbPool, project: &str, id: Option<&str>) -> Result<()> {
    if let Some(id) = id {
        let inserted = sqlx::query(
            "INSERT INTO ide_security_baselines (project_id, scan_id)
             SELECT project_id, id FROM ide_security_scans WHERE project_id = ? AND id = ? AND status = 'completed'
             ON CONFLICT(project_id) DO UPDATE SET scan_id = excluded.scan_id",
        ).bind(project).bind(id).execute(pool).await.map_err(db_error)?;
        if inserted.rows_affected() == 0 {
            return Err(Error::NotFound("completed project scan".into()));
        }
    } else {
        sqlx::query("DELETE FROM ide_security_baselines WHERE project_id = ?")
            .bind(project)
            .execute(pool)
            .await
            .map_err(db_error)?;
    }
    Ok(())
}

pub async fn set_suppression(
    pool: &DbPool,
    project: &str,
    fingerprint: &str,
    reason: Option<&str>,
) -> Result<()> {
    if fingerprint.len() != 64 || !fingerprint.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Error::Invalid("Invalid finding fingerprint.".into()));
    }
    if let Some(reason) = reason {
        let reason = reason.trim();
        if reason.is_empty() || reason.len() > 800 {
            return Err(Error::Invalid(
                "Enter a suppression reason of 1–800 bytes.".into(),
            ));
        }
        sqlx::query(
            "INSERT INTO ide_security_suppressions (project_id, fingerprint, reason) VALUES (?, ?, ?)
             ON CONFLICT(project_id, fingerprint) DO UPDATE SET reason = excluded.reason",
        ).bind(project).bind(fingerprint).bind(reason).execute(pool).await.map_err(db_error)?;
    } else {
        sqlx::query(
            "DELETE FROM ide_security_suppressions WHERE project_id = ? AND fingerprint = ?",
        )
        .bind(project)
        .bind(fingerprint)
        .execute(pool)
        .await
        .map_err(db_error)?;
    }
    Ok(())
}

pub(crate) async fn annotations(
    pool: &DbPool,
    project: &str,
    scan: &ScanSummary,
) -> Result<FindingAnnotations> {
    if scan.status != ScanStatus::Completed {
        return Ok(FindingAnnotations::default());
    }
    let previous: Option<String> = sqlx::query_scalar(
        "SELECT findings FROM ide_security_scans
         WHERE project_id = ? AND status = 'completed' AND (created_at, id) < (?, ?)
         ORDER BY created_at DESC, id DESC LIMIT 1",
    )
    .bind(project)
    .bind(&scan.created_at)
    .bind(&scan.id)
    .fetch_optional(pool)
    .await
    .map_err(db_error)?;
    let baseline: Option<String> = sqlx::query_scalar(
        "SELECT s.findings FROM ide_security_baselines b JOIN ide_security_scans s ON s.id = b.scan_id
         WHERE b.project_id = ? AND s.project_id = b.project_id",
    ).bind(project).fetch_optional(pool).await.map_err(db_error)?;
    let suppressions = sqlx::query_as::<_, (String, String)>(
        "SELECT fingerprint, reason FROM ide_security_suppressions WHERE project_id = ?",
    )
    .bind(project)
    .fetch_all(pool)
    .await
    .map_err(db_error)?;
    Ok(FindingAnnotations {
        existing: fingerprints(previous)?,
        baseline: fingerprints(baseline)?,
        suppressions: suppressions.into_iter().collect::<BTreeMap<_, _>>(),
    })
}

fn fingerprints(json: Option<String>) -> Result<Vec<String>> {
    Ok(json
        .as_deref()
        .map(decode::<Vec<Finding>>)
        .transpose()?
        .unwrap_or_default()
        .into_iter()
        .map(|finding| finding.fingerprint)
        .collect())
}
