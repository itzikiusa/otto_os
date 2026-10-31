//! Run-history retention (perf W6): the iterations of finished goal loops had
//! no delete path at all. Kept in its own block of the retention job:
//! one window, [`RetentionPolicy::run_history_days`](super::RetentionPolicy).
//! OFF by default (`0` = keep forever): these are user records, so the block
//! only runs once someone sets a window (floored at [`MIN_RUN_HISTORY_DAYS`]).
//!
//! Safety rules, on top of the module's (cutoff-bounded, batched, paused):
//! - only TERMINAL rows are touched — a running/paused/blocked goal loop keeps
//!   everything whatever its age;
//! - a finished goal loop keeps its LAST iteration (the outcome its detail
//!   page shows); only the earlier ones go.

use super::{RetentionRepo, BATCH, BATCH_PAUSE};
use crate::convert::dberr;
use otto_core::Result;

/// Smallest `run_history_days`.
pub const MIN_RUN_HISTORY_DAYS: i64 = 14;
/// Default `run_history_days`: `0` = off (keep run history forever; opt-in).
pub const DEFAULT_RUN_HISTORY_DAYS: i64 = 0;

/// Rows removed by one run-history pass.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RunHistoryReport {
    pub goal_loop_iterations: u64,
}

impl RunHistoryReport {
    pub fn total(&self) -> u64 {
        self.goal_loop_iterations
    }
}

impl RetentionRepo {
    /// One run-history pass over everything older than `cutoff` (RFC 3339).
    pub(super) async fn prune_run_history(&self, cutoff: &str) -> Result<RunHistoryReport> {
        Ok(RunHistoryReport {
            goal_loop_iterations: self.prune_goal_loop_iterations(cutoff).await?,
        })
    }

    /// Every iteration but the last of goal loops that finished (terminal)
    /// before `cutoff`.
    async fn prune_goal_loop_iterations(&self, cutoff: &str) -> Result<u64> {
        let q = format!(
            "DELETE FROM goal_loop_iterations WHERE rowid IN \
             (SELECT i.rowid FROM goal_loops l \
              JOIN goal_loop_iterations i ON i.loop_id = l.id \
              WHERE l.finished_at IS NOT NULL AND l.finished_at < ? \
                AND l.status IN ('succeeded','exhausted','failed','stopped') \
                AND i.idx < (SELECT MAX(m.idx) FROM goal_loop_iterations m \
                             WHERE m.loop_id = l.id) \
              LIMIT {BATCH})"
        );
        let mut total = 0u64;
        loop {
            let n = sqlx::query(sqlx::AssertSqlSafe(q.as_str()))
                .bind(cutoff)
                .execute(&self.pool)
                .await
                .map_err(dberr("retention: goal_loop_iterations"))?
                .rows_affected();
            total += n;
            if n < BATCH as u64 {
                return Ok(total);
            }
            tokio::time::sleep(BATCH_PAUSE).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{RetentionPolicy, RetentionRepo};
    use crate::convert::fmt;
    use crate::DbPool;
    use chrono::{Duration, Utc};

    async fn mem_pool() -> DbPool {
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(false);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool.into()
    }

    /// The default policy with the (opt-in) run-history window switched on.
    fn opted_in() -> RetentionPolicy {
        RetentionPolicy {
            run_history_days: 90,
            ..RetentionPolicy::default()
        }
    }

    fn ago(days: i64) -> String {
        fmt(Utc::now() - Duration::days(days))
    }

    async fn ids(pool: &DbPool, sql: &str) -> Vec<String> {
        sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
            .fetch_all(pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn finished_loops_keep_last_iteration() {
        let pool = mem_pool().await;
        for (id, status, fin) in [
            ("done-old", "succeeded", Some(ago(200))),
            ("done-young", "failed", Some(ago(2))),
            ("live", "running", None),
        ] {
            sqlx::query(
                "INSERT INTO goal_loops (id, workspace_id, name, repo_path, definition_json, \
                 limits_json, config_json, status, created_by, created_at, updated_at, \
                 finished_at) VALUES (?, 'w', 'n', '/r', '{}', '{}', '{}', ?, 'u', ?, ?, ?)",
            )
            .bind(id)
            .bind(status)
            .bind(ago(300))
            .bind(ago(300))
            .bind(fin)
            .execute(&pool)
            .await
            .unwrap();
            for idx in 1..=3 {
                sqlx::query(
                    "INSERT INTO goal_loop_iterations (id, loop_id, workspace_id, idx, status, \
                     started_at) VALUES (?, ?, 'w', ?, 'done', ?)",
                )
                .bind(format!("{id}-{idx}"))
                .bind(id)
                .bind(idx)
                .bind(ago(300))
                .execute(&pool)
                .await
                .unwrap();
            }
        }
        let r = RetentionRepo::new(pool.clone())
            .prune(&opted_in())
            .await
            .unwrap();
        assert_eq!(r.run_history.goal_loop_iterations, 2);
        let iters = ids(&pool, "SELECT id FROM goal_loop_iterations ORDER BY id").await;
        assert_eq!(
            iters,
            [
                "done-old-3",
                "done-young-1",
                "done-young-2",
                "done-young-3",
                "live-1",
                "live-2",
                "live-3"
            ]
        );
        // A second pass finds nothing more.
        let again = RetentionRepo::new(pool.clone())
            .prune(&opted_in())
            .await
            .unwrap();
        assert_eq!(again.run_history.total(), 0);
    }

    #[test]
    fn run_history_is_opt_in_with_a_floor() {
        let p = RetentionPolicy::from_setting(Some(&serde_json::json!({"run_history_days": 1})));
        assert_eq!(p.run_history_days, super::MIN_RUN_HISTORY_DAYS);
        // Off by default, and 0 / negative mean "keep forever".
        assert_eq!(RetentionPolicy::default().run_history_days, 0);
        assert_eq!(super::DEFAULT_RUN_HISTORY_DAYS, 0);
        for off in [0, -5] {
            let p =
                RetentionPolicy::from_setting(Some(&serde_json::json!({"run_history_days": off})));
            assert_eq!(p.run_history_days, 0);
        }
    }

    #[tokio::test]
    async fn default_policy_keeps_run_history() {
        let pool = mem_pool().await;
        sqlx::query(
            "INSERT INTO goal_loops (id, workspace_id, name, repo_path, definition_json, \
             limits_json, config_json, status, created_by, created_at, updated_at, \
             finished_at) VALUES ('l1', 'w', 'n', '/r', '{}', '{}', '{}', 'succeeded', 'u', ?1, ?1, ?1)",
        )
        .bind(ago(400))
        .execute(&pool)
        .await
        .unwrap();
        for idx in 1..=2 {
            sqlx::query(
                "INSERT INTO goal_loop_iterations (id, loop_id, workspace_id, idx, status, \
                 started_at) VALUES (?, 'l1', 'w', ?, 'done', ?)",
            )
            .bind(format!("l1-{idx}"))
            .bind(idx)
            .bind(ago(400))
            .execute(&pool)
            .await
            .unwrap();
        }
        let r = RetentionRepo::new(pool.clone())
            .prune(&RetentionPolicy::default())
            .await
            .unwrap();
        assert_eq!(r.run_history.total(), 0);
        let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM goal_loop_iterations")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(left, 2);
    }
}
