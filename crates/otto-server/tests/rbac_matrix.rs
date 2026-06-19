//! RBAC denial-matrix integration tests for the central feature-policy guard
//! (Task 1.4) — the core security tests of Phase 1.
//!
//! These exercise the *real* guard code path end-to-end: the Axum
//! [`axum::extract::MatchedPath`] read, [`otto_server::policy::policy_for`], and
//! [`otto_state::GrantsRepo::capability_of`] over a real (in-memory) SQLite DB —
//! producing a `403` JSON `Problem` on denial or passing the request through to
//! a stub handler (`200`) on allow.
//!
//! ## Harness note (adaptation from the plan)
//! The plan suggested mirroring `auth_security.rs` and spinning up the full
//! `build_router`. `build_router` requires a fully-assembled `ServerCtx` (~30
//! `Arc` service handles: `SessionManager`, `Orchestrator`, `SwarmService`,
//! `UsageEngine`, a `Spawner`, a `SecretStore`, …) for which no test constructor
//! exists — building one here would be enormous and brittle, and would couple
//! the guard's security test to unrelated subsystems.
//!
//! Instead we build a *minimal* router that layers the **same** guard middleware
//! (`otto_server::feature_guard::feature_guard`, which is generic over any state
//! that `impl`s `HasGrants`) over a tiny [`TestState`] holding a real
//! `GrantsRepo`, plus a layer that injects the authenticated `AuthUser` — exactly
//! how `auth_middleware` does in production. Stub handlers are registered at the
//! *exact* route templates the plan's matrix targets, so `MatchedPath` resolves
//! to the real `/api/v1/...` template and `policy_for` sees what it sees in
//! production. The guard logic under test is identical; only the surrounding
//! service graph is stubbed. (Task 1.5's coverage test backstops the policy
//! table against the live route set.)

use axum::body::Body;
use axum::extract::Request;
use axum::http::{Method, StatusCode};
use axum::middleware::{from_fn, from_fn_with_state, Next};
use axum::routing::{get, post, put};
use axum::Router;
use chrono::Utc;
use http_body_util::BodyExt;
use otto_core::auth::AuthUser;
use otto_core::domain::{Capability, Feature, User};
use otto_server::feature_guard::feature_guard;
use otto_state::{GrantsRepo, SqlitePool};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::sync::Arc;
use tower::ServiceExt; // for `oneshot`

// ---------------------------------------------------------------------------
// Minimal app state: just enough for the guard (a GrantsRepo).
// ---------------------------------------------------------------------------

/// Test-only state implementing `HasGrants`, mirroring how `ServerCtx` exposes a
/// `GrantsRepo` to the guard in production.
#[derive(Clone)]
struct TestState {
    grants: GrantsRepo,
}

impl otto_server::feature_guard::HasGrants for TestState {
    fn grants(&self) -> GrantsRepo {
        self.grants.clone()
    }
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// In-memory SQLite pool with the full otto-state schema applied.
async fn mem_pool() -> SqlitePool {
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
    pool
}

/// Seed a user row and return the `User` (mirrors `grants.rs` test helper).
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

/// Build a minimal router mounting stub handlers at the **real** `/api/v1`
/// templates the matrix targets, with the feature guard (after an `AuthUser`
/// injection layer) applied exactly as `build_router` layers it: the guard is a
/// `route_layer` immediately after auth, and the whole thing is nested under
/// `/api/v1` so `MatchedPath` carries the prefix the policy table expects.
fn app(pool: SqlitePool, user: User) -> Router {
    let state = TestState {
        grants: GrantsRepo::new(pool),
    };

    // Stub handlers — every one returns 200 so a *pass* through the guard is
    // observable; the guard is the only thing that can turn these into 403.
    async fn ok() -> &'static str {
        "ok"
    }

    let protected = Router::new()
        // Database feature
        .route("/connections/{id}/db/tables", get(ok))
        .route("/connections/{id}/db/query", post(ok))
        // Connections feature
        .route("/connections", post(ok))
        .route("/connections/{id}/open", post(ok))
        // Agents feature
        .route("/workspaces/{id}/sessions", get(ok))
        // Swarm feature
        .route("/workspaces/{id}/swarm/swarms", get(ok))
        // Users / Settings (Admin)
        .route("/users", post(ok))
        .route("/settings", put(ok))
        // An intentionally-unmapped protected route (fail-closed → Deny).
        .route("/foo", get(ok));

    // Guard layer: same generic middleware production uses, parameterized on the
    // test state. Layered as a `route_layer` so it only runs on matched routes
    // (and so `MatchedPath` is present).
    let protected = protected.route_layer(from_fn_with_state(
        state.clone(),
        feature_guard::<TestState>,
    ));

    // Inject the authenticated user the way `auth_middleware` does (an
    // `AuthUser` request extension), *before* the guard runs.
    let injected_user = Arc::new(user);
    let protected = protected.layer(from_fn(move |mut req: Request, next: Next| {
        let u = injected_user.clone();
        async move {
            req.extensions_mut().insert(AuthUser((*u).clone()));
            next.run(req).await
        }
    }));

    Router::new().nest("/api/v1", protected).with_state(state)
}

