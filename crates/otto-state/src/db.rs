//! Pool bootstrap: WAL mode, foreign keys, busy timeout, embedded migrations.

use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
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
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5));

    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(opts)
        .await
        .map_err(|e| Error::Internal(format!("sqlite connect: {e}")))?;

    sqlx::migrate!()
        .run(&pool)
        .await
        .map_err(|e| Error::Internal(format!("migrate: {e}")))?;

    Ok(pool)
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
