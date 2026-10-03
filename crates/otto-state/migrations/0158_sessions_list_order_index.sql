-- The session list (`SessionsRepo::list_filtered`) runs
-- `WHERE workspace_id = ? AND archived = ? ORDER BY created_at, id` — and the
-- archived pager the same with `DESC … LIMIT`. The only covering index was
-- `(workspace_id, archived)`, so SQLite built a temp B-tree for the sort on
-- every call. This one serves both the filter and the order (either way).
CREATE INDEX IF NOT EXISTS idx_sessions_ws_archived_created
    ON sessions(workspace_id, archived, created_at, id);
