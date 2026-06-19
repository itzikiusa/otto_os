//! Integration tests for the admin active-sessions overview + terminate
//! (RBAC Task 4.2):
//!   GET  /api/v1/admin/sessions
//!   POST /api/v1/admin/sessions/{id}/terminate
//!
//! Uses the same minimal-router harness as `rbac_matrix.rs` / `grants_api.rs`:
//! a lightweight `TestCtx` (pool + a real `SessionManager`) that implements
//! `AdminSessionsCtx` + `HasGrants`, the production feature-guard middleware,
//! and a stub `AuthUser` injection layer — without assembling the full
//! `ServerCtx` graph.
//!
//! The security focus:
//! - a **non-root** user holding `Users:Admin` AND root can list sessions owned
//!   by *other* users (the sanctioned cross-user view);
//! - a user *without* `Users:Admin` → 403;
//! - `terminate` kills the session (it ends up `exited`) **and** fires the
//!   forced-eviction signal — proven by subscribing an `evict_signal` receiver
//!   before the call and asserting it receives the unit (the Task 4.1 WS
//!   integration assertion, exercised end-to-end here).

use std::sync::Arc;

use axum::body::Body;
use axum::extract::Request;
use axum::http::{Method, StatusCode};
use axum::middleware::{from_fn, from_fn_with_state, Next};
use axum::routing::{get, post};
use axum::Router;
use chrono::Utc;
use http_body_util::BodyExt;
use otto_core::auth::AuthUser;
use otto_core::domain::{
    Capability, Feature, SessionKind, SessionStatus, User, Workspace,
};
use otto_server::feature_guard::feature_guard;
use otto_server::routes::admin_sessions::{list_sessions, terminate, AdminSessionsCtx};
use otto_sessions::{ProviderRegistry, SessionManager};
use otto_state::{GrantsRepo, NewAuditEntry, NewSession, SessionsRepo, SqlitePool, UsersRepo};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use tokio::sync::broadcast;
use tower::ServiceExt; // for `oneshot`

// ---------------------------------------------------------------------------
// Minimal test state implementing both HasGrants and AdminSessionsCtx.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct TestCtx {
    pool: SqlitePool,
    manager: Arc<SessionManager>,
}

impl otto_server::feature_guard::HasGrants for TestCtx {
    fn grants(&self) -> GrantsRepo {
        GrantsRepo::new(self.pool.clone())
    }
}

