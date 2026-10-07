use super::*;
use otto_state::DbPool;

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
