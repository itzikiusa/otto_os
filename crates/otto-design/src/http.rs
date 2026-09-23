//! Design Hall REST router + handlers (`/api/v1/design/…`, see
//! `docs/contracts/api.md` § Design Hall).
//!
//! Every route is flat `/design/…`: collection creates carry `workspace_id`
//! in the body, item routes resolve the workspace from the row. Reads need
//! workspace `Viewer`, writes `Editor`, hard deletes `Admin`; list/search
//! results are filtered to the workspaces the caller can view (root: all).
//! The `Feature::Design` axis (View / Edit, Admin for `/design/admin/*`) is
//! enforced upstream by the server's deny-by-default policy middleware.

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Extension, Json, Router};
use otto_core::api::Problem;
use otto_core::auth::{AuthUser, RoleChecker};
use otto_core::domain::{User, WorkspaceRole};
use otto_core::{Error, Id};
use serde::Deserialize;
use serde_json::Value;

use crate::service::{
    bound_json, clean_title, decode_content, merge_meta, Author, CreateInput, DesignService,
    SaveOpts, MAX_META_BYTES,
};
use crate::store::{stamp, ArtifactFilter, NewProject};
use crate::types::*;

/// JSON bodies that may carry content (base64 inflates the 25 MB raw cap).
const CONTENT_BODY_LIMIT: usize = 40 * 1024 * 1024;
/// PATCH bodies may carry a ≤ 2 MB PNG thumbnail as base64.
const PATCH_BODY_LIMIT: usize = 4 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Context trait
// ---------------------------------------------------------------------------

/// Host-application context required by the design router.
pub trait DesignCtx: Clone + Send + Sync + 'static {
    /// A (cheap) service handle bound to the host's pool, data dir and event bus.
    fn design(&self) -> DesignService;
    fn roles(&self) -> &Arc<dyn RoleChecker>;
    /// Deep, host-owned format validation run on every content write (e.g.
    /// otto-server validates `scene3d` against its schema). The cheap checks
    /// in `format::validate` always run too.
    fn validate_content(&self, _format: &str, _bytes: &[u8]) -> otto_core::Result<()> {
        Ok(())
    }
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

// ---------------------------------------------------------------------------
// Extractors
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct IdPath {
    id: Id,
}

#[derive(Deserialize)]
struct VersionPath {
    id: Id,
    v: String,
}

#[derive(Deserialize)]
struct LinkPath {
    id: Id,
    link_id: Id,
}

#[derive(Deserialize, Default)]
struct ProjectListQuery {
    #[serde(default)]
    workspace_id: Option<Id>,
    #[serde(default)]
    include_archived: Option<bool>,
}

#[derive(Deserialize, Default)]
struct DetailQuery {
    #[serde(default)]
    content: Option<bool>,
    #[serde(default)]
    version: Option<String>,
}

#[derive(Deserialize, Default)]
struct HardQuery {
    #[serde(default)]
    hard: Option<bool>,
}

/// Filters shared by `GET /design/artifacts` and `GET /design/search`.
#[derive(Deserialize, Default)]
struct ArtifactQuery {
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    workspace_id: Option<Id>,
    #[serde(default)]
    project_id: Option<Id>,
    #[serde(default)]
    studio: Option<String>,
    #[serde(default)]
    format: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    story_id: Option<Id>,
    #[serde(default)]
    include_children: Option<bool>,
    #[serde(default)]
    author_kind: Option<String>,
    #[serde(default)]
    since: Option<String>,
    #[serde(default)]
    until: Option<String>,
    #[serde(default)]
    include_archived: Option<bool>,
    #[serde(default)]
    limit: Option<i64>,
    #[serde(default)]
    offset: Option<i64>,
}

#[derive(Deserialize, Default)]
struct VersionListQuery {
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    limit: Option<i64>,
    #[serde(default)]
    offset: Option<i64>,
}

#[derive(Deserialize, Default)]
struct LinksQuery {
    /// `out` (Uses) | `in` (Used in) | `both` (default).
    #[serde(default)]
    dir: Option<String>,
}

