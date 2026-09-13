-- Scope and keyset before transcript resolution; claimed ids intentionally span providers.
CREATE INDEX idx_sessions_history_workspace ON sessions(workspace_id, last_active_at DESC, id);
CREATE INDEX idx_sessions_history_owner ON sessions(workspace_id, created_by, last_active_at DESC, id);
CREATE INDEX idx_sessions_transcript_path ON sessions(transcript_path);
CREATE INDEX idx_sessions_provider_session_id ON sessions(provider_session_id);
CREATE INDEX idx_transcript_history_activity ON transcript_index(COALESCE(last_active_at,started_at,'' ) DESC, path);
