//! Room owner routes. Session ownership and the workspace role are checked
//! here; the room lifecycle itself is `otto_rooms::lifecycle`.
use crate::{
    auth::{require_ws_role, CurrentAuthContext},
    ApiResult, ServerCtx,
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use otto_core::{api::*, domain::WorkspaceRole, Error, Id};
use otto_rooms::{auth::owner_auth, lifecycle};

pub async fn create(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path(id): Path<String>,
    Json(req): Json<CreateRoomReq>,
) -> ApiResult<Json<RoomCredential>> {
    owner_auth(&auth)?;
    let sid: Id = id;
    let session = ctx.manager.get(&sid).await?;
    if session.created_by != auth.effective_user.id {
        return Err(Error::Forbidden("Only the session owner can start a room".into()).into());
    }
    require_ws_role(
        &ctx,
        &auth.effective_user,
        &session.workspace_id,
        WorkspaceRole::Editor,
    )
    .await?;
    let actor = auth.real_user.id.as_str();
    Ok(Json(
        lifecycle::open(&ctx, &session, actor, &req.name).await?,
    ))
}
pub async fn list(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
) -> ApiResult<Json<Vec<RoomSnapshot>>> {
    owner_auth(&auth)?;
    Ok(Json(
        lifecycle::list(&ctx, auth.effective_user.id.as_str()).await?,
    ))
}
pub async fn end(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    owner_auth(&auth)?;
    lifecycle::end(&ctx, &id, auth.effective_user.id.as_str()).await?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn invite(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path(id): Path<String>,
    Json(req): Json<RoomInviteReq>,
) -> ApiResult<Json<RoomInvite>> {
    owner_auth(&auth)?;
    let (owner, actor) = (auth.effective_user.id.as_str(), auth.real_user.id.as_str());
    Ok(Json(
        lifecycle::invite(&ctx, &id, owner, actor, req.role).await?,
    ))
}
pub async fn join(
    State(ctx): State<ServerCtx>,
    Json(req): Json<JoinRoomReq>,
) -> ApiResult<Json<RoomCredential>> {
    Ok(Json(
        ctx.rooms.join(&req.room_id, &req.invite, &req.name).await?,
    ))
}
