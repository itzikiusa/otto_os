//! Per-session ownership isolation tests (Task 3.2, leaks #L1–#L7; Task 3.3,
//! leaks #L8–#L9).
//!
//! These drive the **real** sessions REST router (`api_router`) and the
//! terminal WebSocket router (`ws_router`) end-to-end via tower's `oneshot`,
//! with the `AuthUser` extension injected exactly as the production
//! `auth_middleware` does. No PTYs are spawned: sessions are inserted straight
//! into the migrated SQLite store (the manager's `get`/`list*` read from that
//! store), so the test exercises only the authorization path.
//!
//! The security property under test (Task 3.2): a workspace **editor** who is
//! *not* the session owner must get **403** on get/patch/delete/restart/
//! archive/unarchive of another user's session, and must not see that session
//! in the list — while the owner, a workspace **admin**, and **root** retain
//! full access.
//!
//! Task 3.3 additions:
//! - #L9: terminal attach (`GET /ws/term/{id}?token=`) is now owner-only; a
//!   workspace editor who is not the owner gets 403 before the WS upgrade.
//! - #L8: `POST /app/kill-sessions` requires root; non-root gets 403.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::Request;
use axum::http::{Method, StatusCode};
use axum::Router;
use chrono::Utc;
use otto_core::auth::{AuthUser, RoleChecker};
use otto_core::domain::{Session, SessionKind, User};
use otto_core::Id;
use otto_rbac::{tokens::AuthRepo, RbacAuthenticator, RbacRoleChecker};
use otto_sessions::{api_router, ws_router, ProviderRegistry, SessionManager, SessionsCtx};
use otto_state::{SessionsRepo, SqlitePool, WorkspacesRepo};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use tokio::sync::broadcast;
use tower::ServiceExt; // for `oneshot`

// ---------------------------------------------------------------------------
// Test context implementing SessionsCtx
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Ctx {
    manager: Arc<SessionManager>,
    roles: Arc<dyn RoleChecker>,
    workspaces: WorkspacesRepo,
}

impl SessionsCtx for Ctx {
    fn manager(&self) -> &Arc<SessionManager> {
        &self.manager
    }
    fn roles(&self) -> &Arc<dyn RoleChecker> {
        &self.roles
    }
    fn workspaces(&self) -> &WorkspacesRepo {
        &self.workspaces
    }
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

async fn mem_pool() -> SqlitePool {
    let opts = SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1) // one shared in-memory connection
        .connect_with(opts)
        .await
        .expect("connect in-memory sqlite");
    sqlx::migrate!("../otto-state/migrations")
        .run(&pool)
        .await
        .expect("run migrations");
    pool
}

fn user(id: &str, is_root: bool) -> User {
    User {
        id: id.into(),
        username: id.into(),
        display_name: id.into(),
        is_root,
        disabled: false,
        created_at: Utc::now(),
    }
}

async fn seed_user(pool: &SqlitePool, id: &str, is_root: bool) {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO users (id, username, password_hash, display_name, is_root, created_at)
         VALUES (?, ?, 'x', ?, ?, ?)",
    )
    .bind(id)
    .bind(id)
    .bind(id)
    .bind(is_root as i64)
    .bind(&now)
    .execute(pool)
    .await
    .expect("seed user");
}

async fn seed_workspace(pool: &SqlitePool, ws_id: &str) {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO workspaces (id, name, root_path, settings_json, archived, created_at)
         VALUES (?, 'ws', '/tmp', '{}', 0, ?)",
    )
    .bind(ws_id)
    .bind(&now)
    .execute(pool)
    .await
    .expect("seed workspace");
}

async fn set_member(pool: &SqlitePool, ws_id: &str, user_id: &str, role: &str) {
    sqlx::query("INSERT INTO workspace_members (workspace_id, user_id, role) VALUES (?, ?, ?)")
        .bind(ws_id)
        .bind(user_id)
        .bind(role)
        .execute(pool)
        .await
        .expect("set member");
}

