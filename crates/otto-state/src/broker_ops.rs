//! Broker operator workflow tables: lag alerts and replay evidence.
//!
//! `broker_lag_alerts` — per-cluster topic/group alert thresholds; the metrics
//! sweep in `otto-brokers::metrics` checks these and surfaces breaches in the
//! GET response. `broker_replays` — append-only evidence rows written after each
//! DLQ/replay run.

use chrono::Utc;
use otto_core::{new_id, Id, Result};
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, dberr_unique, fmt, ts};
use chrono::{DateTime};

// ---------------------------------------------------------------------------
// Lag alerts
// ---------------------------------------------------------------------------

/// A persisted lag-alert row.
#[derive(Debug, Clone)]
pub struct LagAlertRow {
    pub id: Id,
    pub cluster_id: Id,
    pub topic: String,
    pub group_name: String,
    /// Alert fires when current lag ≥ this threshold.
    pub threshold: i64,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
}

/// Fields for a new lag alert.
pub struct NewLagAlert {
    pub cluster_id: Id,
    pub topic: String,
    pub group_name: String,
    pub threshold: i64,
}

fn row_to_alert(r: &sqlx::sqlite::SqliteRow) -> Result<LagAlertRow> {
    Ok(LagAlertRow {
        id: r.get("id"),
        cluster_id: r.get("cluster_id"),
        topic: r.get("topic"),
        group_name: r.get("group_name"),
        threshold: r.get::<i64, _>("threshold"),
        enabled: r.get::<i64, _>("enabled") != 0,
        created_at: ts(&r.get::<String, _>("created_at"))?,
    })
}

// ---------------------------------------------------------------------------
// Replay evidence
// ---------------------------------------------------------------------------

/// A persisted replay-evidence row.
#[derive(Debug, Clone)]
pub struct ReplayRow {
    pub id: Id,
    pub cluster_id: Id,
    pub source_topic: String,
    pub target_topic: String,
    /// Number of messages successfully produced to the target.
    pub count: i64,
    /// JSON array: each element is `{partition, offset, key?, outcome}`.
    pub evidence_json: String,
    pub created_at: DateTime<Utc>,
}

