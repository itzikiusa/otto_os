//! SQLite rows for audio jobs (`media_audio_jobs`, migration 0225).

use chrono::{DateTime, Utc};
use otto_core::{Error, Result};
use sqlx::Row;

use super::AudioJob;
use crate::store::MediaStore;

fn dberr(op: &'static str) -> impl Fn(sqlx::Error) -> Error {
    move |e| Error::Internal(format!("{op}: {e}"))
}

fn ts(s: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .map_err(|e| Error::Internal(format!("audio job bad timestamp '{s}': {e}")))
}

impl MediaStore {
    pub async fn insert_audio_job(&self, j: &AudioJob) -> Result<()> {
        sqlx::query(
            "INSERT INTO media_audio_jobs
             (id, workspace_id, kind, title, prompt, seconds, looped, bpm, seed, story_id,
              artifact_id, status, progress_pct, stage, clip_id, error, created_by,
              created_at, updated_at, finished_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&j.id)
        .bind(&j.workspace_id)
        .bind(&j.kind)
        .bind(&j.title)
        .bind(&j.prompt)
        .bind(j.seconds)
        .bind(j.looped)
        .bind(j.bpm)
        .bind(j.seed)
        .bind(&j.story_id)
        .bind(&j.artifact_id)
        .bind(&j.status)
        .bind(j.progress_pct)
        .bind(&j.stage)
        .bind(&j.clip_id)
        .bind(&j.error)
        .bind(&j.created_by)
        .bind(j.created_at.to_rfc3339())
        .bind(j.updated_at.to_rfc3339())
        .bind(j.finished_at.map(|t| t.to_rfc3339()))
        .execute(self.pool())
        .await
        .map_err(dberr("media.audio_jobs.insert"))?;
        Ok(())
    }

    pub async fn require_audio_job(&self, id: &str) -> Result<AudioJob> {
        let row = sqlx::query("SELECT * FROM media_audio_jobs WHERE id = ?")
            .bind(id)
            .fetch_optional(self.pool())
            .await
            .map_err(dberr("media.audio_jobs.get"))?
            .ok_or_else(|| Error::NotFound(format!("audio job {id}")))?;
        row_audio_job(&row)
    }

    pub async fn list_audio_jobs(
        &self,
        workspace_id: &str,
        kind: Option<&str>,
        limit: i64,
    ) -> Result<Vec<AudioJob>> {
        let rows = sqlx::query(
            "SELECT * FROM media_audio_jobs
             WHERE workspace_id = ? AND (? IS NULL OR kind = ?)
             ORDER BY created_at DESC, id DESC LIMIT ?",
        )
        .bind(workspace_id)
        .bind(kind)
        .bind(kind)
        .bind(limit.clamp(1, 200))
        .fetch_all(self.pool())
        .await
        .map_err(dberr("media.audio_jobs.list"))?;
        rows.iter().map(row_audio_job).collect()
    }

    /// A live job's progress (never backwards, below 100 until it is stored).
    pub async fn update_audio_job_progress(&self, id: &str, pct: i64, stage: &str) -> Result<()> {
        sqlx::query(
            "UPDATE media_audio_jobs SET status = 'running',
             progress_pct = MAX(progress_pct, ?), stage = ?, updated_at = ?
             WHERE id = ? AND status IN ('queued', 'running')",
        )
        .bind(pct.clamp(0, 99))
        .bind(stage)
        .bind(Utc::now().to_rfc3339())
        .bind(id)
        .execute(self.pool())
        .await
        .map_err(dberr("media.audio_jobs.progress"))?;
        Ok(())
    }

    /// End a live job (a finished one is left as it is).
    pub async fn finish_audio_job(
        &self,
        id: &str,
        status: &str,
        stage: &str,
        clip_id: Option<&str>,
        error: Option<&str>,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "UPDATE media_audio_jobs SET status = ?, stage = ?, clip_id = ?, error = ?,
             progress_pct = CASE WHEN ? = 'succeeded' THEN 100 ELSE progress_pct END,
             updated_at = ?, finished_at = ?
             WHERE id = ? AND status IN ('queued', 'running')",
        )
        .bind(status)
        .bind(stage)
        .bind(clip_id)
        .bind(error)
        .bind(status)
        .bind(&now)
        .bind(&now)
        .bind(id)
        .execute(self.pool())
        .await
        .map_err(dberr("media.audio_jobs.finish"))?;
        Ok(())
    }

    /// Boot: the runners of live jobs died with the daemon.
    pub async fn fail_interrupted_audio_jobs(&self) -> Result<u64> {
        let now = Utc::now().to_rfc3339();
        let r = sqlx::query(
            "UPDATE media_audio_jobs SET status = 'failed', stage = 'Failed',
             error = 'Otto restarted while this was being made.', updated_at = ?, finished_at = ?
             WHERE status IN ('queued', 'running')",
        )
        .bind(&now)
        .bind(&now)
        .execute(self.pool())
        .await
        .map_err(dberr("media.audio_jobs.fail_interrupted"))?;
        Ok(r.rows_affected())
    }
}

fn row_audio_job(r: &sqlx::sqlite::SqliteRow) -> Result<AudioJob> {
    let finished: Option<String> = r.get("finished_at");
    Ok(AudioJob {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        kind: r.get("kind"),
        title: r.get("title"),
        prompt: r.get("prompt"),
        seconds: r.get("seconds"),
        looped: r.get::<i64, _>("looped") != 0,
        bpm: r.get("bpm"),
        seed: r.get("seed"),
        story_id: r.get("story_id"),
        artifact_id: r.get("artifact_id"),
        status: r.get("status"),
        progress_pct: r.get("progress_pct"),
        stage: r.get("stage"),
        clip_id: r.get("clip_id"),
        error: r.get("error"),
        created_by: r.get("created_by"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
        updated_at: ts(&r.get::<String, _>("updated_at"))?,
        finished_at: finished.as_deref().map(ts).transpose()?,
        clip: None,
    })
}
