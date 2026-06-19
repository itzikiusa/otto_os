//! Deny-by-default **scope guard** matrix (mobile plan Tasks 1.4 + 1.5) — the
//! security boundary that pins a scoped *share-link* token to exactly one
//! session and rejects everything else.
//!
//! These exercise the *real* guard code path end-to-end: the same
//! [`otto_server::feature_guard::feature_guard`] middleware production layers
//! after auth, fed an [`otto_core::auth::AuthContext`] produced by the **real**
//! [`otto_rbac::AuthRepo::authenticate`] over an in-memory SQLite DB. So the test
//! proves both halves of the fix together:
//!
//! 1. **Part 1 (is_root drop):** the share token is minted by a *root* owner, yet
//!    `authenticate()` forces `is_root=false` on the principal — so the 403s below
//!    hold *even though* S1's owner is root (a leaked share can never carry root).
//! 2. **Part 2 (deny-by-default):** with a scope present, the feature policy is
//!    skipped and only `GET /sessions/{id}` (== scope) and `POST /sessions/{id}/input`
//!    (== scope, Editor only) pass; everything else is `403`.
//!
//! ## Harness note
//! Like `rbac_matrix.rs`, building a full `ServerCtx` for `build_router` is
//! infeasible (≈30 service handles, no test constructor). We instead mount stub
//! handlers at the **exact** `/api/v1` route templates the matrix targets and
//! layer the **same** guard middleware over a minimal `HasGrants` state, then
//! inject the request's [`AuthContext`] exactly as `auth_middleware` does (both
//! the `AuthUser` *and* the full `AuthContext` extension). The guard logic under
//! test is identical; only the surrounding service graph is stubbed.

use axum::body::Body;
use axum::extract::Request;
use axum::http::{Method, StatusCode};
use axum::middleware::{from_fn, from_fn_with_state, Next};
use axum::routing::{get, post};
use axum::Router;
use chrono::Utc;
use otto_core::auth::{AuthContext, AuthUser};
use otto_core::domain::WorkspaceRole;
use otto_core::{new_id, Id};
use otto_rbac::AuthRepo;
use otto_server::feature_guard::feature_guard;
use otto_state::{GrantsRepo, SqlitePool};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::sync::Arc;
use tower::ServiceExt; // for `oneshot`

// ---------------------------------------------------------------------------
// Minimal app state: just enough for the guard (a GrantsRepo).
// ---------------------------------------------------------------------------

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

/// Seed a user row (with an explicit `is_root`) and return its id.
async fn seed_user(pool: &SqlitePool, username: &str, is_root: bool) -> Id {
    let id = new_id();
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
    id
}

/// Build the minimal router: stub handlers at the real `/api/v1` templates the
/// matrix targets, the `feature_guard` as a `route_layer`, and an injection layer
/// that drops the request's already-resolved [`AuthContext`] into extensions the
/// way `auth_middleware` does (both `AuthUser` = effective user and the full
/// `AuthContext`). The whole thing is nested under `/api/v1` so `MatchedPath`
/// carries the prefix the guard reads.
fn app(pool: SqlitePool, ctx: AuthContext) -> Router {
    let state = TestState {
        grants: GrantsRepo::new(pool),
    };

    async fn ok() -> &'static str {
        "ok"
    }

    let protected = Router::new()
        // The two allow-listed session routes.
        .route("/sessions/{id}", get(ok).patch(ok).delete(ok))
        .route("/sessions/{id}/input", post(ok))
        // Session-control writes (must be denied for scoped tokens).
        .route("/sessions/{id}/restart", post(ok))
        // Enumeration (must be denied).
        .route("/workspaces/{id}/sessions", get(ok).post(ok))
        // A few representative non-session surfaces (all denied).
        .route("/usage/summary", get(ok))
        .route("/users", get(ok).post(ok))
        .route("/connections", get(ok).post(ok));

    let protected = protected.route_layer(from_fn_with_state(
        state.clone(),
        feature_guard::<TestState>,
    ));

    // Inject the resolved AuthContext (mirrors `auth_middleware`): both the
    // effective `AuthUser` and the full `AuthContext` so the guard sees the scope.
    let ctx = Arc::new(ctx);
    let protected = protected.layer(from_fn(move |mut req: Request, next: Next| {
        let ctx = ctx.clone();
        async move {
            req.extensions_mut()
                .insert(AuthUser(ctx.effective_user.clone()));
            req.extensions_mut().insert((*ctx).clone());
            next.run(req).await
        }
    }));

    Router::new().nest("/api/v1", protected).with_state(state)
}

async fn status(app: &Router, method: Method, path: &str) -> StatusCode {
    let req = Request::builder()
        .method(method)
        .uri(path)
        .body(Body::empty())
        .unwrap();
    app.clone().oneshot(req).await.unwrap().status()
}

/// Mint a share token for `session_id` owned by a **root** user, authenticate it
/// (the real path that drops `is_root` and builds the scope), and return the
/// resolved `AuthContext` (the same one `auth_middleware` would insert). Asserts
/// the principal is non-root and carries the expected scope.
async fn root_owned_share_ctx(
    pool: &SqlitePool,
    session_id: &str,
    role: WorkspaceRole,
) -> AuthContext {
    let repo = AuthRepo::new(pool.clone());
    let owner = seed_user(pool, &format!("root-{session_id}-{}", role.as_str()), true).await;
    let (raw, _info) = repo
        .issue_share_token(&owner, &Id::from(session_id), role, 3600, None)
        .await
        .expect("mint share");
    let ctx = repo.authenticate(&raw).await.expect("authenticate share");
    // Part 1: a root-owned share must NOT carry root.
    assert!(!ctx.real_user.is_root, "share real_user must be non-root");
    assert!(
        !ctx.effective_user.is_root,
        "share effective_user must be non-root"
    );
    let scope = ctx.scope.clone().expect("share token must carry a scope");
    assert_eq!(scope.session_id, Id::from(session_id));
    assert_eq!(scope.role, role);
    ctx
}

