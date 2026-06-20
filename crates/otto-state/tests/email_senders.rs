//! Regression test for migration `0044_email_senders.sql`: the per-user email
//! sender table the email-OTP gate needs must exist after a fresh migrate, with
//! the `secret_ref` (Keychain reference, NOT the password) and `verified_at`
//! columns. Mirrors the `pragma_table_info` shape used in `share_tokens.rs`.

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;

/// An in-memory pool with every migration applied (matches the harness used in
/// `share_tokens.rs` / `otto-rbac` / `otto-server`).
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

/// After migrating, `email_senders` carries the columns the sender record needs:
/// the primary-key `user_id`, the `gmail_address`, the opaque `secret_ref`
/// (never the password), and a nullable `verified_at`.
#[tokio::test]
async fn email_senders_has_expected_columns() {
    let pool = mem_pool().await;
    let cols: Vec<String> =
        sqlx::query_scalar("SELECT name FROM pragma_table_info('email_senders')")
            .fetch_all(&pool)
            .await
            .expect("pragma_table_info(email_senders)");

    for c in ["user_id", "gmail_address", "secret_ref", "verified_at"] {
        assert!(
            cols.iter().any(|x| x == c),
            "missing column {c} on email_senders (cols: {cols:?})"
        );
    }
}

/// `user_id` is the primary key (one sender per user).
#[tokio::test]
async fn email_senders_user_id_is_primary_key() {
    let pool = mem_pool().await;
    let pk: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM pragma_table_info('email_senders') WHERE pk > 0",
    )
    .fetch_all(&pool)
    .await
    .expect("pragma_table_info pk");
    assert_eq!(pk, vec!["user_id".to_string()], "user_id must be the sole PK");
}
