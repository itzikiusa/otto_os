// Contract mirror: otto-media (3D generation providers/jobs + TTS synthesis/clips
// + workspace-confined asset export). Part of lib/api/types (re-exported via ../types.ts).

import type { Id } from './core';
import type { DesignArtifact } from './design';

// ── 3D Generation Providers & Jobs (`/api/v1/design/3d/*`) ──────────────────

export type Media3dProviderKind =
  | 'hunyuan3d_omni'
  | 'trellis_2'
  | 'custom_http'
  | 'custom_cli';

export type Media3dTransport = 'http' | 'local_cli';

export type Media3dOutputFormat = 'glb' | 'gltf' | 'obj';

export type Media3dControlType = 'none' | 'point' | 'voxel' | 'bbox' | 'pose';

export type Media3dPipelineStage = 'generate' | 'gameready' | 'rig' | 'animate';

export interface Media3dCapabilities {
  text_to_3d: boolean;
  image_to_3d: boolean;
  multimodal_controls: Media3dControlType[];
  pbr_textures: boolean;
  output_formats: Media3dOutputFormat[];
  supports_poly_budget: boolean;
  supports_seed: boolean;
  /** Pipeline stages this provider supports (`generate`, `gameready`, `rig`, `animate`). */
  supported_stages: Media3dPipelineStage[];
}

export interface Media3dProvider {
  id: Id;
  name: string;
  kind: Media3dProviderKind;
  transport: Media3dTransport;
  builtin: boolean;
  enabled: boolean;
  /** True when the provider has enough configuration (endpoint or CLI binary) to run. */
  ready: boolean;
  /** Short human-readable readiness note (e.g. "Set endpoint URL or local CLI path"). */
  status_detail: string;
  /** Optional license or territory restriction notice (e.g. Hunyuan3D EU/UK/KR exclusion). */
  license_warning?: string | null;
  endpoint_url: string | null;
  cli_binary: string | null;
  cli_args_template: string[];
  cli_workdir: string | null;
  auth_header: string | null;
  /** True when an API key is stored in the OS Keychain; raw secrets are never returned. */
  has_api_key: boolean;
  /** Admin opt-in allowing loopback/RFC1918/CGNAT HTTP endpoints (metadata + Otto port always blocked). */
  allow_local_network: boolean;
  timeout_secs: number;
  /** Max output asset size in bytes (default 128 MiB, configurable up to 256 MiB). */
  max_output_bytes: number;
  capabilities: Media3dCapabilities;
  default_params: Record<string, unknown>;
  created_at: string;
  updated_at: string;
}

export interface CreateMedia3dProviderReq {
  name: string;
  kind: Media3dProviderKind;
  transport: Media3dTransport;
  enabled?: boolean;
  license_warning?: string | null;
  endpoint_url?: string | null;
  cli_binary?: string | null;
  cli_args_template?: string[];
  cli_workdir?: string | null;
  auth_header?: string | null;
  /** Stored write-only in Keychain; never persisted in SQLite. */
  api_key?: string | null;
  allow_local_network?: boolean;
  timeout_secs?: number;
  max_output_bytes?: number;
  default_params?: Record<string, unknown>;
}

export interface UpdateMedia3dProviderReq {
  name?: string;
  transport?: Media3dTransport;
  enabled?: boolean;
  license_warning?: string | null;
  endpoint_url?: string | null;
  cli_binary?: string | null;
  cli_args_template?: string[];
  cli_workdir?: string | null;
  auth_header?: string | null;
  /** Omitted = keep existing; `""` = delete stored key; non-empty = replace. */
  api_key?: string | null;
  allow_local_network?: boolean;
  timeout_secs?: number;
  max_output_bytes?: number;
  default_params?: Record<string, unknown>;
}

export interface ProviderTestResult {
  ok: boolean;
  latency_ms: number | null;
  message: string;
  detected_version?: string | null;
  details?: Record<string, unknown> | null;
}

export interface GameAssetPreset {
  id: string;
  label: string;
  description: string;
  poly_budget: number;
  texture_size: number;
  resolution: number;
  output_format: Media3dOutputFormat;
}

export type Media3dJobStatus =
  | 'queued'
  | 'running'
  | 'succeeded'
  | 'failed'
  | 'cancelled';

