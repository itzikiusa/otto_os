use super::{registry::SharedRoom, types::*};
use crate::{auth::CurrentAuthContext, ApiResult, ServerCtx};
use axum::{
    extract::State,
    http::{header, HeaderMap},
    Json,
};
use std::time::{Duration, Instant};

pub(super) async fn create(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Json(request): Json<Create>,
) -> ApiResult<(HeaderMap, Json<Credential>)> {
    crate::rooms::owner_auth(&auth)?;
    let configured = otto_state::SettingsRepo::new(ctx.pool.clone())
        .get("room_public_origin")
        .await?;
    let origin = crate::rooms::valid_origin(
        configured
            .as_ref()
            .and_then(|v| v.as_str())
            .unwrap_or(&ctx.base_url),
    )?;
    let mut credential = ctx
        .game_rooms
        .create(&auth.effective_user.id, &request.name, request.config)
        .await?;
    credential.invite_url = credential.invite.as_ref().map(|invite| {
        format!(
            "{origin}/#/game-room/{}?invite={invite}",
            credential.room_id
        )
    });
    let shared = ctx.game_rooms.get(&credential.room_id).await?;
    maintain(ctx, shared);
    Ok((private_headers(), Json(credential)))
}
pub(super) async fn join(
    State(ctx): State<ServerCtx>,
    Json(request): Json<Join>,
) -> ApiResult<(HeaderMap, Json<Credential>)> {
    let credential = ctx
        .game_rooms
        .join(&request.room_id, &request.invite, &request.name)
        .await?;
    Ok((private_headers(), Json(credential)))
}
fn private_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CACHE_CONTROL,
        "no-store".parse().expect("static header"),
    );
    headers
}
fn maintain(ctx: ServerCtx, shared: SharedRoom) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        loop {
            tick.tick().await;
            let mut room = shared.lock().await;
            room.maintain(Instant::now());
            if room.phase == Phase::Closed {
                let id = room.id.clone();
                drop(room);
                ctx.game_rooms.forget(&id).await;
                break;
            }
        }
    });
}
