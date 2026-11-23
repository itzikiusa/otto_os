//! Axum REST router for `otto-media`:
//!
//! 3D Generation Providers, Presets, Jobs & Workspace Export (`/api/v1/design/3d/*`):
//! - `GET    /design/3d/providers`                         (Design:View)
//! - `POST   /design/3d/providers`                         (Design:Admin + RouteClass::Admin + non-agent)
//! - `PATCH  /design/3d/providers/{id}`                    (Design:Admin + RouteClass::Admin + non-agent)
//! - `DELETE /design/3d/providers/{id}`                    (Design:Admin + RouteClass::Admin + non-agent)
//! - `POST   /design/3d/providers/{id}/test`               (Design:Admin + RouteClass::Admin + non-agent)
//! - `GET    /design/3d/presets`                           (Design:View)
//! - `POST   /design/3d/jobs`                              (Design:Edit + ws Editor)
//! - `GET    /design/3d/jobs`                              (Design:View + ws Viewer)
//! - `GET    /design/3d/jobs/{id}`                         (Design:View + ws Viewer)
//! - `POST   /design/3d/jobs/{id}/cancel`                  (Design:Edit + ws Editor)
//! - `POST   /design/3d/artifacts/{id}/export-workspace`   (Design:Edit + ws Editor)
//!
//! Built-in & Extensible TTS (`/api/v1/product/tts/*`):
//! - `GET    /product/tts/providers`                       (Product:View)
//! - `POST   /product/tts/providers`                       (Product:Admin + RouteClass::Admin + non-agent)
//! - `PATCH  /product/tts/providers/{id}`                  (Product:Admin + RouteClass::Admin + non-agent)
//! - `DELETE /product/tts/providers/{id}`                  (Product:Admin + RouteClass::Admin + non-agent)
//! - `POST   /product/tts/providers/{id}/test`             (Product:Admin + RouteClass::Admin + non-agent)
//! - `GET    /product/tts/providers/{id}/voices`           (Product:View)
//! - `POST   /product/tts/synthesize`                      (Product:Edit + ws Editor)
//! - `GET    /product/tts/clips`                           (Product:View + ws Viewer)
//! - `GET    /product/tts/clips/{id}`                      (Product:View + ws Viewer)
//! - `GET    /product/tts/clips/{id}/audio`                (Product:View + ws Viewer)
//! - `DELETE /product/tts/clips/{id}`                      (Product:Edit + ws Editor)
//! - `POST   /product/tts/clips/{id}/export-workspace`     (Product:Edit + ws Editor)
//!
//! Sounds and music: `/product/audio/*` in [`crate::audio::http`].

use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Extension, Json, Router};
use otto_core::auth::{AuthUser, RoleChecker};
use otto_core::domain::{User, Workspace, WorkspaceRole};
use otto_core::{Error, Id};
use otto_design::DesignService;
use otto_state::WorkspacesRepo;
use serde::Deserialize;
use serde_json::json;

use crate::productions::{AgentPick, ProductionPlanner};
use crate::service::MediaService;
use crate::types::{
    builtin_game_asset_presets, CreateMedia3dProviderReq, CreateTtsProviderReq, ExportWorkspaceReq,
    StartMedia3dJobReq, SynthesizeTtsReq, UpdateMedia3dProviderReq, UpdateTtsProviderReq,
};

/// Allow base64-encoded reference images (up to 12 MB raw ≈ 16 MB base64) + multi-modal controls.
const JOB_BODY_LIMIT: usize = 24 * 1024 * 1024;

pub trait MediaCtx: Clone + Send + Sync + 'static {
    fn media(&self) -> &Arc<MediaService>;
    fn design(&self) -> DesignService;
    fn roles(&self) -> &Arc<dyn RoleChecker>;
    fn workspaces(&self) -> &WorkspacesRepo;
    /// The planner agent turn of a production in `ws`, run as `user` with
    /// the production's `provider`, `model` and `effort` (`None`: the
    /// workspace default agent, the provider's default model, the model's
    /// default effort); a pick a session could not run is `Invalid` (the
    /// host: `productions::agent::agent_planner` on its sessions).
    fn production_planner(
        &self,
        ws: &Workspace,
        user: &User,
        pick: &AgentPick,
    ) -> impl Future<Output = otto_core::Result<Arc<dyn ProductionPlanner>>> + Send;
}

