use super::*;

#[tokio::test]
async fn refine_binding_and_detach_are_scoped_to_workspace_and_user() {
    let pool = otto_state::db::test_pool().await;
    for id in ["alice", "bob"] {
        sqlx::query("INSERT INTO users(id,username,password_hash,is_root,created_at) VALUES(?,?, 'unused',1,'2026-10-08T00:00:00Z')")
            .bind(id).bind(id).execute(&pool).await.unwrap();
    }
    let dir = tempfile::tempdir().unwrap();
    for id in ["wa", "wb"] {
        sqlx::query("INSERT INTO workspaces(id,name,root_path,created_at) VALUES(?,?,?,'2026-10-08T00:00:00Z')")
            .bind(id).bind(id).bind(dir.path().to_str().unwrap()).execute(&pool).await.unwrap();
    }
    sqlx::query("INSERT INTO sessions(id,workspace_id,kind,provider,title,status,cwd,created_by,created_at,last_active_at) VALUES('sa','wa','agent','claude','Owned refine','exited',?,'alice','2026-10-08T00:00:00Z','2026-10-08T00:00:00Z')")
        .bind(dir.path().to_str().unwrap()).execute(&pool).await.unwrap();
    let ctx = ServerCtx::for_tests(&pool, dir.path()).await;
    let vault = ctx
        .vault
        .store()
        .create_vault("wa", "Global vault", dir.path().to_str().unwrap(), false)
        .await
        .unwrap();
    let mut run = tests::sample_run("done");
    run.ws_id = "wa".into();
    run.vault_id = vault;
    run.kind = "refine".into();
    run.note_path = "note.md".into();
    run.agents.truncate(1);
    run.agents[0].session_id = Some("sa".into());
    otto_state::VaultDocsRunsRepo::new(pool.clone())
        .upsert(&run_row(&run))
        .await
        .unwrap();
    // A newer turn in another workspace remains visible in global history,
    // but must not shadow the older conversation owned by Alice in wa.
    sqlx::query("INSERT INTO sessions(id,workspace_id,kind,provider,title,status,cwd,created_by,created_at,last_active_at) VALUES('sb','wb','agent','claude','Other workspace refine','exited',?,'alice','2026-10-08T01:00:00Z','2026-10-08T01:00:00Z')")
        .bind(dir.path().to_str().unwrap()).execute(&pool).await.unwrap();
    let mut other_run = run.clone();
    other_run.id = "later-other-workspace".into();
    other_run.ws_id = "wb".into();
    other_run.started_at = "2099-01-01T00:00:00Z".into();
    other_run.agents[0].session_id = Some("sb".into());
    let runs = otto_state::VaultDocsRunsRepo::new(pool.clone());
    runs.upsert(&run_row(&other_run)).await.unwrap();
    assert_eq!(
        runs.latest_refine_for_note(vault, "note.md")
            .await
            .unwrap()
            .unwrap()
            .id,
        other_run.id
    );
    let users = otto_state::UsersRepo::new(pool);
    let alice = users.get(&"alice".into()).await.unwrap();
    let bob = users.get(&"bob".into()).await.unwrap();
    let read = |ws: &str, user| {
        refine_session(
            Path((ws.to_string(), vault)),
            State(ctx.clone()),
            CurrentUser(user),
            Query(RefineSessionQ {
                path: "note.md".into(),
            }),
        )
    };
    assert_eq!(
        read("wa", alice.clone())
            .await
            .unwrap()
            .0
            .session_id
            .as_deref(),
        Some("sa")
    );
    assert!(
        read("wa", bob.clone())
            .await
            .unwrap()
            .0
            .session_id
            .is_none(),
        "another user must not inherit Alice's conversation"
    );
    assert!(
        read("wb", bob.clone())
            .await
            .unwrap()
            .0
            .session_id
            .is_none(),
        "another workspace must not inherit an unauthorized session"
    );
    assert_eq!(
        read("wb", alice.clone())
            .await
            .unwrap()
            .0
            .session_id
            .as_deref(),
        Some("sb")
    );
    let reset = reset_refine_session(
        Path(("wa".into(), vault)),
        State(ctx.clone()),
        CurrentUser(bob),
        Query(RefineSessionQ {
            path: "note.md".into(),
        }),
    )
    .await
    .unwrap();
    assert!(!reset.0.running);
    assert_eq!(
        read("wa", alice.clone())
            .await
            .unwrap()
            .0
            .session_id
            .as_deref(),
        Some("sa"),
        "Bob's reset must not detach Alice"
    );
    let reset = reset_refine_session(
        Path(("wa".into(), vault)),
        State(ctx.clone()),
        CurrentUser(alice.clone()),
        Query(RefineSessionQ {
            path: "note.md".into(),
        }),
    )
    .await
    .unwrap();
    assert!(!reset.0.running);
    assert!(
        read("wa", alice).await.unwrap().0.session_id.is_none(),
        "own reset tombstone prevents rehydration"
    );
}

