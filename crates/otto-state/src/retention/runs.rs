//! Run-history retention (perf W6): `otto_runs` (+ `otto_run_events`),
//! `swarm_runs`, `swarm_messages` and the iterations of finished goal loops
//! had no delete path at all. Kept in its own block of the retention job:
//! one window, [`RetentionPolicy::run_history_days`](super::RetentionPolicy).
//!
//! Safety rules, on top of the module's (cutoff-bounded, batched, paused):
//! - only TERMINAL rows are touched — a queued/running Run with Otto, swarm
//!   run or goal loop keeps everything whatever its age;
//! - a Run with Otto a Proof Pack references (its `proof_pack_id`, or a
//!   `proof_packs` row of kind `otto_run`) is kept;
//! - a pruned swarm run is first folded into its swarm's
//!   `pruned_runs`/`pruned_cost_usd` rollup (migration 0174), in the SAME
//!   transaction, so the lifetime budget (`swarm_spend`) never shrinks;
//! - a finished goal loop keeps its LAST iteration (the outcome its detail
//!   page shows); only the earlier ones go.

use super::{RetentionRepo, BATCH, BATCH_PAUSE};
use crate::convert::dberr;
use otto_core::Result;

/// Smallest `run_history_days`.
pub const MIN_RUN_HISTORY_DAYS: i64 = 14;
/// Default `run_history_days`.
pub const DEFAULT_RUN_HISTORY_DAYS: i64 = 90;

/// Rows removed by one run-history pass.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RunHistoryReport {
    pub otto_runs: u64,
    pub otto_run_events: u64,
    pub swarm_runs: u64,
    pub swarm_messages: u64,
    pub goal_loop_iterations: u64,
}

impl RunHistoryReport {
    pub fn total(&self) -> u64 {
        self.otto_runs
            + self.otto_run_events
            + self.swarm_runs
            + self.swarm_messages
            + self.goal_loop_iterations
    }
}

/// `?, ?, …` for `n` binds.
fn marks(n: usize) -> String {
    vec!["?"; n].join(",")
}

impl RetentionRepo {
    /// One run-history pass over everything older than `cutoff` (RFC 3339).
    pub(super) async fn prune_run_history(&self, cutoff: &str) -> Result<RunHistoryReport> {
        let mut r = RunHistoryReport::default();
        (r.otto_runs, r.otto_run_events) = self.prune_otto_runs(cutoff).await?;
        r.swarm_runs = self.prune_swarm_runs(cutoff).await?;
        r.swarm_messages = self
            .delete_older("swarm_messages", "created_at", cutoff)
            .await?;
        r.goal_loop_iterations = self.prune_goal_loop_iterations(cutoff).await?;
        Ok(r)
    }

