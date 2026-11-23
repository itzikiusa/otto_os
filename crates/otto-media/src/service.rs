//! Core service orchestrating 3D providers/jobs, TTS providers/synthesis/clips,
//! Keychain secrets, Design Hall auto-versioning, and workspace-confined exports.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use chrono::Utc;
use otto_core::cancel::{CancelGuard, CancelRegistryExt};
use otto_core::event::Event;
use otto_core::secrets::{delete_async, get_async, put_async, SecretStore};
use otto_core::{new_id, Error, Id, Result};
use otto_design::service::{Author, CreateInput, DesignService, SaveOpts, MAX_PROVENANCE_BYTES};
use otto_state::DbPool;
use serde_json::{json, Value};
use tokio::sync::{broadcast, Mutex, Semaphore};

use crate::export::{
    export_bytes_to_workspace, parse_safe_relative, ALLOWED_3D_EXPORT_EXTS,
    ALLOWED_AUDIO_EXPORT_EXTS,
};
use crate::gen3d::{driver_for_kind, validate_3d_output, Gen3dDriver, Gen3dOutput, Gen3dRequest};
use crate::net::validate_endpoint_url;
use crate::store::MediaStore;
use crate::tts::{driver_for_tts_kind, TtsDriver, TtsSynthRequest};
use crate::types::{
    builtin_game_asset_presets, capabilities_for_3d_provider, secret_key_3d, secret_key_tts,
    tts_mime_for_format, CreateMedia3dProviderReq, CreateTtsProviderReq, ExportWorkspaceResp,
    Media3dJob, Media3dProvider, Media3dProviderRecord, ProviderTestResult, StartMedia3dJobReq,
    SynthesizeTtsReq, TtsClip, TtsClipRecord, TtsProvider, TtsProviderRecord, TtsVoice,
    UpdateMedia3dProviderReq, UpdateTtsProviderReq, BUILTIN_MACOS_SAY_ID,
    DEFAULT_3D_MAX_OUTPUT_BYTES, DEFAULT_3D_TIMEOUT_SECS, MAX_3D_OUTPUT_BYTES, MAX_3D_THEN_STAGES,
    MAX_3D_TIMEOUT_SECS, MAX_CLI_ARGS, MAX_PROMPT_CHARS, MAX_PROVIDER_NAME_CHARS,
    MAX_TTS_TEXT_CHARS, MEDIA_3D_FORMATS, MEDIA_3D_KINDS, MEDIA_3D_PIPELINE_STAGES,
    MEDIA_3D_TRANSPORTS, MIN_3D_OUTPUT_BYTES, MIN_3D_TIMEOUT_SECS, TTS_FORMATS, TTS_KINDS,
};

const MAX_CONCURRENT_3D_JOBS: usize = 2;
const MAX_CONCURRENT_TTS_JOBS: usize = 4;
const VOICE_CACHE_TTL: Duration = Duration::from_secs(300);
const MAX_REF_IMAGE_BYTES: usize = 12 * 1024 * 1024;
/// How deep a JSON value nests (otto-design refuses past 8 levels).
fn depth(v: &Value) -> usize {
    match v {
        Value::Object(m) => 1 + m.values().map(depth).max().unwrap_or(0),
        Value::Array(a) => 1 + a.iter().map(depth).max().unwrap_or(0),
        _ => 0,
    }
}

/// A local CLI's progress reaches the job row at most this often (tests: well
/// inside the fake CLI's 1 s between lines, so no line is coalesced away).
pub(crate) const CLI_PROGRESS_FLUSH: Duration = if cfg!(test) {
    Duration::from_millis(100)
} else {
    Duration::from_secs(1)
};

pub struct MediaService {
    store: MediaStore,
    data_dir: PathBuf,
    secrets: Arc<dyn SecretStore>,
    events: Option<broadcast::Sender<Event>>,
    gen3d_permits: Arc<Semaphore>,
    /// Stop flags of 3D and audio jobs (ids are unique across both).
    pub(crate) gen3d_cancels: otto_core::cancel::CancelRegistry,
    tts_permits: Arc<Semaphore>,
    tts_voice_cache: Mutex<HashMap<String, (Instant, Vec<TtsVoice>)>>,
    gen3d_override: Option<Arc<dyn Gen3dDriver>>,
    tts_override: Option<Arc<dyn TtsDriver>>,
    /// Productions: one read-modify-write at a time, their Stop flags, and
    /// their GPU steps one at a time (a model's CPU step overlaps the next).
    pub(crate) production_lock: Mutex<()>,
    pub(crate) production_cancels: otto_core::cancel::CancelRegistry,
    pub(crate) production_gpu: Arc<Semaphore>,
    /// The one CPU job beside it (a game-ready pass, moves).
    pub(crate) production_cpu: Arc<Semaphore>,
    /// The making production's live state (its lanes share it): a steer or
    /// a redraw while it runs goes through it.
    pub(crate) production_runs:
        std::sync::Mutex<HashMap<Id, Arc<Mutex<crate::productions::Production>>>>,
    /// Running productions in Start order: the first one makes, the rest wait.
    pub(crate) production_queue: std::sync::Mutex<Vec<Id>>,
    /// The sound engine, one job at a time (beside the 3D work).
    pub(crate) audio_turn: Arc<Semaphore>,
    /// A running 3D job's latest engine step key (Forge's `"concept"`, `"gameready"`…).
    engine_steps: std::sync::Mutex<HashMap<Id, String>>,
}

impl MediaService {
    pub fn new(
        pool: DbPool,
        data_dir: PathBuf,
        secrets: Arc<dyn SecretStore>,
        events: Option<broadcast::Sender<Event>>,
    ) -> Self {
        Self {
            store: MediaStore::new(pool),
            data_dir,
            secrets,
            events,
            gen3d_permits: Arc::new(Semaphore::new(MAX_CONCURRENT_3D_JOBS)),
            gen3d_cancels: Default::default(),
            tts_permits: Arc::new(Semaphore::new(MAX_CONCURRENT_TTS_JOBS)),
            tts_voice_cache: Mutex::new(HashMap::new()),
            gen3d_override: None,
            tts_override: None,
            production_lock: Mutex::new(()),
            production_cancels: Default::default(),
            production_gpu: Arc::new(Semaphore::new(1)),
            production_cpu: Arc::new(Semaphore::new(1)),
            production_queue: Default::default(),
            production_runs: Default::default(),
            audio_turn: Arc::new(Semaphore::new(1)),
            engine_steps: Default::default(),
        }
    }

    pub(crate) fn engine_steps(&self) -> std::sync::MutexGuard<'_, HashMap<Id, String>> {
        self.engine_steps.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Test hook: inject a deterministic fake 3D generator driver.
    pub fn with_gen3d_driver(mut self, driver: Arc<dyn Gen3dDriver>) -> Self {
        self.gen3d_override = Some(driver);
        self
    }

