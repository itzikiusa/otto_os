//! Resuming an agent whose folder was deleted is refused end-to-end (S20-306):
//! `POST /sessions/{id}/resume` (and `/restart`) answer 409 with the
//! "no longer exists" message, and the folder is NOT recreated empty — the
//! agent never starts in a blank directory pretending to be the user's repo.
//! The manager's pure check is unit-tested in otto-sessions; this pins the
//! HTTP path (auth → route → manager → error mapping).
use otto_core::domain::{SessionKind, SessionStatus};
use otto_server::ServerCtx;
use otto_state::{DbPool, NewSession, SessionsRepo};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

async fn mem_pool() -> DbPool {
    let opts = SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .expect("connect in-memory sqlite");
    sqlx::migrate!("../otto-state/migrations")
        .run(&pool)
        .await
        .expect("run migrations");
    pool.into()
}

#[tokio::test]
async fn resuming_an_agent_whose_folder_was_deleted_is_409_and_creates_nothing() {
    let tmp = tempfile::TempDir::new().unwrap();
    let pool = mem_pool().await;
    let ctx = ServerCtx::for_tests(&pool, tmp.path().join("data")).await;
    let users = otto_state::UsersRepo::new(pool.clone());
    let owner = users
        .create("owner", "unused", "Owner", true)
        .await
        .unwrap();
    let token = otto_rbac::AuthRepo::new(pool.clone())
        .issue(&owner.id)
        .await
        .unwrap();
    let ws = ctx
        .workspaces
        .create("W", tmp.path().to_str().unwrap(), &owner.id)
        .await
        .unwrap();
    // The project folder the agent worked in — deleted since.
    let gone = tmp.path().join("projects").join("deleted-repo");
    let sessions = SessionsRepo::new(pool.clone());
    let s = sessions
        .create(NewSession {
            workspace_id: ws.id.clone(),
            kind: SessionKind::Agent,
            provider: "claude".into(),
            title: "old work".into(),
            cwd: gone.to_string_lossy().into_owned(),
            provider_session_id: Some("00000000-0000-4000-8000-000000000001".into()),
            connection_id: None,
            created_by: owner.id.clone(),
            meta: serde_json::json!({}),
        })
        .await
        .unwrap();
    sessions
        .update_status(&s.id, SessionStatus::Exited)
        .await
        .unwrap();

    let (api_extras, root_extras) = otto_server::modules::module_routers(&ctx);
    let app = otto_server::build_router(ctx.clone(), api_extras, root_extras);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}/api/v1", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::new();

    for verb in ["resume", "restart"] {
        let r = client
            .post(format!("{origin}/sessions/{}/{verb}", s.id))
            .bearer_auth(&token)
            .json(&serde_json::json!({}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 409, "{verb} of a deleted folder");
        let body: serde_json::Value = r.json().await.unwrap();
        assert!(
            body["message"]
                .as_str()
                .unwrap_or("")
                .contains("no longer exists"),
            "{verb}: the refusal names the missing folder: {body}"
        );
        assert!(!gone.exists(), "{verb} must not recreate the folder empty");
    }
    // The row is untouched: still exited, still resumable once restored.
    let after = sessions.get(&s.id).await.unwrap();
    assert_eq!(after.status, SessionStatus::Exited);
    assert!(!after.archived);
    server.abort();
}
