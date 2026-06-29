//! Activity trail / tasks / summary ownership isolation tests (Task 3.5,
//! leaks #L16–#L18).
//!
//! These prove the three isolation properties through a real in-memory SQLite
//! store:
//!
//! - **#L16 / #L17**: a workspace Editor who is *not* the session owner gets
//!   403 on `GET .../trail` and `GET .../tasks` for another user's session;
//!   the owner, a workspace Admin, and root all get through (non-403).
//!
//! - **#L18**: `GET .../activity/summary` returns **only the caller's own
//!   sessions** for a non-admin caller, while a workspace Admin and root see
//!   the full cross-user roll-up.
//!
//! The harness builds a real minimal `ServerCtx` (pool + manager + roles +
//! events) and exercises the handlers via `tower::ServiceExt::oneshot`, with
//! the `AuthUser` extension injected exactly as the production
//! `auth_middleware` does.

use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::Request;
use axum::http::{Method, StatusCode};
use axum::Router;
use chrono::Utc;
use http_body_util::BodyExt;
use otto_core::auth::AuthUser;
use otto_core::domain::User;
use otto_core::secrets::SecretStore;
use otto_core::{Error, Id, Result};
use otto_rbac::RbacRoleChecker;
use otto_server::ServerCtx;
use otto_sessions::{ProviderRegistry, SessionManager};
use otto_state::{
    ConnectionSectionsRepo, ConnectionsRepo, DbExplorerRepo, GitStore,
    IntegrationsRepo, IssuesRepo, NewSession, ProductRepo, ReviewsRepo, SessionsRepo, SkillEvalsRepo,
    SqlitePool, SwarmRepo, WorkspacesRepo,
};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use tokio::sync::broadcast;
use tower::ServiceExt; // for `oneshot`

// ---------------------------------------------------------------------------
// Stubs for unused ServerCtx dependencies
// ---------------------------------------------------------------------------

/// Minimal SecretStore that always errors (not called by activity handlers).
struct NoopSecrets;
impl SecretStore for NoopSecrets {
    fn put(&self, _key: &str, _value: &str) -> Result<()> {
        Err(Error::Internal("noop secrets".into()))
    }
    fn get(&self, _key: &str) -> Result<Option<String>> {
        Err(Error::Internal("noop secrets".into()))
    }
    fn delete(&self, _key: &str) -> Result<()> {
        Err(Error::Internal("noop secrets".into()))
    }
}

/// Minimal Spawner that always errors (not called by activity handlers).
struct NoopSpawner;
impl otto_connections::Spawner for NoopSpawner {
    fn spawn_connection<'a>(
        &'a self,
        _ws_id: &'a Id,
        _user_id: &'a Id,
        _conn: &'a otto_core::domain::Connection,
        _spec: otto_pty::CommandSpec,
        _first_command: Option<String>,
        _title: Option<String>,
    ) -> otto_core::auth::BoxFuture<'a, Result<otto_core::domain::Session>> {
        Box::pin(async { Err(Error::Internal("noop spawner".into())) })
    }
}

