-- Add 'postgres' to the connections.kind CHECK constraint.
--
-- SQLite cannot ALTER a CHECK in place, so the column's allowed set is widened
-- by rebuilding the table (create-new → copy → drop → rename), like 0064.
--
-- Unlike 0064's leaf table, `connections` is a PARENT: `sessions` has
-- `connection_id REFERENCES connections(id) ON DELETE SET NULL` (0001_init.sql:49,
-- the only real FK into connections — the other `connection_id` columns in
-- 0021/0063 are plain TEXT with no constraint). With foreign-key enforcement ON
-- (this app opens SQLite with `foreign_keys=true`, otto-state/src/db.rs) the
-- `DROP TABLE connections` performs an implicit DELETE of every row FIRST, which
-- fires that ON DELETE SET NULL and would WIPE every session's connection link.
--
-- We cannot disable FK enforcement here: sqlx-sqlite ALWAYS runs a migration
-- inside a transaction (it ignores `-- no-transaction`), and `PRAGMA foreign_keys`
-- / `PRAGMA legacy_alter_table` are no-ops once a transaction is open. So we back
-- up the session→connection links before the DROP nulls them and restore them
-- once the rebuilt table exists (verified FK-clean via `PRAGMA foreign_key_check`).
--
-- The full current DDL is the 0001 base plus every later
-- `ALTER TABLE connections ADD COLUMN` (0008 section_id, 0032 environment/read_only,
-- 0050 last_opened_at/pinned).
CREATE TABLE connections_new (
    id            TEXT PRIMARY KEY,
    workspace_id  TEXT REFERENCES workspaces(id) ON DELETE CASCADE,
    name          TEXT NOT NULL,
    kind          TEXT NOT NULL CHECK (kind IN ('ssh','mysql','redis','mongodb','clickhouse','postgres','custom')),
    params_json   TEXT NOT NULL DEFAULT '{}',
    secret_ref    TEXT,
    first_command TEXT,
    created_by    TEXT NOT NULL REFERENCES users(id),
    created_at    TEXT NOT NULL,
    section_id    TEXT REFERENCES connection_sections(id) ON DELETE SET NULL,
    environment   TEXT NOT NULL DEFAULT 'dev' CHECK (environment IN ('dev','staging','prod')),
    read_only     INTEGER NOT NULL DEFAULT 0 CHECK (read_only IN (0,1)),
    last_opened_at TEXT,
    pinned        INTEGER NOT NULL DEFAULT 0
);

INSERT INTO connections_new
    (id, workspace_id, name, kind, params_json, secret_ref, first_command,
     created_by, created_at, section_id, environment, read_only, last_opened_at, pinned)
SELECT
    id, workspace_id, name, kind, params_json, secret_ref, first_command,
    created_by, created_at, section_id, environment, read_only, last_opened_at, pinned
FROM connections;

-- Preserve the session→connection links that the DROP below will SET NULL.
CREATE TEMP TABLE _session_conn_backup AS
    SELECT id, connection_id FROM sessions WHERE connection_id IS NOT NULL;

DROP TABLE connections;

ALTER TABLE connections_new RENAME TO connections;

-- Restore the links now that the parent exists again (ids are unchanged, so the
-- FK is satisfied against the rebuilt table).
UPDATE sessions
   SET connection_id = (SELECT b.connection_id FROM _session_conn_backup b WHERE b.id = sessions.id)
 WHERE id IN (SELECT id FROM _session_conn_backup);

DROP TABLE _session_conn_backup;

CREATE INDEX idx_connections_ws ON connections(workspace_id);
CREATE INDEX idx_connections_recency ON connections(last_opened_at DESC);
