//! Integration tests for the share-link API (mobile plan Task 1.9):
//!   POST   /api/v1/sessions/{id}/share
//!   GET    /api/v1/sessions/{id}/shares
//!   DELETE /api/v1/auth/shares/{share_id}
//!   POST   /api/v1/auth/shares/revoke-all
//!
//! Uses the same minimal-router harness as `grants_api.rs` / `share_scope_guard.rs`:
//! a lightweight test state (pool + SessionManager), the feature-guard middleware
//! layered, and a synthetic `AuthContext` + `AuthUser` injection layer.
//!
//! The full `ServerCtx` is NOT assembled here — that would couple this test
//! to ~30 unrelated subsystems.

use axum::body::Body;
use axum::extract::Request;
use axum::http::{Method, StatusCode};
use axum::middleware::{from_fn, from_fn_with_state, Next};
use axum::routing::{get, post};
use axum::Router;
use chrono::Utc;
use otto_core::auth::{AuthContext, AuthUser};
use otto_core::domain::{User, WorkspaceRole};
use otto_core::{new_id, Id};
use otto_rbac::AuthRepo;
use otto_server::feature_guard::feature_guard;
use otto_state::{GrantsRepo, SqlitePool};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::sync::Arc;
use tower::ServiceExt; // for `oneshot`

// ---------------------------------------------------------------------------
// Pool + user fixtures
// ---------------------------------------------------------------------------

async fn mk_pool() -> SqlitePool {
    let opts = SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
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
    User {
        id,
        username: username.to_string(),
        display_name: username.to_string(),
        is_root,
        disabled: false,
        created_at: Utc::now(),
    }
}

// ---------------------------------------------------------------------------
// Policy-level tests (no server required)
// ---------------------------------------------------------------------------

#[test]
fn share_routes_have_correct_policy() {
    use axum::http::Method;
    use otto_server::policy::{policy_for, PolicyDecision};
    use otto_core::domain::{Capability, Feature};

    // POST /sessions/{id}/share → Agents:Edit
    assert_eq!(
        policy_for(&Method::POST, "/api/v1/sessions/{id}/share"),
        PolicyDecision::Require(Feature::Agents, Capability::Edit),
        "POST /sessions/{{id}}/share must be Agents:Edit"
    );

    // GET /sessions/{id}/shares → Agents:View
    assert_eq!(
        policy_for(&Method::GET, "/api/v1/sessions/{id}/shares"),
        PolicyDecision::Require(Feature::Agents, Capability::View),
        "GET /sessions/{{id}}/shares must be Agents:View"
    );
    // POST /sessions/{id}/shares (if someone typos) → Agents:Edit (not exempt)
    assert_eq!(
        policy_for(&Method::POST, "/api/v1/sessions/{id}/shares"),
        PolicyDecision::Require(Feature::Agents, Capability::Edit),
    );

    // /auth/shares/* → Exempt (self-owned)
    assert_eq!(
        policy_for(&Method::DELETE, "/api/v1/auth/shares/{share_id}"),
        PolicyDecision::Exempt,
        "DELETE /auth/shares/{{id}} must be Exempt (self-owned)"
    );
    assert_eq!(
        policy_for(&Method::POST, "/api/v1/auth/shares/revoke-all"),
        PolicyDecision::Exempt,
        "POST /auth/shares/revoke-all must be Exempt (self-owned)"
    );
}

// ---------------------------------------------------------------------------
// AuthRepo-level behavioral tests (security-critical, TDD path)
// ---------------------------------------------------------------------------

/// Owner mints a viewer share → token authenticates with the correct scope.
#[tokio::test]
async fn owner_mints_viewer_share_and_token_authenticates() {
    let pool = mk_pool().await;
    let repo = AuthRepo::new(pool.clone());
    let owner = seed_user(&pool, "owner", false).await;

    let (raw, info) = repo
        .issue_share_token(&owner.id, &Id::from("S1"), WorkspaceRole::Viewer, 3600, None)
        .await
        .expect("mint share");

    // The minted token authenticates and carries the correct scope.
    let ctx = repo.authenticate(&raw).await.expect("authenticate share");
    let scope = ctx.scope.expect("share token must carry scope");
    assert_eq!(scope.session_id, Id::from("S1"));
    assert_eq!(scope.role, WorkspaceRole::Viewer);
    assert_eq!(info.token_prefix.len(), 12);

    // URL pattern: info carries the session_id for building /#/s/<id>/<token>
    let url = format!("/#/s/{}/{}", info.session_id, raw);
    assert!(url.contains("/#/s/S1/"), "url must contain /#/s/<session_id>/");
}

