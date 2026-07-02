-- Unified Connections hub: broker (Kafka) clusters join the SAME section tree
-- as SSH/DB connections, so the Connections page can show everything under one
-- roof with type filters instead of three separate (often empty-looking) trees.
--
-- 1) Copy every broker_cluster_sections row into connection_sections keeping
--    the SAME ids (ULIDs; no collision with existing rows) — this keeps every
--    broker_clusters.section_id value pointing at a live section, and the
--    parent_id links stay internally consistent within the copied set.
-- 2) Repoint broker_clusters.section_id's FK at connection_sections. SQLite
--    can't ALTER a foreign key, so rebuild the table (child-side rebuild only:
--    connection_sections is never dropped, so no ON DELETE SET NULL storm —
--    the 0093 lesson).
-- 3) Drop the now-orphaned broker_cluster_sections tree.
--
-- `scope` ('connections' | 'db') is dead — the backend already serves ONE
-- global tree to both pages — so normalize legacy 'db' rows for consistency.

INSERT INTO connection_sections (id, workspace_id, parent_id, name, position, created_by, created_at, scope)
SELECT id, workspace_id, parent_id, name, position, created_by, created_at, 'connections'
FROM broker_cluster_sections;

UPDATE connection_sections SET scope = 'connections' WHERE scope <> 'connections';

CREATE TABLE broker_clusters_new (
    id                        TEXT PRIMARY KEY,
    workspace_id              TEXT REFERENCES workspaces(id) ON DELETE CASCADE,
    name                      TEXT NOT NULL,
    bootstrap_servers         TEXT NOT NULL,
    security_protocol         TEXT NOT NULL DEFAULT 'plaintext',
    sasl_mechanism            TEXT,
    sasl_username             TEXT,
    secret_ref                TEXT,
    tls_skip_verify           INTEGER NOT NULL DEFAULT 0,
    schema_registry_url       TEXT,
    schema_registry_username  TEXT,
    sr_secret_ref             TEXT,
    metrics_url               TEXT,
    color                     TEXT,
    environment               TEXT NOT NULL DEFAULT 'dev',
    read_only                 INTEGER NOT NULL DEFAULT 0,
    created_by                TEXT NOT NULL REFERENCES users(id),
    created_at                TEXT NOT NULL,
    ssh_config                TEXT,
    section_id                TEXT REFERENCES connection_sections(id) ON DELETE SET NULL
);

INSERT INTO broker_clusters_new (
    id, workspace_id, name, bootstrap_servers, security_protocol, sasl_mechanism,
    sasl_username, secret_ref, tls_skip_verify, schema_registry_url,
    schema_registry_username, sr_secret_ref, metrics_url, color, environment,
    read_only, created_by, created_at, ssh_config, section_id
)
SELECT
    id, workspace_id, name, bootstrap_servers, security_protocol, sasl_mechanism,
    sasl_username, secret_ref, tls_skip_verify, schema_registry_url,
    schema_registry_username, sr_secret_ref, metrics_url, color, environment,
    read_only, created_by, created_at, ssh_config, section_id
FROM broker_clusters;

DROP TABLE broker_clusters;
ALTER TABLE broker_clusters_new RENAME TO broker_clusters;
CREATE INDEX idx_broker_clusters_ws ON broker_clusters(workspace_id, name);

DROP TABLE broker_cluster_sections;
