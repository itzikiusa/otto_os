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

use std::path::{Path, PathBuf};
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

// ---------------------------------------------------------------------------
// Offline compaction at boot (perf2/03 N1)
// ---------------------------------------------------------------------------
//
// An in-place `VACUUM` of a large file holds the write lock for the whole
// rewrite, so a running daemon can only do it for SMALL files (or on an
// explicit "Compact now"). Big files are compacted at the next start, BEFORE
// the pool opens — nothing can be writing then, so nobody stalls and no write
// made after the snapshot can be lost (a copy taken while the daemon runs
// would miss every later write, and the daemon writes constantly):
//
// 1. `VACUUM INTO '<db>.compact-tmp'` with `auto_vacuum=INCREMENTAL` (a
//    sequential copy — measured ~0.7 s for a 400 MB file on this class of
//    SSD, vs ~2 s for an in-place VACUUM through the WAL);
// 2. verify the copy: `quick_check`, the same schema, the same row count in
//    every table, `auto_vacuum=2`;
// 3. swap: write `<db>.compact-swapped`, rename `<db>` → `<db>.precompact`,
//    the copy → `<db>`;
// 4. the daemon opens + migrates the new file and only then calls
//    [`confirm_offline_compaction`], which deletes `<db>.precompact`. A boot
//    that dies before that leaves the `compact-swapped` marker, and the next
//    start rolls back to the old file (and won't retry automatically).
//
// It runs automatically when [`needs_compaction`] (not converted yet, >20 %
// and >64 MiB free) or when requested ([`request_compaction`], the Settings
// "Compact at next restart" button).

/// Explicit request marker ("Compact at next restart").
const REQUEST_SUFFIX: &str = ".compact-requested";
/// The `VACUUM INTO` target while it is built and verified.
const TMP_SUFFIX: &str = ".compact-tmp";
/// The pre-compaction file, kept until the new one opened cleanly.
const PRECOMPACT_SUFFIX: &str = ".precompact";
/// Present between the swap and [`confirm_offline_compaction`].
const SWAPPED_SUFFIX: &str = ".compact-swapped";
/// Written when an automatic attempt failed or was rolled back: no further
/// AUTOMATIC attempts (an explicit request still runs).
const FAILED_SUFFIX: &str = ".compact-failed";
/// Automatic offline compaction is skipped above this much live data (a
/// multi-GB copy at boot is better left to an explicit request).
pub const BOOT_OFFLINE_COMPACT_MAX_LIVE_BYTES: i64 = 4 * 1024 * 1024 * 1024;
/// Conservative throughput for duration estimates (`VACUUM INTO` + checks).
const OFFLINE_COMPACT_BYTES_PER_SEC: i64 = 150 * 1024 * 1024;

fn sibling(db: &Path, suffix: &str) -> PathBuf {
    let mut s = db.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// Rough wall time of an offline compaction of `s` (ms).
pub fn estimate_offline_compact_ms(s: &DbStats) -> u64 {
    let live = (s.size_bytes() - s.free_bytes()).max(0);
    ((live * 1000) / OFFLINE_COMPACT_BYTES_PER_SEC).max(500) as u64
}

/// Ask for an offline compaction at the next start (idempotent). Also clears
/// a previous automatic failure — an explicit request always runs.
pub fn request_compaction(db: &Path) -> Result<()> {
    let _ = std::fs::remove_file(sibling(db, FAILED_SUFFIX));
    std::fs::write(sibling(db, REQUEST_SUFFIX), chrono::Utc::now().to_rfc3339())
        .map_err(|e| otto_core::Error::Internal(format!("compaction request: {e}")))
}

/// Withdraw a [`request_compaction`] AND opt out of the automatic one (the
/// user said "not at the next start"); a later request re-enables it.
/// `true` when an explicit request was pending.
pub fn cancel_compaction_request(db: &Path) -> bool {
    let _ = std::fs::write(sibling(db, FAILED_SUFFIX), "cancelled");
    std::fs::remove_file(sibling(db, REQUEST_SUFFIX)).is_ok()
}

/// Whether an offline compaction will run at the next start: requested, or
/// needed and not blocked by an earlier failed automatic attempt.
pub fn compaction_scheduled(db: &Path, s: &DbStats) -> bool {
    sibling(db, REQUEST_SUFFIX).exists()
        || (needs_compaction(s)
            && !sibling(db, FAILED_SUFFIX).exists()
            && s.size_bytes() - s.free_bytes() <= BOOT_OFFLINE_COMPACT_MAX_LIVE_BYTES)
}

/// Result of [`offline_compact_at_boot`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OfflineOutcome {
    /// Nothing to do.
    NotNeeded,
    /// Compacted; the old file waits for [`confirm_offline_compaction`].
    Compacted(CompactReport),
    /// A previous swap never confirmed: the old file was put back.
    RolledBack,
    /// Needed but not done (reason); the daemon boots on the untouched file.
    Skipped(String),
}

