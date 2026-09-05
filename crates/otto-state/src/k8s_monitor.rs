//! Kubernetes monitoring — per-cluster probe configuration + the collector's
//! last-cycle status (migration 0116). One row of each per cluster, keyed by
//! `cluster_id`; both cascade with the cluster.

use chrono::Utc;
use otto_core::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt};

/// `k8s_monitor_configs` row. JSON columns are surfaced as `Value`; the
/// typed model (validation, presets) lives in `otto_k8s::monitor::probes`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct K8sMonitorConfigRow {
    pub cluster_id: String,
    pub enabled: bool,
    pub interval_secs: i64,
    pub namespaces: Value,
    pub probes: Value,
    pub exclusions: Value,
    pub transport: String,
    pub concurrency: i64,
    pub retention_days: i64,
    pub updated_at: String,
}

impl K8sMonitorConfigRow {
    /// Disabled defaults for a cluster with no saved config.
    pub fn default_for(cluster_id: &str) -> Self {
        Self {
            cluster_id: cluster_id.to_string(),
            enabled: false,
            interval_secs: 60,
            namespaces: Value::Array(vec![]),
            probes: Value::Array(vec![]),
            exclusions: Value::Array(vec![]),
            transport: "auto".into(),
            concurrency: 8,
            retention_days: 14,
            updated_at: fmt(Utc::now()),
        }
    }
}

/// `k8s_monitor_status` row — written by the collector after every cycle.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct K8sMonitorStatusRow {
    pub cluster_id: String,
    pub last_cycle_at: Option<String>,
    pub last_ok_at: Option<String>,
    pub last_error: String,
    pub transport_used: String,
    pub metrics_server: String,
    pub pods_seen: i64,
    pub pods_scraped: i64,
    pub pods_failed: i64,
    pub cycle_ms: i64,
    /// Previous cycle's pod snapshot — the classification diff input. Not
    /// serialised to API callers.
    #[serde(skip_serializing, default)]
    pub snapshot: Value,
}

impl K8sMonitorStatusRow {
    pub fn empty(cluster_id: &str) -> Self {
        Self {
            cluster_id: cluster_id.to_string(),
            last_cycle_at: None,
            last_ok_at: None,
            last_error: String::new(),
            transport_used: String::new(),
            metrics_server: "unknown".into(),
            pods_seen: 0,
            pods_scraped: 0,
            pods_failed: 0,
            cycle_ms: 0,
            snapshot: Value::Object(Default::default()),
        }
    }
}

fn json_or(s: &str, fallback: Value) -> Value {
    serde_json::from_str(s).unwrap_or(fallback)
}

fn row_to_config(r: &sqlx::sqlite::SqliteRow) -> K8sMonitorConfigRow {
    K8sMonitorConfigRow {
        cluster_id: r.get("cluster_id"),
        enabled: r.get::<i64, _>("enabled") != 0,
        interval_secs: r.get("interval_secs"),
        namespaces: json_or(&r.get::<String, _>("namespaces_json"), Value::Array(vec![])),
        probes: json_or(&r.get::<String, _>("probes_json"), Value::Array(vec![])),
        exclusions: json_or(&r.get::<String, _>("exclusions_json"), Value::Array(vec![])),
        transport: r.get("transport"),
        concurrency: r.get("concurrency"),
        retention_days: r.get("retention_days"),
        updated_at: r.get("updated_at"),
    }
}

fn row_to_status(r: &sqlx::sqlite::SqliteRow) -> K8sMonitorStatusRow {
    K8sMonitorStatusRow {
        cluster_id: r.get("cluster_id"),
        last_cycle_at: r.get("last_cycle_at"),
        last_ok_at: r.get("last_ok_at"),
        last_error: r.get("last_error"),
        transport_used: r.get("transport_used"),
        metrics_server: r.get("metrics_server"),
        pods_seen: r.get("pods_seen"),
        pods_scraped: r.get("pods_scraped"),
        pods_failed: r.get("pods_failed"),
        cycle_ms: r.get("cycle_ms"),
        snapshot: json_or(&r.get::<String, _>("snapshot_json"), Value::Object(Default::default())),
    }
}

#[derive(Clone)]
pub struct K8sMonitorRepo {
    pool: SqlitePool,
}

