//! Retention for the append-only audit/event tables (`work_events`,
//! `mcp_tool_calls`, `mcp_call_log`, `audit_log`).
//!
//! These tables had no delete path at all, so they grew with every tool call
//! and work-graph signal for the life of the install (a real `otto.db` reached
//! 514 MB, ~45 % of it these four tables). The daemon runs [`RetentionRepo::prune`]
//! hourly with a [`RetentionPolicy`] read from the `data_retention` setting.
//!
//! Safety rules, enforced here rather than trusted to the caller:
//! - **Never touch a row younger than its window.** Every DELETE is bounded by
//!   a cutoff timestamp; `work_events` additionally keeps each ACTIVE item's
//!   newest `work_events_keep_per_item` rows regardless of age. An item with
//!   no event for `work_events_idle_days` is not active: its history ages out
//!   with the window like any other row (r3-07-07 — before, every item with
//!   at most N events kept them forever, so the 30-day window bounded
//!   nothing for the typical item).
//! - **Floors.** A misconfigured setting can't shrink a window below
//!   [`MIN_DAYS`] / [`MIN_AUDIT_LOG_DAYS`] / [`MIN_KEEP_PER_ITEM`].
//! - **Small, index-driven transactions.** Deletes run in batches of
//!   [`BATCH`] rows, each a bounded range on an index, with a short sleep
//!   between batches so queued interactive writers get the lock (r3-07-19:
//!   5 000-row batches back to back held it 40–130 ms at a time and made
//!   concurrent writes wait up to 504 ms). Candidate items are found by reads
//!   on the reader pool, never under the write lock.
//! - `enabled: false` turns the whole job off.

use crate::DbPool;
use chrono::{Duration, Utc};
use otto_core::Result;
use serde::{Deserialize, Serialize};

use crate::convert::{dberr, fmt};

/// Settings key holding the JSON [`RetentionPolicy`] (partial objects merge
/// over the defaults).
pub const SETTING_KEY: &str = "data_retention";

/// Smallest age window (days) any table can be configured to.
pub const MIN_DAYS: i64 = 7;
/// `audit_log` is the security audit trail — it gets a higher floor.
pub const MIN_AUDIT_LOG_DAYS: i64 = 30;
/// Smallest per-item `work_events` history kept regardless of age.
pub const MIN_KEEP_PER_ITEM: i64 = 50;
/// Rows deleted per statement (one short write transaction each).
const BATCH: i64 = 1_000;
/// Pause between batches: lets writers queued in SQLite's busy handler take
/// the lock before the pruner's next batch.
const BATCH_PAUSE: std::time::Duration = std::time::Duration::from_millis(25);
/// Default `work_events_idle_days`.
pub const DEFAULT_WORK_EVENTS_IDLE_DAYS: i64 = 90;

/// Retention windows. Defaults are conservative; see
/// `docs/features/backup-restore.md` ("Data retention").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RetentionPolicy {
    /// Master switch for the hourly prune job.
    pub enabled: bool,
    /// `work_events`: rows older than this AND outside the item's newest
    /// `work_events_keep_per_item` are pruned.
    pub work_events_days: i64,
    pub work_events_keep_per_item: i64,
    /// `work_events`: an item whose NEWEST event is older than this is idle
    /// and loses the keep-newest-N protection; its rows then age out with
    /// `work_events_days`. Never below `work_events_days`.
    pub work_events_idle_days: i64,
    /// `mcp_tool_calls` + `mcp_call_log` age window.
    pub mcp_audit_days: i64,
    /// `audit_log` age window.
    pub audit_log_days: i64,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            work_events_days: 30,
            work_events_keep_per_item: 500,
            work_events_idle_days: DEFAULT_WORK_EVENTS_IDLE_DAYS,
            mcp_audit_days: 90,
            audit_log_days: 90,
        }
    }
}

impl RetentionPolicy {
    /// Parse the setting value (missing / malformed → defaults) and apply the
    /// floors.
    pub fn from_setting(v: Option<&serde_json::Value>) -> Self {
        let p: Self = v
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        p.clamped()
    }

    /// The same policy with every window raised to its floor.
    pub fn clamped(mut self) -> Self {
        self.work_events_days = self.work_events_days.max(MIN_DAYS);
        self.work_events_keep_per_item = self.work_events_keep_per_item.max(MIN_KEEP_PER_ITEM);
        self.work_events_idle_days = self.work_events_idle_days.max(self.work_events_days);
        self.mcp_audit_days = self.mcp_audit_days.max(MIN_DAYS);
        self.audit_log_days = self.audit_log_days.max(MIN_AUDIT_LOG_DAYS);
        self
    }
}

