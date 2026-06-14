-- One row per self-reflection run (scheduled or manual).
CREATE TABLE improvement_runs (
    id                TEXT PRIMARY KEY,
    workspace_id      TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    trigger           TEXT NOT NULL CHECK (trigger IN ('scheduled','manual')),
    status            TEXT NOT NULL CHECK (status IN ('running','done','skipped','failed')),
    summary           TEXT NOT NULL DEFAULT '',
    sessions_reviewed INTEGER NOT NULL DEFAULT 0,
    applied           INTEGER NOT NULL DEFAULT 0,
    pending           INTEGER NOT NULL DEFAULT 0,
    error             TEXT,
    started_at        TEXT NOT NULL,
    finished_at       TEXT
);
CREATE INDEX idx_improvement_runs_ws ON improvement_runs(workspace_id, started_at DESC);

-- One row per proposed edit = the version log.
CREATE TABLE improvement_edits (
    id              TEXT PRIMARY KEY,
    run_id          TEXT NOT NULL REFERENCES improvement_runs(id) ON DELETE CASCADE,
    workspace_id    TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    target          TEXT NOT NULL CHECK (target IN ('skill','memory')),
    target_ref      TEXT NOT NULL,              -- skill name or memory filename
    target_path     TEXT NOT NULL,              -- resolved absolute path
    kind            TEXT NOT NULL CHECK (kind IN ('add','modify','remove')),
    risk            TEXT NOT NULL CHECK (risk IN ('low','structural')),
    status          TEXT NOT NULL CHECK (status IN ('pending','applied','rejected','rolled_back','conflict')),
    rationale       TEXT NOT NULL DEFAULT '',
    evidence_json   TEXT NOT NULL DEFAULT '[]', -- ["session_id", ...]
    before_content  TEXT,                        -- NULL when the file did not exist
    after_content   TEXT NOT NULL,
    applied_at      TEXT,
    actor           TEXT,                        -- "system" | user_id
    created_at      TEXT NOT NULL
);
CREATE INDEX idx_improvement_edits_run ON improvement_edits(run_id);
CREATE INDEX idx_improvement_edits_ws_status ON improvement_edits(workspace_id, status);
