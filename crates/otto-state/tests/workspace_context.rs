use otto_core::{api::UpdateWorkspaceContextReq, Error};
use otto_state::WorkspacesRepo;
use serde_json::{json, Value};

async fn fixture() -> (WorkspacesRepo, String) {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    sqlx::query("INSERT INTO workspaces(id,name,root_path,settings_json,created_at) VALUES ('ws','Workspace','/tmp',?, '2026-01-01T00:00:00Z')")
        .bind(json!({"other":true,"context":{"extra_context_md":"existing","repo_rules_md":"rule"}}).to_string())
        .execute(&pool).await.unwrap();
    (WorkspacesRepo::new(pool), "ws".into())
}
fn patch(value: Value) -> UpdateWorkspaceContextReq {
    serde_json::from_value(value).unwrap()
}

#[tokio::test]
async fn workspace_context_cas_preserves_legacy_fields_and_explicitly_clears() {
    let (repo, id) = fixture().await;
    let initial = patch(
        json!({"context_version":0,"goal_md":"Ship","memory_md":"UTC","references":["vault://guide"],"skills":["triage"],"soul":"careful"}),
    );
    let saved = repo.update_context(&id, &initial).await.unwrap();
    assert_eq!(saved.settings["context"]["context_version"], 1);
    assert_eq!(saved.settings["context"]["extra_context_md"], "existing");
    assert!(matches!(
        repo.update_context(&id, &initial).await,
        Err(Error::Conflict(_))
    ));
    let legacy = repo
        .update_context(&id, &patch(json!({"extra_context_md":"legacy edit"})))
        .await
        .unwrap();
    assert_eq!(legacy.settings["context"]["goal_md"], "Ship");
    assert_eq!(legacy.settings["context"]["memory_md"], "UTC");
    assert_eq!(legacy.settings["context"]["skills"], json!(["triage"]));
    assert_eq!(legacy.settings["context"]["soul"], "careful");
    assert!(matches!(
        repo.update_context(&id, &patch(json!({"memory_md":"unversioned"})))
            .await,
        Err(Error::Invalid(_))
    ));
    let cleared = repo
        .update_context(
            &id,
            &patch(json!({"context_version":2,"memory_md":"","references":[],"soul":null})),
        )
        .await
        .unwrap();
    assert_eq!(cleared.settings["context"]["memory_md"], "");
    assert_eq!(cleared.settings["context"]["references"], json!([]));
    assert!(cleared.settings["context"]["soul"].is_null());
    assert_eq!(cleared.settings["context"]["repo_rules_md"], "rule");
}

#[tokio::test]
async fn concurrent_workspace_context_editors_cannot_both_save_the_same_revision() {
    let (repo, id) = fixture().await;
    let a = patch(json!({"context_version":0,"memory_md":"first"}));
    let b = patch(json!({"context_version":0,"memory_md":"second"}));
    let (a, b) = tokio::join!(repo.update_context(&id, &a), repo.update_context(&id, &b));
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    assert!(matches!(a, Err(Error::Conflict(_))) || matches!(b, Err(Error::Conflict(_))));
    assert_eq!(
        repo.get(&id).await.unwrap().settings["context"]["context_version"],
        1
    );
}

#[tokio::test]
async fn workspace_context_survives_stale_generic_settings_and_machine_rule_updates() {
    let (repo, id) = fixture().await;
    let stale = repo.get(&id).await.unwrap().settings;
    repo.update_context(
        &id,
        &patch(json!({"context_version":0,"memory_md":"new memory"})),
    )
    .await
    .unwrap();
    repo.update_repo_rules(&id, "new rules").await.unwrap();
    repo.update(&id, None, None, Some(&stale), None)
        .await
        .unwrap();
    let saved = repo
        .update_context(&id, &patch(json!({"context_version":1,"goal_md":"goal"})))
        .await
        .unwrap();
    assert_eq!(saved.settings["context"]["memory_md"], "new memory");
    assert_eq!(saved.settings["context"]["repo_rules_md"], "new rules");
    assert_eq!(saved.settings["other"], true);
    let replaced = repo
        .update(&id, None, None, Some(&json!({"setting":42})), None)
        .await
        .unwrap();
    assert_eq!(replaced.settings["context"]["memory_md"], "new memory");
    assert_eq!(replaced.settings["setting"], 42);
}

#[tokio::test]
async fn workspace_context_validates_bounds_and_isolates_workspaces() {
    let (repo, id) = fixture().await;
    assert!(matches!(
        repo.update_context(
            &id,
            &patch(json!({"context_version":0,"goal_md":"a".repeat(32_001)}))
        )
        .await,
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        repo.update_context(
            &id,
            &patch(json!({"context_version":0,"references":vec!["x";101]}))
        )
        .await,
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        repo.update_context(
            &"missing".into(),
            &patch(json!({"context_version":0,"goal_md":"x"}))
        )
        .await,
        Err(Error::NotFound(_))
    ));
    assert_eq!(
        repo.get(&id).await.unwrap().settings["context"]["extra_context_md"],
        "existing"
    );
}