/// Rows removed by one [`RetentionRepo::prune`] pass.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RetentionReport {
    pub work_events: u64,
    pub mcp_tool_calls: u64,
    pub mcp_call_log: u64,
    pub audit_log: u64,
}

impl RetentionReport {
    pub fn total(&self) -> u64 {
        self.work_events + self.mcp_tool_calls + self.mcp_call_log + self.audit_log
    }
}

#[derive(Clone)]
pub struct RetentionRepo {
    pool: DbPool,
}

impl RetentionRepo {
    pub fn new(pool: impl Into<DbPool>) -> Self {
        let pool: DbPool = pool.into();
        Self { pool }
    }

    /// One retention pass. The policy is re-clamped here, so no caller can
    /// delete inside the floors. A disabled policy is a no-op.
    pub async fn prune(&self, policy: &RetentionPolicy) -> Result<RetentionReport> {
        let p = policy.clone().clamped();
        let mut report = RetentionReport::default();
        if !p.enabled {
            return Ok(report);
        }
        let now = Utc::now();
        let cutoff = |days: i64| fmt(now - Duration::days(days));

        report.work_events = self
            .prune_work_events(
                &cutoff(p.work_events_days),
                &cutoff(p.work_events_idle_days),
                p.work_events_keep_per_item,
            )
            .await?;
        let mcp_cut = cutoff(p.mcp_audit_days);
        report.mcp_tool_calls = self
            .delete_older("mcp_tool_calls", "created_at", &mcp_cut)
            .await?;
        report.mcp_call_log = self
            .delete_older("mcp_call_log", "created_at", &mcp_cut)
            .await?;
        report.audit_log = self
            .delete_older("audit_log", "ts", &cutoff(p.audit_log_days))
            .await?;
        Ok(report)
    }

    /// Batched `DELETE … WHERE <col> < cutoff`. Timestamps are stored as
    /// RFC 3339 UTC (`convert::fmt`), so string order is time order.
    async fn delete_older(&self, table: &str, col: &str, cutoff: &str) -> Result<u64> {
        // `table`/`col` are compile-time constants from `prune`, never input.
        let q = format!(
            "DELETE FROM {table} WHERE rowid IN \
             (SELECT rowid FROM {table} WHERE {col} < ? LIMIT {BATCH})"
        );
        let mut total = 0u64;
        loop {
            let n = sqlx::query(&q)
                .bind(cutoff)
                .execute(&self.pool)
                .await
                .map_err(dberr("retention prune"))?
                .rows_affected();
            total += n;
            if n < BATCH as u64 {
                return Ok(total);
            }
            tokio::time::sleep(BATCH_PAUSE).await;
        }
    }

