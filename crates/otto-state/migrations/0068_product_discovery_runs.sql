CREATE TABLE product_discovery_runs (
    id            TEXT PRIMARY KEY,
    story_id      TEXT NOT NULL,
    workspace_id  TEXT NOT NULL,
    swarm_id      TEXT NOT NULL,
    project_id    TEXT NOT NULL,          -- the discovery swarm project (story_id NOT set on it)
    status        TEXT NOT NULL DEFAULT 'running', -- 'running' | 'done' | 'error' | 'stopped'
    brief_md      TEXT NOT NULL,          -- the assembled discovery brief (audit/repro)
    report_md     TEXT,                   -- consolidated findings, filled as work completes
    created_by    TEXT NOT NULL,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);
CREATE INDEX idx_product_discovery_runs_story ON product_discovery_runs(story_id, created_at);
