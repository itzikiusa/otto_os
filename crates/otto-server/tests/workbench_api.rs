//! HTTP-level integration tests for the Workbench routes
//! (`/workspaces/{ws}/workbench/...`): revision coalescing + checkpoint seal,
//! revisions / diff / restore, per-owner isolation, the trash → permanent
//! delete lifecycle, and image assets.
//!
//! Same "real minimal ServerCtx" harness as `canvas_refs_api.rs` (in-memory
//! sqlite with every migration, stub secrets/spawner); requests go through the
//! real handlers via `tower::ServiceExt::oneshot` with the `AuthUser`
//! extension injected as the production auth middleware does.

use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::Request;
use axum::http::{header, Method, StatusCode};
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
    ConnectionSectionsRepo, ConnectionsRepo, DbExplorerRepo, DbPool, GitStore, IntegrationsRepo,
    IssuesRepo, ProductRepo, ReviewsRepo, SessionsRepo, SkillEvalsRepo, SwarmRepo, WorkspacesRepo,
};
use serde_json::{json, Value};
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

async fn seed_user(pool: &DbPool, id: &str, is_root: bool) {
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

async fn seed_workspace(pool: &DbPool, ws_id: &str) {
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

async fn set_member(pool: &DbPool, ws_id: &str, user_id: &str, role: &str) {
    sqlx::query("INSERT INTO workspace_members (workspace_id, user_id, role) VALUES (?, ?, ?)")
        .bind(ws_id)
        .bind(user_id)
        .bind(role)
        .execute(pool)
        .await
        .expect("set member");
}

// ---------------------------------------------------------------------------
// Minimal ServerCtx construction (mirrors activity_isolation.rs::test_ctx)
// ---------------------------------------------------------------------------

async fn test_ctx(pool: &DbPool) -> ServerCtx {
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
        producer: Arc::new(otto_improve::RealProposalProducer::new(
            orchestrator.clone(),
        )),
        events: events.clone(),
        library_root: PathBuf::from("/tmp/otto-test-lib-workbench-api"),
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
        otto_usage::UsageConfig {
            enabled: false, // This fixture does not exercise metrics or start ClickHouse.
            ..Default::default()
        },
        PathBuf::from("/tmp/otto-test-usage-workbench-api"),
    )
    .await;
    let context_library = otto_context::Library::new("/tmp/otto-test-ctxlib-workbench-api");

    ServerCtx {
        pool: pool.clone(),
        secrets,
        events: events.clone(),
        authenticator: Arc::new(otto_rbac::RbacAuthenticator::new(pool.clone())),
        roles,
        auth_cache: otto_rbac::AuthCache::new(),
        version: "test".into(),
        base_url: "http://127.0.0.1:0".into(),
        data_dir: PathBuf::from("/tmp/otto-test-workbench-api"),
        plugins: Arc::new(otto_server::plugins::PluginManager::new(
            otto_state::PluginsRepo::new(pool.clone()),
            PathBuf::from("/tmp/otto-test-plugins-workbench-api"),
            PathBuf::from("/tmp/otto-test-workbench-api"),
            "http://127.0.0.1:7700/api/v1/plugin-host".into(),
        )),
        manager,
        workspaces: WorkspacesRepo::new(pool.clone()),
        connections,
        db_explorer,
        db_assist: otto_server::db_assist::new_registry(),
        transcript_cache: Default::default(),
        rooms: Default::default(),
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
        ui_bridge: Default::default(),
        browser: Arc::new(otto_server::routes::browser::BrowserEngineHandle::new(
            None,
            PathBuf::from("/tmp/otto-test-workbench-api"),
        )),
    }
}

/// Minimal router exposing only the workbench endpoints at their production paths.
fn workbench_router(ctx: ServerCtx) -> Router {
    Router::new()
        .merge(otto_server::routes::workbench::workbench_routes())
        .with_state(ctx)
}

struct Resp {
    status: StatusCode,
    content_type: Option<String>,
    body: Vec<u8>,
}

impl Resp {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|e| {
            panic!(
                "non-JSON body ({e}) for {}: {}",
                self.status,
                String::from_utf8_lossy(&self.body)
            )
        })
    }
}

async fn send(
    app: &Router,
    caller: &User,
    method: Method,
    uri: &str,
    content_type: Option<&str>,
    body: Vec<u8>,
) -> Resp {
    let mut b = Request::builder().method(method).uri(uri);
    if let Some(ct) = content_type {
        b = b.header("content-type", ct);
    }
    let mut req = b.body(Body::from(body)).unwrap();
    req.extensions_mut().insert(AuthUser(caller.clone()));
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let content_type = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let body = resp
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    Resp {
        status,
        content_type,
        body,
    }
}

async fn get(app: &Router, u: &User, uri: &str) -> Resp {
    send(app, u, Method::GET, uri, None, Vec::new()).await
}

async fn post(app: &Router, u: &User, uri: &str, body: Value) -> Resp {
    let bytes = serde_json::to_vec(&body).unwrap();
    send(app, u, Method::POST, uri, Some("application/json"), bytes).await
}

