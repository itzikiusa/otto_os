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

// --- Run history (perf W6) ---------------------------------------------------
mod runs;
pub use runs::{RunHistoryReport, DEFAULT_RUN_HISTORY_DAYS, MIN_RUN_HISTORY_DAYS};
// -----------------------------------------------------------------------------

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
/// Default `review_retry_days` (14-daemon-perf P4).
pub const DEFAULT_REVIEW_RETRY_DAYS: i64 = 14;
/// Smallest `review_retry_days`: a finished review keeps its retry artifacts
/// at least this long.
pub const MIN_REVIEW_RETRY_DAYS: i64 = 3;
/// Smallest `notifications_max_rows` (perf §15 R4).
pub const MIN_NOTIFICATIONS_MAX_ROWS: i64 = 500;
/// Smallest `room_messages_keep_per_room` (perf §15 R4).
pub const MIN_ROOM_MESSAGES_KEEP: i64 = 500;

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
    /// `review_agent_prompts` + `review_diffs` (the durable per-agent retry
    /// artifacts): rows of a FINISHED review (`done`/`error`/`cancelled`, or
    /// whose review row is gone) older than this are pruned. A running
    /// review's artifacts are never touched.
    pub review_retry_days: i64,
    /// `notifications`: READ notices older than this are pruned.
    pub notifications_read_days: i64,
    /// `notifications`: UNREAD notices older than this are pruned too (a badge
    /// nobody cleared in this long is noise). Never below the read window.
    pub notifications_unread_days: i64,
    /// `notifications`: hard row cap — only the newest N survive, whatever
    /// their age.
    pub notifications_max_rows: i64,
    /// `agent_room_messages`: each room keeps its newest N messages. OPT-IN:
    /// `0` (the default) keeps every message — room transcripts are user
    /// records, so only a set cap (floored at [`MIN_ROOM_MESSAGES_KEEP`])
    /// deletes anything.
    pub room_messages_keep_per_room: i64,
    // --- Run history (perf W6) ---
    /// `otto_runs` (+ events), `swarm_runs` (spend rolled up first),
    /// `swarm_messages`, and all-but-the-last iteration of finished goal
    /// loops: terminal rows older than this are pruned (see
    /// `retention/runs.rs`). OPT-IN: `0` (the default) keeps them forever —
    /// these are user records, so only a set window (floored at
    /// [`MIN_RUN_HISTORY_DAYS`]) deletes anything.
    pub run_history_days: i64,
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
            review_retry_days: DEFAULT_REVIEW_RETRY_DAYS,
            notifications_read_days: 30,
            notifications_unread_days: 90,
            notifications_max_rows: 5_000,
            room_messages_keep_per_room: 0,
            // --- Run history (perf W6) ---
            run_history_days: DEFAULT_RUN_HISTORY_DAYS,
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
        self.review_retry_days = self.review_retry_days.max(MIN_REVIEW_RETRY_DAYS);
        self.notifications_read_days = self.notifications_read_days.max(MIN_DAYS);
        self.notifications_unread_days = self
            .notifications_unread_days
            .max(self.notifications_read_days);
        self.notifications_max_rows = self.notifications_max_rows.max(MIN_NOTIFICATIONS_MAX_ROWS);
        // Opt-in: 0 (or below) = keep every message; a set cap is floored.
        self.room_messages_keep_per_room = if self.room_messages_keep_per_room <= 0 {
            0
        } else {
            self.room_messages_keep_per_room.max(MIN_ROOM_MESSAGES_KEEP)
        };
        // --- Run history (perf W6) ---
        // Opt-in: 0 (or below) = keep forever; a set window is floored.
        self.run_history_days = if self.run_history_days <= 0 {
            0
        } else {
            self.run_history_days.max(MIN_RUN_HISTORY_DAYS)
        };
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
    pub review_agent_prompts: u64,
    pub review_diffs: u64,
    pub notifications: u64,
    pub room_messages: u64,
    // --- Run history (perf W6) ---
    pub run_history: RunHistoryReport,
}