#[derive(Deserialize, Default)]
struct SignalListQuery {
    #[serde(default)]
    workspace_id: Option<Id>,
    #[serde(default)]
    artifact_id: Option<Id>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    since: Option<String>,
    #[serde(default)]
    limit: Option<i64>,
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Build the design router. Paths are relative to the `/api/v1` mount point.
pub fn router<S: DesignCtx>() -> Router<S> {
    Router::new()
        .route(
            "/design/projects",
            get(list_projects::<S>).post(create_project::<S>),
        )
        .route(
            "/design/projects/{id}",
            get(get_project::<S>)
                .patch(update_project::<S>)
                .delete(delete_project::<S>),
        )
        .route(
            "/design/artifacts",
            get(list_artifacts::<S>)
                .post(create_artifact::<S>)
                .layer(DefaultBodyLimit::max(CONTENT_BODY_LIMIT)),
        )
        .route(
            "/design/artifacts/{id}",
            get(get_artifact::<S>)
                .patch(update_artifact::<S>)
                .delete(delete_artifact::<S>)
                .layer(DefaultBodyLimit::max(PATCH_BODY_LIMIT)),
        )
        .route(
            "/design/artifacts/{id}/content",
            get(get_content::<S>)
                .put(put_content::<S>)
                .layer(DefaultBodyLimit::max(CONTENT_BODY_LIMIT)),
        )
        .route("/design/artifacts/{id}/thumbnail", get(get_thumbnail::<S>))
        .route(
            "/design/artifacts/{id}/versions",
            get(list_versions::<S>)
                .post(commit_version::<S>)
                .layer(DefaultBodyLimit::max(CONTENT_BODY_LIMIT)),
        )
        .route(
            "/design/artifacts/{id}/versions/{v}/content",
            get(version_content::<S>),
        )
        .route("/design/artifacts/{id}/approve", post(approve::<S>))
        .route(
            "/design/artifacts/{id}/links",
            get(list_links::<S>).post(create_link::<S>),
        )
        .route(
            "/design/artifacts/{id}/links/{link_id}",
            delete(delete_link::<S>),
        )
        .route("/design/search", get(search::<S>))
        .route(
            "/design/signals",
            get(list_signals::<S>).post(create_signal::<S>),
        )
        .route("/design/admin/import", post(run_import::<S>))
        .route("/design/admin/prune", post(prune::<S>))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// The workspaces the caller may see: `Some([ws])` when one is requested
/// (checked), `None` for root (no filter), else every known workspace the
/// caller is a viewer of.
async fn visible<S: DesignCtx>(
    ctx: &S,
    svc: &DesignService,
    user: &User,
    requested: Option<&Id>,
) -> ApiResult<Option<Vec<Id>>> {
    if let Some(ws) = requested {
        ctx.roles().check(user, ws, WorkspaceRole::Viewer).await?;
        return Ok(Some(vec![ws.clone()]));
    }
    if user.is_root {
        return Ok(None);
    }
    let mut out = Vec::new();
    for ws in svc.store().known_workspaces().await? {
        if ctx
            .roles()
            .check(user, &ws, WorkspaceRole::Viewer)
            .await
            .is_ok()
        {
            out.push(ws);
        }
    }
    Ok(Some(out))
}

async fn load_artifact<S: DesignCtx>(
    ctx: &S,
    svc: &DesignService,
    user: &User,
    id: &str,
    role: WorkspaceRole,
) -> ApiResult<DesignArtifact> {
    let a = svc.store().require_artifact(id).await?;
    ctx.roles().check(user, &a.workspace_id, role).await?;
    Ok(a)
}

async fn load_project<S: DesignCtx>(
    ctx: &S,
    svc: &DesignService,
    user: &User,
    id: &str,
    role: WorkspaceRole,
) -> ApiResult<DesignProject> {
    let p = svc
        .store()
        .get_project(id)
        .await?
        .ok_or_else(|| Error::NotFound(format!("design project {id}")))?;
    ctx.roles().check(user, &p.workspace_id, role).await?;
    Ok(p)
}

/// Normalize an RFC 3339 query bound to the storage format.
fn norm_time(t: Option<String>) -> ApiResult<Option<String>> {
    match t.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(s) => chrono::DateTime::parse_from_rfc3339(&s)
            .map(|d| Some(stamp(d.with_timezone(&chrono::Utc))))
            .map_err(|e| ApiErr(Error::Invalid(format!("bad timestamp {s:?}: {e}")))),
    }
}

fn filter_from(
    q: ArtifactQuery,
    workspaces: Option<Vec<Id>>,
    default_limit: i64,
) -> ApiResult<ArtifactFilter> {
    if let Some(s) = &q.status {
        if !one_of(STATUSES, s) {
            return Err(ApiErr(Error::Invalid(format!("unknown status {s:?}"))));
        }
    }
    Ok(ArtifactFilter {
        workspaces,
        project_id: q.project_id,
        studio: q.studio,
        format: q.format,
        status: q.status,
        story_id: q.story_id,
        include_children: q.include_children.unwrap_or(false),
        author_kind: q.author_kind,
        since: norm_time(q.since)?,
        until: norm_time(q.until)?,
        include_archived: q.include_archived.unwrap_or(false),
        limit: q.limit.unwrap_or(default_limit),
        offset: q.offset.unwrap_or(0),
    })
}

/// Raw bytes of one version, served with the artifact's mime. `sandbox` CSP +
/// `nosniff` keep an HTML/SVG design from ever running at the daemon origin.
fn content_response(a: &DesignArtifact, v: &DesignVersion, bytes: Vec<u8>) -> ApiResult<Response> {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, a.mime.as_str())
        .header("x-content-type-options", "nosniff")
        .header(header::CONTENT_SECURITY_POLICY, "sandbox")
        .header(header::CACHE_CONTROL, "no-store")
        .header(header::ETAG, format!("\"{}\"", v.blob_sha256))
        .header("x-design-version", v.id.as_str())
        .header("x-design-seq", v.seq.to_string())
        .body(Body::from(bytes))
        .map_err(|e| ApiErr(Error::Internal(format!("build response: {e}"))))
}

// ---------------------------------------------------------------------------
// Projects
// ---------------------------------------------------------------------------