export interface StartMedia3dJobReq {
  workspace_id: Id;
  project_id?: Id | null;
  /** Pipeline stage (`generate` default, or `gameready` / `rig` / `animate` on an existing mesh). */
  stage?: Media3dPipelineStage;
  /** Source 3D artifact to transform for `gameready` / `rig` / `animate` stages. */
  source_artifact_id?: Id | null;
  /** Specific version of `source_artifact_id` (defaults to head). */
  source_version_id?: Id | null;
  /** When set, commits the generated 3D mesh as a new version on this Design Hall artifact. */
  target_artifact_id?: Id | null;
  title?: string | null;
  provider_id: Id;
  prompt?: string | null;
  /** Base64-encoded PNG/JPEG/WebP reference image. */
  image_b64?: string | null;
  image_mime?: string | null;
  /** Existing Design Hall graphics artifact to use as reference image. */
  image_artifact_id?: Id | null;
  output_format?: Media3dOutputFormat;
  /** Game-asset preset id (e.g. `low_poly_prop`, `stylized_character`, `environment_asset`, `hero_mesh`). */
  preset?: string | null;
  /** Target polygon/triangle budget (passed as `decimation_target` for TRELLIS.2 / `{poly_budget}` for CLI). */
  poly_budget?: number | null;
  seed?: number | null;
  /** Provider-specific parameters:
   *  - Hunyuan3D-Omni: `control_type` (`none`|`point`|`voxel`|`bbox`|`pose`), `control_input_b64`,
   *    `control_filename`, `bbox` (`[w, h, d]` or 6 floats), `use_ema`, `flashvdm`, `steps`, `guidance_scale`
   *  - TRELLIS.2: `resolution` (`512`|`1024`|`1536`), `steps`, `guidance_scale`, `decimation_target`,
   *    `texture_size` (`1024`|`2048`|`4096`), `remesh`, `pbr`
   */
  params?: Record<string, unknown>;
  /** Automatically save output mesh into Design Hall (default true). */
  auto_save_artifact?: boolean;
  /** Transform stages run after `stage` in the same job, each on the previous stage's mesh
   *  and committing its own version (e.g. `['rig', 'animate']` after `generate`). */
  then_stages?: Media3dPipelineStage[];
  /** Optional workspace-relative path to also export the output mesh immediately. */
  export_relative_path?: string | null;
}

export interface Media3dJob {
  id: Id;
  workspace_id: Id;
  project_id: Id | null;
  provider_id: Id;
  provider_kind: Media3dProviderKind;
  /** The stage running now (a chained job moves through `then_stages`). */
  pipeline_stage: Media3dPipelineStage;
  /** Stages chained after the first one (empty for a single-stage job). */
  then_stages?: Media3dPipelineStage[];
  source_artifact_id: Id | null;
  source_version_id: Id | null;
  target_artifact_id: Id | null;
  title: string;
  prompt: string | null;
  output_format: Media3dOutputFormat;
  preset: string | null;
  poly_budget: number | null;
  seed: number | null;
  status: Media3dJobStatus;
  progress_pct: number;
  stage: string;
  artifact_id: Id | null;
  version_id: Id | null;
  exported_path: string | null;
  size_bytes: number | null;
  manifest?: Record<string, unknown> | null;
  /** Why a job failed; on a `succeeded` job, why its `export_relative_path` export failed. */
  error: string | null;
  created_by: Id;
  created_at: string;
  updated_at: string;
  finished_at: string | null;
}

// ── Workspace-Confined Asset Export ─────────────────────────────────────────

export interface ExportWorkspaceReq {
  /** Workspace-relative path (e.g. `assets/models/shield.glb` or `assets/audio/hero.wav`). */
  relative_path: string;
  /** Optional Design Hall version id / `vN` (3D artifact export only; defaults to head). */
  version_id?: string | null;
  /** Overwrite an existing file at `relative_path` (default false → 409). */
  overwrite?: boolean;
}

export interface ExportWorkspaceResp {
  /** Canonical workspace-relative path that was written. */
  path: string;
  /** Absolute path inside the workspace root. */
  absolute_path: string;
  size_bytes: number;
  format: string;
  sha256: string;
}

// ── Built-in & Extensible TTS (`/api/v1/product/tts/*`) ─────────────────────

export type TtsProviderKind =
  | 'macos_say'
  | 'openai_compat'
  | 'elevenlabs'
  | 'kokoro'
  | 'custom_http';

export type TtsFormat = 'wav' | 'mp3' | 'm4a' | 'aiff';

