use super::*;
use otto_state::DbPool;

/// Exercise CI's first workflow graph at Tokio's 2 MiB default. Debug stack
/// layouts vary by ABI: 1 MiB is the measured macOS ARM negative-control budget,
/// not a stricter Linux requirement. A subprocess contains a stack-overflow abort.
#[test]
fn real_workflow_fits_a_small_worker_stack() {
    let stack_bytes = if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        1024 * 1024
    } else {
        2 * 1024 * 1024
    };
    check_real_workflow_stack(
        "real_workflow_fits_a_small_worker_stack",
        stack_bytes,
        false,
    );
}

#[test]
fn real_loop_fits_the_default_worker_stack() {
    check_real_workflow_stack(
        "real_loop_fits_the_default_worker_stack",
        2 * 1024 * 1024,
        true,
    );
}

fn check_real_workflow_stack(test_name: &str, stack_bytes: usize, loop_node: bool) {
    const CHILD: &str = "OTTO_WORKFLOW_STACK_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        // This synchronous parent has no runtime; the child owns the Tokio probe.
        #[allow(clippy::disallowed_methods)]
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg(format!("workflow_node_driver::tests::{test_name}"))
            .arg("--nocapture")
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .thread_stack_size(stack_bytes)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        use crate::routes::browser::tests::{seed_workspace, test_ctx};
        use crate::workflow_engine::run_workflow;
        use otto_core::workflows::{RunScope, RunStatus, WorkflowGraph};
        use otto_state::{WorkflowsRepo, WorkspacesRepo};
        use serde_json::json;

        let (dir, pool) = fixture().await;
        seed_workspace(&pool, "stack-ws").await;
        sqlx::query("INSERT INTO users(id,username,password_hash,display_name,is_root,created_at) VALUES('stack-u','stack-u','x','U',0,?)")
            .bind(chrono::Utc::now().to_rfc3339()).execute(&pool).await.unwrap();
        let ctx = test_ctx(&pool, dir.path().to_path_buf()).await;
        let ws = WorkspacesRepo::new(pool.clone()).get(&"stack-ws".into()).await.unwrap();
        let repo = WorkflowsRepo::new(pool);
        let step = if loop_node {
            json!({"id":"set","kind":"loop","name":"set","x":0,"y":0,"params":{"max_iterations":2,"steps":[{"kind":"transform","name":"tick","params":{"json":{"note":"ctx"}}}]}})
        } else {
            json!({"id":"set","kind":"transform","name":"set","x":0,"y":0,"params":{"json":{"note":"ctx"}}})
        };
        let graph: WorkflowGraph = serde_json::from_value(json!({"nodes": [
            {"id":"trigger","kind":"manual_trigger","name":"trigger","x":0,"y":0},
            step,
            {"id":"tail","kind":"log","name":"tail","x":0,"y":0}
        ], "edges": [
            {"id":"a","source":"trigger","target":"set"},
            {"id":"b","source":"set","target":"tail"}
        ]})).unwrap();
        let wf = repo.create(&ws.id, "stack probe", "", "", &graph, &"stack-u".into()).await.unwrap();
        let run = repo.create_run(&wf.id, &ws.id, &json!({}), None).await.unwrap();
        let future = run_workflow(ctx, ws, wf, run.id.clone(), json!({}), RunScope::default(), None);
        eprintln!("real workflow future: {} bytes", std::mem::size_of_val(&future));
        tokio::spawn(future).await.unwrap();
        let finished = repo.get_run(&run.id).await.unwrap();
        assert_eq!(finished.status, RunStatus::Success, "{:?}", finished.error);
        assert_eq!(finished.nodes.len(), 3);
        let output = finished.nodes[1].output.as_ref().unwrap();
        if loop_node {
            assert_eq!(output["iterations"], 2);
        } else {
            assert_eq!(output["note"], "ctx");
        }
    });
}