async fn patch(app: &Router, u: &User, uri: &str, body: Value) -> Resp {
    let bytes = serde_json::to_vec(&body).unwrap();
    send(app, u, Method::PATCH, uri, Some("application/json"), bytes).await
}

async fn delete(app: &Router, u: &User, uri: &str) -> Resp {
    send(app, u, Method::DELETE, uri, None, Vec::new()).await
}

/// One workspace `ws1` with `alice` and `bob` as editors; returns the app.
async fn setup() -> Router {
    let pool = mem_pool().await;
    seed_user(&pool, "alice", false).await;
    seed_user(&pool, "bob", false).await;
    seed_workspace(&pool, "ws1").await;
    set_member(&pool, "ws1", "alice", "editor").await;
    set_member(&pool, "ws1", "bob", "editor").await;
    workbench_router(test_ctx(&pool).await)
}

const DOCS: &str = "/workspaces/ws1/workbench/docs";

async fn create_doc(app: &Router, u: &User, name: &str, content: &str) -> String {
    let r = post(
        app,
        u,
        DOCS,
        json!({ "name": name, "language": "txt", "content": content }),
    )
    .await;
    assert_eq!(
        r.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&r.body)
    );
    let doc = r.json();
    assert_eq!(doc["content"], content);
    assert_eq!(doc["rev"], 1);
    doc["id"].as_str().unwrap().to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Autosaves coalesce into one `auto` revision, an identical-content
/// checkpoint seals it, the next save opens a new one; revisions / revision
/// detail / diff / restore round-trip over HTTP.
#[tokio::test]
async fn revisions_coalesce_seal_diff_and_restore() {
    let app = setup().await;
    let alice = user("alice", false);
    let id = create_doc(&app, &alice, "notes.txt", "a\n").await;
    let doc_uri = format!("{DOCS}/{id}");

    let r = get(&app, &alice, &doc_uri).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.json()["content"], "a\n");

    // Two quick autosaves fold into ONE `auto` revision (seq 2, saves = 2).
    for c in ["b\n", "c\n"] {
        let r = patch(&app, &alice, &doc_uri, json!({ "content": c })).await;
        assert_eq!(r.status, StatusCode::OK);
        assert_eq!(r.json()["rev"], 2);
    }
    let revs = get(&app, &alice, &format!("{doc_uri}/revisions"))
        .await
        .json();
    assert_eq!(revs.as_array().unwrap().len(), 2);
    assert_eq!(revs[0]["seq"], 2);
    assert_eq!(revs[0]["kind"], "auto");
    assert_eq!(revs[0]["saves"], 2);

    // ⌘S on unchanged content seals the burst instead of adding a revision.
    let r = patch(
        &app,
        &alice,
        &doc_uri,
        json!({ "content": "c\n", "checkpoint": true }),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    let revs = get(&app, &alice, &format!("{doc_uri}/revisions"))
        .await
        .json();
    assert_eq!(revs.as_array().unwrap().len(), 2);
    assert_eq!(revs[0]["kind"], "checkpoint");

    // The next autosave starts a new revision.
    let r = patch(&app, &alice, &doc_uri, json!({ "content": "d\n" })).await;
    assert_eq!(r.json()["rev"], 3);
    let revs = get(&app, &alice, &format!("{doc_uri}/revisions"))
        .await
        .json();
    let seqs: Vec<i64> = revs
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["seq"].as_i64().unwrap())
        .collect();
    assert_eq!(seqs, vec![3, 2, 1], "newest first");
    assert_eq!(revs[2]["kind"], "create");

    // Revision detail carries that revision's content.
    let r = get(&app, &alice, &format!("{doc_uri}/revisions/2")).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.json()["content"], "c\n");
    let r = get(&app, &alice, &format!("{doc_uri}/revisions/99")).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);

    // Diff rev 1 → current: `a` removed, `d` added.
    let r = get(&app, &alice, &format!("{doc_uri}/diff?from=1")).await;
    assert_eq!(r.status, StatusCode::OK);
    let diff = r.json();
    assert_eq!(diff["added"], 1);
    assert_eq!(diff["removed"], 1);
    let lines = diff["lines"].as_array().unwrap();
    assert!(lines.iter().any(|l| l["op"] == "del" && l["text"] == "a"));
    assert!(lines.iter().any(|l| l["op"] == "add" && l["text"] == "d"));
    // Between two stored revisions.
    let r = get(&app, &alice, &format!("{doc_uri}/diff?from=2&to=3")).await;
    assert_eq!(r.json()["to"], 3);

    // Restoring rev 1 APPENDS a `restore` revision; nothing is overwritten.
    let r = post(
        &app,
        &alice,
        &format!("{doc_uri}/revisions/1/restore"),
        json!({}),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    let doc = r.json();
    assert_eq!(doc["content"], "a\n");
    assert_eq!(doc["rev"], 4);
    let revs = get(&app, &alice, &format!("{doc_uri}/revisions"))
        .await
        .json();
    assert_eq!(revs.as_array().unwrap().len(), 4);
    assert_eq!(revs[0]["kind"], "restore");
    assert_eq!(revs[0]["restored_from"], 1);
    assert_eq!(get(&app, &alice, &doc_uri).await.json()["content"], "a\n");
}