/// Insert a session row owned by `created_by` and return its id.
async fn insert_session(repo: &SessionsRepo, ws: &str, created_by: &str) -> Id {
    let s = repo
        .create(otto_state::NewSession {
            workspace_id: ws.into(),
            kind: SessionKind::Agent,
            provider: "shell".into(),
            title: "t".into(),
            cwd: "/tmp".into(),
            provider_session_id: None,
            connection_id: None,
            created_by: created_by.into(),
            meta: serde_json::Value::Null,
        })
        .await
        .expect("insert session");
    s.id
}

/// Build the router + context over a freshly seeded pool.
async fn app(pool: &SqlitePool) -> Router {
    let repo = SessionsRepo::new(pool.clone());
    let (events, _rx) = broadcast::channel(64);
    let providers = ProviderRegistry::new(None);
    let manager = Arc::new(SessionManager::new(repo, events, providers));
    let ctx = Ctx {
        manager,
        roles: Arc::new(RbacRoleChecker::new(pool.clone())),
        workspaces: WorkspacesRepo::new(pool.clone()),
    };
    api_router::<Ctx>().with_state(ctx)
}

/// Issue a request as `caller` (AuthUser injected like the auth middleware) and
/// return the response status.
async fn status_as(app: &Router, caller: &User, method: Method, uri: &str) -> StatusCode {
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    req.extensions_mut().insert(AuthUser(caller.clone()));
    app.clone().oneshot(req).await.unwrap().status()
}

/// The per-session control endpoints we gate (method, suffix path).
fn control_routes(id: &str) -> Vec<(Method, String)> {
    vec![
        (Method::GET, format!("/sessions/{id}")),
        (Method::PATCH, format!("/sessions/{id}")),
        (Method::DELETE, format!("/sessions/{id}")),
        (Method::POST, format!("/sessions/{id}/restart")),
        (Method::POST, format!("/sessions/{id}/archive")),
        (Method::POST, format!("/sessions/{id}/unarchive")),
    ]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// #L2–#L7: a non-owner workspace **editor** is forbidden on every per-session
/// operation against another user's session.
#[tokio::test]
async fn non_owner_editor_forbidden_on_all_controls() {
    let pool = mem_pool().await;
    seed_user(&pool, "alice", false).await;
    seed_user(&pool, "bob", false).await;
    seed_workspace(&pool, "ws1").await;
    set_member(&pool, "ws1", "alice", "editor").await;
    set_member(&pool, "ws1", "bob", "editor").await;

    let repo = SessionsRepo::new(pool.clone());
    let sid = insert_session(&repo, "ws1", "alice").await; // alice owns it
    let app = app(&pool).await;
    let bob = user("bob", false);

    for (method, uri) in control_routes(&sid) {
        let status = status_as(&app, &bob, method.clone(), &uri).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "bob (editor, non-owner) must get 403 on {method} {uri}, got {status}"
        );
    }
}

/// The **owner** can act on their own session (not 403).
#[tokio::test]
async fn owner_can_control_own_session() {
    let pool = mem_pool().await;
    seed_user(&pool, "alice", false).await;
    seed_workspace(&pool, "ws1").await;
    set_member(&pool, "ws1", "alice", "viewer").await; // even a mere viewer-owner

    let repo = SessionsRepo::new(pool.clone());
    let sid = insert_session(&repo, "ws1", "alice").await;
    let app = app(&pool).await;
    let alice = user("alice", false);

    // GET/PATCH/archive/unarchive should not be 403 for the owner. (restart and
    // delete touch the live PTY layer, which isn't spun up here; we assert the
    // ownership gate lets the owner through on the read/metadata ops.)
    for (method, uri) in [
        (Method::GET, format!("/sessions/{sid}")),
        (Method::PATCH, format!("/sessions/{sid}")),
        (Method::POST, format!("/sessions/{sid}/archive")),
        (Method::POST, format!("/sessions/{sid}/unarchive")),
    ] {
        let status = status_as(&app, &alice, method.clone(), &uri).await;
        assert_ne!(
            status,
            StatusCode::FORBIDDEN,
            "owner alice must not be forbidden on {method} {uri}"
        );
    }
}