impl K8sMonitorRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn get_config(&self, cluster_id: &str) -> Result<Option<K8sMonitorConfigRow>> {
        let r = sqlx::query("SELECT * FROM k8s_monitor_configs WHERE cluster_id = ?")
            .bind(cluster_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("k8s monitor config"))?;
        Ok(r.as_ref().map(row_to_config))
    }

    /// Insert-or-replace the whole config; `updated_at` is set server-side.
    pub async fn upsert_config(&self, row: &K8sMonitorConfigRow) -> Result<K8sMonitorConfigRow> {
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO k8s_monitor_configs (cluster_id, enabled, interval_secs, namespaces_json,
                                              probes_json, exclusions_json, transport, concurrency,
                                              retention_days, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(cluster_id) DO UPDATE SET
                enabled = excluded.enabled, interval_secs = excluded.interval_secs,
                namespaces_json = excluded.namespaces_json, probes_json = excluded.probes_json,
                exclusions_json = excluded.exclusions_json, transport = excluded.transport,
                concurrency = excluded.concurrency, retention_days = excluded.retention_days,
                updated_at = excluded.updated_at",
        )
        .bind(&row.cluster_id)
        .bind(row.enabled as i64)
        .bind(row.interval_secs)
        .bind(row.namespaces.to_string())
        .bind(row.probes.to_string())
        .bind(row.exclusions.to_string())
        .bind(&row.transport)
        .bind(row.concurrency)
        .bind(row.retention_days)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("upsert k8s monitor config"))?;
        self.get_config(&row.cluster_id)
            .await?
            .ok_or_else(|| otto_core::Error::NotFound("k8s monitor config".into()))
    }

    /// Every enabled config (the scheduler's reconcile input), cluster-sorted.
    pub async fn list_enabled(&self) -> Result<Vec<K8sMonitorConfigRow>> {
        let rows = sqlx::query("SELECT * FROM k8s_monitor_configs WHERE enabled = 1 ORDER BY cluster_id")
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("list k8s monitor configs"))?;
        Ok(rows.iter().map(row_to_config).collect())
    }

    pub async fn get_status(&self, cluster_id: &str) -> Result<Option<K8sMonitorStatusRow>> {
        let r = sqlx::query("SELECT * FROM k8s_monitor_status WHERE cluster_id = ?")
            .bind(cluster_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("k8s monitor status"))?;
        Ok(r.as_ref().map(row_to_status))
    }

    pub async fn upsert_status(&self, row: &K8sMonitorStatusRow) -> Result<()> {
        sqlx::query(
            "INSERT INTO k8s_monitor_status (cluster_id, last_cycle_at, last_ok_at, last_error,
                                             transport_used, metrics_server, pods_seen, pods_scraped,
                                             pods_failed, cycle_ms, snapshot_json)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(cluster_id) DO UPDATE SET
                last_cycle_at = excluded.last_cycle_at, last_ok_at = excluded.last_ok_at,
                last_error = excluded.last_error, transport_used = excluded.transport_used,
                metrics_server = excluded.metrics_server, pods_seen = excluded.pods_seen,
                pods_scraped = excluded.pods_scraped, pods_failed = excluded.pods_failed,
                cycle_ms = excluded.cycle_ms, snapshot_json = excluded.snapshot_json",
        )
        .bind(&row.cluster_id)
        .bind(&row.last_cycle_at)
        .bind(&row.last_ok_at)
        .bind(&row.last_error)
        .bind(&row.transport_used)
        .bind(&row.metrics_server)
        .bind(row.pods_seen)
        .bind(row.pods_scraped)
        .bind(row.pods_failed)
        .bind(row.cycle_ms)
        .bind(row.snapshot.to_string())
        .execute(&self.pool)
        .await
        .map_err(dberr("upsert k8s monitor status"))?;
        Ok(())
    }

    /// Drop both rows (cluster removed / monitoring reset).
    pub async fn delete(&self, cluster_id: &str) -> Result<()> {
        sqlx::query("DELETE FROM k8s_monitor_configs WHERE cluster_id = ?")
            .bind(cluster_id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete k8s monitor config"))?;
        sqlx::query("DELETE FROM k8s_monitor_status WHERE cluster_id = ?")
            .bind(cluster_id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete k8s monitor status"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool_with_cluster() -> SqlitePool {
        let pool = crate::db::test_pool().await;
        sqlx::query(
            "INSERT INTO k8s_clusters (id, name, source, context_name, environment, created_at, updated_at)
             VALUES ('c1', 'C', 'imported', 'ctx', 'dev', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    #[tokio::test]
    async fn config_roundtrip_and_enabled_filter() {
        let repo = K8sMonitorRepo::new(pool_with_cluster().await);
        assert!(repo.get_config("c1").await.unwrap().is_none());
        let mut row = K8sMonitorConfigRow::default_for("c1");
        row.enabled = true;
        row.probes = serde_json::json!([{"name":"info","port":9000,"path":"/actuator/info","format":"json"}]);
        let saved = repo.upsert_config(&row).await.unwrap();
        assert!(saved.enabled);
        assert_eq!(saved.probes[0]["port"], 9000);
        assert_eq!(repo.list_enabled().await.unwrap().len(), 1);
        row.enabled = false;
        repo.upsert_config(&row).await.unwrap();
        assert!(repo.list_enabled().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn status_roundtrip_and_delete() {
        let repo = K8sMonitorRepo::new(pool_with_cluster().await);
        let mut st = K8sMonitorStatusRow::empty("c1");
        st.pods_seen = 5;
        st.metrics_server = "forbidden: x".into();
        st.snapshot = serde_json::json!({"ns/p": {"phase": "Running"}});
        repo.upsert_status(&st).await.unwrap();
        let got = repo.get_status("c1").await.unwrap().unwrap();
        assert_eq!(got.pods_seen, 5);
        assert_eq!(got.metrics_server, "forbidden: x");
        assert_eq!(got.snapshot["ns/p"]["phase"], "Running");
        repo.delete("c1").await.unwrap();
        assert!(repo.get_status("c1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn cascade_with_cluster() {
        let pool = pool_with_cluster().await;
        let repo = K8sMonitorRepo::new(pool.clone());
        repo.upsert_config(&K8sMonitorConfigRow::default_for("c1")).await.unwrap();
        sqlx::query("DELETE FROM k8s_clusters WHERE id = 'c1'").execute(&pool).await.unwrap();
        assert!(repo.get_config("c1").await.unwrap().is_none());
    }
}