    /// Test hook: inject a deterministic fake TTS driver.
    pub fn with_tts_driver(mut self, driver: Arc<dyn TtsDriver>) -> Self {
        self.tts_override = Some(driver);
        self
    }

    pub fn store(&self) -> &MediaStore {
        &self.store
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub(crate) fn design_service(&self) -> DesignService {
        DesignService::new(
            self.store.pool().clone(),
            self.data_dir.clone(),
            self.events.clone(),
        )
    }

    fn tts_clips_dir(&self) -> PathBuf {
        self.data_dir.join("media").join("tts")
    }

    async fn has_secret(&self, key_ref: Option<&str>) -> bool {
        let Some(k) = key_ref.filter(|s| !s.trim().is_empty()) else {
            return false;
        };
        get_async(&self.secrets, k)
            .await
            .ok()
            .flatten()
            .is_some_and(|v| !v.is_empty())
    }

    async fn read_secret(&self, key_ref: Option<&str>) -> Result<Option<String>> {
        let Some(k) = key_ref.filter(|s| !s.trim().is_empty()) else {
            return Ok(None);
        };
        get_async(&self.secrets, k).await
    }

    // -----------------------------------------------------------------------
    // 3D Providers
    // -----------------------------------------------------------------------

    pub async fn list_3d_providers(&self) -> Result<Vec<Media3dProvider>> {
        let recs = self.store.list_3d_providers().await?;
        let mut out = Vec::with_capacity(recs.len());
        for rec in recs {
            let has_key = self.has_secret(rec.secret_key_ref.as_deref()).await;
            out.push(rec.into_public(has_key));
        }
        Ok(out)
    }

    pub async fn create_3d_provider(
        &self,
        req: CreateMedia3dProviderReq,
    ) -> Result<Media3dProvider> {
        let name = clean_provider_name(&req.name)?;
        let kind = req.kind.trim().to_ascii_lowercase();
        if !MEDIA_3D_KINDS.contains(&kind.as_str()) {
            return Err(Error::Invalid(format!(
                "kind must be one of: {}",
                MEDIA_3D_KINDS.join(", ")
            )));
        }
        let transport = req.transport.trim().to_ascii_lowercase();
        if !MEDIA_3D_TRANSPORTS.contains(&transport.as_str()) {
            return Err(Error::Invalid(format!(
                "transport must be one of: {}",
                MEDIA_3D_TRANSPORTS.join(", ")
            )));
        }
        let allow_local = req.allow_local_network.unwrap_or(false);
        let api_key_clean = req
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let endpoint_url = clean_opt_str(req.endpoint_url);
        if let Some(url) = &endpoint_url {
            validate_endpoint_url(url, api_key_clean.is_some(), allow_local)?;
        }
        let cli_args = clean_cli_args(req.cli_args_template.unwrap_or_default())?;
        let timeout_secs = req
            .timeout_secs
            .unwrap_or(DEFAULT_3D_TIMEOUT_SECS)
            .clamp(MIN_3D_TIMEOUT_SECS, MAX_3D_TIMEOUT_SECS);
        let max_output_bytes = req
            .max_output_bytes
            .unwrap_or(DEFAULT_3D_MAX_OUTPUT_BYTES)
            .clamp(MIN_3D_OUTPUT_BYTES, MAX_3D_OUTPUT_BYTES as i64);
        let default_params = clean_params_object(req.default_params)?;

        let id = new_id();
        let secret_ref = if let Some(key) = &api_key_clean {
            let k = secret_key_3d(&id);
            put_async(&self.secrets, &k, key).await?;
            Some(k)
        } else {
            None
        };

        let now = Utc::now();
        let rec = Media3dProviderRecord {
            id: id.clone(),
            name,
            kind,
            transport,
            builtin: false,
            enabled: req.enabled.unwrap_or(true),
            license_warning: clean_opt_str(req.license_warning),
            endpoint_url,
            cli_binary: clean_cli_binary(req.cli_binary)?,
            cli_args_template: cli_args,
            cli_workdir: clean_opt_str(req.cli_workdir),
            auth_header: clean_auth_header(req.auth_header)?,
            secret_key_ref: secret_ref,
            allow_local_network: allow_local,
            timeout_secs,
            max_output_bytes,
            default_params,
            created_at: now,
            updated_at: now,
        };
        let saved = match self.store.insert_3d_provider(&rec).await {
            Ok(s) => s,
            Err(e) => {
                if let Some(k) = &rec.secret_key_ref {
                    let _ = delete_async(&self.secrets, k).await;
                }
                return Err(e);
            }
        };
        let has_key = self.has_secret(saved.secret_key_ref.as_deref()).await;
        Ok(saved.into_public(has_key))
    }

    pub async fn update_3d_provider(
        &self,
        id: &str,
        req: UpdateMedia3dProviderReq,
    ) -> Result<Media3dProvider> {
        let mut rec = self.store.require_3d_provider(id).await?;
        if let Some(name) = req.name {
            rec.name = clean_provider_name(&name)?;
        }
        if let Some(t) = req.transport {
            let t = t.trim().to_ascii_lowercase();
            if !MEDIA_3D_TRANSPORTS.contains(&t.as_str()) {
                return Err(Error::Invalid(format!(
                    "transport must be one of: {}",
                    MEDIA_3D_TRANSPORTS.join(", ")
                )));
            }
            rec.transport = t;
        }
        if let Some(enabled) = req.enabled {
            rec.enabled = enabled;
        }
        if let Some(lw) = req.license_warning {
            rec.license_warning = clean_opt_str(Some(lw));
        }
        if let Some(allow_local) = req.allow_local_network {
            rec.allow_local_network = allow_local;
        }
        if let Some(url) = req.endpoint_url {
            rec.endpoint_url = clean_opt_str(Some(url));
        }
        if let Some(bin) = req.cli_binary {
            rec.cli_binary = clean_cli_binary(Some(bin))?;
        }
        if let Some(args) = req.cli_args_template {
            rec.cli_args_template = clean_cli_args(args)?;
        }
        if let Some(wd) = req.cli_workdir {
            rec.cli_workdir = clean_opt_str(Some(wd));
        }
        if let Some(hdr) = req.auth_header {
            rec.auth_header = clean_auth_header(Some(hdr))?;
        }
        if let Some(ts) = req.timeout_secs {
            rec.timeout_secs = ts.clamp(MIN_3D_TIMEOUT_SECS, MAX_3D_TIMEOUT_SECS);
        }
        if let Some(max_b) = req.max_output_bytes {
            rec.max_output_bytes = max_b.clamp(MIN_3D_OUTPUT_BYTES, MAX_3D_OUTPUT_BYTES as i64);
        }
        if let Some(dp) = req.default_params {
            rec.default_params = clean_params_object(Some(dp))?;
        }

        // Determine final credential presence BEFORE validating endpoint URL.
        let will_have_key = match &req.api_key {
            Some(s) => !s.trim().is_empty(),
            None => self.has_secret(rec.secret_key_ref.as_deref()).await,
        };
        if let Some(url) = &rec.endpoint_url {
            validate_endpoint_url(url, will_have_key, rec.allow_local_network)?;
        }

        if let Some(raw_key) = req.api_key {
            let trimmed = raw_key.trim();
            if trimmed.is_empty() {
                if let Some(k) = rec.secret_key_ref.take() {
                    let _ = delete_async(&self.secrets, &k).await;
                }
            } else {
                let k = secret_key_3d(&rec.id);
                put_async(&self.secrets, &k, trimmed).await?;
                rec.secret_key_ref = Some(k);
            }
        }

        rec.updated_at = Utc::now();
        let saved = self.store.update_3d_provider(&rec).await?;
        let has_key = self.has_secret(saved.secret_key_ref.as_deref()).await;
        Ok(saved.into_public(has_key))
    }

    pub async fn delete_3d_provider(&self, id: &str) -> Result<()> {
        let rec = self.store.require_3d_provider(id).await?;
        if rec.builtin {
            return Err(Error::Conflict(
                "built-in 3D providers cannot be deleted — disable or reconfigure them instead"
                    .into(),
            ));
        }
        if let Some(k) = &rec.secret_key_ref {
            let _ = delete_async(&self.secrets, k).await;
        }
        self.store.delete_3d_provider(id).await
    }

    pub async fn test_3d_provider(&self, id: &str) -> Result<ProviderTestResult> {
        let rec = self.store.require_3d_provider(id).await?;
        let api_key = self.read_secret(rec.secret_key_ref.as_deref()).await?;
        let driver = self
            .gen3d_override
            .clone()
            .unwrap_or_else(|| driver_for_kind(&rec.kind));
        driver.test_connection(&rec, api_key.as_deref()).await
    }

    // -----------------------------------------------------------------------
    // 3D Generation Jobs
    // -----------------------------------------------------------------------

    pub async fn start_3d_job(
        self: &Arc<Self>,
        req: StartMedia3dJobReq,
        user_id: &str,
        workspace_root: Option<PathBuf>,
    ) -> Result<Media3dJob> {
        let provider = self.store.require_3d_provider(&req.provider_id).await?;
        if !provider.enabled {
            return Err(Error::Conflict(format!(
                "3D provider '{}' is disabled",
                provider.name
            )));
        }

        let pipeline_stage = req
            .stage
            .as_deref()
            .unwrap_or("generate")
            .trim()
            .to_ascii_lowercase();
        if !MEDIA_3D_PIPELINE_STAGES.contains(&pipeline_stage.as_str()) {
            return Err(Error::Invalid(format!(
                "stage must be one of: {}",
                MEDIA_3D_PIPELINE_STAGES.join(", ")
            )));
        }
        let caps = capabilities_for_3d_provider(&provider.id, &provider.kind);
        if !caps.supported_stages.iter().any(|s| s == &pipeline_stage) {
            return Err(Error::Invalid(format!(
                "provider '{}' does not support pipeline stage '{pipeline_stage}'",
                provider.name
            )));
        }
        // Stages chained after this one run in the same job on its output mesh.
        let then_stages: Vec<String> = req
            .then_stages
            .iter()
            .map(|s| s.trim().to_ascii_lowercase())
            .collect();
        if then_stages.len() > MAX_3D_THEN_STAGES {
            return Err(Error::Invalid(format!(
                "then_stages takes at most {MAX_3D_THEN_STAGES} stages"
            )));
        }
        for s in &then_stages {
            if s == "generate" || !MEDIA_3D_PIPELINE_STAGES.contains(&s.as_str()) {
                return Err(Error::Invalid(
                    "then_stages takes only gameready, rig or animate".into(),
                ));
            }
            if !caps.supported_stages.iter().any(|c| c == s) {
                return Err(Error::Invalid(format!(
                    "provider '{}' does not support pipeline stage '{s}'",
                    provider.name
                )));
            }
        }

        // Validate prompt / image presence.
        let prompt = req
            .prompt
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        if prompt
            .as_ref()
            .is_some_and(|p| p.chars().count() > MAX_PROMPT_CHARS)
        {
            return Err(Error::Invalid(format!(
                "prompt is at most {MAX_PROMPT_CHARS} characters"
            )));
        }

        let design = self.design_service();
        let (image_bytes, image_mime) = if let Some(b64) = req
            .image_b64
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let clean = b64.split_once(',').map(|(_, r)| r).unwrap_or(b64);
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(clean)
                .map_err(|e| Error::Invalid(format!("invalid image_b64: {e}")))?;
            if bytes.len() > MAX_REF_IMAGE_BYTES {
                return Err(Error::PayloadTooLarge(
                    "reference image exceeds 12 MB limit".into(),
                ));
            }
            (Some(bytes), req.image_mime.clone())
        } else if let Some(aid) = &req.image_artifact_id {
            let a = design.store().require_artifact(aid).await?;
            if a.workspace_id != req.workspace_id {
                return Err(Error::Forbidden(
                    "image_artifact_id belongs to a different workspace".into(),
                ));
            }
            let (_, bytes) = design.head_content(&a).await?;
            if bytes.len() > MAX_REF_IMAGE_BYTES {
                return Err(Error::PayloadTooLarge(
                    "reference image artifact exceeds 12 MB limit".into(),
                ));
            }
            (Some(bytes), Some(a.mime))
        } else {
            (None, None)
        };

        // Resolve source mesh when running a transform stage (`gameready` / `rig` / `animate`)
        // or when `source_artifact_id` is explicitly supplied.
        let source_artifact_id = clean_opt_str(req.source_artifact_id).or_else(|| {
            if pipeline_stage != "generate" {
                clean_opt_str(req.target_artifact_id.clone())
            } else {
                None
            }
        });
        let source_version_id_req = clean_opt_str(req.source_version_id);
        let (source_mesh_bytes, source_mesh_format, resolved_source_version_id, source_title) =
            if let Some(sid) = &source_artifact_id {
                let a = design.store().require_artifact(sid).await?;
                if a.workspace_id != req.workspace_id {
                    return Err(Error::Forbidden(
                        "source_artifact_id belongs to a different workspace".into(),
                    ));
                }
                let (ver, bytes) = match source_version_id_req.as_deref() {
                    Some(vref) => {
                        let ver = design.resolve_version(&a, vref).await?;
                        let b = design.version_bytes(&ver).await?;
                        (ver, b)
                    }
                    None => design.head_content(&a).await?,
                };
                if bytes.len() > MAX_3D_OUTPUT_BYTES {
                    return Err(Error::PayloadTooLarge(
                        "source 3D mesh exceeds 256 MiB limit".into(),
                    ));
                }
                (Some(bytes), Some(a.format), Some(ver.id), Some(a.title))
            } else {
                (None, None, None, None)
            };

        if pipeline_stage != "generate" && source_mesh_bytes.is_none() {
            return Err(Error::Invalid(format!(
                "stage '{pipeline_stage}' requires source_artifact_id (or target_artifact_id)"
            )));
        }
        if pipeline_stage == "generate"
            && prompt.is_none()
            && image_bytes.is_none()
            && source_mesh_bytes.is_none()
        {
            return Err(Error::Invalid(
                "provide a text prompt, reference image, or source_artifact_id".into(),
            ));
        }

        // For transform stages (`gameready`/`rig`/`animate`), default target to source so
        // the transformed mesh lands as a new version on the artifact!
        let target_artifact_id = clean_opt_str(req.target_artifact_id).or_else(|| {
            if pipeline_stage != "generate" {
                source_artifact_id.clone()
            } else {
                None
            }
        });
        let target_format = match &target_artifact_id {
            Some(target_id) => {
                let a = design.store().require_artifact(target_id).await?;
                if a.workspace_id != req.workspace_id {
                    return Err(Error::Forbidden(
                        "target_artifact_id belongs to a different workspace".into(),
                    ));
                }
                Some(a.format)
            }
            None => None,
        };

        // Resolve preset defaults.
        let preset_spec = req
            .preset
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .map(|pid| {
                builtin_game_asset_presets()
                    .into_iter()
                    .find(|p| p.id == pid)
                    .ok_or_else(|| Error::Invalid(format!("unknown preset '{pid}'")))
            })
            .transpose()?;
        let output_format = req
            .output_format
            .as_deref()
            .or_else(|| preset_spec.as_ref().map(|p| p.output_format.as_str()))
            .unwrap_or("glb")
            .trim()
            .to_ascii_lowercase();
        if !MEDIA_3D_FORMATS.contains(&output_format.as_str()) {
            return Err(Error::Invalid(format!(
                "output_format must be one of: {}",
                MEDIA_3D_FORMATS.join(", ")
            )));
        }
        // Refused now rather than after the run: the provider would write another
        // format, or the target's new version would not validate.
        if !caps.output_formats.contains(&output_format) {
            return Err(Error::Invalid(format!(
                "provider '{}' writes only: {}",
                provider.name,
                caps.output_formats.join(", ")
            )));
        }
        if target_format.is_some_and(|f| f != output_format) {
            return Err(Error::Invalid(format!(
                "output_format '{output_format}' differs from the target artifact's format"
            )));
        }
        let poly_budget = req
            .poly_budget
            .or_else(|| preset_spec.as_ref().map(|p| p.poly_budget));
        if poly_budget == Some(0) {
            return Err(Error::Invalid("poly_budget must be at least 1".into()));
        }

        let mut params = clean_params_object(req.params)?;
        if let Some(ps) = &preset_spec {
            if let Some(obj) = params.as_object_mut() {
                obj.entry("resolution".to_string())
                    .or_insert_with(|| json!(ps.resolution));
                obj.entry("texture_size".to_string())
                    .or_insert_with(|| json!(ps.texture_size));
                obj.entry("decimation_target".to_string())
                    .or_insert_with(|| json!(ps.poly_budget));
            }
        }

        let title = req
            .title
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or(source_title)
            .or_else(|| {
                prompt
                    .as_deref()
                    .map(|p| p.chars().take(60).collect::<String>())
            })
            .unwrap_or_else(|| format!("{} 3D Asset", provider.name));

        // Checked before the GPU work, not after it.
        let export_rel = clean_opt_str(req.export_relative_path);
        if let Some(rel) = &export_rel {
            parse_safe_relative(rel, ALLOWED_3D_EXPORT_EXTS)?;
        }

        let job_id = new_id();
        let now = Utc::now();
        let job = Media3dJob {
            id: job_id.clone(),
            workspace_id: req.workspace_id.clone(),
            project_id: req.project_id.clone(),
            provider_id: provider.id.clone(),
            provider_kind: provider.kind.clone(),
            pipeline_stage: pipeline_stage.clone(),
            then_stages,
            source_artifact_id,
            source_version_id: resolved_source_version_id,
            target_artifact_id,
            title: title.clone(),
            prompt: prompt.clone(),
            output_format: output_format.clone(),
            preset: req.preset.clone(),
            poly_budget: poly_budget.map(i64::from),
            seed: req.seed,
            status: "queued".into(),
            progress_pct: 5,
            stage: "Queued".into(),
            artifact_id: None,
            version_id: None,
            exported_path: None,
            size_bytes: None,
            manifest: None,
            error: None,
            created_by: user_id.to_string(),
            created_at: now,
            updated_at: now,
            finished_at: None,
        };
        let saved = self.store.insert_3d_job(&job).await?;

        let cancel_guard = self.gen3d_cancels.register(&job_id);
        let svc = Arc::clone(self);
        let auto_save = req.auto_save_artifact.unwrap_or(true);
        let max_output_bytes = provider.effective_max_output_bytes();
        let gen_req = Gen3dRequest {
            job_id: job_id.clone(),
            pipeline_stage,
            prompt,
            image_bytes,
            image_mime,
            source_mesh_bytes,
            source_mesh_format,
            output_format,
            poly_budget,
            seed: req.seed,
            params,
            max_output_bytes,
            timeout: Duration::from_secs(provider.timeout_secs.max(15) as u64),
            progress: None,
        };
        let user_id_owned = user_id.to_string();

        tokio::spawn(otto_core::auth::carry_request_credential(async move {
            svc.execute_3d_job(
                saved.clone(),
                provider,
                gen_req,
                user_id_owned,
                auto_save,
                workspace_root,
                export_rel,
                cancel_guard,
            )
            .await;
        }));

        Ok(job)
    }

