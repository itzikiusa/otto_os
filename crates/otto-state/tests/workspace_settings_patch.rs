use otto_core::Error;
use otto_state::WorkspacesRepo;
use serde_json::json;

async fn fixture() -> WorkspacesRepo {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    sqlx::query("INSERT INTO workspaces(id,name,root_path,settings_json,created_at) VALUES ('ws','Workspace','/tmp',?, '2026-01-01T00:00:00Z')")
        .bind(json!({"notes":"old","default_provider":"claude","api_client":{"history_max_rows":100,"allow_local":false},"context":{"memory_md":"keep","context_version":1}}).to_string())
        .execute(&pool).await.unwrap();
    WorkspacesRepo::new(pool)
}

#[tokio::test]
async fn independent_concurrent_settings_patches_preserve_every_field() {
    let repo = fixture().await;
    let id = "ws".into();
    let notes = json!({"notes":"new"});
    let provider = json!({"default_provider":"codex"});
    let retention = json!({"api_client":{"history_max_rows":0}});
    let local = json!({"api_client":{"allow_local":true}});
    let (a, b, c, d) = tokio::join!(
        repo.patch_settings(&id, &notes),
        repo.patch_settings(&id, &provider),
        repo.patch_settings(&id, &retention),
        repo.patch_settings(&id, &local),
    );
    a.unwrap();
    b.unwrap();
    c.unwrap();
    d.unwrap();
    let saved = repo.get(&id).await.unwrap();
    assert_eq!(saved.settings["notes"], "new");
    assert_eq!(saved.settings["default_provider"], "codex");
    assert_eq!(
        saved.settings["api_client"],
        json!({"history_max_rows":0,"allow_local":true})
    );
    assert_eq!(saved.settings["context"]["memory_md"], "keep");
}

#[tokio::test]
async fn settings_patch_null_removes_only_named_keys_and_arrays_replace() {
    let repo = fixture().await;
    let id = "ws".into();
    repo.patch_settings(
        &id,
        &json!({"list":[1,2],"notes":"","api_client":{"history_max_rows":null}}),
    )
    .await
    .unwrap();
    let saved = repo
        .patch_settings(&id, &json!({"list":[],"default_provider":null}))
        .await
        .unwrap();
    assert_eq!(saved.settings["notes"], "");
    assert_eq!(saved.settings["list"], json!([]));
    assert!(saved.settings.get("default_provider").is_none());
    assert!(saved.settings["api_client"]
        .get("history_max_rows")
        .is_none());
    assert_eq!(saved.settings["api_client"]["allow_local"], false);
}

#[tokio::test]
async fn settings_patch_rejects_non_objects_and_context_and_missing_workspaces() {
    let repo = fixture().await;
    for patch in [
        json!(null),
        json!([]),
        json!("bad"),
        json!({"context":null}),
        json!({"context":{"memory_md":"overwrite"}}),
    ] {
        assert!(matches!(
            repo.patch_settings(&"ws".into(), &patch).await,
            Err(Error::Invalid(_))
        ));
    }
    assert!(matches!(
        repo.patch_settings(&"absent".into(), &json!({"notes":"x"}))
            .await,
        Err(Error::NotFound(_))
    ));
    assert_eq!(
        repo.get(&"ws".into()).await.unwrap().settings["context"]["memory_md"],
        "keep"
    );
}