impl AdminSessionsCtx for TestCtx {
    fn sessions_repo(&self) -> SessionsRepo {
        SessionsRepo::new(self.pool.clone())
    }
    fn users_repo(&self) -> UsersRepo {
        UsersRepo::new(self.pool.clone())
    }
    fn manager(&self) -> Arc<SessionManager> {
        self.manager.clone()
    }
    async fn audit_entry(&self, entry: NewAuditEntry) {
        let action = entry.action.clone();
        if let Err(e) = otto_state::AuditRepo::new(self.pool.clone()).insert(entry).await {
            tracing::warn!(%action, "test: audit insert failed: {e}");
        }
    }
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

async fn mk_pool() -> SqlitePool {
    let opts = SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        // A bare in-memory SQLite gives each *connection* its own private DB, so
        // the pool must be pinned to ONE connection for the schema (and the rows
        // the SessionManager + repos share) to be visible across calls. All our
        // handler work is sequential per request, so a single connection is fine
        // (mirrors the rbac_matrix / grants_api harness).
        .max_connections(1)
        .connect_with(opts)
        .await
        .expect("connect sqlite");
    sqlx::migrate!("../otto-state/migrations")
        .run(&pool)
        .await
        .expect("migrations");
    pool
}

async fn seed_user(pool: &SqlitePool, username: &str, is_root: bool) -> User {
    let id = otto_core::new_id();
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO users (id, username, password_hash, display_name, is_root, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(username)
    .bind("hash")
    .bind(username)
    .bind(is_root as i64)
    .bind(&now)
    .execute(pool)
    .await
    .expect("seed user");
    User {
        id,
        username: username.to_string(),
        display_name: username.to_string(),
        is_root,
        disabled: false,
        created_at: Utc::now(),
    }
}

async fn seed_workspace(pool: &SqlitePool) -> Workspace {
    let id = otto_core::new_id();
    let now = Utc::now().to_rfc3339();
    sqlx::query("INSERT INTO workspaces (id, name, root_path, created_at) VALUES (?, ?, ?, ?)")
        .bind(&id)
        .bind("ws")
        .bind("/tmp")
        .bind(&now)
        .execute(pool)
        .await
        .expect("seed workspace");
    Workspace {
        id,
        name: "ws".into(),
        root_path: "/tmp".into(),
        settings: serde_json::json!({}),
        archived: false,
        created_at: Utc::now(),
    }
}

/// Insert a session row owned by `owner` (no live PTY — we exercise the DB +
/// signal paths, which is all the overview/terminate need without a real PTY).
async fn seed_session(repo: &SessionsRepo, ws: &Workspace, owner: &User, title: &str) -> String {
    let s = repo
        .create(NewSession {
            workspace_id: ws.id.clone(),
            kind: SessionKind::Agent,
            provider: "claude".into(),
            title: title.into(),
            cwd: "/tmp".into(),
            provider_session_id: None,
            connection_id: None,
            created_by: owner.id.clone(),
            meta: serde_json::json!({}),
        })
        .await
        .expect("create session");
    s.id
}

fn make_manager(pool: SqlitePool) -> Arc<SessionManager> {
    let (events, _rx) = broadcast::channel(16);
    Arc::new(SessionManager::new(
        SessionsRepo::new(pool),
        events,
        ProviderRegistry::new(None),
    ))
}

/// Build a minimal router with the real admin-session handlers + the feature
/// guard layered, with a synthetic `AuthUser` injected for `actor`.
fn build_app(ctx: TestCtx, actor: User) -> Router {
    let protected = Router::new()
        .route("/admin/sessions", get(list_sessions::<TestCtx>))
        .route(
            "/admin/sessions/{id}/terminate",
            post(terminate::<TestCtx>),
        );

    let protected = protected.route_layer(from_fn_with_state(
        ctx.clone(),
        feature_guard::<TestCtx>,
    ));

    let injected = Arc::new(actor);
    let protected = protected.layer(from_fn(move |mut req: Request, next: Next| {
        let u = injected.clone();
        async move {
            req.extensions_mut().insert(AuthUser((*u).clone()));
            next.run(req).await
        }
    }));

    Router::new().nest("/api/v1", protected).with_state(ctx)
}

async fn do_req(
    app: &Router,
    method: Method,
    path: &str,
) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method(method)
        .uri(path)
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

// ---------------------------------------------------------------------------
// A non-root Users:Admin sees sessions owned by OTHER users (cross-user view).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn users_admin_sees_other_users_sessions() {
    let pool = mk_pool().await;
    let admin = seed_user(&pool, "admin", false).await; // NON-root admin
    let alice = seed_user(&pool, "alice", false).await;
    let ws = seed_workspace(&pool).await;

    // Grant admin Users:Admin (the overview's gate) — NOT root.
    GrantsRepo::new(pool.clone())
        .set_grants(&admin.id, &[(Feature::Users, Capability::Admin)])
        .await
        .unwrap();

    let repo = SessionsRepo::new(pool.clone());
    let alice_sid = seed_session(&repo, &ws, &alice, "alice-1").await;

    let manager = make_manager(pool.clone());
    let ctx = TestCtx { pool, manager };
    let app = build_app(ctx, admin);

    let (st, body) = do_req(&app, Method::GET, "/api/v1/admin/sessions").await;
    assert_eq!(st, StatusCode::OK, "Users:Admin must read the overview: {body}");

    let sessions = body["sessions"].as_array().expect("sessions array");
    let row = sessions
        .iter()
        .find(|r| r["id"].as_str() == Some(alice_sid.as_str()))
        .expect("admin must see alice's session (cross-user view)");
    assert_eq!(row["owner_username"], "alice", "owner resolved to username");
    assert_eq!(row["owner_id"], alice.id, "owner id is alice");
    assert_eq!(row["live"], false, "no live PTY in this test");
    assert_eq!(row["viewers"], 0);
    assert_eq!(row["status"], "running");
}

// ---------------------------------------------------------------------------
// Root sees all sessions too (no grant rows needed).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn root_sees_all_sessions() {
    let pool = mk_pool().await;
    let root = seed_user(&pool, "root", true).await;
    let alice = seed_user(&pool, "alice", false).await;
    let bob = seed_user(&pool, "bob", false).await;
    let ws = seed_workspace(&pool).await;

    let repo = SessionsRepo::new(pool.clone());
    let a = seed_session(&repo, &ws, &alice, "alice-1").await;
    let b = seed_session(&repo, &ws, &bob, "bob-1").await;

    let manager = make_manager(pool.clone());
    let ctx = TestCtx { pool, manager };
    let app = build_app(ctx, root);

    let (st, body) = do_req(&app, Method::GET, "/api/v1/admin/sessions").await;
    assert_eq!(st, StatusCode::OK, "root must read the overview: {body}");