    #[allow(clippy::too_many_arguments)]
    async fn execute_3d_job(
        self: Arc<Self>,
        job: Media3dJob,
        provider: Media3dProviderRecord,
        mut req: Gen3dRequest,
        user_id: String,
        auto_save: bool,
        workspace_root: Option<PathBuf>,
        export_rel: Option<String>,
        cancel_guard: CancelGuard,
    ) {
        let _guard = &cancel_guard;
        let _permit = match self.gen3d_permits.clone().acquire_owned().await {
            Ok(p) => p,
            Err(_) => {
                let _ = self
                    .store
                    .finish_3d_job_terminal(&job.id, "failed", "Failed", Some("job queue closed"))
                    .await;
                return;
            }
        };

        if cancel_guard.is_cancelled() {
            let _ = self
                .store
                .finish_3d_job_terminal(&job.id, "cancelled", "Cancelled", None)
                .await;
            return;
        }

        let api_key = match self.read_secret(provider.secret_key_ref.as_deref()).await {
            Ok(k) => k,
            Err(e) => {
                let _ = self
                    .store
                    .finish_3d_job_terminal(&job.id, "failed", "Failed", Some(&e.to_string()))
                    .await;
                return;
            }
        };

        let driver = self
            .gen3d_override
            .clone()
            .unwrap_or_else(|| driver_for_kind(&provider.kind));

        // A chained job (`then_stages`) runs its stages one after the other on
        // the previous stage's mesh; each commits its own version.
        let stages: Vec<String> = std::iter::once(job.pipeline_stage.clone())
            .chain(job.then_stages.iter().cloned())
            .collect();
        let spans = stage_progress_spans(&stages);
        let mut target = job.target_artifact_id.clone();
        let mut source = (
            job.source_artifact_id.clone(),
            job.source_version_id.clone(),
        );
        let mut saved: Option<(Id, Id)> = None;
        let mut output: Option<Gen3dOutput> = None;

        for (k, stage) in stages.iter().enumerate() {
            if k > 0 {
                if cancel_guard.is_cancelled() {
                    let _ = self
                        .store
                        .finish_3d_job_terminal(&job.id, "cancelled", "Cancelled", None)
                        .await;
                    return;
                }
                let prev = output.take().expect("an earlier stage produced output");
                req.pipeline_stage = stage.clone();
                req.source_mesh_format = Some(prev.format);
                req.source_mesh_bytes = Some(prev.bytes);
                req.image_bytes = None;
                req.image_mime = None;
                let (aid, vid) = saved.clone().unzip();
                let _ = self
                    .store
                    .advance_3d_job_stage(&job.id, stage, aid.as_deref(), vid.as_deref())
                    .await;
            }
            let (lo, hi) = spans[k];
            let label = match stage.as_str() {
                "gameready" => "Making game-ready",
                "rig" => "Rigging skeleton",
                "animate" => "Animating",
                _ => "Generating 3D mesh",
            };
            let _ = self
                .store
                .update_3d_job_progress(
                    &job.id,
                    "running",
                    lo.max(20),
                    &format!("{label} with {}", provider.name),
                )
                .await;

            // A local CLI's stderr progress reaches the job row at most once per
            // flush, scaled into this stage's share of the bar.
            let (progress_tx, mut progress_rx) = tokio::sync::watch::channel(None);
            req.progress = Some(progress_tx);
            let (svc, job_id) = (Arc::clone(&self), job.id.clone());
            let progress_writer = tokio::spawn(async move {
                while progress_rx.changed().await.is_ok() {
                    let latest = progress_rx.borrow_and_update().clone();
                    if let Some((pct, label, step)) = latest {
                        let pct = lo + pct.clamp(0, 100) * (hi - lo) / 100;
                        let _ = svc
                            .store
                            .update_3d_job_progress(&job_id, "running", pct, &label)
                            .await;
                        svc.engine_steps().insert(job_id.clone(), step);
                    }
                    tokio::time::sleep(CLI_PROGRESS_FLUSH).await;
                }
                svc.engine_steps().remove(&job_id);
            });
            let generated = driver
                .generate(&req, &provider, api_key.as_deref(), cancel_guard.flag())
                .await;
            req.progress = None; // closes the channel: the writer ends after its last write
            let _ = progress_writer.await;
            let out = match generated {
                Ok(o) => o,
                Err(e) => {
                    let (st, label) = if cancel_guard.is_cancelled() {
                        ("cancelled", "Cancelled")
                    } else {
                        ("failed", "Failed")
                    };
                    let _ = self
                        .store
                        .finish_3d_job_terminal(&job.id, st, label, Some(&e.to_string()))
                        .await;
                    return;
                }
            };
            // Cancelled while the provider finished (a synchronous HTTP call never
            // looks at the flag): the row is already `cancelled`, so save nothing.
            if cancel_guard.is_cancelled() {
                return;
            }
            if let Err(e) = validate_3d_output(&out.format, &out.bytes, req.max_output_bytes) {
                let _ = self
                    .store
                    .finish_3d_job_terminal(
                        &job.id,
                        "failed",
                        "Validation failed",
                        Some(&e.to_string()),
                    )
                    .await;
                return;
            }
            let saving = if stages.len() == 1 { 80 } else { hi };
            let _ = self
                .store
                .update_3d_job_progress(&job.id, "running", saving, "Saving 3D artifact version")
                .await;
            if auto_save {
                match self
                    .commit_3d_output(
                        &job,
                        &provider,
                        stage,
                        &source,
                        target.as_deref(),
                        &out,
                        &user_id,
                    )
                    .await
                {
                    Ok((aid, vid)) => {
                        target = Some(aid.clone());
                        source = (Some(aid.clone()), Some(vid.clone()));
                        saved = Some((aid, vid));
                    }
                    Err(e) => {
                        let _ = self
                            .store
                            .finish_3d_job_terminal(
                                &job.id,
                                "failed",
                                "Artifact save failed",
                                Some(&e.to_string()),
                            )
                            .await;
                        return;
                    }
                }
            }
            output = Some(out);
        }
        let output = output.expect("every job runs at least one stage");
        let size_bytes = output.bytes.len() as i64;
        let (saved_artifact_id, saved_version_id) = saved.unzip();

        let (mut exported_path, mut export_error) = (None, None);
        if let (Some(rel), Some(ws_root)) = (export_rel, workspace_root) {
            match export_bytes_to_workspace(
                &ws_root,
                &self.data_dir,
                &rel,
                ALLOWED_3D_EXPORT_EXTS,
                &output.bytes,
                true,
            )
            .await
            {
                Ok(exp) => exported_path = Some(exp.path),
                Err(e) => {
                    tracing::warn!(job = %job.id, "3D job export to {rel} failed: {e}");
                    let msg = format!("export to {rel} failed: {e}");
                    // Nothing was kept: the job failed. A saved artifact still succeeds.
                    if saved_artifact_id.is_none() {
                        let _ = self
                            .store
                            .finish_3d_job_terminal(&job.id, "failed", "Export failed", Some(&msg))
                            .await;
                        return;
                    }
                    export_error = Some(msg);
                }
            }
        }

        let _ = self
            .store
            .finish_3d_job_success(
                &job.id,
                saved_artifact_id.as_deref(),
                saved_version_id.as_deref(),
                exported_path.as_deref(),
                size_bytes,
                output.manifest.as_ref(),
                export_error.as_deref(),
            )
            .await;
    }

