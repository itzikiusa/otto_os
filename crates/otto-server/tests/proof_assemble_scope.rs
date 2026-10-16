//! `POST /proof-packs/{id}/assemble` runs `git` in `cwd` as the daemon and
//! files the diff where every workspace Viewer can read it. Only the root
//! owner on their own credential names any path; a non-root member is
//! confined to the pack's workspace root / registered repos / session.
use otto_core::domain::{Capability, Feature, WorkspaceRole};
use otto_server::ServerCtx;
use otto_state::DbPool;
use serde_json::json;
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
async fn a_member_cannot_assemble_a_diff_from_outside_the_workspace() {
    let tmp = tempfile::TempDir::new().unwrap();
    let pool = mem_pool().await;
    let ctx = ServerCtx::for_tests(&pool, tmp.path().join("data")).await;
    let users = otto_state::UsersRepo::new(pool.clone());
    let owner = users
        .create("owner", "unused", "Owner", true)
        .await
        .unwrap();
    let member = users
        .create("member", "unused", "Member", false)
        .await
        .unwrap();
    otto_state::GrantsRepo::new(pool.clone())
        .set_grants(&member.id, &[(Feature::ProofPack, Capability::Edit)])
        .await
        .unwrap();
    let auth = otto_rbac::AuthRepo::new(pool.clone());
    let member_token = auth.issue(&member.id).await.unwrap();
    let owner_token = auth.issue(&owner.id).await.unwrap();

    let ws_root = tmp.path().join("ws");
    let outside = tmp.path().join("someone-elses-repo");
    std::fs::create_dir_all(ws_root.join("sub")).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let ws = ctx
        .workspaces
        .create("W", ws_root.to_str().unwrap(), &owner.id)
        .await
        .unwrap();
    ctx.workspaces
        .set_member(&ws.id, &member.id, WorkspaceRole::Editor)
        .await
        .unwrap();

    let (api_extras, root_extras) = otto_server::modules::module_routers(&ctx);
    let app = otto_server::build_router(ctx.clone(), api_extras, root_extras);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}/api/v1", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::new();

    let pack: serde_json::Value = client
        .post(format!("{origin}/workspaces/{}/proof-packs", ws.id))
        .bearer_auth(&member_token)
        .json(&json!({ "work_item_kind": "manual", "work_item_id": "m1" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let pack_id = pack["pack"]["id"]
        .as_str()
        .or_else(|| pack["id"].as_str())
        .unwrap_or_else(|| panic!("pack id in {pack}"))
        .to_string();
    let assemble = |token: &str, cwd: &std::path::Path| {
        client
            .post(format!("{origin}/proof-packs/{pack_id}/assemble"))
            .bearer_auth(token)
            .json(&json!({ "cwd": cwd }))
            .send()
    };

    // Outside the workspace (and a `..` escape out of it): refused.
    for cwd in [
        outside.clone(),
        ws_root.join("..").join("someone-elses-repo"),
    ] {
        let r = assemble(&member_token, &cwd).await.unwrap();
        assert_eq!(r.status(), 403, "{}", cwd.display());
    }
    // Inside the workspace root: the member's normal flow still works.
    let r = assemble(&member_token, &ws_root.join("sub")).await.unwrap();
    assert_eq!(r.status(), 200);
    // The root owner on their own credential reads their own machine.
    let r = assemble(&owner_token, &outside).await.unwrap();
    assert_eq!(r.status(), 200);
    server.abort();
}