pub(crate) struct MediaErr(Error);

impl From<Error> for MediaErr {
    fn from(e: Error) -> Self {
        Self(e)
    }
}

impl IntoResponse for MediaErr {
    fn into_response(self) -> Response {
        let (status, code) = match &self.0 {
            Error::NotFound(_) => (StatusCode::NOT_FOUND, "not_found"),
            Error::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            Error::Forbidden(_) => (StatusCode::FORBIDDEN, "forbidden"),
            Error::Conflict(_) => (StatusCode::CONFLICT, "conflict"),
            Error::Invalid(_) => (StatusCode::BAD_REQUEST, "invalid"),
            Error::PayloadTooLarge(_) => (StatusCode::PAYLOAD_TOO_LARGE, "payload_too_large"),
            Error::UnsupportedMedia(_) => (StatusCode::UNSUPPORTED_MEDIA_TYPE, "unsupported_media"),
            Error::Upstream(_) => (StatusCode::BAD_GATEWAY, "upstream"),
            Error::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal"),
        };
        // A 500's detail (host paths, SQLite, Keychain) goes to the log, not the caller.
        if status == StatusCode::INTERNAL_SERVER_ERROR {
            tracing::error!("media internal error: {}", self.0);
        }
        (
            status,
            Json(json!({ "code": code, "message": self.0.public_message() })),
        )
            .into_response()
    }
}

pub(crate) type ApiResult<T> = std::result::Result<T, MediaErr>;

fn deny_if_agent(action: &str) -> Result<(), MediaErr> {
    if otto_core::auth::request_is_agent() {
        return Err(MediaErr(Error::Forbidden(format!(
            "an agent session credential cannot {action} — a person signed in to Otto must do this"
        ))));
    }
    Ok(())
}

async fn workspace_root_of<S: MediaCtx>(ctx: &S, workspace_id: &Id) -> ApiResult<PathBuf> {
    let ws = ctx.workspaces().get(workspace_id).await?;
    if ws.root_path.trim().is_empty() {
        return Err(Error::Invalid(format!(
            "workspace '{}' has no root_path configured for export",
            ws.name
        ))
        .into());
    }
    Ok(PathBuf::from(ws.root_path))
}

#[derive(Deserialize)]
struct IdPath {
    id: Id,
}

#[derive(Deserialize, Default)]
struct JobsQuery {
    #[serde(default)]
    workspace_id: Option<Id>,
    #[serde(default)]
    artifact_id: Option<Id>,
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Deserialize)]
struct ClipsQuery {
    workspace_id: Id,
    #[serde(default)]
    story_id: Option<Id>,
    #[serde(default)]
    artifact_id: Option<Id>,
    /// `voice` | `sound` | `music`; every kind when omitted.
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    limit: Option<i64>,
}

