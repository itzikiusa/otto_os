//! Memory REST router. Paths are relative to the `/api/v1` mount point.
//! Reads require workspace `Viewer`; mutations require `Editor`.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use serde::{Deserialize, Serialize};

use otto_core::api::Problem;
use otto_core::auth::{AuthUser, RoleChecker};
use otto_core::domain::{User, WorkspaceRole};
use otto_core::{Error, Id};
use otto_state::memory::ListFilter;

use crate::service::MemoryService;
use crate::types::*;

// ---------------------------------------------------------------------------
// Context trait
// ---------------------------------------------------------------------------

/// Host-application context required by the memory router.
pub trait MemoryCtx: Clone + Send + Sync + 'static {
    fn memory(&self) -> &Arc<MemoryService>;
    fn roles(&self) -> &Arc<dyn RoleChecker>;
}

// ---------------------------------------------------------------------------
// Error → response
// ---------------------------------------------------------------------------

struct ApiErr(Error);

impl From<Error> for ApiErr {
    fn from(e: Error) -> Self {
        ApiErr(e)
    }
}

impl IntoResponse for ApiErr {
    fn into_response(self) -> Response {
        let status = match &self.0 {
            Error::NotFound(_) => StatusCode::NOT_FOUND,
            Error::Unauthorized => StatusCode::UNAUTHORIZED,
            Error::Forbidden(_) => StatusCode::FORBIDDEN,
            Error::Conflict(_) => StatusCode::CONFLICT,
            Error::Invalid(_) => StatusCode::BAD_REQUEST,
            Error::PayloadTooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
            Error::UnsupportedMedia(_) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Error::Upstream(_) => StatusCode::BAD_GATEWAY,
            Error::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let problem = Problem {
            code: self.0.code().to_string(),
            message: self.0.to_string(),
        };
        (status, Json(problem)).into_response()
    }
}

type ApiResult<T> = std::result::Result<T, ApiErr>;

async fn require<C: MemoryCtx>(
    c: &C,
    user: &User,
    ws: &Id,
    min: WorkspaceRole,
) -> std::result::Result<(), ApiErr> {
    c.roles().check(user, ws, min).await.map_err(ApiErr)
}

// ---------------------------------------------------------------------------
// Path / query extractors
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct WsPath {
    ws: Id,
}

#[derive(Deserialize)]
struct WsIdPath {
    ws: Id,
    id: Id,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ListQ {
    collection: Option<String>,
    kind: Option<String>,
    story_id: Option<String>,
    tag: Option<String>,
    include_inactive: bool,
    limit: i64,
}

#[derive(Deserialize)]
struct RecallReq {
    story_id: String,
    #[serde(default)]
    focus: Option<String>,
    #[serde(default)]
    token_budget: usize,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct GraphQ {
    collection: Option<String>,
}

// ---------------------------------------------------------------------------
// Graph DTOs
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
pub struct GraphNode {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub collection: String,
}

#[derive(Serialize, Deserialize)]
pub struct GraphData {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<MemoryLink>,
}

fn default_code_collection() -> String {
    "code".into()
}

#[derive(Deserialize)]
struct IngestTextReq {
    #[serde(default = "default_code_collection")]
    collection: String,
    path: String,
    content: String,
}

#[derive(Serialize)]
struct IngestTextResp {
    chunks: usize,
}

#[derive(Deserialize)]
struct ImportGraphReq {
    #[serde(default = "default_code_collection")]
    collection: String,
    graph: crate::ingest::GraphifyGraph,
}

#[derive(Serialize)]
struct EntityGraphResp {
    links: Vec<MemoryLink>,
    neighbors: Vec<Memory>,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn list<C: MemoryCtx>(
    State(c): State<C>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(WsPath { ws }): Path<WsPath>,
    Query(q): Query<ListQ>,
) -> ApiResult<Json<Vec<Memory>>> {
    require(&c, &user, &ws, WorkspaceRole::Viewer).await?;
    let f = ListFilter {
        collection: q.collection,
        kind: q.kind,
        story_id: q.story_id,
        tag: q.tag,
        include_inactive: q.include_inactive,
        limit: q.limit,
        viewer: if user.is_root { None } else { Some(user.id.clone()) },
    };
    Ok(Json(c.memory().list(&ws, f).await?))
}

async fn create<C: MemoryCtx>(
    State(c): State<C>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(WsPath { ws }): Path<WsPath>,
    Json(body): Json<NewMemory>,
) -> ApiResult<Json<Memory>> {
    require(&c, &user, &ws, WorkspaceRole::Editor).await?;
    let mut v = c.memory().save(&ws, &user.id, vec![body]).await?;
    Ok(Json(v.remove(0)))
}

async fn get_one<C: MemoryCtx>(
    State(c): State<C>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(WsIdPath { ws, id }): Path<WsIdPath>,
) -> ApiResult<Json<Memory>> {
    require(&c, &user, &ws, WorkspaceRole::Viewer).await?;
    let m = c.memory().get(&ws, &id).await?;
    // Another user's private memory is invisible (404, not 403, to avoid leaking
    // its existence).
    if m.visibility == "private" && m.created_by != user.id && !user.is_root {
        return Err(ApiErr(Error::NotFound("memory".into())));
    }
    Ok(Json(m))
}

async fn patch_one<C: MemoryCtx>(
    State(c): State<C>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(WsIdPath { ws, id }): Path<WsIdPath>,
    Json(p): Json<MemoryPatch>,
) -> ApiResult<Json<Memory>> {
    require(&c, &user, &ws, WorkspaceRole::Editor).await?;
    Ok(Json(c.memory().update(&ws, &id, p).await?))
}

async fn delete_one<C: MemoryCtx>(
    State(c): State<C>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(WsIdPath { ws, id }): Path<WsIdPath>,
) -> ApiResult<StatusCode> {
    require(&c, &user, &ws, WorkspaceRole::Editor).await?;
    c.memory().forget(&ws, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn search<C: MemoryCtx>(
    State(c): State<C>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(WsPath { ws }): Path<WsPath>,
    Json(mut q): Json<MemoryQuery>,
) -> ApiResult<Json<Vec<MemoryHit>>> {
    require(&c, &user, &ws, WorkspaceRole::Viewer).await?;
    q.viewer = if user.is_root { None } else { Some(user.id.clone()) };
    Ok(Json(c.memory().search(&ws, q).await?))
}

async fn recall<C: MemoryCtx>(
    State(c): State<C>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(WsPath { ws }): Path<WsPath>,
    Json(r): Json<RecallReq>,
) -> ApiResult<Json<RecallBrief>> {
    require(&c, &user, &ws, WorkspaceRole::Viewer).await?;
    let opts = RecallOpts {
        focus: r.focus,
        token_budget: r.token_budget,
        kinds: vec![],
        viewer: if user.is_root { None } else { Some(user.id.clone()) },
    };
    Ok(Json(c.memory().recall_brief(&ws, &r.story_id, opts).await?))
}

async fn links<C: MemoryCtx>(
    State(c): State<C>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(WsIdPath { ws, id }): Path<WsIdPath>,
) -> ApiResult<Json<Vec<MemoryLink>>> {
    require(&c, &user, &ws, WorkspaceRole::Viewer).await?;
    Ok(Json(c.memory().links(&ws, &id).await?))
}

async fn graph<C: MemoryCtx>(
    State(c): State<C>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(WsPath { ws }): Path<WsPath>,
    Query(q): Query<GraphQ>,
) -> ApiResult<Json<GraphData>> {
    require(&c, &user, &ws, WorkspaceRole::Viewer).await?;
    let repo = c.memory().repo();
    let nodes = repo
        .graph_nodes(&ws, q.collection.as_deref())
        .await?
        .into_iter()
        .map(|(id, label, kind, collection)| GraphNode {
            id,
            label,
            kind,
            collection,
        })
        .collect();
    let edges = repo.all_links(&ws).await?;
    Ok(Json(GraphData { nodes, edges }))
}

async fn ingest_text<C: MemoryCtx>(
    State(c): State<C>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(WsPath { ws }): Path<WsPath>,
    Json(req): Json<IngestTextReq>,
) -> ApiResult<Json<IngestTextResp>> {
    require(&c, &user, &ws, WorkspaceRole::Editor).await?;
    let chunks = c
        .memory()
        .ingest_text(&ws, &user.id, &req.collection, &req.path, &req.content)
        .await?;
    Ok(Json(IngestTextResp { chunks }))
}

async fn import_graph<C: MemoryCtx>(
    State(c): State<C>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(WsPath { ws }): Path<WsPath>,
    Json(req): Json<ImportGraphReq>,
) -> ApiResult<Json<crate::ingest::ImportStats>> {
    require(&c, &user, &ws, WorkspaceRole::Editor).await?;
    Ok(Json(
        c.memory()
            .import_graph(&ws, &user.id, &req.collection, req.graph)
            .await?,
    ))
}

async fn entity_graph<C: MemoryCtx>(
    State(c): State<C>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(WsIdPath { ws, id }): Path<WsIdPath>,
) -> ApiResult<Json<EntityGraphResp>> {
    require(&c, &user, &ws, WorkspaceRole::Viewer).await?;
    let (links, neighbors) = c.memory().entity_graph(&ws, &id).await?;
    Ok(Json(EntityGraphResp { links, neighbors }))
}

// ---------------------------------------------------------------------------
// Vault v2 — code intelligence handlers
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct IndexRepoReq {
    root: String,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct SymbolsQ {
    q: Option<String>,
    repo_id: Option<String>,
    limit: i64,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct CodeGraphQ {
    repo_id: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct NeighborhoodQ {
    depth: usize,
}

#[derive(Deserialize)]
struct WsNodePath {
    ws: Id,
    node_id: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct BrainReq {
    focus: String,
    cwd: Option<String>,
    budget: usize,
}

#[derive(Deserialize)]
struct DocReq {
    #[serde(default)]
    repo_id: Option<String>,
    title: String,
    body: String,
    #[serde(default)]
    documents: Vec<String>,
}

async fn index_repo<C: MemoryCtx>(
    State(c): State<C>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(WsPath { ws }): Path<WsPath>,
    Json(req): Json<IndexRepoReq>,
) -> ApiResult<Json<IndexResult>> {
    require(&c, &user, &ws, WorkspaceRole::Editor).await?;
    let root = std::path::Path::new(&req.root);
    if !root.is_dir() {
        return Err(ApiErr(Error::Invalid(format!("not a directory: {}", req.root))));
    }
    // Register the repo + mark it indexing, then run the heavy scan+embed in the
    // BACKGROUND — embedding with a large neural model can take many minutes, so
    // we never block the request. The UI polls repo status (indexing → ready).
    let canon = root
        .canonicalize()
        .map_err(|e| ApiErr(Error::Invalid(format!("repo path: {e}"))))?;
    let canon_str = canon.to_string_lossy().to_string();
    let name = req
        .name
        .clone()
        .or_else(|| canon.file_name().map(|f| f.to_string_lossy().to_string()))
        .unwrap_or_else(|| canon_str.clone());
    let repo_id = c.memory().code().upsert_repo(&ws, &canon_str, &name).await?;
    c.memory()
        .code()
        .set_repo_state(&repo_id, "indexing", None, None, None)
        .await?;

    let mem = c.memory().clone();
    let ws2 = ws.to_string();
    let by = user.id.clone();
    let nm = req.name.clone();
    let rid = repo_id.clone();
    tokio::spawn(async move {
        if let Err(e) = mem.index_repo(&ws2, &by, &canon, nm.as_deref()).await {
            tracing::warn!("vault: background index of {ws2} failed: {e}");
            let _ = mem
                .code()
                .set_repo_state(&rid, "error", Some(&e.to_string()), None, None)
                .await;
        }
    });
    // Counts arrive once indexing completes (poll repo status).
    Ok(Json(IndexResult { repo_id, files: 0, symbols: 0, edges: 0, chunks: 0 }))
}

async fn list_repos<C: MemoryCtx>(
    State(c): State<C>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(WsPath { ws }): Path<WsPath>,
) -> ApiResult<Json<Vec<otto_state::CodeRepo>>> {
    require(&c, &user, &ws, WorkspaceRole::Viewer).await?;
    Ok(Json(c.memory().list_repos(&ws).await?))
}

async fn symbols<C: MemoryCtx>(
    State(c): State<C>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(WsPath { ws }): Path<WsPath>,
    Query(q): Query<SymbolsQ>,
) -> ApiResult<Json<Vec<otto_state::CodeSymbol>>> {
    require(&c, &user, &ws, WorkspaceRole::Viewer).await?;
    let limit = if q.limit > 0 { q.limit } else { 50 };
    Ok(Json(
        c.memory()
            .search_symbols(&ws, q.q.as_deref().unwrap_or(""), q.repo_id.as_deref(), limit)
            .await?,
    ))
}

async fn code_graph<C: MemoryCtx>(
    State(c): State<C>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(WsPath { ws }): Path<WsPath>,
    Query(q): Query<CodeGraphQ>,
) -> ApiResult<Json<otto_state::CodeGraph>> {
    require(&c, &user, &ws, WorkspaceRole::Viewer).await?;
    Ok(Json(c.memory().code_graph(&ws, q.repo_id.as_deref()).await?))
}

async fn full_graph<C: MemoryCtx>(
    State(c): State<C>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(WsPath { ws }): Path<WsPath>,
    Query(q): Query<CodeGraphQ>,
) -> ApiResult<Json<FullGraph>> {
    require(&c, &user, &ws, WorkspaceRole::Viewer).await?;
    Ok(Json(c.memory().full_graph(&ws, q.repo_id.as_deref()).await?))
}

async fn neighborhood<C: MemoryCtx>(
    State(c): State<C>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(WsNodePath { ws, node_id }): Path<WsNodePath>,
    Query(q): Query<NeighborhoodQ>,
) -> ApiResult<Json<otto_state::CodeGraph>> {
    require(&c, &user, &ws, WorkspaceRole::Viewer).await?;
    let depth = if q.depth == 0 { 2 } else { q.depth };
    Ok(Json(c.memory().code_neighborhood(&ws, &node_id, depth).await?))
}

async fn brain<C: MemoryCtx>(
    State(c): State<C>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(WsPath { ws }): Path<WsPath>,
    Json(req): Json<BrainReq>,
) -> ApiResult<Json<RepoBrain>> {
    require(&c, &user, &ws, WorkspaceRole::Viewer).await?;
    let cwd = req.cwd.as_deref().map(std::path::Path::new);
    Ok(Json(c.memory().repo_brain(&ws, &req.focus, cwd, req.budget).await?))
}

async fn create_doc<C: MemoryCtx>(
    State(c): State<C>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(WsPath { ws }): Path<WsPath>,
    Json(req): Json<DocReq>,
) -> ApiResult<Json<Memory>> {
    require(&c, &user, &ws, WorkspaceRole::Editor).await?;
    Ok(Json(
        c.memory()
            .upsert_doc(&ws, &user.id, req.repo_id.as_deref(), &req.title, &req.body, &req.documents)
            .await?,
    ))
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Build the memory router. Paths are relative to the `/api/v1` mount point.
pub fn router<C: MemoryCtx>() -> Router<C> {
    Router::new()
        .route(
            "/workspaces/{ws}/memories",
            get(list::<C>).post(create::<C>),
        )
        .route(
            "/workspaces/{ws}/memories/{id}",
            get(get_one::<C>).patch(patch_one::<C>).delete(delete_one::<C>),
        )
        .route("/workspaces/{ws}/memories/{id}/links", get(links::<C>))
        .route("/workspaces/{ws}/memory/search", post(search::<C>))
        .route("/workspaces/{ws}/memory/recall", post(recall::<C>))
        .route("/workspaces/{ws}/memory/graph", get(graph::<C>))
        .route("/workspaces/{ws}/memory/ingest-text", post(ingest_text::<C>))
        .route("/workspaces/{ws}/memory/import-graph", post(import_graph::<C>))
        .route(
            "/workspaces/{ws}/memory/entities/{id}/graph",
            get(entity_graph::<C>),
        )
        // -- Vault v2: code intelligence --
        .route("/workspaces/{ws}/vault/repos", get(list_repos::<C>))
        .route("/workspaces/{ws}/vault/repos/index", post(index_repo::<C>))
        .route("/workspaces/{ws}/vault/symbols", get(symbols::<C>))
        .route("/workspaces/{ws}/vault/graph", get(code_graph::<C>))
        .route("/workspaces/{ws}/vault/fullgraph", get(full_graph::<C>))
        .route("/workspaces/{ws}/vault/graph/{node_id}", get(neighborhood::<C>))
        .route("/workspaces/{ws}/vault/brain", post(brain::<C>))
        .route("/workspaces/{ws}/vault/docs", post(create_doc::<C>))
}