/// A workspace **admin** (non-owner) and **root** retain full control.
#[tokio::test]
async fn admin_and_root_can_control_any_session() {
    let pool = mem_pool().await;
    seed_user(&pool, "alice", false).await;
    seed_user(&pool, "carol", false).await; // workspace admin
    seed_workspace(&pool, "ws1").await;
    set_member(&pool, "ws1", "alice", "editor").await;
    set_member(&pool, "ws1", "carol", "admin").await;

    let repo = SessionsRepo::new(pool.clone());
    let sid = insert_session(&repo, "ws1", "alice").await; // alice owns it
    let app = app(&pool).await;

    for caller in [user("carol", false), user("root", true)] {
        let status = status_as(&app, &caller, Method::GET, &format!("/sessions/{sid}")).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "{} must read alice's session, got {status}",
            caller.username
        );
        let status = status_as(
            &app,
            &caller,
            Method::POST,
            &format!("/sessions/{sid}/archive"),
        )
        .await;
        assert_ne!(
            status,
            StatusCode::FORBIDDEN,
            "{} must control alice's session",
            caller.username
        );
    }
}

/// #L1: the session list is owner-scoped for a non-admin caller (B does not see
/// A's sessions), while admin/root see everyone's.
#[tokio::test]
async fn list_is_owner_scoped_for_non_admin() {
    let pool = mem_pool().await;
    seed_user(&pool, "alice", false).await;
    seed_user(&pool, "bob", false).await;
    seed_user(&pool, "carol", false).await;
    seed_workspace(&pool, "ws1").await;
    set_member(&pool, "ws1", "alice", "editor").await;
    set_member(&pool, "ws1", "bob", "editor").await;
    set_member(&pool, "ws1", "carol", "admin").await;

    let repo = SessionsRepo::new(pool.clone());
    let a1 = insert_session(&repo, "ws1", "alice").await;
    let b1 = insert_session(&repo, "ws1", "bob").await;
    let app = app(&pool).await;

    // bob (editor, non-admin) sees only his own.
    let bob_list = list_sessions(&app, &user("bob", false), "ws1").await;
    let bob_ids: Vec<&str> = bob_list.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(
        bob_ids,
        vec![b1.as_str()],
        "bob must see only his own session"
    );
    assert!(!bob_ids.contains(&a1.as_str()), "bob must not see alice's");

    // carol (workspace admin) and root see both.
    for caller in [user("carol", false), user("root", true)] {
        let list = list_sessions(&app, &caller, "ws1").await;
        assert_eq!(
            list.len(),
            2,
            "{} (admin/root) must see all sessions",
            caller.username
        );
    }
}

/// GET the workspace session list as `caller`, returning the deserialized rows.
async fn list_sessions(app: &Router, caller: &User, ws: &str) -> Vec<Session> {
    let mut req = Request::builder()
        .method(Method::GET)
        .uri(format!("/workspaces/{ws}/sessions"))
        .body(Body::empty())
        .unwrap();
    req.extensions_mut().insert(AuthUser(caller.clone()));
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "list must be 200");
    let body = http_body_util::BodyExt::collect(resp.into_body())
        .await
        .unwrap()
        .to_bytes();
    serde_json::from_slice(&body).expect("decode session list")
}

// ---------------------------------------------------------------------------
// Task 3.3 helpers
// ---------------------------------------------------------------------------

/// Build the WS router (carries its own token authenticator) over the same
/// pool, sharing the manager+roles+workspaces with the HTTP app.
async fn ws_app(pool: &SqlitePool) -> Router {
    let repo = SessionsRepo::new(pool.clone());
    let (events, _rx) = broadcast::channel(64);
    let providers = ProviderRegistry::new(None);
    let manager = Arc::new(SessionManager::new(repo, events, providers));
    let ctx = Ctx {
        manager,
        roles: Arc::new(RbacRoleChecker::new(pool.clone())),
        workspaces: WorkspacesRepo::new(pool.clone()),
    };
    let auth = Arc::new(RbacAuthenticator::new(pool.clone()));
    ws_router(auth, ctx)
}

/// Issue a real bearer token for `user_id` via `AuthRepo` so the WS handler
/// can validate it via `RbacAuthenticator::authenticate`.
async fn mint_token(pool: &SqlitePool, user_id: &str) -> String {
    let repo = AuthRepo::new(pool.clone());
    repo.issue(&user_id.into()).await.expect("issue token")
}

