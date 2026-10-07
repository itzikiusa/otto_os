use super::*;

/// Review S1-17: auto-archive re-decides under the resume lock against
/// the CURRENT row, so a session that became active after the sweep's
/// snapshot is left alone.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auto_archive_rechecks_staleness_under_the_lock() {
    let (manager, repo, workspace, user) = test_manager().await;
    let s = manager
        .create(
            &workspace,
            &user,
            CreateSessionReq {
                kind: SessionKind::Agent,
                provider: Some("shell".into()),
                title: Some("Stale".into()),
                cwd: Some("/tmp".into()),
                connection_id: None,
                model: None,
                meta: None,
            },
            None,
        )
        .await
        .unwrap();
    let cutoff = chrono::Utc::now() - chrono::Duration::days(3);
    sqlx::query("UPDATE sessions SET last_active_at='2000-01-01T00:00:00Z' WHERE id=?")
        .bind(&s.id)
        .execute(&repo.pool())
        .await
        .unwrap();
    assert!(
        !manager.archive_if_stale(&s.id, cutoff).await.unwrap(),
        "a live PTY must survive"
    );
    manager.kill_session(&s.id).await.unwrap();
    // `kill_session` kills the PTY in place; the status task's exit arm
    // evicts the handle from `live` and stamps `Exited` (+ last_active_at)
    // later, under the resume lock. On a slow runner that lands after the
    // stale stamp below — the session still looks live/fresh and the
    // archive is (correctly) refused. Wait for the eviction, then take the
    // lock once so the exit arm's status write is done too.
    tokio::time::timeout(Duration::from_secs(10), async {
        while manager.is_live(&s.id) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("killed PTY is evicted from live");
    drop(manager.resume_lock(&s.id).lock().await);
    let set_active = |at: chrono::DateTime<chrono::Utc>| {
        let pool = repo.pool();
        let id = s.id.clone();
        async move {
            sqlx::query("UPDATE sessions SET last_active_at = ? WHERE id = ?")
                .bind(at.to_rfc3339())
                .bind(&id)
                .execute(&pool)
                .await
                .unwrap();
        }
    };
    // The snapshot said stale, but the row is fresh now (the user opened it).
    set_active(chrono::Utc::now()).await;
    assert!(!manager.archive_if_stale(&s.id, cutoff).await.unwrap());
    assert!(!repo.get(&s.id).await.unwrap().archived);
    // Still stale under the lock → archived.
    set_active(chrono::Utc::now() - chrono::Duration::days(10)).await;
    let attached = manager.attach(&s.id);
    assert!(!manager.archive_if_stale(&s.id, cutoff).await.unwrap());
    drop(attached);
    repo.set_meta(&s.id, &serde_json::json!({"keep_alive":true}))
        .await
        .unwrap();
    assert!(!manager.archive_if_stale(&s.id, cutoff).await.unwrap());
    repo.set_meta(&s.id, &serde_json::json!({})).await.unwrap();
    assert!(manager.archive_if_stale(&s.id, cutoff).await.unwrap());
    assert!(repo.get(&s.id).await.unwrap().archived);
}

#[tokio::test]
async fn quality_auto_archive_ignores_irrelevant_history_and_reaches_later_candidates() {
    let (mut manager, repo, workspace, user) = test_manager().await;
    let settings = otto_state::SettingsRepo::new(repo.pool());
    settings
        .put("session_auto_archive_days", &serde_json::json!(3))
        .await
        .unwrap();
    Arc::get_mut(&mut manager).unwrap().settings = Some(settings);
    let pool = repo.pool();
    // Archived history must never be mapped to Session: a legacy bad
    // timestamp and a large payload cannot poison the current sweep.
    for i in 0..270 {
        let id = format!("quality-archive-{i:04}");
        let archived = i < 130;
        let meta = if archived {
            serde_json::json!({"history": "x".repeat(8192)})
        } else if i < 259 {
            serde_json::json!({"keep_alive":true})
        } else {
            serde_json::json!({})
        };
        sqlx::query("INSERT INTO sessions(id,workspace_id,kind,provider,title,status,cwd,created_by,created_at,last_active_at,archived,meta_json) VALUES(?,?,'agent','claude','archive fixture','exited','/tmp',?,'2000-01-01T00:00:00Z',?,?,?)")
            .bind(&id).bind(&workspace.id).bind(&user)
            .bind(if archived { "unreadable legacy timestamp" } else { "2000-01-01T00:00:00Z" })
            .bind(archived as i64).bind(meta.to_string()).execute(&pool).await.unwrap();
    }
    let attached = "quality-archive-0259".to_string();
    let guard = manager.attach(&attached);
    assert_eq!(manager.auto_archive_stale().await, 10);
    assert!(!repo.get(&attached).await.unwrap().archived);
    assert!(
        !repo
            .get(&"quality-archive-0258".into())
            .await
            .unwrap()
            .archived
    );
    for i in 260..270 {
        assert!(
            repo.get(&format!("quality-archive-{i:04}"))
                .await
                .unwrap()
                .archived
        );
    }
    drop(guard);
}
