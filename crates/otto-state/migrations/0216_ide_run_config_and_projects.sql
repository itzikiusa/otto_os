-- Otto Full IDE — persisted Maven/Spring run-configuration profile flags and
-- SQLite-backed user-scoped Named Multi-Repo Projects (`otto_ide::environments`,
-- `otto_ide::projects`).
--
-- ADDITIVE ONLY: every added NOT NULL column carries an explicit DEFAULT and no
-- seed rows are inserted, keeping rollback compatibility (`migration_compat`)
-- and fresh-DB state-archive restore intact.

ALTER TABLE ide_run_configs
    ADD COLUMN maven_profiles_json TEXT NOT NULL DEFAULT '[]';

ALTER TABLE ide_run_configs
    ADD COLUMN spring_profiles_json TEXT NOT NULL DEFAULT '[]';

ALTER TABLE ide_run_configs
    ADD COLUMN skip_tests INTEGER NOT NULL DEFAULT 0;

ALTER TABLE ide_run_configs
    ADD COLUMN offline INTEGER NOT NULL DEFAULT 0;

CREATE TABLE IF NOT EXISTS ide_named_projects (
    id            TEXT PRIMARY KEY DEFAULT '',
    owner_user_id TEXT NOT NULL DEFAULT '',
    name          TEXT NOT NULL DEFAULT '',
    primary_root  TEXT NOT NULL DEFAULT '',
    folders_json  TEXT NOT NULL DEFAULT '[]',
    pinned        INTEGER NOT NULL DEFAULT 0,
    updated_at    TEXT NOT NULL DEFAULT ''
);

CREATE INDEX IF NOT EXISTS idx_ide_named_projects_owner
    ON ide_named_projects(owner_user_id, updated_at DESC);
