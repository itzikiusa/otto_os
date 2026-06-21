//! Impersonation guardrail tests (RBAC Task 5.2) — the deliverable.
//!
//! These exercise the **real** code paths end-to-end:
//! - `AuthRepo::issue_impersonation_token` + `AuthRepo::authenticate` (the
//!   effective-user overlay: `real_user`=admin, `effective_user`=target),
//! - the real `auth_middleware`-equivalent injection (both `AuthUser`=effective
//!   and the full `AuthContext`, exactly as production inserts them),
//! - the real central `feature_guard` (so authorization runs against the
//!   *effective* user), and
//! - the real `impersonate::{start, stop}` handlers (every anti-escalation
//!   guardrail) plus the real PAT-mint guard.
//!
//! ## Harness note
//! As in `rbac_matrix.rs` / `grants_api.rs`, we do NOT assemble the full
//! `ServerCtx` (~30 `Arc` service handles). Instead a minimal `TestCtx` holds a
//! real SQLite pool and implements the small ctx traits the handlers need
//! (`ImpersonateCtx`, `GrantsCtx`/`HasGrants`). The auth layer calls the real
//! `AuthRepo::authenticate` on the `Authorization: Bearer` header — so the
//! impersonation overlay, the guard, and the handlers are the production code.

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode};
use axum::middleware::{from_fn_with_state, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json};
use chrono::Utc;
use http_body_util::BodyExt;
use otto_core::auth::{AuthContext, AuthUser};
use otto_core::domain::{Capability, Feature, User};
use otto_core::Error;
use otto_rbac::AuthRepo;
use otto_server::auth::{BearerToken, CurrentAuthContext};
use otto_server::error::ApiError;
use otto_server::feature_guard::feature_guard;
use otto_server::routes::grants::GrantsCtx;
use otto_server::routes::impersonate::{start, stop, ImpersonateCtx};
use otto_state::{AuditRepo, GrantsRepo, NewAuditEntry, SqlitePool, UsersRepo};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use tower::ServiceExt; // for `oneshot`

// ---------------------------------------------------------------------------
// Minimal test state implementing the ctx traits the handlers need.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct TestCtx {
    pool: SqlitePool,
}

impl otto_server::feature_guard::HasGrants for TestCtx {
    fn grants(&self) -> GrantsRepo {
        GrantsRepo::new(self.pool.clone())
    }
}

impl ImpersonateCtx for TestCtx {
    fn auth_repo(&self) -> AuthRepo {
        AuthRepo::new(self.pool.clone())
    }
    fn grants_repo(&self) -> GrantsRepo {
        GrantsRepo::new(self.pool.clone())
    }
    fn users_repo(&self) -> UsersRepo {
        UsersRepo::new(self.pool.clone())
    }
    async fn audit_entry(&self, entry: NewAuditEntry) {
        let action = entry.action.clone();
        if let Err(e) = AuditRepo::new(self.pool.clone()).insert(entry).await {
            tracing::warn!(%action, "test: audit insert failed: {e}");
        }
    }
}

impl GrantsCtx for TestCtx {
    fn grants_repo(&self) -> GrantsRepo {
        GrantsRepo::new(self.pool.clone())
    }
    fn audit_repo(&self) -> AuditRepo {
        AuditRepo::new(self.pool.clone())
    }
    fn users_repo(&self) -> UsersRepo {
        UsersRepo::new(self.pool.clone())
    }
    fn plugins_repo(&self) -> otto_state::PluginsRepo {
        otto_state::PluginsRepo::new(self.pool.clone())
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
        "INSERT INTO users (id, username, password_hash, display_name, is_root, disabled, created_at)
         VALUES (?, ?, ?, ?, ?, 0, ?)",
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

/// A faithful copy of the PAT-mint guard from `routes/auth_routes.rs::create_token`:
/// an impersonated request (real != effective) may not mint a token. We mirror
/// only the guard here because the production handler needs the full `ServerCtx`;
/// the extractor and the guard expression are identical to production.
async fn create_token_guarded(auth: CurrentAuthContext) -> Result<&'static str, ApiError> {
    if auth.real_user().id != auth.effective_user().id {
        return Err(ApiError(Error::Forbidden(
            "an impersonated session cannot mint API tokens".into(),
        )));
    }
    Ok("minted")
}

/// A `Database`-gated stub: reaching it means the feature guard allowed the
/// request, i.e. the *effective* user had `Database:Edit`. Used to prove
/// authorization runs against the effective (impersonated) identity.
async fn db_write_stub() -> &'static str {
    "wrote"
}