// ---------------------------------------------------------------------------
// Database pool + fixtures
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
    sqlx::query(
        "INSERT INTO workspace_members (workspace_id, user_id, role) VALUES (?, ?, ?)",
    )
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
        .create(NewSession {
            workspace_id: ws.into(),
            kind: otto_core::domain::SessionKind::Agent,
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

/// Seed one task row for a session to make it appear in summary.
async fn seed_task(pool: &SqlitePool, ws_id: &str, session_id: &str) {
    let task_id = otto_core::new_id();
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO agent_tasks
            (id, session_id, workspace_id, ext_id, title, status, position, created_at, updated_at)
         VALUES (?, ?, ?, NULL, 'task', 'pending', 0, ?, ?)",
    )
    .bind(&task_id)
    .bind(session_id)
    .bind(ws_id)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .expect("seed task");
}

// ---------------------------------------------------------------------------
// Minimal ServerCtx construction for activity-handler tests
// ---------------------------------------------------------------------------

async fn test_ctx(pool: &SqlitePool) -> ServerCtx {
    let (events, _rx) = broadcast::channel(64);
    let secrets: Arc<dyn SecretStore> = Arc::new(NoopSecrets);
    let roles = Arc::new(RbacRoleChecker::new(pool.clone()));
    let repo = SessionsRepo::new(pool.clone());
    let providers = ProviderRegistry::new(None);
    let manager = Arc::new(SessionManager::new(repo, events.clone(), providers));
    let orchestrator = Arc::new(otto_orchestrator::Orchestrator::new("claude"));
    let improve_engine = Arc::new(otto_improve::ImprovementEngine {
        improvements: otto_state::ImprovementsRepo::new(pool.clone()),
        sessions: SessionsRepo::new(pool.clone()),
        workspaces: WorkspacesRepo::new(pool.clone()),
        producer: Arc::new(otto_improve::RealProposalProducer::new(orchestrator.clone())),
        events: events.clone(),
        library_root: PathBuf::from("/tmp/otto-test-lib"),
    });
    let connections = Arc::new(otto_connections::ConnectionsService::new(
        ConnectionsRepo::new(pool.clone()),
        ConnectionSectionsRepo::new(pool.clone()),
        secrets.clone(),
    ));
    let db_explorer = Arc::new(otto_dbviewer::DbViewerService::new(
        ConnectionsRepo::new(pool.clone()),
        secrets.clone(),
        DbExplorerRepo::new(pool.clone()),
    ));
    let brokers = Arc::new(otto_brokers::BrokersService::new(
        otto_state::BrokerClustersRepo::new(pool.clone()),
        otto_state::BrokerClusterSectionsRepo::new(pool.clone()),
        secrets.clone(),
        None,
    ));
    let mcp = Arc::new(otto_mcp::McpService::new(pool.clone(), secrets.clone()));
    let swarm_repo = SwarmRepo::new(pool.clone());
    let swarm = Arc::new(otto_swarm::SwarmService::new(swarm_repo.clone()));
    let product_repo = ProductRepo::new(pool.clone());
    let product = Arc::new(otto_product::ProductService::new(
        product_repo.clone(),
        IssuesRepo::new(pool.clone()),
        secrets.clone(),
    ));
    let usage = otto_usage::UsageEngine::start(
        otto_usage::UsageConfig::default(),
        PathBuf::from("/tmp/otto-test-usage"),
    )
    .await;
    let context_library = otto_context::Library::new("/tmp/otto-test-ctxlib");

    ServerCtx {
        pool: pool.clone(),
        secrets,
        events: events.clone(),
        authenticator: Arc::new(otto_rbac::RbacAuthenticator::new(pool.clone())),
        roles,
        auth_cache: otto_rbac::AuthCache::new(),
        version: "test".into(),
        base_url: "http://127.0.0.1:0".into(),
        data_dir: PathBuf::from("/tmp/otto-test"),
        plugins: Arc::new(otto_server::plugins::PluginManager::new(
            otto_state::PluginsRepo::new(pool.clone()),
            PathBuf::from("/tmp/otto-test-plugins"),
            PathBuf::from("/tmp/otto-test"),
            "http://127.0.0.1:7700/api/v1/plugin-host".into(),
        )),
        manager,
        workspaces: WorkspacesRepo::new(pool.clone()),
        connections,
        db_explorer,
        db_assist: otto_server::db_assist::new_registry(),
        brokers,
        mcp,
        spawner: Arc::new(NoopSpawner),
        git_store: GitStore::new(pool.clone()),
        issues_store: IssuesRepo::new(pool.clone()),
        integrations_store: IntegrationsRepo::new(pool.clone()),
        channel_bridge: None,
        reviews_store: ReviewsRepo::new(pool.clone()),
        findings_store: otto_state::ReviewFindingsRepo::new(pool.clone()),
        finding_events_store: otto_state::FindingEventsRepo::new(pool.clone()),
        repo_rules_store: otto_state::RepoRulesRepo::new(pool.clone()),
        proof_packs_store: otto_state::ReviewProofPacksRepo::new(pool.clone()),
        skill_evals_store: SkillEvalsRepo::new(pool.clone()),
        golden_tasks_store: otto_state::GoldenTasksRepo::new(pool.clone()),
        eval_matrices_store: otto_state::EvalMatricesRepo::new(pool.clone()),
        skill_eval_cancels: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        review_cancels: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        orchestrator,
        improve_engine,
        context_library,
        usage,
        product,
        product_repo,
        attachment_repo: otto_state::ProductAttachmentRepo::new(pool.clone()),
        discovery_repo: otto_state::ProductDiscoveryRepo::new(pool.clone()),
        refinement_repo: otto_state::ProductRefinementRepo::new(pool.clone()),
        mockup_repo: otto_state::ProductMockupRepo::new(pool.clone()),
        discovery_chat_repo: otto_state::DiscoveryChatRepo::new(pool.clone()),
        canvas_repo: otto_state::CanvasRepo::new(pool.clone()),
        product_agent_cancels: otto_server::product_run::new_cancel_registry(),
        memory: Arc::new(otto_memory::MemoryService::with_defaults(pool.clone())),
        swarm,
        swarm_repo,
        swarm_coords: otto_server::swarm_runtime::new_registry(),
        swarm_run_cancels: otto_server::swarm_run::new_cancel_registry(),
        goal_loops_repo: otto_state::GoalLoopsRepo::new(pool.clone()),
        goal_loops: otto_server::goal_loop::new_registry(),
        workgraph: Arc::new(otto_workgraph::WorkGraphService::new(
            otto_state::WorkGraphRepo::new(pool.clone()),
            events.clone(),
        )),
        scheduled_tasks: otto_state::ScheduledTasksRepo::new(pool.clone()),
        proof_repo: otto_state::ProofRepo::new(pool.clone()),
        proof_locks: otto_server::proof::new_locks(),
        runs: otto_state::RunsRepo::new(pool.clone()),
        runs_engine: otto_server::run_engine::RunEngine::new(),
    }
}

/// Build a minimal router exposing only the activity endpoints under their
/// production paths, using the given `ServerCtx`.
fn activity_router(ctx: ServerCtx) -> Router {
    use axum::routing::get;
    use otto_server::routes::activity::{
        append_trail, list_tasks, list_trail, put_tasks, workspace_summary,
    };
    Router::new()
        .route(
            "/workspaces/{wid}/sessions/{sid}/trail",
            get(list_trail).post(append_trail),
        )
        .route(
            "/workspaces/{wid}/sessions/{sid}/tasks",
            get(list_tasks).put(put_tasks),
        )
        .route(
            "/workspaces/{wid}/activity/summary",
            get(workspace_summary),
        )
        .with_state(ctx)
}

/// Issue a request with a JSON body as `caller` and return the status code.
async fn send_json_as(
    app: &Router,
    caller: &User,
    method: Method,
    uri: &str,
    body: serde_json::Value,
) -> StatusCode {
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    req.extensions_mut().insert(AuthUser(caller.clone()));
    app.clone().oneshot(req).await.unwrap().status()
}

/// Issue a GET as `caller` and return the status code.
async fn get_as(app: &Router, caller: &User, uri: &str) -> StatusCode {
    let mut req = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .body(Body::empty())
        .unwrap();
    req.extensions_mut().insert(AuthUser(caller.clone()));
    app.clone().oneshot(req).await.unwrap().status()
}

/// GET the summary endpoint and deserialize the body (for non-403 callers).
async fn get_summary(
    app: &Router,
    caller: &User,
    ws: &str,
) -> Vec<serde_json::Value> {
    let mut req = Request::builder()
        .method(Method::GET)
        .uri(format!("/workspaces/{ws}/activity/summary"))
        .body(Body::empty())
        .unwrap();
    req.extensions_mut().insert(AuthUser(caller.clone()));
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "summary must be 200");
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&body).expect("decode summary")
}

