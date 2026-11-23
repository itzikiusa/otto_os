//! Sounds and music (docs/contracts/api.md "Sounds and music"): Otto Forge's
//! audio stage, `forge-sfx` (effects, ambience loops) and `forge-music`, found
//! beside the detected `forge` binary and run as a job like a 3D local CLI
//! (argv only, JSON Lines progress, timeout, process-group cancel; one at a
//! time, beside Smart mode's 3D work). The WAV is stored as a clip of
//! kind `sound` or `music`, so every voice-clip player and export works, with
//! the Forge manifest (model, licence, loudness, loop seam) as provenance.

pub mod http;
mod store;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use otto_core::cancel::{CancelGuard, CancelRegistryExt};
use otto_core::{new_id, Error, Id, Result};
use serde::{Deserialize, Serialize};

use crate::gen3d::{read_optional_manifest, run_cli_argv};
use crate::service::MediaService;
use crate::tts::wav_duration_ms;
use crate::types::{TtsClip, TtsClipRecord, BUILTIN_OTTO_FORGE_ID};

pub const AUDIO_KINDS: &[&str] = &["sound", "music"];
pub const MAX_AUDIO_PROMPT_CHARS: usize = 512;
/// A 590 s stereo 48 kHz 16-bit track is about 113 MB.
pub const MAX_AUDIO_CLIP_BYTES: usize = 128 * 1024 * 1024;
/// An effect takes about 90 s and long music under two minutes; the rest is
/// room for a model load on a busy machine.
const AUDIO_TIMEOUT: Duration = Duration::from_secs(20 * 60);
pub const SOUND_ENGINE_MISSING: &str = "The sound engine is not installed. In the Otto Forge folder run ./install.sh --stage audio --with-models.";

/// The engine's length limits in seconds, `(min, max)` (docs/audio.md in Otto
/// Forge): an effect is one 30 s window and a loop needs 3.5 s of it; music
/// runs 10-590 s and a loop needs margin.
pub fn audio_limits(kind: &str, looped: bool) -> (f64, f64) {
    match (kind, looped) {
        ("music", true) => (10.0, 500.0),
        ("music", false) => (10.0, 590.0),
        (_, true) => (0.5, 26.5),
        _ => (0.5, 30.0),
    }
}