/// Editor share carries Editor scope.
#[tokio::test]
async fn owner_mints_editor_share_carries_editor_scope() {
    let pool = mk_pool().await;
    let repo = AuthRepo::new(pool.clone());
    let owner = seed_user(&pool, "owner", false).await;

    let (raw, _info) = repo
        .issue_share_token(&owner.id, &Id::from("S1"), WorkspaceRole::Editor, 3600, None)
        .await
        .expect("mint editor share");

    let ctx = repo.authenticate(&raw).await.expect("authenticate");
    let scope = ctx.scope.expect("must carry scope");
    assert_eq!(scope.role, WorkspaceRole::Editor);
}

/// Admin role is rejected.
#[tokio::test]
async fn mint_admin_share_is_rejected() {
    let pool = mk_pool().await;
    let repo = AuthRepo::new(pool.clone());
    let owner = seed_user(&pool, "owner", false).await;

    let err = repo
        .issue_share_token(&owner.id, &Id::from("S1"), WorkspaceRole::Admin, 3600, None)
        .await;
    assert!(err.is_err(), "admin-role share must be rejected");
}

/// Revoke makes subsequent authenticate fail.
#[tokio::test]
async fn revoked_share_token_cannot_authenticate() {
    let pool = mk_pool().await;
    let repo = AuthRepo::new(pool.clone());
    let owner = seed_user(&pool, "owner", false).await;

    let (raw, info) = repo
        .issue_share_token(&owner.id, &Id::from("S1"), WorkspaceRole::Viewer, 3600, None)
        .await
        .expect("mint");

    assert!(repo.authenticate(&raw).await.is_ok(), "before revoke");

    repo.revoke_share(&owner.id, &info.id).await.expect("revoke");

    assert!(
        repo.authenticate(&raw).await.is_err(),
        "after revoke: authenticate must fail"
    );
}

/// revoke_all_shares_for_user revokes all and returns session ids.
#[tokio::test]
async fn revoke_all_shares_clears_all_for_user() {
    let pool = mk_pool().await;
    let repo = AuthRepo::new(pool.clone());
    let owner = seed_user(&pool, "owner", false).await;
    let other = seed_user(&pool, "other", false).await;

    let (raw1, _) = repo
        .issue_share_token(&owner.id, &Id::from("S1"), WorkspaceRole::Viewer, 3600, None)
        .await
        .unwrap();
    let (raw2, _) = repo
        .issue_share_token(&owner.id, &Id::from("S2"), WorkspaceRole::Editor, 3600, None)
        .await
        .unwrap();
    // Other user's share must be unaffected.
    let (raw_other, _) = repo
        .issue_share_token(&other.id, &Id::from("S3"), WorkspaceRole::Viewer, 3600, None)
        .await
        .unwrap();

    let session_ids = repo
        .revoke_all_shares_for_user(&owner.id)
        .await
        .expect("revoke all");

    // Both sessions returned.
    let mut ids: Vec<String> = session_ids.iter().map(|id| id.to_string()).collect();
    ids.sort();
    assert!(ids.contains(&"S1".to_string()), "S1 in revoked sessions");
    assert!(ids.contains(&"S2".to_string()), "S2 in revoked sessions");

    // Owner's shares no longer authenticate.
    assert!(
        repo.authenticate(&raw1).await.is_err(),
        "raw1 must fail after revoke_all"
    );
    assert!(
        repo.authenticate(&raw2).await.is_err(),
        "raw2 must fail after revoke_all"
    );

    // Other user's share is unaffected.
    assert!(
        repo.authenticate(&raw_other).await.is_ok(),
        "other user's share must be unaffected"
    );
}

/// list_shares_for_session returns only the live share for the given session.
#[tokio::test]
async fn list_shares_returns_live_shares_for_session() {
    let pool = mk_pool().await;
    let repo = AuthRepo::new(pool.clone());
    let owner = seed_user(&pool, "owner", false).await;

    let (_raw1, info1) = repo
        .issue_share_token(&owner.id, &Id::from("S1"), WorkspaceRole::Viewer, 3600, Some("viewer".into()))
        .await
        .unwrap();
    let (_raw2, info2) = repo
        .issue_share_token(&owner.id, &Id::from("S1"), WorkspaceRole::Editor, 3600, None)
        .await
        .unwrap();
    // Share for another session must not appear.
    let (_raw3, _) = repo
        .issue_share_token(&owner.id, &Id::from("S2"), WorkspaceRole::Viewer, 3600, None)
        .await
        .unwrap();

    // Revoke info2.
    repo.revoke_share(&owner.id, &info2.id).await.unwrap();

    let listed = repo.list_shares_for_session(&Id::from("S1")).await.unwrap();
    assert_eq!(listed.len(), 1, "only the live S1 share is listed");
    assert_eq!(listed[0].id, info1.id);
    assert_eq!(listed[0].role, WorkspaceRole::Viewer);
}

