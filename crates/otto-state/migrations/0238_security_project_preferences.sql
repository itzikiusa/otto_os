CREATE TABLE ide_security_schedules (
    project_id TEXT PRIMARY KEY REFERENCES ide_named_projects(id) ON DELETE CASCADE,
    cron TEXT NOT NULL,
    timezone TEXT NOT NULL,
    armed_at TEXT NOT NULL,
    last_slot TEXT
);

CREATE TABLE ide_security_suppressions (
    project_id TEXT NOT NULL REFERENCES ide_named_projects(id) ON DELETE CASCADE,
    fingerprint TEXT NOT NULL,
    reason TEXT NOT NULL,
    PRIMARY KEY (project_id, fingerprint)
);

CREATE TABLE ide_security_baselines (
    project_id TEXT PRIMARY KEY REFERENCES ide_named_projects(id) ON DELETE CASCADE,
    scan_id TEXT NOT NULL REFERENCES ide_security_scans(id) ON DELETE CASCADE
);
