//! Integration tests for the Grants API endpoints (Task 2.1):
//!   GET  /api/v1/users/{id}/grants
//!   PUT  /api/v1/users/{id}/grants
//!   GET  /api/v1/auth/capabilities
//!
//! Uses the same minimal-router harness as `rbac_matrix.rs`: a lightweight
//! `TestGrantCtx` (pool + repos) that implements `GrantsCtx` + `HasGrants`,
//! plus the feature-guard middleware and a stub `AuthUser` injection layer.
//!
//! The full `ServerCtx` is NOT assembled here — that would couple this test to
//! ~30 unrelated subsystems.

use axum::body::Body;
use axum::extract::Request;
use axum::http::{Method, StatusCode};
use axum::middleware::{from_fn, from_fn_with_state, Next};
use axum::routing::{get, post};
use axum::Router;
use chrono::Utc;
use http_body_util::BodyExt;
use otto_core::auth::AuthUser;
use otto_core::domain::{Capability, Feature, User};
use otto_server::feature_guard::feature_guard;
use otto_server::routes::grants::{capabilities, get_grants, put_grants, GrantsCtx};
use otto_state::{AuditRepo, GrantsRepo, NewAuditEntry, PluginsRepo, SqlitePool, UsersRepo};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::sync::Arc;
use tower::ServiceExt; // for `oneshot`

// ---------------------------------------------------------------------------
// Minimal test state implementing both HasGrants and GrantsCtx.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct TestGrantCtx {
    pool: SqlitePool,
}

impl otto_server::feature_guard::HasGrants for TestGrantCtx {
    fn grants(&self) -> GrantsRepo {
        GrantsRepo::new(self.pool.clone())
    }
}

impl GrantsCtx for TestGrantCtx {
    fn grants_repo(&self) -> GrantsRepo {
        GrantsRepo::new(self.pool.clone())
    }
    fn audit_repo(&self) -> AuditRepo {
        AuditRepo::new(self.pool.clone())
    }
    fn users_repo(&self) -> UsersRepo {
        UsersRepo::new(self.pool.clone())
    }
    fn plugins_repo(&self) -> PluginsRepo {
        PluginsRepo::new(self.pool.clone())
    }
    async fn audit_entry(&self, entry: NewAuditEntry) {
        let action = entry.action.clone();
        if let Err(e) = AuditRepo::new(self.pool.clone()).insert(entry).await {
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

/// Build a minimal router with the actual grant handlers + the feature guard
/// layered, with a synthetic `AuthUser` injected for `actor`.
fn build_app(pool: SqlitePool, actor: User) -> Router {
    let state = TestGrantCtx { pool };

    let protected = Router::new()
        .route(
            "/users/{id}/grants",
            get(get_grants::<TestGrantCtx>).put(put_grants::<TestGrantCtx>),
        )
        .route("/auth/capabilities", get(capabilities::<TestGrantCtx>))
        // Sentinel: a Users:Admin-guarded route to verify the guard denies non-admins.
        .route("/users", post(|| async { "ok" }));

    // Layer the feature guard (same as production) as a route_layer.
    let protected = protected.route_layer(from_fn_with_state(
        state.clone(),
        feature_guard::<TestGrantCtx>,
    ));

    // Inject the actor's AuthUser extension (same as auth_middleware).
    let injected = Arc::new(actor);
    let protected = protected.layer(from_fn(move |mut req: Request, next: Next| {
        let u = injected.clone();
        async move {
            req.extensions_mut().insert(AuthUser((*u).clone()));
            next.run(req).await
        }
    }));

    Router::new()
        .nest("/api/v1", protected)
        .with_state(state)
}

// ---------------------------------------------------------------------------
// HTTP helpers
// ---------------------------------------------------------------------------

async fn do_req(
    app: &Router,
    method: Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let body_bytes = match body {
        Some(ref v) => serde_json::to_vec(v).unwrap(),
        None => vec![],
    };
    let req = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json")
        .body(Body::from(body_bytes))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

// ---------------------------------------------------------------------------
// Policy smoke checks (no DB, pure policy_for calls).
// ---------------------------------------------------------------------------

#[test]
fn grants_routes_have_correct_policy() {
    use axum::http::Method;
    use otto_server::policy::{policy_for, PolicyDecision};

    // GET and PUT /users/{id}/grants → Users:Admin (the /users/ prefix rule).
    assert_eq!(
        policy_for(&Method::GET, "/api/v1/users/{id}/grants"),
        PolicyDecision::Require(Feature::Users, Capability::Admin),
        "GET /users/{{id}}/grants must require Users:Admin"
    );
    assert_eq!(
        policy_for(&Method::PUT, "/api/v1/users/{id}/grants"),
        PolicyDecision::Require(Feature::Users, Capability::Admin),
        "PUT /users/{{id}}/grants must require Users:Admin"
    );

    // GET /auth/capabilities → Exempt (self-scoped, any authed user).
    assert_eq!(
        policy_for(&Method::GET, "/api/v1/auth/capabilities"),
        PolicyDecision::Exempt,
        "GET /auth/capabilities must be Exempt"
    );
}

// ---------------------------------------------------------------------------
// Admin reads and sets a user's grants, then reads them back.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn admin_can_set_and_read_grants() {
    let pool = mk_pool().await;
    let admin = seed_user(&pool, "admin", true).await;
    let alice = seed_user(&pool, "alice", false).await;

    let app = build_app(pool.clone(), admin.clone());

    // PUT grants for alice.
    let put_body = serde_json::json!({
        "grants": [
            {"feature": "database", "capability": "view"},
            {"feature": "git", "capability": "edit"}
        ]
    });
    let (st, resp) = do_req(
        &app,
        Method::PUT,
        &format!("/api/v1/users/{}/grants", alice.id),
        Some(put_body),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "admin PUT grants failed: {resp}");

    // GET grants for alice.
    let (st, body) = do_req(
        &app,
        Method::GET,
        &format!("/api/v1/users/{}/grants", alice.id),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "admin GET grants failed: {body}");
    let grants = body["grants"].as_array().expect("grants array");
    assert_eq!(grants.len(), 2, "alice should have 2 grants; got: {body}");

    let features: Vec<&str> = grants
        .iter()
        .map(|g| g["feature"].as_str().unwrap())
        .collect();
    assert!(features.contains(&"database"), "database grant present");
    assert!(features.contains(&"git"), "git grant present");
}

// ---------------------------------------------------------------------------
// Non-admin (no Users grant) gets 403 on PUT /users/{id}/grants.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn non_admin_cannot_put_grants() {
    let pool = mk_pool().await;
    let alice = seed_user(&pool, "alice", false).await;
    let target = seed_user(&pool, "target", false).await;

    // alice has Database:View only — no Users grant.
    GrantsRepo::new(pool.clone())
        .set_grants(&alice.id, &[(Feature::Database, Capability::View)])
        .await
        .unwrap();

    let app = build_app(pool.clone(), alice);

    let put_body = serde_json::json!({
        "grants": [{"feature": "database", "capability": "admin"}]
    });
    let (st, body) = do_req(
        &app,
        Method::PUT,
        &format!("/api/v1/users/{}/grants", target.id),
        Some(put_body),
    )
    .await;
    assert_eq!(
        st,
        StatusCode::FORBIDDEN,
        "non-admin must be denied PUT /users/{{id}}/grants; got: {body}"
    );
    assert_eq!(body["code"], "forbidden");
}

// ---------------------------------------------------------------------------
// /auth/capabilities reflects a non-root caller's grants.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn capabilities_reflects_grant() {
    let pool = mk_pool().await;
    let alice = seed_user(&pool, "alice", false).await;

    GrantsRepo::new(pool.clone())
        .set_grants(&alice.id, &[(Feature::Database, Capability::View)])
        .await
        .unwrap();

    let app = build_app(pool.clone(), alice);
    let (st, body) = do_req(&app, Method::GET, "/api/v1/auth/capabilities", None).await;
    assert_eq!(st, StatusCode::OK, "capabilities call failed: {body}");

    let caps = body["capabilities"].as_object().expect("capabilities object");
    assert_eq!(
        caps.get("database").and_then(|v| v.as_str()),
        Some("view"),
        "database capability should be view"
    );
    // A feature alice has no grant for should report "none".
    assert_eq!(
        caps.get("git").and_then(|v| v.as_str()),
        Some("none"),
        "no git grant → none"
    );
}

// ---------------------------------------------------------------------------
// Root ⇒ all features are "admin" in /auth/capabilities.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn capabilities_root_gets_all_admin() {
    let pool = mk_pool().await;
    let root = seed_user(&pool, "root", true).await;

