use super::*;

/// A refused second Ask AI never reaches the job's `load` (the only writer of
/// the scene file), so it cannot overwrite the file a running turn edits.
#[tokio::test]
async fn rejected_turn_cannot_overwrite_active_agent_file() {
    let id = otto_core::new_id();
    let path = std::env::temp_dir().join(format!("otto-canvas-turn-{id}"));
    let active = claim_scene(&id).expect("first turn");
    write_scene_source(&path, "agent work in progress")
        .await
        .unwrap();
    assert!(
        claim_scene(&id).is_none(),
        "second turn refused before its load"
    );
    let text = tokio::fs::read_to_string(&path).await.unwrap();
    tokio::fs::remove_file(&path).await.unwrap();
    drop(active);
    assert_eq!(text, "agent work in progress");
}

#[tokio::test]
async fn scratch_write_failure_is_reported_and_releases_ownership() {
    let id = otto_core::new_id();
    let missing = std::env::temp_dir()
        .join(format!("otto-canvas-absent-{id}"))
        .join("canvas.d2");
    let claim = claim_scene(&id).unwrap();
    assert!(write_scene_source(&missing, "source").await.is_err());
    drop(claim);
    assert!(
        claim_scene(&id).is_some(),
        "failed preparation releases ownership"
    );
}

async fn scene_fixture() -> (
    sqlx::SqlitePool,
    otto_state::CanvasRepo,
    otto_state::CanvasScene,
) {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::migrate!("../otto-state/migrations")
        .run(&pool)
        .await
        .unwrap();
    let repo = otto_state::CanvasRepo::new(pool.clone());
    let scene = repo
        .create(otto_state::NewScene {
            workspace_id: "w".into(),
            story_id: None,
            title: "Before".into(),
            doc_json: serde_json::json!({"format":"d2","source":"a -> b","viewport":{"x":1}})
                .to_string(),
            provider: "claude".into(),
            section: None,
            created_by: "u".into(),
        })
        .await
        .unwrap();
    (pool, repo, scene)
}

#[tokio::test]
async fn rejected_database_write_never_reports_unpersisted_success() {
    let (pool, repo, scene) = scene_fixture().await;
    sqlx::query("CREATE TRIGGER reject_canvas_write BEFORE UPDATE ON canvas_scenes BEGIN SELECT RAISE(ABORT, 'fixture write rejected'); END")
        .execute(&pool).await.unwrap();
    let result = commit_assist_doc(
        &repo,
        &scene,
        "a -> b",
        serde_json::json!({"format":"d2","source":"a -> changed"}),
        &mut String::new(),
    )
    .await;
    assert!(
        result.is_err(),
        "failed commit must not be broadcast as a successful document"
    );
    assert_eq!(
        repo.get(&scene.id).await.unwrap().unwrap().doc_json,
        scene.doc_json
    );
}

#[tokio::test]
async fn metadata_conflict_preserves_fresh_document_fields() {
    let (_pool, repo, scene) = scene_fixture().await;
    repo.update(
        &scene.id,
        otto_state::SceneUpdate {
            doc_json: Some(
                serde_json::json!({"format":"d2","source":"a -> b","viewport":{"x":77}})
                    .to_string(),
            ),
            title: Some("Renamed".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let result = commit_assist_doc(
        &repo,
        &scene,
        "a -> b",
        serde_json::json!({"format":"d2","source":"a -> changed","viewport":{"x":1}}),
        &mut String::new(),
    )
    .await
    .unwrap();
    assert_eq!(
        result["viewport"]["x"], 77,
        "agent source must merge onto the fresh metadata"
    );
    assert_eq!(result["source"], "a -> changed");
    assert_eq!(repo.get(&scene.id).await.unwrap().unwrap().title, "Renamed");
}

#[tokio::test]
async fn concurrent_manual_source_edit_wins_with_explanation() {
    let (_pool, repo, scene) = scene_fixture().await;
    let manual = serde_json::json!({"format":"d2","source":"manual"});
    repo.update(
        &scene.id,
        otto_state::SceneUpdate {
            doc_json: Some(manual.to_string()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let mut note = String::new();
    let result = commit_assist_doc(
        &repo,
        &scene,
        "a -> b",
        serde_json::json!({"format":"d2","source":"agent"}),
        &mut note,
    )
    .await
    .unwrap();
    assert_eq!(result, manual);
    assert!(note.contains("NOT applied"));
}

#[tokio::test]
async fn agent_file_read_enforces_scene_byte_budget() {
    let path = std::env::temp_dir().join(format!("otto-canvas-size-{}", otto_core::new_id()));
    let file = tokio::fs::File::create(&path).await.unwrap();
    file.set_len(crate::http::SCENE_BODY_LIMIT as u64 + 1)
        .await
        .unwrap();
    let result = read_agent_source(&path).await;
    tokio::fs::remove_file(path).await.unwrap();
    assert!(matches!(result, Err(Error::PayloadTooLarge(_))));
}

#[tokio::test]
async fn replacement_session_is_persisted_for_the_next_turn() {
    let (_, repo, scene) = scene_fixture().await;
    repo.set_session(&scene.id, &"deleted-session".into())
        .await
        .unwrap();
    let scene = repo.get(&scene.id).await.unwrap().unwrap();
    remember_session(&repo, &scene, &"replacement-session".into()).await;
    assert_eq!(
        repo.get(&scene.id)
            .await
            .unwrap()
            .unwrap()
            .session_id
            .as_deref(),
        Some("replacement-session")
    );
}

#[tokio::test]
async fn stopping_assist_restores_current_manual_content_and_format() {
    let (_pool, repo, scene) = scene_fixture().await;
    repo.update(
        &scene.id,
        otto_state::SceneUpdate {
            doc_json: Some(
                serde_json::json!({"format":"mermaid", "source":"flowchart TD\n manual"})
                    .to_string(),
            ),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let dir = std::env::temp_dir().join(format!("otto-stop-{}", otto_core::new_id()));
    tokio::fs::create_dir_all(&dir).await.unwrap();
    let (doc, result) = stopped_document(&repo, &scene.id, &dir).await.unwrap();
    assert_eq!(doc["format"], "mermaid");
    assert_eq!(result.mermaid.as_deref(), Some("flowchart TD\n manual"));
    assert_eq!(
        tokio::fs::read_to_string(dir.join("canvas.mermaid"))
            .await
            .unwrap(),
        "flowchart TD\n manual"
    );
    tokio::fs::remove_dir_all(dir).await.unwrap();
}
