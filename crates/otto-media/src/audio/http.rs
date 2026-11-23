//! Sounds and music routes (`/product/audio/*`): RBAC like the TTS routes,
//! Product feature gates from the path, then the job's workspace role (Viewer
//! reads, Editor writes).

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use otto_core::auth::AuthUser;
use otto_core::domain::WorkspaceRole;
use otto_core::Id;
use serde::Deserialize;

use super::StartAudioJobReq;
use crate::http::{ApiResult, MediaCtx};

#[derive(Deserialize)]
struct ListQuery {
    workspace_id: Id,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    limit: Option<i64>,
}

pub fn router<S: MediaCtx>() -> Router<S> {
    Router::new()
        .route("/product/audio/engine", get(engine::<S>))
        .route("/product/audio/jobs", get(list::<S>).post(start::<S>))
        .route("/product/audio/jobs/{id}", get(get_one::<S>))
        .route("/product/audio/jobs/{id}/cancel", post(cancel::<S>))
}

async fn engine<S: MediaCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(_user)): Extension<AuthUser>,
) -> Response {
    Json(ctx.media().audio_engine().await).into_response()
}

async fn start<S: MediaCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<StartAudioJobReq>,
) -> ApiResult<Response> {
    ctx.roles()
        .check(&user, &req.workspace_id, WorkspaceRole::Editor)
        .await?;
    let job = ctx.media().start_audio_job(req, &user.id).await?;
    Ok((StatusCode::ACCEPTED, Json(job)).into_response())
}

async fn list<S: MediaCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Query(q): Query<ListQuery>,
) -> ApiResult<Response> {
    ctx.roles()
        .check(&user, &q.workspace_id, WorkspaceRole::Viewer)
        .await?;
    let jobs = ctx
        .media()
        .store()
        .list_audio_jobs(&q.workspace_id, q.kind.as_deref(), q.limit.unwrap_or(50))
        .await?;
    Ok(Json(jobs).into_response())
}

async fn get_one<S: MediaCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Response> {
    let job = ctx.media().get_audio_job(&id).await?;
    ctx.roles()
        .check(&user, &job.workspace_id, WorkspaceRole::Viewer)
        .await?;
    Ok(Json(job).into_response())
}

async fn cancel<S: MediaCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Response> {
    let job = ctx.media().store().require_audio_job(&id).await?;
    ctx.roles()
        .check(&user, &job.workspace_id, WorkspaceRole::Editor)
        .await?;
    Ok(Json(ctx.media().cancel_audio_job(&id).await?).into_response())
}
