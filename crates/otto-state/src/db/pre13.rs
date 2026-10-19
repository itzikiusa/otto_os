//! Preserve child edit history around the immutable 0013 parent-table rebuild.
//! The rescue table deliberately has no FK: DROP of the original parent must
//! not cascade into the rescue copy. It survives interrupted migration/restore.

use crate::convert::dberr;
use otto_core::{Error, Result};
use sqlx::{Executor, Sqlite, SqlitePool};

const STASH: &str = "_otto_pre13_improvement_edits";
// These are every column of 0009's child table, unchanged by later migrations.
const COLUMNS: &str = "id, run_id, workspace_id, target, target_ref, target_path, kind, risk, status, rationale, evidence_json, before_content, after_content, applied_at, actor, created_at";

async fn table_exists<'e>(conn: impl Executor<'e, Database = Sqlite>, name: &str) -> Result<bool> {
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?)",
    )
    .bind(name.to_owned())
    .fetch_one(conn)
    .await
    .map_err(dberr("pre13 preservation: table lookup"))
}

/// Commit the rescue copy BEFORE running any migration. If this fails, boot
/// must stop: the ordinary pre-migration file snapshot is only best effort.
pub(super) async fn prepare(pool: &SqlitePool) -> Result<()> {
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(dberr("pre13 preservation: begin"))?;
    if table_exists(&mut *tx, STASH).await? {
        // A previous boot may have committed 0013 and then failed elsewhere.
        // Never overwrite its rescue copy with the now-empty child table.
        tracing::warn!("pre13 preservation: retained unfinished improvement edit rescue table; restoration will retry after migrations");
        return Ok(());
    }
    if !table_exists(&mut *tx, "_sqlx_migrations").await? {
        return Ok(()); // fresh database, nothing to preserve
    }
    let applied: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM _sqlx_migrations WHERE version = 13 AND success = 1)",
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(dberr("pre13 preservation: migration state"))?;
    if applied || !table_exists(&mut *tx, "improvement_edits").await? {
        return Ok(());
    }
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM improvement_edits")
        .fetch_one(&mut *tx)
        .await
        .map_err(dberr("pre13 preservation: count edits"))?;
    if count == 0 {
        return Ok(());
    }
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "CREATE TABLE {STASH} AS SELECT {COLUMNS} FROM improvement_edits"
    )))
    .execute(&mut *tx)
    .await
    .map_err(dberr("pre13 preservation: copy edits"))?;
    tx.commit()
        .await
        .map_err(dberr("pre13 preservation: commit rescue copy"))?;
    tracing::warn!(count, "pre13 preservation: saved improvement edits before immutable migration 0013; rescue copy retained until verified restoration");
    Ok(())
}