/// The auth layer: read `Authorization: Bearer <token>`, run the REAL
/// `AuthRepo::authenticate` (so an impersonation token resolves to its overlay
/// `AuthContext`), and insert the SAME extensions production's `auth_middleware`
/// inserts — `BearerToken`, `AuthUser`(=effective), and the full `AuthContext`.
async fn auth_layer(State(ctx): State<TestCtx>, mut req: Request, next: Next) -> Response {
    let token = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_owned);
    let Some(token) = token else {
        return ApiError(Error::Unauthorized).into_response();
    };
    match AuthRepo::new(ctx.pool.clone()).authenticate(&token).await {
        Ok(auth) => {
            req.extensions_mut().insert(BearerToken(token));
            req.extensions_mut()
                .insert(AuthUser(auth.effective_user.clone()));
            req.extensions_mut().insert(auth);
            next.run(req).await
        }
        Err(e) => ApiError(e).into_response(),
    }
}

/// Build the production-shaped router: auth layer → feature guard → real
/// handlers, nested under `/api/v1` so `MatchedPath` carries the prefix the
/// policy table keys on.
fn build_app(pool: SqlitePool) -> axum::Router {
    let state = TestCtx { pool };

    let protected = axum::Router::new()
        .route(
            "/admin/impersonate/{user_id}",
            post(start::<TestCtx>),
        )
        .route("/admin/impersonate/stop", post(stop::<TestCtx>))
        // PAT mint (guard mirrored from the real create_token).
        .route("/auth/tokens", post(create_token_guarded))
        // Identity echo (Exempt /auth/me): proves real/effective on a request.
        .route("/auth/me", get(me_echo))
        // A Database:Edit-gated stub to prove authz uses the effective user.
        .route("/connections/{id}/db/query", post(db_write_stub));

    // The real central guard, layered as a route_layer (so MatchedPath is set).
    let protected = protected.route_layer(from_fn_with_state(
        state.clone(),
        feature_guard::<TestCtx>,
    ));

    // The auth layer wraps everything (runs before the guard).
    let protected = protected.layer(from_fn_with_state(state.clone(), auth_layer));

    axum::Router::new()
        .nest("/api/v1", protected)
        .with_state(state)
}

/// `GET /auth/me`-style echo returning `{ real, effective }` ids so a test can
/// observe the overlay a token produced.
async fn me_echo(Extension(auth): Extension<AuthContext>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "real": auth.real_user.id,
        "effective": auth.effective_user.id,
        "real_username": auth.real_user.username,
        "effective_username": auth.effective_user.username,
    }))
}

// ---------------------------------------------------------------------------
// HTTP helpers
// ---------------------------------------------------------------------------

