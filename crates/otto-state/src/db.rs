//! Pool bootstrap: WAL mode, foreign keys, busy timeout, embedded migrations.

use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;

use otto_core::{Error, Result};

use crate::DbPool;

/// Writer connections. Two, not one: a deferred read-only transaction or a
/// long statement on one never stalls every other write in-process, while at
/// most one connection at a time sits in SQLite's busy handler (see
/// [`crate::DbPool`]).
const WRITE_CONNECTIONS: u32 = 2;
/// Read-only connections: WAL lets all of them run beside the writer.
const READ_CONNECTIONS: u32 = 8;

/// Open (creating if needed) the Otto database at `path`, run migrations, and
/// return the split writer/reader handle ([`DbPool`]).
pub async fn open(path: &Path) -> Result<DbPool> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| Error::Internal(format!("create data dir: {e}")))?;
    }

    let opts = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))
        .map_err(|e| Error::Internal(format!("sqlite options: {e}")))?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        // WAL defaults to synchronous=FULL: an fsync (full disk barrier on
        // macOS) on EVERY commit. With one writer at a time, concurrent write
        // load queued interactive statements for seconds (observed: trivial
        // agent_trail INSERTs at 3-6s, create-session at 2-3s). NORMAL is the
        // documented safe pairing with WAL — the log survives app/OS crashes;
        // only a power-loss can drop the last few commits, never corrupt.
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5))
        // Without a limit a checkpointed WAL keeps its high-water size on
        // disk forever (206 MB observed). 64 MiB: big enough that normal
        // bursts never re-grow it, small enough to cap the waste
        // (14-daemon-perf P3; the hourly `maintenance::hourly` TRUNCATEs).
        .pragma("journal_size_limit", "67108864");
    // Readers share the file but never write: `read_only` makes a mis-routed
    // write fail loudly. Journal mode is the file's (WAL, set by the writer).
    let read_opts = opts
        .clone()
        .create_if_missing(false)
        .journal_mode(SqliteJournalMode::Wal)
        .read_only(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(WRITE_CONNECTIONS)
        .connect_with(opts)
        .await
        .map_err(|e| Error::Internal(format!("sqlite connect: {e}")))?;

    restrict_db_file_mode(path);

    let mut migrator = sqlx::migrate!();
    // Applied versions this binary doesn't know are a NEWER build's additive
    // migrations (they are append-only): a rollback to the previous app — or
    // a manual `ottod.prev` restore — must boot against that schema instead
    // of exiting with sqlx's `VersionMissing` and crash-looping under
    // KeepAlive. Checksums of the versions it DOES know are still validated.
    migrator.set_ignore_missing(true);
    // A schema migration is the one boot step that rewrites the user's whole
    // database irreversibly, and a deploy that rolls back to the previous app
    // can't un-apply it. Snapshot first — only when something is pending, so
    // an ordinary restart never pays the copy — and BEFORE the renumber
    // repairs below, so the copy is a true "before" image.
    snapshot_before_migrations(&pool, path, &migrator).await;

    // Must run BEFORE the migrator runs — repairs DBs bricked by the vault-docs
    // migration renumber before sqlx validates recorded checksums by version.
    repair_renumbered_vault_migrations(&pool).await?;
    repair_renumbered_migrations(&pool, RENUMBERED).await?;

    warn_unknown_applied_versions(&pool, &migrator).await;
    migrator
        .run(&pool)
        .await
        .map_err(|e| Error::Internal(format!("migrate: {e}")))?;

    // Lazily connected: the file (and its WAL) exist by now.
    let read = SqlitePoolOptions::new()
        .max_connections(READ_CONNECTIONS)
        .connect_lazy_with(read_opts);
    Ok(DbPool::split(read, pool))
}