/// Issue a request and return its status code.
async fn status(app: &Router, method: Method, path: &str) -> StatusCode {
    let req = Request::builder()
        .method(method)
        .uri(path)
        .body(Body::empty())
        .unwrap();
    app.clone().oneshot(req).await.unwrap().status()
}

/// Issue a request and return (status, parsed JSON body).
async fn status_and_body(
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
    let st = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (st, json)
}

/// Helper: build the app for a user with exactly the given grants.
async fn app_for(grants: &[(Feature, Capability)], is_root: bool) -> Router {
    let pool = mem_pool().await;
    let u = seed_user(&pool, "alice", is_root).await;
    if !grants.is_empty() {
        GrantsRepo::new(pool.clone())
            .set_grants(&u.id, grants)
            .await
            .unwrap();
    }
    app(pool, u)
}

// ---------------------------------------------------------------------------
// The denial matrix — a Database:View-only user is provably confined.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn db_view_user_can_read_db() {
    let app = app_for(&[(Feature::Database, Capability::View)], false).await;
    assert_eq!(
        status(&app, Method::GET, "/api/v1/connections/c1/db/tables").await,
        StatusCode::OK,
        "Database:View must allow a DB read"
    );
}

#[tokio::test]
async fn db_view_user_cannot_write_db() {
    let app = app_for(&[(Feature::Database, Capability::View)], false).await;
    let (st, body) = status_and_body(&app, Method::POST, "/api/v1/connections/c1/db/query").await;
    assert_eq!(st, StatusCode::FORBIDDEN, "DB write needs Database:Edit");
    // Denial must be a JSON `Problem` with a `forbidden` code.
    assert_eq!(body["code"], "forbidden", "body: {body}");
    assert!(body["message"].is_string());
}

#[tokio::test]
async fn db_view_user_cannot_manage_conns() {
    let app = app_for(&[(Feature::Database, Capability::View)], false).await;
    assert_eq!(
        status(&app, Method::POST, "/api/v1/connections").await,
        StatusCode::FORBIDDEN,
        "connection management is Connections:Admin"
    );
}

#[tokio::test]
async fn db_view_user_cannot_touch_agents() {
    let app = app_for(&[(Feature::Database, Capability::View)], false).await;
    assert_eq!(
        status(&app, Method::GET, "/api/v1/workspaces/w1/sessions").await,
        StatusCode::FORBIDDEN,
        "listing sessions needs Agents:View"
    );
}

#[tokio::test]
async fn db_view_user_cannot_touch_swarm() {
    let app = app_for(&[(Feature::Database, Capability::View)], false).await;
    assert_eq!(
        status(&app, Method::GET, "/api/v1/workspaces/w1/swarm/swarms").await,
        StatusCode::FORBIDDEN,
        "swarm needs Swarm:View"
    );
}

#[tokio::test]
async fn db_view_user_cannot_admin() {
    let app = app_for(&[(Feature::Database, Capability::View)], false).await;
    assert_eq!(
        status(&app, Method::POST, "/api/v1/users").await,
        StatusCode::FORBIDDEN,
        "user CRUD is Users:Admin"
    );
    assert_eq!(
        status(&app, Method::PUT, "/api/v1/settings").await,
        StatusCode::FORBIDDEN,
        "settings is Settings:Admin"
    );
}

#[tokio::test]
async fn connections_edit_user_can_open_but_not_manage_globals() {
    let app = app_for(&[(Feature::Connections, Capability::Edit)], false).await;
    // Open a connection = Connections:Edit → allowed.
    assert_eq!(
        status(&app, Method::POST, "/api/v1/connections/c1/open").await,
        StatusCode::OK,
        "Connections:Edit must allow opening a connection"
    );
    // Create a global connection = Connections:Admin → denied.
    assert_eq!(
        status(&app, Method::POST, "/api/v1/connections").await,
        StatusCode::FORBIDDEN,
        "global connection management requires Connections:Admin"
    );
}

#[tokio::test]
async fn root_passes_everything() {
    let app = app_for(&[], true).await;
    for (m, p) in [
        (Method::GET, "/api/v1/connections/c1/db/tables"),
        (Method::POST, "/api/v1/connections/c1/db/query"),
        (Method::POST, "/api/v1/connections"),
        (Method::POST, "/api/v1/connections/c1/open"),
        (Method::GET, "/api/v1/workspaces/w1/sessions"),
        (Method::GET, "/api/v1/workspaces/w1/swarm/swarms"),
        (Method::POST, "/api/v1/users"),
        (Method::PUT, "/api/v1/settings"),
    ] {
        let st = status(&app, m.clone(), p).await;
        assert_ne!(st, StatusCode::FORBIDDEN, "root must pass {m} {p}");
        assert_eq!(st, StatusCode::OK, "root reaches the stub for {m} {p}");
    }
}

#[tokio::test]
async fn unknown_protected_route_403() {
    // Even with a broad grant, an unmapped protected route fails closed (Deny).
    let app = app_for(&[(Feature::Database, Capability::Admin)], false).await;
    assert_eq!(
        status(&app, Method::GET, "/api/v1/foo").await,
        StatusCode::FORBIDDEN,
        "a protected route with no policy entry must fail closed (403)"
    );
}