fn mv_if_exists(from: &Path, to: &Path) -> std::io::Result<()> {
    match std::fs::rename(from, to) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        r => r,
    }
}

fn rm_if_exists(p: &Path) {
    let _ = std::fs::remove_file(p);
}

fn fsync_dir(db: &Path) {
    if let Some(dir) = db.parent() {
        if let Ok(d) = std::fs::File::open(dir) {
            let _ = d.sync_all();
        }
    }
}

/// Undo an unconfirmed swap: the new file (and the WAL/SHM the failed boot
/// created FOR it — a WAL must never be replayed into another file) is
/// discarded, the pre-compaction file put back.
fn roll_back(db: &Path) -> std::io::Result<()> {
    let pre = sibling(db, PRECOMPACT_SUFFIX);
    if pre.exists() {
        for sfx in ["-wal", "-shm"] {
            rm_if_exists(&sibling(db, sfx));
        }
        rm_if_exists(db);
        std::fs::rename(&pre, db)?;
    }
    // Also when the original was never moved (a crash right after its
    // WAL/SHM were set aside): they belong to `db` either way.
    for sfx in ["-wal", "-shm"] {
        mv_if_exists(
            &sibling(db, &format!("{PRECOMPACT_SUFFIX}{sfx}")),
            &sibling(db, sfx),
        )?;
    }
    rm_if_exists(&sibling(db, SWAPPED_SUFFIX));
    fsync_dir(db);
    Ok(())
}

async fn raw_conn(path: &Path) -> Result<sqlx::SqliteConnection> {
    use sqlx::ConnectOptions;
    sqlx::sqlite::SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(false)
        .busy_timeout(std::time::Duration::from_secs(5))
        .connect()
        .await
        .map_err(dberr("offline compact: open"))
}

async fn conn_i64(c: &mut sqlx::SqliteConnection, sql: &str) -> Result<i64> {
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_string()))
        .fetch_one(&mut *c)
        .await
        .map_err(dberr("offline compact: read"))
}

async fn conn_stats(c: &mut sqlx::SqliteConnection) -> Result<DbStats> {
    Ok(DbStats {
        page_size: conn_i64(c, "PRAGMA page_size").await?,
        page_count: conn_i64(c, "PRAGMA page_count").await?,
        freelist_count: conn_i64(c, "PRAGMA freelist_count").await?,
        auto_vacuum: conn_i64(c, "PRAGMA auto_vacuum").await?,
    })
}

/// Schema text + per-table row counts: what the copy must match.
async fn fingerprint(c: &mut sqlx::SqliteConnection) -> Result<(String, Vec<(String, i64)>)> {
    let schema: Vec<(String, String)> = sqlx::query_as(
        "SELECT name, COALESCE(sql, '') FROM sqlite_schema \
         WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name",
    )
    .fetch_all(&mut *c)
    .await
    .map_err(dberr("offline compact: schema"))?;
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_schema WHERE type = 'table' \
         AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )
    .fetch_all(&mut *c)
    .await
    .map_err(dberr("offline compact: tables"))?;
    let mut counts = Vec::with_capacity(tables.len());
    for t in tables {
        // Virtual tables (FTS5) answer through their shadow tables, which are
        // counted in their own right.
        let virt = schema
            .iter()
            .any(|(n, sql)| n == &t && sql.to_ascii_uppercase().contains("VIRTUAL TABLE"));
        if virt {
            continue;
        }
        let q = format!("SELECT count(*) FROM \"{}\"", t.replace('"', "\"\""));
        counts.push((t, conn_i64(c, &q).await?));
    }
    let text = schema
        .into_iter()
        .map(|(n, sql)| format!("{n}\u{1}{sql}"))
        .collect::<Vec<_>>()
        .join("\u{2}");
    Ok((text, counts))
}