    /// Commit one stage's mesh: a new version of `target` when there is one, else a
    /// new artifact. Returns the artifact and version ids.
    #[allow(clippy::too_many_arguments)]
    async fn commit_3d_output(
        &self,
        job: &Media3dJob,
        provider: &Media3dProviderRecord,
        stage: &str,
        source: &(Option<Id>, Option<Id>),
        target: Option<&str>,
        output: &Gen3dOutput,
        user_id: &str,
    ) -> Result<(Id, Id)> {
        let design = self.design_service();
        let mut provenance = json!({
            "generator": {
                "provider_id": provider.id,
                "provider_kind": provider.kind,
                "provider_name": provider.name,
                "pipeline_stage": stage,
                "source_artifact_id": source.0,
                "source_version_id": source.1,
                "job_id": job.id,
                "preset": job.preset,
                "poly_budget": job.poly_budget,
                "seed": job.seed,
            }
        });
        if let Some(m) = &output.manifest {
            provenance["manifest"] = m.clone();
            // Forge repeats the prompt in its manifest; past otto-design's cap
            // the save would fail, so the version points at the job instead.
            // An engine manifest nesting its inputs' manifests (animate on rig
            // on generate) is past otto-design's 8 levels too.
            if provenance.to_string().len() > MAX_PROVENANCE_BYTES || depth(&provenance) > 8 {
                provenance["manifest"] =
                    json!({ "omitted": "over the provenance cap; see the 3D job's manifest" });
            }
        }
        let message = if stage == "generate" {
            format!("Generated via {}", provider.name)
        } else {
            format!("Stage '{stage}' via {}", provider.name)
        };
        let res = if let Some(target_id) = target {
            let existing = design.store().require_artifact(target_id).await?;
            design
                .commit_bytes(
                    &existing,
                    output.bytes.clone(),
                    SaveOpts {
                        base: None,
                        kind: "named".into(),
                        author: Author::user(user_id),
                        message,
                        provenance,
                        force: false,
                        validate: true,
                        change: "content",
                    },
                )
                .await?
        } else {
            design
                .create_artifact(CreateInput {
                    workspace_id: job.workspace_id.clone(),
                    project_id: job.project_id.clone(),
                    studio: Some("3d".into()),
                    format: output.format.clone(),
                    title: job.title.clone(),
                    tags: vec!["generated-3d".into(), provider.kind.clone()],
                    meta: json!({
                        "generated_by": provider.kind,
                        "generator_provider_id": provider.id,
                        "pipeline_stage": stage,
                        "poly_budget": job.poly_budget,
                    }),
                    content: Some(output.bytes.clone()),
                    story_id: None,
                    derived_from: None,
                    created_by: user_id.to_string(),
                    author: Author::user(user_id),
                    message: Some(message),
                    source: None,
                    created_at: None,
                    version_kind: Some("named".into()),
                    validate: true,
                    provenance: Some(provenance),
                })
                .await?
        };
        Ok((res.artifact.id, res.version.id))
    }

