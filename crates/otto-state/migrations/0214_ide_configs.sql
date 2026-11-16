-- Otto Full IDE — per-project toolchain/interpreter environments and saved
-- Run/Debug configurations (`otto_ide::environments`).
--
-- ADDITIVE ONLY: two new tables + their indexes; no existing table is touched
-- and no seed rows are inserted, so both rollback compatibility
-- (`migration_compat`) and fresh-DB state-archive restore remain intact.
--
-- Secret environment variables NEVER enter SQLite: `env_vars_json` stores only
-- `{ key, value: null, secret: true, secret_ref: "ide-env:<hash>:<key>" }`,
-- while the secret payload lives in `otto-keychain` (`SecretStore`).

CREATE TABLE IF NOT EXISTS ide_project_environments (
    project_key        TEXT PRIMARY KEY DEFAULT '',
    target_kind        TEXT NOT NULL DEFAULT 'local',
    connection_id      TEXT,
    root_path          TEXT NOT NULL DEFAULT '',
    workspace_id       TEXT,
    java_home          TEXT,
    go_root            TEXT,
    rust_toolchain     TEXT,
    node_home          TEXT,
    python_interpreter TEXT,
    dotenv_path        TEXT,
    extra_path         TEXT,
    env_vars_json      TEXT NOT NULL DEFAULT '[]',
    created_at         TEXT NOT NULL DEFAULT '',
    updated_at         TEXT NOT NULL DEFAULT ''
);

CREATE INDEX IF NOT EXISTS idx_ide_project_environments_root
    ON ide_project_environments(target_kind, root_path);

CREATE TABLE IF NOT EXISTS ide_run_configs (
    id                 TEXT PRIMARY KEY DEFAULT '',
    project_key        TEXT NOT NULL DEFAULT '',
    target_kind        TEXT NOT NULL DEFAULT 'local',
    connection_id      TEXT,
    root_path          TEXT NOT NULL DEFAULT '',
    name               TEXT NOT NULL DEFAULT '',
    runner             TEXT NOT NULL DEFAULT 'custom',
    command            TEXT,
    args_json          TEXT NOT NULL DEFAULT '[]',
    cwd                TEXT,
    env_overrides_json TEXT NOT NULL DEFAULT '{}',
    before_launch      TEXT,
    target_path        TEXT,
    test_filter        TEXT,
    created_at         TEXT NOT NULL DEFAULT '',
    updated_at         TEXT NOT NULL DEFAULT ''
);

CREATE INDEX IF NOT EXISTS idx_ide_run_configs_project
    ON ide_run_configs(project_key, created_at);
