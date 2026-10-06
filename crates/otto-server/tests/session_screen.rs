//! `GET /api/v1/sessions/{id}/screen` through the real router (auth + feature
//! guard + handler): a live shell session's screen carries what was typed via
//! `POST /sessions/{id}/input`; the read is gated like the transcript (ws
//! viewer + owner-or-admin); an unknown id 404s; a session without a live PTY
//! answers `live: false` without being spawned.
use axum::body::Body;
use axum::http::Request;
use axum::Router;
use otto_core::domain::{Capability, Feature, SessionKind, WorkspaceRole};
use otto_server::ServerCtx;
use otto_state::{DbPool, GrantsRepo, NewSession, SessionsRepo};
use serde_json::{json, Value};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use tower::ServiceExt; // for `oneshot`

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

async fn call(
    app: &Router,
    method: &str,
    path: &str,
    token: &str,
    body: Option<Value>,
) -> (u16, Value) {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"));
    let body = match body {
        Some(v) => {
            req = req.header("content-type", "application/json");
            Body::from(serde_json::to_vec(&v).unwrap())
        }
        None => Body::empty(),
    };
    let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = resp.status().as_u16();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn session_screen_live_gated_unknown_and_offline() {
    let tmp = tempfile::TempDir::new().unwrap();
    let pool = mem_pool().await;
    let ctx = ServerCtx::for_tests(&pool, tmp.path().to_path_buf()).await;
    let users = otto_state::UsersRepo::new(pool.clone());
    let owner = users
        .create("owner", "unused", "Owner", true)
        .await
        .unwrap();
    // A non-root workspace VIEWER who holds the feature grant: the feature
    // guard lets them through, so a 403 here is the handler's owner gate.
    let viewer = users
        .create("viewer", "unused", "Viewer", false)
        .await
        .unwrap();
    let stranger = users
        .create("stranger", "unused", "Stranger", false)
        .await
        .unwrap();
    let grants = GrantsRepo::new(pool.clone());
    for u in [&viewer, &stranger] {
        grants
            .set_grants(&u.id, &[(Feature::Agents, Capability::View)])
            .await
            .unwrap();
    }
    let auth = otto_rbac::AuthRepo::new(pool.clone());
    let owner_token = auth.issue(&owner.id).await.unwrap();
    let viewer_token = auth.issue(&viewer.id).await.unwrap();
    let stranger_token = auth.issue(&stranger.id).await.unwrap();
    let workspace = ctx
        .workspaces
        .create("School", tmp.path().to_str().unwrap(), &owner.id)
        .await
        .unwrap();
    ctx.workspaces
        .set_member(&workspace.id, &viewer.id, WorkspaceRole::Viewer)
        .await
        .unwrap();
    let session = ctx
        .manager
        .create(
            &workspace,
            &owner.id,
            otto_core::api::CreateSessionReq {
                kind: SessionKind::Agent,
                provider: Some("shell".into()),
                title: Some("Screen test".into()),
                cwd: None,
                connection_id: None,
                meta: None,
                model: None,
            },
            Some(otto_pty::CommandSpec {
                program: "/bin/sh".into(),
                args: vec![],
                cwd: Some(tmp.path().to_string_lossy().into()),
                env: vec![("PS1".into(), "$ ".into())],
            }),
        )
        .await
        .unwrap();
    // The full daemon router: `/sessions/{id}/input` lives in the module
    // routers, `/screen` in the core protected routes.
    let (api_extras, root_extras) = otto_server::modules::module_routers(&ctx);
    let app = otto_server::build_router(ctx.clone(), api_extras, root_extras);
    let screen = format!("/api/v1/sessions/{}/screen", session.id);

    let (status, _) = call(
        &app,
        "POST",
        &format!("/api/v1/sessions/{}/input", session.id),
        &owner_token,
        Some(json!({"text": "echo SCHOOL_MARKER_42"})),
    )
    .await;
    assert!((200..300).contains(&status), "input: {status}");
    // The echoed command line also contains the marker; the command's OUTPUT
    // is the row that is exactly the marker — proof the shell ran it.
    let mut last = Value::Null;
    let mut found = false;
    for _ in 0..100 {
        let (status, body) = call(&app, "GET", &screen, &owner_token, None).await;
        assert_eq!(status, 200, "owner reads the screen: {body}");
        last = body;
        found = last["lines"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l.as_str().map(str::trim) == Some("SCHOOL_MARKER_42"));
        if found {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(found, "marker never reached the screen: {last}");
    assert_eq!(last["live"], true);
    assert!(last["cols"].as_u64().unwrap() > 0 && last["rows"].as_u64().unwrap() > 0);
    let lines = last["lines"].as_array().unwrap();
    assert!(lines.len() <= 60);
    assert_ne!(lines.last().unwrap(), "", "trailing blank rows are dropped");

    // Gate: a ws viewer who is not the owner (nor admin), and a non-member.
    let (status, _) = call(&app, "GET", &screen, &viewer_token, None).await;
    assert_eq!(status, 403, "non-owner viewer");
    let (status, _) = call(&app, "GET", &screen, &stranger_token, None).await;
    assert_eq!(status, 403, "non-member");

    // Unknown session id.
    let (status, _) = call(
        &app,
        "GET",
        "/api/v1/sessions/does-not-exist/screen",
        &owner_token,
        None,
    )
    .await;
    assert_eq!(status, 404);

    // A session row with no live PTY: `live: false`, and the read never
    // spawns it.
    let offline = SessionsRepo::new(pool.clone())
        .create(NewSession {
            workspace_id: workspace.id.clone(),
            kind: SessionKind::Agent,
            provider: "shell".into(),
            title: "offline".into(),
            cwd: tmp.path().to_string_lossy().into(),
            provider_session_id: None,
            connection_id: None,
            created_by: owner.id.clone(),
            meta: Value::Null,
        })
        .await
        .unwrap();
    let (status, body) = call(
        &app,
        "GET",
        &format!("/api/v1/sessions/{}/screen", offline.id),
        &owner_token,
        None,
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(
        body,
        json!({"live": false, "cols": 0, "rows": 0, "lines": []})
    );
    assert!(
        !ctx.manager.is_live(&offline.id),
        "a screen read never spawns"
    );

    ctx.manager.kill_session(&session.id).await.unwrap();
}
