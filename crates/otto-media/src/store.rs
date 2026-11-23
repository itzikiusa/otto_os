//! SQLite persistence for 3D generation providers, 3D jobs, TTS providers, and TTS clips.

use chrono::{DateTime, Utc};
use otto_core::{Error, Id, Result};
use otto_state::DbPool;
use serde_json::{json, Value};
use sqlx::Row;

use crate::types::{
    Media3dJob, Media3dProviderRecord, TtsClip, TtsClipRecord, TtsProviderRecord,
    TtsVisemeTimeline, BUILTIN_HUNYUAN_ID, BUILTIN_MACOS_SAY_ID, BUILTIN_OTTO_FORGE_ID,
    BUILTIN_TRELLIS2_ID, DEFAULT_3D_MAX_OUTPUT_BYTES, DEFAULT_3D_TIMEOUT_SECS,
    HUNYUAN3D_LICENSE_WARNING, OTTO_FORGE_TIMEOUT_SECS,
};

#[derive(Clone)]
pub struct MediaStore {
    pool: DbPool,
}

fn dberr(op: &'static str) -> impl Fn(sqlx::Error) -> Error {
    move |e| Error::Internal(format!("{op}: {e}"))
}

fn stamp(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339()
}

fn parse_ts(s: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .map_err(|e| Error::Internal(format!("media bad timestamp '{s}': {e}")))
}

fn opt_ts(s: Option<String>) -> Result<Option<DateTime<Utc>>> {
    s.filter(|v| !v.is_empty())
        .map(|v| parse_ts(&v))
        .transpose()
}

impl MediaStore {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &DbPool {
        &self.pool
    }

    /// Idempotently seed built-in 3D providers (`otto-forge`, `trellis-2`,
    /// `hunyuan3d-omni` — with `hunyuan3d-omni` NOT default and carrying its
    /// territory license warning) and built-in TTS provider (`macos-say`).
    /// Never overwrites user edits.
    pub async fn ensure_builtins(&self) -> Result<()> {
        let now = stamp(Utc::now());

        let forge_args = json!([
            "{stage}",
            "--mesh",
            "{mesh}",
            "--image",
            "{image}",
            "--prompt",
            "{prompt}",
            "--output",
            "{output}",
            "--poly-budget",
            "{poly_budget}",
            "--seed",
            "{seed}",
            "--clips",
            "{clips}"
        ])
        .to_string();

        sqlx::query(
            "INSERT OR IGNORE INTO media_3d_providers
             (id, name, kind, transport, builtin, enabled, license_warning, endpoint_url,
              cli_binary, cli_args_json, cli_workdir, auth_header, secret_key_ref,
              allow_local_network, timeout_secs, max_output_bytes, default_params_json,
              created_at, updated_at)
             VALUES (?, ?, 'custom_cli', 'local_cli', 1, 1, NULL, NULL, NULL, ?, NULL,
                     NULL, NULL, 0, ?, ?, '{}', ?, ?)",
        )
        .bind(BUILTIN_OTTO_FORGE_ID)
        .bind("Otto Forge (local)")
        .bind(&forge_args)
        .bind(OTTO_FORGE_TIMEOUT_SECS)
        .bind(DEFAULT_3D_MAX_OUTPUT_BYTES)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("media.seed.otto_forge"))?;

        let trellis_defaults = json!({
            "resolution": 1024,
            "steps": 12,
            "guidance_scale": 7.5,
            "decimation_target": 100000,
            "texture_size": 2048,
            "remesh": true,
            "pbr": true
        })
        .to_string();