/// `chmod 0600` the database and its WAL/SHM side files (best-effort): they
/// hold session transcripts, tokens' metadata and audit rows, and SQLite
/// creates them with the umask's 0644 inside a 0755 data dir.
fn restrict_db_file_mode(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let base = path.as_os_str().to_owned();
        for suffix in ["", "-wal", "-shm"] {
            let mut p = base.clone();
            p.push(suffix);
            let p = std::path::PathBuf::from(p);
            if let Ok(meta) = std::fs::metadata(&p) {
                if meta.permissions().mode() & 0o077 != 0 {
                    if let Err(e) =
                        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600))
                    {
                        tracing::warn!("chmod 0600 {}: {e}", p.display());
                    }
                }
            }
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// Applied versions in `applied` that `embedded` (this binary's migrations)
/// does not contain — a newer build's migrations. Sorted.
fn unknown_applied_versions(applied: &[i64], embedded: &[i64]) -> Vec<i64> {
    let known: std::collections::HashSet<i64> = embedded.iter().copied().collect();
    let mut unknown: Vec<i64> = applied
        .iter()
        .copied()
        .filter(|v| !known.contains(v))
        .collect();
    unknown.sort_unstable();
    unknown
}

/// Log (WARN) the applied migrations this binary doesn't know, so a rollback
/// running on a newer schema is visible in `ottod.log` rather than silent.
async fn warn_unknown_applied_versions(pool: &SqlitePool, migrator: &sqlx::migrate::Migrator) {
    let Ok(applied) =
        sqlx::query_scalar::<_, i64>("SELECT version FROM _sqlx_migrations WHERE success = 1")
            .fetch_all(pool)
            .await
    else {
        return; // fresh database (no table yet)
    };
    let embedded: Vec<i64> = migrator.iter().map(|m| m.version).collect();
    let unknown = unknown_applied_versions(&applied, &embedded);
    if !unknown.is_empty() {
        tracing::warn!(
            "database carries {} migration(s) newer than this build {:?} — running on a \
             newer (additive) schema, e.g. after a rollback; they are left in place",
            unknown.len(),
            unknown
        );
    }
}

/// Open the EXISTING Otto database at `path` for a short-lived helper process
/// (the per-session `ottod mcp-tools` bridge) — same connection settings as
/// [`open`], but no repair `UPDATE`s, no `sqlx::migrate!`, no file creation.
///
/// Why: every agent session spawns a bridge, and [`open`] at each spawn took
/// the write lock for two repair `UPDATE`s and re-validated ~150 migration
/// checksums against the live daemon's database — fighting the daemon's writer
/// at every session start — and a bridge binary NEWER than the running daemon
/// could even apply migrations behind its back. The daemon owns the schema;
/// a helper only ever attaches to it. Fails when the file or the
/// `_sqlx_migrations` table is missing (the daemon has never initialized it).
pub async fn open_existing(path: &Path) -> Result<DbPool> {
    if !path.exists() {
        return Err(Error::NotFound(format!("database {}", path.display())));
    }
    let opts = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))
        .map_err(|e| Error::Internal(format!("sqlite options: {e}")))?
        .create_if_missing(false)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5));
    let read_opts = opts.clone().read_only(true);
    // A helper writes rarely (audit rows): one writer connection, opened on
    // first use, and a couple of lazy readers.
    let write = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_lazy_with(opts);
    let read = SqlitePoolOptions::new()
        .max_connections(2)
        .connect_lazy_with(read_opts);
    let pool = DbPool::split(read, write);
    let initialized: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='_sqlx_migrations')",
    )
    .fetch_one(&pool)
    .await
    .map_err(|e| Error::Internal(format!("sqlite open: {e}")))?;
    if !initialized {
        return Err(Error::Internal(format!(
            "database {} has no schema yet (the daemon initializes it)",
            path.display()
        )));
    }
    Ok(pool)
}

