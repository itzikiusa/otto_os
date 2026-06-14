-- Widen improvement_runs.trigger to allow 'live' (the in-loop skill evolver).
-- SQLite can't ALTER a CHECK in place, so rebuild via create-new → copy →
-- drop-old → rename. The child improvement_edits FK references the *name*
-- "improvement_runs"; dropping the old table briefly dangles it within this
-- migration transaction (no FK check fires — we touch no improvement_edits
-- rows), and the final RENAME restores the name so the FK resolves again.
CREATE TABLE improvement_runs_new (
    id                TEXT PRIMARY KEY,
    workspace_id      TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    trigger           TEXT NOT NULL CHECK (trigger IN ('scheduled','manual','live')),
    status            TEXT NOT NULL CHECK (status IN ('running','done','skipped','failed')),
    summary           TEXT NOT NULL DEFAULT '',
    sessions_reviewed INTEGER NOT NULL DEFAULT 0,
    applied           INTEGER NOT NULL DEFAULT 0,
    pending           INTEGER NOT NULL DEFAULT 0,
    error             TEXT,
    started_at        TEXT NOT NULL,
    finished_at       TEXT
);

INSERT INTO improvement_runs_new SELECT * FROM improvement_runs;
DROP TABLE improvement_runs;
ALTER TABLE improvement_runs_new RENAME TO improvement_runs;
CREATE INDEX idx_improvement_runs_ws ON improvement_runs(workspace_id, started_at DESC);