impl RetentionReport {
    pub fn total(&self) -> u64 {
        self.work_events
            + self.mcp_tool_calls
            + self.mcp_call_log
            + self.audit_log
            + self.review_agent_prompts
            + self.review_diffs
            + self.notifications
            + self.room_messages
            // --- Run history (perf W6) ---
            + self.run_history.total()
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
        let review_cut = cutoff(p.review_retry_days);
        report.review_agent_prompts = self
            .delete_review_artifacts("review_agent_prompts", &review_cut)
            .await?;
        report.review_diffs = self
            .delete_review_artifacts("review_diffs", &review_cut)
            .await?;
        report.notifications = self
            .prune_notifications(
                &cutoff(p.notifications_read_days),
                &cutoff(p.notifications_unread_days),
                p.notifications_max_rows,
            )
            .await?;
        if p.room_messages_keep_per_room > 0 {
            report.room_messages = self
                .prune_room_messages(p.room_messages_keep_per_room)
                .await?;
        }
        // --- Run history (perf W6) ---
        if p.run_history_days > 0 {
            report.run_history = self.prune_run_history(&cutoff(p.run_history_days)).await?;
        }
        Ok(report)
    }

    /// Run a batched `DELETE … WHERE rowid IN (<select> LIMIT BATCH)` until a
    /// batch comes back short. `q` must carry its own `LIMIT {BATCH}`.
    async fn delete_batched(&self, q: &str, binds: &[&str], what: &'static str) -> Result<u64> {
        let mut total = 0u64;
        loop {
            let mut query = sqlx::query(sqlx::AssertSqlSafe(q));
            for b in binds {
                query = query.bind(*b);
            }
            let n = query
                .execute(&self.pool)
                .await
                .map_err(dberr(what))?
                .rows_affected();
            total += n;
            if n < BATCH as u64 {
                return Ok(total);
            }
            tokio::time::sleep(BATCH_PAUSE).await;
        }
    }

    /// `notifications` (perf §15 R4): read notices past `read_cutoff`, unread
    /// ones past `unread_cutoff`, then everything beyond the newest
    /// `max_rows`. Producers de-dupe per session/run, so without this the
    /// table grew for the life of the install.
    async fn prune_notifications(
        &self,
        read_cutoff: &str,
        unread_cutoff: &str,
        max_rows: i64,
    ) -> Result<u64> {
        let aged = self
            .delete_batched(
                &format!(
                    "DELETE FROM notifications WHERE rowid IN \
                     (SELECT rowid FROM notifications \
                      WHERE created_at < ? AND (read = 1 OR created_at < ?) LIMIT {BATCH})"
                ),
                &[read_cutoff, unread_cutoff],
                "retention: notifications",
            )
            .await?;
        let capped = self
            .delete_batched(
                &format!(
                    "DELETE FROM notifications WHERE rowid IN \
                     (SELECT rowid FROM notifications \
                      ORDER BY created_at DESC, id DESC LIMIT {BATCH} OFFSET {max_rows})"
                ),
                &[],
                "retention: notifications cap",
            )
            .await?;
        Ok(aged + capped)
    }

    /// `agent_room_messages` (perf §15 R4): each room keeps its newest `keep`
    /// messages (by insertion order, `rowid`). Rooms over the cap are found on
    /// the denormalized `agent_rooms.message_count` (maintained by
    /// `add_message`'s transaction since `0156`) — a read of the rooms table,
    /// not a `GROUP BY` count over every message each hour (perf §15 N7).
    /// Each trimmed room's count is recomputed (`last_message_at` is
    /// unaffected — only the oldest rows go).
    async fn prune_room_messages(&self, keep: i64) -> Result<u64> {
        let rooms: Vec<String> =
            sqlx::query_scalar("SELECT id FROM agent_rooms WHERE message_count > ?")
                .bind(keep)
                .fetch_all(&self.pool)
                .await
                .map_err(dberr("retention: room candidates"))?;
        let q = format!(
            "DELETE FROM agent_room_messages WHERE rowid IN \
             (SELECT rowid FROM agent_room_messages WHERE room_id = ? \
              ORDER BY rowid DESC LIMIT {BATCH} OFFSET {keep})"
        );
        let mut total = 0u64;
        for room in &rooms {
            total += self
                .delete_batched(&q, &[room.as_str()], "retention: room messages")
                .await?;
            sqlx::query(
                "UPDATE agent_rooms SET message_count = \
                 (SELECT COUNT(*) FROM agent_room_messages m WHERE m.room_id = agent_rooms.id) \
                 WHERE id = ?",
            )
            .bind(room)
            .execute(&self.pool)
            .await
            .map_err(dberr("retention: room message count"))?;
        }
        Ok(total)
    }

