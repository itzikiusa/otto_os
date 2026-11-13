-- Otto Full IDE — multi-folder project roots, user login/interactive shell env
-- inheritance toggle, IntelliJ/GoLand run configuration extensions, and custom
-- local SDK path registry (`otto_ide::environments`, `otto_ide::toolchains`).
--
-- ADDITIVE ONLY: every new NOT NULL column carries an explicit DEFAULT so
-- rollback compatibility (`migration_compat`) and state-archive restore remain
-- 100% intact.

ALTER TABLE ide_project_environments
    ADD COLUMN additional_roots_json TEXT NOT NULL DEFAULT '[]';

ALTER TABLE ide_project_environments
    ADD COLUMN inherit_shell_env INTEGER NOT NULL DEFAULT 1;

ALTER TABLE ide_run_configs
    ADD COLUMN build_flags_json TEXT NOT NULL DEFAULT '[]';

ALTER TABLE ide_run_configs
    ADD COLUMN env_file TEXT;

ALTER TABLE ide_run_configs
    ADD COLUMN inherit_shell_env INTEGER NOT NULL DEFAULT 1;

ALTER TABLE ide_run_configs
    ADD COLUMN sdk_path TEXT;

ALTER TABLE ide_run_configs
    ADD COLUMN toolchain_id TEXT;

ALTER TABLE ide_run_configs
    ADD COLUMN compound_ids_json TEXT NOT NULL DEFAULT '[]';

CREATE TABLE IF NOT EXISTS ide_local_sdks (
    id         TEXT PRIMARY KEY DEFAULT '',
    host_key   TEXT NOT NULL DEFAULT 'local',
    ecosystem  TEXT NOT NULL DEFAULT '',
    version    TEXT NOT NULL DEFAULT '',
    path       TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT '',
    UNIQUE(host_key, ecosystem, path)
);

CREATE INDEX IF NOT EXISTS idx_ide_local_sdks_host
    ON ide_local_sdks(host_key, ecosystem);
