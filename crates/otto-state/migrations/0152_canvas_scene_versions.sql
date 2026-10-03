-- C5: Canvas scene version history. Mermaid / D2 / Excalidraw scenes had no
-- history at all, and an Ask AI turn replaces the whole document — one bad
-- turn (or an accidental overwrite) was unrecoverable.
--
-- Each row is the scene's document as it was JUST BEFORE a change: before every
-- agent commit, before a restore, and at most once every 10 minutes across user
-- saves. The repo keeps the newest 30 per scene. `origin` is
-- 'agent' | 'user' | 'restore'.
CREATE TABLE canvas_scene_versions (
    id          TEXT PRIMARY KEY,
    scene_id    TEXT NOT NULL REFERENCES canvas_scenes(id) ON DELETE CASCADE,
    doc_json    TEXT NOT NULL,
    origin      TEXT NOT NULL,
    created_by  TEXT,
    created_at  TEXT NOT NULL
);

CREATE INDEX idx_canvas_scene_versions_scene ON canvas_scene_versions(scene_id, created_at);
