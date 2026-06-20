-- Broker operator workflows: lag alerts + replay evidence.
-- Migration 0055 reserved for Task B6.

CREATE TABLE broker_lag_alerts (
    id          TEXT PRIMARY KEY,
    cluster_id  TEXT NOT NULL,
    topic       TEXT NOT NULL,
    group_name  TEXT NOT NULL,
    threshold   INTEGER NOT NULL,   -- lag count that triggers a breach notice
    enabled     INTEGER NOT NULL DEFAULT 1,  -- 1 = active, 0 = paused
    created_at  TEXT NOT NULL
);
CREATE INDEX idx_broker_lag_alerts_cluster ON broker_lag_alerts(cluster_id);

CREATE TABLE broker_replays (
    id              TEXT PRIMARY KEY,
    cluster_id      TEXT NOT NULL,
    source_topic    TEXT NOT NULL,
    target_topic    TEXT NOT NULL,
    count           INTEGER NOT NULL DEFAULT 0,  -- messages replayed
    evidence_json   TEXT NOT NULL,               -- JSON array of replayed offsets + outcome
    created_at      TEXT NOT NULL
);
CREATE INDEX idx_broker_replays_cluster ON broker_replays(cluster_id, created_at);
