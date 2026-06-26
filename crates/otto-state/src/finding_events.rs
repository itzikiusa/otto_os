//! Audit trail for review findings — one immutable row per action/transition.
//! This is the spine of "closing the loop with evidence": every triage action
//! (fix/verify/jira/false-positive/approval/repo-rule/regression-test/…) appends
//! a timestamped, attributed event with its concrete artifact in `detail`.

use chrono::Utc;
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt};
use otto_core::finding::FindingEvent;
use otto_core::{new_id, Result};

#[derive(Clone)]
pub struct FindingEventsRepo {
    pool: SqlitePool,
}

impl FindingEventsRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Append one audit event. `detail` carries action-specific evidence
    /// (`{session_id?,commit?,test?,jira_key?,evidence?,note?,comment_id?}`).
    #[allow(clippy::too_many_arguments)]
    pub async fn append(
        &self,
        finding_id: &str,
        workspace_id: &str,
        kind: &str,
        actor: &str,
        from_status: Option<&str>,
        to_status: Option<&str>,
        detail: serde_json::Value,
    ) -> Result<FindingEvent> {
        let id = new_id();
        let now = fmt(Utc::now());
        let detail_json = serde_json::to_string(&detail).unwrap_or_else(|_| "{}".to_string());
        sqlx::query(
            "INSERT INTO finding_events \
             (id, finding_id, workspace_id, kind, actor, from_status, to_status, detail_json, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(finding_id)
        .bind(workspace_id)
        .bind(kind)
        .bind(actor)
        .bind(from_status)
        .bind(to_status)
        .bind(&detail_json)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("append finding event"))?;
        self.get(&id).await
    }

    async fn get(&self, id: &str) -> Result<FindingEvent> {
        let row = sqlx::query("SELECT * FROM finding_events WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("get finding event"))?;
        Self::row(&row)
    }

    /// The full event timeline for a finding, oldest first.
    pub async fn list_for_finding(&self, finding_id: &str) -> Result<Vec<FindingEvent>> {
        let rows = sqlx::query(
            "SELECT * FROM finding_events WHERE finding_id = ? ORDER BY created_at, id",
        )
        .bind(finding_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list finding events"))?;
        rows.iter().map(Self::row).collect()
    }

    fn row(r: &sqlx::sqlite::SqliteRow) -> Result<FindingEvent> {
        let detail_raw: String = r.try_get("detail_json").unwrap_or_else(|_| "{}".to_string());
        let detail = serde_json::from_str(&detail_raw).unwrap_or(serde_json::Value::Null);
        Ok(FindingEvent {
            id: r.get("id"),
            finding_id: r.get("finding_id"),
            kind: r.get("kind"),
            actor: r.get("actor"),
            from_status: r.try_get("from_status").ok().flatten(),
            to_status: r.try_get("to_status").ok().flatten(),
            detail,
            created_at: r.try_get("created_at").unwrap_or_default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn mem_pool() -> SqlitePool {
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(false);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!().run(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn append_and_list_timeline() {
        let repo = FindingEventsRepo::new(mem_pool().await);
        let fid = "f1";
        repo.append(fid, "ws1", "created", "agent:grill", None, Some("open"), serde_json::json!({"comment_id": "c1"}))
            .await
            .unwrap();
        let e2 = repo
            .append(fid, "ws1", "fix_requested", "u1", Some("open"), Some("accepted"), serde_json::json!({"session_id": "s1"}))
            .await
            .unwrap();
        assert_eq!(e2.kind, "fix_requested");
        assert_eq!(e2.detail["session_id"], "s1");

        let events = repo.list_for_finding(fid).await.unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].kind, "created");
        assert_eq!(events[1].to_status.as_deref(), Some("accepted"));
        // a different finding has no events
        assert!(repo.list_for_finding("other").await.unwrap().is_empty());
    }
}