/// The boot-time offline compaction — call it BEFORE the pool opens. Never
/// fails the boot: every problem degrades to [`OfflineOutcome::Skipped`]
/// with the original file untouched.
pub async fn offline_compact_at_boot(db: &Path) -> OfflineOutcome {
    offline_compact_with(db, AUTO_COMPACT_MIN_FREE_BYTES).await
}

async fn offline_compact_with(db: &Path, min_free_bytes: i64) -> OfflineOutcome {
    if sibling(db, SWAPPED_SUFFIX).exists() {
        // The last start swapped but never confirmed (open/migrate failed or
        // it crashed first): put the old file back, no automatic retry.
        return match roll_back(db) {
            Ok(()) => {
                let _ = std::fs::write(sibling(db, FAILED_SUFFIX), "rolled back");
                rm_if_exists(&sibling(db, REQUEST_SUFFIX));
                OfflineOutcome::RolledBack
            }
            Err(e) => OfflineOutcome::Skipped(format!("roll back failed: {e}")),
        };
    }
    // A confirm interrupted after removing the marker: the old file is done.
    rm_if_exists(&sibling(db, PRECOMPACT_SUFFIX));
    rm_if_exists(&sibling(db, TMP_SUFFIX));
    if !db.exists() {
        return OfflineOutcome::NotNeeded;
    }
    let requested = sibling(db, REQUEST_SUFFIX).exists();
    match offline_compact_inner(db, requested, min_free_bytes).await {
        Ok(o) => {
            if requested && !matches!(o, OfflineOutcome::Skipped(_)) {
                rm_if_exists(&sibling(db, REQUEST_SUFFIX));
            }
            o
        }
        Err(e) => {
            rm_if_exists(&sibling(db, TMP_SUFFIX));
            rm_if_exists(&sibling(db, REQUEST_SUFFIX));
            let _ = std::fs::write(sibling(db, FAILED_SUFFIX), e.to_string());
            OfflineOutcome::Skipped(e.to_string())
        }
    }
}

