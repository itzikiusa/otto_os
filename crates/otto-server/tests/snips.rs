//! Integration tests for the snips feature (screenshot → annotate → clipboard):
//!   POST   /api/v1/snips             (base64 PNG upload; doubles as the E2E seed path)
//!   POST   /api/v1/snips/capture     (interactive `screencapture -i`; test seam via
//!                                     `OTTO_SNIP_CAPTURE_CMD`)
//!   GET    /api/v1/snips
//!   GET    /api/v1/snips/{id}/image
//!   GET    /api/v1/snips/{id}/annotated
//!   POST   /api/v1/snips/{id}/annotated
//!   POST   /api/v1/snips/{id}/copy
//!   DELETE /api/v1/snips/{id}
//!
//! Uses the same "real minimal ServerCtx" harness as `canvas_refs_api.rs`
//! (stub secrets/spawner, in-memory sqlite, `tower::ServiceExt::oneshot` with
//! the `AuthUser` extension injected as the production auth middleware does),
//! with a per-test temp `data_dir` so the file-backed snip store and the
//! `clipboard-last.png` sink can be asserted on disk. `OTTO_E2E=1` is set so
//! the clipboard writer never touches the real macOS pasteboard; the two
//! capture tests serialize `OTTO_SNIP_CAPTURE_CMD` mutation behind a Mutex
//! (env vars are process-global).

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use axum::body::Body;
use axum::extract::Request;
use axum::http::{Method, StatusCode};
use axum::Router;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
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
    ConnectionSectionsRepo, ConnectionsRepo, DbExplorerRepo, GitStore, IntegrationsRepo,
    IssuesRepo, ProductRepo, ReviewsRepo, SessionsRepo, SkillEvalsRepo, SqlitePool, SwarmRepo,
    WorkspacesRepo,
};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use tempfile::TempDir;
use tokio::sync::broadcast;
use tower::ServiceExt; // for `oneshot`

/// 60×40 solid-red PNG (132 bytes) — the shared fixture for upload/capture tests.
const PNG_B64: &str = "iVBORw0KGgoAAAANSUhEUgAAADwAAAAoCAYAAACiu5n/AAAAS0lEQVR4nO3PQQ0AIBDAMIThXwVeQAbJro/913X2vpNavweAgYGBgYGB5wRcD7gecD3gesD1gOsB1wOuB1wPuB5wPeB6wPWA6wHXe+cRy1yXJ5HmAAAAAElFTkSuQmCC";
/// Same dimensions, solid blue — a byte-distinct valid PNG for the annotated slot.
const PNG2_B64: &str = "iVBORw0KGgoAAAANSUhEUgAAADwAAAAoCAYAAACiu5n/AAAAS0lEQVR4nO3PMQ0AIADAMOSgCe14ARkko8f+dcy1z0+N1wPAwMDAwMDA/wRcD7gecD3gesD1gOsB1wOuB1wPuB5wPeB6wPWA6wHXuyw3KSsam61wAAAAAElFTkSuQmCC";

fn fixture_png() -> Vec<u8> {
    B64.decode(PNG_B64).expect("decode fixture")
}

fn fixture_png2() -> Vec<u8> {
    B64.decode(PNG2_B64).expect("decode fixture2")
}

/// Env-mutation guard: capture tests set `OTTO_SNIP_CAPTURE_CMD` process-wide.
/// tokio's Mutex so the guard may be held across the request `.await`s.
static CAPTURE_ENV: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
fn capture_env_lock() -> &'static tokio::sync::Mutex<()> {
    CAPTURE_ENV.get_or_init(|| tokio::sync::Mutex::new(()))
}

// ---------------------------------------------------------------------------
// Stubs (mirrors canvas_refs_api.rs)
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

fn user(id: &str) -> User {
    User {
        id: id.into(),
        username: id.into(),
        display_name: id.into(),
        is_root: false,
        disabled: false,
        created_at: Utc::now(),
    }
}