pub fn api_router<S: MediaCtx>() -> Router<S> {
    Router::new()
        // ── 3D Providers & Presets ──────────────────────────────────────────
        .route(
            "/design/3d/providers",
            get(list_3d_providers::<S>).post(create_3d_provider::<S>),
        )
        .route(
            "/design/3d/providers/{id}",
            patch(update_3d_provider::<S>).delete(delete_3d_provider::<S>),
        )
        .route(
            "/design/3d/providers/{id}/test",
            post(test_3d_provider::<S>),
        )
        .route("/design/3d/presets", get(list_3d_presets))
        // ── 3D Jobs & Artifact Workspace Export ─────────────────────────────
        .route(
            "/design/3d/jobs",
            get(list_3d_jobs::<S>)
                .post(start_3d_job::<S>)
                .layer(DefaultBodyLimit::max(JOB_BODY_LIMIT)),
        )
        .route("/design/3d/jobs/{id}", get(get_3d_job::<S>))
        .route("/design/3d/jobs/{id}/cancel", post(cancel_3d_job::<S>))
        .route(
            "/design/3d/artifacts/{id}/export-workspace",
            post(export_3d_artifact::<S>),
        )
        // ── TTS Providers, Voices, Synthesis & Clips ────────────────────────
        .route(
            "/product/tts/providers",
            get(list_tts_providers::<S>).post(create_tts_provider::<S>),
        )
        .route(
            "/product/tts/providers/{id}",
            patch(update_tts_provider::<S>).delete(delete_tts_provider::<S>),
        )
        .route(
            "/product/tts/providers/{id}/test",
            post(test_tts_provider::<S>),
        )
        .route(
            "/product/tts/providers/{id}/voices",
            get(list_tts_voices::<S>),
        )
        .route("/product/tts/synthesize", post(synthesize_tts::<S>))
        .route("/product/tts/clips", get(list_tts_clips::<S>))
        .route(
            "/product/tts/clips/{id}",
            get(get_tts_clip::<S>).delete(delete_tts_clip::<S>),
        )
        .route(
            "/product/tts/clips/{id}/audio",
            get(serve_tts_clip_audio::<S>),
        )
        .route(
            "/product/tts/clips/{id}/export-workspace",
            post(export_tts_clip::<S>),
        )
        .merge(crate::productions::http::router::<S>())
        .merge(crate::audio::http::router::<S>())
}

// ---------------------------------------------------------------------------
// 3D Provider Handlers
// ---------------------------------------------------------------------------

async fn list_3d_providers<S: MediaCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(_user)): Extension<AuthUser>,
) -> ApiResult<Response> {
    let list = ctx.media().list_3d_providers().await?;
    Ok(Json(list).into_response())
}

async fn create_3d_provider<S: MediaCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(_user)): Extension<AuthUser>,
    Json(req): Json<CreateMedia3dProviderReq>,
) -> ApiResult<Response> {
    deny_if_agent("register 3D generation providers")?;
    let p = ctx.media().create_3d_provider(req).await?;
    Ok((StatusCode::CREATED, Json(p)).into_response())
}

async fn update_3d_provider<S: MediaCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(_user)): Extension<AuthUser>,
    Path(IdPath { id }): Path<IdPath>,
    Json(req): Json<UpdateMedia3dProviderReq>,
) -> ApiResult<Response> {
    deny_if_agent("modify 3D generation providers")?;
    let p = ctx.media().update_3d_provider(&id, req).await?;
    Ok(Json(p).into_response())
}

async fn delete_3d_provider<S: MediaCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(_user)): Extension<AuthUser>,
    Path(IdPath { id }): Path<IdPath>,
) -> ApiResult<Response> {
    deny_if_agent("delete 3D generation providers")?;
    ctx.media().delete_3d_provider(&id).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn test_3d_provider<S: MediaCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(_user)): Extension<AuthUser>,
    Path(IdPath { id }): Path<IdPath>,
) -> ApiResult<Response> {
    deny_if_agent("test 3D generation providers")?;
    let res = ctx.media().test_3d_provider(&id).await?;
    Ok(Json(res).into_response())
}

async fn list_3d_presets(Extension(AuthUser(_user)): Extension<AuthUser>) -> Response {
    Json(builtin_game_asset_presets()).into_response()
}

// ---------------------------------------------------------------------------
// 3D Jobs & Workspace Export Handlers
// ---------------------------------------------------------------------------

async fn start_3d_job<S: MediaCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<StartMedia3dJobReq>,
) -> ApiResult<Response> {
    ctx.roles()
        .check(&user, &req.workspace_id, WorkspaceRole::Editor)
        .await?;
    let ws_root = if req.export_relative_path.is_some() {
        Some(workspace_root_of(&ctx, &req.workspace_id).await?)
    } else {
        None
    };
    let job = ctx.media().start_3d_job(req, &user.id, ws_root).await?;
    Ok((StatusCode::ACCEPTED, Json(job)).into_response())
}

