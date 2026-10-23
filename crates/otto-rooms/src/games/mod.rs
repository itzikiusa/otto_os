//! Multiplayer game capabilities (host + guests) are isolated from sessions
//! and owner credentials. Nothing here is persisted.
mod registry;
#[cfg(test)]
mod tests;
mod types;
pub use registry::{
    Member, Notice, Registry, Room, SharedRoom, Target, GRACE, MAX_MEMBER_ENTRIES, RATE_LIMITED,
};
use std::time::{Duration, Instant};
pub use types::*;

/// Create a game room for the owner (the handler authenticated them): the
/// invite URL points at the configured public room origin.
pub async fn open<C: crate::RoomsCtx>(
    ctx: &C,
    owner: &str,
    request: Create,
) -> otto_core::Result<Credential> {
    let origin = crate::ice::public_origin(ctx).await?;
    let mut credential = ctx
        .game_rooms()
        .create(owner, &request.name, request.config)
        .await?;
    credential.invite_url = credential.invite.as_ref().map(|invite| {
        format!(
            "{origin}/#/game-room/{}?invite={invite}",
            credential.room_id
        )
    });
    let shared = ctx.game_rooms().get(&credential.room_id).await?;
    maintain(ctx.clone(), shared);
    Ok(credential)
}

/// One maintenance task per game room: pauses, grace expiry and closing; a
/// closed room leaves the registry.
pub fn maintain<C: crate::RoomsCtx>(ctx: C, shared: SharedRoom) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        loop {
            tick.tick().await;
            let mut room = shared.lock().await;
            room.maintain(Instant::now());
            if room.phase == Phase::Closed {
                let id = room.id.clone();
                drop(room);
                ctx.game_rooms().forget(&id).await;
                break;
            }
        }
    });
}