// ---------------------------------------------------------------------------
// Tests — #L16 trail isolation
// ---------------------------------------------------------------------------

/// #L16: a non-owner workspace Editor gets 403 on another user's trail.
#[tokio::test]
async fn non_owner_editor_forbidden_on_trail() {
    let pool = mem_pool().await;
    seed_user(&pool, "alice", false).await;
    seed_user(&pool, "bob", false).await;
    seed_workspace(&pool, "ws1").await;
    set_member(&pool, "ws1", "alice", "editor").await;
    set_member(&pool, "ws1", "bob", "editor").await;

    let repo = SessionsRepo::new(pool.clone());
    let sid = insert_session(&repo, "ws1", "alice").await; // alice owns it

    let ctx = test_ctx(&pool).await;
    let app = activity_router(ctx);
    let bob = user("bob", false);

    let status = get_as(&app, &bob, &format!("/workspaces/ws1/sessions/{sid}/trail")).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "bob (editor, non-owner) must get 403 on alice's trail, got {status}"
    );
}

/// #L16 complement: the owner, a workspace Admin, and root can read the trail.
#[tokio::test]
async fn owner_admin_root_can_read_trail() {
    let pool = mem_pool().await;
    seed_user(&pool, "alice", false).await;
    seed_user(&pool, "carol", false).await;
    seed_workspace(&pool, "ws1").await;
    set_member(&pool, "ws1", "alice", "viewer").await;
    set_member(&pool, "ws1", "carol", "admin").await;

    let repo = SessionsRepo::new(pool.clone());
    let sid = insert_session(&repo, "ws1", "alice").await; // alice owns it
    let ctx = test_ctx(&pool).await;
    let app = activity_router(ctx);

    for (label, u) in [
        ("owner alice", user("alice", false)),
        ("ws-admin carol", user("carol", false)),
        ("root", user("root_usr", true)),
    ] {
        let status = get_as(&app, &u, &format!("/workspaces/ws1/sessions/{sid}/trail")).await;
        assert_ne!(
            status,
            StatusCode::FORBIDDEN,
            "{label} must not be forbidden on trail, got {status}"
        );
    }
}

