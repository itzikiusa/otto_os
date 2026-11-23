-- Product 3D Polygon Comments + Native 3D Generation Providers/Jobs + Built-in/Extensible TTS.
--
-- ADDITIVE ONLY & ROLLBACK-SAFE (crates/otto-state/tests/migration_compat.rs):
--   1. One nullable column (`pin_3d_json`) added to `design_comments`.
--   2. Four brand-new tables (`media_3d_providers`, `media_3d_jobs`,
--      `media_tts_providers`, `media_tts_clips`) + their indexes.
-- Previous builds ignore `pin_3d_json` and never read the new tables.

ALTER TABLE design_comments ADD COLUMN pin_3d_json TEXT;

CREATE TABLE IF NOT EXISTS media_3d_providers (
    id                  TEXT PRIMARY KEY,
    name                TEXT NOT NULL,
    kind                TEXT NOT NULL,           -- hunyuan3d_omni | trellis_2 | custom_http | custom_cli
    transport           TEXT NOT NULL,           -- http | local_cli
    builtin             INTEGER NOT NULL DEFAULT 0,
    enabled             INTEGER NOT NULL DEFAULT 1,
    license_warning     TEXT,                    -- optional license/territory restriction notice
    endpoint_url        TEXT,
    cli_binary          TEXT,
    cli_args_json       TEXT NOT NULL DEFAULT '[]',
    cli_workdir         TEXT,
    auth_header         TEXT,
    secret_key_ref      TEXT,                    -- Keychain key name only; NEVER raw secret
    allow_local_network INTEGER NOT NULL DEFAULT 0,
    timeout_secs        INTEGER NOT NULL DEFAULT 900,
    max_output_bytes    INTEGER NOT NULL DEFAULT 134217728, -- 128 MiB default (up to 256 MiB)
    default_params_json TEXT NOT NULL DEFAULT '{}',
    created_at          TEXT NOT NULL,
    updated_at          TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS media_3d_jobs (
    id                  TEXT PRIMARY KEY,
    workspace_id        TEXT NOT NULL,
    project_id          TEXT,
    provider_id         TEXT NOT NULL,
    provider_kind       TEXT NOT NULL,
    pipeline_stage      TEXT NOT NULL DEFAULT 'generate', -- generate | gameready | rig | animate
    source_artifact_id  TEXT,
    source_version_id   TEXT,
    target_artifact_id  TEXT,
    title               TEXT NOT NULL,
    prompt              TEXT,
    output_format       TEXT NOT NULL DEFAULT 'glb',
    preset              TEXT,
    poly_budget         INTEGER,
    seed                INTEGER,
    status              TEXT NOT NULL DEFAULT 'queued', -- queued | running | succeeded | failed | cancelled
    progress_pct        INTEGER NOT NULL DEFAULT 0,
    stage               TEXT NOT NULL DEFAULT 'Queued',
    artifact_id         TEXT,
    version_id          TEXT,
    exported_path       TEXT,
    size_bytes          INTEGER,
    manifest_json       TEXT,                    -- optional <output>.manifest.json provenance
    error               TEXT,
    created_by          TEXT NOT NULL,
    created_at          TEXT NOT NULL,
    updated_at          TEXT NOT NULL,
    finished_at         TEXT
);
CREATE INDEX IF NOT EXISTS idx_media_3d_jobs_workspace ON media_3d_jobs(workspace_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_media_3d_jobs_artifact ON media_3d_jobs(artifact_id, created_at DESC);

CREATE TABLE IF NOT EXISTS media_tts_providers (
    id                  TEXT PRIMARY KEY,
    name                TEXT NOT NULL,
    kind                TEXT NOT NULL,           -- macos_say | openai_compat | elevenlabs | kokoro | custom_http
    builtin             INTEGER NOT NULL DEFAULT 0,
    enabled             INTEGER NOT NULL DEFAULT 1,
    endpoint_url        TEXT,
    default_model       TEXT,
    default_voice       TEXT,
    auth_header         TEXT,
    secret_key_ref      TEXT,                    -- Keychain key name only; NEVER raw secret
    allow_local_network INTEGER NOT NULL DEFAULT 0,
    created_at          TEXT NOT NULL,
    updated_at          TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS media_tts_clips (
    id                  TEXT PRIMARY KEY,
    workspace_id        TEXT NOT NULL,
    story_id            TEXT,
    artifact_id         TEXT,
    provider_id         TEXT NOT NULL,
    provider_kind       TEXT NOT NULL,
    voice_id            TEXT NOT NULL,
    title               TEXT NOT NULL,
    character_name      TEXT,
    text                TEXT NOT NULL,
    format              TEXT NOT NULL DEFAULT 'wav',
    mime                TEXT NOT NULL DEFAULT 'audio/wav',
    speed               REAL NOT NULL DEFAULT 1.0,
    size_bytes          INTEGER NOT NULL DEFAULT 0,
    duration_ms         INTEGER,
    file_name           TEXT NOT NULL,
    exported_path       TEXT,
    visemes_json        TEXT,                    -- optional { duration_sec, cues: [{start, end, value}] }
    created_by          TEXT NOT NULL,
    created_at          TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_media_tts_clips_workspace ON media_tts_clips(workspace_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_media_tts_clips_story ON media_tts_clips(story_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_media_tts_clips_artifact ON media_tts_clips(artifact_id, created_at DESC);