async fn list_3d_jobs<S: MediaCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Query(q): Query<JobsQuery>,
) -> ApiResult<Response> {
    if let Some(ws) = q.workspace_id.as_ref().filter(|s| !s.trim().is_empty()) {
        ctx.roles().check(&user, ws, WorkspaceRole::Viewer).await?;
        let jobs = ctx
            .media()
            .store()
            .list_3d_jobs(Some(ws), q.artifact_id.as_deref(), q.limit.unwrap_or(50))
            .await?;
        return Ok(Json(jobs).into_response());
    }
    if let Some(aid) = q.artifact_id.as_deref().filter(|s| !s.trim().is_empty()) {
        let a = ctx.design().store().require_artifact(aid).await?;
        ctx.roles()
            .check(&user, &a.workspace_id, WorkspaceRole::Viewer)
            .await?;
        let jobs = ctx
            .media()
            .store()
            .list_3d_jobs(Some(&a.workspace_id), Some(aid), q.limit.unwrap_or(50))
            .await?;
        return Ok(Json(jobs).into_response());
    }
    // Without filter: return only jobs from workspaces this user can view.
    let all = ctx
        .media()
        .store()
        .list_3d_jobs(None, None, q.limit.unwrap_or(50))
        .await?;
    let mut visible = Vec::with_capacity(all.len());
    for job in all {
        if ctx
            .roles()
            .check(&user, &job.workspace_id, WorkspaceRole::Viewer)
            .await
            .is_ok()
        {
            visible.push(job);
        }
    }
    Ok(Json(visible).into_response())
}

async fn get_3d_job<S: MediaCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(IdPath { id }): Path<IdPath>,
) -> ApiResult<Response> {
    let job = ctx.media().store().require_3d_job(&id).await?;
    ctx.roles()
        .check(&user, &job.workspace_id, WorkspaceRole::Viewer)
        .await?;
    Ok(Json(job).into_response())
}

async fn cancel_3d_job<S: MediaCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(IdPath { id }): Path<IdPath>,
) -> ApiResult<Response> {
    let job = ctx.media().store().require_3d_job(&id).await?;
    ctx.roles()
        .check(&user, &job.workspace_id, WorkspaceRole::Editor)
        .await?;
    let updated = ctx.media().cancel_3d_job(&id).await?;
    Ok(Json(updated).into_response())
}

async fn export_3d_artifact<S: MediaCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(IdPath { id }): Path<IdPath>,
    Json(req): Json<ExportWorkspaceReq>,
) -> ApiResult<Response> {
    let artifact = ctx.design().store().require_artifact(&id).await?;
    ctx.roles()
        .check(&user, &artifact.workspace_id, WorkspaceRole::Editor)
        .await?;
    let ws_root = workspace_root_of(&ctx, &artifact.workspace_id).await?;
    let res = ctx
        .media()
        .export_3d_artifact_to_workspace(
            &id,
            &ws_root,
            &req.relative_path,
            req.version_id.as_deref(),
            req.overwrite,
        )
        .await?;
    Ok(Json(res).into_response())
}

// ---------------------------------------------------------------------------
// TTS Provider, Voice, Synthesis & Clip Handlers
// ---------------------------------------------------------------------------

async fn list_tts_providers<S: MediaCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(_user)): Extension<AuthUser>,
) -> ApiResult<Response> {
    let list = ctx.media().list_tts_providers().await?;
    Ok(Json(list).into_response())
}

async fn create_tts_provider<S: MediaCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(_user)): Extension<AuthUser>,
    Json(req): Json<CreateTtsProviderReq>,
) -> ApiResult<Response> {
    deny_if_agent("register TTS providers")?;
    let p = ctx.media().create_tts_provider(req).await?;
    Ok((StatusCode::CREATED, Json(p)).into_response())
}

async fn update_tts_provider<S: MediaCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(_user)): Extension<AuthUser>,
    Path(IdPath { id }): Path<IdPath>,
    Json(req): Json<UpdateTtsProviderReq>,
) -> ApiResult<Response> {
    deny_if_agent("modify TTS providers")?;
    let p = ctx.media().update_tts_provider(&id, req).await?;
    Ok(Json(p).into_response())
}