// ---------------------------------------------------------------------------
// Tests — #L17 tasks isolation
// ---------------------------------------------------------------------------

/// #L17: a non-owner workspace Editor gets 403 on another user's tasks.
#[tokio::test]
async fn non_owner_editor_forbidden_on_tasks() {
    let pool = mem_pool().await;
    seed_user(&pool, "alice", false).await;
    seed_user(&pool, "bob", false).await;
    seed_workspace(&pool, "ws1").await;
    set_member(&pool, "ws1", "alice", "editor").await;
    set_member(&pool, "ws1", "bob", "editor").await;

    let repo = SessionsRepo::new(pool.clone());
    let sid = insert_session(&repo, "ws1", "alice").await; // alice owns it

    let ctx = test_ctx(&pool).await;
    let app = activity_router(ctx);
    let bob = user("bob", false);

    let status = get_as(&app, &bob, &format!("/workspaces/ws1/sessions/{sid}/tasks")).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "bob (editor, non-owner) must get 403 on alice's tasks, got {status}"
    );
}

/// #L17 complement: the owner, a workspace Admin, and root can read tasks.
#[tokio::test]
async fn owner_admin_root_can_read_tasks() {
    let pool = mem_pool().await;
    seed_user(&pool, "alice", false).await;
    seed_user(&pool, "carol", false).await;
    seed_workspace(&pool, "ws1").await;
    set_member(&pool, "ws1", "alice", "viewer").await;
    set_member(&pool, "ws1", "carol", "admin").await;

    let repo = SessionsRepo::new(pool.clone());
    let sid = insert_session(&repo, "ws1", "alice").await; // alice owns it
    let ctx = test_ctx(&pool).await;
    let app = activity_router(ctx);

    for (label, u) in [
        ("owner alice", user("alice", false)),
        ("ws-admin carol", user("carol", false)),
        ("root", user("root_usr", true)),
    ] {
        let status = get_as(&app, &u, &format!("/workspaces/ws1/sessions/{sid}/tasks")).await;
        assert_ne!(
            status,
            StatusCode::FORBIDDEN,
            "{label} must not be forbidden on tasks, got {status}"
        );
    }
}

// ---------------------------------------------------------------------------
// Tests — #L18 summary isolation
// ---------------------------------------------------------------------------

/// #L18: a non-admin caller's summary excludes other users' sessions.
#[tokio::test]
async fn non_admin_summary_excludes_other_users_sessions() {
    let pool = mem_pool().await;
    seed_user(&pool, "alice", false).await;
    seed_user(&pool, "bob", false).await;
    seed_workspace(&pool, "ws1").await;
    set_member(&pool, "ws1", "alice", "editor").await;
    set_member(&pool, "ws1", "bob", "editor").await;

    let repo = SessionsRepo::new(pool.clone());
    let alice_sid = insert_session(&repo, "ws1", "alice").await;
    let bob_sid = insert_session(&repo, "ws1", "bob").await;

    // Give both sessions tasks so they appear in the summary.
    seed_task(&pool, "ws1", &alice_sid).await;
    seed_task(&pool, "ws1", &bob_sid).await;

    let ctx = test_ctx(&pool).await;
    let app = activity_router(ctx);

    // Bob (editor, non-admin) should see only his own session in the summary.
    let bob = user("bob", false);
    let bob_summary = get_summary(&app, &bob, "ws1").await;
    let bob_session_ids: Vec<&str> = bob_summary
        .iter()
        .map(|s| s["session_id"].as_str().unwrap())
        .collect();
    assert_eq!(
        bob_session_ids,
        vec![bob_sid.as_str()],
        "bob's summary must contain only his session"
    );
    assert!(
        !bob_session_ids.contains(&alice_sid.as_str()),
        "alice's session must not appear in bob's summary"
    );
}

