-- Kubernetes monitoring: per-cluster cap on prometheus series kept per pod
-- per cycle (histogram buckets are dropped first when a body overflows it).
ALTER TABLE k8s_monitor_configs ADD COLUMN series_cap INTEGER NOT NULL DEFAULT 1500;