    let ids: Vec<&str> = body["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&a.as_str()), "root sees alice's session");
    assert!(ids.contains(&b.as_str()), "root sees bob's session");
}

// ---------------------------------------------------------------------------
// A user WITHOUT Users:Admin is denied (403) — no cross-user leak.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn non_admin_cannot_list_sessions() {
    let pool = mk_pool().await;
    let alice = seed_user(&pool, "alice", false).await;
    let ws = seed_workspace(&pool).await;

    // alice has Agents:Edit only — no Users grant.
    GrantsRepo::new(pool.clone())
        .set_grants(&alice.id, &[(Feature::Agents, Capability::Edit)])
        .await
        .unwrap();
    let repo = SessionsRepo::new(pool.clone());
    seed_session(&repo, &ws, &alice, "alice-1").await;

    let manager = make_manager(pool.clone());
    let ctx = TestCtx { pool, manager };
    let app = build_app(ctx, alice);

    let (st, body) = do_req(&app, Method::GET, "/api/v1/admin/sessions").await;
    assert_eq!(
        st,
        StatusCode::FORBIDDEN,
        "a user without Users:Admin must be denied the overview: {body}"
    );
    assert_eq!(body["code"], "forbidden");
}

#[tokio::test]
async fn non_admin_cannot_terminate() {
    let pool = mk_pool().await;
    let alice = seed_user(&pool, "alice", false).await;
    let ws = seed_workspace(&pool).await;
    let repo = SessionsRepo::new(pool.clone());
    let sid = seed_session(&repo, &ws, &alice, "alice-1").await;

    let manager = make_manager(pool.clone());
    let ctx = TestCtx { pool, manager };
    let app = build_app(ctx, alice); // no grants at all

    let (st, body) = do_req(
        &app,
        Method::POST,
        &format!("/api/v1/admin/sessions/{sid}/terminate"),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN, "no Users:Admin → 403: {body}");
}

// ---------------------------------------------------------------------------
// terminate: kills the session (→ exited) AND fires the forced-eviction signal.
// The eviction assertion is the integration coverage deferred from Task 4.1:
// we subscribe an `evict_signal` receiver before terminate and assert it fires.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn terminate_kills_session_and_evicts_viewers() {
    let pool = mk_pool().await;
    let admin = seed_user(&pool, "admin", false).await; // non-root Users:Admin
    let alice = seed_user(&pool, "alice", false).await;
    let ws = seed_workspace(&pool).await;

    GrantsRepo::new(pool.clone())
        .set_grants(&admin.id, &[(Feature::Users, Capability::Admin)])
        .await
        .unwrap();

    let repo = SessionsRepo::new(pool.clone());
    let sid = seed_session(&repo, &ws, &alice, "alice-1").await;

    let manager = make_manager(pool.clone());

    // Simulate an attached /ws/term viewer: subscribe to the forced-disconnect
    // signal BEFORE terminate (exactly what the WS loop does on attach).
    let mut evict_rx = manager.evict_signal(&sid);
    assert!(evict_rx.try_recv().is_err(), "no eviction fired yet");

    let ctx = TestCtx {
        pool: pool.clone(),
        manager: manager.clone(),
    };
    let app = build_app(ctx, admin.clone());

    let (st, _body) = do_req(
        &app,
        Method::POST,
        &format!("/api/v1/admin/sessions/{sid}/terminate"),
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT, "terminate returns 204");

    // The forced-eviction signal fired → the attached viewer is dropped.
    assert!(
        evict_rx.recv().await.is_ok(),
        "terminate must fire the eviction signal so attached /ws/term viewers close"
    );

    // The session is now exited (kill_session marks it; the row is kept).
    let after = repo.get(&sid).await.expect("session row kept after terminate");
    assert_eq!(
        after.status,
        SessionStatus::Exited,
        "terminate marks the session exited"
    );

    // An audit entry recorded the actor + owner.
    use otto_core::api::AuditLogQuery;
    let audit = otto_state::AuditRepo::new(pool.clone());
    let entries = audit
        .list(&AuditLogQuery {
            action: Some("session.terminated".into()),
            user_id: None,
            from: None,
            to: None,
            limit: Some(10),
            offset: None,
        })
        .await
        .expect("list audit");
    assert!(!entries.is_empty(), "session.terminated audit entry written");
    let e = &entries[0];
    assert_eq!(e.user_id.as_deref(), Some(admin.id.as_str()), "actor = admin");
    assert_eq!(e.target.as_deref(), Some(sid.as_str()), "target = session id");
    let detail = e.detail.as_ref().expect("audit detail");
    assert_eq!(
        detail.get("owner_id").and_then(|v| v.as_str()),
        Some(alice.id.as_str()),
        "audit records the session owner"
    );
}
