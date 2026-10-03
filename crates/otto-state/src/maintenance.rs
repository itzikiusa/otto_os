//! SQLite housekeeping for `otto.db` (14-daemon-perf P3).
//!
//! WAL + `synchronous=NORMAL` alone never give space back: the WAL grew to
//! 206 MB with no `journal_size_limit`, the query planner had no
//! `sqlite_stat1`, and `auto_vacuum=0` left a third of the file on the
//! freelist. Two entry points:
//!
//! - [`hourly`] — cheap, run from the daemon's hourly retention loop:
//!   `PRAGMA optimize`, `wal_checkpoint(TRUNCATE)` (PASSIVE when a reader
//!   pins the log), and `incremental_vacuum(4000)` once the file has been
//!   converted to `auto_vacuum=INCREMENTAL`.
//! - [`compact`] — the one-time conversion (`auto_vacuum=INCREMENTAL` +
//!   `VACUUM`). It rewrites the whole file and holds the write lock for the
//!   duration (tens of seconds on a large DB). Admins can run it any time
//!   (`POST /admin/db/compact`); it also runs ONCE on its own when
//!   [`needs_compaction`] (> 20 % and > 64 MiB of the file on the freelist,
//!   not yet converted): at boot before the listener when the live data is
//!   small enough to finish in a couple of seconds ([`boot`]), otherwise from
//!   the hourly pass while no session is live. After the conversion the
//!   hourly `incremental_vacuum` keeps it small, so it never triggers again.
//! - [`boot`] — seeds `sqlite_stat1` (`PRAGMA optimize=0x10002`) right after
//!   migrations so the planner has statistics from the first query, not after
//!   the first hourly pass.
//!
//! sqlx's SQLite driver runs every connection on its own worker thread, so
//! awaiting these statements never blocks a Tokio worker.

use std::time::Instant;

use otto_core::Result;
use serde::Serialize;
use sqlx::Row;

use crate::convert::dberr;
use crate::DbPool;

/// Pages freed per hourly `incremental_vacuum` step (~16 MB at 4 KiB pages):
/// bounded so one pass never holds the writer for long.
pub const INCREMENTAL_VACUUM_PAGES: i64 = 4_000;

/// `PRAGMA auto_vacuum` values.
const AUTO_VACUUM_INCREMENTAL: i64 = 2;

/// Auto-compaction trigger: at least this much on the freelist…
pub const AUTO_COMPACT_MIN_FREE_BYTES: i64 = 64 * 1024 * 1024;
/// …and at least this share of the file (percent).
pub const AUTO_COMPACT_MIN_FREE_PCT: i64 = 20;
/// Boot compacts inline (before the listener) only when the LIVE data
/// (size − free) is below this — a rewrite of ≤ 128 MiB takes ~1–2 s on SSD.
/// Bigger files wait for an idle hourly pass instead of delaying boot.
pub const BOOT_COMPACT_MAX_LIVE_BYTES: i64 = 128 * 1024 * 1024;
/// While checkpointing with TRUNCATE, wait at most this long for readers so
/// the writer connection is never parked for the full 5 s busy timeout.
const CHECKPOINT_BUSY_MS: i64 = 250;
/// The pool's normal busy timeout (db.rs), restored after the checkpoint.
const NORMAL_BUSY_MS: i64 = 5_000;

/// Whether the one-time [`compact`] should run on its own: not converted yet,
/// and both > [`AUTO_COMPACT_MIN_FREE_BYTES`] and
/// > [`AUTO_COMPACT_MIN_FREE_PCT`] % of the file are free pages.
pub fn needs_compaction(s: &DbStats) -> bool {
    needs_compaction_over(s, AUTO_COMPACT_MIN_FREE_BYTES)
}

fn needs_compaction_over(s: &DbStats, min_free_bytes: i64) -> bool {
    s.auto_vacuum != AUTO_VACUUM_INCREMENTAL
        && s.page_count > 0
        && s.free_bytes() > min_free_bytes
        && s.freelist_count * 100 > s.page_count * AUTO_COMPACT_MIN_FREE_PCT
}