/// One-time data repair for DBs bricked by the vault-docs migration **renumber**
/// (commit 2df6850): the vault-docs migrations were originally applied at
/// versions 103/104, then the files were *renamed* to 105/106 and 103/104 were
/// reused for the new `external_app` + `web_logins` migrations. sqlx keys the
/// applied set by version **number**, so on every pre-renumber install it now
/// compares disk-0103's checksum (`external_app`) against the recorded 103
/// (`vault docs`) and aborts the whole boot with
/// `migration 103 was previously applied but has been modified` — a permanent
/// crash-loop for anyone who ran Otto before the renumber.
///
/// The recorded 103/104 checksums are byte-identical to the renamed 0105/0106
/// files (the renumber was a pure rename), so we simply renumber the two
/// recorded rows to 105/106. sqlx then finds 105/106 already applied (checksums
/// match, skipped) and applies the genuinely-pending 103 (`external_app`) and
/// 104 (`web_logins`) it never ran — sqlx 0.8 applies pending migrations in
/// version order regardless of ordering versus the max applied version.
///
/// Guards make this safe and idempotent for *every* population:
/// - fresh DBs have no `_sqlx_migrations` table yet → no-op;
/// - post-renumber DBs record `external_app` at 103 (not `vault docs`) → no-op;
/// - the `NOT EXISTS` clause prevents a primary-key clash on re-run.
async fn repair_renumbered_vault_migrations(pool: &SqlitePool) -> Result<()> {
    // A brand-new DB has no migrations table yet — sqlx creates it. Nothing to
    // repair, and touching it here would error.
    let has_table: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='_sqlx_migrations')",
    )
    .fetch_one(pool)
    .await
    .map_err(|e| Error::Internal(format!("migrate repair probe: {e}")))?;
    if !has_table {
        return Ok(());
    }

    // Only the bricked population has vault-docs recorded at 103/104. The
    // description guard prevents misfiring on post-renumber DBs (where 103 is
    // `external_app`); NOT EXISTS prevents a PK clash if 105/106 already exist.
    let moved = sqlx::query(
        "UPDATE _sqlx_migrations SET version = 105 \
         WHERE version = 103 AND description = 'vault docs' \
           AND NOT EXISTS (SELECT 1 FROM _sqlx_migrations WHERE version = 105)",
    )
    .execute(pool)
    .await
    .map_err(|e| Error::Internal(format!("migrate repair 103->105: {e}")))?
    .rows_affected();

    sqlx::query(
        "UPDATE _sqlx_migrations SET version = 106 \
         WHERE version = 104 AND description = 'vault docs runs' \
           AND NOT EXISTS (SELECT 1 FROM _sqlx_migrations WHERE version = 106)",
    )
    .execute(pool)
    .await
    .map_err(|e| Error::Internal(format!("migrate repair 104->106: {e}")))?;

    if moved > 0 {
        tracing::warn!(
            "migrate: repaired renumbered vault-docs migrations (103/104 -> 105/106); \
             sqlx will now apply external_app + web_logins"
        );
    }
    Ok(())
}

/// Migrations that were RENUMBERED while feature branches raced for the same
/// version on 2026-09-05: `(old version, sqlx description, new version)`.
/// An install that applied a branch build under the old number has the
/// identical file content recorded (sqlx checksum = sha384 of the file, and
/// the files were only renamed), so re-versioning the row makes sqlx see the
/// new number as already applied and then apply whatever it genuinely lacks —
/// in either order the two populations upgraded.
const RENUMBERED: &[(i64, &str, i64)] = &[
    // feat/conversation-view shipped as 0115–0117; main took those numbers.
    (115, "sessions transcript path", 121),
    (116, "agent tasks source", 122),
    (117, "transcript index", 123),
    // feat/resource-access-governance shipped as 0116/0117.
    (116, "resource access", 119),
    (117, "database changes", 120),
    // feat/product-design-arena shipped as 0115.
    (115, "product epic tree", 124),
];

