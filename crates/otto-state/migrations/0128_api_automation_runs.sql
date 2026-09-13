-- Durable execution reports survive UI navigation and daemon restarts. Snapshots
-- contain redacted request metadata; credentials and dataset values stay in memory.
CREATE TABLE api_automation_runs (
    id TEXT PRIMARY KEY NOT NULL,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    automation_id TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL,
    finished_at TEXT,
    record_json TEXT NOT NULL
);
CREATE INDEX api_automation_runs_workspace_created ON api_automation_runs(workspace_id, created_at DESC, id DESC);