/// What [`boot`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BootMaintenance {
    pub stats: DbStats,
    /// Compacted inline (small file).
    pub compacted: Option<CompactReport>,
    /// Needs compaction but too big for boot — the hourly pass does it idle.
    pub deferred_compaction: bool,
}

/// Boot-time pass (before the listener): seed planner statistics, and run
/// the one-time compaction inline if it is needed AND cheap.
pub async fn boot(pool: &DbPool) -> Result<BootMaintenance> {
    boot_with(pool, AUTO_COMPACT_MIN_FREE_BYTES).await
}

async fn boot_with(pool: &DbPool, min_free_bytes: i64) -> Result<BootMaintenance> {
    // `analysis_limit` bounds ANALYZE per index (approximate stats are
    // enough for the planner); 0x10002 = analyze tables that need it, even
    // on a fresh connection that ran no queries yet.
    let mut conn = pool.acquire().await.map_err(dberr("boot: acquire"))?;
    sqlx::query("PRAGMA analysis_limit = 1000")
        .execute(&mut *conn)
        .await
        .map_err(dberr("boot: analysis_limit"))?;
    sqlx::query("PRAGMA optimize = 0x10002")
        .execute(&mut *conn)
        .await
        .map_err(dberr("boot: optimize"))?;
    drop(conn);
    let s = stats(pool).await?;
    if !needs_compaction_over(&s, min_free_bytes) {
        return Ok(BootMaintenance {
            stats: s,
            compacted: None,
            deferred_compaction: false,
        });
    }
    if s.size_bytes() - s.free_bytes() <= BOOT_COMPACT_MAX_LIVE_BYTES {
        let r = compact(pool).await?;
        return Ok(BootMaintenance {
            stats: s,
            compacted: Some(r),
            deferred_compaction: false,
        });
    }
    Ok(BootMaintenance {
        stats: s,
        compacted: None,
        deferred_compaction: true,
    })
}

/// Size snapshot of the database file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct DbStats {
    pub page_size: i64,
    pub page_count: i64,
    pub freelist_count: i64,
    /// 0 = none, 1 = full, 2 = incremental.
    pub auto_vacuum: i64,
}

impl DbStats {
    pub fn size_bytes(&self) -> i64 {
        self.page_size * self.page_count
    }
    pub fn free_bytes(&self) -> i64 {
        self.page_size * self.freelist_count
    }
}

/// What one [`hourly`] pass did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MaintenanceReport {
    /// `true` when the TRUNCATE checkpoint was blocked and PASSIVE ran instead.
    pub checkpoint_fell_back: bool,
    /// Pages reclaimed by `incremental_vacuum` (0 when not converted).
    pub vacuumed_pages: i64,
}

/// Result of the one-time [`compact`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CompactReport {
    pub before_bytes: i64,
    pub after_bytes: i64,
    pub freed_bytes: i64,
    pub duration_ms: u64,
    pub auto_vacuum: i64,
}

async fn pragma_i64(pool: &DbPool, name: &str) -> Result<i64> {
    // `name` is a compile-time constant from this module, never input.
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!("PRAGMA {name}")))
        .fetch_one(pool.writer())
        .await
        .map_err(dberr("pragma read"))
}

/// Current size / freelist / auto-vacuum mode.
pub async fn stats(pool: &DbPool) -> Result<DbStats> {
    Ok(DbStats {
        page_size: pragma_i64(pool, "page_size").await?,
        page_count: pragma_i64(pool, "page_count").await?,
        freelist_count: pragma_i64(pool, "freelist_count").await?,
        auto_vacuum: pragma_i64(pool, "auto_vacuum").await?,
    })
}