/// #L18 complement: a workspace Admin and root see the full summary.
#[tokio::test]
async fn admin_and_root_see_full_summary() {
    let pool = mem_pool().await;
    seed_user(&pool, "alice", false).await;
    seed_user(&pool, "bob", false).await;
    seed_user(&pool, "carol", false).await;
    seed_workspace(&pool, "ws1").await;
    set_member(&pool, "ws1", "alice", "editor").await;
    set_member(&pool, "ws1", "bob", "editor").await;
    set_member(&pool, "ws1", "carol", "admin").await;

    let repo = SessionsRepo::new(pool.clone());
    let alice_sid = insert_session(&repo, "ws1", "alice").await;
    let bob_sid = insert_session(&repo, "ws1", "bob").await;
    seed_task(&pool, "ws1", &alice_sid).await;
    seed_task(&pool, "ws1", &bob_sid).await;

    let ctx = test_ctx(&pool).await;
    let app = activity_router(ctx);

    // Carol (ws-admin) sees both sessions.
    let carol = user("carol", false);
    let carol_summary = get_summary(&app, &carol, "ws1").await;
    assert_eq!(
        carol_summary.len(),
        2,
        "workspace admin must see all sessions in summary"
    );

    // Root (any user with is_root=true) sees both.
    let root = user("root_usr", true);
    let root_summary = get_summary(&app, &root, "ws1").await;
    assert_eq!(
        root_summary.len(),
        2,
        "root must see all sessions in summary"
    );
}

// ---------------------------------------------------------------------------
// Tests — write-path ownership (append_trail / put_tasks)
//
// The read handlers were owner-gated; these prove the WRITE handlers are too,
// so a non-owner workspace Editor can't inject trail entries into, or
// wipe/overwrite the task list of, another user's session.
// ---------------------------------------------------------------------------

/// A non-owner workspace Editor gets 403 on `POST .../trail` and `PUT .../tasks`.
#[tokio::test]
async fn non_owner_editor_forbidden_on_trail_and_task_writes() {
    let pool = mem_pool().await;
    seed_user(&pool, "alice", false).await;
    seed_user(&pool, "bob", false).await;
    seed_workspace(&pool, "ws1").await;
    set_member(&pool, "ws1", "alice", "editor").await;
    set_member(&pool, "ws1", "bob", "editor").await;

    let repo = SessionsRepo::new(pool.clone());
    let sid = insert_session(&repo, "ws1", "alice").await; // alice owns it

    let ctx = test_ctx(&pool).await;
    let app = activity_router(ctx);
    let bob = user("bob", false);

    let trail_status = send_json_as(
        &app,
        &bob,
        Method::POST,
        &format!("/workspaces/ws1/sessions/{sid}/trail"),
        serde_json::json!({ "summary": "injected by bob" }),
    )
    .await;
    assert_eq!(
        trail_status,
        StatusCode::FORBIDDEN,
        "bob (editor, non-owner) must get 403 appending to alice's trail, got {trail_status}"
    );

    let tasks_status = send_json_as(
        &app,
        &bob,
        Method::PUT,
        &format!("/workspaces/ws1/sessions/{sid}/tasks"),
        serde_json::json!({ "tasks": [] }),
    )
    .await;
    assert_eq!(
        tasks_status,
        StatusCode::FORBIDDEN,
        "bob (editor, non-owner) must get 403 overwriting alice's tasks, got {tasks_status}"
    );
}

/// The owner (and root) can write their own session's trail/tasks (not 403).
#[tokio::test]
async fn owner_and_root_can_write_trail_and_tasks() {
    let pool = mem_pool().await;
    seed_user(&pool, "alice", false).await;
    seed_workspace(&pool, "ws1").await;
    set_member(&pool, "ws1", "alice", "editor").await;

    let repo = SessionsRepo::new(pool.clone());
    let sid = insert_session(&repo, "ws1", "alice").await; // alice owns it

    let ctx = test_ctx(&pool).await;
    let app = activity_router(ctx);

    for (label, u) in [
        ("owner alice", user("alice", false)),
        ("root", user("root_usr", true)),
    ] {
        let trail_status = send_json_as(
            &app,
            &u,
            Method::POST,
            &format!("/workspaces/ws1/sessions/{sid}/trail"),
            serde_json::json!({ "summary": "own note" }),
        )
        .await;
        assert_ne!(
            trail_status,
            StatusCode::FORBIDDEN,
            "{label} must not be forbidden appending to their own trail, got {trail_status}"
        );

        let tasks_status = send_json_as(
            &app,
            &u,
            Method::PUT,
            &format!("/workspaces/ws1/sessions/{sid}/tasks"),
            serde_json::json!({ "tasks": [] }),
        )
        .await;
        assert_ne!(
            tasks_status,
            StatusCode::FORBIDDEN,
            "{label} must not be forbidden overwriting their own tasks, got {tasks_status}"
        );
    }
}
