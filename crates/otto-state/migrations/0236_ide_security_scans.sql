CREATE TABLE ide_security_settings (
    project_id TEXT PRIMARY KEY REFERENCES ide_named_projects(id) ON DELETE CASCADE,
    settings   TEXT NOT NULL
);

CREATE TABLE ide_security_scans (
    id          TEXT PRIMARY KEY,
    project_id  TEXT NOT NULL REFERENCES ide_named_projects(id) ON DELETE CASCADE,
    status      TEXT NOT NULL CHECK (status IN ('queued', 'running', 'completed', 'failed', 'cancelled')),
    created_at  TEXT NOT NULL,
    started_at  TEXT,
    finished_at TEXT,
    engine      TEXT,
    metrics     TEXT,
    error       TEXT,
    count       INTEGER NOT NULL DEFAULT 0,
    findings    TEXT NOT NULL DEFAULT '[]'
);

CREATE INDEX ide_security_scans_history
    ON ide_security_scans(project_id, created_at DESC, id DESC);

CREATE UNIQUE INDEX ide_security_scans_one_live_per_project
    ON ide_security_scans(project_id) WHERE status IN ('queued', 'running');
