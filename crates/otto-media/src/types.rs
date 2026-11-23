//! Domain types, DTOs, and built-in catalogs for `otto-media` (3D generation
//! providers/jobs + built-in/extensible TTS + workspace-confined exports).
//! Mirrored in `ui/src/lib/api/types/media.ts`.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use otto_core::Id;
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------
// Constants & Vocabularies
// ---------------------------------------------------------------------------

pub const BUILTIN_OTTO_FORGE_ID: &str = "otto-forge";
pub const BUILTIN_TRELLIS2_ID: &str = "trellis-2";
pub const BUILTIN_HUNYUAN_ID: &str = "hunyuan3d-omni";
pub const BUILTIN_MACOS_SAY_ID: &str = "macos-say";

pub const HUNYUAN3D_LICENSE_WARNING: &str =
    "Tencent Hunyuan3D Community License excludes the EU, UK, and South Korea (including use of generated outputs). Review license terms before use.";

pub const MEDIA_3D_KINDS: &[&str] = &["hunyuan3d_omni", "trellis_2", "custom_http", "custom_cli"];
pub const MEDIA_3D_TRANSPORTS: &[&str] = &["http", "local_cli"];
pub const MEDIA_3D_FORMATS: &[&str] = &["glb", "gltf", "obj"];
pub const MEDIA_3D_CONTROLS: &[&str] = &["none", "point", "voxel", "bbox", "pose"];
pub const MEDIA_3D_PIPELINE_STAGES: &[&str] = &["generate", "gameready", "rig", "animate"];
/// At most this many stages chain after a job's first (`then_stages`).
pub const MAX_3D_THEN_STAGES: usize = 3;

pub const TTS_KINDS: &[&str] = &[
    "macos_say",
    "openai_compat",
    "elevenlabs",
    "kokoro",
    "custom_http",
];
pub const TTS_FORMATS: &[&str] = &["wav", "mp3", "m4a", "aiff"];

/// Default per-provider 3D output limit (128 MiB — accommodates rigged 4K PBR GLBs).
pub const DEFAULT_3D_MAX_OUTPUT_BYTES: i64 = 128 * 1024 * 1024;
/// Upper ceiling for configurable per-provider `max_output_bytes` (256 MiB).
pub const MAX_3D_OUTPUT_BYTES: usize = 256 * 1024 * 1024;
pub const MIN_3D_OUTPUT_BYTES: i64 = 1024 * 1024;
pub const MAX_TTS_AUDIO_BYTES: usize = 25 * 1024 * 1024;
pub const MAX_TTS_TEXT_CHARS: usize = 10_000;
pub const MAX_PROMPT_CHARS: usize = 4_000;
pub const MAX_PROVIDER_NAME_CHARS: usize = 120;
pub const MAX_CLI_ARGS: usize = 64;
pub const DEFAULT_3D_TIMEOUT_SECS: i64 = 900;
pub const MIN_3D_TIMEOUT_SECS: i64 = 15;
pub const MAX_3D_TIMEOUT_SECS: i64 = 3_600;
/// Built-in Otto Forge: a local `forge generate` (Pixal3D, finish, game-ready,
/// queued behind the shared heavy lock) measured up to ~26 min, so it gets the cap.
pub const OTTO_FORGE_TIMEOUT_SECS: i64 = MAX_3D_TIMEOUT_SECS;

pub fn secret_key_3d(provider_id: &str) -> String {
    format!("otto:media3d:{provider_id}:api_key")
}

pub fn secret_key_tts(provider_id: &str) -> String {
    format!("otto:tts:{provider_id}:api_key")
}

pub fn tts_mime_for_format(fmt: &str) -> Option<&'static str> {
    match fmt {
        "wav" => Some("audio/wav"),
        "mp3" => Some("audio/mpeg"),
        "m4a" => Some("audio/mp4"),
        "aiff" => Some("audio/aiff"),
        _ => None,
    }
}

/// Auto-detect local Otto Forge CLI (`forge` on `PATH`, or `~/otto-forge`).
/// Pure filesystem lookup — never spawns a shell or `which` subprocess.
pub fn detect_otto_forge_binary() -> Option<PathBuf> {
    find_otto_forge(
        std::env::var_os("PATH"),
        std::env::var_os("HOME").map(PathBuf::from),
    )
}