    let app = build_app(pool.clone(), root);
    let (st, body) = do_req(&app, Method::GET, "/api/v1/auth/capabilities", None).await;
    assert_eq!(st, StatusCode::OK, "capabilities call failed: {body}");

    let caps = body["capabilities"].as_object().expect("capabilities object");
    // At least the 18 built-in features are present. Installed custom plugins add
    // their own slug-keyed capabilities too (string-keyed RBAC axis), so the map
    // may be larger than 18 — assert a lower bound, not an exact count.
    assert!(
        caps.len() >= 18,
        "root capabilities should cover at least the 18 built-in features; got: {body}"
    );
    // Every value is "admin" (including any plugin slugs — root is admin everywhere).
    for (feat, cap_val) in caps {
        assert_eq!(
            cap_val.as_str(),
            Some("admin"),
            "root should have admin for feature '{feat}'"
        );
    }
}

// ---------------------------------------------------------------------------
// PUT /users/{id}/grants writes an audit entry with actor + target + old/new.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn put_grants_writes_audit_entry() {
    let pool = mk_pool().await;
    let admin = seed_user(&pool, "admin", true).await;
    let alice = seed_user(&pool, "alice", false).await;

    let app = build_app(pool.clone(), admin.clone());

    let put_body = serde_json::json!({
        "grants": [{"feature": "agents", "capability": "edit"}]
    });
    let (st, resp) = do_req(
        &app,
        Method::PUT,
        &format!("/api/v1/users/{}/grants", alice.id),
        Some(put_body),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "PUT grants failed: {resp}");

    // Verify an audit entry was written.
    use otto_core::api::AuditLogQuery;
    let audit = AuditRepo::new(pool.clone());
    let entries = audit
        .list(&AuditLogQuery {
            action: Some("grant.changed".into()),
            user_id: None,
            from: None,
            to: None,
            limit: Some(10),
            offset: None,
        })
        .await
        .expect("list audit log");
    assert!(
        !entries.is_empty(),
        "grant.changed audit entry should exist"
    );
    let entry = &entries[0];
    assert_eq!(
        entry.user_id.as_deref(),
        Some(admin.id.as_str()),
        "actor should be admin"
    );
    assert_eq!(
        entry.target.as_deref(),
        Some(alice.id.as_str()),
        "target should be alice"
    );
    // Detail should contain old/new grant lists.
    let detail = entry.detail.as_ref().expect("audit detail should be set");
    assert!(detail.get("old").is_some(), "detail.old present");
    assert!(detail.get("new").is_some(), "detail.new present");
}
