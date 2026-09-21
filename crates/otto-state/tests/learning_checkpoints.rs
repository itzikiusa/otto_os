use otto_state::ImprovementsRepo;

#[tokio::test]
async fn claims_are_exclusive_and_only_success_advances_checkpoint() {
    let tmp = tempfile::tempdir().unwrap();
    let pool = otto_state::open(&tmp.path().join("test.db")).await.unwrap();
    let repo = ImprovementsRepo::new(pool);
    let first = repo.claim_evidence("session:a").await.unwrap().unwrap();
    assert!(repo.claim_evidence("session:a").await.unwrap().is_none());
    assert_eq!(first.checkpoint, serde_json::Value::Null);
    repo.finish_evidence("session:a", &first.token, None)
        .await
        .unwrap();
    let retry = repo.claim_evidence("session:a").await.unwrap().unwrap();
    assert_eq!(retry.checkpoint, serde_json::Value::Null);
    let next = serde_json::json!({"offset":42});
    repo.finish_evidence("session:a", &retry.token, Some(&next))
        .await
        .unwrap();
    let again = repo.claim_evidence("session:a").await.unwrap().unwrap();
    assert_eq!(again.checkpoint, next);
    // An expired owner's late completion must not erase the active owner.
    repo.finish_evidence("session:a", &first.token, Some(&serde_json::json!({})))
        .await
        .unwrap();
    assert!(repo.claim_evidence("session:a").await.unwrap().is_none());
    assert!(repo.claim_evidence("session:b").await.unwrap().is_some());
}

#[tokio::test]
async fn expired_claim_is_recovered_and_old_owner_cannot_commit() {
    let tmp = tempfile::tempdir().unwrap();
    let pool = otto_state::open(&tmp.path().join("test.db")).await.unwrap();
    let repo = ImprovementsRepo::new(pool.clone());
    let abandoned = repo
        .claim_evidence("session:abandoned")
        .await
        .unwrap()
        .unwrap();
    sqlx::query("UPDATE learning_checkpoints SET lease_until='2000-01-01T00:00:00Z'")
        .execute(&pool)
        .await
        .unwrap();
    let active = repo
        .claim_evidence("session:abandoned")
        .await
        .unwrap()
        .unwrap();
    assert_ne!(abandoned.token, active.token);
    repo.finish_evidence(
        "session:abandoned",
        &abandoned.token,
        Some(&serde_json::json!("wrong")),
    )
    .await
    .unwrap();
    assert!(repo
        .claim_evidence("session:abandoned")
        .await
        .unwrap()
        .is_none());
    repo.finish_evidence("session:abandoned", &active.token, None)
        .await
        .unwrap();
    assert_eq!(
        repo.claim_evidence("session:abandoned")
            .await
            .unwrap()
            .unwrap()
            .checkpoint,
        serde_json::Value::Null
    );
}

#[tokio::test]
async fn learning_trail_filters_recency_and_noise_before_limit() {
    use otto_core::domain::{SessionKind, TrailKind, TrailLevel, TrailSource};
    let tmp = tempfile::tempdir().unwrap();
    let pool = otto_state::open(&tmp.path().join("test.db")).await.unwrap();
    let uid = otto_state::UsersRepo::new(pool.clone())
        .create("test", "pw", "root", true)
        .await
        .unwrap()
        .id;
    let ws = otto_state::WorkspacesRepo::new(pool.clone())
        .create("test", tmp.path().to_str().unwrap(), &uid)
        .await
        .unwrap();
    let sessions = otto_state::SessionsRepo::new(pool.clone());
    let s = sessions
        .create(otto_state::NewSession {
            workspace_id: ws.id.clone(),
            kind: SessionKind::Agent,
            provider: "agy".into(),
            title: "test".into(),
            cwd: tmp.path().to_str().unwrap().into(),
            provider_session_id: None,
            connection_id: None,
            created_by: uid,
            meta: serde_json::json!({}),
        })
        .await
        .unwrap();
    let activity = otto_state::ActivityRepo::new(pool.clone());
    let row = |kind, source, summary: &str| otto_state::NewTrail {
        session_id: s.id.clone(),
        workspace_id: ws.id.clone(),
        source,
        kind,
        level: TrailLevel::Info,
        summary: summary.into(),
        detail: None,
    };
    let old = activity
        .append_trail(row(TrailKind::Note, TrailSource::User, "stale correction"))
        .await
        .unwrap();
    sqlx::query("UPDATE agent_trail SET ts='2000-01-01T00:00:00Z' WHERE id=?")
        .bind(&old.id)
        .execute(&pool)
        .await
        .unwrap();
    let recent = activity
        .append_trail(row(
            TrailKind::Prompt,
            TrailSource::User,
            "recent correction",
        ))
        .await
        .unwrap();
    for _ in 0..110 {
        activity
            .append_trail(row(TrailKind::Tool, TrailSource::Agent, "tool noise"))
            .await
            .unwrap();
    }
    let repo = ImprovementsRepo::new(pool);
    let rows = repo
        .learning_trail(&s.id, None, "2020-01-01T00:00:00Z")
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].summary, "recent correction");
    assert!(repo
        .learning_trail(&s.id, Some(&recent.id), "2020-01-01T00:00:00Z")
        .await
        .unwrap()
        .is_empty());
}