async fn test_ctx(pool: &SqlitePool, data_dir: PathBuf) -> ServerCtx {
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
        library_root: PathBuf::from("/tmp/otto-test-lib-snips"),
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
        PathBuf::from("/tmp/otto-test-usage-snips"),
    )
    .await;
    let context_library = otto_context::Library::new("/tmp/otto-test-ctxlib-snips");

    ServerCtx {
        pool: pool.clone(),
        secrets,
        events: events.clone(),
        authenticator: Arc::new(otto_rbac::RbacAuthenticator::new(pool.clone())),
        roles,
        auth_cache: otto_rbac::AuthCache::new(),
        version: "test".into(),
        base_url: "http://127.0.0.1:0".into(),
        data_dir: data_dir.clone(),
        plugins: Arc::new(otto_server::plugins::PluginManager::new(
            otto_state::PluginsRepo::new(pool.clone()),
            PathBuf::from("/tmp/otto-test-plugins-snips"),
            data_dir,
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
    }
}

/// Build a minimal router exposing the snips endpoints under production paths.
fn snips_router(ctx: ServerCtx) -> Router {
    Router::new()
        .merge(otto_server::routes::snips::snips_routes())
        .with_state(ctx)
}

/// One-stop test app: temp data dir + router. Sets `OTTO_E2E=1` so clipboard
/// writes hit only the file sink (never the real pasteboard).
async fn test_app() -> (TempDir, PathBuf, Router) {
    std::env::set_var("OTTO_E2E", "1");
    let tmp = TempDir::new().expect("tempdir");
    let data_dir = tmp.path().to_path_buf();
    let pool = mem_pool().await;
    let ctx = test_ctx(&pool, data_dir.clone()).await;
    let app = snips_router(ctx);
    (tmp, data_dir, app)
}

async fn send(
    app: &Router,
    method: Method,
    uri: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, Vec<u8>) {
    let b = Request::builder().method(method).uri(uri);
    let req = match body {
        Some(v) => b
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&v).unwrap()))
            .unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let mut req = req;
    req.extensions_mut().insert(AuthUser(user("alice")));
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let body = resp.into_body().collect().await.unwrap().to_bytes().to_vec();
    (status, body)
}

fn json(body: &[u8]) -> serde_json::Value {
    serde_json::from_slice(body).unwrap_or(serde_json::Value::Null)
}

// ---------------------------------------------------------------------------
// Upload + retrieval + clipboard sink
// ---------------------------------------------------------------------------