async fn list_projects<S: DesignCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Query(q): Query<ProjectListQuery>,
) -> ApiResult<Response> {
    let svc = ctx.design();
    let ws = visible(&ctx, &svc, &user, q.workspace_id.as_ref()).await?;
    let projects = svc
        .store()
        .list_projects(ws.as_deref(), q.include_archived.unwrap_or(false))
        .await?;
    Ok(Json(projects).into_response())
}

async fn create_project<S: DesignCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<CreateProjectReq>,
) -> ApiResult<Response> {
    ctx.roles()
        .check(&user, &req.workspace_id, WorkspaceRole::Editor)
        .await?;
    let svc = ctx.design();
    let name = clean_title(&req.name)?;
    if let Some(epic) = &req.epic_story_id {
        if svc.store().story_workspace(epic).await?.is_none() {
            return Err(ApiErr(Error::NotFound(format!("product story {epic}"))));
        }
    }
    if let Some(b) = &req.brand_kit_id {
        let kit = svc.store().require_artifact(b).await?;
        if kit.workspace_id != req.workspace_id {
            return Err(ApiErr(Error::Invalid(
                "the brand kit belongs to another workspace".into(),
            )));
        }
    }
    let meta = bound_json(req.meta.unwrap_or(Value::Null), MAX_META_BYTES, "meta")?;
    let project = svc
        .store()
        .create_project(NewProject {
            workspace_id: req.workspace_id,
            name,
            description: req
                .description
                .unwrap_or_default()
                .chars()
                .take(4_000)
                .collect(),
            epic_story_id: req.epic_story_id,
            swarm_project_id: req.swarm_project_id,
            brand_kit_id: req.brand_kit_id,
            meta,
            created_by: user.id.clone(),
        })
        .await?;
    Ok((StatusCode::CREATED, Json(project)).into_response())
}

async fn get_project<S: DesignCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(IdPath { id }): Path<IdPath>,
) -> ApiResult<Response> {
    let svc = ctx.design();
    let p = load_project(&ctx, &svc, &user, &id, WorkspaceRole::Viewer).await?;
    Ok(Json(p).into_response())
}

/// `""` clears an optional id; `None` keeps it.
fn patch_opt(current: Option<Id>, patch: Option<String>) -> Option<Id> {
    match patch {
        None => current,
        Some(s) if s.trim().is_empty() => None,
        Some(s) => Some(s.trim().to_string()),
    }
}

async fn update_project<S: DesignCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(IdPath { id }): Path<IdPath>,
    Json(req): Json<UpdateProjectReq>,
) -> ApiResult<Response> {
    let svc = ctx.design();
    let mut p = load_project(&ctx, &svc, &user, &id, WorkspaceRole::Editor).await?;
    if let Some(n) = req.name {
        p.name = clean_title(&n)?;
    }
    if let Some(d) = req.description {
        p.description = d.chars().take(4_000).collect();
    }
    p.epic_story_id = patch_opt(p.epic_story_id, req.epic_story_id);
    if let Some(epic) = &p.epic_story_id {
        if svc.store().story_workspace(epic).await?.is_none() {
            return Err(ApiErr(Error::NotFound(format!("product story {epic}"))));
        }
    }
    p.swarm_project_id = patch_opt(p.swarm_project_id, req.swarm_project_id);
    p.brand_kit_id = patch_opt(p.brand_kit_id, req.brand_kit_id);
    p.cover_artifact_id = patch_opt(p.cover_artifact_id, req.cover_artifact_id);
    for aid in [&p.brand_kit_id, &p.cover_artifact_id]
        .into_iter()
        .flatten()
    {
        let a = svc.store().require_artifact(aid).await?;
        if a.workspace_id != p.workspace_id {
            return Err(ApiErr(Error::Invalid(format!(
                "artifact {aid} belongs to another workspace"
            ))));
        }
    }
    if let Some(a) = req.archived {
        p.archived = a;
    }
    if let Some(m) = req.meta {
        merge_meta(&mut p.meta, m)?;
        p.meta = bound_json(p.meta.clone(), MAX_META_BYTES, "meta")?;
    }
    let updated = svc.store().write_project(&p).await?;
    Ok(Json(updated).into_response())
}

