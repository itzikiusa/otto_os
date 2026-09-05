-- Feature: Kubernetes monitoring — per-cluster probe config + last-cycle status.
-- Conventions: TEXT ids, RFC3339 TEXT timestamps, *_json TEXT blobs, INTEGER
-- booleans, FK ON DELETE CASCADE. Samples/events themselves live in the
-- embedded ClickHouse (`otto_k8s::monitor::schema`), never in SQLite.
CREATE TABLE IF NOT EXISTS k8s_monitor_configs (
    cluster_id      TEXT PRIMARY KEY REFERENCES k8s_clusters(id) ON DELETE CASCADE,
    enabled         INTEGER NOT NULL DEFAULT 0,
    interval_secs   INTEGER NOT NULL DEFAULT 60,
    -- [] => the cluster's default_namespace (required when that is NULL)
    namespaces_json TEXT NOT NULL DEFAULT '[]',
    -- [{name, port?, path, format: prometheus|json|health, mappings[], include[], exclude[], timeout_ms}]
    probes_json     TEXT NOT NULL DEFAULT '[]',
    -- [{kind: namespace|pod|workload, match} | {kind: label, selector}]
    exclusions_json TEXT NOT NULL DEFAULT '[]',
    transport       TEXT NOT NULL DEFAULT 'auto' CHECK (transport IN ('auto', 'proxy', 'port_forward')),
    concurrency     INTEGER NOT NULL DEFAULT 8,
    retention_days  INTEGER NOT NULL DEFAULT 14,
    updated_at      TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS k8s_monitor_status (
    cluster_id      TEXT PRIMARY KEY REFERENCES k8s_clusters(id) ON DELETE CASCADE,
    last_cycle_at   TEXT,
    last_ok_at      TEXT,
    last_error      TEXT NOT NULL DEFAULT '',
    transport_used  TEXT NOT NULL DEFAULT '',
    -- ok | forbidden: <kubectl message> | absent | unknown
    metrics_server  TEXT NOT NULL DEFAULT 'unknown',
    pods_seen       INTEGER NOT NULL DEFAULT 0,
    pods_scraped    INTEGER NOT NULL DEFAULT 0,
    pods_failed     INTEGER NOT NULL DEFAULT 0,
    cycle_ms        INTEGER NOT NULL DEFAULT 0,
    -- previous cycle's pod snapshot (classification diff input)
    snapshot_json   TEXT NOT NULL DEFAULT '{}'
);
