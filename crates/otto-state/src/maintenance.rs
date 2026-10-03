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
//!   duration (tens of seconds on a large DB), so it only ever runs behind an
//!   explicit, confirmed admin action (`POST /admin/db/compact`) — never
//!   automatically.
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
    let row = sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .fetch_one(pool.writer())
        .await
        .map_err(dberr("wal checkpoint"))?;
    let busy: i64 = row.try_get(0).unwrap_or(0);
    if busy != 0 {
        report.checkpoint_fell_back = true;
        sqlx::query("PRAGMA wal_checkpoint(PASSIVE)")
            .execute(pool.writer())
            .await
            .map_err(dberr("wal checkpoint passive"))?;
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
/// gate it behind an explicit, confirmed admin action.
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
}
