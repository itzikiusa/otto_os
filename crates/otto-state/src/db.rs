//! Pool bootstrap: WAL mode, foreign keys, busy timeout, embedded migrations.

use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous,
};
use sqlx::SqlitePool;

use otto_core::{Error, Result};

/// Open (creating if needed) the Otto database at `path` and run migrations.
pub async fn open(path: &Path) -> Result<SqlitePool> {
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
        .busy_timeout(Duration::from_secs(5));

    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(opts)
        .await
        .map_err(|e| Error::Internal(format!("sqlite connect: {e}")))?;

    // Must run BEFORE sqlx::migrate!() — repairs DBs bricked by the vault-docs
    // migration renumber before sqlx validates recorded checksums by version.
    repair_renumbered_vault_migrations(&pool).await?;
    repair_renumbered_migrations(&pool, RENUMBERED).await?;

    sqlx::migrate!()
        .run(&pool)
        .await
        .map_err(|e| Error::Internal(format!("migrate: {e}")))?;

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

/// In-memory pool with all migrations applied — for tests only. A single
/// connection keeps the `sqlite::memory:` schema alive for the pool's lifetime.
pub async fn test_pool() -> SqlitePool {
    let opts = SqliteConnectOptions::from_str("sqlite::memory:")
        .expect("sqlite memory options")
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .expect("open in-memory sqlite");
    sqlx::migrate!()
        .run(&pool)
        .await
        .expect("run migrations");
    pool
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

    async fn versions(pool: &SqlitePool) -> Vec<i64> {
        sqlx::query_scalar("SELECT version FROM _sqlx_migrations ORDER BY version")
            .fetch_all(pool)
            .await
            .unwrap()
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
        repair_renumbered_migrations(&pool, RENUMBERED).await.unwrap();
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
        repair_renumbered_migrations(&pool, RENUMBERED).await.unwrap();
        assert_eq!(versions(&pool).await, vec![115, 119, 120]);

        // Population D ran the product-design-arena build (0115 = epic tree).
        let pool = migrations_pool(&[(114, "x"), (115, "product epic tree")]).await;
        repair_renumbered_migrations(&pool, RENUMBERED).await.unwrap();
        assert_eq!(versions(&pool).await, vec![114, 124]);

        // Population C is a correct main install: nothing moves.
        let pool = migrations_pool(&[
            (115, "workflow runs created by"),
            (116, "k8s monitor"),
            (117, "k8s monitor series cap"),
            (119, "resource access"),
        ])
        .await;
        repair_renumbered_migrations(&pool, RENUMBERED).await.unwrap();
        assert_eq!(versions(&pool).await, vec![115, 116, 117, 119]);
        // Idempotent.
        repair_renumbered_migrations(&pool, RENUMBERED).await.unwrap();
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