/// Hourly maintenance. Every step is best-effort-safe: none rewrites the
/// file or changes its mode.
pub async fn hourly(pool: &DbPool) -> Result<MaintenanceReport> {
    let mut report = MaintenanceReport::default();
    sqlx::query("PRAGMA optimize")
        .execute(pool.writer())
        .await
        .map_err(dberr("pragma optimize"))?;
    // TRUNCATE resets the WAL to zero bytes; it returns busy=1 when a reader
    // still needs the log — then a PASSIVE checkpoint copies what it can.
    // A short per-connection busy timeout: TRUNCATE waits for readers, and
    // with the pool's 5 s it could park this writer connection that long.
    {
        let mut conn = pool.acquire().await.map_err(dberr("checkpoint: acquire"))?;
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "PRAGMA busy_timeout = {CHECKPOINT_BUSY_MS}"
        )))
        .execute(&mut *conn)
        .await
        .map_err(dberr("checkpoint: busy_timeout"))?;
        let res = sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .fetch_one(&mut *conn)
            .await;
        let busy: i64 = match &res {
            Ok(row) => row.try_get(0).unwrap_or(0),
            Err(_) => 1,
        };
        if busy != 0 {
            report.checkpoint_fell_back = true;
            let _ = sqlx::query("PRAGMA wal_checkpoint(PASSIVE)")
                .execute(&mut *conn)
                .await;
        }
        // Always restore before the connection returns to the pool.
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "PRAGMA busy_timeout = {NORMAL_BUSY_MS}"
        )))
        .execute(&mut *conn)
        .await
        .map_err(dberr("checkpoint: busy_timeout restore"))?;
    }
    if pragma_i64(pool, "auto_vacuum").await? == AUTO_VACUUM_INCREMENTAL {
        let before = pragma_i64(pool, "freelist_count").await?;
        if before > 0 {
            // One connection for the vacuum: `incremental_vacuum` returns a
            // row per step and only completes once they are all read.
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "PRAGMA incremental_vacuum({INCREMENTAL_VACUUM_PAGES})"
            )))
            .fetch_all(pool.writer())
            .await
            .map_err(dberr("incremental vacuum"))?;
            let after = pragma_i64(pool, "freelist_count").await?;
            report.vacuumed_pages = (before - after).max(0);
        }
    }
    Ok(report)
}