/// Send a bare GET (no WS upgrade headers) to the terminal endpoint with the
/// given raw token in the query string and return the status code. When the
/// auth/owner gate fires the handler returns 403 *before* the upgrade, so this
/// lets us distinguish "forbidden" from "auth passed, upgrade rejected".
async fn term_ws_status(app: &Router, session_id: &Id, token: &str) -> StatusCode {
    let req = Request::builder()
        .method(Method::GET)
        .uri(format!("/ws/term/{session_id}?token={token}"))
        .body(Body::empty())
        .unwrap();
    app.clone().oneshot(req).await.unwrap().status()
}

// ---------------------------------------------------------------------------
// Task 3.3 tests — #L9 terminal attach
// ---------------------------------------------------------------------------

/// #L9: a workspace **editor** who is NOT the session owner must get 403 on
/// `GET /ws/term/{id}` before the WebSocket upgrade.
#[tokio::test]
async fn non_owner_editor_cannot_attach_terminal() {
    let pool = mem_pool().await;
    seed_user(&pool, "alice", false).await;
    seed_user(&pool, "bob", false).await;
    seed_workspace(&pool, "ws1").await;
    set_member(&pool, "ws1", "alice", "editor").await;
    set_member(&pool, "ws1", "bob", "editor").await;

    let repo = SessionsRepo::new(pool.clone());
    let sid = insert_session(&repo, "ws1", "alice").await; // alice owns it
    let app = ws_app(&pool).await;

    // Bob (editor, non-owner) must be rejected before the WS upgrade.
    let bob_token = mint_token(&pool, "bob").await;
    let status = term_ws_status(&app, &sid, &bob_token).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "bob (editor, non-owner) must get 403 on terminal attach, got {status}"
    );
}

/// #L9 complement: the **owner** and a workspace **admin** must pass the
/// attach gate (not get 403); root too. The request will fail the WS upgrade
/// (no Upgrade header) but that's 426 / 400, not 403.
#[tokio::test]
async fn owner_and_admin_can_attach_terminal() {
    let pool = mem_pool().await;
    seed_user(&pool, "alice", false).await;
    seed_user(&pool, "carol", false).await;
    // "root_usr" is seeded as root (is_root = true)
    seed_user(&pool, "root_usr", true).await;
    seed_workspace(&pool, "ws1").await;
    set_member(&pool, "ws1", "alice", "editor").await;
    set_member(&pool, "ws1", "carol", "admin").await;

    let repo = SessionsRepo::new(pool.clone());
    let sid = insert_session(&repo, "ws1", "alice").await; // alice owns it
    let app = ws_app(&pool).await;

    for (label, uid) in [("owner alice", "alice"), ("ws-admin carol", "carol"), ("root", "root_usr")] {
        let token = mint_token(&pool, uid).await;
        let status = term_ws_status(&app, &sid, &token).await;
        assert_ne!(
            status,
            StatusCode::FORBIDDEN,
            "{label} must not be forbidden on terminal attach, got {status}"
        );
    }
}

// ---------------------------------------------------------------------------
// Task 3.3 tests — #L8 kill-all root gate
// ---------------------------------------------------------------------------

/// #L8: a non-root authenticated user must get 403 on `POST /app/kill-sessions`.
#[tokio::test]
async fn non_root_cannot_kill_all_sessions() {
    let pool = mem_pool().await;
    seed_user(&pool, "alice", false).await;
    seed_workspace(&pool, "ws1").await;
    set_member(&pool, "ws1", "alice", "admin").await; // give her the highest non-root role

    let app = app(&pool).await;
    let alice = user("alice", false);
    let status = status_as(&app, &alice, Method::POST, "/app/kill-sessions").await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "non-root must get 403 on kill-all, got {status}"
    );
}

/// #L8 complement: root must be able to call `POST /app/kill-sessions` (not 403).
#[tokio::test]
async fn root_can_kill_all_sessions() {
    let pool = mem_pool().await;
    seed_user(&pool, "root_usr", true).await;

    let app = app(&pool).await;
    let root = user("root_usr", true);
    let status = status_as(&app, &root, Method::POST, "/app/kill-sessions").await;
    assert_ne!(
        status,
        StatusCode::FORBIDDEN,
        "root must not be forbidden on kill-all, got {status}"
    );
}
