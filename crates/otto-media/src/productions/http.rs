//! Productions routes (`/design/productions*`, contract v1). RBAC like the
//! 3D job routes: Design feature gates from the path, then the production's
//! workspace role (Viewer reads, Editor everything else).

use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use otto_core::auth::AuthUser;
use otto_core::domain::{User, WorkspaceRole};
use otto_core::Id;
use serde::Deserialize;

use super::{CreateProductionReq, PatchProductionReq, RevertReq, SteerReq};
use crate::http::{ApiResult, MediaCtx};

#[derive(Deserialize)]
struct ListQuery {
    workspace_id: Id,
}

pub fn router<S: MediaCtx>() -> Router<S> {
    Router::new()
        .route("/design/productions", get(list::<S>).post(create::<S>))
        .route(
            "/design/productions/{id}",
            get(get_one::<S>).patch(patch::<S>),
        )
        .route("/design/productions/{id}/start", post(start::<S>))
        .route("/design/productions/{id}/cancel", post(cancel::<S>))
        .route("/design/productions/{id}/steer", post(steer::<S>))
        .route("/design/productions/{id}/revert", post(revert::<S>))
        .route("/design/productions/{id}/scene", post(rebuild_scene::<S>))
        .route(
            "/design/productions/{id}/items/{item_id}/preview",
            get(preview::<S>),
        )
        .route(
            "/design/productions/{id}/items/{item_id}/redraw",
            post(redraw::<S>),
        )
        .route("/design/creations", get(creations::<S>))
}

/// Everything made in Create across every workspace the caller is a Viewer
/// of (Create is not workspace-scoped for a person; each piece still lives
/// in the workspace it was made in).
async fn creations<S: MediaCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
) -> ApiResult<Response> {
    let mut seen = Vec::new();
    for ws in ctx.workspaces().list_all().await? {
        let role = ctx.roles().check(&user, &ws.id, WorkspaceRole::Viewer);
        if role.await.is_ok() {
            seen.push(ws);
        }
    }
    Ok(Json(ctx.media().creations(&seen).await?).into_response())
}

/// The production's workspace role check; returns the user for the call.
async fn gate<S: MediaCtx>(ctx: &S, user: &User, id: &str, min: WorkspaceRole) -> ApiResult<()> {
    let p = ctx.media().store().require_production(id).await?;
    ctx.roles().check(user, &p.workspace_id, min).await?;
    Ok(())
}

async fn create<S: MediaCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<CreateProductionReq>,
) -> ApiResult<Response> {
    ctx.roles()
        .check(&user, &req.workspace_id, WorkspaceRole::Editor)
        .await?;
    // A new scene of a project: the caller sees the creation it names.
    if let Some(of) = &req.project_id {
        gate(&ctx, &user, of, WorkspaceRole::Viewer).await?;
    }
    let ws = ctx.workspaces().get(&req.workspace_id).await?;
    let planner = ctx.production_planner(&ws, &user, &req.agent()).await?;
    let p = ctx
        .media()
        .create_production(req, &user.id, planner)
        .await?;
    Ok((StatusCode::CREATED, Json(p)).into_response())
}

async fn list<S: MediaCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Query(q): Query<ListQuery>,
) -> ApiResult<Response> {
    ctx.roles()
        .check(&user, &q.workspace_id, WorkspaceRole::Viewer)
        .await?;
    let ws = ctx.workspaces().get(&q.workspace_id).await?;
    let list = ctx.media().list_productions(&[ws], 200).await?;
    Ok(Json(list).into_response())
}

async fn get_one<S: MediaCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Response> {
    gate(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    Ok(Json(ctx.media().get_production(&id).await?).into_response())
}

/// An item's picture (404 while it has none); `?rev=` only busts caches.
async fn preview<S: MediaCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path((id, item_id)): Path<(Id, String)>,
) -> ApiResult<Response> {
    gate(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    let (bytes, mime) = ctx.media().production_preview(&id, &item_id).await?;
    let headers = [
        (header::CONTENT_TYPE, mime),
        (header::CACHE_CONTROL, "private, no-cache"),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
    ];
    Ok((headers, bytes).into_response())
}

/// "Not this one": the piece is drawn again (another seed), at its turn.
async fn redraw<S: MediaCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path((id, item_id)): Path<(Id, String)>,
) -> ApiResult<Response> {
    gate(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    Ok(Json(ctx.media().redraw_item(&id, &item_id).await?).into_response())
}

async fn patch<S: MediaCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<PatchProductionReq>,
) -> ApiResult<Response> {
    gate(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    Ok(Json(ctx.media().patch_production(&id, req).await?).into_response())
}

async fn start<S: MediaCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Response> {
    gate(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    Ok(Json(ctx.media().start_production(&id, &user.id).await?).into_response())
}

/// The scene written again from what is made, with the current builder.
async fn rebuild_scene<S: MediaCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Response> {
    gate(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    Ok(Json(ctx.media().rebuild_scene(&id, &user.id).await?).into_response())
}

async fn cancel<S: MediaCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Response> {
    gate(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    Ok(Json(ctx.media().cancel_production(&id).await?).into_response())
}

async fn steer<S: MediaCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<SteerReq>,
) -> ApiResult<Response> {
    let p = ctx.media().store().require_production(&id).await?;
    ctx.roles()
        .check(&user, &p.workspace_id, WorkspaceRole::Editor)
        .await?;
    let ws = ctx.workspaces().get(&p.workspace_id).await?;
    let planner = ctx.production_planner(&ws, &user, &p.pick()).await?;
    Ok(Json(
        ctx.media()
            .steer_production(&id, req, planner, &user.id)
            .await?,
    )
    .into_response())
}

async fn revert<S: MediaCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<RevertReq>,
) -> ApiResult<Response> {
    gate(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    let p = ctx
        .media()
        .revert_production(&id, &req.revision_id, &user.id)
        .await?;
    Ok(Json(p).into_response())
}
