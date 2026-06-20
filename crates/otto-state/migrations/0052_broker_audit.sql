-- Audit log for broker write operations (produce, delete-topic, alter-config,
-- offset-reset). Surfaced as an optional "recent writes" list in the Groups UI.
CREATE TABLE broker_write_audit (
    id          TEXT PRIMARY KEY,
    cluster_id  TEXT NOT NULL,
    user_id     TEXT NOT NULL REFERENCES users(id),
    operation   TEXT NOT NULL,   -- 'produce' | 'delete_topic' | 'alter_config' | 'offset_reset'
    detail      TEXT NOT NULL,   -- JSON: topic, group, partition counts etc.
    performed_at TEXT NOT NULL
);
CREATE INDEX idx_broker_write_audit_cluster ON broker_write_audit(cluster_id, performed_at);