async fn fixture() -> (tempfile::TempDir, DbPool) {
    let dir = tempfile::tempdir().unwrap();
    let pool = otto_state::open(&dir.path().join("driver.sqlite"))
        .await
        .unwrap();
    sqlx::query("CREATE TABLE driver_probe (value TEXT NOT NULL)")
        .execute(&pool)
        .await
        .unwrap();
    (dir, pool)
}

#[tokio::test]
async fn progress_write_does_not_park_the_node_holding_its_sqlite_lock() {
    let (_dir, pool) = fixture().await;
    let (log, mut logs) = tokio::sync::mpsc::unbounded_channel();
    let node = async {
        let mut tx = pool.begin().await.unwrap();
        sqlx::query("INSERT INTO driver_probe VALUES ('checkpoint')")
            .execute(&mut *tx)
            .await
            .unwrap();
        log.send(()).unwrap();
        // Make the monitor receive the log while this transaction is open.
        tokio::task::yield_now().await;
        tx.commit().await.unwrap();
        Ok::<_, sqlx::Error>(())
    };
    let result = drive(node, async |mut done| {
        loop {
            tokio::select! {
                biased;
                Some(()) = logs.recv() => {
                    // The old inline select parked the transaction owner
                    // here until SQLite's five-second busy timeout expired.
                    sqlx::query("INSERT INTO driver_probe VALUES ('progress')")
                        .execute(&pool).await?;
                }
                result = &mut done => return result.unwrap(),
            }
        }
    })
    .await;
    result.unwrap();
    let values: Vec<String> = sqlx::query_scalar("SELECT value FROM driver_probe ORDER BY rowid")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(values, ["checkpoint", "progress"]);
}

#[tokio::test]
async fn node_error_waits_for_in_flight_progress_before_returning() {
    let (started, ready) = oneshot::channel();
    let (finished, flushed) = oneshot::channel();
    let result = drive(
        async {
            ready.await.unwrap();
            Err::<(), _>("node failed")
        },
        async |done| {
            started.send(()).unwrap();
            let result = done.await.unwrap();
            tokio::task::yield_now().await;
            finished.send(()).unwrap();
            result
        },
    )
    .await;
    assert_eq!(result, Err("node failed"));
    flushed.await.unwrap();
}

#[tokio::test]
async fn monitor_cancellation_drops_node_and_rolls_back_its_transaction() {
    let (_dir, pool) = fixture().await;
    let (locked, ready) = oneshot::channel();
    let result = drive(
        async {
            let mut tx = pool.begin().await.unwrap();
            sqlx::query("INSERT INTO driver_probe VALUES ('unfinished')")
                .execute(&mut *tx)
                .await
                .unwrap();
            locked.send(()).unwrap();
            std::future::pending::<()>().await;
            tx.commit().await.unwrap();
            "finished"
        },
        async |_| {
            ready.await.unwrap();
            "canceled"
        },
    )
    .await;
    assert_eq!(result, "canceled");
    sqlx::query("INSERT INTO driver_probe VALUES ('after cancel')")
        .execute(&pool)
        .await
        .unwrap();
    let values: Vec<String> = sqlx::query_scalar("SELECT value FROM driver_probe")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(values, ["after cancel"]);
}

#[tokio::test]
async fn dropping_driver_rolls_back_node_without_detached_work() {
    let (_dir, pool) = fixture().await;
    let (locked, ready) = oneshot::channel();
    let mut driver = Box::pin(drive(
        async {
            let mut tx = pool.begin().await.unwrap();
            sqlx::query("INSERT INTO driver_probe VALUES ('unfinished')")
                .execute(&mut *tx)
                .await
                .unwrap();
            locked.send(()).unwrap();
            std::future::pending::<()>().await;
            tx.commit().await.unwrap();
        },
        |_| std::future::pending::<()>(),
    ));
    tokio::select! {
        () = &mut driver => panic!("driver completed before caller canceled"),
        result = ready => result.unwrap(),
    }
    drop(driver);
    let mut tx = pool.begin().await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM driver_probe")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(count, 0, "the dropped node's write was rolled back");
    tx.commit().await.unwrap();
}