/// Archive (default) or, with `?hard=true` (workspace Admin), delete an EMPTY
/// project. Artifacts are never deleted with their project.
async fn delete_project<S: DesignCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(IdPath { id }): Path<IdPath>,
    Query(q): Query<HardQuery>,
) -> ApiResult<Response> {
    let svc = ctx.design();
    if q.hard.unwrap_or(false) {
        load_project(&ctx, &svc, &user, &id, WorkspaceRole::Admin).await?;
        svc.store().delete_project(&id).await?;
    } else {
        let mut p = load_project(&ctx, &svc, &user, &id, WorkspaceRole::Editor).await?;
        p.archived = true;
        svc.store().write_project(&p).await?;
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

// ---------------------------------------------------------------------------
// Artifacts
// ---------------------------------------------------------------------------

async fn list_artifacts<S: DesignCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Query(q): Query<ArtifactQuery>,
) -> ApiResult<Response> {
    let svc = ctx.design();
    let ws = visible(&ctx, &svc, &user, q.workspace_id.as_ref()).await?;
    let f = filter_from(q, ws, 100)?;
    Ok(Json(svc.store().list_artifacts(&f).await?).into_response())
}

async fn create_artifact<S: DesignCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<CreateArtifactReq>,
) -> ApiResult<Response> {
    ctx.roles()
        .check(&user, &req.workspace_id, WorkspaceRole::Editor)
        .await?;
    let svc = ctx.design();
    let content = decode_content(req.content, req.content_b64)?;
    if let Some(bytes) = &content {
        ctx.validate_content(&req.format, bytes)?;
    }
    if let Some(f) = &req.derived_from {
        // Forking needs read access to the source.
        load_artifact(&ctx, &svc, &user, &f.artifact_id, WorkspaceRole::Viewer).await?;
    }
    if let Some(story) = &req.story_id {
        if svc.store().story_workspace(story).await?.is_none() {
            return Err(ApiErr(Error::NotFound(format!("product story {story}"))));
        }
    }
    let author =
        Author::from_request(&user.id, req.author_kind.as_deref(), req.session_id.clone())?;
    let saved = svc
        .create_artifact(CreateInput {
            workspace_id: req.workspace_id,
            project_id: req.project_id,
            studio: req.studio,
            format: req.format,
            title: req.title,
            tags: req.tags,
            meta: req.meta.unwrap_or(Value::Null),
            content,
            story_id: req.story_id,
            derived_from: req.derived_from,
            created_by: user.id.clone(),
            author,
            message: req.message,
            source: None,
            created_at: None,
            version_kind: None,
            validate: true,
        })
        .await?;
    Ok((StatusCode::CREATED, Json(saved)).into_response())
}

/// Detail; `?content=true[&version=v3]` inlines the (text) source — what
/// the `design_get` MCP tool reads.
async fn get_artifact<S: DesignCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(IdPath { id }): Path<IdPath>,
    Query(q): Query<DetailQuery>,
) -> ApiResult<Response> {
    let svc = ctx.design();
    let a = load_artifact(&ctx, &svc, &user, &id, WorkspaceRole::Viewer).await?;
    let mut detail = svc.detail(a).await?;
    if q.content.unwrap_or(false) || q.version.is_some() {
        detail = svc.with_content(detail, q.version.as_deref()).await?;
    }
    Ok(Json(detail).into_response())
}

async fn update_artifact<S: DesignCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(IdPath { id }): Path<IdPath>,
    Json(req): Json<UpdateArtifactReq>,
) -> ApiResult<Response> {
    let svc = ctx.design();
    let a = load_artifact(&ctx, &svc, &user, &id, WorkspaceRole::Editor).await?;
    let updated = svc.update_meta(&a.id, req, &Author::user(&user.id)).await?;
    Ok(Json(updated).into_response())
}

/// Archive (default: status → `archived`, every version kept) or, with
/// `?hard=true` (workspace Admin), delete the graph rows. Blobs are never
/// deleted here (only the opt-in prune GC removes blobs).
async fn delete_artifact<S: DesignCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(IdPath { id }): Path<IdPath>,
    Query(q): Query<HardQuery>,
) -> ApiResult<Response> {
    let svc = ctx.design();
    if q.hard.unwrap_or(false) {
        let a = load_artifact(&ctx, &svc, &user, &id, WorkspaceRole::Admin).await?;
        svc.hard_delete(&a.id).await?;
    } else {
        let a = load_artifact(&ctx, &svc, &user, &id, WorkspaceRole::Editor).await?;
        svc.update_meta(
            &a.id,
            UpdateArtifactReq {
                status: Some("archived".into()),
                ..Default::default()
            },
            &Author::user(&user.id),
        )
        .await?;
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

// ---------------------------------------------------------------------------
// Content + versions
// ---------------------------------------------------------------------------

async fn get_content<S: DesignCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(IdPath { id }): Path<IdPath>,
) -> ApiResult<Response> {
    let svc = ctx.design();
    let a = load_artifact(&ctx, &svc, &user, &id, WorkspaceRole::Viewer).await?;
    let (v, bytes) = svc.head_content(&a).await?;
    content_response(&a, &v, bytes)
}

async fn put_content<S: DesignCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(IdPath { id }): Path<IdPath>,
    Json(req): Json<ContentPutReq>,
) -> ApiResult<Response> {
    let svc = ctx.design();
    let a = load_artifact(&ctx, &svc, &user, &id, WorkspaceRole::Editor).await?;
    let bytes = decode_content(req.content, req.content_b64)?
        .ok_or_else(|| Error::Invalid("send `content` or `content_b64`".into()))?;
    ctx.validate_content(&a.format, &bytes)?;
    let author = Author::from_request(&user.id, req.author_kind.as_deref(), req.session_id)?;
    let kind = if author.kind == "agent" {
        "agent"
    } else {
        "autosave"
    };
    let saved = svc
        .commit_bytes(
            &a,
            bytes,
            SaveOpts {
                base: req.base_version,
                kind: kind.into(),
                author,
                message: req.message.unwrap_or_default(),
                provenance: req.provenance.unwrap_or(Value::Null),
                force: false,
                validate: true,
                change: "content",
            },
        )
        .await?;
    Ok(Json(saved).into_response())
}