// ---------------------------------------------------------------------------
// Policy guard tests (scope guard denies mint/list for scoped tokens)
// ---------------------------------------------------------------------------

/// Build a minimal router with the share mint/list handlers + feature guard,
/// injecting the given AuthContext (mirrors what auth_middleware does).
fn build_scope_check_app(pool: SqlitePool, auth_ctx: AuthContext) -> Router {
    #[derive(Clone)]
    struct MinState {
        grants: GrantsRepo,
    }
    impl otto_server::feature_guard::HasGrants for MinState {
        fn grants(&self) -> GrantsRepo {
            self.grants.clone()
        }
    }

    let state = MinState {
        grants: GrantsRepo::new(pool.clone()),
    };

    async fn stub_ok() -> &'static str {
        "ok"
    }

    let protected = Router::new()
        .route("/sessions/{id}/share", post(stub_ok))
        .route("/sessions/{id}/shares", get(stub_ok));

    let protected = protected.route_layer(from_fn_with_state(
        state.clone(),
        feature_guard::<MinState>,
    ));

    let ctx = Arc::new(auth_ctx);
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

/// A scoped token calling POST /sessions/{id}/share → 403 (scope guard denies it).
#[tokio::test]
async fn scoped_token_cannot_mint_share() {
    let pool = mk_pool().await;
    let repo = AuthRepo::new(pool.clone());
    let owner_id = new_id();
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO users (id, username, password_hash, display_name, is_root, created_at)
         VALUES (?, ?, ?, ?, 0, ?)",
    )
    .bind(&owner_id)
    .bind("scoped-owner")
    .bind("hash")
    .bind("scoped-owner")
    .bind(&now)
    .execute(&pool)
    .await
    .unwrap();

    let (raw, _info) = repo
        .issue_share_token(&Id::from(&owner_id), &Id::from("S1"), WorkspaceRole::Viewer, 3600, None)
        .await
        .expect("mint share for scoped-owner");

    // Authenticate to get the scoped AuthContext.
    let auth_ctx = repo.authenticate(&raw).await.expect("authenticate scoped token");
    assert!(auth_ctx.scope.is_some(), "must be a scoped context");

    let app = build_scope_check_app(pool, auth_ctx);

    // A scoped token hitting POST /sessions/S1/share → 403 (scope guard denies non-allow-listed routes)
    let st = status(&app, Method::POST, "/api/v1/sessions/S1/share").await;
    assert_eq!(st, StatusCode::FORBIDDEN, "scoped token must be denied POST /sessions/{{id}}/share");
}

/// A scoped token calling GET /sessions/{id}/shares → 403.
#[tokio::test]
async fn scoped_token_cannot_list_shares() {
    let pool = mk_pool().await;
    let repo = AuthRepo::new(pool.clone());
    let owner_id = new_id();
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO users (id, username, password_hash, display_name, is_root, created_at)
         VALUES (?, ?, ?, ?, 0, ?)",
    )
    .bind(&owner_id)
    .bind("scoped-owner2")
    .bind("hash")
    .bind("scoped-owner2")
    .bind(&now)
    .execute(&pool)
    .await
    .unwrap();

    let (raw, _) = repo
        .issue_share_token(&Id::from(&owner_id), &Id::from("S1"), WorkspaceRole::Viewer, 3600, None)
        .await
        .unwrap();

    let auth_ctx = repo.authenticate(&raw).await.unwrap();
    let app = build_scope_check_app(pool, auth_ctx);

    let st = status(&app, Method::GET, "/api/v1/sessions/S1/shares").await;
    assert_eq!(st, StatusCode::FORBIDDEN, "scoped token must be denied GET /sessions/{{id}}/shares");
}

/// share_session_id returns the correct session id for a share.
#[tokio::test]
async fn share_session_id_lookup_works() {
    let pool = mk_pool().await;
    let repo = AuthRepo::new(pool.clone());
    let owner = seed_user(&pool, "owner", false).await;

    let (_raw, info) = repo
        .issue_share_token(&owner.id, &Id::from("SESS-42"), WorkspaceRole::Viewer, 3600, None)
        .await
        .unwrap();

    let found = repo
        .share_session_id(&owner.id, &info.id)
        .await
        .unwrap();
    assert_eq!(found, Some(Id::from("SESS-42")), "must return the pinned session id");

    // Another user cannot find it.
    let other = seed_user(&pool, "other", false).await;
    let found_other = repo.share_session_id(&other.id, &info.id).await.unwrap();
    assert!(found_other.is_none(), "other user must not find the share");
}