async fn offline_compact_inner(
    db: &Path,
    requested: bool,
    min_free_bytes: i64,
) -> Result<OfflineOutcome> {
    use sqlx::Connection;
    let started = Instant::now();
    let mut src = raw_conn(db).await?;
    let before = conn_stats(&mut src).await?;
    let live = before.size_bytes() - before.free_bytes();
    if !requested {
        if !needs_compaction_over(&before, min_free_bytes) || sibling(db, FAILED_SUFFIX).exists() {
            return Ok(OfflineOutcome::NotNeeded);
        }
        if live > BOOT_OFFLINE_COMPACT_MAX_LIVE_BYTES {
            return Ok(OfflineOutcome::Skipped(format!(
                "{live} bytes of live data — use Settings ▸ Database storage to compact"
            )));
        }
    }
    // Fold the WAL in first: the copy reads through it either way, but the
    // swap below must leave no WAL that belongs to the old file.
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(&mut src)
        .await
        .map_err(dberr("offline compact: checkpoint"))?;
    let tmp = sibling(db, TMP_SUFFIX);
    rm_if_exists(&tmp);
    // `auto_vacuum` set on the source connection is what VACUUM INTO writes
    // into the copy's header (the source file itself is not modified).
    sqlx::query("PRAGMA auto_vacuum = INCREMENTAL")
        .execute(&mut src)
        .await
        .map_err(dberr("offline compact: auto_vacuum"))?;
    sqlx::query("VACUUM INTO ?")
        .bind(tmp.to_string_lossy().to_string())
        .execute(&mut src)
        .await
        .map_err(dberr("offline compact: vacuum into"))?;
    let want = fingerprint(&mut src).await?;
    src.close().await.map_err(dberr("offline compact: close"))?;

    let mut copy = raw_conn(&tmp).await?;
    let check: String = sqlx::query_scalar("PRAGMA quick_check")
        .fetch_one(&mut copy)
        .await
        .map_err(dberr("offline compact: quick_check"))?;
    if check != "ok" {
        return Err(otto_core::Error::Internal(format!(
            "offline compact: copy failed quick_check: {check}"
        )));
    }
    let mut after = conn_stats(&mut copy).await?;
    if after.auto_vacuum != AUTO_VACUUM_INCREMENTAL {
        // Older SQLite: convert the copy in place (still offline).
        sqlx::query("PRAGMA auto_vacuum = INCREMENTAL")
            .execute(&mut copy)
            .await
            .map_err(dberr("offline compact: copy auto_vacuum"))?;
        sqlx::query("VACUUM")
            .execute(&mut copy)
            .await
            .map_err(dberr("offline compact: copy vacuum"))?;
        after = conn_stats(&mut copy).await?;
    }
    let got = fingerprint(&mut copy).await?;
    copy.close()
        .await
        .map_err(dberr("offline compact: close copy"))?;
    if got != want {
        return Err(otto_core::Error::Internal(
            "offline compact: the copy does not match the original (schema or row counts)".into(),
        ));
    }
    std::fs::File::open(&tmp)
        .and_then(|f| f.sync_all())
        .map_err(|e| otto_core::Error::Internal(format!("offline compact: fsync copy: {e}")))?;
    let wal = sibling(db, "-wal");
    if std::fs::metadata(&wal)
        .map(|m| m.len() > 0)
        .unwrap_or(false)
    {
        return Err(otto_core::Error::Internal(
            "offline compact: the WAL was not folded in; left the file untouched".into(),
        ));
    }

    // Swap. The marker goes first: from here until the daemon confirms, a
    // failed start rolls back.
    let io = |what: &str, e: std::io::Error| {
        otto_core::Error::Internal(format!("offline compact: {what}: {e}"))
    };
    std::fs::write(sibling(db, SWAPPED_SUFFIX), chrono::Utc::now().to_rfc3339())
        .map_err(|e| io("marker", e))?;
    for sfx in ["-wal", "-shm"] {
        mv_if_exists(
            &sibling(db, sfx),
            &sibling(db, &format!("{PRECOMPACT_SUFFIX}{sfx}")),
        )
        .map_err(|e| io("move wal", e))?;
    }
    if let Err(e) = std::fs::rename(db, sibling(db, PRECOMPACT_SUFFIX)) {
        let _ = roll_back(db);
        return Err(io("rename original", e));
    }
    if let Err(e) = std::fs::rename(&tmp, db) {
        let _ = roll_back(db);
        return Err(io("rename copy", e));
    }
    fsync_dir(db);
    Ok(OfflineOutcome::Compacted(CompactReport {
        before_bytes: before.size_bytes(),
        after_bytes: after.size_bytes(),
        freed_bytes: (before.size_bytes() - after.size_bytes()).max(0),
        duration_ms: started.elapsed().as_millis() as u64,
        auto_vacuum: after.auto_vacuum,
    }))
}