export interface TtsProvider {
  id: Id;
  name: string;
  kind: TtsProviderKind;
  builtin: boolean;
  enabled: boolean;
  ready: boolean;
  status_detail: string;
  endpoint_url: string | null;
  default_model: string | null;
  default_voice: string | null;
  auth_header: string | null;
  has_api_key: boolean;
  allow_local_network: boolean;
  supported_formats: TtsFormat[];
  created_at: string;
  updated_at: string;
}

export interface CreateTtsProviderReq {
  name: string;
  kind: Exclude<TtsProviderKind, 'macos_say'>;
  enabled?: boolean;
  endpoint_url?: string | null;
  default_model?: string | null;
  default_voice?: string | null;
  auth_header?: string | null;
  api_key?: string | null;
  allow_local_network?: boolean;
}

export interface UpdateTtsProviderReq {
  name?: string;
  enabled?: boolean;
  endpoint_url?: string | null;
  default_model?: string | null;
  default_voice?: string | null;
  auth_header?: string | null;
  /** Omitted = keep existing; `""` = delete stored key; non-empty = replace. */
  api_key?: string | null;
  allow_local_network?: boolean;
}

export interface TtsVoice {
  id: string;
  name: string;
  locale: string | null;
  gender: string | null;
  preview_text: string | null;
  provider_id: Id;
}

export interface SynthesizeTtsReq {
  workspace_id: Id;
  story_id?: Id | null;
  artifact_id?: Id | null;
  /** Defaults to `"macos-say"` when omitted. */
  provider_id?: Id | null;
  voice_id: string;
  /** Plain text to synthesize (1..=10,000 chars). */
  text: string;
  /** Speech rate multiplier (0.5..=2.0, default 1.0). */
  speed?: number | null;
  /** Output format (`wav` default; macOS `say` supports `wav`, `m4a`, `aiff`). */
  format?: TtsFormat;
  title?: string | null;
  character_name?: string | null;
  model?: string | null;
  voice_settings?: Record<string, unknown> | null;
}

export interface TtsVisemeCue {
  start: number;
  end: number;
  value: string;
}

export interface TtsVisemeTimeline {
  duration_sec: number;
  cues: TtsVisemeCue[];
}

export interface TtsClip {
  id: Id;
  workspace_id: Id;
  story_id: Id | null;
  artifact_id: Id | null;
  provider_id: Id;
  /** `otto_forge` for a sound or music clip. */
  provider_kind: TtsProviderKind | 'otto_forge';
  /** A sound or music clip: the command (`forge-sfx` | `forge-music`). */
  voice_id: string;
  title: string;
  character_name: string | null;
  text: string;
  format: TtsFormat;
  mime: string;
  speed: number;
  size_bytes: number;
  duration_ms: number | null;
  /** Relative API URL (`/api/v1/product/tts/clips/{id}/audio`). */
  audio_url: string;
  exported_path: string | null;
  /** Optional lip-sync viseme timeline for 3D character animation. */
  visemes?: TtsVisemeTimeline | null;
  /** `voice` for speech; `sound` / `music` from Otto Forge's audio stage. */
  kind: MediaClipKind;
  /** Sound and music only: the engine's manifest (model, licence, loudness, `loop.seam_ok`). */
  manifest?: Record<string, unknown> | null;
  created_by: Id;
  created_at: string;
}

// ── Sounds and music (`/api/v1/product/audio/*`, docs/contracts/api.md) ──────

/** What a stored clip is: speech (TTS), or a sound / music track from Otto Forge's audio stage. */
export type MediaClipKind = 'voice' | 'sound' | 'music';
export type AudioKind = 'sound' | 'music';
export type AudioJobStatus = 'queued' | 'running' | 'succeeded' | 'failed' | 'cancelled' | 'not_installed';

/** `GET /product/audio/engine`: a state, never an error. */
export interface AudioEngine {
  state: 'ready' | 'not_installed';
  /** Plain words; when missing, how to install it. */
  message: string;
}

/** Seconds: sound 0.5–30 (loop ≤ 26.5); music 10–590 (loop ≤ 500). */
export interface StartAudioJobReq {
  workspace_id: Id;
  kind: AudioKind;
  /** 1–512 characters: the sound, or the music brief (mood, tempo, instruments). */
  prompt: string;
  seconds: number;
  /** A seamless loop (an ambience bed for a sound). */
  loop?: boolean;
  /** Music only, 30–300. */
  bpm?: number | null;
  seed?: number | null;
  title?: string | null;
  story_id?: Id | null;
  artifact_id?: Id | null;
}