/// Restore and verify the rescue copy in ONE transaction, including its drop.
/// Any SQL error, absent parent, or differing existing ID leaves it intact.
pub(super) async fn restore(pool: &SqlitePool) -> Result<()> {
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(dberr("pre13 restore: begin"))?;
    if !table_exists(&mut *tx, STASH).await? {
        return Ok(());
    }
    let applied: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM _sqlx_migrations WHERE version = 13 AND success = 1)",
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(dberr("pre13 restore: migration state"))?;
    if !applied {
        return Err(Error::Internal(
            "pre13 restore: migration 0013 is not applied; rescue table retained".into(),
        ));
    }
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _otto_pre13_improvement_edits")
        .fetch_one(&mut *tx)
        .await
        .map_err(dberr("pre13 restore: count rescue rows"))?;
    // Check even IDs already present in the live table: a matching legacy row
    // written with foreign_keys off must not hide a missing parent from INSERT.
    let missing_parent: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM _otto_pre13_improvement_edits s \
         LEFT JOIN improvement_runs r ON r.id = s.run_id \
         LEFT JOIN workspaces w ON w.id = s.workspace_id \
         WHERE r.id IS NULL OR w.id IS NULL)",
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(dberr("pre13 restore: verify parents"))?;
    if missing_parent {
        return Err(Error::Internal(
            "pre13 restore: preserved edits have a missing run or workspace; rescue table retained"
                .into(),
        ));
    }
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "INSERT INTO improvement_edits ({COLUMNS}) \
         SELECT {COLUMNS} FROM {STASH} s \
         WHERE NOT EXISTS (SELECT 1 FROM improvement_edits e WHERE e.id = s.id)"
    )))
    .execute(&mut *tx)
    .await
    .map_err(dberr("pre13 restore: insert edits; rescue table retained"))?;
    let mismatches: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT COUNT(*) FROM (SELECT {COLUMNS} FROM {STASH} \
         EXCEPT SELECT {COLUMNS} FROM improvement_edits)"
    )))
    .fetch_one(&mut *tx)
    .await
    .map_err(dberr("pre13 restore: verify edits; rescue table retained"))?;
    if mismatches != 0 {
        return Err(Error::Internal(format!("pre13 restore: {mismatches} preserved edit rows conflict with existing data; no rows overwritten, rescue table retained")));
    }
    sqlx::query("DROP TABLE _otto_pre13_improvement_edits")
        .execute(&mut *tx)
        .await
        .map_err(dberr("pre13 restore: remove verified rescue copy"))?;
    tx.commit()
        .await
        .map_err(dberr("pre13 restore: commit restored edits"))?;
    tracing::warn!(
        count,
        "pre13 restore: verified complete improvement edit history and removed rescue table"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
    use std::path::Path;

    async fn raw(path: &Path) -> SqlitePool {
        SqlitePoolOptions::new()
            .max_connections(2)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(path)
                    .create_if_missing(true)
                    .foreign_keys(true)
                    .journal_mode(SqliteJournalMode::Wal),
            )
            .await
            .unwrap()
    }

    async fn migrate_through(pool: &SqlitePool, version: i64) {
        let embedded = sqlx::migrate!();
        let migrations = embedded
            .iter()
            .filter(|m| m.version <= version)
            .cloned()
            .collect();
        sqlx::migrate::Migrator::with_migrations(migrations)
            .run(pool)
            .await
            .unwrap();
    }

    async fn seeded() -> (tempfile::TempDir, SqlitePool) {
        let dir = tempfile::tempdir().unwrap();
        let pool = raw(&dir.path().join("state.db")).await;
        migrate_through(&pool, 12).await;
        sqlx::query("INSERT INTO users (id, username, password_hash, display_name, is_root, created_at) VALUES ('u', 'u', 'hash', 'U', 1, '2026-01-01T00:00:00Z')")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO workspaces (id, name, root_path, created_at) VALUES ('w', 'W', '/tmp', '2026-01-01T00:00:00Z')")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO improvement_runs (id, workspace_id, trigger, status, started_at) VALUES ('r', 'w', 'manual', 'done', '2026-01-01T00:00:00Z')")
            .execute(&pool).await.unwrap();
        for id in ["e1", "e2"] {
            sqlx::query("INSERT INTO improvement_edits (id, run_id, workspace_id, target, target_ref, target_path, kind, risk, status, rationale, evidence_json, before_content, after_content, applied_at, actor, created_at) VALUES (?, 'r', 'w', 'skill', 'example', '/tmp/example', 'modify', 'low', 'applied', 'Reason', '[\"session\"]', 'Before café', 'After\ntext', NULL, 'u', '2026-01-01T00:00:00Z')")
                .bind(id).execute(&pool).await.unwrap();
        }
        (dir, pool)
    }

    async fn edits(pool: &SqlitePool) -> Vec<String> {
        sqlx::query_scalar("SELECT json_array(id, run_id, workspace_id, target, target_ref, target_path, kind, risk, status, rationale, evidence_json, before_content, after_content, applied_at, actor, created_at) FROM improvement_edits ORDER BY id")
            .fetch_all(pool).await.unwrap()
    }

    async fn stash_count(pool: &SqlitePool) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM _otto_pre13_improvement_edits")
            .fetch_one(pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn populated_upgrade_through_production_open_preserves_every_edit_column() {
        let (dir, pool) = seeded().await;
        let expected = edits(&pool).await;
        pool.close().await;
        let pool = crate::db::open(&dir.path().join("state.db")).await.unwrap();
        assert_eq!(edits(pool.writer()).await, expected);
        assert!(!table_exists(pool.writer(), STASH).await.unwrap());
        let violations = sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert!(violations.is_empty());
        pool.close().await;
        // A subsequent current-schema boot is an idempotent no-op.
        let pool = crate::db::open(&dir.path().join("state.db")).await.unwrap();
        assert_eq!(edits(pool.writer()).await, expected);
        assert!(!table_exists(pool.writer(), STASH).await.unwrap());
        pool.close().await;
    }

    #[tokio::test]
    async fn interrupted_after_13_reuses_durable_stash_instead_of_empty_live_rows() {
        let (dir, pool) = seeded().await;
        let expected = edits(&pool).await;
        prepare(&pool).await.unwrap();
        prepare(&pool).await.unwrap();
        assert_eq!(stash_count(&pool).await, 2);
        // Restore before the destructive migration must not consume the stash.
        assert!(restore(&pool).await.is_err());
        assert_eq!(stash_count(&pool).await, 2);
        pool.close().await;
        let pool = raw(&dir.path().join("state.db")).await;
        migrate_through(&pool, 13).await;
        assert!(edits(&pool).await.is_empty(), "unaltered 0013 cascades");
        prepare(&pool).await.unwrap();
        assert_eq!(stash_count(&pool).await, 2);
        pool.close().await;
        let pool = crate::db::open(&dir.path().join("state.db")).await.unwrap();
        assert_eq!(edits(pool.writer()).await, expected);
        assert!(!table_exists(pool.writer(), STASH).await.unwrap());
        pool.close().await;
    }

    #[tokio::test]
    async fn failed_restore_and_conflicting_rows_keep_rescue_data_and_are_retryable() {
        let (_dir, pool) = seeded().await;
        let expected = edits(&pool).await;
        prepare(&pool).await.unwrap();
        migrate_through(&pool, 13).await;
        sqlx::query("CREATE TRIGGER reject_second_edit BEFORE INSERT ON improvement_edits WHEN NEW.id = 'e2' BEGIN SELECT RAISE(ABORT, 'injected restore failure'); END")
            .execute(&pool).await.unwrap();
        assert!(restore(&pool).await.is_err());
        assert!(
            edits(&pool).await.is_empty(),
            "first insert also rolls back"
        );
        assert_eq!(stash_count(&pool).await, 2);
        sqlx::query("DROP TRIGGER reject_second_edit")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO improvement_edits SELECT * FROM _otto_pre13_improvement_edits WHERE id = 'e1'")
            .execute(&pool).await.unwrap();
        sqlx::query(
            "UPDATE improvement_edits SET after_content = 'conflicting user data' WHERE id = 'e1'",
        )
        .execute(&pool)
        .await
        .unwrap();
        let conflict = edits(&pool).await;
        assert!(restore(&pool).await.is_err());
        assert_eq!(
            edits(&pool).await,
            conflict,
            "never overwrite a conflicting row"
        );
        assert_eq!(stash_count(&pool).await, 2);
        sqlx::query("DELETE FROM improvement_edits WHERE id = 'e1'")
            .execute(&pool)
            .await
            .unwrap();
        restore(&pool).await.unwrap();
        restore(&pool).await.unwrap();
        assert_eq!(edits(&pool).await, expected);
        assert!(!table_exists(&pool, STASH).await.unwrap());
        pool.close().await;
    }

    #[tokio::test]
    async fn missing_parent_refuses_restore_without_consuming_stash() {
        let (_dir, pool) = seeded().await;
        prepare(&pool).await.unwrap();
        migrate_through(&pool, 13).await;
        sqlx::query("DELETE FROM improvement_runs WHERE id = 'r'")
            .execute(&pool)
            .await
            .unwrap();
        assert!(restore(&pool).await.is_err());
        assert_eq!(stash_count(&pool).await, 2);
        assert!(edits(&pool).await.is_empty());
        pool.close().await;
    }

    #[tokio::test]
    async fn preservation_failure_stops_production_boot_before_migration_13() {
        let (dir, pool) = seeded().await;
        let expected = edits(&pool).await;
        // Inject a SQLite DDL failure without changing production code: a view
        // occupying the reserved name prevents creation of the rescue table.
        sqlx::query("CREATE VIEW _otto_pre13_improvement_edits AS SELECT * FROM improvement_edits")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
        assert!(crate::db::open(&dir.path().join("state.db")).await.is_err());
        let pool = raw(&dir.path().join("state.db")).await;
        assert_eq!(edits(&pool).await, expected);
        let applied: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations WHERE version = 13")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            applied, 0,
            "failed preservation must prevent the destructive migration"
        );
        sqlx::query("DROP VIEW _otto_pre13_improvement_edits")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
        let pool = crate::db::open(&dir.path().join("state.db")).await.unwrap();
        assert_eq!(edits(pool.writer()).await, expected);
        pool.close().await;
    }

    #[tokio::test]
    async fn fresh_and_already_upgraded_databases_do_not_create_a_stash() {
        let dir = tempfile::tempdir().unwrap();
        let pool = raw(&dir.path().join("state.db")).await;
        prepare(&pool).await.unwrap();
        restore(&pool).await.unwrap();
        assert!(!table_exists(&pool, STASH).await.unwrap());
        migrate_through(&pool, 13).await;
        prepare(&pool).await.unwrap();
        restore(&pool).await.unwrap();
        assert!(!table_exists(&pool, STASH).await.unwrap());
        pool.close().await;
    }
}