/// Generalised form of [`repair_renumbered_vault_migrations`]: for every
/// `(old, description, new)` move the applied row from `old` to `new` when it
/// carries that description and `new` is not applied yet. Idempotent; a
/// no-op on fresh DBs and on DBs that never ran the old numbering.
async fn repair_renumbered_migrations(pool: &SqlitePool, table: &[(i64, &str, i64)]) -> Result<()> {
    let has_table: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='_sqlx_migrations')",
    )
    .fetch_one(pool)
    .await
    .map_err(|e| Error::Internal(format!("migrate repair probe: {e}")))?;
    if !has_table {
        return Ok(());
    }
    for (old, description, new) in table {
        let moved = sqlx::query(
            "UPDATE _sqlx_migrations SET version = ? \
             WHERE version = ? AND description = ? \
               AND NOT EXISTS (SELECT 1 FROM _sqlx_migrations WHERE version = ?)",
        )
        .bind(new)
        .bind(old)
        .bind(description)
        .bind(new)
        .execute(pool)
        .await
        .map_err(|e| Error::Internal(format!("migrate repair {old}->{new}: {e}")))?
        .rows_affected();
        if moved > 0 {
            tracing::warn!(
                "migrate: repaired renumbered migration '{description}' ({old} -> {new}); \
                 sqlx will now apply the migrations this install lacks"
            );
        }
    }
    Ok(())
}

/// Pre-migration snapshots kept in `<data dir>/backups/` (newest first).
const KEEP_MIGRATION_SNAPSHOTS: usize = 3;

/// Embedded (up) migration versions the database has not applied yet.
/// `applied` holds the successfully-applied versions from `_sqlx_migrations`.
fn pending_versions(applied: &[i64], embedded: &[i64]) -> Vec<i64> {
    let applied: std::collections::HashSet<i64> = applied.iter().copied().collect();
    let mut pending: Vec<i64> = embedded
        .iter()
        .copied()
        .filter(|v| !applied.contains(v))
        .collect();
    pending.sort_unstable();
    pending
}

/// Snapshot file name: `<db file>.pre-<last applied version>-<UTC stamp>`.
fn snapshot_name(db_file: &str, last_applied: i64, now: chrono::DateTime<chrono::Utc>) -> String {
    format!(
        "{db_file}.pre-{last_applied}-{}",
        now.format("%Y%m%dT%H%M%SZ")
    )
}

/// Which of `names` (files in the backups dir) to delete so only the newest
/// `keep` snapshots of `db_file` survive. Ordered by the fixed-width UTC stamp
/// at the end of the name, not by the version, so a downgrade-then-upgrade
/// still keeps the most RECENT copies. Files that don't match the snapshot
/// pattern are never selected.
fn snapshots_to_prune(db_file: &str, names: &[String], keep: usize) -> Vec<String> {
    let prefix = format!("{db_file}.pre-");
    let mut snaps: Vec<(&str, &String)> = names
        .iter()
        .filter_map(|n| {
            let rest = n.strip_prefix(&prefix)?;
            let (version, stamp) = rest.rsplit_once('-')?;
            let valid = version.parse::<i64>().is_ok()
                && stamp.len() == 16
                && stamp.ends_with('Z')
                && stamp.as_bytes()[8] == b'T';
            valid.then_some((stamp, n))
        })
        .collect();
    // Newest first.
    snaps.sort_by(|a, b| b.0.cmp(a.0));
    snaps
        .into_iter()
        .skip(keep)
        .map(|(_, n)| n.clone())
        .collect()
}

/// Where a snapshot is written before it is complete.
fn partial_snapshot_path(target: &Path) -> std::path::PathBuf {
    let mut p = target.as_os_str().to_owned();
    p.push(".partial");
    std::path::PathBuf::from(p)
}

/// Remove `<db_file>.pre-*.partial` leftovers of an interrupted snapshot.
fn sweep_partial_snapshots(backups: &Path, db_file: &str) {
    let prefix = format!("{db_file}.pre-");
    let Ok(rd) = std::fs::read_dir(backups) else {
        return;
    };
    for name in rd
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.starts_with(&prefix) && n.ends_with(".partial"))
    {
        match std::fs::remove_file(backups.join(&name)) {
            Ok(()) => tracing::warn!("migrate snapshot: removed interrupted snapshot {name}"),
            Err(e) => tracing::warn!("migrate snapshot: remove {name}: {e}"),
        }
    }
}

