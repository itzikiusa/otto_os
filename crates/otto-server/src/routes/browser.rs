//! Browser module: reader/annotate tabs (per-workspace) + DOM annotations +
//! on-demand page fetch, backed by `otto-browser`'s Lightpanda-or-plain-fetch
//! engine.
//!
//! RBAC is two-axis like the rest of the app: `Feature::Browser` gates the
//! feature (View for reads, Edit for writes and `/page`) via `policy.rs`;
//! every handler additionally enforces the workspace-role axis with
//! `require_ws_role`. The flat by-id routes (`/browser/tabs/{id}`,
//! `/browser/annotations/{id}`) load the row first and check the role on its
//! `workspace_id` — the IDOR guard, since the feature axis is workspace-blind.
//!
//! `GET /browser/page` fetches a caller-supplied URL on the daemon's behalf,
//! so it netguard-checks it (`otto_netguard::check_url`) BEFORE it reaches
//! [`otto_browser::BrowserService::page`] — that crate does no SSRF checking
//! of its own (see its crate docs); this route is the caller. Navigating a
//! reader-mode tab (`PATCH .../tabs/{id}` with a new `url`) runs the same
//! fetch pipeline and adopts the fetched page's title, so the tab list never
//! shows a stale/user-typed title for a page the reader actually rendered.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, patch};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use otto_core::domain::WorkspaceRole;
use otto_core::event::Event;
use otto_core::{Error, Id};
use otto_state::{BrowserAnnotation, BrowserTab, NewBrowserAnnotation, NewBrowserTab};