async fn get_thumbnail<S: DesignCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(IdPath { id }): Path<IdPath>,
) -> ApiResult<Response> {
    let svc = ctx.design();
    let a = load_artifact(&ctx, &svc, &user, &id, WorkspaceRole::Viewer).await?;
    let sha = a
        .thumb_blob
        .as_deref()
        .ok_or_else(|| Error::NotFound(format!("design artifact {} has no thumbnail", a.id)))?;
    let bytes = svc.blobs().get(sha).await?;
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "image/png")
        .header("x-content-type-options", "nosniff")
        .header(header::ETAG, format!("\"{sha}\""))
        .body(Body::from(bytes))
        .map_err(|e| ApiErr(Error::Internal(format!("build response: {e}"))))
}

async fn list_versions<S: DesignCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(IdPath { id }): Path<IdPath>,
    Query(q): Query<VersionListQuery>,
) -> ApiResult<Response> {
    let svc = ctx.design();
    let a = load_artifact(&ctx, &svc, &user, &id, WorkspaceRole::Viewer).await?;
    let versions = svc
        .store()
        .list_versions(
            &a.id,
            q.kind.as_deref(),
            q.limit.unwrap_or(200),
            q.offset.unwrap_or(0),
        )
        .await?;
    Ok(Json(versions).into_response())
}

/// Named commit: `content`/`content_b64` if sent, else the working copy.
async fn commit_version<S: DesignCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(IdPath { id }): Path<IdPath>,
    Json(req): Json<CommitReq>,
) -> ApiResult<Response> {
    let svc = ctx.design();
    let a = load_artifact(&ctx, &svc, &user, &id, WorkspaceRole::Editor).await?;
    let bytes = match decode_content(req.content, req.content_b64)? {
        Some(b) => b,
        None => svc.working_bytes(&a).await?,
    };
    ctx.validate_content(&a.format, &bytes)?;
    let author = Author::from_request(&user.id, req.author_kind.as_deref(), req.session_id)?;
    let saved = svc
        .commit_named(
            &a,
            Some(bytes),
            req.base_version,
            author,
            req.message,
            req.provenance.unwrap_or(Value::Null),
        )
        .await?;
    Ok((StatusCode::CREATED, Json(saved)).into_response())
}

async fn version_content<S: DesignCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(VersionPath { id, v }): Path<VersionPath>,
) -> ApiResult<Response> {
    let svc = ctx.design();
    let a = load_artifact(&ctx, &svc, &user, &id, WorkspaceRole::Viewer).await?;
    let version = svc.resolve_version(&a, &v).await?;
    let bytes = svc.version_bytes(&version).await?;
    content_response(&a, &version, bytes)
}

async fn approve<S: DesignCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(IdPath { id }): Path<IdPath>,
    Json(req): Json<ApproveReq>,
) -> ApiResult<Response> {
    let svc = ctx.design();
    let a = load_artifact(&ctx, &svc, &user, &id, WorkspaceRole::Editor).await?;
    let updated = svc
        .approve(&a.id, req.version_id.as_deref(), &Author::user(&user.id))
        .await?;
    Ok(Json(updated).into_response())
}

// ---------------------------------------------------------------------------
// Links
// ---------------------------------------------------------------------------

async fn list_links<S: DesignCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(IdPath { id }): Path<IdPath>,
    Query(q): Query<LinksQuery>,
) -> ApiResult<Response> {
    let svc = ctx.design();
    let a = load_artifact(&ctx, &svc, &user, &id, WorkspaceRole::Viewer).await?;
    let dir = q.dir.unwrap_or_else(|| "both".into());
    let mut links = Vec::new();
    if matches!(dir.as_str(), "out" | "both") {
        links.extend(svc.store().links_out(&a.id).await?);
    }
    if matches!(dir.as_str(), "in" | "both") {
        links.extend(svc.store().links_in(&a.id).await?);
    }
    if !matches!(dir.as_str(), "out" | "in" | "both") {
        return Err(ApiErr(Error::Invalid(format!(
            "dir must be out|in|both, not {dir:?}"
        ))));
    }
    // Summaries of the artifacts on the other end — only those the caller
    // may view (a cross-workspace link never leaks a title).
    let mut other: Vec<String> = Vec::new();
    for l in &links {
        let o = if l.src_artifact_id == a.id {
            (l.dst_kind == "artifact").then(|| l.dst_id.clone())
        } else {
            Some(l.src_artifact_id.clone())
        };
        if let Some(o) = o {
            if o != a.id && !other.contains(&o) {
                other.push(o);
            }
        }
    }
    let mut cache: HashMap<Id, bool> = HashMap::new();
    let mut artifacts = Vec::new();
    for x in svc.store().artifacts_by_ids(&other).await? {
        let ok = match cache.get(&x.workspace_id) {
            Some(ok) => *ok,
            None => {
                let ok = ctx
                    .roles()
                    .check(&user, &x.workspace_id, WorkspaceRole::Viewer)
                    .await
                    .is_ok();
                cache.insert(x.workspace_id.clone(), ok);
                ok
            }
        };
        if ok {
            artifacts.push(x);
        }
    }
    Ok(Json(LinksResp { links, artifacts }).into_response())
}

