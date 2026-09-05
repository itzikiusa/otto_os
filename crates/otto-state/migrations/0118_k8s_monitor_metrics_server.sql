-- Kubernetes monitoring: per-cluster switch for probing metrics-server every
-- cycle (off = the call is skipped entirely; status reports "disabled").
ALTER TABLE k8s_monitor_configs ADD COLUMN metrics_server INTEGER NOT NULL DEFAULT 1;