    /// Terminal Run-with-Otto runs last updated before `cutoff`, no Proof
    /// Pack attached. Events go explicitly too (the FK cascade needs
    /// `foreign_keys`, which a tool connection may lack).
    async fn prune_otto_runs(&self, cutoff: &str) -> Result<(u64, u64)> {
        let (mut runs, mut events) = (0u64, 0u64);
        loop {
            let ids: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                "SELECT r.id FROM otto_runs r \
                 WHERE r.status IN ('completed','failed','rejected','cancelled') \
                   AND r.updated_at < ? AND r.proof_pack_id IS NULL \
                   AND NOT EXISTS (SELECT 1 FROM proof_packs p \
                                   WHERE p.work_item_kind = 'otto_run' AND p.work_item_id = r.id) \
                 LIMIT {BATCH}"
            )))
            .bind(cutoff)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("retention: otto_runs scan"))?;
            if ids.is_empty() {
                return Ok((runs, events));
            }
            let m = marks(ids.len());
            let mut tx = self
                .pool
                .begin()
                .await
                .map_err(dberr("retention: otto_runs begin"))?;
            // Re-check terminal under the write lock (a run can't leave a
            // terminal state today, but the guard is free).
            let q = format!(
                "DELETE FROM otto_runs WHERE id IN ({m}) \
                 AND status IN ('completed','failed','rejected','cancelled') RETURNING id"
            );
            let mut del = sqlx::query_scalar::<_, String>(sqlx::AssertSqlSafe(q.as_str()));
            for id in &ids {
                del = del.bind(id);
            }
            let gone = del
                .fetch_all(&mut *tx)
                .await
                .map_err(dberr("retention: otto_runs delete"))?;
            if !gone.is_empty() {
                let q = format!(
                    "DELETE FROM otto_run_events WHERE run_id IN ({})",
                    marks(gone.len())
                );
                let mut del = sqlx::query(sqlx::AssertSqlSafe(q.as_str()));
                for id in &gone {
                    del = del.bind(id);
                }
                events += del
                    .execute(&mut *tx)
                    .await
                    .map_err(dberr("retention: otto_run_events delete"))?
                    .rows_affected();
            }
            tx.commit()
                .await
                .map_err(dberr("retention: otto_runs commit"))?;
            runs += gone.len() as u64;
            if ids.len() < BATCH as usize {
                return Ok((runs, events));
            }
            tokio::time::sleep(BATCH_PAUSE).await;
        }
    }

    /// Terminal swarm runs finished before `cutoff`, rolled into their
    /// swarm's spend first (same transaction).
    async fn prune_swarm_runs(&self, cutoff: &str) -> Result<u64> {
        let mut total = 0u64;
        loop {
            let ids: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                "SELECT id FROM swarm_runs \
                 WHERE finished_at IS NOT NULL AND finished_at < ? \
                   AND status IN ('done','error','stopped') \
                 LIMIT {BATCH}"
            )))
            .bind(cutoff)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("retention: swarm_runs scan"))?;
            if ids.is_empty() {
                return Ok(total);
            }
            let m = marks(ids.len());
            let terminal = "status IN ('done','error','stopped')";
            let mut tx = self
                .pool
                .begin()
                .await
                .map_err(dberr("retention: swarm_runs begin"))?;
            let q = format!(
                "UPDATE swarms SET \
                   pruned_runs = pruned_runs + (SELECT COUNT(*) FROM swarm_runs r \
                       WHERE r.swarm_id = swarms.id AND r.id IN ({m}) AND r.{terminal}), \
                   pruned_cost_usd = pruned_cost_usd + (SELECT COALESCE(SUM(r.cost_usd), 0.0) \
                       FROM swarm_runs r \
                       WHERE r.swarm_id = swarms.id AND r.id IN ({m}) AND r.{terminal}) \
                 WHERE id IN (SELECT swarm_id FROM swarm_runs WHERE id IN ({m}))"
            );
            let mut up = sqlx::query(sqlx::AssertSqlSafe(q.as_str()));
            for _ in 0..3 {
                for id in &ids {
                    up = up.bind(id);
                }
            }
            up.execute(&mut *tx)
                .await
                .map_err(dberr("retention: swarm spend rollup"))?;
            let q = format!("DELETE FROM swarm_runs WHERE id IN ({m}) AND {terminal}");
            let mut del = sqlx::query(sqlx::AssertSqlSafe(q.as_str()));
            for id in &ids {
                del = del.bind(id);
            }
            total += del
                .execute(&mut *tx)
                .await
                .map_err(dberr("retention: swarm_runs delete"))?
                .rows_affected();
            tx.commit()
                .await
                .map_err(dberr("retention: swarm_runs commit"))?;
            if ids.len() < BATCH as usize {
                return Ok(total);
            }
            tokio::time::sleep(BATCH_PAUSE).await;
        }
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

    fn ago(days: i64) -> String {
        fmt(Utc::now() - Duration::days(days))
    }

    async fn exec(pool: &DbPool, sql: &str) {
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .execute(pool)
            .await
            .unwrap();
    }

    async fn ids(pool: &DbPool, sql: &str) -> Vec<String> {
        sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
            .fetch_all(pool)
            .await
            .unwrap()
    }

    async fn otto_run(pool: &DbPool, id: &str, status: &str, updated: &str, pack: Option<&str>) {
        sqlx::query(
            "INSERT INTO otto_runs (id, workspace_id, title, source_kind, source_ref, status, \
             proof_pack_id, created_by, created_at, updated_at) \
             VALUES (?, 'w', 't', 'manual', 'r', ?, ?, 'u', ?, ?)",
        )
        .bind(id)
        .bind(status)
        .bind(pack)
        .bind(updated)
        .bind(updated)
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO otto_run_events (id, run_id, workspace_id, kind, message, created_at) \
             VALUES (?, ?, 'w', 'note', 'm', ?)",
        )
        .bind(format!("ev-{id}"))
        .bind(id)
        .bind(updated)
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn otto_runs_only_terminal_old_and_unproven() {
        let pool = mem_pool().await;
        otto_run(&pool, "old-done", "completed", &ago(200), None).await;
        otto_run(&pool, "old-failed", "failed", &ago(120), None).await;
        otto_run(&pool, "old-live", "implementing", &ago(200), None).await;
        otto_run(&pool, "old-proven", "completed", &ago(200), Some("pp1")).await;
        otto_run(&pool, "young-done", "completed", &ago(5), None).await;
        let r = RetentionRepo::new(pool.clone())
            .prune(&RetentionPolicy::default())
            .await
            .unwrap();
        assert_eq!(r.run_history.otto_runs, 2);
        assert_eq!(r.run_history.otto_run_events, 2);
        let left = ids(&pool, "SELECT id FROM otto_runs ORDER BY id").await;
        assert_eq!(left, ["old-live", "old-proven", "young-done"]);
        let ev = ids(&pool, "SELECT run_id FROM otto_run_events ORDER BY run_id").await;
        assert_eq!(
            ev,
            ["old-live", "old-proven", "young-done"],
            "events follow runs"
        );
    }

    #[tokio::test]
    async fn swarm_runs_roll_into_spend_before_pruning() {
        let pool = mem_pool().await;
        exec(
            &pool,
            "INSERT INTO swarms (id, workspace_id, name, created_by, created_at, updated_at) \
             VALUES ('s1', 'w', 'S', 'u', '2020-01-01T00:00:00Z', '2020-01-01T00:00:00Z')",
        )
        .await;
        let run = |id: &str, status: &str, fin: Option<String>, cost: Option<f64>| {
            let (id, status) = (id.to_string(), status.to_string());
            let pool = pool.clone();
            async move {
                sqlx::query(
                    "INSERT INTO swarm_runs (id, swarm_id, workspace_id, agent_id, status, \
                     cost_usd, enqueued_at, finished_at) VALUES (?, 's1', 'w', 'a', ?, ?, ?, ?)",
                )
                .bind(&id)
                .bind(&status)
                .bind(cost)
                .bind(ago(300))
                .bind(fin)
                .execute(&pool)
                .await
                .unwrap();
            }
        };
        run("old1", "done", Some(ago(200)), Some(1.5)).await;
        run("old2", "error", Some(ago(150)), None).await;
        run("old-running", "running", None, Some(9.0)).await;
        run("young", "done", Some(ago(3)), Some(0.25)).await;
        let swarms = crate::swarm::SwarmRepo::new(pool.clone());
        let before = swarms.swarm_spend(&"s1".to_string()).await.unwrap();
        assert_eq!(before.total_runs, 4);

        let r = RetentionRepo::new(pool.clone())
            .prune(&RetentionPolicy::default())
            .await
            .unwrap();
        assert_eq!(r.run_history.swarm_runs, 2);
        let left = ids(&pool, "SELECT id FROM swarm_runs ORDER BY id").await;
        assert_eq!(left, ["old-running", "young"]);
        let after = swarms.swarm_spend(&"s1".to_string()).await.unwrap();
        assert_eq!(after.total_runs, before.total_runs, "budget never refunded");
        assert!((after.cost_usd - before.cost_usd).abs() < 1e-9);
    }

    #[tokio::test]
    async fn swarm_messages_age_out_and_finished_loops_keep_last_iteration() {
        let pool = mem_pool().await;
        for (id, ts) in [("m-old", ago(200)), ("m-young", ago(2))] {
            sqlx::query(
                "INSERT INTO swarm_messages (id, swarm_id, workspace_id, body, created_at) \
                 VALUES (?, 's', 'w', 'b', ?)",
            )
            .bind(id)
            .bind(ts)
            .execute(&pool)
            .await
            .unwrap();
        }
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
            .prune(&RetentionPolicy::default())
            .await
            .unwrap();
        assert_eq!(r.run_history.swarm_messages, 1);
        assert_eq!(r.run_history.goal_loop_iterations, 2);
        assert_eq!(
            ids(&pool, "SELECT id FROM swarm_messages").await,
            ["m-young"]
        );
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
            .prune(&RetentionPolicy::default())
            .await
            .unwrap();
        assert_eq!(again.run_history.total(), 0);
    }

    #[test]
    fn run_history_window_has_a_floor() {
        let p = RetentionPolicy::from_setting(Some(&serde_json::json!({"run_history_days": 1})));
        assert_eq!(p.run_history_days, super::MIN_RUN_HISTORY_DAYS);
        assert_eq!(
            RetentionPolicy::default().run_history_days,
            super::DEFAULT_RUN_HISTORY_DAYS
        );
    }
}