    pub async fn cancel_3d_job(&self, id: &str) -> Result<Media3dJob> {
        let job = self.store.require_3d_job(id).await?;
        if matches!(job.status.as_str(), "succeeded" | "failed" | "cancelled") {
            return Ok(job);
        }
        self.gen3d_cancels.signal(id);
        self.store
            .finish_3d_job_terminal(id, "cancelled", "Cancelled", None)
            .await
    }

    /// Export a 3D Design Hall artifact's bytes into a confined workspace path.
    pub async fn export_3d_artifact_to_workspace(
        &self,
        artifact_id: &str,
        workspace_root: &Path,
        relative_path: &str,
        version_id: Option<&str>,
        overwrite: bool,
    ) -> Result<ExportWorkspaceResp> {
        let design = self.design_service();
        let artifact = design.store().require_artifact(artifact_id).await?;
        let bytes = match version_id.filter(|v| !v.trim().is_empty()) {
            Some(v) => {
                let ver = design.resolve_version(&artifact, v).await?;
                design.version_bytes(&ver).await?
            }
            None => {
                let (_, b) = design.head_content(&artifact).await?;
                b
            }
        };
        export_bytes_to_workspace(
            workspace_root,
            &self.data_dir,
            relative_path,
            ALLOWED_3D_EXPORT_EXTS,
            &bytes,
            overwrite,
        )
        .await
    }

