-- Person-scoped drafts never live in browser storage. Payloads expire after
-- 30 days; revision tombstones prevent stale clients resurrecting expired data.
CREATE TABLE runtime_composer_drafts (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    session_id TEXT NOT NULL REFERENCES agent_runtime_sessions(id) ON DELETE CASCADE,
    revision INTEGER NOT NULL DEFAULT 0,
    payload_json TEXT,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (user_id, workspace_id, session_id)
);
CREATE INDEX idx_runtime_composer_drafts_expiry ON runtime_composer_drafts(updated_at);
