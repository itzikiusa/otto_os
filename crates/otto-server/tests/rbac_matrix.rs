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
use otto_state::{DbPool, GrantsRepo};
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

/// Seed a user row and return the `User` (mirrors `grants.rs` test helper).
async fn seed_user(pool: &DbPool, username: &str, is_root: bool) -> User {
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
fn app(pool: DbPool, user: User) -> Router {
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
        // DB Assistant — gated on Connections:Edit (NOT Database), so it must win
        // over the generic `/connections/{id}/db/` Database prefix.
        .route("/connections/{id}/db/assist", post(ok))
        // Connections feature
        .route("/connections", post(ok))
        .route("/connections/{id}/open", post(ok))
        // Agents feature
        .route("/workspaces/{id}/sessions", get(ok))
        // Swarm feature
        .route("/workspaces/{id}/swarm/swarms", get(ok))
        // Personal documents: read-only grants cannot save memory/context.
        .route("/personal-agents/{id}/memory", get(ok).put(ok))
        .route("/personal-agents/{id}/context", get(ok).put(ok))
        // Otto Assistant (Feature::Agents): reads View, writes Edit; the route
        // preview is a POST read.
        .route("/assistant/threads", get(ok).post(ok))
        .route("/assistant/route/preview", post(ok))
        .route("/assistant/agent/{tool}", post(ok))
        // Users / Settings (Admin)
        .route("/users", post(ok))
        .route("/settings", put(ok))
        // Design Hall (Feature::Design): reads View, writes Edit, admin Admin.
        .route("/design/artifacts", get(ok).post(ok))
        .route("/design/admin/import", post(ok))
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
async fn db_assist_route_is_connections_edit_not_database() {
    // The DB Assistant start route is gated on Connections:Edit — and that rule
    // must win over the `/connections/{id}/db/` Database prefix.
    // A Connections:Edit user is allowed.
    let app = app_for(&[(Feature::Connections, Capability::Edit)], false).await;
    assert_eq!(
        status(&app, Method::POST, "/api/v1/connections/c1/db/assist").await,
        StatusCode::OK,
        "Connections:Edit must allow starting a DB Assistant turn"
    );
    // A Database:Edit user (who CAN run queries) is DENIED — proving the route is
    // Connections-gated, not Database-gated.
    let app = app_for(&[(Feature::Database, Capability::Edit)], false).await;
    assert_eq!(
        status(&app, Method::POST, "/api/v1/connections/c1/db/assist").await,
        StatusCode::FORBIDDEN,
        "DB Assistant is Connections:Edit, not Database:Edit"
    );
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

#[tokio::test]
async fn personal_document_viewer_cannot_save() {
    let viewer = app_for(&[(Feature::ScheduledTasks, Capability::View)], false).await;
    let editor = app_for(&[(Feature::ScheduledTasks, Capability::Edit)], false).await;
    for path in [
        "/api/v1/personal-agents/a/memory",
        "/api/v1/personal-agents/a/context",
    ] {
        assert_eq!(status(&viewer, Method::GET, path).await, StatusCode::OK);
        assert_eq!(
            status(&viewer, Method::PUT, path).await,
            StatusCode::FORBIDDEN
        );
        assert_eq!(status(&editor, Method::PUT, path).await, StatusCode::OK);
    }
}

#[tokio::test]
async fn assistant_routes_follow_the_agents_view_edit_ladder() {
    let viewer = app_for(&[(Feature::Agents, Capability::View)], false).await;
    assert_eq!(
        status(&viewer, Method::GET, "/api/v1/assistant/threads").await,
        StatusCode::OK
    );
    assert_eq!(
        status(&viewer, Method::POST, "/api/v1/assistant/route/preview").await,
        StatusCode::OK,
        "the route preview is a read"
    );
    assert_eq!(
        status(&viewer, Method::POST, "/api/v1/assistant/threads").await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        status(&viewer, Method::POST, "/api/v1/assistant/agent/remember").await,
        StatusCode::FORBIDDEN
    );
    let editor = app_for(&[(Feature::Agents, Capability::Edit)], false).await;
    assert_eq!(
        status(&editor, Method::POST, "/api/v1/assistant/threads").await,
        StatusCode::OK
    );
    assert_eq!(
        status(&editor, Method::POST, "/api/v1/assistant/agent/remember").await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn design_grants_follow_the_view_edit_admin_ladder() {
    let viewer = app_for(&[(Feature::Design, Capability::View)], false).await;
    assert_eq!(
        status(&viewer, Method::GET, "/api/v1/design/artifacts").await,
        StatusCode::OK
    );
    assert_eq!(
        status(&viewer, Method::POST, "/api/v1/design/artifacts").await,
        StatusCode::FORBIDDEN
    );
    let editor = app_for(&[(Feature::Design, Capability::Edit)], false).await;
    assert_eq!(
        status(&editor, Method::POST, "/api/v1/design/artifacts").await,
        StatusCode::OK
    );
    assert_eq!(
        status(&editor, Method::POST, "/api/v1/design/admin/import").await,
        StatusCode::FORBIDDEN,
        "the legacy import is a Design:Admin action"
    );
    let admin = app_for(&[(Feature::Design, Capability::Admin)], false).await;
    assert_eq!(
        status(&admin, Method::POST, "/api/v1/design/admin/import").await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn canvas_grant_alone_does_not_open_design_routes() {
    // The migration copies existing canvas grants to `design` once; at runtime
    // the two features are independent axes.
    let app = app_for(&[(Feature::Canvas, Capability::Admin)], false).await;
    assert_eq!(
        status(&app, Method::GET, "/api/v1/design/artifacts").await,
        StatusCode::FORBIDDEN
    );
}

// ---------------------------------------------------------------------------
// Credential class (S11-01/03, S8-01, S1-02, S3-01): every Admin / Secret
// route refuses an agent session's own credential, even when it acts as root.
// ---------------------------------------------------------------------------

/// The credential a request carries in [`class_app`].
#[derive(Clone, Copy, Debug)]
enum Cred {
    /// A person's own login token.
    Human,
    /// An author session's API token (`managed_session_id`).
    AgentToken,
    /// A session's internal MCP credential used as a bearer (`mcp_session_id`).
    AgentMcpSession,
}

/// Every registered `(method, template)` the policy tags with a credential
/// class, mounted as a stub behind the real guard; the caller is ROOT so the
/// feature axis always passes and only the class gate can refuse.
fn class_app(pool: DbPool, user: User, cred: Cred) -> (Router, Vec<(Method, String, String)>) {
    use otto_server::policy::route_class;
    let state = TestState {
        grants: GrantsRepo::new(pool),
    };
    async fn ok() -> &'static str {
        "ok"
    }
    let root = super::policy_coverage::repo_root();
    let methods = [
        Method::GET,
        Method::POST,
        Method::PUT,
        Method::PATCH,
        Method::DELETE,
    ];
    let mut protected: Router<TestState> = Router::new();
    let mut tagged = Vec::new();
    for template in super::policy_coverage::registered_routes(&root) {
        if template.starts_with("/ws/")
            || template == "/browser/proxy"
            || template.starts_with("/plugins/")
        {
            continue;
        }
        let full = format!("/api/v1{template}");
        let classes: Vec<_> = methods
            .iter()
            .filter_map(|m| route_class(m, &full).map(|c| (m.clone(), c)))
            .collect();
        if classes.is_empty() {
            continue;
        }
        protected = protected.route(&template, axum::routing::any(ok));
        // `{id}` → `x1`, `{*rest}` → `a`: any concrete segment matches.
        let concrete: String = template
            .split('/')
            .map(|seg| {
                if seg.starts_with("{*") {
                    "a".to_string()
                } else if seg.starts_with('{') {
                    "x1".to_string()
                } else {
                    seg.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("/");
        for (m, c) in classes {
            tagged.push((m, format!("/api/v1{concrete}"), format!("{c:?}")));
        }
    }
    let protected = protected.route_layer(from_fn_with_state(
        state.clone(),
        feature_guard::<TestState>,
    ));
    let injected = Arc::new(user);
    let protected = protected.layer(from_fn(move |mut req: Request, next: Next| {
        let u = injected.clone();
        async move {
            let mut ctx = otto_core::auth::AuthContext {
                real_user: (*u).clone(),
                effective_user: (*u).clone(),
                scope: None,
                mcp_only: false,
                mcp_scope: None,
                mcp_internal: false,
                mcp_session_id: None,
                managed_session_id: None,
            };
            match cred {
                Cred::Human => {}
                Cred::AgentToken => ctx.managed_session_id = Some("agent-sess".into()),
                Cred::AgentMcpSession => ctx.mcp_session_id = Some("agent-sess".into()),
            }
            req.extensions_mut().insert(AuthUser((*u).clone()));
            req.extensions_mut().insert(ctx);
            next.run(req).await
        }
    }));
    (
        Router::new().nest("/api/v1", protected).with_state(state),
        tagged,
    )
}

#[tokio::test]
async fn agent_credentials_are_refused_on_every_admin_and_secret_route() {
    let pool = mem_pool().await;
    let root = seed_user(&pool, "root-owner", true).await;
    let (human_app, tagged) = class_app(pool.clone(), root.clone(), Cred::Human);
    // Sanity floor: the scanner and the tag table both still work, and the
    // findings' named routes are among the tagged ones.
    let admin_or_secret: Vec<_> = tagged.iter().filter(|(_, _, c)| c != "Outward").collect();
    assert!(
        admin_or_secret.len() >= 60,
        "only {} tagged",
        admin_or_secret.len()
    );
    for must in [
        (Method::PUT, "/api/v1/settings"),
        (Method::POST, "/api/v1/plugin-admin/install"),
        (Method::PATCH, "/api/v1/users/x1"),
        (Method::POST, "/api/v1/admin/impersonate/x1"),
        (Method::POST, "/api/v1/state/connections/export"),
        (Method::POST, "/api/v1/browser/credentials/x1/reveal"),
        (Method::POST, "/api/v1/sessions/x1/share"),
        (Method::POST, "/api/v1/workflow-runs/x1/approve"),
    ] {
        assert!(
            admin_or_secret
                .iter()
                .any(|(m, p, _)| *m == must.0 && p == must.1),
            "{} {} must be tagged Admin/Secret",
            must.0,
            must.1
        );
    }

    let mut wrong = Vec::new();
    for cred in [Cred::AgentToken, Cred::AgentMcpSession] {
        let (agent_app, _) = class_app(pool.clone(), root.clone(), cred);
        for (m, path, class) in &tagged {
            let human = status(&human_app, m.clone(), path).await;
            let agent = status(&agent_app, m.clone(), path).await;
            let want_agent = if class == "Outward" {
                // Tag only until the user decides (see `credential_class_gate`).
                StatusCode::OK
            } else {
                StatusCode::FORBIDDEN
            };
            if human != StatusCode::OK || agent != want_agent {
                wrong.push(format!(
                    "{cred:?} {m} {path} [{class}]: human {human}, agent {agent} (want {want_agent})"
                ));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "credential-class gate:\n{}",
        wrong.join("\n")
    );
}

// ---------------------------------------------------------------------------
// S11 item 6 — every non-GET route replayed with an agent-session token.
// ---------------------------------------------------------------------------

/// `template` alone, mounted behind the real guard + auth injection, with a
/// stub handler that is ROOT-GATED exactly like a `require_root` write
/// handler: 200 when `otto_server::auth::require_root` passes, else 403.
fn root_gated_one(state: TestState, template: &str, user: User, cred: Cred) -> Router {
    async fn root_gated(
        axum::extract::Extension(AuthUser(u)): axum::extract::Extension<AuthUser>,
    ) -> StatusCode {
        if otto_server::auth::require_root(&u).is_ok() {
            StatusCode::OK
        } else {
            StatusCode::FORBIDDEN
        }
    }
    let protected: Router<TestState> = Router::new()
        .route(template, axum::routing::any(root_gated))
        .route_layer(from_fn_with_state(
            state.clone(),
            feature_guard::<TestState>,
        ));
    let protected = protected.layer(from_fn(move |mut req: Request, next: Next| {
        let u = user.clone();
        async move {
            let mut ctx = otto_core::auth::AuthContext {
                real_user: u.clone(),
                effective_user: u.clone(),
                scope: None,
                mcp_only: false,
                mcp_scope: None,
                mcp_internal: false,
                mcp_session_id: None,
                managed_session_id: None,
            };
            match cred {
                Cred::Human => {}
                Cred::AgentToken => ctx.managed_session_id = Some("agent-sess".into()),
                Cred::AgentMcpSession => ctx.mcp_session_id = Some("agent-sess".into()),
            }
            req.extensions_mut().insert(AuthUser(u));
            req.extensions_mut().insert(ctx);
            next.run(req).await
        }
    }));
    Router::new().nest("/api/v1", protected).with_state(state)
}

/// Replays EVERY registered route × POST/PUT/PATCH/DELETE through the real
/// guard into a root-gated stub, as the person and as an agent session's
/// own credentials (both authorizing as ROOT). A person passes everywhere;
/// an agent passes ONLY where `policy::agent_root_write_allowed` (the
/// reviewed allow-list: the Outward routes) says so — everywhere else the
/// class gate or the withheld root refuses it with 403. This is the guard
/// that would have caught S11-301/303/306/308: a new root-gated write is
/// closed to agents by default, and widening the allow-list shows up here.
#[tokio::test]
async fn every_write_route_refuses_agent_root_authority_unless_allow_listed() {
    use otto_server::policy::{agent_root_write_allowed, route_class, RouteClass};
    let pool = mem_pool().await;
    let root_user = seed_user(&pool, "root-owner", true).await;
    let state = TestState {
        grants: GrantsRepo::new(pool),
    };
    let root = super::policy_coverage::repo_root();
    let methods = [Method::POST, Method::PUT, Method::PATCH, Method::DELETE];
    let mut wrong = Vec::new();
    let mut allowed = 0usize;
    let mut refused = 0usize;
    for template in super::policy_coverage::registered_routes(&root) {
        // Root-mounted (not behind the `/api/v1` guard) or plugin-proxied.
        if template.starts_with("/ws/")
            || template == "/browser/proxy"
            || template == "/health"
            || template == "/meta"
            || template.starts_with("/plugins/")
        {
            continue;
        }
        let full = format!("/api/v1{template}");
        let concrete: String = template
            .split('/')
            .map(|seg| {
                if seg.starts_with("{*") {
                    "a".to_string()
                } else if seg.starts_with('{') {
                    "x1".to_string()
                } else {
                    seg.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("/");
        let path = format!("/api/v1{concrete}");
        let human = root_gated_one(state.clone(), &template, root_user.clone(), Cred::Human);
        let agents = [
            root_gated_one(
                state.clone(),
                &template,
                root_user.clone(),
                Cred::AgentToken,
            ),
            root_gated_one(
                state.clone(),
                &template,
                root_user.clone(),
                Cred::AgentMcpSession,
            ),
        ];
        for m in &methods {
            let ok_for_agent = agent_root_write_allowed(m, &full);
            // The allow-list never contains a person-only route.
            if ok_for_agent
                && matches!(
                    route_class(m, &full),
                    Some(RouteClass::Admin | RouteClass::Secret)
                )
            {
                wrong.push(format!("{m} {full}: allow-listed AND Admin/Secret"));
            }
            let h = status(&human, m.clone(), &path).await;
            if h != StatusCode::OK {
                wrong.push(format!("{m} {full}: person got {h}"));
            }
            let want = if ok_for_agent {
                allowed += 1;
                StatusCode::OK
            } else {
                refused += 1;
                StatusCode::FORBIDDEN
            };
            for app in &agents {
                let a = status(app, m.clone(), &path).await;
                if a != want {
                    wrong.push(format!("{m} {full}: agent got {a}, want {want}"));
                }
            }
        }
    }
    // Sanity floors: the scanner works and the allow-list is the Outward set
    // (PR create/merge/approve, push, Jira/Confluence writes, …), not empty.
    assert!(
        refused >= 2000,
        "only {refused} refused pairs — scanner broke?"
    );
    assert!(allowed >= 20, "only {allowed} allow-listed pairs");
    assert!(
        agent_root_write_allowed(&Method::POST, "/api/v1/repos/{id}/prs/{number}/merge"),
        "the otto-pr skill's merge must stay open to agents"
    );
    assert!(
        wrong.is_empty(),
        "agent root authority on writes ({} wrong):\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}