    // -----------------------------------------------------------------------
    // TTS Providers, Voices, Synthesis & Clips
    // -----------------------------------------------------------------------

    pub async fn list_tts_providers(&self) -> Result<Vec<TtsProvider>> {
        let recs = self.store.list_tts_providers().await?;
        let mut out = Vec::with_capacity(recs.len());
        for rec in recs {
            let has_key = self.has_secret(rec.secret_key_ref.as_deref()).await;
            out.push(rec.into_public(has_key));
        }
        Ok(out)
    }

    pub async fn create_tts_provider(&self, req: CreateTtsProviderReq) -> Result<TtsProvider> {
        let name = clean_provider_name(&req.name)?;
        let kind = req.kind.trim().to_ascii_lowercase();
        if kind == "macos_say" || !TTS_KINDS.contains(&kind.as_str()) {
            return Err(Error::Invalid(
                "kind must be one of: openai_compat, elevenlabs, kokoro, custom_http".into(),
            ));
        }
        let allow_local = req.allow_local_network.unwrap_or(kind == "kokoro");
        let api_key_clean = req
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let endpoint_url = clean_opt_str(req.endpoint_url);
        if let Some(url) = &endpoint_url {
            validate_endpoint_url(url, api_key_clean.is_some(), allow_local)?;
        }
        let id = new_id();
        let secret_ref = if let Some(key) = &api_key_clean {
            let k = secret_key_tts(&id);
            put_async(&self.secrets, &k, key).await?;
            Some(k)
        } else {
            None
        };
        let now = Utc::now();
        let rec = TtsProviderRecord {
            id: id.clone(),
            name,
            kind,
            builtin: false,
            enabled: req.enabled.unwrap_or(true),
            endpoint_url,
            default_model: clean_opt_str(req.default_model),
            default_voice: clean_opt_str(req.default_voice),
            auth_header: clean_auth_header(req.auth_header)?,
            secret_key_ref: secret_ref,
            allow_local_network: allow_local,
            created_at: now,
            updated_at: now,
        };
        let saved = match self.store.insert_tts_provider(&rec).await {
            Ok(s) => s,
            Err(e) => {
                if let Some(k) = &rec.secret_key_ref {
                    let _ = delete_async(&self.secrets, k).await;
                }
                return Err(e);
            }
        };
        let has_key = self.has_secret(saved.secret_key_ref.as_deref()).await;
        Ok(saved.into_public(has_key))
    }