/// The one-time compaction: switch the file to `auto_vacuum=INCREMENTAL`
/// (only takes effect through a `VACUUM`) and rewrite it, returning the
/// before/after size. Holds the write lock for the whole rewrite — callers
/// run it from an explicit, confirmed admin action, or automatically only
/// when [`needs_compaction`] and nothing else is writing (boot, idle hour).
pub async fn compact(pool: &DbPool) -> Result<CompactReport> {
    let before = stats(pool).await?;
    let started = Instant::now();
    // Same connection for both statements: the pragma is per-connection
    // state until the VACUUM persists it into the file header.
    let mut conn = pool.acquire().await.map_err(dberr("compact: acquire"))?;
    sqlx::query("PRAGMA auto_vacuum = INCREMENTAL")
        .execute(&mut *conn)
        .await
        .map_err(dberr("compact: auto_vacuum"))?;
    sqlx::query("VACUUM")
        .execute(&mut *conn)
        .await
        .map_err(dberr("compact: vacuum"))?;
    // The rewrite went through the WAL; fold it back so the file on disk
    // (and the WAL) actually shrink now.
    let _ = sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(&mut *conn)
        .await;
    drop(conn);
    let after = stats(pool).await?;
    Ok(CompactReport {
        before_bytes: before.size_bytes(),
        after_bytes: after.size_bytes(),
        freed_bytes: (before.size_bytes() - after.size_bytes()).max(0),
        duration_ms: started.elapsed().as_millis() as u64,
        auto_vacuum: after.auto_vacuum,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};

    async fn file_pool(dir: &std::path::Path) -> DbPool {
        let opts = SqliteConnectOptions::new()
            .filename(dir.join("t.db"))
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal);
        SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap()
            .into()
    }

    #[tokio::test]
    async fn compact_converts_to_incremental_and_frees_pages() {
        let dir = tempfile::tempdir().unwrap();
        let pool = file_pool(dir.path()).await;
        sqlx::query("CREATE TABLE t (b BLOB)")
            .execute(&pool)
            .await
            .unwrap();
        for _ in 0..200 {
            sqlx::query("INSERT INTO t VALUES (zeroblob(8192))")
                .execute(&pool)
                .await
                .unwrap();
        }
        sqlx::query("DELETE FROM t").execute(&pool).await.unwrap();
        let before = stats(&pool).await.unwrap();
        assert_eq!(before.auto_vacuum, 0);
        assert!(before.freelist_count > 100);
        // Hourly maintenance never vacuums a non-converted file.
        let h = hourly(&pool).await.unwrap();
        assert_eq!(h.vacuumed_pages, 0);
        assert_eq!(
            stats(&pool).await.unwrap().freelist_count,
            before.freelist_count
        );

        let r = compact(&pool).await.unwrap();
        assert_eq!(r.auto_vacuum, AUTO_VACUUM_INCREMENTAL);
        assert!(r.freed_bytes > 0, "{r:?}");
        assert_eq!(stats(&pool).await.unwrap().freelist_count, 0);

        // Once converted, the hourly pass reclaims new free pages.
        for _ in 0..50 {
            sqlx::query("INSERT INTO t VALUES (zeroblob(8192))")
                .execute(&pool)
                .await
                .unwrap();
        }
        sqlx::query("DELETE FROM t").execute(&pool).await.unwrap();
        assert!(stats(&pool).await.unwrap().freelist_count > 0);
        let h = hourly(&pool).await.unwrap();
        assert!(h.vacuumed_pages > 0);
        assert_eq!(stats(&pool).await.unwrap().freelist_count, 0);
    }

    fn st(page_count: i64, freelist_count: i64, auto_vacuum: i64) -> DbStats {
        DbStats {
            page_size: 4096,
            page_count,
            freelist_count,
            auto_vacuum,
        }
    }

    #[test]
    fn auto_compaction_trigger() {
        // The live install: 223,437 pages, 79,559 free (36 %, 310 MB).
        assert!(needs_compaction(&st(223_437, 79_559, 0)));
        // Already converted → never again.
        assert!(!needs_compaction(&st(223_437, 79_559, 2)));
        // 30 % free but only ~12 MB → not worth a rewrite.
        assert!(!needs_compaction(&st(10_000, 3_000, 0)));
        // 100 MB free but only 10 % of a big file → not yet.
        assert!(!needs_compaction(&st(250_000, 25_000, 0)));
        assert!(!needs_compaction(&st(0, 0, 0)));
    }

    async fn fragmented(pool: &DbPool) {
        sqlx::query("CREATE TABLE t (id INTEGER PRIMARY KEY, k TEXT, b BLOB)")
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("CREATE INDEX t_k ON t(k)")
            .execute(pool)
            .await
            .unwrap();
        for i in 0..200 {
            sqlx::query("INSERT INTO t (k, b) VALUES (?, zeroblob(8192))")
                .bind(format!("k{}", i % 7))
                .execute(pool)
                .await
                .unwrap();
        }
        sqlx::query("DELETE FROM t WHERE id > 20")
            .execute(pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn boot_seeds_planner_stats_and_compacts_small_fragmented_db() {
        let dir = tempfile::tempdir().unwrap();
        let pool = file_pool(dir.path()).await;
        fragmented(&pool).await;
        // Default thresholds: this tiny file is never auto-compacted.
        let b = boot(&pool).await.unwrap();
        assert!(b.compacted.is_none() && !b.deferred_compaction);
        let stat1: i64 =
            sqlx::query_scalar("SELECT count(*) FROM sqlite_master WHERE name = 'sqlite_stat1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(stat1, 1, "PRAGMA optimize=0x10002 must create sqlite_stat1");
        // With a low threshold the same file compacts inline at boot.
        let b = boot_with(&pool, 0).await.unwrap();
        let r = b.compacted.expect("compacted at boot");
        assert_eq!(r.auto_vacuum, AUTO_VACUUM_INCREMENTAL);
        assert!(r.freed_bytes > 0);
        // …and never again once converted.
        let b = boot_with(&pool, 0).await.unwrap();
        assert!(b.compacted.is_none() && !b.deferred_compaction);
    }
}