fn row_to_replay(r: &sqlx::sqlite::SqliteRow) -> Result<ReplayRow> {
    Ok(ReplayRow {
        id: r.get("id"),
        cluster_id: r.get("cluster_id"),
        source_topic: r.get("source_topic"),
        target_topic: r.get("target_topic"),
        count: r.get::<i64, _>("count"),
        evidence_json: r.get("evidence_json"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
    })
}

// ---------------------------------------------------------------------------
// Repository
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct BrokerOpsRepo {
    pool: SqlitePool,
}

impl BrokerOpsRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    // ---- lag alerts --------------------------------------------------------

    /// List all alerts for a cluster (enabled and disabled).
    pub async fn list_alerts(&self, cluster_id: &Id) -> Result<Vec<LagAlertRow>> {
        let rows = sqlx::query(
            "SELECT id, cluster_id, topic, group_name, threshold, enabled, created_at \
             FROM broker_lag_alerts WHERE cluster_id = ? \
             ORDER BY topic, group_name",
        )
        .bind(cluster_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list lag alerts"))?;
        rows.iter().map(row_to_alert).collect()
    }

    /// Create a new lag alert.
    pub async fn create_alert(&self, a: NewLagAlert) -> Result<LagAlertRow> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO broker_lag_alerts \
             (id, cluster_id, topic, group_name, threshold, enabled, created_at) \
             VALUES (?, ?, ?, ?, ?, 1, ?)",
        )
        .bind(&id)
        .bind(&a.cluster_id)
        .bind(&a.topic)
        .bind(&a.group_name)
        .bind(a.threshold)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr_unique(
            "create lag alert",
            "a lag alert for this cluster/topic/group already exists",
        ))?;
        self.get_alert(&id).await
    }

    /// Fetch one alert by id.
    pub async fn get_alert(&self, id: &Id) -> Result<LagAlertRow> {
        let r = sqlx::query(
            "SELECT id, cluster_id, topic, group_name, threshold, enabled, created_at \
             FROM broker_lag_alerts WHERE id = ?",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .map_err(dberr("lag alert"))?;
        row_to_alert(&r)
    }

    /// Delete an alert by id.
    pub async fn delete_alert(&self, id: &Id) -> Result<()> {
        let affected = sqlx::query("DELETE FROM broker_lag_alerts WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete lag alert"))?
            .rows_affected();
        if affected == 0 {
            return Err(otto_core::Error::NotFound(format!("lag alert {id}")));
        }
        Ok(())
    }

    // ---- replay evidence ---------------------------------------------------

    /// Append a replay evidence row.
    pub async fn record_replay(
        &self,
        cluster_id: &Id,
        source_topic: &str,
        target_topic: &str,
        count: i64,
        evidence: serde_json::Value,
    ) -> Result<ReplayRow> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO broker_replays \
             (id, cluster_id, source_topic, target_topic, count, evidence_json, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(cluster_id)
        .bind(source_topic)
        .bind(target_topic)
        .bind(count)
        .bind(evidence.to_string())
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("record replay"))?;
        self.get_replay(&id).await
    }

    pub async fn get_replay(&self, id: &Id) -> Result<ReplayRow> {
        let r = sqlx::query(
            "SELECT id, cluster_id, source_topic, target_topic, count, evidence_json, created_at \
             FROM broker_replays WHERE id = ?",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .map_err(dberr("replay"))?;
        row_to_replay(&r)
    }

    /// Recent replay rows for a cluster (newest first, capped at `limit`).
    pub async fn recent_replays(&self, cluster_id: &Id, limit: i64) -> Result<Vec<ReplayRow>> {
        let rows = sqlx::query(
            "SELECT id, cluster_id, source_topic, target_topic, count, evidence_json, created_at \
             FROM broker_replays WHERE cluster_id = ? \
             ORDER BY created_at DESC LIMIT ?",
        )
        .bind(cluster_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("recent replays"))?;
        rows.iter().map(row_to_replay).collect()
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    async fn mem_pool() -> SqlitePool {
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(false); // no users table needed for this repo
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!().run(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn lag_alert_crud() {
        let pool = mem_pool().await;
        let repo = BrokerOpsRepo::new(pool);
        let cluster_id = new_id();

        // Create.
        let a = repo
            .create_alert(NewLagAlert {
                cluster_id: cluster_id.clone(),
                topic: "orders".into(),
                group_name: "consumer-1".into(),
                threshold: 1000,
            })
            .await
            .unwrap();
        assert_eq!(a.topic, "orders");
        assert_eq!(a.threshold, 1000);
        assert!(a.enabled);

        // List.
        let list = repo.list_alerts(&cluster_id).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, a.id);

        // Delete.
        repo.delete_alert(&a.id).await.unwrap();
        assert!(repo.list_alerts(&cluster_id).await.unwrap().is_empty());

        // Delete non-existent returns NotFound.
        assert!(matches!(
            repo.delete_alert(&a.id).await,
            Err(otto_core::Error::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn replay_evidence_roundtrip() {
        let pool = mem_pool().await;
        let repo = BrokerOpsRepo::new(pool);
        let cluster_id = new_id();

        let evidence = serde_json::json!([{"partition":0,"offset":42,"outcome":"ok"}]);
        let r = repo
            .record_replay(&cluster_id, "dlq-orders", "orders", 1, evidence)
            .await
            .unwrap();
        assert_eq!(r.source_topic, "dlq-orders");
        assert_eq!(r.count, 1);

        let recent = repo.recent_replays(&cluster_id, 10).await.unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].id, r.id);
    }
}