    pub async fn update_tts_provider(
        &self,
        id: &str,
        req: UpdateTtsProviderReq,
    ) -> Result<TtsProvider> {
        let mut rec = self.store.require_tts_provider(id).await?;
        if let Some(name) = req.name {
            rec.name = clean_provider_name(&name)?;
        }
        if let Some(enabled) = req.enabled {
            rec.enabled = enabled;
        }
        if let Some(allow_local) = req.allow_local_network {
            rec.allow_local_network = allow_local;
        }
        if let Some(url) = req.endpoint_url {
            rec.endpoint_url = clean_opt_str(Some(url));
        }
        if let Some(m) = req.default_model {
            rec.default_model = clean_opt_str(Some(m));
        }
        if let Some(v) = req.default_voice {
            rec.default_voice = clean_opt_str(Some(v));
        }
        if let Some(h) = req.auth_header {
            rec.auth_header = clean_auth_header(Some(h))?;
        }
        let will_have_key = match &req.api_key {
            Some(s) => !s.trim().is_empty(),
            None => self.has_secret(rec.secret_key_ref.as_deref()).await,
        };
        if let Some(url) = &rec.endpoint_url {
            validate_endpoint_url(url, will_have_key, rec.allow_local_network)?;
        }
        if let Some(raw_key) = req.api_key {
            let trimmed = raw_key.trim();
            if trimmed.is_empty() {
                if let Some(k) = rec.secret_key_ref.take() {
                    let _ = delete_async(&self.secrets, &k).await;
                }
            } else {
                let k = secret_key_tts(&rec.id);
                put_async(&self.secrets, &k, trimmed).await?;
                rec.secret_key_ref = Some(k);
            }
        }
        rec.updated_at = Utc::now();
        let saved = self.store.update_tts_provider(&rec).await?;
        self.tts_voice_cache.lock().await.remove(id);
        let has_key = self.has_secret(saved.secret_key_ref.as_deref()).await;
        Ok(saved.into_public(has_key))
    }

    pub async fn delete_tts_provider(&self, id: &str) -> Result<()> {
        let rec = self.store.require_tts_provider(id).await?;
        if rec.builtin {
            return Err(Error::Conflict(
                "built-in macOS TTS provider cannot be deleted".into(),
            ));
        }
        if let Some(k) = &rec.secret_key_ref {
            let _ = delete_async(&self.secrets, k).await;
        }
        self.tts_voice_cache.lock().await.remove(id);
        self.store.delete_tts_provider(id).await
    }

    pub async fn test_tts_provider(&self, id: &str) -> Result<ProviderTestResult> {
        let rec = self.store.require_tts_provider(id).await?;
        let api_key = self.read_secret(rec.secret_key_ref.as_deref()).await?;
        let driver = self
            .tts_override
            .clone()
            .unwrap_or_else(|| driver_for_tts_kind(&rec.kind));
        driver.test_connection(&rec, api_key.as_deref()).await
    }

    pub async fn list_tts_voices(&self, provider_id: &str) -> Result<Vec<TtsVoice>> {
        {
            let cache = self.tts_voice_cache.lock().await;
            if let Some((at, voices)) = cache.get(provider_id) {
                if at.elapsed() < VOICE_CACHE_TTL {
                    return Ok(voices.clone());
                }
            }
        }
        let rec = self.store.require_tts_provider(provider_id).await?;
        let api_key = self.read_secret(rec.secret_key_ref.as_deref()).await?;
        let driver = self
            .tts_override
            .clone()
            .unwrap_or_else(|| driver_for_tts_kind(&rec.kind));
        let voices = driver.list_voices(&rec, api_key.as_deref()).await?;
        self.tts_voice_cache
            .lock()
            .await
            .insert(provider_id.to_string(), (Instant::now(), voices.clone()));
        Ok(voices)
    }

    pub async fn synthesize_tts(&self, req: SynthesizeTtsReq, user_id: &str) -> Result<TtsClip> {
        let provider_id = req
            .provider_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(BUILTIN_MACOS_SAY_ID);
        let provider = self.store.require_tts_provider(provider_id).await?;
        if !provider.enabled {
            return Err(Error::Conflict(format!(
                "TTS provider '{}' is disabled",
                provider.name
            )));
        }

        let text: String = req
            .text
            .chars()
            .filter(|c| *c != '\0')
            .collect::<String>()
            .trim()
            .to_string();
        if text.is_empty() {
            return Err(Error::Invalid("text cannot be empty".into()));
        }
        if text.chars().count() > MAX_TTS_TEXT_CHARS {
            return Err(Error::Invalid(format!(
                "text is at most {MAX_TTS_TEXT_CHARS} characters"
            )));
        }

        let voice_id = req.voice_id.trim();
        if voice_id.is_empty() || voice_id.len() > 128 {
            return Err(Error::Invalid("voice_id is required (≤ 128 chars)".into()));
        }
        let artifact_id = clean_opt_str(req.artifact_id);
        if let Some(aid) = &artifact_id {
            let a = self.design_service().store().require_artifact(aid).await?;
            if a.workspace_id != req.workspace_id {
                return Err(Error::Forbidden(
                    "artifact_id belongs to a different workspace".into(),
                ));
            }
        }

        let format = req
            .format
            .as_deref()
            .unwrap_or("wav")
            .trim()
            .to_ascii_lowercase();
        if !TTS_FORMATS.contains(&format.as_str()) {
            return Err(Error::Invalid(format!(
                "format must be one of: {}",
                TTS_FORMATS.join(", ")
            )));
        }
        let speed = req.speed.unwrap_or(1.0);
        if !speed.is_finite() || !(0.5..=2.0).contains(&speed) {
            return Err(Error::Invalid("speed must be between 0.5 and 2.0".into()));
        }

        let _permit = self
            .tts_permits
            .acquire()
            .await
            .map_err(|_| Error::Internal("TTS concurrency limiter closed".into()))?;

        let api_key = self.read_secret(provider.secret_key_ref.as_deref()).await?;
        let driver = self
            .tts_override
            .clone()
            .unwrap_or_else(|| driver_for_tts_kind(&provider.kind));

        let synth = driver
            .synthesize(
                &TtsSynthRequest {
                    voice_id: voice_id.to_string(),
                    text: text.clone(),
                    speed,
                    format: format.clone(),
                    model: clean_opt_str(req.model),
                    voice_settings: req.voice_settings,
                },
                &provider,
                api_key.as_deref(),
            )
            .await?;

        let clip_id = new_id();
        let file_name = format!("{clip_id}.{}", synth.format);
        let clip_path = self.write_clip_file(&file_name, &synth.bytes).await?;

        let title = clean_opt_str(req.title).unwrap_or_else(|| {
            let preview: String = text.chars().take(48).collect();
            if let Some(ch) = req
                .character_name
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                format!("{ch}: {preview}")
            } else {
                preview
            }
        });
        let mime = tts_mime_for_format(&synth.format)
            .unwrap_or(&synth.mime)
            .to_string();

