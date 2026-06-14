-- Archived sessions: PTY killed, row + history retained, hidden from the
-- active list until restored or deleted.
ALTER TABLE sessions ADD COLUMN archived INTEGER NOT NULL DEFAULT 0;
CREATE INDEX idx_sessions_ws_archived ON sessions(workspace_id, archived);