// ---------------------------------------------------------------------------
// Viewer-share matrix
// ---------------------------------------------------------------------------

#[tokio::test]
async fn viewer_share_can_get_its_session() {
    let pool = mem_pool().await;
    let ctx = root_owned_share_ctx(&pool, "S1", WorkspaceRole::Viewer).await;
    let app = app(pool, ctx);
    assert_eq!(
        status(&app, Method::GET, "/api/v1/sessions/S1").await,
        StatusCode::OK,
        "a viewer share may GET its own pinned session"
    );
}

#[tokio::test]
async fn viewer_share_cannot_input() {
    let pool = mem_pool().await;
    let ctx = root_owned_share_ctx(&pool, "S1", WorkspaceRole::Viewer).await;
    let app = app(pool, ctx);
    assert_eq!(
        status(&app, Method::POST, "/api/v1/sessions/S1/input").await,
        StatusCode::FORBIDDEN,
        "a viewer share may NOT send input"
    );
}

#[tokio::test]
async fn viewer_share_cannot_touch_other_session() {
    let pool = mem_pool().await;
    let ctx = root_owned_share_ctx(&pool, "S1", WorkspaceRole::Viewer).await;
    let app = app(pool, ctx);
    assert_eq!(
        status(&app, Method::GET, "/api/v1/sessions/S2").await,
        StatusCode::FORBIDDEN,
        "a share may never reach another session id"
    );
}

#[tokio::test]
async fn viewer_share_cannot_enumerate_sessions() {
    let pool = mem_pool().await;
    let ctx = root_owned_share_ctx(&pool, "S1", WorkspaceRole::Viewer).await;
    let app = app(pool, ctx);
    assert_eq!(
        status(&app, Method::GET, "/api/v1/workspaces/W1/sessions").await,
        StatusCode::FORBIDDEN,
        "session enumeration is denied for a scoped token"
    );
}

#[tokio::test]
async fn viewer_share_cannot_reach_non_session_routes() {
    let pool = mem_pool().await;
    let ctx = root_owned_share_ctx(&pool, "S1", WorkspaceRole::Viewer).await;
    let app = app(pool, ctx);
    for (m, p) in [
        (Method::GET, "/api/v1/usage/summary"),
        (Method::GET, "/api/v1/users"),
        (Method::GET, "/api/v1/connections"),
        (Method::POST, "/api/v1/sessions/S1/restart"),
    ] {
        assert_eq!(
            status(&app, m.clone(), p).await,
            StatusCode::FORBIDDEN,
            "a scoped token must be denied {m} {p}"
        );
    }
}

// ---------------------------------------------------------------------------
// Editor-share matrix
// ---------------------------------------------------------------------------

#[tokio::test]
async fn editor_share_can_input_its_session() {
    let pool = mem_pool().await;
    let ctx = root_owned_share_ctx(&pool, "S1", WorkspaceRole::Editor).await;
    let app = app(pool, ctx);
    assert_eq!(
        status(&app, Method::POST, "/api/v1/sessions/S1/input").await,
        StatusCode::OK,
        "an editor share may send input to its pinned session"
    );
    // And it can still GET its session.
    assert_eq!(
        status(&app, Method::GET, "/api/v1/sessions/S1").await,
        StatusCode::OK,
    );
}

#[tokio::test]
async fn editor_share_still_bounded_to_one_session() {
    let pool = mem_pool().await;
    let ctx = root_owned_share_ctx(&pool, "S1", WorkspaceRole::Editor).await;
    let app = app(pool, ctx);
    // Other session, enumeration, restart, and non-session routes still denied.
    for (m, p) in [
        (Method::GET, "/api/v1/sessions/S2"),
        (Method::POST, "/api/v1/sessions/S2/input"),
        (Method::GET, "/api/v1/workspaces/W1/sessions"),
        (Method::POST, "/api/v1/sessions/S1/restart"),
        (Method::DELETE, "/api/v1/sessions/S1"),
        (Method::GET, "/api/v1/usage/summary"),
        (Method::GET, "/api/v1/users"),
    ] {
        assert_eq!(
            status(&app, m.clone(), p).await,
            StatusCode::FORBIDDEN,
            "an editor share must still be denied {m} {p}"
        );
    }
}

// ---------------------------------------------------------------------------
// Non-scoped tokens are completely unaffected by the scope branch.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn normal_root_token_unaffected() {
    // A normal (unscoped) root token still passes the feature axis on a normal
    // route — the scope branch is a no-op when `scope == None`.
    let pool = mem_pool().await;
    let owner = seed_user(&pool, "rootuser", true).await;
    let repo = AuthRepo::new(pool.clone());
    let raw = repo.issue(&owner).await.unwrap();
    let ctx = repo.authenticate(&raw).await.unwrap();
    assert!(ctx.scope.is_none(), "a normal token is unscoped");
    assert!(ctx.effective_user.is_root, "a normal root token keeps root");

    let app = app(pool, ctx);
    // Root passes the feature guard on a normal Agents route…
    assert_eq!(
        status(&app, Method::GET, "/api/v1/sessions/S1").await,
        StatusCode::OK,
    );
    // …and on a normally-denied-to-shares route (enumeration / users), proving
    // the feature path (not the scope path) runs for unscoped tokens.
    assert_eq!(
        status(&app, Method::GET, "/api/v1/workspaces/W1/sessions").await,
        StatusCode::OK,
    );
    assert_eq!(status(&app, Method::GET, "/api/v1/users").await, StatusCode::OK);
}