export interface AudioJob {
  id: Id;
  workspace_id: Id;
  kind: AudioKind;
  title: string;
  prompt: string;
  seconds: number;
  loop: boolean;
  bpm: number | null;
  seed: number | null;
  story_id: Id | null;
  artifact_id: Id | null;
  status: AudioJobStatus;
  progress_pct: number;
  /** The progress message (the engine's own while running; the install hint when `not_installed`). */
  stage: string;
  clip_id: Id | null;
  error: string | null;
  created_by: Id;
  created_at: string;
  updated_at: string;
  finished_at: string | null;
  /** `GET …/jobs/{id}` of a succeeded job: the stored clip. */
  clip?: TtsClip;
}

// ── Smart mode: productions (contract v1, docs/contracts/api.md) ─────────────

export type ProductionUse = 'phone' | 'pc' | 'closeup';
export type ProductionStatus = 'planning' | 'ready' | 'running' | 'done' | 'failed' | 'cancelled';
export type ProductionItemKind = 'character' | 'object' | 'voice_line' | 'sound' | 'music';
export type ProductionItemStatus = 'waiting' | 'working' | 'done' | 'failed' | 'skipped';
export type ProductionSocket = 'right_hand' | 'left_hand' | 'back' | 'hip_left' | 'hip_right' | 'neck';

export interface ProductionAttach {
  item: string;
  where: string;
}

export interface ProductionItemDetails {
  effect?: 'none' | 'rain_on_glass' | 'steam';
  prompt?: string;
  /** Plain description of the voice. */
  voice?: string;
  /** Item id of the character who speaks. */
  speaker?: string;
  text?: string;
  nonverbal?: boolean;
  loop?: boolean;
  /** Music only: the performance plays through once when the scene is played. */
  featured?: boolean;
  seconds?: number;
  /** Real size in metres: a character's height, an object's longest side. */
  size_m?: number;
  /** Engine moves: idle | walk | run. */
  moves?: string[];
  attach_to?: ProductionAttach;
  /** Character only: props (each its own object item) and where they are held or worn. */
  holds?: { item: string; socket: ProductionSocket }[];
  /** Character only: interaction template ids (shoot_bow, swing_sword, pull_sword). */
  actions?: string[];
  /** [width, height, depth] in metres, when the piece's proportions matter. */
  dims_m?: [number, number, number];
  /** Voice line only: slow | normal | fast. */
  pace?: 'slow' | 'normal' | 'fast';
  /** Voice line only: a few plain words ("tired, kind"). */
  mood?: string;
  /** Voice line only: the id of the line it answers (the conversation's order). */
  replies_to?: string;
  /** Character only: what its body is, so the scene can pose and place it. */
  body?: 'two_legs' | 'four_legs' | 'bird' | 'fish' | 'no_legs' | 'many_legs' | 'wheels';
  /** A character or object that IS a made piece of the production's project: it goes into the scene as it is, nothing is made. */
  reuse?: ProductionReuse;
  /** Character only: its pose in this scene (made by the engine's `forge pose`). */
  pose?: ProductionPose;
}

/** A character's pose: sit on a seat, kneel, hold a palm out, look at something. */
export interface ProductionPose {
  kind: 'stand' | 'sit' | 'kneel' | 'crouch' | 'hold_out' | 'lie' | 'curl' | 'lean' | 'peek';
  arms?: 'crossed';
  play?: string;
  sing_into?: string;
  plant?: string;
  /** The seat it sits on (an item id). */
  on?: string;
  /** What it looks at: an item id, or "camera". */
  look_at?: string;
  hand?: 'left' | 'right';
  /** A fist round what it holds (from its holds; set by the server). */
  grip?: { hand: 'left' | 'right'; style: 'raise' | 'carry' | 'wind' | 'cup'; instrument?: string; toward?: string; other?: { style: 'raise' | 'carry'; toward?: string | null } };
}

/** Which made piece of the project an item is (`artifact_id`, `version_id` are what a client sends; the rest the server fills in). */
export interface ProductionReuse {
  artifact_id: Id;
  version_id?: Id;
  /** The creation it was made in, its item there (its picture: `productionPreviewUrl`), and its title ("From …"). */
  production_id?: Id;
  item_id?: string;
  from?: string;
  /** Its name there (its voice is designed from it). */
  name?: string;
  rig?: { body_plan?: string; legs?: number; forward?: [number, number, number] };
  /** A line it spoke there (a voice engine that picks voices speaks its new lines in that voice). */
  line?: Id;
}

