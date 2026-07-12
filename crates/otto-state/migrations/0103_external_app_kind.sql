-- Add 'external_app' to the connections.kind CHECK constraint.
--
-- The Connectors feature stores each Azure AD SSO tile as a
-- `ConnectionKind::ExternalApp` connection (kind = 'external_app'); its `params`
-- carry app_kind/launch_url and any acquired credentials ride `secret_ref` →
-- Keychain. The 0001 CHECK constraint didn't allow this value, so inserts would
-- fail — this widens the allowed set.
--
-- SQLite cannot ALTER a CHECK in place, so the column's allowed set is widened
-- by rebuilding the table (create-new → copy → drop → rename), exactly like 0094
-- (which added 'postgres'). `connections` is a PARENT of `sessions`
-- (`connection_id REFERENCES connections(id) ON DELETE SET NULL`); with foreign
-- keys ON the DROP below implicitly DELETEs every row and would null those
-- session links, so we back them up and restore them after the rebuild — see
-- 0094 for the full rationale.
--
-- The DDL below is the current shape: the 0094 rebuild (itself the 0001 base
-- plus the 0008/0032/0050 ADD COLUMNs). Nothing has altered `connections` since.
CREATE TABLE connections_new (
    id            TEXT PRIMARY KEY,
    workspace_id  TEXT REFERENCES workspaces(id) ON DELETE CASCADE,
    name          TEXT NOT NULL,
    kind          TEXT NOT NULL CHECK (kind IN ('ssh','mysql','redis','mongodb','clickhouse','postgres','custom','external_app')),
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
