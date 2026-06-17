-- Workflow engine: an n8n-style node graph (mostly generated from a natural-
-- language description via "agent mode", optionally hand-edited) plus its run
-- history. Mirrors otto_core::workflows::{Workflow,WorkflowRun}. ULID string
-- PKs, UTC RFC3339 timestamps, JSON in *_json columns. Rows cascade with their
-- workspace.

CREATE TABLE workflows (
    id           TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    description  TEXT NOT NULL DEFAULT '',
    -- WorkflowGraph: { nodes:[...], edges:[...] }.
    graph_json   TEXT NOT NULL DEFAULT '{"nodes":[],"edges":[]}',
    created_by   TEXT NOT NULL REFERENCES users(id),
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
);
CREATE INDEX idx_workflows_ws ON workflows(workspace_id, updated_at DESC);

CREATE TABLE workflow_runs (
    id           TEXT PRIMARY KEY,
    workflow_id  TEXT NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    status       TEXT NOT NULL CHECK (status IN ('pending','running','success','error','canceled')),
    input_json   TEXT NOT NULL DEFAULT 'null',
    -- Vec<NodeRunState>: per-node status/output/error.
    nodes_json   TEXT NOT NULL DEFAULT '[]',
    error        TEXT,
    started_at   TEXT NOT NULL,
    finished_at  TEXT
);
CREATE INDEX idx_workflow_runs_wf ON workflow_runs(workflow_id, started_at DESC);
