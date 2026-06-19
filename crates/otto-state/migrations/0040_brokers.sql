-- Message Brokers: saved Kafka cluster connection profiles. Secrets (the SASL
-- password and the schema-registry password) live only in the Keychain; rows
-- carry opaque refs (`broker-{id}`, `broker-sr-{id}`), never the secret itself.
-- workspace_id NULL = global (root-managed), visible to every workspace.
CREATE TABLE broker_clusters (
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
    created_at                TEXT NOT NULL
);

CREATE INDEX idx_broker_clusters_ws ON broker_clusters(workspace_id, name);
