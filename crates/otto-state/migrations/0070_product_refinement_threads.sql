CREATE TABLE product_refinement_threads (
    id                TEXT PRIMARY KEY,
    story_id          TEXT NOT NULL,
    workspace_id      TEXT NOT NULL,
    discovery_run_id  TEXT,
    cwd               TEXT NOT NULL,
    title             TEXT NOT NULL,
    status            TEXT NOT NULL DEFAULT 'active',
    model             TEXT,
    created_by        TEXT NOT NULL,
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL
);
CREATE INDEX idx_refine_threads_story ON product_refinement_threads(story_id, created_at);