        let rec = TtsClipRecord {
            clip: TtsClip {
                id: clip_id.clone(),
                workspace_id: req.workspace_id,
                story_id: clean_opt_str(req.story_id),
                artifact_id,
                provider_id: provider.id,
                provider_kind: provider.kind,
                voice_id: voice_id.to_string(),
                title,
                character_name: clean_opt_str(req.character_name),
                text,
                format: synth.format,
                mime,
                speed,
                size_bytes: synth.bytes.len() as i64,
                duration_ms: synth.duration_ms,
                audio_url: format!("/api/v1/product/tts/clips/{clip_id}/audio"),
                exported_path: None,
                visemes: synth.visemes,
                kind: "voice".into(),
                manifest: None,
                created_by: user_id.to_string(),
                created_at: Utc::now(),
            },
            file_name,
        };
        let saved = match self.store.insert_tts_clip(&rec).await {
            Ok(s) => s,
            Err(e) => {
                let _ = tokio::fs::remove_file(&clip_path).await;
                return Err(e);
            }
        };
        Ok(saved.clip)
    }

    /// Write a clip's audio into the clip store (atomically); returns its path.
    pub(crate) async fn write_clip_file(&self, file_name: &str, bytes: &[u8]) -> Result<PathBuf> {
        let dir = self.tts_clips_dir();
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| Error::Internal(format!("create TTS clips dir: {e}")))?;
        let clip_path = dir.join(file_name);
        let tmp_path = dir.join(format!(".{file_name}.tmp"));
        tokio::fs::write(&tmp_path, bytes)
            .await
            .map_err(|e| Error::Internal(format!("write TTS clip: {e}")))?;
        tokio::fs::rename(&tmp_path, &clip_path)
            .await
            .map_err(|e| Error::Internal(format!("persist TTS clip: {e}")))?;
        Ok(clip_path)
    }

    pub async fn read_tts_clip_bytes(&self, rec: &TtsClipRecord) -> Result<Vec<u8>> {
        let path = self.tts_clips_dir().join(&rec.file_name);
        tokio::fs::read(&path)
            .await
            .map_err(|e| Error::NotFound(format!("TTS audio file missing: {e}")))
    }

    pub async fn delete_tts_clip(&self, id: &str) -> Result<()> {
        let rec = self.store.delete_tts_clip(id).await?;
        let path = self.tts_clips_dir().join(&rec.file_name);
        let _ = tokio::fs::remove_file(&path).await;
        Ok(())
    }

    pub async fn export_tts_clip_to_workspace(
        &self,
        rec: &TtsClipRecord,
        workspace_root: &Path,
        relative_path: &str,
        overwrite: bool,
    ) -> Result<ExportWorkspaceResp> {
        let bytes = self.read_tts_clip_bytes(rec).await?;
        let resp = export_bytes_to_workspace(
            workspace_root,
            &self.data_dir,
            relative_path,
            ALLOWED_AUDIO_EXPORT_EXTS,
            &bytes,
            overwrite,
        )
        .await?;
        let _ = self
            .store
            .set_tts_clip_exported(&rec.clip.id, &resp.path)
            .await;
        Ok(resp)
    }
}

fn clean_provider_name(raw: &str) -> Result<String> {
    let clean: String = raw
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .to_string();
    if clean.is_empty() {
        return Err(Error::Invalid("provider name cannot be empty".into()));
    }
    if clean.chars().count() > MAX_PROVIDER_NAME_CHARS {
        return Err(Error::Invalid(format!(
            "provider name is at most {MAX_PROVIDER_NAME_CHARS} characters"
        )));
    }
    Ok(clean)
}

fn clean_opt_str(raw: Option<String>) -> Option<String> {
    raw.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// `auth_header` is sent as a header name: refused when saved, not on every call.
fn clean_auth_header(raw: Option<String>) -> Result<Option<String>> {
    let hdr = clean_opt_str(raw);
    if hdr
        .as_deref()
        .is_some_and(|h| reqwest::header::HeaderName::from_bytes(h.as_bytes()).is_err())
    {
        return Err(Error::Invalid(
            "auth_header must be an HTTP header name".into(),
        ));
    }
    Ok(hdr)
}

/// A local CLI is spawned by path, never looked up on `PATH` (launchd keeps
/// the daemon's minimal): a bare name like `forge` could never run.
fn clean_cli_binary(raw: Option<String>) -> Result<Option<String>> {
    let bin = clean_opt_str(raw);
    if bin.as_deref().is_some_and(|b| !Path::new(b).is_absolute()) {
        return Err(Error::Invalid(
            "cli_binary must be the executable's absolute path".into(),
        ));
    }
    Ok(bin)
}

fn clean_cli_args(args: Vec<String>) -> Result<Vec<String>> {
    if args.len() > MAX_CLI_ARGS {
        return Err(Error::Invalid(format!(
            "cli_args_template is limited to {MAX_CLI_ARGS} arguments"
        )));
    }
    for a in &args {
        if a.contains('\0') {
            return Err(Error::Invalid(
                "cli_args_template cannot contain NUL bytes".into(),
            ));
        }
    }
    Ok(args)
}

fn clean_params_object(val: Option<Value>) -> Result<Value> {
    match val {
        None => Ok(json!({})),
        Some(v) if v.is_object() => {
            let s = serde_json::to_string(&v)
                .map_err(|e| Error::Invalid(format!("invalid params JSON: {e}")))?;
            if s.len() > 64 * 1024 {
                return Err(Error::Invalid("params object exceeds 64 KiB".into()));
            }
            Ok(v)
        }
        Some(_) => Err(Error::Invalid("params must be a JSON object".into())),
    }
}

/// The share of the progress bar (`from..to` percent) each stage of a job fills.
/// A lone stage keeps its CLI's own percentages; a chain splits 20..95 by how
/// long each stage takes (Otto Forge on an M5 Pro: generate about 4 minutes,
/// rig 1, game-ready 20 s, animate 15 s).
pub(crate) fn stage_progress_spans(stages: &[String]) -> Vec<(i64, i64)> {
    if stages.len() == 1 {
        return vec![(0, 100)];
    }
    let weight = |s: &str| match s {
        "generate" => 16,
        "rig" => 4,
        "gameready" => 2,
        _ => 1,
    };
    let total: i64 = stages.iter().map(|s| weight(s)).sum();
    let (mut from, mut done) = (20, 0);
    stages
        .iter()
        .map(|s| {
            done += weight(s);
            let to = 20 + 75 * done / total;
            let span = (from, to);
            from = to;
            span
        })
        .collect()
}