async fn delete_tts_provider<S: MediaCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(_user)): Extension<AuthUser>,
    Path(IdPath { id }): Path<IdPath>,
) -> ApiResult<Response> {
    deny_if_agent("delete TTS providers")?;
    ctx.media().delete_tts_provider(&id).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn test_tts_provider<S: MediaCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(_user)): Extension<AuthUser>,
    Path(IdPath { id }): Path<IdPath>,
) -> ApiResult<Response> {
    deny_if_agent("test TTS providers")?;
    let res = ctx.media().test_tts_provider(&id).await?;
    Ok(Json(res).into_response())
}

async fn list_tts_voices<S: MediaCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(_user)): Extension<AuthUser>,
    Path(IdPath { id }): Path<IdPath>,
) -> ApiResult<Response> {
    let voices = ctx.media().list_tts_voices(&id).await?;
    Ok(Json(voices).into_response())
}

async fn synthesize_tts<S: MediaCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<SynthesizeTtsReq>,
) -> ApiResult<Response> {
    ctx.roles()
        .check(&user, &req.workspace_id, WorkspaceRole::Editor)
        .await?;
    let clip = ctx.media().synthesize_tts(req, &user.id).await?;
    Ok((StatusCode::CREATED, Json(clip)).into_response())
}

async fn list_tts_clips<S: MediaCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Query(q): Query<ClipsQuery>,
) -> ApiResult<Response> {
    ctx.roles()
        .check(&user, &q.workspace_id, WorkspaceRole::Viewer)
        .await?;
    let clips = ctx
        .media()
        .store()
        .list_tts_clips(
            &q.workspace_id,
            q.story_id.as_deref(),
            q.artifact_id.as_deref(),
            q.kind.as_deref(),
            q.limit.unwrap_or(100),
        )
        .await?;
    Ok(Json(clips).into_response())
}

async fn get_tts_clip<S: MediaCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(IdPath { id }): Path<IdPath>,
) -> ApiResult<Response> {
    let rec = ctx.media().store().require_tts_clip(&id).await?;
    ctx.roles()
        .check(&user, &rec.clip.workspace_id, WorkspaceRole::Viewer)
        .await?;
    Ok(Json(rec.clip).into_response())
}

async fn serve_tts_clip_audio<S: MediaCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(IdPath { id }): Path<IdPath>,
) -> ApiResult<Response> {
    let rec = ctx.media().store().require_tts_clip(&id).await?;
    ctx.roles()
        .check(&user, &rec.clip.workspace_id, WorkspaceRole::Viewer)
        .await?;
    let bytes = ctx.media().read_tts_clip_bytes(&rec).await?;
    let resp = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, &rec.clip.mime)
        .header(header::CACHE_CONTROL, "private, max-age=3600")
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .body(Body::from(bytes))
        .map_err(|e| Error::Internal(format!("build audio response: {e}")))?;
    Ok(resp)
}

async fn delete_tts_clip<S: MediaCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(IdPath { id }): Path<IdPath>,
) -> ApiResult<Response> {
    let rec = ctx.media().store().require_tts_clip(&id).await?;
    ctx.roles()
        .check(&user, &rec.clip.workspace_id, WorkspaceRole::Editor)
        .await?;
    ctx.media().delete_tts_clip(&id).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn export_tts_clip<S: MediaCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(IdPath { id }): Path<IdPath>,
    Json(req): Json<ExportWorkspaceReq>,
) -> ApiResult<Response> {
    let rec = ctx.media().store().require_tts_clip(&id).await?;
    ctx.roles()
        .check(&user, &rec.clip.workspace_id, WorkspaceRole::Editor)
        .await?;
    let ws_root = workspace_root_of(&ctx, &rec.clip.workspace_id).await?;
    let res = ctx
        .media()
        .export_tts_clip_to_workspace(&rec, &ws_root, &req.relative_path, req.overwrite)
        .await?;
    Ok(Json(res).into_response())
}