    /// `work_events`, per item that owns any row older than `cutoff` (found
    /// on `idx_work_events_ts`, a read — the reader pool, no write lock):
    ///
    /// - **active** item (newest event at or after `idle_cutoff`): delete the
    ///   rows that are BOTH older than `cutoff` AND older than the item's
    ///   `keep`-th newest event (ties at that boundary are kept); an item at
    ///   or under `keep` events loses nothing;
    /// - **idle** item (newest event before `idle_cutoff`): delete every row
    ///   older than `cutoff` — the window applies to it like to any table.
    ///
    /// Each DELETE batch is a bounded `(work_item_id, ts)` range on
    /// `idx_work_events_item`.
    async fn prune_work_events(&self, cutoff: &str, idle_cutoff: &str, keep: i64) -> Result<u64> {
        let items: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT work_item_id FROM work_events INDEXED BY idx_work_events_ts \
             WHERE ts < ?",
        )
        .bind(cutoff)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("retention: work_events scan"))?;
        let mut total = 0u64;
        for item in items {
            let newest: Option<String> =
                sqlx::query_scalar("SELECT MAX(ts) FROM work_events WHERE work_item_id = ?")
                    .bind(&item)
                    .fetch_one(&self.pool)
                    .await
                    .map_err(dberr("retention: work_events newest"))?;
            let idle = newest.as_deref().is_none_or(|n| n < idle_cutoff);
            let bound = if idle {
                cutoff.to_string()
            } else {
                let boundary: Option<String> = sqlx::query_scalar(
                    "SELECT ts FROM work_events WHERE work_item_id = ? \
                     ORDER BY ts DESC LIMIT 1 OFFSET ?",
                )
                .bind(&item)
                .bind(keep - 1)
                .fetch_optional(&self.pool)
                .await
                .map_err(dberr("retention: work_events boundary"))?;
                let Some(boundary) = boundary else {
                    continue; // at or under `keep` events
                };
                if boundary.as_str() < cutoff {
                    boundary
                } else {
                    cutoff.to_string()
                }
            };
            loop {
                let n = sqlx::query(&format!(
                    "DELETE FROM work_events WHERE rowid IN \
                     (SELECT rowid FROM work_events WHERE work_item_id = ? AND ts < ? \
                      LIMIT {BATCH})"
                ))
                .bind(&item)
                .bind(&bound)
                .execute(&self.pool)
                .await
                .map_err(dberr("retention: work_events prune"))?
                .rows_affected();
                total += n;
                if n < BATCH as u64 {
                    break;
                }
                tokio::time::sleep(BATCH_PAUSE).await;
            }
        }
        Ok(total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

    async fn mem_pool() -> DbPool {
        let opts = SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(false);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool.into()
    }

    fn ago(days: i64) -> String {
        fmt(Utc::now() - Duration::days(days))
    }

    async fn work_event(pool: &DbPool, id: &str, item: &str, ts: &str) {
        sqlx::query(
            "INSERT INTO work_events (id, work_item_id, workspace_id, ts, actor, event_type, \
             payload_json, created_at) VALUES (?, ?, 'w', ?, 'system', 'progress', '{}', ?)",
        )
        .bind(id)
        .bind(item)
        .bind(ts)
        .bind(ts)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn count(pool: &DbPool, sql: &str) -> i64 {
        sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
    }

    #[test]
    fn policy_defaults_merge_and_floors() {
        assert_eq!(
            RetentionPolicy::from_setting(None),
            RetentionPolicy::default()
        );
        let p = RetentionPolicy::from_setting(Some(&serde_json::json!({
            "mcp_audit_days": 1, "audit_log_days": 2, "work_events_keep_per_item": 3
        })));
        assert_eq!(p.mcp_audit_days, MIN_DAYS);
        assert_eq!(p.audit_log_days, MIN_AUDIT_LOG_DAYS);
        assert_eq!(p.work_events_keep_per_item, MIN_KEEP_PER_ITEM);
        assert_eq!(p.work_events_days, 30, "unspecified keys keep defaults");
        assert_eq!(p.work_events_idle_days, DEFAULT_WORK_EVENTS_IDLE_DAYS);
        // The idle window can never be shorter than the age window.
        let q = RetentionPolicy::from_setting(Some(&serde_json::json!({
            "work_events_days": 60, "work_events_idle_days": 10
        })));
        assert_eq!(q.work_events_idle_days, 60);
        // Malformed → defaults, not "delete everything".
        let bad = RetentionPolicy::from_setting(Some(&serde_json::json!("nope")));
        assert_eq!(bad, RetentionPolicy::default());
    }

    #[tokio::test]
    async fn work_events_keep_newest_per_item_and_young_rows() {
        let pool = mem_pool().await;
        // Item A: 60 old events + 5 young → only old ones beyond the newest 50 go.
        for i in 0..60 {
            work_event(&pool, &format!("a-old-{i:03}"), "A", &ago(100 + i)).await;
        }
        for i in 0..5 {
            work_event(&pool, &format!("a-new-{i}"), "A", &ago(1)).await;
        }
        // Item B: 70 young events → all kept (younger than the window).
        for i in 0..70 {
            work_event(&pool, &format!("b-{i}"), "B", &ago(2)).await;
        }
        // Item C: 10 ancient events, nothing for 400 days → IDLE: its rows
        // age out with the window even though it is under `keep` (r3-07-07).
        for i in 0..10 {
            work_event(&pool, &format!("c-{i}"), "C", &ago(400)).await;
        }
        // Item D: 10 events 40–49 days old — past the window but active
        // within the idle window → under `keep`, untouched.
        for i in 0..10 {
            work_event(&pool, &format!("d-{i}"), "D", &ago(40 + i)).await;
        }
        let policy = RetentionPolicy {
            work_events_keep_per_item: 50,
            ..Default::default()
        };
        let r = RetentionRepo::new(pool.clone())
            .prune(&policy)
            .await
            .unwrap();
        assert_eq!(r.work_events, 15 + 10);
        let a = count(
            &pool,
            "SELECT COUNT(*) FROM work_events WHERE work_item_id='A'",
        )
        .await;
        assert_eq!(a, 50);
        assert_eq!(
            count(
                &pool,
                "SELECT COUNT(*) FROM work_events WHERE id LIKE 'a-new-%'"
            )
            .await,
            5
        );
        assert_eq!(
            count(
                &pool,
                "SELECT COUNT(*) FROM work_events WHERE work_item_id='B'"
            )
            .await,
            70
        );
        assert_eq!(
            count(
                &pool,
                "SELECT COUNT(*) FROM work_events WHERE work_item_id='C'"
            )
            .await,
            0,
            "an idle item's history ages out with the window"
        );
        assert_eq!(
            count(
                &pool,
                "SELECT COUNT(*) FROM work_events WHERE work_item_id='D'"
            )
            .await,
            10,
            "a recently active item keeps its newest N"
        );
        // Idempotent.
        let again = RetentionRepo::new(pool.clone())
            .prune(&policy)
            .await
            .unwrap();
        assert_eq!(again.total(), 0);
    }

    #[tokio::test]
    async fn age_windows_for_audit_tables_and_disable_switch() {
        let pool = mem_pool().await;
        for (id, days) in [("old", 120), ("young", 10)] {
            sqlx::query(
                "INSERT INTO mcp_tool_calls (id, tool, args_json, ok, created_at) \
                 VALUES (?, 't', '{}', 1, ?)",
            )
            .bind(id)
            .bind(ago(days))
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO mcp_call_log (id, tool, decision, created_at) \
                 VALUES (?, 't', 'allowed', ?)",
            )
            .bind(id)
            .bind(ago(days))
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query("INSERT INTO audit_log (id, ts, action) VALUES (?, ?, 'x')")
                .bind(id)
                .bind(ago(days))
                .execute(&pool)
                .await
                .unwrap();
        }
        let repo = RetentionRepo::new(pool.clone());
        let off = RetentionPolicy {
            enabled: false,
            ..Default::default()
        };
        assert_eq!(repo.prune(&off).await.unwrap().total(), 0);

        let r = repo.prune(&RetentionPolicy::default()).await.unwrap();
        assert_eq!((r.mcp_tool_calls, r.mcp_call_log, r.audit_log), (1, 1, 1));
        for t in ["mcp_tool_calls", "mcp_call_log", "audit_log"] {
            let ids: Vec<String> = sqlx::query_scalar(&format!("SELECT id FROM {t}"))
                .fetch_all(&pool)
                .await
                .unwrap();
            assert_eq!(ids, vec!["young".to_string()], "{t}");
        }
    }

    #[tokio::test]
    async fn dedupe_migration_keeps_earliest_artifact_added() {
        // The 0142 migration already ran in mem_pool(); re-run its DELETE to
        // prove it is idempotent and correct on fresh duplicates.
        let pool = mem_pool().await;
        for (id, ts, payload, actor) in [
            ("d1", "2026-01-01T00:00:00+00:00", r#"{"ref":"x"}"#, "agent"),
            ("d2", "2026-01-02T00:00:00+00:00", r#"{"ref":"x"}"#, "agent"),
            ("d3", "2026-01-03T00:00:00+00:00", r#"{"ref":"x"}"#, "agent"),
            ("d4", "2026-01-02T00:00:00+00:00", r#"{"ref":"y"}"#, "agent"),
            ("d5", "2026-01-04T00:00:00+00:00", r#"{"ref":"x"}"#, "user"),
        ] {
            sqlx::query(
                "INSERT INTO work_events (id, work_item_id, workspace_id, ts, actor, event_type, \
                 payload_json, created_at) VALUES (?, 'I', 'w', ?, ?, 'artifact_added', ?, ?)",
            )
            .bind(id)
            .bind(ts)
            .bind(actor)
            .bind(payload)
            .bind(ts)
            .execute(&pool)
            .await
            .unwrap();
        }
        let sql = include_str!("../migrations/0142_work_events_dedupe_retention.sql");
        let delete = sql
            .split(';')
            .find(|s| s.contains("DELETE FROM work_events"))
            .unwrap();
        for _ in 0..2 {
            sqlx::query(delete).execute(&pool).await.unwrap();
        }
        let mut ids: Vec<String> = sqlx::query_scalar("SELECT id FROM work_events")
            .fetch_all(&pool)
            .await
            .unwrap();
        ids.sort();
        assert_eq!(ids, vec!["d1", "d4", "d5"]);
    }
}