async fn create_link<S: DesignCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(IdPath { id }): Path<IdPath>,
    Json(req): Json<CreateLinkReq>,
) -> ApiResult<Response> {
    let svc = ctx.design();
    let a = load_artifact(&ctx, &svc, &user, &id, WorkspaceRole::Editor).await?;
    if req.dst_kind == "artifact" {
        // Linking to an artifact needs read access to it.
        load_artifact(&ctx, &svc, &user, &req.dst_id, WorkspaceRole::Viewer).await?;
    }
    let link = svc.create_link(&a, req, &Author::user(&user.id)).await?;
    Ok((StatusCode::CREATED, Json(link)).into_response())
}

async fn delete_link<S: DesignCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(LinkPath { id, link_id }): Path<LinkPath>,
) -> ApiResult<Response> {
    let svc = ctx.design();
    let a = load_artifact(&ctx, &svc, &user, &id, WorkspaceRole::Editor).await?;
    svc.delete_link(&a, &link_id).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

// ---------------------------------------------------------------------------
// Search (the References drawer)
// ---------------------------------------------------------------------------

async fn search<S: DesignCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Query(q): Query<ArtifactQuery>,
) -> ApiResult<Response> {
    let svc = ctx.design();
    let ws = visible(&ctx, &svc, &user, q.workspace_id.as_ref()).await?;
    let text = q.q.clone().unwrap_or_default();
    let f = filter_from(q, ws, 50)?;
    let mut hits = Vec::new();
    for (artifact, snippet, score) in svc.store().search(&text, &f).await? {
        let reference_count = svc.store().reference_count(&artifact.id).await?;
        let story_ids = svc.store().story_ids_for(&artifact.id).await?;
        hits.push(SearchHit {
            artifact,
            snippet,
            score,
            reference_count,
            story_ids,
        });
    }
    Ok(Json(hits).into_response())
}

// ---------------------------------------------------------------------------
// Signals
// ---------------------------------------------------------------------------

async fn list_signals<S: DesignCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Query(q): Query<SignalListQuery>,
) -> ApiResult<Response> {
    let svc = ctx.design();
    let ws = match &q.artifact_id {
        Some(aid) => {
            let a = load_artifact(&ctx, &svc, &user, aid, WorkspaceRole::Viewer).await?;
            Some(vec![a.workspace_id])
        }
        None => visible(&ctx, &svc, &user, q.workspace_id.as_ref()).await?,
    };
    let since = norm_time(q.since)?;
    let signals = svc
        .store()
        .list_signals(
            ws.as_deref(),
            q.artifact_id.as_deref(),
            q.kind.as_deref(),
            since.as_deref(),
            q.limit.unwrap_or(200),
        )
        .await?;
    Ok(Json(signals).into_response())
}

async fn create_signal<S: DesignCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<SignalReq>,
) -> ApiResult<Response> {
    let svc = ctx.design();
    let a = load_artifact(&ctx, &svc, &user, &req.artifact_id, WorkspaceRole::Editor).await?;
    let sig = svc.record_signal(&a, req, &Author::user(&user.id)).await?;
    Ok((StatusCode::CREATED, Json(sig)).into_response())
}

// ---------------------------------------------------------------------------
// Admin (Design:Admin via the policy table)
// ---------------------------------------------------------------------------

/// Re-run the idempotent legacy import now (it also runs at startup).
async fn run_import<S: DesignCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(_user)): Extension<AuthUser>,
) -> ApiResult<Response> {
    let svc = ctx.design();
    let report = crate::import::run(&svc).await?;
    Ok(Json(report).into_response())
}

