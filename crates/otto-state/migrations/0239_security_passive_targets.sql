CREATE TABLE ide_security_passive_targets (
    project_id TEXT PRIMARY KEY REFERENCES ide_named_projects(id) ON DELETE CASCADE,
    run_config_id TEXT NOT NULL,
    url TEXT NOT NULL,
    allow_public INTEGER NOT NULL DEFAULT 0,
    scheduled INTEGER NOT NULL DEFAULT 0,
    confirmed_at TEXT NOT NULL
);