use crate::auth::{require_ws_role, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

pub fn routes() -> Router<ServerCtx> {
    Router::new()
        .route(
            "/workspaces/{wid}/browser/tabs",
            get(list_tabs).post(create_tab),
        )
        .route("/browser/tabs/{id}", patch(update_tab).delete(delete_tab))
        .route("/workspaces/{wid}/browser/page", get(fetch_page))
        .route(
            "/workspaces/{wid}/browser/annotations",
            get(list_annotations).post(create_annotation),
        )
        .route(
            "/browser/annotations/{id}",
            patch(update_annotation).delete(delete_annotation),
        )
}

// ---------------------------------------------------------------------------
// Lazily-started browser engine
// ---------------------------------------------------------------------------

/// Holds the config needed to start the real [`otto_browser::BrowserService`]
/// and defers actually doing so until the first `/browser/page` (or reader-mode
/// navigation) request — starting the Lightpanda sidecar eagerly at boot would
/// make daemon startup depend on locating/spawning an external process for a
/// feature most sessions never touch. Cheap to construct; held as
/// `Arc<BrowserEngineHandle>` on [`ServerCtx`].
pub struct BrowserEngineHandle {
    configured_bin: Option<String>,
    data_dir: std::path::PathBuf,
    cell: tokio::sync::OnceCell<otto_browser::BrowserService>,
}

impl BrowserEngineHandle {
    pub fn new(configured_bin: Option<String>, data_dir: std::path::PathBuf) -> Self {
        Self {
            configured_bin,
            data_dir,
            cell: tokio::sync::OnceCell::new(),
        }
    }

    async fn service(&self) -> &otto_browser::BrowserService {
        self.cell
            .get_or_init(|| async {
                otto_browser::BrowserService::autodetect(
                    self.configured_bin.as_deref(),
                    self.data_dir.clone(),
                )
                .await
            })
            .await
    }

    /// Caller must netguard-check `url` first — see module docs.
    pub async fn page(&self, url: &str) -> Result<otto_browser::Page, otto_browser::EngineError> {
        self.service().await.page(url).await
    }
}

/// Map a browser-engine failure onto the shared `Error` → HTTP status convention.
fn engine_err(e: otto_browser::EngineError) -> ApiError {
    use otto_browser::EngineError::*;
    ApiError(match e {
        TooLarge(cap) => Error::PayloadTooLarge(format!("page exceeds {cap} bytes")),
        Timeout(secs) => Error::Upstream(format!("page fetch timed out after {secs}s")),
        Nav(msg) => Error::Upstream(format!("navigation failed: {msg}")),
        Unavailable(msg) => Error::Upstream(format!("browser engine unavailable: {msg}")),
    })
}

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct CreateTabReq {
    url: String,
}

#[derive(Deserialize, Default)]
struct PatchTabReq {
    url: Option<String>,
    title: Option<String>,
    mode: Option<String>,
}

#[derive(Deserialize)]
struct PageQuery {
    url: String,
}

/// `{url,title,markdown,html,engine,degraded}` — mirrors `otto_browser::Page`
/// field-for-field (that struct isn't `Serialize`, so this is the wire copy).
#[derive(Serialize)]
struct BrowserPageResp {
    url: String,
    title: String,
    markdown: String,
    html: String,
    engine: String,
    degraded: bool,
}

#[derive(Deserialize)]
struct AnnotationQuery {
    url: Option<String>,
}

#[derive(Deserialize)]
struct CreateAnnotationReq {
    url: String,
    selector: String,
    #[serde(default)]
    excerpt: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    comment: String,
    #[serde(default)]
    color: Option<String>,
    #[serde(default)]
    tab_id: Option<Id>,
}

#[derive(Deserialize)]
struct PatchAnnotationReq {
    comment: String,
}

// ---------------------------------------------------------------------------
// WS event publishing
// ---------------------------------------------------------------------------

fn publish_tab_updated(ctx: &ServerCtx, tab: &BrowserTab) {
    let _ = ctx.events.send(Event::BrowserTabUpdated {
        workspace_id: tab.workspace_id.clone(),
        tab: serde_json::to_value(tab).unwrap_or(serde_json::Value::Null),
    });
}

fn publish_annotation_added(ctx: &ServerCtx, annotation: &BrowserAnnotation) {
    let _ = ctx.events.send(Event::BrowserAnnotationAdded {
        workspace_id: annotation.workspace_id.clone(),
        annotation: serde_json::to_value(annotation).unwrap_or(serde_json::Value::Null),
    });
}

// ---------------------------------------------------------------------------
// Tabs
// ---------------------------------------------------------------------------

/// `GET /workspaces/{wid}/browser/tabs`
async fn list_tabs(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<BrowserTab>>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    Ok(Json(ctx.browser_tabs.list(&wid).await.map_err(ApiError)?))
}

/// `POST /workspaces/{wid}/browser/tabs`
async fn create_tab(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<CreateTabReq>,
) -> ApiResult<Json<BrowserTab>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    if req.url.trim().is_empty() {
        return Err(ApiError(Error::Invalid("url is required".into())));
    }
    let tab = ctx
        .browser_tabs
        .create(NewBrowserTab {
            workspace_id: wid,
            url: req.url,
        })
        .await
        .map_err(ApiError)?;
    publish_tab_updated(&ctx, &tab);
    Ok(Json(tab))
}

/// `PATCH /browser/tabs/{id}` — `{url?, title?, mode?}`. Navigating a
/// reader-mode tab re-fetches the page and adopts its title (see module docs).
async fn update_tab(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<PatchTabReq>,
) -> ApiResult<Json<BrowserTab>> {
    let tab = ctx
        .browser_tabs
        .get(&id)
        .await
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::NotFound(format!("browser tab {id}"))))?;
    require_ws_role(&ctx, &user, &tab.workspace_id, WorkspaceRole::Editor).await?;

    if let Some(mode) = req.mode.as_deref() {
        if mode != "reader" && mode != "live" {
            return Err(ApiError(Error::Invalid(format!(
                "unknown browser tab mode {mode:?} (expected \"reader\" or \"live\")"
            ))));
        }
        ctx.browser_tabs.set_mode(&id, mode).await.map_err(ApiError)?;
    }
    let effective_mode = req.mode.as_deref().unwrap_or(tab.mode.as_str());

    if let Some(url) = req.url {
        if effective_mode == "reader" {
            otto_netguard::check_url(&url)
                .await
                .map_err(|m| ApiError(Error::Invalid(m)))?;
            let page = ctx.browser.page(&url).await.map_err(engine_err)?;
            ctx.browser_tabs
                .update_nav(&id, &url, &page.title)
                .await
                .map_err(ApiError)?;
        } else {
            let title = req.title.unwrap_or(tab.title);
            ctx.browser_tabs
                .update_nav(&id, &url, &title)
                .await
                .map_err(ApiError)?;
        }
    } else if let Some(title) = req.title {
        ctx.browser_tabs
            .update_nav(&id, &tab.url, &title)
            .await
            .map_err(ApiError)?;
    }

    let updated = ctx
        .browser_tabs
        .get(&id)
        .await
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::NotFound(format!("browser tab {id}"))))?;
    publish_tab_updated(&ctx, &updated);
    Ok(Json(updated))
}

