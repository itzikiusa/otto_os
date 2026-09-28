use super::{
    auth::{eligible, owner_auth},
    socket,
};
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
    eligible(&session)?;
    let handle = ctx
        .manager
        .live_handle(&sid)
        .filter(|h| !h.has_exited())
        .ok_or_else(|| Error::Conflict("Start the local session before opening a room".into()))?;
    let credentials = ctx
        .rooms
        .create(
            sid.as_str(),
            auth.effective_user.id.as_str(),
            &req.name,
            &session.title,
            &session.provider,
        )
        .await?;
    if let Err(error) = ctx
        .manager
        .begin_room_authority(
            &sid,
            &credentials.room_id,
            &auth.effective_user.id,
            &credentials.member_id,
        )
        .await
    {
        ctx.rooms.forget(&credentials.room_id).await;
        return Err(error.into());
    }
    let room = ctx.rooms.get(&credentials.room_id).await?;
    {
        let mut room = room.lock().await;
        room.spawn_seq = handle.spawn_seq();
        socket::sync_authority(&ctx, &mut room).await?;
    }
    super::audit(
        &ctx,
        &credentials.room_id,
        Some(auth.real_user.id),
        Some(credentials.member_id.clone()),
        "create",
    )
    .await;
    socket::maintain(ctx, room);
    Ok(Json(credentials))
}
pub async fn list(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
) -> ApiResult<Json<Vec<RoomSnapshot>>> {
    owner_auth(&auth)?;
    let mut result = vec![];
    for shared in ctx.rooms.all().await {
        let mut room = shared.lock().await;
        if room.owner == auth.effective_user.id.as_str() && room.live().is_ok() {
            socket::sync_authority(&ctx, &mut room).await?;
            result.push(room.snapshot(&room.host)?);
        }
    }
    Ok(Json(result))
}
pub async fn end(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    owner_auth(&auth)?;
    let shared = ctx.rooms.get(&id).await?;
    let mut room = shared.lock().await;
    if room.owner != auth.effective_user.id.as_str() {
        return Err(Error::Forbidden("Only the room owner can end sharing".into()).into());
    }
    socket::end_room(&ctx, &mut room, "The host ended the room").await;
    let owner = room.owner.clone();
    drop(room);
    super::audit(&ctx, &id, Some(owner), None, "end").await;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn invite(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path(id): Path<String>,
    Json(req): Json<RoomInviteReq>,
) -> ApiResult<Json<RoomInvite>> {
    owner_auth(&auth)?;
    let configured = otto_state::SettingsRepo::new(ctx.pool.clone())
        .get("room_public_origin")
        .await?;
    let origin = configured
        .as_ref()
        .and_then(|v| v.as_str())
        .unwrap_or(&ctx.base_url);
    let origin = super::ice::valid_origin(origin)?;
    let invite = ctx
        .rooms
        .invite(&id, auth.effective_user.id.as_str(), req.role)
        .await?;
    let url = format!("{origin}/#/room/{id}/{invite}");
    super::audit(&ctx, &id, Some(auth.real_user.id), None, "invite").await;
    Ok(Json(RoomInvite {
        invite,
        url,
        expires_at: (chrono::Utc::now() + chrono::Duration::minutes(10)).to_rfc3339(),
    }))
}
pub async fn join(
    State(ctx): State<ServerCtx>,
    Json(req): Json<JoinRoomReq>,
) -> ApiResult<Json<RoomCredential>> {
    Ok(Json(
        ctx.rooms.join(&req.room_id, &req.invite, &req.name).await?,
    ))
}