/** A made piece of a project, as it was planned; `details.reuse` says which model it is and where it was made. */
export interface ProductionPiece {
  kind: 'character' | 'object';
  name: string;
  description: string;
  details: ProductionItemDetails;
}

export interface ProductionItemResult {
  artifact_id?: Id;
  version_id?: Id;
  clip_id?: Id;
  /** What the engine's rig says the skeleton is (absent: no rig or nothing said). */
  rig?: { body_plan?: string; legs?: number; forward?: [number, number, number] };
  /** This scene's posed model (the made one stays unposed), or why the engine could not pose it. */
  pose?: { spec: ProductionPose; artifact_id?: Id; version_id?: Id; manifest?: Record<string, unknown>; failed?: string };
}

export interface ProductionItem {
  id: string;
  kind: ProductionItemKind;
  name: string;
  /** One plain sentence. */
  description: string;
  enabled: boolean;
  details: ProductionItemDetails;
  status: ProductionItemStatus;
  progress_pct: number;
  message?: string;
  result?: ProductionItemResult;
  /** The result before the last steering round redid this item. */
  previous_result?: ProductionItemResult;
  /** While working: which step of how many, in plain words ("Drawing the picture", "Adding a skeleton"…). */
  step?: ProductionItemStep;
  /** RFC 3339: when it last started working. */
  working_since?: string;
  /** Next, but waiting for the engine's turn (another job holds it). */
  waiting?: 'engine';
  /** A picture of it exists (its concept picture as soon as it is drawn, else its model's thumbnail): `productionPreviewUrl`. */
  has_preview?: boolean;
  /** Grows with each new picture (cache key). */
  preview_rev?: number;
  /**
   * How far a model got while it is not done: `picture` its picture is drawn (the shape comes from it),
   * `draft` a first look is in the scene (getting it ready for games still to come; a character's skeleton after that),
   * `mesh` a character's model is made, its skeleton and moves not yet (it stands in the scene).
   */
  made?: 'picture' | 'draft' | 'mesh';
  /** The seed its picture and shape are drawn with (absent: 42). */
  seed?: number;
}

export interface ProductionItemStep {
  n: number;
  of: number;
  label: string;
  /** How far into this step (0–1), when the engine says so (building the 3D shape). */
  fraction?: number;
}

/** The agent, model and effort a production plans with (absent: the default). */
export interface ProductionAgent {
  provider?: string;
  model?: string;
  effort?: string;
}

export interface ProductionPlacement {
  item: string;
  /** left | center | right | front | back | far | wall, or a combination like "front-left" ("far": a backdrop piece; "wall": indoors, against the back wall); empty for a piece with `rel`. */
  place: string;
  /** Where it is relative to another piece (instead of a place). */
  rel?: ProductionRel;
  facing?: string;
  attach_to?: ProductionAttach;
  scale_hint?: 'small' | 'normal' | 'big';
}

export interface ProductionPlan {
  lineup?: string[];
  title: string;
  summary: string;
  items: ProductionItem[];
  /** Honest limits, in plain words. */
  notes: string[];
  /** The notes the scene's last build said (the next build takes back the ones no longer true). */
  scene_notes?: string[];
  layout: ProductionPlacement[];
  /** Where and when the scene is (absent on older plans). */
  setting?: ProductionSetting;
}

/** A piece's relation to another (crates/otto-media/src/productions/relation.rs). */
export interface ProductionRel {
  to: string;
  how: 'on' | 'under' | 'in' | 'beside' | 'behind' | 'in_front_of' | 'above' | 'around' | 'held' | 'worn';
  /** on / in / under: left|center|right and/or front|middle|back joined with "-". */
  spot?: string;
  /** beside. */
  side?: 'left' | 'right' | 'front' | 'back';
  /** held. */
  hand?: 'right_hand' | 'left_hand' | 'both';
  /** held: the item it is held over or pointed at. */
  toward?: string;
  /** worn. */
  part?: 'head' | 'neck' | 'back' | 'hip' | 'shoulders';
}

