use super::*;
async fn fixture() -> (tempfile::TempDir, DatabaseChangesRepo) {
    let dir = tempfile::tempdir().unwrap();
    let pool = crate::open(&dir.path().join("test.db")).await.unwrap();
    (dir, DatabaseChangesRepo::new(pool))
}
fn input() -> ChangeInput {
    ChangeInput {
        title: "Add column".into(),
        description: String::new(),
        script: "ALTER TABLE t ADD x int".into(),
        targets: vec![ChangeTarget {
            connection_id: "conn".into(),
            node: "db:shop".into(),
        }],
    }
}
fn snapshots() -> Vec<TargetSnapshot> {
    vec![TargetSnapshot {
        target: input().targets.remove(0),
        environment: "prod".into(),
        policy_revision: 2,
        connection_fingerprint: "native-credential-digest".into(),
    }]
}
async fn approved(repo: &DatabaseChangesRepo) -> DatabaseChange {
    let c = repo.create(&input(), "author", "author").await.unwrap();
    let c = repo
        .validate(&c, "executor", &snapshots(), "author", "author")
        .await
        .unwrap();
    let c = repo
        .transition(&c, "awaiting_review", "author", "author", "")
        .await
        .unwrap();
    repo.transition(&c, "approved", "reviewer", "reviewer", "")
        .await
        .unwrap()
}
#[tokio::test]
async fn independent_approval_cannot_be_bypassed_by_impersonation() {
    let (_dir, repo) = fixture().await;
    let c = repo.create(&input(), "author", "root").await.unwrap();
    let c = repo
        .validate(&c, "executor", &snapshots(), "author", "root")
        .await
        .unwrap();
    let c = repo
        .transition(&c, "awaiting_review", "author", "root", "")
        .await
        .unwrap();
    for (actor, real) in [
        ("author", "author"),
        ("reviewer", "root"),
        ("root", "root"),
        ("reviewer", "author"),
    ] {
        assert!(matches!(
            repo.transition(&c, "approved", actor, real, "").await,
            Err(Error::Forbidden(_))
        ))
    }
    assert_eq!(
        repo.transition(&c, "approved", "reviewer", "reviewer", "")
            .await
            .unwrap()
            .status,
        "approved"
    );
}
#[tokio::test]
async fn revision_erases_approval_and_stale_claim_fails() {
    let (_dir, repo) = fixture().await;
    let c = approved(&repo).await;
    let mut changed = input();
    changed.script.push_str("; DROP TABLE t");
    let revised = repo.revise(&c, &changed, "author", "author").await.unwrap();
    assert_eq!(revised.revision, 2);
    assert!(revised.approval_hash.is_none());
    assert!(repo
        .claim(&c, &snapshots(), "executor", "executor")
        .await
        .is_err());
}
#[tokio::test]
async fn credentials_targets_and_executor_are_hash_bound() {
    let (_dir, repo) = fixture().await;
    let c = approved(&repo).await;
    let mut changed = snapshots();
    changed[0].connection_fingerprint = "new-secret".into();
    assert!(repo
        .claim(&c, &changed, "executor", "executor")
        .await
        .is_err());
    changed = snapshots();
    changed[0].policy_revision += 1;
    assert!(repo
        .claim(&c, &changed, "executor", "executor")
        .await
        .is_err());
    assert!(repo
        .claim(&c, &snapshots(), "reviewer", "reviewer")
        .await
        .is_err());
}
#[tokio::test]
async fn duplicate_and_competing_claims_are_atomic_and_unknown_keeps_lock() {
    let (_dir, repo) = fixture().await;
    let first = approved(&repo).await;
    let second = approved(&repo).await;
    let attempts = repo
        .claim(&first, &snapshots(), "executor", "executor")
        .await
        .unwrap();
    assert!(repo
        .claim(&first, &snapshots(), "executor", "executor")
        .await
        .is_err());
    assert!(repo
        .claim(&second, &snapshots(), "executor", "executor")
        .await
        .is_err());
    assert_eq!(repo.get(&second.id).await.unwrap().status, "approved");
    repo.start_attempt(&attempts[0].id).await.unwrap();
    repo.finish_attempt(&attempts[0].id, false).await.unwrap();
    let unknown = repo
        .finish(&first.id, "executor", "executor")
        .await
        .unwrap();
    assert_eq!(unknown.status, "outcome_unknown");
    assert!(repo
        .claim(&second, &snapshots(), "executor", "executor")
        .await
        .is_err());
    let reconciled = repo
        .reconcile(
            &unknown,
            &attempts[0].id,
            "failed",
            "Inspected schema: column absent",
            "executor",
            "executor",
        )
        .await
        .unwrap();
    assert_eq!(reconciled.status, "failed");
    assert!(repo
        .claim(&second, &snapshots(), "executor", "executor")
        .await
        .is_ok());
}
#[tokio::test]
async fn recovery_never_requeues_sent_sql() {
    let (_dir, repo) = fixture().await;
    let c = approved(&repo).await;
    let a = repo
        .claim(&c, &snapshots(), "executor", "executor")
        .await
        .unwrap();
    repo.start_attempt(&a[0].id).await.unwrap();
    assert_eq!(repo.recover_interrupted().await.unwrap(), 1);
    assert_eq!(repo.recover_interrupted().await.unwrap(), 0);
    assert_eq!(
        repo.attempts(&c.id).await.unwrap()[0].state,
        "outcome_unknown"
    );
    assert!(repo.start_attempt(&a[0].id).await.is_err());
}
#[tokio::test]
async fn cancellation_cannot_be_overwritten_by_late_success() {
    let (_dir, repo) = fixture().await;
    let c = approved(&repo).await;
    let a = repo
        .claim(&c, &snapshots(), "executor", "executor")
        .await
        .unwrap();
    repo.start_attempt(&a[0].id).await.unwrap();
    let running = repo.get(&c.id).await.unwrap();
    repo.request_cancel(&running, "executor", "executor")
        .await
        .unwrap();
    assert!(repo.finish_attempt(&a[0].id, true).await.is_err());
    assert_eq!(
        repo.attempts(&c.id).await.unwrap()[0].state,
        "outcome_unknown"
    );
}
#[tokio::test]
async fn crash_before_send_releases_unsent_targets_without_replay() {
    let (_dir, repo) = fixture().await;
    let c = approved(&repo).await;
    let a = repo
        .claim(&c, &snapshots(), "executor", "executor")
        .await
        .unwrap();
    repo.recover_interrupted().await.unwrap();
    assert_eq!(repo.get(&c.id).await.unwrap().status, "failed");
    assert_eq!(repo.attempts(&c.id).await.unwrap()[0].state, "cancelled");
    assert!(repo.start_attempt(&a[0].id).await.is_err());
    let next = approved(&repo).await;
    assert!(repo
        .claim(&next, &snapshots(), "executor", "executor")
        .await
        .is_ok());
}
#[tokio::test]
async fn partial_rollout_preserves_completed_target_and_stops_unstarted_target() {
    let (_dir, repo) = fixture().await;
    let mut input = input();
    input.targets.push(ChangeTarget {
        connection_id: "other".into(),
        node: "db:shop".into(),
    });
    let mut snapshots = snapshots();
    let mut second = snapshots[0].clone();
    second.target.connection_id = "other".into();
    snapshots.push(second);
    let c = repo.create(&input, "author", "author").await.unwrap();
    let c = repo
        .validate(&c, "executor", &snapshots, "author", "author")
        .await
        .unwrap();
    let c = repo
        .transition(&c, "awaiting_review", "author", "author", "")
        .await
        .unwrap();
    let c = repo
        .transition(&c, "approved", "reviewer", "reviewer", "")
        .await
        .unwrap();
    let a = repo
        .claim(&c, &snapshots, "executor", "executor")
        .await
        .unwrap();
    repo.start_attempt(&a[0].id).await.unwrap();
    repo.finish_attempt(&a[0].id, true).await.unwrap();
    assert_eq!(
        repo.finish(&c.id, "executor", "executor")
            .await
            .unwrap()
            .status,
        "partially_applied"
    );
    let a = repo.attempts(&c.id).await.unwrap();
    assert_eq!(a[0].state, "succeeded");
    assert_eq!(a[1].state, "cancelled");
    assert!(repo.start_attempt(&a[1].id).await.is_err());
}
#[tokio::test]
async fn cancellation_before_send_is_terminal_after_restart() {
    let (_dir, repo) = fixture().await;
    let c = approved(&repo).await;
    repo.claim(&c, &snapshots(), "executor", "executor")
        .await
        .unwrap();
    let running = repo.get(&c.id).await.unwrap();
    repo.request_cancel(&running, "executor", "executor")
        .await
        .unwrap();
    repo.recover_interrupted().await.unwrap();
    assert_eq!(repo.get(&c.id).await.unwrap().status, "cancelled");
}
#[tokio::test]
async fn uncertain_single_target_can_be_reconciled_as_partially_applied() {
    let (_dir, repo) = fixture().await;
    let c = approved(&repo).await;
    let a = repo
        .claim(&c, &snapshots(), "executor", "executor")
        .await
        .unwrap();
    repo.start_attempt(&a[0].id).await.unwrap();
    repo.finish_attempt(&a[0].id, false).await.unwrap();
    let unknown = repo.finish(&c.id, "executor", "executor").await.unwrap();
    let partial = repo
        .reconcile(
            &unknown,
            &a[0].id,
            "partially_applied",
            "First DDL committed; second column absent",
            "executor",
            "executor",
        )
        .await
        .unwrap();
    assert_eq!(partial.status, "partially_applied");
}
