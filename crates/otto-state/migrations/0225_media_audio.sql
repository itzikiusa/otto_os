-- Sounds and music (docs/contracts/api.md "Sounds and music"): Otto Forge's
-- audio stage makes a WAV that is stored as a clip, so every voice-clip player
-- and export works.
-- ADDITIVE ONLY: two columns with a DEFAULT / nullable on media_tts_clips and a
-- new table. A previous build reads sound and music clips as voice clips (they
-- play) and never reads media_audio_jobs.
ALTER TABLE media_tts_clips ADD COLUMN kind TEXT NOT NULL DEFAULT 'voice'; -- voice | sound | music
ALTER TABLE media_tts_clips ADD COLUMN manifest_json TEXT;               -- the engine's manifest (model, licence, loudness, loop seam)

CREATE TABLE IF NOT EXISTS media_audio_jobs (
    id            TEXT PRIMARY KEY,
    workspace_id  TEXT NOT NULL,
    kind          TEXT NOT NULL,                 -- sound | music
    title         TEXT NOT NULL,
    prompt        TEXT NOT NULL,
    seconds       REAL NOT NULL,
    looped        INTEGER NOT NULL DEFAULT 0,
    bpm           INTEGER,
    seed          INTEGER,
    story_id      TEXT,
    artifact_id   TEXT,
    status        TEXT NOT NULL,                 -- queued | running | succeeded | failed | cancelled | not_installed
    progress_pct  INTEGER NOT NULL DEFAULT 0,
    stage         TEXT NOT NULL DEFAULT 'Queued',
    clip_id       TEXT,                          -- media_tts_clips.id once made
    error         TEXT,
    created_by    TEXT NOT NULL,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL,
    finished_at   TEXT
);
CREATE INDEX IF NOT EXISTS idx_media_audio_jobs_workspace
    ON media_audio_jobs(workspace_id, created_at DESC);
