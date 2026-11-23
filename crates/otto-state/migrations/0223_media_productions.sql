-- Smart mode (productions, docs/contracts/api.md "Productions"): one row per
-- production; the plan (items with their progress and results, the layout)
-- and the steering revisions are JSON. A new table only: the previous build
-- never reads it.
CREATE TABLE IF NOT EXISTS media_productions (
    id                TEXT PRIMARY KEY,
    workspace_id      TEXT NOT NULL,
    description       TEXT NOT NULL,
    use_for           TEXT NOT NULL DEFAULT 'pc',
    status            TEXT NOT NULL,
    message           TEXT,
    plan_json         TEXT NOT NULL DEFAULT '{}',
    revisions_json    TEXT NOT NULL DEFAULT '[]',
    scene_artifact_id TEXT,
    created_by        TEXT NOT NULL,
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_media_productions_workspace
    ON media_productions(workspace_id, created_at DESC);
