//! The room's view of the session manager's terminal authority, the room
//! audit trail and the per-room maintenance worker. The socket run loop in
//! otto-server calls these under the room lock.
use crate::{
    actions::Effect,
    registry::{Room, SharedRoom, GRACE},
    RoomAudit, RoomsCtx,
};
use otto_core::{api::*, Error, Result};
use std::time::{Duration, Instant};

/// Audit membership/permission decisions, never chat, SDP, media or credentials.
pub async fn audit<C: RoomsCtx>(
    ctx: &C,
    room: &str,
    user: Option<String>,
    member: Option<String>,
    action: &str,
) {
    ctx.audit(RoomAudit {
        user_id: user,
        action: format!("room.{action}"),
        target: room.into(),
        detail: member.map(|id| serde_json::json!({"member_id":id})),
    })
    .await;
}
/// The audited actions and the member they target (`None` = the actor).
pub fn audit_action(action: &RoomAction) -> Option<(&'static str, Option<String>)> {
    use RoomAction::*;
    Some(match action {
        Admit { member_id, .. } => ("admit", Some(member_id.clone())),
        Reject { member_id } => ("reject", Some(member_id.clone())),
        Remove { member_id } => ("remove", Some(member_id.clone())),
        Role { member_id, .. } => ("role", Some(member_id.clone())),
        GrantControl { member_id } => ("grant_control", Some(member_id.clone())),
        ReleaseControl => ("release_control", None),
        Mute { member_id, .. } => ("mute", Some(member_id.clone())),
        GrantPresent { member_id, .. } => ("grant_present", Some(member_id.clone())),
        GrantAnnotation { member_id, .. } => ("grant_annotation", Some(member_id.clone())),
        RevokeAnnotation { member_id, .. } => ("revoke_annotation", Some(member_id.clone())),
        StartPresent { .. } => ("start_present", None),
        StopPresent { .. } => ("stop_present", None),
        Leave => ("leave", None),
        End => ("end", None),
        _ => return None,
    })
}
/// Copy the manager's driver and grant epoch into the room. Fails when the
/// session's authority no longer belongs to this room.
pub async fn sync_authority<C: RoomsCtx>(ctx: &C, room: &mut Room) -> Result<()> {
    let authority = ctx
        .authority(&room.session)
        .await
        .ok_or(Error::Unauthorized)?;
    if authority.room_id != room.id {
        return Err(Error::Unauthorized);
    }
    room.driver = authority.driver.unwrap_or_else(|| room.host.clone());
    room.grant_epoch = authority.epoch;
    Ok(())
}
pub async fn end_room<C: RoomsCtx>(ctx: &C, room: &mut Room, reason: &str) {
    ctx.end_authority(&room.session, &room.id).await;
    room.end(reason);
    audit(ctx, &room.id, None, None, "closed").await;
}
/// Carry out an action's PTY-authority effect, then resync the room.
pub async fn effect<C: RoomsCtx>(ctx: &C, room: &mut Room, effect: Effect) -> Result<()> {
    match effect {
        Effect::None => {}
        Effect::Driver(member) => {
            room.grant_epoch = ctx
                .set_driver(&room.session, &room.id, member.as_deref())
                .await?;
        }
        Effect::End => {
            end_room(ctx, room, "The host ended the room").await;
            return Ok(());
        }
    }
    sync_authority(ctx, room).await?;
    Ok(())
}
/// A member's socket closed: withdraw its media, reclaim the driver when it
/// held control (or was the host), and publish the new state.
pub async fn disconnect<C: RoomsCtx>(ctx: &C, shared: &SharedRoom, member: &str, generation: u64) {
    let mut room = shared.lock().await;
    if room.ended {
        return;
    }
    if room.disconnect(member, generation)
        && ctx.set_driver(&room.session, &room.id, None).await.is_err()
    {
        end_room(ctx, &mut room, "Terminal control ended").await;
        return;
    }
    let _ = sync_authority(ctx, &mut room).await;
    room.changed();
}
/// Connect a member's socket: bump its generation and fence the old one.
/// Reconnect cannot restore a previous grant. Advance even when the same
/// member is regranted, fencing old terminal sockets and queued frames.
/// `None` ends (or refuses) the connection.
pub async fn connect<C: RoomsCtx>(ctx: &C, room: &mut Room, member: &str) -> Option<u64> {
    let generation = room.connect(member).ok()?;
    if ctx
        .fence_connection(&room.session, &room.id, member)
        .await
        .is_err()
        || sync_authority(ctx, room).await.is_err()
    {
        end_room(ctx, room, "Terminal control ended").await;
        return None;
    }
    Some(generation)
}
/// One bounded maintenance task per room, including when no sockets remain.
/// It never resumes/kills a PTY or broadcasts through ordinary session streams.
pub fn maintain<C: RoomsCtx>(ctx: C, shared: SharedRoom) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        loop {
            tick.tick().await;
            let mut room = shared.lock().await;
            if room.ended {
                let id = room.id.clone();
                drop(room);
                ctx.rooms().forget(&id).await;
                break;
            }
            let live = ctx
                .live_handle(&room.session)
                .is_some_and(|h| !h.has_exited() && h.spawn_seq() == room.spawn_seq);
            let host_expired = room
                .members
                .get(&room.host)
                .is_none_or(|m| m.disconnected.is_some_and(|t| t.elapsed() >= GRACE));
            if !live || room.live().is_err() || host_expired {
                end_room(
                    &ctx,
                    &mut room,
                    if host_expired {
                        "Host disconnected"
                    } else {
                        "Session sharing ended"
                    },
                )
                .await;
                continue;
            }
            let (mut changed, reclaim) = room.sweep(Instant::now());
            if reclaim && ctx.set_driver(&room.session, &room.id, None).await.is_err() {
                end_room(&ctx, &mut room, "Terminal control ended").await;
                continue;
            }
            changed |= room.settle_audio();
            let previous = room.grant_epoch;
            if sync_authority(&ctx, &mut room).await.is_err() {
                end_room(&ctx, &mut room, "Terminal control ended").await;
                continue;
            }
            if changed || previous != room.grant_epoch {
                room.changed();
            }
        }
    });
}
impl Room {
    /// One maintenance pass over the members: heartbeat timeouts disconnect,
    /// expired or long-disconnected guests leave, presenter windows close and
    /// expired marks drop. Returns `(changed, reclaim the driver)`.
    pub fn sweep(&mut self, now: Instant) -> (bool, bool) {
        let mut changed = false;
        let mut reclaim = false;
        let disconnected: Vec<_> = self
            .members
            .values()
            .filter(|m| m.view.connected && m.heartbeat.elapsed() >= Duration::from_secs(10))
            .map(|m| (m.view.id.clone(), m.view.generation))
            .collect();
        for (id, generation) in disconnected {
            reclaim |= self.disconnect(&id, generation);
            changed = true;
        }
        let expired: Vec<_> = self
            .members
            .values()
            .filter(|m| {
                !m.valid()
                    || (m.view.id != self.host
                        && m.disconnected.is_some_and(|t| t.elapsed() >= GRACE)
                        && m.view.admission == RoomAdmission::Admitted)
            })
            .map(|m| m.view.id.clone())
            .collect();
        for id in expired {
            self.withdraw_media(&id);
            self.members.remove(&id);
            if self.driver == id {
                self.driver = self.host.clone();
                reclaim = true;
            }
            changed = true;
        }
        for member in self.members.values_mut() {
            if member.present_until.is_some_and(|t| t < now) {
                member.view.presenter_allowed = false;
                member.present_until = None;
                changed = true;
            }
        }
        self.marks.retain(|m| {
            chrono::DateTime::parse_from_rfc3339(&m.expires_at)
                .is_ok_and(|t| t > chrono::Utc::now())
        });
        (changed, reclaim)
    }
    /// The host must confirm an audio change within 3 s, else room audio
    /// stops for everyone. Returns whether the state changed.
    pub fn settle_audio(&mut self) -> bool {
        if !self
            .audio_pending_since
            .is_some_and(|t| t.elapsed() >= Duration::from_secs(3))
        {
            return false;
        }
        let active = self.members.values().any(|m| m.view.audio_joined);
        self.audio_pending_since = None;
        if !active {
            self.audio_enforced_epoch = self.audio_epoch;
            return false;
        }
        for member in self.members.values_mut() {
            member.view.audio_joined = false;
            member.view.muted = true;
        }
        self.audio_epoch += 1;
        self.event(
            None,
            RoomEvent::Error {
                code: "audio_unavailable".into(),
                message: "Room audio stopped because the host did not confirm the audio change"
                    .into(),
            },
        );
        true
    }
}