/** A production's setting, in closed words (crates/otto-media/src/productions/setting.rs). */
export interface ProductionSetting {
  /** One plain sentence: "A wooden pier over the sea at night, in thick fog." */
  summary: string;
  ground: 'water' | 'grass' | 'meadow' | 'sand' | 'soil' | 'stone' | 'snow' | 'moon_dust' | 'sea_floor' | 'wood_floor' | 'carpet' | 'tiles' | 'stage' | 'road' | 'none';
  time: 'day' | 'dusk' | 'night' | 'dawn';
  weather: 'clear' | 'fog' | 'rain' | 'snow';
  indoor: boolean;
  /** The items that glow. */
  lights?: { item: string; color: 'warm' | 'cool'; strength: 'low' | 'medium' | 'high' }[];
  /** Absent: earth. */
  world?: 'earth' | 'space' | 'underwater';
  in_sky?: ('sun' | 'moon' | 'earth' | 'planet' | 'stars')[];
}

export interface ProductionRevision {
  id: Id;
  comment: string;
  /** One plain sentence: what changes. */
  summary: string;
  changed: string[];
  added: string[];
  removed: string[];
  /** What the change already shows without making anything, a plain sentence a piece ("The monster is bigger."). */
  done?: string[];
  at: string;
}

export interface Production {
  id: Id;
  workspace_id: Id;
  /** Its project: the id of the project's first creation (its own id when it started one). */
  project_id: Id;
  /** The project's name (its first creation's title), when that is another creation. */
  project_name?: string;
  /** While `ready`: the made pieces of the project's other creations (newest version of each), any item can be switched to. */
  project_pieces?: ProductionPiece[];
  description: string;
  use: ProductionUse;
  status: ProductionStatus;
  message?: string;
  /** Seconds until everything is made. */
  eta_secs?: number;
  /** Of that, until every piece's picture is drawn (absent: none still to draw). */
  pictures_eta_secs?: number;
  /** Until the scene can be looked at, every piece with a shape (absent: it can be). */
  scene_eta_secs?: number;
  /** When pieces are reused: what `eta_secs` would be with every piece made. */
  eta_unreused_secs?: number;
  plan: ProductionPlan;
  scene_artifact_id?: Id;
  revisions: ProductionRevision[];
  /** The agent and model every planner turn runs with (picked on create; absent: the workspace default agent, its default model). */
  provider?: string;
  model?: string;
  /** The reasoning effort of every planner turn (absent: the model's default). */
  effort?: string;
  /** The same pick in one place. */
  agent?: ProductionAgent;
  /** Running and waiting its turn: how many productions are ahead (absent: it is the one being made). */
  queue_position?: number;
  /** Every 3D piece is in the scene (characters may still be getting skeletons and moves): it can be looked at. */
  scene_ready?: boolean;
  created_by: Id;
  created_at: string;
  updated_at: string;
}

/** `GET /design/productions` row (no plan). */
export interface ProductionSummary {
  id: Id;
  workspace_id: Id;
  project_id: Id;
  /** The project's name, when its first creation is another one the list holds. */
  project_name?: string;
  description: string;
  use: ProductionUse;
  status: ProductionStatus;
  message?: string;
  title: string;
  /** Its workspace's name (the list across workspaces says where each is). */
  workspace_name?: string;
  queue_position?: number;
  /** Enabled items made, of how many. */
  items_done: number;
  items_total: number;
  /** Its scene, once there is one (a thumbnail for its card). */
  scene_artifact_id?: Id;
  created_at: string;
  updated_at: string;
}

/** `GET /design/creations`: the Create shelf across every workspace the caller can see. */
export interface Creations {
  productions: ProductionSummary[];
  /** 3D models not inside a production. */
  models: DesignArtifact[];
  /** Voice, sound and music clips not inside a production. */
  clips: TtsClip[];
}

export interface CreateProductionReq {
  workspace_id: Id;
  description: string;
  use?: ProductionUse;
  /** The agent that plans (absent: the workspace default) and its model (absent: the agent's default). 400 when it can't run them. */
  provider?: string;
  model?: string;
  /** One of the model's `supported_efforts` (absent: its default); 400 when it does not list it. */
  effort?: string;
  /** A creation of the project this is a new scene of (absent: a new project). The caller must see it. */
  project_id?: Id;
}

export interface PatchProductionReq {
  items: { id: string; enabled?: boolean; name?: string; description?: string; details?: ProductionItemDetails }[];
}

export interface ProductionPin {
  item_id?: string;
  artifact_id?: string;
  point?: [number, number, number];
  note: string;
}

export interface SteerProductionReq {
  comment: string;
  pins?: ProductionPin[];
}