/// Docs are per user: another editor of the same workspace sees nothing.
#[tokio::test]
async fn docs_are_private_to_their_owner() {
    let app = setup().await;
    let alice = user("alice", false);
    let bob = user("bob", false);
    let id = create_doc(&app, &alice, "mine.sql", "SELECT 1").await;
    let doc_uri = format!("{DOCS}/{id}");

    let r = get(&app, &bob, DOCS).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.json().as_array().unwrap().is_empty());
    assert_eq!(
        get(&app, &bob, &doc_uri).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(&app, &bob, &format!("{doc_uri}/revisions"))
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        patch(&app, &bob, &doc_uri, json!({ "content": "x" }))
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        delete(&app, &bob, &doc_uri).await.status,
        StatusCode::NOT_FOUND
    );
    // Alice's doc is untouched.
    assert_eq!(
        get(&app, &alice, &doc_uri).await.json()["content"],
        "SELECT 1"
    );
    assert_eq!(
        get(&app, &alice, DOCS)
            .await
            .json()
            .as_array()
            .unwrap()
            .len(),
        1
    );

    // A non-member is refused outright.
    let mallory = user("mallory", false);
    let r = get(&app, &mallory, DOCS).await;
    assert!(r.status.is_client_error(), "non-member got {}", r.status);
}

/// Trash is a soft delete; only a TRASHED doc can be purged, and the purge
/// erases its history.
#[tokio::test]
async fn trash_restore_and_permanent_delete() {
    let app = setup().await;
    let alice = user("alice", false);
    let id = create_doc(&app, &alice, "tmp.md", "# hi").await;
    let doc_uri = format!("{DOCS}/{id}");

    // Purging a live doc is refused.
    let r = delete(&app, &alice, &format!("{doc_uri}?permanent=true")).await;
    assert_eq!(r.status, StatusCode::CONFLICT);

    // Soft delete → 200 + the doc with deleted_at.
    let r = delete(&app, &alice, &doc_uri).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.json()["deleted_at"].is_string());
    assert!(get(&app, &alice, DOCS)
        .await
        .json()
        .as_array()
        .unwrap()
        .is_empty());
    let trash = get(&app, &alice, &format!("{DOCS}?trash=true"))
        .await
        .json();
    assert_eq!(trash.as_array().unwrap().len(), 1);
    assert_eq!(trash[0]["id"], id.as_str());
    // History is still readable while trashed; editing is not allowed.
    assert_eq!(
        get(&app, &alice, &format!("{doc_uri}/revisions"))
            .await
            .status,
        StatusCode::OK
    );
    let r = patch(&app, &alice, &doc_uri, json!({ "content": "edit" })).await;
    assert_eq!(r.status, StatusCode::CONFLICT);

    // Restore → back in the list with its content.
    let r = post(&app, &alice, &format!("{doc_uri}/restore"), json!({})).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.json()["deleted_at"].is_null());
    assert_eq!(
        get(&app, &alice, DOCS)
            .await
            .json()
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(get(&app, &alice, &doc_uri).await.json()["content"], "# hi");

    // Trash again, then delete forever: 204, and the doc + history are gone.
    assert_eq!(delete(&app, &alice, &doc_uri).await.status, StatusCode::OK);
    let r = delete(&app, &alice, &format!("{doc_uri}?permanent=true")).await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    assert_eq!(
        get(&app, &alice, &doc_uri).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(&app, &alice, &format!("{doc_uri}/revisions"))
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert!(get(&app, &alice, &format!("{DOCS}?trash=true"))
        .await
        .json()
        .as_array()
        .unwrap()
        .is_empty());
}

/// Images upload as raw bytes (magic-byte sniffed) and come back verbatim.
#[tokio::test]
async fn image_assets_roundtrip_and_reject_non_images() {
    let app = setup().await;
    let alice = user("alice", false);
    let assets = "/workspaces/ws1/workbench/assets";
    let png: Vec<u8> = [b"\x89PNG\r\n\x1a\n".as_slice(), b"\0\0\0\rIHDRfake"].concat();

    let r = send(
        &app,
        &alice,
        Method::POST,
        assets,
        Some("image/png"),
        png.clone(),
    )
    .await;
    assert_eq!(
        r.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&r.body)
    );
    let asset = r.json();
    assert_eq!(asset["mime"], "image/png");
    assert_eq!(asset["size"], png.len());
    let aid = asset["id"].as_str().unwrap().to_string();

    let r = get(&app, &alice, &format!("{assets}/{aid}")).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.content_type.as_deref(), Some("image/png"));
    assert_eq!(r.body, png);

    // Another user can't fetch it.
    let bob = user("bob", false);
    assert_eq!(
        get(&app, &bob, &format!("{assets}/{aid}")).await.status,
        StatusCode::NOT_FOUND
    );

    // Text bytes declared as PNG are refused.
    let r = send(
        &app,
        &alice,
        Method::POST,
        assets,
        Some("image/png"),
        b"definitely not an image".to_vec(),
    )
    .await;
    assert!(r.status.is_client_error(), "got {}", r.status);
}
