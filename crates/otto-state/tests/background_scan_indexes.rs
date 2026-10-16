//! Query-plan guards for migration 0186: the timer-driven / page-load queries
//! it indexes must use those indexes instead of scanning their tables.

use sqlx::{Row, SqlitePool};

async fn pool() -> SqlitePool {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    pool
}

/// `EXPLAIN QUERY PLAN` details for `sql` (literal values, no binds).
async fn plan(pool: &SqlitePool, sql: &str) -> Vec<String> {
    sqlx::query(sqlx::AssertSqlSafe(format!("EXPLAIN QUERY PLAN {sql}")))
        .fetch_all(pool)
        .await
        .unwrap()
        .iter()
        .map(|row| row.get::<String, _>("detail"))
        .collect()
}

fn uses(details: &[String], index: &str) {
    assert!(
        details.iter().any(|d| d.contains(index)),
        "expected {index}: {details:?}"
    );
    // `SCAN t USING INDEX idx` over a PARTIAL index walks only its rows.
    assert!(
        !details
            .iter()
            .any(|d| d.starts_with("SCAN ") && !d.contains(" USING ")),
        "full table scan: {details:?}"
    );
}

#[tokio::test]
async fn timer_and_list_queries_use_their_indexes() {
    let pool = pool().await;
    // workflows.rs list_active_run_ids_global — the 60 s orphan sweep.
    let d = plan(
        &pool,
        "SELECT id FROM workflow_runs WHERE status IN ('pending','running') \
         ORDER BY started_at DESC LIMIT 200",
    )
    .await;
    uses(&d, "idx_workflow_runs_active");
    assert!(!d.iter().any(|s| s.contains("TEMP B-TREE")), "{d:?}");

    // assistant.rs expired_incognito — the 30 s assistant tick.
    let d = plan(
        &pool,
        "SELECT * FROM assistant_threads WHERE incognito = 1 \
         AND COALESCE(last_turn_at, created_at) < '2026-01-01'",
    )
    .await;
    uses(&d, "idx_assistant_threads_incognito");

    // canvas.rs list_for_user — the Canvas page.
    let d = plan(
        &pool,
        "SELECT id, title FROM canvas_scenes WHERE created_by = 'u' ORDER BY updated_at DESC",
    )
    .await;
    uses(&d, "idx_canvas_scenes_owner_updated");
    assert!(!d.iter().any(|s| s.contains("TEMP B-TREE")), "{d:?}");

    // swarm.rs pending_task_results — a task's latest run.
    let d = plan(
        &pool,
        "SELECT latest.id FROM swarm_runs latest WHERE latest.task_id = 't' \
         ORDER BY latest.enqueued_at DESC, latest.rowid DESC LIMIT 1",
    )
    .await;
    uses(&d, "idx_swarm_runs_task_enqueued");
    assert!(!d.iter().any(|s| s.contains("TEMP B-TREE")), "{d:?}");
}