/// Opt-in retention: dry run unless `apply: true`. One artifact needs
/// workspace Admin on it; the whole library needs root.
async fn prune<S: DesignCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<PruneReq>,
) -> ApiResult<Response> {
    let svc = ctx.design();
    match &req.artifact_id {
        Some(aid) => {
            load_artifact(&ctx, &svc, &user, aid, WorkspaceRole::Admin).await?;
        }
        None if !user.is_root => {
            return Err(ApiErr(Error::Forbidden(
                "pruning the whole design library is root-only; pass artifact_id".into(),
            )));
        }
        None => {}
    }
    let report = svc
        .prune(
            req.artifact_id.as_deref(),
            req.apply,
            req.window_secs
                .unwrap_or(crate::retention::DEFAULT_WINDOW_SECS),
        )
        .await?;
    Ok(Json(report).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{Method, Request};
    use http_body_util::BodyExt;
    use otto_core::auth::BoxFuture;
    use tower::ServiceExt;

    /// Allows every workspace except `w-denied`.
    struct Roles;

    impl RoleChecker for Roles {
        fn check<'a>(
            &'a self,
            _user: &'a User,
            workspace_id: &'a Id,
            _min: WorkspaceRole,
        ) -> BoxFuture<'a, otto_core::Result<()>> {
            let denied = workspace_id == "w-denied";
            Box::pin(async move {
                if denied {
                    Err(Error::Forbidden("not a member".into()))
                } else {
                    Ok(())
                }
            })
        }
    }

    #[derive(Clone)]
    struct TestCtx {
        pool: sqlx::SqlitePool,
        dir: Arc<tempfile::TempDir>,
        roles: Arc<dyn RoleChecker>,
    }

    impl DesignCtx for TestCtx {
        fn design(&self) -> DesignService {
            DesignService::new(self.pool.clone(), self.dir.path(), None)
        }
        fn roles(&self) -> &Arc<dyn RoleChecker> {
            &self.roles
        }
        fn validate_content(&self, format: &str, bytes: &[u8]) -> otto_core::Result<()> {
            if format == "html" && bytes.starts_with(b"<script") {
                return Err(Error::Invalid("host validator refused".into()));
            }
            Ok(())
        }
    }

    async fn app() -> (Router, TestCtx) {
        let ctx = TestCtx {
            pool: otto_state::db::test_pool().await,
            dir: Arc::new(tempfile::tempdir().unwrap()),
            roles: Arc::new(Roles),
        };
        assert!(ctx.design().store().ensure_fts().await);
        let user = User {
            id: "u1".into(),
            username: "u1".into(),
            display_name: "U1".into(),
            is_root: false,
            disabled: false,
            created_at: chrono::Utc::now(),
        };
        let r = router::<TestCtx>()
            .with_state(ctx.clone())
            .layer(Extension(AuthUser(user)));
        (r, ctx)
    }

    async fn call(
        app: &Router,
        method: Method,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Vec<u8>, axum::http::HeaderMap) {
        let mut b = Request::builder().method(method).uri(uri);
        let body = match body {
            Some(v) => {
                b = b.header("content-type", "application/json");
                Body::from(v.to_string())
            }
            None => Body::empty(),
        };
        let resp = app.clone().oneshot(b.body(body).unwrap()).await.unwrap();
        let status = resp.status();
        let headers = resp.headers().clone();
        let bytes = resp
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec();
        (status, bytes, headers)
    }

    fn json_of(b: &[u8]) -> Value {
        serde_json::from_slice(b).unwrap_or(Value::Null)
    }

    #[tokio::test]
    async fn project_artifact_content_version_roundtrip() {
        let (app, _ctx) = app().await;
        let (st, b, _) = call(
            &app,
            Method::POST,
            "/design/projects",
            Some(serde_json::json!({ "workspace_id": "w1", "name": "Rewards+" })),
        )
        .await;
        assert_eq!(st, StatusCode::CREATED, "{}", String::from_utf8_lossy(&b));
        let project = json_of(&b);

        let (st, b, _) = call(
            &app,
            Method::POST,
            "/design/artifacts",
            Some(serde_json::json!({
                "workspace_id": "w1", "project_id": project["id"], "format": "html",
                "title": "Landing", "content": "<h1>Win</h1>"
            })),
        )
        .await;
        assert_eq!(st, StatusCode::CREATED, "{}", String::from_utf8_lossy(&b));
        let created = json_of(&b);
        let aid = created["artifact"]["id"].as_str().unwrap().to_string();
        let v1 = created["version"]["id"].as_str().unwrap().to_string();

        // Raw content with version headers + sandbox CSP.
        let (st, b, h) = call(
            &app,
            Method::GET,
            &format!("/design/artifacts/{aid}/content"),
            None,
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(b, b"<h1>Win</h1>");
        assert_eq!(h.get("x-design-version").unwrap(), v1.as_str());
        assert_eq!(h.get("content-security-policy").unwrap(), "sandbox");

        // Save on the right base → v2; on a stale base → 409.
        let (st, b, _) = call(
            &app,
            Method::PUT,
            &format!("/design/artifacts/{aid}/content"),
            Some(serde_json::json!({ "content": "<h1>Win more</h1>", "base_version": v1 })),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{}", String::from_utf8_lossy(&b));
        assert_eq!(json_of(&b)["version"]["seq"], 2);
        let (st, _, _) = call(
            &app,
            Method::PUT,
            &format!("/design/artifacts/{aid}/content"),
            Some(serde_json::json!({ "content": "<h1>Lost</h1>", "base_version": v1 })),
        )
        .await;
        assert_eq!(st, StatusCode::CONFLICT);
        // The host validator hook runs on every content write.
        let (st, _, _) = call(
            &app,
            Method::PUT,
            &format!("/design/artifacts/{aid}/content"),
            Some(serde_json::json!({ "content": "<script>x</script>" })),
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST);

        // Named commit snapshots the working copy; v1 content stays readable.
        let (st, b, _) = call(
            &app,
            Method::POST,
            &format!("/design/artifacts/{aid}/versions"),
            Some(serde_json::json!({ "message": "Hero v2" })),
        )
        .await;
        assert_eq!(st, StatusCode::CREATED, "{}", String::from_utf8_lossy(&b));
        assert_eq!(json_of(&b)["version"]["kind"], "named");
        let (_, b, _) = call(
            &app,
            Method::GET,
            &format!("/design/artifacts/{aid}/versions"),
            None,
        )
        .await;
        assert_eq!(json_of(&b).as_array().unwrap().len(), 3);
        let (st, b, _) = call(
            &app,
            Method::GET,
            &format!("/design/artifacts/{aid}/versions/v1/content"),
            None,
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(b, b"<h1>Win</h1>");

        // Approve, then search finds it (shipped-first ordering is exercised
        // in the store tests).
        let (st, b, _) = call(
            &app,
            Method::POST,
            &format!("/design/artifacts/{aid}/approve"),
            Some(serde_json::json!({})),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(json_of(&b)["status"], "approved");
        let (st, b, _) = call(&app, Method::GET, "/design/search?q=win", None).await;
        assert_eq!(st, StatusCode::OK);
        let hits = json_of(&b);
        assert_eq!(hits.as_array().unwrap().len(), 1, "{hits}");
        assert_eq!(hits[0]["artifact"]["id"], aid.as_str());

        // Detail + archive (soft delete keeps everything).
        let (_, b, _) = call(&app, Method::GET, &format!("/design/artifacts/{aid}"), None).await;
        assert_eq!(json_of(&b)["head"]["seq"], 3);
        let (st, _, _) = call(
            &app,
            Method::DELETE,
            &format!("/design/artifacts/{aid}"),
            None,
        )
        .await;
        assert_eq!(st, StatusCode::NO_CONTENT);
        let (_, b, _) = call(&app, Method::GET, &format!("/design/artifacts/{aid}"), None).await;
        assert_eq!(json_of(&b)["artifact"]["status"], "archived");
        let (_, b, _) = call(&app, Method::GET, "/design/artifacts", None).await;
        assert!(
            json_of(&b).as_array().unwrap().is_empty(),
            "archived hidden by default"
        );
    }

    #[tokio::test]
    async fn workspace_rbac_links_and_signals() {
        let (app, _ctx) = app().await;
        let (st, _, _) = call(
            &app,
            Method::POST,
            "/design/artifacts",
            Some(serde_json::json!({ "workspace_id": "w-denied", "format": "mermaid", "title": "X" })),
        )
        .await;
        assert_eq!(st, StatusCode::FORBIDDEN);

        let mk = |title: &str| serde_json::json!({ "workspace_id": "w1", "format": "scene3d", "title": title });
        let (_, a, _) = call(&app, Method::POST, "/design/artifacts", Some(mk("Card"))).await;
        let (_, b, _) = call(&app, Method::POST, "/design/artifacts", Some(mk("Scene"))).await;
        let a = json_of(&a)["artifact"]["id"].as_str().unwrap().to_string();
        let b = json_of(&b)["artifact"]["id"].as_str().unwrap().to_string();
        let (st, l, _) = call(
            &app,
            Method::POST,
            &format!("/design/artifacts/{b}/links"),
            Some(serde_json::json!({ "rel": "embeds", "dst_kind": "artifact", "dst_id": a })),
        )
        .await;
        assert_eq!(st, StatusCode::CREATED, "{}", String::from_utf8_lossy(&l));
        let link_id = json_of(&l)["id"].as_str().unwrap().to_string();
        // The reverse would be a render cycle.
        let (st, _, _) = call(
            &app,
            Method::POST,
            &format!("/design/artifacts/{a}/links"),
            Some(serde_json::json!({ "rel": "embeds", "dst_kind": "artifact", "dst_id": b })),
        )
        .await;
        assert_eq!(st, StatusCode::CONFLICT);
        let (_, body, _) = call(
            &app,
            Method::GET,
            &format!("/design/artifacts/{a}/links?dir=in"),
            None,
        )
        .await;
        let v = json_of(&body);
        assert_eq!(v["links"].as_array().unwrap().len(), 1);
        assert_eq!(v["artifacts"][0]["id"], b.as_str());
        let (st, _, _) = call(
            &app,
            Method::DELETE,
            &format!("/design/artifacts/{b}/links/{link_id}"),
            None,
        )
        .await;
        assert_eq!(st, StatusCode::NO_CONTENT);

        let (st, s, _) = call(
            &app,
            Method::POST,
            "/design/signals",
            Some(
                serde_json::json!({ "artifact_id": a, "kind": "variant_rejected",
                "payload": { "reason": "too busy" } }),
            ),
        )
        .await;
        assert_eq!(st, StatusCode::CREATED, "{}", String::from_utf8_lossy(&s));
        let (_, s, _) = call(
            &app,
            Method::GET,
            &format!("/design/signals?artifact_id={a}"),
            None,
        )
        .await;
        assert_eq!(json_of(&s)[0]["payload"]["reason"], "too busy");

        // Whole-library prune is root-only; per-artifact dry run is fine.
        let (st, _, _) = call(
            &app,
            Method::POST,
            "/design/admin/prune",
            Some(serde_json::json!({})),
        )
        .await;
        assert_eq!(st, StatusCode::FORBIDDEN);
        let (st, p, _) = call(
            &app,
            Method::POST,
            "/design/admin/prune",
            Some(serde_json::json!({ "artifact_id": a })),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(json_of(&p)["applied"], false);
    }
}