        sqlx::query(
            "INSERT OR IGNORE INTO media_3d_providers
             (id, name, kind, transport, builtin, enabled, license_warning, endpoint_url,
              cli_binary, cli_args_json, cli_workdir, auth_header, secret_key_ref,
              allow_local_network, timeout_secs, max_output_bytes, default_params_json,
              created_at, updated_at)
             VALUES (?, ?, 'trellis_2', 'http', 1, 1, NULL, NULL, NULL, '[]', NULL,
                     'Authorization', NULL, 1, ?, ?, ?, ?, ?)",
        )
        .bind(BUILTIN_TRELLIS2_ID)
        .bind("Microsoft TRELLIS.2")
        .bind(DEFAULT_3D_TIMEOUT_SECS)
        .bind(DEFAULT_3D_MAX_OUTPUT_BYTES)
        .bind(&trellis_defaults)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("media.seed.trellis2"))?;

        let hunyuan_defaults = json!({
            "control_type": "none",
            "use_ema": true,
            "flashvdm": true,
            "steps": 30,
            "guidance_scale": 5.5
        })
        .to_string();

        sqlx::query(
            "INSERT OR IGNORE INTO media_3d_providers
             (id, name, kind, transport, builtin, enabled, license_warning, endpoint_url,
              cli_binary, cli_args_json, cli_workdir, auth_header, secret_key_ref,
              allow_local_network, timeout_secs, max_output_bytes, default_params_json,
              created_at, updated_at)
             VALUES (?, ?, 'hunyuan3d_omni', 'http', 1, 1, ?, NULL, NULL, '[]', NULL,
                     'Authorization', NULL, 1, ?, ?, ?, ?, ?)",
        )
        .bind(BUILTIN_HUNYUAN_ID)
        .bind("Tencent Hunyuan3D-Omni")
        .bind(HUNYUAN3D_LICENSE_WARNING)
        .bind(DEFAULT_3D_TIMEOUT_SECS)
        .bind(DEFAULT_3D_MAX_OUTPUT_BYTES)
        .bind(&hunyuan_defaults)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("media.seed.hunyuan"))?;

        sqlx::query(
            "INSERT OR IGNORE INTO media_tts_providers
             (id, name, kind, builtin, enabled, endpoint_url, default_model, default_voice,
              auth_header, secret_key_ref, allow_local_network, created_at, updated_at)
             VALUES (?, ?, 'macos_say', 1, 1, NULL, NULL, 'Samantha', NULL, NULL, 0, ?, ?)",
        )
        .bind(BUILTIN_MACOS_SAY_ID)
        .bind("macOS Built-in Voices")
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("media.seed.macos_say"))?;

        Ok(())
    }

    // ── 3D Providers ────────────────────────────────────────────────────────

    /// Ordered so `otto-forge` and `trellis-2` precede `hunyuan3d-omni`
    /// (`hunyuan3d-omni` is never the implicit default due to territory restrictions).
    pub async fn list_3d_providers(&self) -> Result<Vec<Media3dProviderRecord>> {
        self.ensure_builtins().await?;
        let rows = sqlx::query(
            "SELECT * FROM media_3d_providers
             ORDER BY CASE id
                 WHEN 'otto-forge' THEN 0
                 WHEN 'trellis-2' THEN 1
                 WHEN 'hunyuan3d-omni' THEN 3
                 ELSE 2
             END ASC, created_at ASC, id ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("media.3d_providers.list"))?;
        rows.iter().map(row_3d_provider).collect()
    }

    pub async fn get_3d_provider(&self, id: &str) -> Result<Option<Media3dProviderRecord>> {
        self.ensure_builtins().await?;
        let row = sqlx::query("SELECT * FROM media_3d_providers WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("media.3d_providers.get"))?;
        row.as_ref().map(row_3d_provider).transpose()
    }

    pub async fn require_3d_provider(&self, id: &str) -> Result<Media3dProviderRecord> {
        self.get_3d_provider(id)
            .await?
            .ok_or_else(|| Error::NotFound(format!("3D provider {id}")))
    }

    pub async fn insert_3d_provider(
        &self,
        rec: &Media3dProviderRecord,
    ) -> Result<Media3dProviderRecord> {
        let cli_args_json = serde_json::to_string(&rec.cli_args_template)
            .map_err(|e| Error::Internal(format!("serialize cli_args: {e}")))?;
        let default_params_json = serde_json::to_string(&rec.default_params)
            .map_err(|e| Error::Internal(format!("serialize default_params: {e}")))?;
        sqlx::query(
            "INSERT INTO media_3d_providers
             (id, name, kind, transport, builtin, enabled, license_warning, endpoint_url,
              cli_binary, cli_args_json, cli_workdir, auth_header, secret_key_ref,
              allow_local_network, timeout_secs, max_output_bytes, default_params_json,
              created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&rec.id)
        .bind(&rec.name)
        .bind(&rec.kind)
        .bind(&rec.transport)
        .bind(i64::from(rec.builtin))
        .bind(i64::from(rec.enabled))
        .bind(&rec.license_warning)
        .bind(&rec.endpoint_url)
        .bind(&rec.cli_binary)
        .bind(&cli_args_json)
        .bind(&rec.cli_workdir)
        .bind(&rec.auth_header)
        .bind(&rec.secret_key_ref)
        .bind(i64::from(rec.allow_local_network))
        .bind(rec.timeout_secs)
        .bind(rec.max_output_bytes)
        .bind(&default_params_json)
        .bind(stamp(rec.created_at))
        .bind(stamp(rec.updated_at))
        .execute(&self.pool)
        .await
        .map_err(dberr("media.3d_providers.insert"))?;
        self.require_3d_provider(&rec.id).await
    }

    pub async fn update_3d_provider(
        &self,
        rec: &Media3dProviderRecord,
    ) -> Result<Media3dProviderRecord> {
        let cli_args_json = serde_json::to_string(&rec.cli_args_template)
            .map_err(|e| Error::Internal(format!("serialize cli_args: {e}")))?;
        let default_params_json = serde_json::to_string(&rec.default_params)
            .map_err(|e| Error::Internal(format!("serialize default_params: {e}")))?;
        let res = sqlx::query(
            "UPDATE media_3d_providers SET
                name = ?, transport = ?, enabled = ?, license_warning = ?, endpoint_url = ?,
                cli_binary = ?, cli_args_json = ?, cli_workdir = ?, auth_header = ?,
                secret_key_ref = ?, allow_local_network = ?, timeout_secs = ?,
                max_output_bytes = ?, default_params_json = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(&rec.name)
        .bind(&rec.transport)
        .bind(i64::from(rec.enabled))
        .bind(&rec.license_warning)
        .bind(&rec.endpoint_url)
        .bind(&rec.cli_binary)
        .bind(&cli_args_json)
        .bind(&rec.cli_workdir)
        .bind(&rec.auth_header)
        .bind(&rec.secret_key_ref)
        .bind(i64::from(rec.allow_local_network))
        .bind(rec.timeout_secs)
        .bind(rec.max_output_bytes)
        .bind(&default_params_json)
        .bind(stamp(rec.updated_at))
        .bind(&rec.id)
        .execute(&self.pool)
        .await
        .map_err(dberr("media.3d_providers.update"))?;
        if res.rows_affected() == 0 {
            return Err(Error::NotFound(format!("3D provider {}", rec.id)));
        }
        self.require_3d_provider(&rec.id).await
    }

    pub async fn delete_3d_provider(&self, id: &str) -> Result<()> {
        let res = sqlx::query("DELETE FROM media_3d_providers WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("media.3d_providers.delete"))?;
        if res.rows_affected() == 0 {
            return Err(Error::NotFound(format!("3D provider {id}")));
        }
        Ok(())
    }

    // ── 3D Jobs ─────────────────────────────────────────────────────────────

    pub async fn insert_3d_job(&self, job: &Media3dJob) -> Result<Media3dJob> {
        let manifest_json = job
            .manifest
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| Error::Internal(format!("serialize manifest: {e}")))?;
        sqlx::query(
            "INSERT INTO media_3d_jobs
             (id, workspace_id, project_id, provider_id, provider_kind, pipeline_stage,
              source_artifact_id, source_version_id, target_artifact_id, title, prompt,
              output_format, preset, poly_budget, seed, status, progress_pct, stage,
              artifact_id, version_id, exported_path, size_bytes, manifest_json, error,
              created_by, created_at, updated_at, finished_at, then_stages_json)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&job.id)
        .bind(&job.workspace_id)
        .bind(&job.project_id)
        .bind(&job.provider_id)
        .bind(&job.provider_kind)
        .bind(&job.pipeline_stage)
        .bind(&job.source_artifact_id)
        .bind(&job.source_version_id)
        .bind(&job.target_artifact_id)
        .bind(&job.title)
        .bind(&job.prompt)
        .bind(&job.output_format)
        .bind(&job.preset)
        .bind(job.poly_budget)
        .bind(job.seed)
        .bind(&job.status)
        .bind(job.progress_pct)
        .bind(&job.stage)
        .bind(&job.artifact_id)
        .bind(&job.version_id)
        .bind(&job.exported_path)
        .bind(job.size_bytes)
        .bind(&manifest_json)
        .bind(&job.error)
        .bind(&job.created_by)
        .bind(stamp(job.created_at))
        .bind(stamp(job.updated_at))
        .bind(job.finished_at.map(stamp))
        .bind(serde_json::to_string(&job.then_stages).unwrap_or_else(|_| "[]".into()))
        .execute(&self.pool)
        .await
        .map_err(dberr("media.3d_jobs.insert"))?;
        self.require_3d_job(&job.id).await
    }

    pub async fn get_3d_job(&self, id: &str) -> Result<Option<Media3dJob>> {
        let row = sqlx::query("SELECT * FROM media_3d_jobs WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("media.3d_jobs.get"))?;
        row.as_ref().map(row_3d_job).transpose()
    }

    pub async fn require_3d_job(&self, id: &str) -> Result<Media3dJob> {
        self.get_3d_job(id)
            .await?
            .ok_or_else(|| Error::NotFound(format!("3D generation job {id}")))
    }

    pub async fn list_3d_jobs(
        &self,
        workspace_id: Option<&str>,
        artifact_id: Option<&str>,
        limit: i64,
    ) -> Result<Vec<Media3dJob>> {
        let limit = limit.clamp(1, 200);
        let rows = sqlx::query(
            "SELECT * FROM media_3d_jobs
             WHERE (? IS NULL OR workspace_id = ?)
               AND (? IS NULL OR artifact_id = ? OR target_artifact_id = ? OR source_artifact_id = ?)
             ORDER BY created_at DESC, id DESC
             LIMIT ?",
        )
        .bind(workspace_id)
        .bind(workspace_id)
        .bind(artifact_id)
        .bind(artifact_id)
        .bind(artifact_id)
        .bind(artifact_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("media.3d_jobs.list"))?;
        rows.iter().map(row_3d_job).collect()
    }

    /// Progress never moves backwards (`MAX`); a finished job is left alone.
    pub async fn update_3d_job_progress(
        &self,
        id: &str,
        status: &str,
        progress_pct: i64,
        stage: &str,
    ) -> Result<()> {
        let now = stamp(Utc::now());
        sqlx::query(
            "UPDATE media_3d_jobs SET status = ?, progress_pct = MAX(progress_pct, ?), stage = ?,
             updated_at = ? WHERE id = ? AND status NOT IN ('cancelled', 'succeeded', 'failed')",
        )
        .bind(status)
        .bind(progress_pct.clamp(0, 100))
        .bind(stage)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("media.3d_jobs.progress"))?;
        Ok(())
    }

    /// A chained job moved on to its next stage: record the stage it runs now
    /// and the version the previous stage committed, so a client can show it.
    pub async fn advance_3d_job_stage(
        &self,
        id: &str,
        stage: &str,
        artifact_id: Option<&str>,
        version_id: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE media_3d_jobs SET pipeline_stage = ?, artifact_id = COALESCE(?, artifact_id),
             version_id = COALESCE(?, version_id), updated_at = ?
             WHERE id = ? AND status NOT IN ('cancelled', 'succeeded', 'failed')",
        )
        .bind(stage)
        .bind(artifact_id)
        .bind(version_id)
        .bind(stamp(Utc::now()))
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("media.3d_jobs.advance"))?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn finish_3d_job_success(
        &self,
        id: &str,
        artifact_id: Option<&str>,
        version_id: Option<&str>,
        exported_path: Option<&str>,
        size_bytes: i64,
        manifest: Option<&Value>,
        error: Option<&str>,
    ) -> Result<Media3dJob> {
        let now = stamp(Utc::now());
        let manifest_json = manifest
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| Error::Internal(format!("serialize manifest: {e}")))?;
        sqlx::query(
            "UPDATE media_3d_jobs SET
                status = 'succeeded', progress_pct = 100, stage = 'Done',
                artifact_id = ?, version_id = ?, exported_path = ?, size_bytes = ?,
                manifest_json = ?, error = ?, updated_at = ?, finished_at = ?
             WHERE id = ? AND status NOT IN ('cancelled', 'succeeded', 'failed')",
        )
        .bind(artifact_id)
        .bind(version_id)
        .bind(exported_path)
        .bind(size_bytes)
        .bind(&manifest_json)
        .bind(error)
        .bind(&now)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("media.3d_jobs.finish_ok"))?;
        self.require_3d_job(id).await
    }

    pub async fn finish_3d_job_terminal(
        &self,
        id: &str,
        status: &str,
        stage: &str,
        error: Option<&str>,
    ) -> Result<Media3dJob> {
        let now = stamp(Utc::now());
        sqlx::query(
            "UPDATE media_3d_jobs SET
                status = ?, stage = ?, error = ?, updated_at = ?, finished_at = ?
             WHERE id = ? AND status NOT IN ('cancelled', 'succeeded', 'failed')",
        )
        .bind(status)
        .bind(stage)
        .bind(error)
        .bind(&now)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("media.3d_jobs.finish_err"))?;
        self.require_3d_job(id).await
    }

    /// Boot recovery: a job's runner dies with the daemon, so a row a previous
    /// life left queued or running fails as interrupted. Returns how many.
    pub async fn fail_interrupted_3d_jobs(&self) -> Result<u64> {
        let now = stamp(Utc::now());
        let res = sqlx::query(
            "UPDATE media_3d_jobs SET status = 'failed', stage = 'Interrupted',
                error = 'Interrupted by a daemon restart — start the job again.',
                updated_at = ?, finished_at = ?
             WHERE status IN ('queued', 'running')",
        )
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("media.3d_jobs.recover"))?;
        Ok(res.rows_affected())
    }

    // ── TTS Providers ───────────────────────────────────────────────────────

    pub async fn list_tts_providers(&self) -> Result<Vec<TtsProviderRecord>> {
        self.ensure_builtins().await?;
        let rows = sqlx::query(
            "SELECT * FROM media_tts_providers ORDER BY builtin DESC, created_at ASC, id ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("media.tts_providers.list"))?;
        rows.iter().map(row_tts_provider).collect()
    }

    pub async fn get_tts_provider(&self, id: &str) -> Result<Option<TtsProviderRecord>> {
        self.ensure_builtins().await?;
        let row = sqlx::query("SELECT * FROM media_tts_providers WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("media.tts_providers.get"))?;
        row.as_ref().map(row_tts_provider).transpose()
    }

    pub async fn require_tts_provider(&self, id: &str) -> Result<TtsProviderRecord> {
        self.get_tts_provider(id)
            .await?
            .ok_or_else(|| Error::NotFound(format!("TTS provider {id}")))
    }

    pub async fn insert_tts_provider(&self, rec: &TtsProviderRecord) -> Result<TtsProviderRecord> {
        sqlx::query(
            "INSERT INTO media_tts_providers
             (id, name, kind, builtin, enabled, endpoint_url, default_model, default_voice,
              auth_header, secret_key_ref, allow_local_network, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&rec.id)
        .bind(&rec.name)
        .bind(&rec.kind)
        .bind(i64::from(rec.builtin))
        .bind(i64::from(rec.enabled))
        .bind(&rec.endpoint_url)
        .bind(&rec.default_model)
        .bind(&rec.default_voice)
        .bind(&rec.auth_header)
        .bind(&rec.secret_key_ref)
        .bind(i64::from(rec.allow_local_network))
        .bind(stamp(rec.created_at))
        .bind(stamp(rec.updated_at))
        .execute(&self.pool)
        .await
        .map_err(dberr("media.tts_providers.insert"))?;
        self.require_tts_provider(&rec.id).await
    }

    pub async fn update_tts_provider(&self, rec: &TtsProviderRecord) -> Result<TtsProviderRecord> {
        let res = sqlx::query(
            "UPDATE media_tts_providers SET
                name = ?, enabled = ?, endpoint_url = ?, default_model = ?, default_voice = ?,
                auth_header = ?, secret_key_ref = ?, allow_local_network = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(&rec.name)
        .bind(i64::from(rec.enabled))
        .bind(&rec.endpoint_url)
        .bind(&rec.default_model)
        .bind(&rec.default_voice)
        .bind(&rec.auth_header)
        .bind(&rec.secret_key_ref)
        .bind(i64::from(rec.allow_local_network))
        .bind(stamp(rec.updated_at))
        .bind(&rec.id)
        .execute(&self.pool)
        .await
        .map_err(dberr("media.tts_providers.update"))?;
        if res.rows_affected() == 0 {
            return Err(Error::NotFound(format!("TTS provider {}", rec.id)));
        }
        self.require_tts_provider(&rec.id).await
    }

    pub async fn delete_tts_provider(&self, id: &str) -> Result<()> {
        let res = sqlx::query("DELETE FROM media_tts_providers WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("media.tts_providers.delete"))?;
        if res.rows_affected() == 0 {
            return Err(Error::NotFound(format!("TTS provider {id}")));
        }
        Ok(())
    }

    // ── TTS Clips ───────────────────────────────────────────────────────────

    pub async fn insert_tts_clip(&self, rec: &TtsClipRecord) -> Result<TtsClipRecord> {
        let c = &rec.clip;
        let visemes_json = c
            .visemes
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| Error::Internal(format!("serialize visemes: {e}")))?;
        sqlx::query(
            "INSERT INTO media_tts_clips
             (id, workspace_id, story_id, artifact_id, provider_id, provider_kind, voice_id,
              title, character_name, text, format, mime, speed, size_bytes, duration_ms,
              file_name, exported_path, visemes_json, created_by, created_at, kind,
              manifest_json)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&c.id)
        .bind(&c.workspace_id)
        .bind(&c.story_id)
        .bind(&c.artifact_id)
        .bind(&c.provider_id)
        .bind(&c.provider_kind)
        .bind(&c.voice_id)
        .bind(&c.title)
        .bind(&c.character_name)
        .bind(&c.text)
        .bind(&c.format)
        .bind(&c.mime)
        .bind(f64::from(c.speed))
        .bind(c.size_bytes)
        .bind(c.duration_ms)
        .bind(&rec.file_name)
        .bind(&c.exported_path)
        .bind(&visemes_json)
        .bind(&c.created_by)
        .bind(stamp(c.created_at))
        .bind(&c.kind)
        .bind(c.manifest.as_ref().map(Value::to_string))
        .execute(&self.pool)
        .await
        .map_err(dberr("media.tts_clips.insert"))?;
        self.require_tts_clip(&c.id).await
    }

    pub async fn get_tts_clip(&self, id: &str) -> Result<Option<TtsClipRecord>> {
        let row = sqlx::query("SELECT * FROM media_tts_clips WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("media.tts_clips.get"))?;
        row.as_ref().map(row_tts_clip).transpose()
    }

    pub async fn require_tts_clip(&self, id: &str) -> Result<TtsClipRecord> {
        self.get_tts_clip(id)
            .await?
            .ok_or_else(|| Error::NotFound(format!("TTS clip {id}")))
    }

    pub async fn list_tts_clips(
        &self,
        workspace_id: &str,
        story_id: Option<&str>,
        artifact_id: Option<&str>,
        kind: Option<&str>,
        limit: i64,
    ) -> Result<Vec<TtsClip>> {
        let limit = limit.clamp(1, 200);
        let rows = sqlx::query(
            "SELECT * FROM media_tts_clips
             WHERE workspace_id = ?
               AND (? IS NULL OR story_id = ?)
               AND (? IS NULL OR artifact_id = ?)
               AND (? IS NULL OR kind = ?)
             ORDER BY created_at DESC, id DESC
             LIMIT ?",
        )
        .bind(workspace_id)
        .bind(story_id)
        .bind(story_id)
        .bind(artifact_id)
        .bind(artifact_id)
        .bind(kind)
        .bind(kind)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("media.tts_clips.list"))?;
        rows.iter()
            .map(|r| row_tts_clip(r).map(|rec| rec.clip))
            .collect()
    }

    pub async fn set_tts_clip_exported(&self, id: &str, exported_path: &str) -> Result<()> {
        sqlx::query("UPDATE media_tts_clips SET exported_path = ? WHERE id = ?")
            .bind(exported_path)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("media.tts_clips.set_exported"))?;
        Ok(())
    }

    /// The model a clip belongs to (a line spoken before its speaker was made).
    pub async fn set_tts_clip_artifact(&self, id: &str, artifact_id: &str) -> Result<()> {
        sqlx::query("UPDATE media_tts_clips SET artifact_id = ? WHERE id = ?")
            .bind(artifact_id)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("media.tts_clips.set_artifact"))?;
        Ok(())
    }

    pub async fn delete_tts_clip(&self, id: &str) -> Result<TtsClipRecord> {
        let rec = self.require_tts_clip(id).await?;
        sqlx::query("DELETE FROM media_tts_clips WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("media.tts_clips.delete"))?;
        Ok(rec)
    }
}

fn row_3d_provider(r: &sqlx::sqlite::SqliteRow) -> Result<Media3dProviderRecord> {
    let args_raw: String = r.get("cli_args_json");
    let cli_args_template: Vec<String> = serde_json::from_str(&args_raw).unwrap_or_default();
    let params_raw: String = r.get("default_params_json");
    let default_params: Value = serde_json::from_str(&params_raw).unwrap_or_else(|_| json!({}));
    Ok(Media3dProviderRecord {
        id: r.get("id"),
        name: r.get("name"),
        kind: r.get("kind"),
        transport: r.get("transport"),
        builtin: r.get::<i64, _>("builtin") != 0,
        enabled: r.get::<i64, _>("enabled") != 0,
        license_warning: r.get("license_warning"),
        endpoint_url: r.get("endpoint_url"),
        cli_binary: r.get("cli_binary"),
        cli_args_template,
        cli_workdir: r.get("cli_workdir"),
        auth_header: r.get("auth_header"),
        secret_key_ref: r.get("secret_key_ref"),
        allow_local_network: r.get::<i64, _>("allow_local_network") != 0,
        timeout_secs: r.get("timeout_secs"),
        max_output_bytes: r.get("max_output_bytes"),
        default_params,
        created_at: parse_ts(&r.get::<String, _>("created_at"))?,
        updated_at: parse_ts(&r.get::<String, _>("updated_at"))?,
    })
}

fn row_3d_job(r: &sqlx::sqlite::SqliteRow) -> Result<Media3dJob> {
    let manifest: Option<Value> = r
        .try_get::<Option<String>, _>("manifest_json")
        .ok()
        .flatten()
        .filter(|s| !s.is_empty())
        .and_then(|s| serde_json::from_str::<Value>(&s).ok());
    Ok(Media3dJob {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        project_id: r.get("project_id"),
        provider_id: r.get("provider_id"),
        provider_kind: r.get("provider_kind"),
        pipeline_stage: r.get("pipeline_stage"),
        then_stages: r
            .try_get::<Option<String>, _>("then_stages_json")
            .ok()
            .flatten()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default(),
        source_artifact_id: r.get("source_artifact_id"),
        source_version_id: r.get("source_version_id"),
        target_artifact_id: r.get("target_artifact_id"),
        title: r.get("title"),
        prompt: r.get("prompt"),
        output_format: r.get("output_format"),
        preset: r.get("preset"),
        poly_budget: r.get("poly_budget"),
        seed: r.get("seed"),
        status: r.get("status"),
        progress_pct: r.get("progress_pct"),
        stage: r.get("stage"),
        artifact_id: r.get("artifact_id"),
        version_id: r.get("version_id"),
        exported_path: r.get("exported_path"),
        size_bytes: r.get("size_bytes"),
        manifest,
        error: r.get("error"),
        created_by: r.get("created_by"),
        created_at: parse_ts(&r.get::<String, _>("created_at"))?,
        updated_at: parse_ts(&r.get::<String, _>("updated_at"))?,
        finished_at: opt_ts(r.get("finished_at"))?,
    })
}

fn row_tts_provider(r: &sqlx::sqlite::SqliteRow) -> Result<TtsProviderRecord> {
    Ok(TtsProviderRecord {
        id: r.get("id"),
        name: r.get("name"),
        kind: r.get("kind"),
        builtin: r.get::<i64, _>("builtin") != 0,
        enabled: r.get::<i64, _>("enabled") != 0,
        endpoint_url: r.get("endpoint_url"),
        default_model: r.get("default_model"),
        default_voice: r.get("default_voice"),
        auth_header: r.get("auth_header"),
        secret_key_ref: r.get("secret_key_ref"),
        allow_local_network: r.get::<i64, _>("allow_local_network") != 0,
        created_at: parse_ts(&r.get::<String, _>("created_at"))?,
        updated_at: parse_ts(&r.get::<String, _>("updated_at"))?,
    })
}

fn row_tts_clip(r: &sqlx::sqlite::SqliteRow) -> Result<TtsClipRecord> {
    let id: Id = r.get("id");
    let visemes: Option<TtsVisemeTimeline> = r
        .try_get::<Option<String>, _>("visemes_json")
        .ok()
        .flatten()
        .filter(|s| !s.is_empty())
        .and_then(|s| serde_json::from_str::<TtsVisemeTimeline>(&s).ok());
    let clip = TtsClip {
        audio_url: format!("/api/v1/product/tts/clips/{id}/audio"),
        id,
        workspace_id: r.get("workspace_id"),
        story_id: r.get("story_id"),
        artifact_id: r.get("artifact_id"),
        provider_id: r.get("provider_id"),
        provider_kind: r.get("provider_kind"),
        voice_id: r.get("voice_id"),
        title: r.get("title"),
        character_name: r.get("character_name"),
        text: r.get("text"),
        format: r.get("format"),
        mime: r.get("mime"),
        speed: r.get::<f64, _>("speed") as f32,
        size_bytes: r.get("size_bytes"),
        duration_ms: r.get("duration_ms"),
        exported_path: r.get("exported_path"),
        visemes,
        kind: r.get("kind"),
        manifest: r
            .get::<Option<String>, _>("manifest_json")
            .and_then(|s| serde_json::from_str(&s).ok()),
        created_by: r.get("created_by"),
        created_at: parse_ts(&r.get::<String, _>("created_at"))?,
    };
    Ok(TtsClipRecord {
        clip,
        file_name: r.get("file_name"),
    })
}