/// Snapshots are a full copy of the user's data: owner-only (0600).
fn restrict_snapshot_mode(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
            tracing::warn!("migrate snapshot: chmod 0600 {}: {e}", path.display());
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// When migrations are pending on an EXISTING database, write a consistent
/// copy (`VACUUM INTO`) to `<dir>/backups/` and keep the newest
/// [`KEEP_MIGRATION_SNAPSHOTS`]. Best-effort by design: a full disk must not
/// brick the daemon, so a failed snapshot is logged loudly and boot goes on.
async fn snapshot_before_migrations(
    pool: &SqlitePool,
    path: &Path,
    migrator: &sqlx::migrate::Migrator,
) {
    let has_table: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='_sqlx_migrations')",
    )
    .fetch_one(pool)
    .await
    .unwrap_or(false);
    if !has_table {
        return; // fresh database — nothing to protect
    }
    let applied: Vec<i64> =
        match sqlx::query_scalar("SELECT version FROM _sqlx_migrations WHERE success = 1")
            .fetch_all(pool)
            .await
        {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("migrate snapshot: cannot read applied migrations: {e}");
                return;
            }
        };
    let embedded: Vec<i64> = migrator
        .iter()
        .filter(|m| !m.migration_type.is_down_migration())
        .map(|m| m.version)
        .collect();
    let pending = pending_versions(&applied, &embedded);
    let (Some(&last_applied), false) = (applied.iter().max(), pending.is_empty()) else {
        return;
    };
    let (Some(dir), Some(file)) = (path.parent(), path.file_name().and_then(|f| f.to_str())) else {
        return;
    };
    let backups = dir.join("backups");
    // A snapshot interrupted by a kill (SIGKILL, power loss) leaves only its
    // `.partial` file — never a name the retention below counts as a good copy.
    sweep_partial_snapshots(&backups, file);
    if let Err(e) = std::fs::create_dir_all(&backups) {
        tracing::error!(
            "migrate snapshot: create {}: {e} — migrating WITHOUT a snapshot",
            backups.display()
        );
        return;
    }
    let target = backups.join(snapshot_name(file, last_applied, chrono::Utc::now()));
    let partial = partial_snapshot_path(&target);
    let started = std::time::Instant::now();
    // `VACUUM INTO` reads one consistent snapshot through the writer and never
    // touches the live file; the target must not exist (the stamp is unique).
    // It writes `<name>.partial` and is renamed only once complete, so a
    // kill mid-copy can't leave a truncated file under a retained name.
    let res = sqlx::query("VACUUM INTO ?")
        .bind(partial.to_string_lossy().into_owned())
        .execute(pool)
        .await
        .map_err(|e| e.to_string())
        .and_then(|_| {
            restrict_snapshot_mode(&partial);
            std::fs::rename(&partial, &target).map_err(|e| format!("rename: {e}"))
        });
    match res {
        Ok(_) => tracing::info!(
            "migrate snapshot: {} pending migration(s) {:?}; saved {} in {} ms",
            pending.len(),
            pending,
            target.display(),
            started.elapsed().as_millis()
        ),
        Err(e) => {
            let _ = std::fs::remove_file(&partial);
            tracing::error!(
                "migrate snapshot: VACUUM INTO {} failed: {e} — migrating WITHOUT a snapshot",
                target.display()
            );
            return;
        }
    }
    let names: Vec<String> = std::fs::read_dir(&backups)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().into_string().ok())
                .collect()
        })
        .unwrap_or_default();
    for old in snapshots_to_prune(file, &names, KEEP_MIGRATION_SNAPSHOTS) {
        match std::fs::remove_file(backups.join(&old)) {
            Ok(()) => tracing::info!("migrate snapshot: pruned old snapshot {old}"),
            Err(e) => tracing::warn!("migrate snapshot: prune {old}: {e}"),
        }
    }
}

