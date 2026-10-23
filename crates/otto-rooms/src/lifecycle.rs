//! Owner-side room lifecycle: open a room on a live session, list, end and
//! invite. The handlers authenticate the owner (and check session ownership
//! and the workspace role) before calling in; `owner` is the effective user
//! and `actor` the real user recorded in the audit trail.
use crate::{auth::eligible, authority, ice, RoomsCtx};
use otto_core::{api::*, domain::Session, Error, Result};

/// Open a room on the owner's live local session: create the registry entry,
/// take the manager's room authority (pinned to this process), start the
/// maintenance worker.
pub async fn open<C: RoomsCtx>(
    ctx: &C,
    session: &Session,
    actor: &str,
    name: &str,
) -> Result<RoomCredential> {
    eligible(session)?;
    let sid = session.id.as_str();
    let owner = session.created_by.as_str();
    let handle = ctx
        .live_handle(sid)
        .filter(|h| !h.has_exited())
        .ok_or_else(|| Error::Conflict("Start the local session before opening a room".into()))?;
    let credentials = ctx
        .rooms()
        .create(sid, owner, name, &session.title, &session.provider)
        .await?;
    if let Err(error) = ctx
        .begin_authority(sid, &credentials.room_id, owner, &credentials.member_id)
        .await
    {
        ctx.rooms().forget(&credentials.room_id).await;
        return Err(error);
    }
    let room = ctx.rooms().get(&credentials.room_id).await?;
    {
        let mut room = room.lock().await;
        room.spawn_seq = handle.spawn_seq();
        authority::sync_authority(ctx, &mut room).await?;
    }
    authority::audit(
        ctx,
        &credentials.room_id,
        Some(actor.into()),
        Some(credentials.member_id.clone()),
        "create",
    )
    .await;
    authority::maintain(ctx.clone(), room);
    Ok(credentials)
}
/// The owner's live rooms, each as the host sees it.
pub async fn list<C: RoomsCtx>(ctx: &C, owner: &str) -> Result<Vec<RoomSnapshot>> {
    let mut result = vec![];
    for shared in ctx.rooms().all().await {
        let mut room = shared.lock().await;
        if room.owner == owner && room.live().is_ok() {
            authority::sync_authority(ctx, &mut room).await?;
            result.push(room.snapshot(&room.host)?);
        }
    }
    Ok(result)
}
pub async fn end<C: RoomsCtx>(ctx: &C, id: &str, owner: &str) -> Result<()> {
    let shared = ctx.rooms().get(id).await?;
    let mut room = shared.lock().await;
    if room.owner != owner {
        return Err(Error::Forbidden(
            "Only the room owner can end sharing".into(),
        ));
    }
    authority::end_room(ctx, &mut room, "The host ended the room").await;
    let owner = room.owner.clone();
    drop(room);
    authority::audit(ctx, id, Some(owner), None, "end").await;
    Ok(())
}
pub async fn invite<C: RoomsCtx>(
    ctx: &C,
    id: &str,
    owner: &str,
    actor: &str,
    role: RoomRole,
) -> Result<RoomInvite> {
    let origin = ice::public_origin(ctx).await?;
    let invite = ctx.rooms().invite(id, owner, role).await?;
    let url = format!("{origin}/#/room/{id}/{invite}");
    authority::audit(ctx, id, Some(actor.into()), None, "invite").await;
    Ok(RoomInvite {
        invite,
        url,
        expires_at: (chrono::Utc::now() + chrono::Duration::minutes(10)).to_rfc3339(),
    })
}
