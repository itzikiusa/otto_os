/// A run canceled while parked behind a full run gate stops waiting at
/// once (its driver guard drops, so a retry isn't refused for hours).
#[tokio::test]
async fn a_queued_run_canceled_in_the_queue_stops_waiting() {
    use crate::routes::browser::tests::{mem_pool, seed_workspace};
    let pool = mem_pool().await;
    seed_workspace(&pool, "ws").await;
    sqlx::query("INSERT INTO users(id,username,password_hash,display_name,is_root,created_at) VALUES('u','u','x','U',1,?)")
        .bind(chrono::Utc::now().to_rfc3339()).execute(&pool).await.unwrap();
    let repo = otto_state::WorkflowsRepo::new(pool.clone());
    let wf = repo
        .create(
            &"ws".into(),
            "WF",
            "",
            "",
            &otto_core::workflows::WorkflowGraph::default(),
            &"u".into(),
        )
        .await
        .unwrap();
    let run = repo
        .create_run(&wf.id, &"ws".into(), &serde_json::json!({}), None)
        .await
        .unwrap();
    let full = tokio::sync::Semaphore::new(0);
    let park = super::park_until_permit_or_cancel(&full, &pool, &run.id);
    let cancel = async {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        repo.request_cancel(&run.id).await.unwrap();
    };
    let (parked, ()) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        tokio::join!(park, cancel)
    })
    .await
    .expect("stopped waiting");
    assert!(parked.is_none());
    // A free slot is still taken normally.
    let open = tokio::sync::Semaphore::new(1);
    let run2 = repo
        .create_run(&wf.id, &"ws".into(), &serde_json::json!({}), None)
        .await
        .unwrap();
    assert!(super::park_until_permit_or_cancel(&open, &pool, &run2.id)
        .await
        .is_some());
}