#[tokio::test]
async fn upload_roundtrip_and_clipboard_sink() {
    let (_tmp, data_dir, app) = test_app().await;

    let (status, body) = send(
        &app,
        Method::POST,
        "/snips",
        Some(serde_json::json!({ "data_b64": PNG_B64, "filename": "shot.png" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "upload: {}", String::from_utf8_lossy(&body));
    let snip = json(&body);
    let id = snip["id"].as_str().expect("id").to_string();
    assert_eq!(snip["width"], 60);
    assert_eq!(snip["height"], 40);
    assert_eq!(snip["source"], "upload");
    assert_eq!(snip["has_annotated"], false);

    // R2: the upload was automatically "copied" — sink file byte-equals the fixture.
    let sink = std::fs::read(data_dir.join("snips/clipboard-last.png")).expect("sink written");
    assert_eq!(sink, fixture_png());

    // Original PNG served back verbatim with the right headers.
    let (status, body) = send(&app, Method::GET, &format!("/snips/{id}/image"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, fixture_png());

    // Listed.
    let (status, body) = send(&app, Method::GET, "/snips", None).await;
    assert_eq!(status, StatusCode::OK);
    let list = json(&body);
    assert_eq!(list.as_array().map(|a| a.len()), Some(1));
    assert_eq!(list[0]["id"], id.as_str());
}

#[tokio::test]
async fn upload_rejects_non_png_and_empty() {
    let (_tmp, data_dir, app) = test_app().await;

    // JPEG magic bytes → 400.
    let jpeg = B64.encode([0xFFu8, 0xD8, 0xFF, 0xE0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let (status, _) = send(
        &app,
        Method::POST,
        "/snips",
        Some(serde_json::json!({ "data_b64": jpeg })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Garbage base64 → 400.
    let (status, _) = send(
        &app,
        Method::POST,
        "/snips",
        Some(serde_json::json!({ "data_b64": "!!!not-base64!!!" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Nothing was copied to the clipboard sink.
    assert!(!data_dir.join("snips/clipboard-last.png").exists());
}

#[tokio::test]
async fn invalid_ids_are_not_found_and_do_not_traverse() {
    let (_tmp, _data_dir, app) = test_app().await;
    // Encoded traversal (axum rejects raw `../` in paths at the routing layer;
    // the encoded form reaches the handler and must fail the id check).
    for bad in ["..%2F..%2Fetc%2Fpasswd", "AB", "a%20b", "x".repeat(65).as_str()] {
        let (status, _) = send(&app, Method::GET, &format!("/snips/{bad}/image"), None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "id {bad:?} must 404");
    }
    // Well-formed but unknown id → 404.
    let (status, _) = send(&app, Method::GET, "/snips/0123456789abcdef/image", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// Annotated save + copy preference + delete
// ---------------------------------------------------------------------------

#[tokio::test]
async fn annotated_save_updates_clipboard_and_copy_prefers_annotated() {
    let (_tmp, data_dir, app) = test_app().await;

    let (_, body) = send(
        &app,
        Method::POST,
        "/snips",
        Some(serde_json::json!({ "data_b64": PNG_B64 })),
    )
    .await;
    let id = json(&body)["id"].as_str().unwrap().to_string();

    let (status, body) = send(
        &app,
        Method::POST,
        &format!("/snips/{id}/annotated"),
        Some(serde_json::json!({ "data_b64": PNG2_B64 })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "save annotated: {}", String::from_utf8_lossy(&body));
    assert_eq!(json(&body)["copied"], true);

    // R4: the clipboard sink now holds the ANNOTATED bytes, not the original.
    let sink = std::fs::read(data_dir.join("snips/clipboard-last.png")).expect("sink");
    assert_eq!(sink, fixture_png2());

    // Annotated file exists and is served.
    let (status, got) = send(&app, Method::GET, &format!("/snips/{id}/annotated"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(got, fixture_png2());

    // List reflects has_annotated.
    let (_, body) = send(&app, Method::GET, "/snips", None).await;
    assert_eq!(json(&body)[0]["has_annotated"], true);

    // Copy prefers the annotated file: wipe the sink, re-copy, sink reappears.
    std::fs::remove_file(data_dir.join("snips/clipboard-last.png")).unwrap();
    let (status, body) = send(&app, Method::POST, &format!("/snips/{id}/copy"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json(&body)["copied"], true);
    let sink = std::fs::read(data_dir.join("snips/clipboard-last.png")).expect("sink");
    assert_eq!(sink, fixture_png2(), "copy must prefer the annotated bytes");

    // Delete removes everything.
    let (status, _) = send(&app, Method::DELETE, &format!("/snips/{id}"), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = send(&app, Method::GET, &format!("/snips/{id}/image"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, body) = send(&app, Method::GET, "/snips", None).await;
    assert_eq!(json(&body).as_array().map(|a| a.len()), Some(0));
}

#[tokio::test]
async fn annotated_rejects_unknown_snip() {
    let (_tmp, _data_dir, app) = test_app().await;
    let (status, _) = send(
        &app,
        Method::POST,
        "/snips/0123456789abcdef/annotated",
        Some(serde_json::json!({ "data_b64": PNG_B64 })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// Capture (via the OTTO_SNIP_CAPTURE_CMD test seam)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn capture_success_creates_snip_and_copies() {
    let _guard = capture_env_lock().lock().await;
    let (_tmp, data_dir, app) = test_app().await;

    // Fixture source the fake "screencapture" copies from.
    let src = data_dir.join("fixture.png");
    std::fs::write(&src, fixture_png()).unwrap();
    std::env::set_var(
        "OTTO_SNIP_CAPTURE_CMD",
        format!("cp {} \"$1\"", src.display()),
    );

    let (status, body) = send(&app, Method::POST, "/snips/capture", Some(serde_json::json!({}))).await;
    std::env::remove_var("OTTO_SNIP_CAPTURE_CMD");

    assert_eq!(status, StatusCode::OK, "capture: {}", String::from_utf8_lossy(&body));
    let resp = json(&body);
    assert_eq!(resp["cancelled"], false);
    let snip = &resp["snip"];
    assert_eq!(snip["source"], "capture");
    assert_eq!(snip["width"], 60);
    assert_eq!(snip["height"], 40);

    // R2: capture auto-copied.
    let sink = std::fs::read(data_dir.join("snips/clipboard-last.png")).expect("sink");
    assert_eq!(sink, fixture_png());
}

#[tokio::test]
async fn capture_cancel_reports_cancelled() {
    let _guard = capture_env_lock().lock().await;
    let (_tmp, data_dir, app) = test_app().await;

    // Fake screencapture that writes nothing and exits 1 (Esc behavior).
    std::env::set_var("OTTO_SNIP_CAPTURE_CMD", "exit 1");
    let (status, body) = send(&app, Method::POST, "/snips/capture", Some(serde_json::json!({}))).await;
    std::env::remove_var("OTTO_SNIP_CAPTURE_CMD");

    assert_eq!(status, StatusCode::OK);
    let resp = json(&body);
    assert_eq!(resp["cancelled"], true);
    assert!(resp["snip"].is_null());
    assert!(!data_dir.join("snips/clipboard-last.png").exists());

    // Nothing listed.
    let (_, body) = send(&app, Method::GET, "/snips", None).await;
    assert_eq!(json(&body).as_array().map(|a| a.len()), Some(0));
}
