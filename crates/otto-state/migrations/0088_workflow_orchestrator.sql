-- Workflow orchestrator upgrades: versioning + run→proof-pack link.
--
-- Append-only migration. Adds a monotonic `version` to each workflow, a
-- `workflow_versions` history table (one snapshot of graph_json per version),
-- and records on each run both the version it executed and the proof pack it
-- produced.
--
-- All new columns are nullable or carry a constant default so existing rows and
-- existing workflows keep working unchanged.

-- Current version pointer on the workflow (existing rows → 1).
ALTER TABLE workflows ADD COLUMN version INTEGER NOT NULL DEFAULT 1;

-- Which version a run executed, and the proof pack assembled for it.
ALTER TABLE workflow_runs ADD COLUMN workflow_version INTEGER;
ALTER TABLE workflow_runs ADD COLUMN proof_pack_id TEXT;

-- Append-only version history. Restoring a version writes a NEW version row
-- equal to the chosen one rather than rewinding the counter.
CREATE TABLE workflow_versions (
    id          TEXT PRIMARY KEY,
    workflow_id TEXT NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,
    version     INTEGER NOT NULL,
    name        TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    graph_json  TEXT NOT NULL,
    note        TEXT NOT NULL DEFAULT '',
    created_by  TEXT NOT NULL,
    created_at  TEXT NOT NULL,
    UNIQUE(workflow_id, version)
);

CREATE INDEX idx_workflow_versions_wf ON workflow_versions(workflow_id, version DESC);

-- Backfill a v1 snapshot for every pre-existing workflow so its history is not
-- empty (matches the "snapshot on create" invariant new workflows get).
INSERT INTO workflow_versions
    (id, workflow_id, version, name, description, graph_json, note, created_by, created_at)
SELECT lower(hex(randomblob(16))), id, 1, name, description, graph_json,
       'initial', created_by, created_at
FROM workflows;
