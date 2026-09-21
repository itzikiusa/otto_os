use otto_core::network_profiles::NetworkProfileInput;
use otto_state::network_profiles::NetworkProfilesRepo;
use serde_json::json;

async fn fixture() -> sqlx::SqlitePool {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    sqlx::query("INSERT INTO users(id,username,password_hash,created_at) VALUES ('user','user','x','2026-01-01T00:00:00Z')").execute(&pool).await.unwrap();
    for id in ["one", "two"] {
        sqlx::query("INSERT INTO workspaces(id,name,root_path,created_at) VALUES (?,?,'/tmp','2026-01-01T00:00:00Z')").bind(id).bind(id).execute(&pool).await.unwrap();
    }
    sqlx::query("INSERT INTO connections(id,workspace_id,name,kind,params_json,created_by,created_at) VALUES ('ssh','one','Office','ssh','{}','user','2026-01-01T00:00:00Z')").execute(&pool).await.unwrap();
    pool
}
fn input() -> NetworkProfileInput {
    serde_json::from_value(
        json!({"name":"Office", "ssh_connection_id":"ssh", "endpoints":[
            {"name":"db", "remote_host":"db.internal", "remote_port":5432}
        ]}),
    )
    .unwrap()
}
#[tokio::test]
async fn network_profiles_preserve_version_and_validate_workspace_and_archive() {
    let pool = fixture().await;
    let repo = NetworkProfilesRepo::new(pool.clone());
    let profile = repo
        .create(&"one".into(), &"user".into(), input())
        .await
        .unwrap();
    assert!(repo
        .create(&"two".into(), &"user".into(), input())
        .await
        .is_err());
    assert!(repo
        .selected(&"two".into(), &json!({"network_profile_id":profile.id}))
        .await
        .is_err());
    assert!(repo
        .selected(&"one".into(), &json!({"network_profile_id":42}))
        .await
        .is_err());
    assert!(repo
        .selected(&"one".into(), &json!({"network_profile_id":null}))
        .await
        .unwrap()
        .is_none());
    let sessions = otto_state::SessionsRepo::new(pool);
    let session = sessions
        .create(otto_state::sessions::NewSession {
            workspace_id: "one".into(),
            kind: otto_core::domain::SessionKind::Agent,
            provider: "shell".into(),
            title: "Shell".into(),
            cwd: "/tmp".into(),
            provider_session_id: None,
            connection_id: None,
            created_by: "user".into(),
            meta: json!({"network_profile_id":profile.id}),
        })
        .await
        .unwrap();
    assert!(sessions
        .replace_meta_keys(&session.id, &json!({"network_profile_id":false}))
        .await
        .is_err());
    sessions
        .replace_meta_keys(&session.id, &json!({"network_profile_id":null}))
        .await
        .unwrap();
    assert!(sessions
        .get(&session.id)
        .await
        .unwrap()
        .meta
        .get("network_profile_id")
        .is_none_or(|value| value.is_null()));
    let mut archived = input();
    archived.archived = true;
    let updated = repo
        .update(&profile.id, archived, profile.version)
        .await
        .unwrap();
    assert_eq!(updated.version, profile.version + 1);
    assert!(repo
        .update(&profile.id, input(), profile.version)
        .await
        .is_err());
    assert!(repo
        .selected(&"one".into(), &json!({"network_profile_id":profile.id}))
        .await
        .is_err());
    assert_eq!(repo.list(&"one".into()).await.unwrap().len(), 1);
}