/// `DELETE /browser/tabs/{id}`
async fn delete_tab(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<StatusCode> {
    let tab = ctx
        .browser_tabs
        .get(&id)
        .await
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::NotFound(format!("browser tab {id}"))))?;
    require_ws_role(&ctx, &user, &tab.workspace_id, WorkspaceRole::Editor).await?;
    ctx.browser_tabs.delete(&id).await.map_err(ApiError)?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Page fetch
// ---------------------------------------------------------------------------

/// `GET /workspaces/{wid}/browser/page?url=…` — fetch a URL on the caller's
/// behalf (netguard-checked; see module docs).
async fn fetch_page(
    Path(wid): Path<Id>,
    Query(q): Query<PageQuery>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<BrowserPageResp>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    otto_netguard::check_url(&q.url)
        .await
        .map_err(|m| ApiError(Error::Invalid(m)))?;
    let page = ctx.browser.page(&q.url).await.map_err(engine_err)?;
    Ok(Json(BrowserPageResp {
        url: page.url,
        title: page.title,
        markdown: page.markdown,
        html: page.html,
        engine: page.engine,
        degraded: page.degraded,
    }))
}

// ---------------------------------------------------------------------------
// Annotations
// ---------------------------------------------------------------------------

/// `GET /workspaces/{wid}/browser/annotations` (optional `?url=` filter)
async fn list_annotations(
    Path(wid): Path<Id>,
    Query(q): Query<AnnotationQuery>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<BrowserAnnotation>>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    let list = match q.url {
        Some(url) => ctx.browser_annotations.list_for_url(&wid, &url).await,
        None => ctx.browser_annotations.list(&wid).await,
    }
    .map_err(ApiError)?;
    Ok(Json(list))
}

/// `POST /workspaces/{wid}/browser/annotations`
async fn create_annotation(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<CreateAnnotationReq>,
) -> ApiResult<Json<BrowserAnnotation>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    if req.url.trim().is_empty() || req.selector.trim().is_empty() {
        return Err(ApiError(Error::Invalid("url and selector are required".into())));
    }
    let annotation = ctx
        .browser_annotations
        .create(NewBrowserAnnotation {
            workspace_id: wid,
            tab_id: req.tab_id,
            url: req.url,
            selector: req.selector,
            excerpt: req.excerpt,
            text: req.text,
            comment: req.comment,
            color: req.color.unwrap_or_else(|| "yellow".into()),
        })
        .await
        .map_err(ApiError)?;
    publish_annotation_added(&ctx, &annotation);
    Ok(Json(annotation))
}

/// `PATCH /browser/annotations/{id}` — `{comment}` (the only editable field).
async fn update_annotation(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<PatchAnnotationReq>,
) -> ApiResult<Json<BrowserAnnotation>> {
    let annotation = ctx
        .browser_annotations
        .get(&id)
        .await
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::NotFound(format!("browser annotation {id}"))))?;
    require_ws_role(&ctx, &user, &annotation.workspace_id, WorkspaceRole::Editor).await?;
    ctx.browser_annotations
        .update_comment(&id, &req.comment)
        .await
        .map_err(ApiError)?;
    let updated = ctx
        .browser_annotations
        .get(&id)
        .await
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::NotFound(format!("browser annotation {id}"))))?;
    Ok(Json(updated))
}

