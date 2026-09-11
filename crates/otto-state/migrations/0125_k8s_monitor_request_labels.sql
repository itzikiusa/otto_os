-- Kubernetes monitoring: opt-in retention of per-request `path` + `method`
-- labels on the request/latency counters (fleet dashboard route drill-down).
ALTER TABLE k8s_monitor_configs ADD COLUMN request_labels INTEGER NOT NULL DEFAULT 0;
