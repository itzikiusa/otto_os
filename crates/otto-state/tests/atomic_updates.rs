//! E14 regressions exercise production file-backed split pools and foreign keys.
use otto_core::domain::{ConnectionKind, Environment};
use otto_state::{
    ConnectionSectionsRepo, ConnectionsRepo, DbPool, NewConnection, NewStory, NewVersion,
    ProductRepo, RetentionPolicy, RetentionRepo,
};
use serde_json::json;

async fn fixture() -> (tempfile::TempDir, DbPool) {
    let dir = tempfile::tempdir().unwrap();
    let pool = otto_state::open(&dir.path().join("state.db"))
        .await
        .unwrap();
    sqlx::query("INSERT INTO users (id, username, password_hash, display_name, is_root, created_at) VALUES ('u', 'u', 'hash', 'User', 1, '2026-01-01T00:00:00Z')")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO workspaces (id, name, root_path, created_at) VALUES ('w', 'Workspace', '/tmp', '2026-01-01T00:00:00Z')")
        .execute(&pool).await.unwrap();
    (dir, pool)
}

#[tokio::test]
async fn rejected_connection_patch_preserves_every_field_and_success_applies_all() {
    let (_dir, pool) = fixture().await;
    let repo = ConnectionsRepo::new(pool.clone());
    let original = repo
        .create(NewConnection {
            workspace_id: Some("w".into()),
            name: "old".into(),
            kind: ConnectionKind::Mysql,
            params: json!({"host":"old"}),
            secret_ref: Some("old-secret".into()),
            first_command: Some("old-command".into()),
            section_id: None,
            environment: Environment::Dev,
            read_only: false,
            created_by: "u".into(),
        })
        .await
        .unwrap();
    let params = json!({"host":"production"});
    let result = repo
        .update(
            &original.id,
            Some("new"),
            Some(&params),
            Some(Some("new-secret")),
            Some(Some("new-command")),
            Some(Some("absent-section")),
            Some(Environment::Prod),
            Some(true),
        )
        .await;
    assert!(result.is_err());
    let unchanged = repo.get(&original.id).await.unwrap();
    assert_eq!(
        serde_json::to_value(&unchanged).unwrap(),
        serde_json::to_value(&original).unwrap()
    );
    let updated = repo
        .update(
            &original.id,
            Some("new"),
            Some(&params),
            Some(Some("new-secret")),
            Some(Some("new-command")),
            Some(None),
            Some(Environment::Prod),
            Some(true),
        )
        .await
        .unwrap();
    assert_eq!(updated.name, "new");
    assert_eq!(updated.params, params);
    assert_eq!(updated.secret_ref.as_deref(), Some("new-secret"));
    assert_eq!(updated.first_command.as_deref(), Some("new-command"));
    assert_eq!(updated.environment, Environment::Prod);
    assert!(updated.read_only);
    // Outer None preserves a nullable field; inner None explicitly clears it.
    let cleared = repo
        .update(
            &original.id,
            None,
            None,
            Some(None),
            Some(None),
            None,
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(cleared.secret_ref, None);
    assert_eq!(cleared.first_command, None);
    assert_eq!(cleared.params, params);
    assert!(cleared.read_only);
    pool.close().await;
}

#[tokio::test]
async fn opposing_section_moves_cannot_both_commit_and_existing_cycles_are_rejected() {
    let (_dir, pool) = fixture().await;
    let repo = ConnectionSectionsRepo::new(pool.clone());
    let ws = "w".to_string();
    let user = "u".to_string();
    let a = repo
        .create(&ws, None, "A", "connections", &user)
        .await
        .unwrap();
    let b = repo
        .create(&ws, None, "B", "connections", &user)
        .await
        .unwrap();
    let c = repo
        .create(&ws, None, "C", "connections", &user)
        .await
        .unwrap();
    let (ab, ba) = tokio::join!(
        repo.reparent(&a.id, Some(&b.id)),
        repo.reparent(&b.id, Some(&a.id))
    );
    assert_ne!(
        ab.is_ok(),
        ba.is_ok(),
        "exactly one opposing move must commit"
    );
    let aa = repo.get(&a.id).await.unwrap();
    let bb = repo.get(&b.id).await.unwrap();
    assert!(aa.parent_id.is_none() || bb.parent_id.is_none());
    assert!(repo.reparent(&c.id, Some(&c.id)).await.is_err());
    assert!(repo.reparent(&c.id, Some("missing")).await.is_err());
    assert!(repo.reparent(&"missing".into(), None).await.is_err());
    // Simulate a cycle already persisted by an older binary.
    sqlx::query("UPDATE connection_sections SET parent_id = CASE id WHEN ? THEN ? ELSE ? END WHERE id IN (?, ?)")
        .bind(&a.id).bind(&b.id).bind(&a.id).bind(&a.id).bind(&b.id).execute(&pool).await.unwrap();
    assert!(repo.reparent(&c.id, Some(&a.id)).await.is_err());
    // Moving a cycle member to the root repairs the old data.
    assert!(repo
        .reparent(&a.id, None)
        .await
        .unwrap()
        .parent_id
        .is_none());
    assert!(repo.reparent(&c.id, Some(&b.id)).await.is_ok());
    pool.close().await;
}

#[tokio::test]
async fn concurrent_product_versions_have_unique_order_and_latest_body() {
    let (_dir, pool) = fixture().await;
    let repo = ProductRepo::new(pool.clone());
    let story = repo
        .create_story(NewStory {
            workspace_id: "w".into(),
            source_kind: "draft".into(),
            account_id: String::new(),
            source_key: String::new(),
            title: "Story".into(),
            url: String::new(),
            issue_type: None,
            stage: "draft".into(),
            cwd: None,
            parent_id: None,
            tree_kind: "story".into(),
            folder: String::new(),
            created_by: "u".into(),
        })
        .await
        .unwrap();
    let results = futures_util::future::join_all((0..32).map(|i| {
        repo.add_version(NewVersion {
            story_id: story.id.clone(),
            kind: "plan".into(),
            title: format!("Plan {i}"),
            body_md: format!("Body {i}"),
            raw_json: None,
            change_notes: None,
            created_by: "u".into(),
        })
    }))
    .await;
    let mut versions = results.into_iter().map(Result::unwrap).collect::<Vec<_>>();
    versions.sort_by_key(|v| v.version_no);
    assert_eq!(
        versions.iter().map(|v| v.version_no).collect::<Vec<_>>(),
        (1..=32).collect::<Vec<_>>()
    );
    let latest = repo.latest_plan_version(&story.id).await.unwrap().unwrap();
    assert_eq!(latest.id, versions[31].id);
    assert_eq!(latest.body_md, versions[31].body_md);
    assert_eq!(repo.list_versions(&story.id).await.unwrap().len(), 32);
    pool.close().await;
}

#[tokio::test]
async fn impossible_retention_windows_fail_before_deleting_and_next_pass_runs() {
    let (_dir, pool) = fixture().await;
    let repo = RetentionRepo::new(pool.clone());
    sqlx::query("INSERT INTO mcp_tool_calls (id, tool, args_json, ok, created_at) VALUES ('old-call', 'probe', '{}', 1, '2000-01-01T00:00:00Z')")
        .execute(&pool).await.unwrap();
    for days in [i64::MAX, 1_000_000_000] {
        let policy = RetentionPolicy {
            audit_log_days: days,
            ..Default::default()
        };
        assert!(repo.prune(&policy).await.is_err());
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM mcp_tool_calls WHERE id = 'old-call'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            count, 1,
            "an invalid later window must not delete earlier tables"
        );
    }
    assert_eq!(
        repo.prune(&RetentionPolicy::default())
            .await
            .unwrap()
            .mcp_tool_calls,
        1
    );
    // A disabled policy must remain a no-op even if old settings are malformed.
    let disabled = RetentionPolicy {
        enabled: false,
        work_events_days: i64::MAX,
        ..Default::default()
    };
    assert_eq!(repo.prune(&disabled).await.unwrap().total(), 0);
    pool.close().await;
}
