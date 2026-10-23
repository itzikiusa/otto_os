//! Operator ICE settings routes (owner + root). Only an opaque Keychain
//! reference is persisted; see `otto_rooms::ice`.
use crate::{
    auth::{require_root, CurrentAuthContext},
    ApiResult, ServerCtx,
};
use axum::{extract::State, Json};
use otto_core::api::*;
use otto_rooms::{auth::owner_auth, ice};

pub async fn get_settings(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
) -> ApiResult<Json<RoomSettings>> {
    owner_auth(&auth)?;
    require_root(&auth.effective_user)?;
    Ok(Json(ice::settings(&ctx).await?))
}
pub async fn put_settings(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Json(req): Json<RoomSettingsReq>,
) -> ApiResult<Json<RoomSettings>> {
    owner_auth(&auth)?;
    require_root(&auth.effective_user)?;
    Ok(Json(ice::put_settings(&ctx, req).await?))
}
