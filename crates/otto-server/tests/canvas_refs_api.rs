//! Integration tests for session ↔ Canvas scene references (Task B):
//!   GET    /api/v1/sessions/{sid}/canvas-refs
//!   POST   /api/v1/sessions/{sid}/canvas-refs
//!   DELETE /api/v1/sessions/{sid}/canvas-refs/{scene_id}
//!
//! Uses the same "real minimal ServerCtx" harness as `activity_isolation.rs`:
//! a real `SessionManager` (backed by an in-memory sqlite pool, sessions
//! inserted directly via `SessionsRepo` rather than spawned — no PTY needed)
//! plus stub `SecretStore`/`Spawner` for the handlers this test never touches.
//! `tower::ServiceExt::oneshot` drives requests through the real handlers with
//! the `AuthUser` extension injected exactly as the production auth
//! middleware does.

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
    CanvasRepo, ConnectionSectionsRepo, ConnectionsRepo, DbExplorerRepo, GitStore,
    IntegrationsRepo, IssuesRepo, NewScene, NewSession, ProductRepo, ReviewsRepo, SessionsRepo,
    SkillEvalsRepo, SqlitePool, SwarmRepo, WorkspacesRepo,
};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use tokio::sync::broadcast;
use tower::ServiceExt; // for `oneshot`

// ---------------------------------------------------------------------------
// Stubs for unused ServerCtx dependencies (mirrors activity_isolation.rs)
// ---------------------------------------------------------------------------

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
    let opts = SqliteConnectOptions::new().in_memory(true).foreign_keys(true);
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

/// Create a canvas scene in the given workspace and return its id.
async fn insert_scene(repo: &CanvasRepo, ws: &str, created_by: &str, title: &str) -> Id {
    let scene = repo
        .create(NewScene {
            workspace_id: ws.into(),
            story_id: None,
            title: title.into(),
            doc_json: r#"{"schema":1,"nodes":[],"edges":[],"slides":[]}"#.into(),
            provider: "claude".into(),
            section: None,
            created_by: created_by.into(),
        })
        .await
        .expect("insert scene");
    scene.id
}

