//! Test helpers — an in-memory pool with a seeded workspace + user. Public (not
//! `#[cfg(test)]`) so integration tests under `tests/` can use it.

use otto_core::new_id;
use sqlx::SqlitePool;

const TS: &str = "2026-06-19T00:00:00+00:00";

/// In-memory SQLite with all migrations applied, plus one workspace + user.
/// Returns (pool, workspace_id, user_id).
pub async fn mem_pool() -> (SqlitePool, String, String) {
    let pool = otto_state::db::test_pool().await;
    let user = new_id();
    let ws = new_id();
    sqlx::query(
        "INSERT INTO users (id, username, password_hash, display_name, is_root, disabled, created_at) \
         VALUES (?, ?, ?, ?, 0, 0, ?)",
    )
    .bind(&user)
    .bind(format!("u_{user}"))
    .bind("x")
    .bind("Tester")
    .bind(TS)
    .execute(&pool)
    .await
    .expect("seed user");
    sqlx::query(
        "INSERT INTO workspaces (id, name, root_path, created_at) VALUES (?, ?, ?, ?)",
    )
    .bind(&ws)
    .bind("Test WS")
    .bind("/tmp/test-ws")
    .bind(TS)
    .execute(&pool)
    .await
    .expect("seed workspace");
    (pool, ws, user)
}