/// `GET /product/audio/engine`: `ready` or `not_installed` (a state, never an error).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioEngine {
    pub state: String,
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct StartAudioJobReq {
    pub workspace_id: Id,
    pub kind: String,
    pub prompt: String,
    pub seconds: f64,
    #[serde(rename = "loop", default)]
    pub looped: bool,
    #[serde(default)]
    pub bpm: Option<i64>,
    #[serde(default)]
    pub seed: Option<i64>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub story_id: Option<Id>,
    #[serde(default)]
    pub artifact_id: Option<Id>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioJob {
    pub id: Id,
    pub workspace_id: Id,
    pub kind: String,
    pub title: String,
    pub prompt: String,
    pub seconds: f64,
    #[serde(rename = "loop")]
    pub looped: bool,
    pub bpm: Option<i64>,
    pub seed: Option<i64>,
    pub story_id: Option<Id>,
    pub artifact_id: Option<Id>,
    /// queued | running | succeeded | failed | cancelled | not_installed
    pub status: String,
    pub progress_pct: i64,
    /// The progress message (the engine's own while running).
    pub stage: String,
    pub clip_id: Option<Id>,
    pub error: Option<String>,
    pub created_by: Id,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    /// The made clip (`GET …/jobs/{id}` of a succeeded job only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip: Option<TtsClip>,
}

fn clean(s: Option<String>) -> Option<String> {
    s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

impl MediaService {
    /// `forge-sfx` (sound), `forge-music` or `forge-voice` (voice) beside the built-in Otto Forge's binary
    /// (its configured `cli_binary`, else the detected `forge` with `forge-doctor`).
    pub async fn audio_command(&self, kind: &str) -> Option<PathBuf> {
        let forge = self
            .store()
            .get_3d_provider(BUILTIN_OTTO_FORGE_ID)
            .await
            .ok()
            .flatten()?
            .effective_cli_binary()?;
        let name = match kind {
            "music" => "forge-music",
            "voice" => "forge-voice",
            _ => "forge-sfx",
        };
        let cmd = Path::new(&forge).with_file_name(name);
        cmd.is_file().then_some(cmd)
    }

    pub async fn audio_engine(&self) -> AudioEngine {
        let ready = self.audio_command("sound").await.is_some()
            && self.audio_command("music").await.is_some();
        let (state, message) = match ready {
            true => ("ready", "Otto Forge makes sounds and music on this Mac."),
            false => ("not_installed", SOUND_ENGINE_MISSING),
        };
        AudioEngine {
            state: state.into(),
            message: message.into(),
        }
    }

    /// Check a request and queue its job (202), or record it as
    /// `not_installed` when the engine's command is missing.
    pub async fn start_audio_job(
        self: &Arc<Self>,
        req: StartAudioJobReq,
        user_id: &str,
    ) -> Result<AudioJob> {
        let kind = req.kind.trim().to_ascii_lowercase();
        if !AUDIO_KINDS.contains(&kind.as_str()) {
            return Err(Error::Invalid("kind must be sound or music".into()));
        }
        let prompt: String = req.prompt.replace('\0', "").trim().to_string();
        if prompt.is_empty() || prompt.chars().count() > MAX_AUDIO_PROMPT_CHARS {
            return Err(Error::Invalid(format!(
                "prompt must be 1-{MAX_AUDIO_PROMPT_CHARS} characters"
            )));
        }
        let (lo, hi) = audio_limits(&kind, req.looped);
        if !(req.seconds.is_finite() && (lo..=hi).contains(&req.seconds)) {
            let what = if req.looped { "looping " } else { "" };
            return Err(Error::Invalid(format!(
                "seconds must be {lo}-{hi} for {what}{kind}"
            )));
        }
        if req.bpm.is_some() && kind != "music" {
            return Err(Error::Invalid("bpm is for music only".into()));
        }
        if req.bpm.is_some_and(|b| !(30..=300).contains(&b)) {
            return Err(Error::Invalid("bpm must be 30-300".into()));
        }
        if req.seed.is_some_and(|s| s < 0) {
            return Err(Error::Invalid("seed must be 0 or more".into()));
        }
        let artifact_id = clean(req.artifact_id);
        if let Some(aid) = &artifact_id {
            let a = self.design_service().store().require_artifact(aid).await?;
            if a.workspace_id != req.workspace_id {
                return Err(Error::Forbidden(
                    "artifact_id belongs to a different workspace".into(),
                ));
            }
        }
        let cmd = self.audio_command(&kind).await;
        let now = Utc::now();
        let job = AudioJob {
            id: new_id(),
            workspace_id: req.workspace_id,
            title: clean(req.title).unwrap_or_else(|| prompt.chars().take(48).collect()),
            kind,
            prompt,
            seconds: req.seconds,
            looped: req.looped,
            bpm: req.bpm,
            seed: req.seed,
            story_id: clean(req.story_id),
            artifact_id,
            status: if cmd.is_some() {
                "queued"
            } else {
                "not_installed"
            }
            .into(),
            progress_pct: 0,
            stage: if cmd.is_some() {
                "Queued"
            } else {
                SOUND_ENGINE_MISSING
            }
            .into(),
            clip_id: None,
            error: None,
            created_by: user_id.to_string(),
            created_at: now,
            updated_at: now,
            finished_at: cmd.is_none().then_some(now),
            clip: None,
        };
        self.store().insert_audio_job(&job).await?;
        if let Some(cmd) = cmd {
            let guard = self.gen3d_cancels.register(&job.id);
            let (svc, run) = (Arc::clone(self), job.clone());
            tokio::spawn(async move { svc.run_audio_job(run, cmd, guard).await });
        }
        Ok(job)
    }

    async fn run_audio_job(self: Arc<Self>, job: AudioJob, cmd: PathBuf, guard: CancelGuard) {
        let _turn = self.audio_turn.clone().acquire_owned().await;
        if guard.is_cancelled() {
            return; // the cancel already wrote the row
        }
        let store = self.store().clone();
        let fail = |e: String| {
            let store = store.clone();
            let id = job.id.clone();
            async move {
                let _ = store
                    .finish_audio_job(&id, "failed", "Failed", None, Some(&e))
                    .await;
            }
        };
        let _ = store
            .update_audio_job_progress(&job.id, 2, "Starting the sound engine")
            .await;
        let tmp = match tempfile::tempdir() {
            Ok(t) => t,
            Err(e) => return fail(format!("create a work folder: {e}")).await,
        };
        let out = tmp.path().join("audio.wav");
        let mut args = vec![
            "--prompt".to_string(),
            job.prompt.clone(),
            "--seconds".into(),
            job.seconds.to_string(),
            "--output".into(),
            out.to_string_lossy().to_string(),
        ];
        if job.looped {
            args.push("--loop".into());
        }
        if let Some(bpm) = job.bpm {
            args.extend(["--bpm".into(), bpm.to_string()]);
        }
        if let Some(seed) = job.seed {
            args.extend(["--seed".into(), seed.to_string()]);
        }

        // The engine's stderr progress reaches the row at most once a second.
        let (tx, mut rx) = tokio::sync::watch::channel::<Option<(i64, String, String)>>(None);
        let (writer_store, id) = (store.clone(), job.id.clone());
        let writer = tokio::spawn(async move {
            while rx.changed().await.is_ok() {
                let latest = rx.borrow_and_update().clone();
                if let Some((pct, label, _)) = latest {
                    let _ = writer_store
                        .update_audio_job_progress(&id, pct, &label)
                        .await;
                }
                tokio::time::sleep(crate::service::CLI_PROGRESS_FLUSH).await;
            }
        });
        let ran = run_cli_argv(&cmd, &args, None, AUDIO_TIMEOUT, guard.flag(), Some(&tx)).await;
        drop(tx);
        let _ = writer.await;
        if guard.is_cancelled() {
            return;
        }
        if let Err(e) = ran {
            return fail(e.public_message()).await;
        }
        let bytes = match tokio::fs::read(&out).await {
            Ok(b) if b.len() > MAX_AUDIO_CLIP_BYTES => {
                return fail("the sound file is over 128 MB".into()).await
            }
            Ok(b) if b.len() >= 44 && &b[0..4] == b"RIFF" && &b[8..12] == b"WAVE" => b,
            _ => return fail("the sound engine wrote no WAV file".into()).await,
        };
        let manifest = read_optional_manifest(&out).await;
        match self.save_audio_clip(&job, &cmd, bytes, manifest).await {
            Ok(clip_id) => {
                let _ = store
                    .finish_audio_job(&job.id, "succeeded", "Done", Some(&clip_id), None)
                    .await;
            }
            Err(e) => fail(e.public_message()).await,
        }
    }

    async fn save_audio_clip(
        &self,
        job: &AudioJob,
        cmd: &Path,
        bytes: Vec<u8>,
        manifest: Option<serde_json::Value>,
    ) -> Result<Id> {
        let clip_id = new_id();
        let file_name = format!("{clip_id}.wav");
        let path = self.write_clip_file(&file_name, &bytes).await?;
        let rec = TtsClipRecord {
            clip: TtsClip {
                id: clip_id.clone(),
                workspace_id: job.workspace_id.clone(),
                story_id: job.story_id.clone(),
                artifact_id: job.artifact_id.clone(),
                provider_id: BUILTIN_OTTO_FORGE_ID.into(),
                provider_kind: "otto_forge".into(),
                voice_id: cmd
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default(),
                title: job.title.clone(),
                character_name: None,
                text: job.prompt.clone(),
                format: "wav".into(),
                mime: "audio/wav".into(),
                speed: 1.0,
                size_bytes: bytes.len() as i64,
                duration_ms: wav_duration_ms(&bytes),
                audio_url: format!("/api/v1/product/tts/clips/{clip_id}/audio"),
                exported_path: None,
                visemes: None,
                kind: job.kind.clone(),
                manifest,
                created_by: job.created_by.clone(),
                created_at: Utc::now(),
            },
            file_name,
        };
        if let Err(e) = self.store().insert_tts_clip(&rec).await {
            let _ = tokio::fs::remove_file(&path).await;
            return Err(e);
        }
        Ok(clip_id)
    }

    /// The job, with its clip once it succeeded.
    pub async fn get_audio_job(&self, id: &str) -> Result<AudioJob> {
        let mut job = self.store().require_audio_job(id).await?;
        if let Some(cid) = &job.clip_id {
            job.clip = self.store().get_tts_clip(cid).await?.map(|r| r.clip);
        }
        Ok(job)
    }

    /// Stop a queued or running job: its engine process group is stopped.
    pub async fn cancel_audio_job(&self, id: &str) -> Result<AudioJob> {
        let job = self.store().require_audio_job(id).await?;
        if !matches!(job.status.as_str(), "queued" | "running") {
            return Ok(job);
        }
        self.gen3d_cancels.signal(id);
        self.store()
            .finish_audio_job(id, "cancelled", "Cancelled", None, None)
            .await?;
        self.get_audio_job(id).await
    }
}