/// In-memory pool with all migrations applied — for tests only. A single
/// connection keeps the `sqlite::memory:` schema alive for the pool's lifetime.
pub async fn test_pool() -> DbPool {
    let opts = SqliteConnectOptions::from_str("sqlite::memory:")
        .expect("sqlite memory options")
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .expect("open in-memory sqlite");
    sqlx::migrate!().run(&pool).await.expect("run migrations");
    DbPool::from(pool)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build an in-memory pool whose single connection keeps the schema alive,
    /// with a `_sqlx_migrations` table shaped exactly like sqlx's own.
    async fn migrations_pool(rows: &[(i64, &str)]) -> SqlitePool {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:").unwrap();
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE _sqlx_migrations ( \
                version BIGINT PRIMARY KEY, \
                description TEXT NOT NULL, \
                installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP, \
                success BOOLEAN NOT NULL, \
                checksum BLOB NOT NULL, \
                execution_time BIGINT NOT NULL )",
        )
        .execute(&pool)
        .await
        .unwrap();
        for (v, d) in rows {
            sqlx::query(
                "INSERT INTO _sqlx_migrations \
                 (version, description, success, checksum, execution_time) \
                 VALUES (?, ?, 1, X'00', 0)",
            )
            .bind(v)
            .bind(d)
            .execute(&pool)
            .await
            .unwrap();
        }
        pool
    }

    #[test]
    fn pending_versions_lists_unapplied_embedded_in_order() {
        assert_eq!(pending_versions(&[1, 2, 3], &[1, 2, 3]), Vec::<i64>::new());
        assert_eq!(pending_versions(&[1, 2], &[3, 1, 2, 4]), vec![3, 4]);
        // A gap (a branch migration applied out of order) is still pending.
        assert_eq!(pending_versions(&[1, 3], &[1, 2, 3]), vec![2]);
        // Applied-but-unknown versions (a newer DB) are not "pending".
        assert_eq!(pending_versions(&[1, 2, 9], &[1, 2]), Vec::<i64>::new());
    }

    #[test]
    fn snapshot_retention_keeps_the_newest_by_stamp() {
        let t = |s: &str| {
            chrono::DateTime::parse_from_rfc3339(s)
                .unwrap()
                .with_timezone(&chrono::Utc)
        };
        let names: Vec<String> = vec![
            snapshot_name("otto.db", 150, t("2026-10-01T10:00:00Z")),
            snapshot_name("otto.db", 152, t("2026-10-03T10:00:00Z")),
            // An older stamp with a HIGHER version (downgrade then upgrade).
            snapshot_name("otto.db", 160, t("2026-09-01T10:00:00Z")),
            snapshot_name("otto.db", 151, t("2026-10-02T10:00:00Z")),
            // Never selected: other files, other dbs, malformed stamps.
            "otto.db".into(),
            "notes.txt".into(),
            "other.db.pre-1-20200101T000000Z".into(),
            "otto.db.pre-x-20200101T000000Z".into(),
            "otto.db.pre-1-2020".into(),
        ];
        assert_eq!(names[0], "otto.db.pre-150-20261001T100000Z");
        let pruned = snapshots_to_prune("otto.db", &names, 3);
        assert_eq!(pruned, vec![names[2].clone()]);
        assert!(snapshots_to_prune("otto.db", &names, 10).is_empty());
        assert_eq!(snapshots_to_prune("otto.db", &names, 0).len(), 4);
    }

    #[tokio::test]
    async fn open_snapshots_only_when_migrations_are_pending() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("otto.db");
        let backups = dir.path().join("backups");
        let count = || {
            std::fs::read_dir(&backups)
                .map(|rd| rd.count())
                .unwrap_or(0)
        };
        // Fresh DB: nothing to protect, no snapshot.
        drop(open(&path).await.unwrap());
        assert_eq!(count(), 0);
        // Up to date: a plain restart never pays the copy.
        drop(open(&path).await.unwrap());
        assert_eq!(count(), 0);
        // Forget the newest migration → it is pending again → one snapshot.
        // (Its DDL already ran, so re-running it may fail; the snapshot is
        // taken BEFORE the migrator, which is what this asserts.)
        {
            let pool = open(&path).await.unwrap();
            sqlx::query("DELETE FROM _sqlx_migrations WHERE version = (SELECT MAX(version) FROM _sqlx_migrations)")
                .execute(&pool)
                .await
                .unwrap();
        }
        let _ = open(&path).await;
        assert_eq!(count(), 1);
        let name = std::fs::read_dir(&backups)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .file_name()
            .into_string()
            .unwrap();
        assert!(name.starts_with("otto.db.pre-"), "{name}");
    }

    #[test]
    fn unknown_applied_versions_are_the_newer_builds() {
        assert_eq!(
            unknown_applied_versions(&[1, 2], &[1, 2, 3]),
            Vec::<i64>::new()
        );
        assert_eq!(unknown_applied_versions(&[9, 1, 2, 7], &[1, 2]), vec![7, 9]);
    }

    /// S10-01: a rollback boots the PREVIOUS binary against a database the
    /// failed (newer) build already migrated. sqlx's default `VersionMissing`
    /// check made it exit and crash-loop; it must open the newer schema.
    #[tokio::test]
    async fn open_tolerates_versions_applied_by_a_newer_build() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("otto.db");
        drop(open(&path).await.unwrap());
        {
            let pool = open(&path).await.unwrap();
            // What a newer build leaves behind: an additive table + its row.
            sqlx::query("CREATE TABLE future_feature (id INTEGER PRIMARY KEY)")
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query(
                "INSERT INTO _sqlx_migrations \
                 (version, description, success, checksum, execution_time) \
                 VALUES (99999999999999, 'future feature', 1, X'00', 0)",
            )
            .execute(&pool)
            .await
            .unwrap();
        }
        let pool = open(&path)
            .await
            .expect("an older binary must boot on a newer additive schema");
        // Left in place — never un-applied or deleted.
        assert!(versions(pool.writer()).await.contains(&99999999999999));
        // And a newer-only DB is not "pending": no snapshot was taken.
        assert!(!dir.path().join("backups").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn snapshots_are_owner_only_and_partials_are_swept() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("otto.db");
        let backups = dir.path().join("backups");
        drop(open(&path).await.unwrap());
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600,
            "otto.db is owner-only"
        );
        {
            let pool = open(&path).await.unwrap();
            sqlx::query("DELETE FROM _sqlx_migrations WHERE version = (SELECT MAX(version) FROM _sqlx_migrations)")
                .execute(&pool)
                .await
                .unwrap();
        }
        // A copy a SIGKILL interrupted in an earlier boot.
        std::fs::create_dir_all(&backups).unwrap();
        let stale = backups.join("otto.db.pre-5-20260101T000000Z.partial");
        std::fs::write(&stale, b"truncated").unwrap();
        let _ = open(&path).await;
        let names: Vec<String> = std::fs::read_dir(&backups)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names.len(), 1, "{names:?}");
        assert!(!names[0].ends_with(".partial"), "{names:?}");
        let mode = std::fs::metadata(backups.join(&names[0]))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    async fn versions(pool: &SqlitePool) -> Vec<i64> {
        sqlx::query_scalar("SELECT version FROM _sqlx_migrations ORDER BY version")
            .fetch_all(pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn open_existing_attaches_without_migrating_or_creating() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("otto.db");
        // Never creates the file.
        assert!(open_existing(&path).await.is_err());
        assert!(!path.exists());
        // An initialized database attaches; the helper sees the schema and can
        // write (audit rows) through its writer.
        let daemon = open(&path).await.unwrap();
        let applied_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations")
            .fetch_one(&daemon)
            .await
            .unwrap();
        let helper = open_existing(&path).await.unwrap();
        sqlx::query("INSERT INTO settings (key, value_json) VALUES ('probe', '1')")
            .execute(&helper)
            .await
            .unwrap();
        let applied_after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations")
            .fetch_one(&helper)
            .await
            .unwrap();
        assert_eq!(applied_before, applied_after);
        // An empty (never-initialized) file is refused, not migrated.
        let blank = dir.path().join("blank.db");
        std::fs::write(&blank, b"").unwrap();
        assert!(open_existing(&blank).await.is_err());
        let tables: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master")
            .fetch_one(
                &DbPool::connect(&format!("sqlite://{}", blank.display()))
                    .await
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(tables, 0);
    }

    #[tokio::test]
    async fn repair_renumbers_bricked_vault_rows_and_is_idempotent() {
        let pool = migrations_pool(&[
            (100, "review agent prompts"),
            (101, "api client durability secrets"),
            (102, "skill review instructions fix"),
            (103, "vault docs"),
            (104, "vault docs runs"),
        ])
        .await;

        repair_renumbered_vault_migrations(&pool).await.unwrap();
        assert_eq!(versions(&pool).await, vec![100, 101, 102, 105, 106]);

        // Running again must not move anything (103/104 are gone now).
        repair_renumbered_vault_migrations(&pool).await.unwrap();
        assert_eq!(versions(&pool).await, vec![100, 101, 102, 105, 106]);
    }

    #[tokio::test]
    async fn repair_moves_branch_numbered_rows_in_either_population() {
        // Population A ran the conversation-view build: transcript migrations
        // sit at 115/116/117 and main's 0115 (workflow runs) never applied.
        let pool = migrations_pool(&[
            (114, "k8s clusters"),
            (115, "sessions transcript path"),
            (116, "agent tasks source"),
            (117, "transcript index"),
        ])
        .await;
        repair_renumbered_migrations(&pool, RENUMBERED)
            .await
            .unwrap();
        assert_eq!(versions(&pool).await, vec![114, 121, 122, 123]);
        // sqlx will now apply 115 (workflow runs) .. 120 as genuinely pending.

        // Population B ran the governance build (116/117 = access + changes)
        // on top of main's 0115; the k8s-monitor 116/117 are still pending.
        let pool = migrations_pool(&[
            (115, "workflow runs created by"),
            (116, "resource access"),
            (117, "database changes"),
        ])
        .await;
        repair_renumbered_migrations(&pool, RENUMBERED)
            .await
            .unwrap();
        assert_eq!(versions(&pool).await, vec![115, 119, 120]);

        // Population D ran the product-design-arena build (0115 = epic tree).
        let pool = migrations_pool(&[(114, "x"), (115, "product epic tree")]).await;
        repair_renumbered_migrations(&pool, RENUMBERED)
            .await
            .unwrap();
        assert_eq!(versions(&pool).await, vec![114, 124]);

        // Population C is a correct main install: nothing moves.
        let pool = migrations_pool(&[
            (115, "workflow runs created by"),
            (116, "k8s monitor"),
            (117, "k8s monitor series cap"),
            (119, "resource access"),
        ])
        .await;
        repair_renumbered_migrations(&pool, RENUMBERED)
            .await
            .unwrap();
        assert_eq!(versions(&pool).await, vec![115, 116, 117, 119]);
        // Idempotent.
        repair_renumbered_migrations(&pool, RENUMBERED)
            .await
            .unwrap();
        assert_eq!(versions(&pool).await, vec![115, 116, 117, 119]);
    }

    #[tokio::test]
    async fn repair_is_noop_on_post_renumber_db() {
        // A correctly-numbered install: 103 is external_app, vault sits at 105/106.
        let pool = migrations_pool(&[
            (102, "skill review instructions fix"),
            (103, "external app kind"),
            (104, "web logins"),
            (105, "vault docs"),
            (106, "vault docs runs"),
        ])
        .await;

        repair_renumbered_vault_migrations(&pool).await.unwrap();
        assert_eq!(versions(&pool).await, vec![102, 103, 104, 105, 106]);
    }

    #[tokio::test]
    async fn repair_is_noop_when_migrations_table_absent() {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:").unwrap();
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        // No _sqlx_migrations table yet (fresh DB) — must not error.
        repair_renumbered_vault_migrations(&pool).await.unwrap();
    }
}