/// The new file opened and migrated cleanly: drop the pre-compaction file.
/// A no-op when no swap is pending. Returns whether one was confirmed.
pub fn confirm_offline_compaction(db: &Path) -> bool {
    if !sibling(db, SWAPPED_SUFFIX).exists() {
        return false;
    }
    // Marker first: if we die between the two, the next start just deletes
    // the stale `.precompact` instead of rolling back a good file.
    rm_if_exists(&sibling(db, SWAPPED_SUFFIX));
    for sfx in ["", "-wal", "-shm"] {
        rm_if_exists(&sibling(db, &format!("{PRECOMPACT_SUFFIX}{sfx}")));
    }
    fsync_dir(db);
    true
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

    async fn seeded_file(dir: &std::path::Path) -> std::path::PathBuf {
        let pool = file_pool(dir).await;
        fragmented(&pool).await;
        pool.close().await;
        dir.join("t.db")
    }

    async fn rows(db: &std::path::Path) -> (i64, i64) {
        let mut c = raw_conn(db).await.unwrap();
        let n = conn_i64(&mut c, "SELECT count(*) FROM t").await.unwrap();
        let av = conn_i64(&mut c, "PRAGMA auto_vacuum").await.unwrap();
        (n, av)
    }

    /// perf2/03 N1: the offline path converts + shrinks a fragmented file with
    /// every row intact, keeps the old file until confirmed, and a database
    /// opened on the result takes writes immediately.
    #[tokio::test]
    async fn offline_compaction_swaps_verified_copy_and_keeps_old_until_confirmed() {
        let dir = tempfile::tempdir().unwrap();
        let db = seeded_file(dir.path()).await;
        let before_len = std::fs::metadata(&db).unwrap().len();
        // Default thresholds: a tiny file is left alone.
        assert_eq!(
            offline_compact_at_boot(&db).await,
            OfflineOutcome::NotNeeded
        );

        let r = match offline_compact_with(&db, 0).await {
            OfflineOutcome::Compacted(r) => r,
            o => panic!("expected Compacted, got {o:?}"),
        };
        assert_eq!(r.auto_vacuum, AUTO_VACUUM_INCREMENTAL);
        assert!(r.freed_bytes > 0 && r.after_bytes < r.before_bytes, "{r:?}");
        assert!(std::fs::metadata(&db).unwrap().len() < before_len);
        assert_eq!(rows(&db).await, (20, AUTO_VACUUM_INCREMENTAL));
        assert!(sibling(&db, PRECOMPACT_SUFFIX).exists(), "old file kept");
        assert!(!sibling(&db, TMP_SUFFIX).exists());

        // The daemon opens the new file and writes at once.
        let pool = file_pool(dir.path()).await;
        let t = Instant::now();
        sqlx::query("INSERT INTO t (k, b) VALUES ('new', zeroblob(10))")
            .execute(&pool)
            .await
            .unwrap();
        assert!(t.elapsed() < std::time::Duration::from_millis(250));
        pool.close().await;
        assert!(confirm_offline_compaction(&db));
        assert!(!sibling(&db, PRECOMPACT_SUFFIX).exists());
        assert!(!confirm_offline_compaction(&db));
        // Converted → never again automatically.
        assert_eq!(
            offline_compact_with(&db, 0).await,
            OfflineOutcome::NotNeeded
        );
    }

    /// An unconfirmed swap (the new file failed to open/migrate) is rolled
    /// back at the next start, with no automatic retry; an explicit request
    /// runs anyway.
    #[tokio::test]
    async fn unconfirmed_swap_rolls_back_and_request_overrides_failure() {
        let dir = tempfile::tempdir().unwrap();
        let db = seeded_file(dir.path()).await;
        assert!(matches!(
            offline_compact_with(&db, 0).await,
            OfflineOutcome::Compacted(_)
        ));
        // The failed boot left a WAL for the NEW file — it must not survive.
        std::fs::write(sibling(&db, "-wal"), b"not-for-the-old-file").unwrap();
        assert_eq!(
            offline_compact_with(&db, 0).await,
            OfflineOutcome::RolledBack
        );
        assert!(!sibling(&db, "-wal").exists());
        assert!(!sibling(&db, PRECOMPACT_SUFFIX).exists());
        assert_eq!(rows(&db).await, (20, 0), "original file is back");
        // No automatic retry after a rollback…
        assert_eq!(
            offline_compact_with(&db, 0).await,
            OfflineOutcome::NotNeeded
        );
        // …but the explicit button runs it (threshold irrelevant).
        request_compaction(&db).unwrap();
        let st = DbStats {
            page_size: 4096,
            page_count: 10,
            freelist_count: 0,
            auto_vacuum: 0,
        };
        assert!(compaction_scheduled(&db, &st));
        assert!(matches!(
            offline_compact_at_boot(&db).await,
            OfflineOutcome::Compacted(_)
        ));
        assert!(!sibling(&db, REQUEST_SUFFIX).exists());
        assert!(confirm_offline_compaction(&db));
        assert_eq!(rows(&db).await, (20, AUTO_VACUUM_INCREMENTAL));
        // Cancel withdraws a request and suppresses the automatic pass.
        request_compaction(&db).unwrap();
        assert!(cancel_compaction_request(&db));
        assert!(!compaction_scheduled(&db, &st));
        assert!(!cancel_compaction_request(&db));
    }

    #[test]
    fn offline_estimate_scales_with_live_data() {
        // The live install: 593 MB live → a few seconds, not minutes.
        let ms = estimate_offline_compact_ms(&st(223_437, 78_607, 0));
        assert!((2_000..10_000).contains(&ms), "{ms}");
        assert_eq!(estimate_offline_compact_ms(&st(10, 0, 0)), 500);
    }
}
