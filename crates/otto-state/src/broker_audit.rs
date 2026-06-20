//! Audit log for broker write operations (produce, delete-topic, alter-config,
//! offset-reset). Append-only; never mutated after insert.

use chrono::Utc;
use otto_core::{new_id, Id, Result};
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt};

#[derive(Debug, Clone)]
pub struct BrokerAuditRow {
    pub id: String,
    pub cluster_id: String,
    pub user_id: String,
    pub operation: String,
    pub detail: String,
    pub performed_at: String,
}

fn row_to_audit(r: &sqlx::sqlite::SqliteRow) -> BrokerAuditRow {
    BrokerAuditRow {
        id: r.get("id"),
        cluster_id: r.get("cluster_id"),
        user_id: r.get("user_id"),
        operation: r.get("operation"),
        detail: r.get("detail"),
        performed_at: r.get("performed_at"),
    }
}

pub struct BrokerAuditRepo {
    pool: SqlitePool,
}

impl BrokerAuditRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Append one audit record.
    pub async fn record(
        &self,
        cluster_id: &Id,
        user_id: &Id,
        operation: &str,
        detail: serde_json::Value,
    ) -> Result<()> {
        let id = new_id();
        let ts = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO broker_write_audit (id, cluster_id, user_id, operation, detail, performed_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(cluster_id)
        .bind(user_id)
        .bind(operation)
        .bind(detail.to_string())
        .bind(&ts)
        .execute(&self.pool)
        .await
        .map_err(dberr("broker audit insert"))?;
        Ok(())
    }

    /// Recent audit rows for a cluster (newest first, capped at `limit`).
    pub async fn recent(
        &self,
        cluster_id: &Id,
        limit: i64,
    ) -> Result<Vec<BrokerAuditRow>> {
        let rows = sqlx::query(
            "SELECT id, cluster_id, user_id, operation, detail, performed_at \
             FROM broker_write_audit \
             WHERE cluster_id = ? \
             ORDER BY performed_at DESC \
             LIMIT ?",
        )
        .bind(cluster_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("broker audit query"))?;
        Ok(rows.iter().map(row_to_audit).collect())
    }
}
