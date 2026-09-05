-- History index (design §4.6): one row per transcript file found under
-- ~/.claude/projects and ~/.codex/sessions by the background scan. Cheap
-- metadata only (head 64 KB + tail 16 KB peek); (mtime, size) lets a rescan
-- skip unchanged files. Rows not claimed by any `sessions.transcript_path` /
-- `provider_session_id` surface in History as `status: on_disk`.
CREATE TABLE transcript_index (
    path                TEXT PRIMARY KEY,
    provider            TEXT NOT NULL,           -- claude | codex
    provider_session_id TEXT,
    cwd                 TEXT,
    title               TEXT,
    first_prompt        TEXT,
    started_at          TEXT,
    last_active_at      TEXT,
    mtime               INTEGER NOT NULL,        -- unix seconds
    size                INTEGER NOT NULL,        -- bytes
    turns               INTEGER,                 -- NULL when the middle was skipped
    indexed_at          TEXT NOT NULL
);
CREATE INDEX idx_transcript_index_psid ON transcript_index(provider_session_id);
CREATE INDEX idx_transcript_index_last ON transcript_index(last_active_at DESC);