fn find_otto_forge(path_var: Option<std::ffi::OsString>, home: Option<PathBuf>) -> Option<PathBuf> {
    let on_path = path_var
        .iter()
        .flat_map(std::env::split_paths)
        .map(|dir| dir.join("forge"));
    let in_home = home.map(|h| h.join("otto-forge").join("bin").join("forge"));
    // Otto Forge's `bin/forge` sits beside `bin/forge-doctor`; a bare `forge` on
    // `PATH` without it is another tool (the Forge Code agent CLI, Foundry).
    on_path
        .chain(in_home)
        .find(|p| p.is_file() && p.with_file_name("forge-doctor").is_file())
}

// ---------------------------------------------------------------------------
// 3D Generation Providers & Presets
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Media3dCapabilities {
    pub text_to_3d: bool,
    pub image_to_3d: bool,
    pub multimodal_controls: Vec<String>,
    pub pbr_textures: bool,
    pub output_formats: Vec<String>,
    pub supports_poly_budget: bool,
    pub supports_seed: bool,
    pub supported_stages: Vec<String>,
}

pub fn capabilities_for_3d_provider(id: &str, kind: &str) -> Media3dCapabilities {
    if id == BUILTIN_OTTO_FORGE_ID {
        return Media3dCapabilities {
            text_to_3d: true,
            image_to_3d: true,
            multimodal_controls: vec!["none".into()],
            pbr_textures: true,
            // `forge` writes GLB only (forge/forge/cli.py).
            output_formats: vec!["glb".into()],
            supports_poly_budget: true,
            supports_seed: true,
            supported_stages: vec![
                "generate".into(),
                "gameready".into(),
                "rig".into(),
                "animate".into(),
            ],
        };
    }
    match kind {
        "hunyuan3d_omni" => Media3dCapabilities {
            text_to_3d: true,
            image_to_3d: true,
            multimodal_controls: vec![
                "none".into(),
                "point".into(),
                "voxel".into(),
                "bbox".into(),
                "pose".into(),
            ],
            pbr_textures: true,
            output_formats: vec!["glb".into(), "obj".into()],
            supports_poly_budget: true,
            supports_seed: true,
            supported_stages: vec!["generate".into()],
        },
        "trellis_2" => Media3dCapabilities {
            text_to_3d: false,
            image_to_3d: true,
            multimodal_controls: vec!["none".into()],
            pbr_textures: true,
            output_formats: vec!["glb".into()],
            supports_poly_budget: true,
            supports_seed: true,
            supported_stages: vec!["generate".into()],
        },
        _ => Media3dCapabilities {
            text_to_3d: true,
            image_to_3d: true,
            multimodal_controls: vec!["none".into()],
            pbr_textures: true,
            output_formats: vec!["glb".into(), "gltf".into(), "obj".into()],
            supports_poly_budget: true,
            supports_seed: true,
            supported_stages: vec![
                "generate".into(),
                "gameready".into(),
                "rig".into(),
                "animate".into(),
            ],
        },
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Media3dProvider {
    pub id: Id,
    pub name: String,
    pub kind: String,
    pub transport: String,
    pub builtin: bool,
    pub enabled: bool,
    pub ready: bool,
    pub status_detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license_warning: Option<String>,
    pub endpoint_url: Option<String>,
    pub cli_binary: Option<String>,
    pub cli_args_template: Vec<String>,
    pub cli_workdir: Option<String>,
    pub auth_header: Option<String>,
    pub has_api_key: bool,
    pub allow_local_network: bool,
    pub timeout_secs: i64,
    pub max_output_bytes: i64,
    pub capabilities: Media3dCapabilities,
    pub default_params: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Internal DB representation (keeps `secret_key_ref` private to backend).
#[derive(Debug, Clone)]
pub struct Media3dProviderRecord {
    pub id: Id,
    pub name: String,
    pub kind: String,
    pub transport: String,
    pub builtin: bool,
    pub enabled: bool,
    pub license_warning: Option<String>,
    pub endpoint_url: Option<String>,
    pub cli_binary: Option<String>,
    pub cli_args_template: Vec<String>,
    pub cli_workdir: Option<String>,
    pub auth_header: Option<String>,
    pub secret_key_ref: Option<String>,
    pub allow_local_network: bool,
    pub timeout_secs: i64,
    pub max_output_bytes: i64,
    pub default_params: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Media3dProviderRecord {
    /// Effective CLI binary path: explicit configured value, or auto-detected
    /// local `forge` binary for the built-in `otto-forge` preset.
    pub fn effective_cli_binary(&self) -> Option<String> {
        if let Some(bin) = self.cli_binary.as_deref().filter(|s| !s.trim().is_empty()) {
            return Some(bin.to_string());
        }
        if self.id == BUILTIN_OTTO_FORGE_ID {
            return detect_otto_forge_binary().map(|p| p.to_string_lossy().to_string());
        }
        None
    }

    pub fn effective_max_output_bytes(&self) -> usize {
        (self.max_output_bytes as usize).clamp(MIN_3D_OUTPUT_BYTES as usize, MAX_3D_OUTPUT_BYTES)
    }

    pub fn into_public(self, has_api_key: bool) -> Media3dProvider {
        let detected_bin = self.effective_cli_binary();
        let (ready, status_detail) = readiness_3d(&self, detected_bin.as_deref());
        let capabilities = capabilities_for_3d_provider(&self.id, &self.kind);
        Media3dProvider {
            id: self.id,
            name: self.name,
            kind: self.kind,
            transport: self.transport,
            builtin: self.builtin,
            enabled: self.enabled,
            ready,
            status_detail,
            license_warning: self.license_warning,
            endpoint_url: self.endpoint_url,
            cli_binary: detected_bin,
            cli_args_template: self.cli_args_template,
            cli_workdir: self.cli_workdir,
            auth_header: self.auth_header,
            has_api_key,
            allow_local_network: self.allow_local_network,
            timeout_secs: self.timeout_secs,
            max_output_bytes: self.max_output_bytes,
            capabilities,
            default_params: self.default_params,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

fn readiness_3d(rec: &Media3dProviderRecord, effective_bin: Option<&str>) -> (bool, String) {
    if !rec.enabled {
        return (false, "Disabled".into());
    }
    match rec.transport.as_str() {
        "http" => {
            if let Some(url) = rec.endpoint_url.as_deref().filter(|s| !s.trim().is_empty()) {
                (true, format!("HTTP endpoint configured ({url})"))
            } else {
                (
                    false,
                    "Configure an HTTP inference endpoint URL or switch to Local CLI".into(),
                )
            }
        }
        "local_cli" => {
            if let Some(bin) = effective_bin.filter(|s| !s.trim().is_empty()) {
                let path = std::path::Path::new(bin);
                if path.is_file() {
                    (true, format!("Local CLI ready ({bin})"))
                } else {
                    (false, format!("CLI executable not found at {bin}"))
                }
            } else if rec.id == BUILTIN_OTTO_FORGE_ID {
                (
                    false,
                    "Not installed — install Otto Forge (`forge` on PATH or ~/otto-forge) or set cli_binary".into(),
                )
            } else {
                (false, "Configure a local CLI executable path".into())
            }
        }
        _ => (false, "Unsupported transport".into()),
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateMedia3dProviderReq {
    pub name: String,
    pub kind: String,
    pub transport: String,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub license_warning: Option<String>,
    #[serde(default)]
    pub endpoint_url: Option<String>,
    #[serde(default)]
    pub cli_binary: Option<String>,
    #[serde(default)]
    pub cli_args_template: Option<Vec<String>>,
    #[serde(default)]
    pub cli_workdir: Option<String>,
    #[serde(default)]
    pub auth_header: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub allow_local_network: Option<bool>,
    #[serde(default)]
    pub timeout_secs: Option<i64>,
    #[serde(default)]
    pub max_output_bytes: Option<i64>,
    #[serde(default)]
    pub default_params: Option<Value>,
}

#[derive(Debug, Default, Deserialize)]
pub struct UpdateMedia3dProviderReq {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub transport: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub license_warning: Option<String>,
    #[serde(default)]
    pub endpoint_url: Option<String>,
    #[serde(default)]
    pub cli_binary: Option<String>,
    #[serde(default)]
    pub cli_args_template: Option<Vec<String>>,
    #[serde(default)]
    pub cli_workdir: Option<String>,
    #[serde(default)]
    pub auth_header: Option<String>,
    /// `None` = unchanged, `Some("")` = clear secret, `Some(val)` = set secret.
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub allow_local_network: Option<bool>,
    #[serde(default)]
    pub timeout_secs: Option<i64>,
    #[serde(default)]
    pub max_output_bytes: Option<i64>,
    #[serde(default)]
    pub default_params: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderTestResult {
    pub ok: bool,
    pub latency_ms: Option<u64>,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detected_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameAssetPreset {
    pub id: String,
    pub label: String,
    pub description: String,
    pub poly_budget: u32,
    pub texture_size: u32,
    pub resolution: u32,
    pub output_format: String,
}

pub fn builtin_game_asset_presets() -> Vec<GameAssetPreset> {
    vec![
        GameAssetPreset {
            id: "low_poly_prop".into(),
            label: "Low-Poly Prop".into(),
            description: "Mobile & stylized game props (5k tris, 1K PBR)".into(),
            poly_budget: 5_000,
            texture_size: 1024,
            resolution: 512,
            output_format: "glb".into(),
        },
        GameAssetPreset {
            id: "stylized_character".into(),
            label: "Stylized Character".into(),
            description: "Playable character / NPC mesh (25k tris, 2K PBR)".into(),
            poly_budget: 25_000,
            texture_size: 2048,
            resolution: 1024,
            output_format: "glb".into(),
        },
        GameAssetPreset {
            id: "environment_asset".into(),
            label: "Environment Asset".into(),
            description: "Architecture, foliage, or modular level piece (40k tris, 2K PBR)".into(),
            poly_budget: 40_000,
            texture_size: 2048,
            resolution: 1024,
            output_format: "glb".into(),
        },
        GameAssetPreset {
            id: "hero_mesh".into(),
            label: "Hero Mesh".into(),
            description: "High-detail hero weapon, vehicle, or showcase mesh (100k tris, 4K PBR)"
                .into(),
            poly_budget: 100_000,
            texture_size: 4096,
            resolution: 1536,
            output_format: "glb".into(),
        },
    ]
}

// ---------------------------------------------------------------------------
// 3D Generation & Pipeline Stage Jobs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct StartMedia3dJobReq {
    pub workspace_id: Id,
    #[serde(default)]
    pub project_id: Option<Id>,
    /// Pipeline stage: `generate` (default) | `gameready` | `rig` | `animate`.
    #[serde(default)]
    pub stage: Option<String>,
    #[serde(default)]
    pub source_artifact_id: Option<Id>,
    #[serde(default)]
    pub source_version_id: Option<Id>,
    #[serde(default)]
    pub target_artifact_id: Option<Id>,
    #[serde(default)]
    pub title: Option<String>,
    pub provider_id: Id,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub image_b64: Option<String>,
    #[serde(default)]
    pub image_mime: Option<String>,
    #[serde(default)]
    pub image_artifact_id: Option<Id>,
    #[serde(default)]
    pub output_format: Option<String>,
    #[serde(default)]
    pub preset: Option<String>,
    #[serde(default)]
    pub poly_budget: Option<u32>,
    #[serde(default)]
    pub seed: Option<i64>,
    #[serde(default)]
    pub params: Option<Value>,
    #[serde(default)]
    pub auto_save_artifact: Option<bool>,
    #[serde(default)]
    pub export_relative_path: Option<String>,
    /// Transform stages (`gameready` | `rig` | `animate`) run after `stage` in the same
    /// job, each on the previous stage's mesh and committing its own version.
    #[serde(default)]
    pub then_stages: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Media3dJob {
    pub id: Id,
    pub workspace_id: Id,
    pub project_id: Option<Id>,
    pub provider_id: Id,
    pub provider_kind: String,
    /// The stage running now (a chained job moves through `then_stages`).
    pub pipeline_stage: String,
    /// Stages chained after the first one (empty for a single-stage job).
    #[serde(default)]
    pub then_stages: Vec<String>,
    pub source_artifact_id: Option<Id>,
    pub source_version_id: Option<Id>,
    pub target_artifact_id: Option<Id>,
    pub title: String,
    pub prompt: Option<String>,
    pub output_format: String,
    pub preset: Option<String>,
    pub poly_budget: Option<i64>,
    pub seed: Option<i64>,
    pub status: String,
    pub progress_pct: i64,
    pub stage: String,
    pub artifact_id: Option<Id>,
    pub version_id: Option<Id>,
    pub exported_path: Option<String>,
    pub size_bytes: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest: Option<Value>,
    pub error: Option<String>,
    pub created_by: Id,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

// ---------------------------------------------------------------------------
// Workspace-Confined Export
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ExportWorkspaceReq {
    pub relative_path: String,
    #[serde(default)]
    pub version_id: Option<String>,
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportWorkspaceResp {
    pub path: String,
    pub absolute_path: String,
    pub size_bytes: u64,
    pub format: String,
    pub sha256: String,
}

// ---------------------------------------------------------------------------
// TTS Providers, Voices, Synthesis & Clips
// ---------------------------------------------------------------------------

pub fn supported_tts_formats(kind: &str) -> Vec<String> {
    match kind {
        "macos_say" => vec!["wav".into(), "m4a".into(), "aiff".into()],
        "elevenlabs" => vec!["mp3".into(), "wav".into()],
        _ => vec!["wav".into(), "mp3".into(), "m4a".into()],
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsProvider {
    pub id: Id,
    pub name: String,
    pub kind: String,
    pub builtin: bool,
    pub enabled: bool,
    pub ready: bool,
    pub status_detail: String,
    pub endpoint_url: Option<String>,
    pub default_model: Option<String>,
    pub default_voice: Option<String>,
    pub auth_header: Option<String>,
    pub has_api_key: bool,
    pub allow_local_network: bool,
    pub supported_formats: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct TtsProviderRecord {
    pub id: Id,
    pub name: String,
    pub kind: String,
    pub builtin: bool,
    pub enabled: bool,
    pub endpoint_url: Option<String>,
    pub default_model: Option<String>,
    pub default_voice: Option<String>,
    pub auth_header: Option<String>,
    pub secret_key_ref: Option<String>,
    pub allow_local_network: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl TtsProviderRecord {
    pub fn into_public(self, has_api_key: bool) -> TtsProvider {
        let (ready, status_detail) = readiness_tts(&self, has_api_key);
        let supported_formats = supported_tts_formats(&self.kind);
        TtsProvider {
            id: self.id,
            name: self.name,
            kind: self.kind,
            builtin: self.builtin,
            enabled: self.enabled,
            ready,
            status_detail,
            endpoint_url: self.endpoint_url,
            default_model: self.default_model,
            default_voice: self.default_voice,
            auth_header: self.auth_header,
            has_api_key,
            allow_local_network: self.allow_local_network,
            supported_formats,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

fn readiness_tts(rec: &TtsProviderRecord, has_api_key: bool) -> (bool, String) {
    if !rec.enabled {
        return (false, "Disabled".into());
    }
    match rec.kind.as_str() {
        "macos_say" => {
            if std::path::Path::new("/usr/bin/say").is_file() {
                (
                    true,
                    "Built-in macOS Speech Synthesis ready (offline)".into(),
                )
            } else {
                (
                    false,
                    "macOS /usr/bin/say not available on this host".into(),
                )
            }
        }
        "elevenlabs" => {
            if has_api_key {
                (true, "ElevenLabs API key stored in Keychain".into())
            } else {
                (
                    false,
                    "Add an ElevenLabs API key (stored in Keychain)".into(),
                )
            }
        }
        "openai_compat" | "kokoro" | "custom_http" => {
            if let Some(url) = rec.endpoint_url.as_deref().filter(|s| !s.trim().is_empty()) {
                (true, format!("HTTP TTS endpoint configured ({url})"))
            } else {
                (false, "Configure an HTTP TTS endpoint URL".into())
            }
        }
        _ => (false, "Unsupported TTS provider kind".into()),
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateTtsProviderReq {
    pub name: String,
    pub kind: String,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub endpoint_url: Option<String>,
    #[serde(default)]
    pub default_model: Option<String>,
    #[serde(default)]
    pub default_voice: Option<String>,
    #[serde(default)]
    pub auth_header: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub allow_local_network: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
pub struct UpdateTtsProviderReq {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub endpoint_url: Option<String>,
    #[serde(default)]
    pub default_model: Option<String>,
    #[serde(default)]
    pub default_voice: Option<String>,
    #[serde(default)]
    pub auth_header: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub allow_local_network: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsVoice {
    pub id: String,
    pub name: String,
    pub locale: Option<String>,
    pub gender: Option<String>,
    pub preview_text: Option<String>,
    pub provider_id: Id,
}

#[derive(Debug, Deserialize)]
pub struct SynthesizeTtsReq {
    pub workspace_id: Id,
    #[serde(default)]
    pub story_id: Option<Id>,
    #[serde(default)]
    pub artifact_id: Option<Id>,
    #[serde(default)]
    pub provider_id: Option<Id>,
    pub voice_id: String,
    pub text: String,
    #[serde(default)]
    pub speed: Option<f32>,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub character_name: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub voice_settings: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TtsVisemeCue {
    pub start: f64,
    pub end: f64,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TtsVisemeTimeline {
    pub duration_sec: f64,
    pub cues: Vec<TtsVisemeCue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsClip {
    pub id: Id,
    pub workspace_id: Id,
    pub story_id: Option<Id>,
    pub artifact_id: Option<Id>,
    pub provider_id: Id,
    pub provider_kind: String,
    pub voice_id: String,
    pub title: String,
    pub character_name: Option<String>,
    pub text: String,
    pub format: String,
    pub mime: String,
    pub speed: f32,
    pub size_bytes: i64,
    pub duration_ms: Option<i64>,
    pub audio_url: String,
    pub exported_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visemes: Option<TtsVisemeTimeline>,
    /// `voice` (speech), or `sound` / `music` from Otto Forge's audio stage.
    #[serde(default = "voice_kind")]
    pub kind: String,
    /// A sound or music clip's engine manifest (model, licence, loudness, loop seam).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest: Option<Value>,
    pub created_by: Id,
    pub created_at: DateTime<Utc>,
}

fn voice_kind() -> String {
    "voice".into()
}

#[derive(Debug, Clone)]
pub struct TtsClipRecord {
    pub clip: TtsClip,
    pub file_name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `forge generate` writes GLB only, so the built-in provider offers only
    /// `glb`; a custom CLI provider keeps every format.
    #[test]
    fn otto_forge_offers_glb_only() {
        let forge = capabilities_for_3d_provider(BUILTIN_OTTO_FORGE_ID, "custom_cli");
        assert_eq!(forge.output_formats, ["glb"]);
        let custom = capabilities_for_3d_provider("my-cli", "custom_cli");
        assert_eq!(custom.output_formats, MEDIA_3D_FORMATS);
    }

    /// Another tool named `forge` on `PATH` (the Forge Code agent CLI,
    /// Foundry) is not Otto Forge: detection falls through to ~/otto-forge.
    #[test]
    fn otto_forge_detection_skips_another_forge_on_path() {
        let root = tempfile::tempdir().unwrap();
        let other = root.path().join("local-bin");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("forge"), "").unwrap();
        let bin = root.path().join("home/otto-forge/bin");
        std::fs::create_dir_all(&bin).unwrap();
        for name in ["forge", "forge-doctor"] {
            std::fs::write(bin.join(name), "").unwrap();
        }
        let found = find_otto_forge(Some(other.into_os_string()), Some(root.path().join("home")));
        assert_eq!(found, Some(bin.join("forge")));
    }
}