/// `DELETE /browser/annotations/{id}`
async fn delete_annotation(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<StatusCode> {
    let annotation = ctx
        .browser_annotations
        .get(&id)
        .await
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::NotFound(format!("browser annotation {id}"))))?;
    require_ws_role(&ctx, &user, &annotation.workspace_id, WorkspaceRole::Editor).await?;
    ctx.browser_annotations.delete(&id).await.map_err(ApiError)?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;
    use std::sync::Arc;

    use axum::body::Body;
    use axum::extract::Request;
    use axum::http::Method;
    use chrono::Utc;
    use http_body_util::BodyExt;
    use otto_core::auth::AuthUser;
    use otto_core::domain::User;
    use otto_core::secrets::SecretStore;
    use otto_core::Result;
    use otto_rbac::RbacRoleChecker;
    use otto_sessions::{ProviderRegistry, SessionManager};
    use otto_state::{
        ConnectionSectionsRepo, ConnectionsRepo, DbExplorerRepo, GitStore, IntegrationsRepo,
        IssuesRepo, ProductRepo, ReviewsRepo, SessionsRepo, SkillEvalsRepo, SqlitePool,
        SwarmRepo, WorkspacesRepo,
    };
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use tempfile::TempDir;
    use tokio::sync::broadcast;
    use tower::ServiceExt; // for `oneshot`

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

    /// Root so `require_ws_role` passes without seeding `workspace_members`
    /// rows (`WorkspacesRepo::role_of` returns `Admin` for root unconditionally).
    fn root_user() -> User {
        User {
            id: "root".into(),
            username: "root".into(),
            display_name: "root".into(),
            is_root: true,
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
            library_root: PathBuf::from("/tmp/otto-test-lib-browser"),
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
            PathBuf::from("/tmp/otto-test-usage-browser"),
        )
        .await;
        let context_library = otto_context::Library::new("/tmp/otto-test-ctxlib-browser");

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
            plugins: Arc::new(crate::plugins::PluginManager::new(
                otto_state::PluginsRepo::new(pool.clone()),
                PathBuf::from("/tmp/otto-test-plugins-browser"),
                data_dir.clone(),
                "http://127.0.0.1:7700/api/v1/plugin-host".into(),
            )),
            manager,
            workspaces: WorkspacesRepo::new(pool.clone()),
            connections,
            db_explorer,
            db_assist: crate::db_assist::new_registry(),
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
            product_agent_cancels: crate::product_run::new_cancel_registry(),
            memory: Arc::new(otto_memory::MemoryService::with_defaults(pool.clone())),
            vault: Arc::new(otto_vault::VaultEngine::new(pool.clone())),
            vault_docs_runs: crate::vault_docs_agent::new_run_registry(),
            vault_docs_refine: crate::vault_docs_agent::new_refine_registry(),
            swarm,
            swarm_repo,
            swarm_coords: crate::swarm_runtime::new_registry(),
            swarm_run_cancels: crate::swarm_run::new_cancel_registry(),
            goal_loops_repo: otto_state::GoalLoopsRepo::new(pool.clone()),
            goal_loops: crate::goal_loop::new_registry(),
            workgraph: Arc::new(otto_workgraph::WorkGraphService::new(
                otto_state::WorkGraphRepo::new(pool.clone()),
                events.clone(),
            )),
            scheduled_tasks: otto_state::ScheduledTasksRepo::new(pool.clone()),
            proof_repo: otto_state::ProofRepo::new(pool.clone()),
            proof_locks: crate::proof::new_locks(),
            runs: otto_state::RunsRepo::new(pool.clone()),
            runs_engine: crate::run_engine::RunEngine::new(),
            browser_tabs: otto_state::BrowserTabsRepo::new(pool.clone()),
            browser_annotations: otto_state::BrowserAnnotationsRepo::new(pool.clone()),
            browser: Arc::new(BrowserEngineHandle::new(
                Some("/definitely/not/a/real/lightpanda/binary".into()),
                data_dir,
            )),
        }
    }

    fn browser_router(ctx: ServerCtx) -> Router {
        Router::new().merge(routes()).with_state(ctx)
    }

    async fn test_app() -> (TempDir, Router) {
        let tmp = TempDir::new().expect("tempdir");
        let pool = mem_pool().await;
        let ctx = test_ctx(&pool, tmp.path().to_path_buf()).await;
        (tmp, browser_router(ctx))
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
        req.extensions_mut().insert(AuthUser(root_user()));
        let resp = app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let body = resp.into_body().collect().await.unwrap().to_bytes().to_vec();
        (status, body)
    }

    async fn post_json(app: &Router, uri: &str, body: serde_json::Value) -> (StatusCode, Vec<u8>) {
        send(app, Method::POST, uri, Some(body)).await
    }

    async fn get(app: &Router, uri: &str) -> (StatusCode, Vec<u8>) {
        send(app, Method::GET, uri, None).await
    }

    fn json(body: &[u8]) -> serde_json::Value {
        serde_json::from_slice(body).unwrap_or(serde_json::Value::Null)
    }

    #[tokio::test]
    async fn annotation_roundtrip_and_page_netguard() {
        let (_tmp, app) = test_app().await;

        let (status, body) = post_json(
            &app,
            "/workspaces/ws1/browser/annotations",
            serde_json::json!({
                "url": "https://a.io", "selector": "#x", "excerpt": "<b>x</b>",
                "text": "x", "comment": "note"
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "create: {}", String::from_utf8_lossy(&body));
        let ann = json(&body);
        assert_eq!(ann["url"], "https://a.io");
        assert_eq!(ann["comment"], "note");
        assert_eq!(ann["color"], "yellow", "default color when omitted");

        let (status, body) = get(&app, "/workspaces/ws1/browser/annotations?url=https://a.io").await;
        assert_eq!(status, StatusCode::OK);
        let list = json(&body);
        assert_eq!(list.as_array().map(|a| a.len()), Some(1));

        let (status, _) = get(
            &app,
            "/workspaces/ws1/browser/page?url=http://169.254.169.254/",
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "netguard must reject metadata IPs");
    }

    #[tokio::test]
    async fn tab_crud_via_http() {
        let (_tmp, app) = test_app().await;

        let (status, body) = post_json(
            &app,
            "/workspaces/ws1/browser/tabs",
            serde_json::json!({"url": "https://example.com"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let tab = json(&body);
        let id = tab["id"].as_str().unwrap().to_string();
        assert_eq!(tab["mode"], "reader");

        // Non-reader-mode nav: no fetch pipeline, title comes straight from the request.
        let (status, body) = send(
            &app,
            Method::PATCH,
            &format!("/browser/tabs/{id}"),
            Some(serde_json::json!({"mode": "live", "url": "https://b.io", "title": "B"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
        let updated = json(&body);
        assert_eq!(updated["mode"], "live");
        assert_eq!(updated["url"], "https://b.io");
        assert_eq!(updated["title"], "B");

        let (status, _) = send(&app, Method::DELETE, &format!("/browser/tabs/{id}"), None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);

        let (status, body) = get(&app, "/workspaces/ws1/browser/tabs").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json(&body).as_array().map(|a| a.len()), Some(0));
    }

    #[tokio::test]
    async fn tab_patch_rejects_unknown_mode() {
        let (_tmp, app) = test_app().await;
        let (_, body) = post_json(
            &app,
            "/workspaces/ws1/browser/tabs",
            serde_json::json!({"url": "https://example.com"}),
        )
        .await;
        let id = json(&body)["id"].as_str().unwrap().to_string();
        let (status, _) = send(
            &app,
            Method::PATCH,
            &format!("/browser/tabs/{id}"),
            Some(serde_json::json!({"mode": "bogus"})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn annotation_update_and_delete_via_http() {
        let (_tmp, app) = test_app().await;
        let (_, body) = post_json(
            &app,
            "/workspaces/ws1/browser/annotations",
            serde_json::json!({
                "url": "https://a.io", "selector": "#x", "excerpt": "e", "text": "t", "comment": "old"
            }),
        )
        .await;
        let id = json(&body)["id"].as_str().unwrap().to_string();

        let (status, body) = send(
            &app,
            Method::PATCH,
            &format!("/browser/annotations/{id}"),
            Some(serde_json::json!({"comment": "new"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json(&body)["comment"], "new");

        let (status, _) = send(
            &app,
            Method::DELETE,
            &format!("/browser/annotations/{id}"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn missing_tab_and_annotation_ids_404() {
        let (_tmp, app) = test_app().await;
        let (status, _) = send(&app, Method::DELETE, "/browser/tabs/nope", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _) = send(&app, Method::DELETE, "/browser/annotations/nope", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}