#[tokio::test]
async fn governance_requires_visibility_before_private_memory_mutation() {
    let pool = otto_state::db::test_pool().await;
    let dir = tempfile::tempdir().unwrap();
    for id in ["owner", "editor"] {
        sqlx::query("INSERT INTO users(id,username,password_hash,is_root,created_at) VALUES(?,?, 'unused',0,'2026-10-08T00:00:00Z')")
            .bind(id).bind(id).execute(&pool).await.unwrap();
    }
    sqlx::query("INSERT INTO workspaces(id,name,root_path,created_at) VALUES('ws','Test',?,'2026-10-08T00:00:00Z')")
        .bind(dir.path().to_str().unwrap()).execute(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO workspace_members(workspace_id,user_id,role) VALUES('ws','editor','editor')",
    )
    .execute(&pool)
    .await
    .unwrap();
    let ctx = ServerCtx::for_tests(&pool, dir.path()).await;
    let memory: otto_memory::NewMemory = serde_json::from_value(serde_json::json!({
        "collection":"product", "scope":"workspace", "kind":"fact", "title":"Private", "body":"Private original", "visibility":"private", "source_kind":"manual"
    })).unwrap();
    let saved = ctx
        .memory
        .save("ws", "owner", vec![memory])
        .await
        .unwrap()
        .remove(0);
    let editor = otto_state::UsersRepo::new(pool)
        .get(&"editor".into())
        .await
        .unwrap();
    let app = crate::memory_gov::memory_gov_routes()
        .layer(axum::Extension(otto_core::auth::AuthUser(editor)))
        .with_state(ctx.clone());
    use tower::ServiceExt;
    for (suffix, body) in [
        (
            format!("{}/state", saved.id),
            serde_json::json!({"state":"contradicted"}),
        ),
        (format!("{}/forget", saved.id), serde_json::json!({})),
        (
            format!("{}/forget/undo", saved.id),
            serde_json::json!({"undo_token":"irrelevant"}),
        ),
        (
            format!("{}/split", saved.id),
            serde_json::json!({"parts":[{"title":"one","body":"part one"},{"title":"two","body":"part two"}]}),
        ),
        (
            "merge".into(),
            serde_json::json!({"ids":[saved.id,"other"],"title":"Merged","body":"merged text"}),
        ),
    ] {
        let response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(format!("/workspaces/ws/memory/{suffix}"))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 404, "governance route {suffix}");
    }
    let preserved = ctx.memory.get("ws", &saved.id).await.unwrap();
    assert_eq!(preserved.state, saved.state);
    assert!(preserved.active);
}

#[test]
fn detached_refine_callbacks_cannot_resurrect_reset_or_replace_new_turn() {
    let registry = new_refine_registry();
    let key = ("ws".into(), "owner".into(), 1, "note.md".into());
    registry.lock().unwrap().insert(
        key.clone(),
        RefineEntry {
            session_id: None,
            running: true,
            turn_id: Some("old".into()),
        },
    );
    update_refine_entry(&registry, &key, "old", Some(&"old-session".into()), false);
    assert_eq!(
        registry.lock().unwrap()[&key].session_id.as_deref(),
        Some("old-session")
    );
    registry
        .lock()
        .unwrap()
        .insert(key.clone(), RefineEntry::default());
    update_refine_entry(&registry, &key, "old", Some(&"old-session".into()), true);
    assert!(registry.lock().unwrap()[&key].session_id.is_none());
    registry.lock().unwrap().insert(
        key.clone(),
        RefineEntry {
            session_id: Some("new-session".into()),
            running: true,
            turn_id: Some("new".into()),
        },
    );
    update_refine_entry(&registry, &key, "old", Some(&"old-session".into()), true);
    let current = registry.lock().unwrap()[&key].clone();
    assert!(current.running);
    assert_eq!(current.session_id.as_deref(), Some("new-session"));
}
