-- Covering indexes for the command-palette / FloatingBar metadata search
-- (`otto_state::search`, run on every debounced keystroke). Additive only.

-- Canvas: `SELECT id,title,section,updated_at FROM canvas_scenes WHERE
-- workspace_id = ? ORDER BY updated_at DESC`. `section` was added by ALTER
-- TABLE (0075), so it sits physically after the multi-megabyte `doc_json` /
-- `thumbnail`; reading it walked every scene's overflow chain — O(total canvas
-- bytes) per keystroke. With every projected column in the index the query
-- never touches the table row.
CREATE INDEX IF NOT EXISTS idx_canvas_scenes_search
    ON canvas_scenes(workspace_id, updated_at DESC, id, title, section);

-- API requests: `SELECT id,name,method,url,updated_at ... WHERE workspace_id
-- = ? ORDER BY position, name`. `position` / `updated_at` sit after `body` /
-- `auth_json`; same overflow-chain cost, smaller rows.
CREATE INDEX IF NOT EXISTS idx_api_requests_search
    ON api_requests(workspace_id, position, name, id, method, url, updated_at);