async fn req(
    app: &axum::Router,
    method: Method,
    path: &str,
    token: Option<&str>,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let body_bytes = match body {
        Some(ref v) => serde_json::to_vec(v).unwrap(),
        None => vec![],
    };
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if let Some(t) = token {
        builder = builder.header("authorization", format!("Bearer {t}"));
    }
    let request = builder.body(Body::from(body_bytes)).unwrap();
    let resp = app.clone().oneshot(request).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

/// Mint a normal session token for `user` (so a request carries that identity).
async fn login_token(pool: &SqlitePool, user: &User) -> String {
    AuthRepo::new(pool.clone()).issue(&user.id).await.unwrap()
}

// ===========================================================================
// Tests
// ===========================================================================

/// Admin starts impersonating a plain user: `/auth/me` then shows
/// `effective`=target / `real`=admin, and **authorization uses effective** — a
/// DB write succeeds only because the *target* has `Database:Edit`.
#[tokio::test]
async fn admin_can_impersonate_plain_user() {
    let pool = mk_pool().await;
    let admin = seed_user(&pool, "admin", false).await;
    let target = seed_user(&pool, "target", false).await;
    // admin holds Users:Admin (authorized to start impersonation);
    // target holds Database:Edit (and nothing on Users).
    GrantsRepo::new(pool.clone())
        .set_grants(&admin.id, &[(Feature::Users, Capability::Admin)])
        .await
        .unwrap();
    GrantsRepo::new(pool.clone())
        .set_grants(&target.id, &[(Feature::Database, Capability::Edit)])
        .await
        .unwrap();

    let app = build_app(pool.clone());
    let admin_token = login_token(&pool, &admin).await;

    // Start impersonation.
    let (st, body) = req(
        &app,
        Method::POST,
        &format!("/api/v1/admin/impersonate/{}", target.id),
        Some(&admin_token),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "start failed: {body}");
    let imp_token = body["token"].as_str().expect("token in response").to_string();

    // /auth/me with the impersonation token shows real=admin, effective=target.
    let (st, me) = req(&app, Method::GET, "/api/v1/auth/me", Some(&imp_token), None).await;
    assert_eq!(st, StatusCode::OK, "me failed: {me}");
    assert_eq!(me["real"].as_str(), Some(admin.id.as_str()), "real=admin");
    assert_eq!(
        me["effective"].as_str(),
        Some(target.id.as_str()),
        "effective=target"
    );

    // Authorization uses EFFECTIVE: target has Database:Edit, so the DB write is
    // allowed by the guard even though the *admin* has no Database grant at all.
    let (st, _) = req(
        &app,
        Method::POST,
        "/api/v1/connections/c1/db/query",
        Some(&imp_token),
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(
        st,
        StatusCode::OK,
        "authz must use effective user (target has Database:Edit)"
    );
}

/// Authorization uses the effective (impersonated) user, and is CONFINED to it:
/// impersonating a `Database:View`-only user → a DB *write* is 403.
#[tokio::test]
async fn authz_uses_effective_user_and_confines() {
    let pool = mk_pool().await;
    let admin = seed_user(&pool, "admin", true).await; // root admin
    let target = seed_user(&pool, "viewer", false).await;
    // target may only VIEW the database — not write.
    GrantsRepo::new(pool.clone())
        .set_grants(&target.id, &[(Feature::Database, Capability::View)])
        .await
        .unwrap();

    let app = build_app(pool.clone());
    let admin_token = login_token(&pool, &admin).await;

    let (st, body) = req(
        &app,
        Method::POST,
        &format!("/api/v1/admin/impersonate/{}", target.id),
        Some(&admin_token),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "start failed: {body}");
    let imp_token = body["token"].as_str().unwrap().to_string();

    // Even though the admin is ROOT, the impersonation overlay drops authority to
    // the target — a Database:View user — so a DB write is denied.
    let (st, _) = req(
        &app,
        Method::POST,
        "/api/v1/connections/c1/db/query",
        Some(&imp_token),
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(
        st,
        StatusCode::FORBIDDEN,
        "impersonating a Database:View user must NOT be able to write"
    );
}

/// Guardrail 2: cannot impersonate the root user → 403.
#[tokio::test]
async fn cannot_impersonate_root() {
    let pool = mk_pool().await;
    let admin = seed_user(&pool, "admin", true).await;
    let root_target = seed_user(&pool, "otherroot", true).await;

    let app = build_app(pool.clone());
    let admin_token = login_token(&pool, &admin).await;

    let (st, body) = req(
        &app,
        Method::POST,
        &format!("/api/v1/admin/impersonate/{}", root_target.id),
        Some(&admin_token),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN, "impersonating root must be 403");
    assert_eq!(body["code"], "forbidden", "body: {body}");
}

/// Guardrail 2: cannot impersonate a fellow `Users:Admin` (no sideways laundering)
/// → 403.
#[tokio::test]
async fn cannot_impersonate_users_admin() {
    let pool = mk_pool().await;
    let admin = seed_user(&pool, "admin", true).await;
    let target = seed_user(&pool, "otheradmin", false).await;
    GrantsRepo::new(pool.clone())
        .set_grants(&target.id, &[(Feature::Users, Capability::Admin)])
        .await
        .unwrap();

    let app = build_app(pool.clone());
    let admin_token = login_token(&pool, &admin).await;

    let (st, body) = req(
        &app,
        Method::POST,
        &format!("/api/v1/admin/impersonate/{}", target.id),
        Some(&admin_token),
        None,
    )
    .await;
    assert_eq!(
        st,
        StatusCode::FORBIDDEN,
        "impersonating a Users:Admin must be 403"
    );
    assert_eq!(body["code"], "forbidden");
}

/// Guardrail 5: cannot impersonate yourself → 403.
#[tokio::test]
async fn cannot_impersonate_self() {
    let pool = mk_pool().await;
    let admin = seed_user(&pool, "admin", true).await;

    let app = build_app(pool.clone());
    let admin_token = login_token(&pool, &admin).await;

    let (st, _body) = req(
        &app,
        Method::POST,
        &format!("/api/v1/admin/impersonate/{}", admin.id),
        Some(&admin_token),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN, "self-impersonation must be 403");
}

/// Guardrail 3: an impersonation token cannot start another impersonation → 403.
#[tokio::test]
async fn impersonation_token_cannot_nest() {
    let pool = mk_pool().await;
    let admin = seed_user(&pool, "admin", true).await;
    let target = seed_user(&pool, "target", false).await;
    let third = seed_user(&pool, "third", false).await;

    let app = build_app(pool.clone());
    let admin_token = login_token(&pool, &admin).await;

    // Start impersonating `target`.
    let (st, body) = req(
        &app,
        Method::POST,
        &format!("/api/v1/admin/impersonate/{}", target.id),
        Some(&admin_token),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "start failed: {body}");
    let imp_token = body["token"].as_str().unwrap().to_string();

    // Now try to start ANOTHER impersonation with the impersonation token.
    let (st, _) = req(
        &app,
        Method::POST,
        &format!("/api/v1/admin/impersonate/{}", third.id),
        Some(&imp_token),
        None,
    )
    .await;
    assert_eq!(
        st,
        StatusCode::FORBIDDEN,
        "an impersonation token must not start another impersonation"
    );
}

/// Guardrail 4: an impersonation token cannot mint a PAT
/// (`POST /auth/tokens`) → 403.
#[tokio::test]
async fn impersonation_token_cannot_mint_pat() {
    let pool = mk_pool().await;
    let admin = seed_user(&pool, "admin", true).await;
    let target = seed_user(&pool, "target", false).await;

    let app = build_app(pool.clone());
    let admin_token = login_token(&pool, &admin).await;

    // A normal (non-impersonated) token CAN mint a PAT through the guarded stub.
    let (st, _) = req(
        &app,
        Method::POST,
        "/api/v1/auth/tokens",
        Some(&admin_token),
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "a normal token may mint a PAT");

    // Start impersonating, then attempt the same mint with the impersonation
    // token — denied.
    let (_st, body) = req(
        &app,
        Method::POST,
        &format!("/api/v1/admin/impersonate/{}", target.id),
        Some(&admin_token),
        None,
    )
    .await;
    let imp_token = body["token"].as_str().unwrap().to_string();

    let (st, _) = req(
        &app,
        Method::POST,
        "/api/v1/auth/tokens",
        Some(&imp_token),
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(
        st,
        StatusCode::FORBIDDEN,
        "an impersonated session must not be able to mint a PAT"
    );
}

/// Audit: `impersonate.start` records BOTH the real (admin) and the effective
/// (target) ids.
#[tokio::test]
async fn audit_records_real_and_effective() {
    let pool = mk_pool().await;
    let admin = seed_user(&pool, "admin", true).await;
    let target = seed_user(&pool, "target", false).await;

    let app = build_app(pool.clone());
    let admin_token = login_token(&pool, &admin).await;

    let (st, _body) = req(
        &app,
        Method::POST,
        &format!("/api/v1/admin/impersonate/{}", target.id),
        Some(&admin_token),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);

    use otto_core::api::AuditLogQuery;
    let entries = AuditRepo::new(pool.clone())
        .list(&AuditLogQuery {
            action: Some("impersonate.start".into()),
            user_id: None,
            from: None,
            to: None,
            limit: Some(10),
            offset: None,
        })
        .await
        .expect("list audit");
    assert!(!entries.is_empty(), "impersonate.start audit entry must exist");
    let e = &entries[0];
    // Actor = the REAL user (the admin).
    assert_eq!(
        e.user_id.as_deref(),
        Some(admin.id.as_str()),
        "audit actor must be the real (admin) user"
    );
    // Target = the effective (impersonated) user.
    assert_eq!(
        e.target.as_deref(),
        Some(target.id.as_str()),
        "audit target must be the effective user"
    );
    let detail = e.detail.as_ref().expect("detail set");
    assert_eq!(
        detail["real_user_id"].as_str(),
        Some(admin.id.as_str()),
        "detail records the real user id"
    );
    assert_eq!(
        detail["effective_user_id"].as_str(),
        Some(target.id.as_str()),
        "detail records the effective user id"
    );
}

/// Stop revokes the presented impersonation token: after `stop`, the token no
/// longer authenticates (401).
#[tokio::test]
async fn stop_revokes() {
    let pool = mk_pool().await;
    let admin = seed_user(&pool, "admin", true).await;
    let target = seed_user(&pool, "target", false).await;

    let app = build_app(pool.clone());
    let admin_token = login_token(&pool, &admin).await;

    let (st, body) = req(
        &app,
        Method::POST,
        &format!("/api/v1/admin/impersonate/{}", target.id),
        Some(&admin_token),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "start failed: {body}");
    let imp_token = body["token"].as_str().unwrap().to_string();

    // The token works before stop.
    let (st, _) = req(&app, Method::GET, "/api/v1/auth/me", Some(&imp_token), None).await;
    assert_eq!(st, StatusCode::OK, "token must work before stop");

    // Stop (revokes the presented impersonation token).
    let (st, _) = req(
        &app,
        Method::POST,
        "/api/v1/admin/impersonate/stop",
        Some(&imp_token),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT, "stop must return 204");

    // After stop the impersonation token is invalid.
    let (st, _) = req(&app, Method::GET, "/api/v1/auth/me", Some(&imp_token), None).await;
    assert_eq!(
        st,
        StatusCode::UNAUTHORIZED,
        "after stop the impersonation token must no longer authenticate"
    );

    // An `impersonate.stop` audit entry was written.
    use otto_core::api::AuditLogQuery;
    let entries = AuditRepo::new(pool.clone())
        .list(&AuditLogQuery {
            action: Some("impersonate.stop".into()),
            user_id: None,
            from: None,
            to: None,
            limit: Some(10),
            offset: None,
        })
        .await
        .expect("list audit");
    assert!(!entries.is_empty(), "impersonate.stop audit entry must exist");
}
