//! Regression test for migration `0044_share_tokens.sql`: the scope columns and
//! kill-switch that scoped "share link" tokens need must exist on `auth_sessions`
//! after a fresh migrate. Mirrors the `pragma_table_info` shape used elsewhere in
//! the workspace to assert a migration's effect on the schema.

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;

/// An in-memory pool with every migration applied (matches the test harness in
/// `otto-rbac` / `otto-server`).
async fn mem_pool() -> SqlitePool {
    let opts = SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .expect("in-memory sqlite");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("migrations");
    pool
}

/// After migrating, `auth_sessions` carries the share-link scope columns added in
/// `0043`: the single-session scope, the capped role, and the explicit kill switch.
#[tokio::test]
async fn auth_sessions_has_share_scope_columns() {
    let pool = mem_pool().await;
    let cols: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info('auth_sessions')")
        .fetch_all(&pool)
        .await
        .expect("pragma_table_info(auth_sessions)");

    for c in ["session_scope", "scope_role", "revoked"] {
        assert!(
            cols.iter().any(|x| x == c),
            "missing column {c} on auth_sessions (cols: {cols:?})"
        );
    }
}

/// The scope lookup index exists so the guard can resolve a scoped token's one
/// session without a table scan.
#[tokio::test]
async fn auth_sessions_has_scope_index() {
    let pool = mem_pool().await;
    let idx: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type='index' AND tbl_name='auth_sessions'",
    )
    .fetch_all(&pool)
    .await
    .expect("index list");
    assert!(
        idx.iter().any(|x| x == "idx_auth_sessions_scope"),
        "missing idx_auth_sessions_scope (indexes: {idx:?})"
    );
}
