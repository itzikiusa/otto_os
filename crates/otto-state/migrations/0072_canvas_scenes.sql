-- Canvas Studio: a visual scene stored as one JSON document ("doc_json").
-- Workspace-scoped; optionally linked to a product story (story_id).
CREATE TABLE canvas_scenes (
    id            TEXT PRIMARY KEY,
    workspace_id  TEXT NOT NULL,
    story_id      TEXT,                 -- optional link to a product story
    title         TEXT NOT NULL,
    doc_json      TEXT NOT NULL,        -- the full Scene JSON (nodes/edges/slides/appState)
    thumbnail     TEXT,                 -- optional data-url preview
    created_by    TEXT NOT NULL,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);
CREATE INDEX idx_canvas_scenes_ws ON canvas_scenes(workspace_id, updated_at);
CREATE INDEX idx_canvas_scenes_story ON canvas_scenes(story_id);