// ---------------------------------------------------------------------------
// Minimal ServerCtx construction (mirrors activity_isolation.rs::test_ctx)
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
        library_root: PathBuf::from("/tmp/otto-test-lib-canvas-refs"),
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
        PathBuf::from("/tmp/otto-test-usage-canvas-refs"),
    )
    .await;
    let context_library = otto_context::Library::new("/tmp/otto-test-ctxlib-canvas-refs");

    ServerCtx {
        pool: pool.clone(),
        secrets,
        events: events.clone(),
        authenticator: Arc::new(otto_rbac::RbacAuthenticator::new(pool.clone())),
        roles,
        auth_cache: otto_rbac::AuthCache::new(),
        version: "test".into(),
        base_url: "http://127.0.0.1:0".into(),
        data_dir: PathBuf::from("/tmp/otto-test-canvas-refs"),
        plugins: Arc::new(otto_server::plugins::PluginManager::new(
            otto_state::PluginsRepo::new(pool.clone()),
            PathBuf::from("/tmp/otto-test-plugins-canvas-refs"),
            PathBuf::from("/tmp/otto-test-canvas-refs"),
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
        wf_skip_current: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
        reviews_store: ReviewsRepo::new(pool.clone()),
        findings_store: otto_state::ReviewFindingsRepo::new(pool.clone()),
        finding_events_store: otto_state::FindingEventsRepo::new(pool.clone()),
        repo_rules_store: otto_state::RepoRulesRepo::new(pool.clone()),
        proof_packs_store: otto_state::ReviewProofPacksRepo::new(pool.clone()),
        skill_evals_store: SkillEvalsRepo::new(pool.clone()),
        golden_tasks_store: otto_state::GoldenTasksRepo::new(pool.clone()),
        eval_matrices_store: otto_state::EvalMatricesRepo::new(pool.clone()),
        skill_eval_cancels: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        skill_reviews_store: otto_state::SkillReviewsRepo::new(pool.clone()),
        skill_review_cancels: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
review_agent_cancels: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
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
        design_jobs: otto_server::design_blender::new_job_registry(),
        memory: Arc::new(otto_memory::MemoryService::with_defaults(pool.clone())),
        vault: Arc::new(otto_vault::VaultEngine::new(pool.clone())),
        vault_docs_runs: otto_server::vault_docs_agent::new_run_registry(),
        vault_docs_refine: otto_server::vault_docs_agent::new_refine_registry(),
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
        browser_tabs: otto_state::BrowserTabsRepo::new(pool.clone()),
        browser_annotations: otto_state::BrowserAnnotationsRepo::new(pool.clone()),
        browser_credentials: otto_state::BrowserCredentialsRepo::new(pool.clone()),
        browser: Arc::new(otto_server::routes::browser::BrowserEngineHandle::new(
            None,
            PathBuf::from("/tmp/otto-test-canvas-refs"),
        )),
    }
}

/// Build a minimal router exposing only the canvas-refs endpoints under their
/// production paths.
fn canvas_refs_router(ctx: ServerCtx) -> Router {
    Router::new()
        .merge(otto_server::canvas_refs::canvas_refs_routes())
        .with_state(ctx)
}

async fn get_as(app: &Router, caller: &User, uri: &str) -> (StatusCode, Vec<u8>) {
    let mut req = Request::builder().method(Method::GET).uri(uri).body(Body::empty()).unwrap();
    req.extensions_mut().insert(AuthUser(caller.clone()));
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let body = resp.into_body().collect().await.unwrap().to_bytes().to_vec();
    (status, body)
}

async fn post_json_as(app: &Router, caller: &User, uri: &str, body: serde_json::Value) -> StatusCode {
    let mut req = Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    req.extensions_mut().insert(AuthUser(caller.clone()));
    app.clone().oneshot(req).await.unwrap().status()
}

async fn delete_as(app: &Router, caller: &User, uri: &str) -> StatusCode {
    let mut req = Request::builder().method(Method::DELETE).uri(uri).body(Body::empty()).unwrap();
    req.extensions_mut().insert(AuthUser(caller.clone()));
    app.clone().oneshot(req).await.unwrap().status()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Full happy-path lifecycle: GET empty → POST → GET has 1 → DELETE → GET empty.
#[tokio::test]
async fn list_add_list_remove_list_roundtrip() {
    let pool = mem_pool().await;
    seed_user(&pool, "alice", false).await;
    seed_workspace(&pool, "ws1").await;
    set_member(&pool, "ws1", "alice", "editor").await;

    let sessions_repo = SessionsRepo::new(pool.clone());
    let sid = insert_session(&sessions_repo, "ws1", "alice").await;
    let canvas_repo = CanvasRepo::new(pool.clone());
    let scene_id = insert_scene(&canvas_repo, "ws1", "alice", "My Scene").await;

    let ctx = test_ctx(&pool).await;
    let app = canvas_refs_router(ctx);
    let alice = user("alice", false);

    // GET empty.
    let (status, body) = get_as(&app, &alice, &format!("/sessions/{sid}/canvas-refs")).await;
    assert_eq!(status, StatusCode::OK);
    let refs: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    assert!(refs.is_empty(), "no refs yet");

    // POST attaches the scene.
    let post_status = post_json_as(
        &app,
        &alice,
        &format!("/sessions/{sid}/canvas-refs"),
        serde_json::json!({ "scene_id": scene_id }),
    )
    .await;
    assert_eq!(post_status, StatusCode::NO_CONTENT);

    // GET has 1.
    let (status, body) = get_as(&app, &alice, &format!("/sessions/{sid}/canvas-refs")).await;
    assert_eq!(status, StatusCode::OK);
    let refs: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0]["id"], scene_id.to_string());

    // DELETE detaches it.
    let del_status = delete_as(&app, &alice, &format!("/sessions/{sid}/canvas-refs/{scene_id}")).await;
    assert_eq!(del_status, StatusCode::NO_CONTENT);

    // GET empty again.
    let (status, body) = get_as(&app, &alice, &format!("/sessions/{sid}/canvas-refs")).await;
    assert_eq!(status, StatusCode::OK);
    let refs: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    assert!(refs.is_empty(), "detached");
}