    /// Batched delete of a review retry-artifact table's rows older than
    /// `cutoff` whose review is terminal or gone. A `running` review keeps
    /// its artifacts whatever their age (a long run can still retry an
    /// agent). Past the window, `POST /reviews/{id}/agents/{i}/retry` falls
    /// back to its existing "prompt unavailable" path.
    async fn delete_review_artifacts(&self, table: &str, cutoff: &str) -> Result<u64> {
        // `table` is a compile-time constant from `prune`, never input.
        let q = format!(
            "DELETE FROM {table} WHERE rowid IN \
             (SELECT a.rowid FROM {table} a LEFT JOIN pr_reviews r ON r.id = a.review_id \
              WHERE a.created_at < ? \
                AND (r.id IS NULL OR r.status IN ('done','error','cancelled')) \
              LIMIT {BATCH})"
        );
        let mut total = 0u64;
        loop {
            let n = sqlx::query(sqlx::AssertSqlSafe(q.as_str()))
                .bind(cutoff)
                .execute(&self.pool)
                .await
                .map_err(dberr("retention: review artifacts"))?
                .rows_affected();
            total += n;
            if n < BATCH as u64 {
                return Ok(total);
            }
            tokio::time::sleep(BATCH_PAUSE).await;
        }
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
            let n = sqlx::query(sqlx::AssertSqlSafe(q.as_str()))
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
                let n = sqlx::query(sqlx::AssertSqlSafe(format!(
                    "DELETE FROM work_events WHERE rowid IN \
                     (SELECT rowid FROM work_events WHERE work_item_id = ? AND ts < ? \
                      LIMIT {BATCH})"
                )))
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
        sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
            .fetch_one(pool)
            .await
            .unwrap()
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
            let ids: Vec<String> =
                sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT id FROM {t}")))
                    .fetch_all(&pool)
                    .await
                    .unwrap();
            assert_eq!(ids, vec!["young".to_string()], "{t}");
        }
    }

    #[tokio::test]
    async fn review_artifacts_pruned_only_for_finished_old_reviews() {
        let pool = mem_pool().await;
        for (id, status) in [("done", "done"), ("run", "running"), ("err", "error")] {
            sqlx::query(
                "INSERT INTO pr_reviews (id, repo_id, pr_number, status, created_at) \
                 VALUES (?, 'r', 1, ?, ?)",
            )
            .bind(id)
            .bind(status)
            .bind(ago(30))
            .execute(&pool)
            .await
            .unwrap();
        }
        // (review, age days): old finished → pruned; young finished, old
        // running → kept; an orphan (review row gone) that is old → pruned.
        for (review, days) in [("done", 20), ("run", 20), ("err", 2), ("gone", 20)] {
            sqlx::query(
                "INSERT INTO review_agent_prompts (review_id, agent_index, prompt, created_at) \
                 VALUES (?, 0, 'p', ?)",
            )
            .bind(review)
            .bind(ago(days))
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO review_diffs (review_id, diff, created_at) VALUES (?, 'd', ?)",
            )
            .bind(review)
            .bind(ago(days))
            .execute(&pool)
            .await
            .unwrap();
        }
        let r = RetentionRepo::new(pool.clone())
            .prune(&RetentionPolicy::default())
            .await
            .unwrap();
        assert_eq!((r.review_agent_prompts, r.review_diffs), (2, 2));
        for t in ["review_agent_prompts", "review_diffs"] {
            let mut ids: Vec<String> =
                sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT review_id FROM {t}")))
                    .fetch_all(&pool)
                    .await
                    .unwrap();
            ids.sort();
            assert_eq!(ids, vec!["err".to_string(), "run".to_string()], "{t}");
        }
        // The floor: 1 day is raised to MIN_REVIEW_RETRY_DAYS.
        let p = RetentionPolicy::from_setting(Some(&serde_json::json!({"review_retry_days": 1})));
        assert_eq!(p.review_retry_days, MIN_REVIEW_RETRY_DAYS);
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

    /// Perf §15 R4: read notices age out at 30 d, unread at 90 d, and the
    /// table is capped at the newest `notifications_max_rows`.
    #[tokio::test]
    async fn notifications_age_windows_and_row_cap() {
        let pool = mem_pool().await;
        for (id, days, read) in [
            ("read-old", 40, 1),
            ("read-young", 5, 1),
            ("unread-mid", 40, 0),
            ("unread-ancient", 120, 0),
        ] {
            sqlx::query(
                "INSERT INTO notifications (id, created_at, read, kind, severity, title, body) \
                 VALUES (?, ?, ?, 'system', 'info', 't', 'b')",
            )
            .bind(id)
            .bind(ago(days))
            .bind(read)
            .execute(&pool)
            .await
            .unwrap();
        }
        let repo = RetentionRepo::new(pool.clone());
        let r = repo.prune(&RetentionPolicy::default()).await.unwrap();
        assert_eq!(r.notifications, 2);
        let mut ids: Vec<String> = sqlx::query_scalar("SELECT id FROM notifications")
            .fetch_all(&pool)
            .await
            .unwrap();
        ids.sort();
        assert_eq!(ids, vec!["read-young", "unread-mid"]);

        // Row cap: 1 200 fresh unread rows, cap 500 → the newest 500 stay.
        sqlx::query(
            "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 1200) \
             INSERT INTO notifications (id, created_at, read, kind, severity, title, body) \
             SELECT printf('cap-%05d', i), strftime('%Y-%m-%dT%H:%M:%S', 'now', printf('-%d seconds', 1200 - i)) || 'Z', \
                    0, 'system', 'info', 't', 'b' FROM n",
        )
        .execute(&pool)
        .await
        .unwrap();
        let policy = RetentionPolicy {
            notifications_max_rows: 1,
            ..Default::default()
        };
        repo.prune(&policy).await.unwrap();
        assert_eq!(
            count(&pool, "SELECT COUNT(*) FROM notifications").await,
            500
        );
        assert_eq!(
            count(
                &pool,
                "SELECT COUNT(*) FROM notifications WHERE id >= 'cap-00701'"
            )
            .await,
            500,
            "the newest rows survive"
        );
    }

    /// Perf §15 R4: each room keeps its newest N messages; the room's
    /// denormalized `message_count` follows the trim.
    #[tokio::test]
    async fn room_messages_capped_per_room() {
        let pool = mem_pool().await;
        for room in ["big", "small"] {
            sqlx::query(
                "INSERT INTO agent_rooms (id, workspace_id, name, created_at, updated_at) \
                 VALUES (?, 'w', ?, 'x', 'x')",
            )
            .bind(room)
            .bind(room)
            .execute(&pool)
            .await
            .unwrap();
        }
        for (room, n) in [("big", 1300), ("small", 20)] {
            sqlx::query(
                "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < ?) \
                 INSERT INTO agent_room_messages (id, room_id, author_kind, author_id, text, created_at) \
                 SELECT printf('%s-%05d', ?, i), ?, 'user', 'u', 'hi', 'x' FROM n",
            )
            .bind(n)
            .bind(room)
            .bind(room)
            .execute(&pool)
            .await
            .unwrap();
        }
        // `add_message` maintains the denormalized count; mirror it here.
        sqlx::query(
            "UPDATE agent_rooms SET message_count = \
             (SELECT COUNT(*) FROM agent_room_messages m WHERE m.room_id = agent_rooms.id)",
        )
        .execute(&pool)
        .await
        .unwrap();
        // Opt-in: the default policy never trims a room.
        let r = RetentionRepo::new(pool.clone())
            .prune(&RetentionPolicy::default())
            .await
            .unwrap();
        assert_eq!(r.room_messages, 0);
        assert_eq!(
            count(&pool, "SELECT COUNT(*) FROM agent_room_messages").await,
            1320
        );
        let probe = pool.statement_probe();
        probe.reset();
        let r = RetentionRepo::new(pool.clone())
            .prune(&RetentionPolicy {
                room_messages_keep_per_room: 1,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(r.room_messages, 800);
        // Candidates come from the denormalized count, not a GROUP BY scan.
        let stmts = probe.take();
        assert!(
            stmts.iter().all(|s| !s.contains("GROUP BY room_id")),
            "{stmts:#?}"
        );
        assert_eq!(
            count(
                &pool,
                "SELECT COUNT(*) FROM agent_room_messages WHERE room_id='big' AND id >= 'big-00801'"
            )
            .await,
            500
        );
        assert_eq!(
            count(
                &pool,
                "SELECT COUNT(*) FROM agent_room_messages WHERE room_id='small'"
            )
            .await,
            20
        );
        assert_eq!(
            count(
                &pool,
                "SELECT message_count FROM agent_rooms WHERE id='big'"
            )
            .await,
            500
        );
    }
}