/// A scene from a DIFFERENT workspace cannot be attached to this session.
#[tokio::test]
async fn cross_workspace_attach_is_rejected() {
    let pool = mem_pool().await;
    seed_user(&pool, "alice", false).await;
    seed_workspace(&pool, "ws1").await;
    seed_workspace(&pool, "ws2").await;
    set_member(&pool, "ws1", "alice", "editor").await;
    set_member(&pool, "ws2", "alice", "editor").await;

    let sessions_repo = SessionsRepo::new(pool.clone());
    let sid = insert_session(&sessions_repo, "ws1", "alice").await; // session in ws1
    let canvas_repo = CanvasRepo::new(pool.clone());
    let other_scene_id = insert_scene(&canvas_repo, "ws2", "alice", "Other WS Scene").await; // scene in ws2

    let ctx = test_ctx(&pool).await;
    let app = canvas_refs_router(ctx);
    let alice = user("alice", false);

    let post_status = post_json_as(
        &app,
        &alice,
        &format!("/sessions/{sid}/canvas-refs"),
        serde_json::json!({ "scene_id": other_scene_id }),
    )
    .await;
    assert_eq!(
        post_status,
        StatusCode::NOT_FOUND,
        "attaching a scene from a different workspace must be rejected"
    );

    // Confirm nothing was attached.
    let (status, body) = get_as(&app, &alice, &format!("/sessions/{sid}/canvas-refs")).await;
    assert_eq!(status, StatusCode::OK);
    let refs: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    assert!(refs.is_empty());
}

/// A workspace Viewer can list refs but cannot attach/detach (403).
#[tokio::test]
async fn viewer_can_list_but_not_mutate() {
    let pool = mem_pool().await;
    seed_user(&pool, "alice", false).await; // owner (editor)
    seed_user(&pool, "vince", false).await; // viewer
    seed_workspace(&pool, "ws1").await;
    set_member(&pool, "ws1", "alice", "editor").await;
    set_member(&pool, "ws1", "vince", "viewer").await;

    let sessions_repo = SessionsRepo::new(pool.clone());
    let sid = insert_session(&sessions_repo, "ws1", "alice").await;
    let canvas_repo = CanvasRepo::new(pool.clone());
    let scene_id = insert_scene(&canvas_repo, "ws1", "alice", "Viewer Test Scene").await;

    let ctx = test_ctx(&pool).await;
    let app = canvas_refs_router(ctx);
    let vince = user("vince", false);

    // Viewer can list (200, empty).
    let (status, _body) = get_as(&app, &vince, &format!("/sessions/{sid}/canvas-refs")).await;
    assert_eq!(status, StatusCode::OK, "viewer must be able to list refs");

    // Viewer cannot attach (403).
    let post_status = post_json_as(
        &app,
        &vince,
        &format!("/sessions/{sid}/canvas-refs"),
        serde_json::json!({ "scene_id": scene_id }),
    )
    .await;
    assert_eq!(post_status, StatusCode::FORBIDDEN, "viewer must not be able to attach a scene");

    // Viewer cannot detach (403), even for a ref that doesn't exist yet — the
    // role check runs before the (idempotent) delete.
    let del_status = delete_as(&app, &vince, &format!("/sessions/{sid}/canvas-refs/{scene_id}")).await;
    assert_eq!(del_status, StatusCode::FORBIDDEN, "viewer must not be able to detach a scene");
}
