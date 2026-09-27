use super::{
    actions::Effect,
    auth::socket_token,
    registry::{Notice, Rate, Room, SharedRoom, GRACE},
};
use crate::{ApiResult, ServerCtx};
use axum::{
    extract::{
        ws::{Message, WebSocket},
        Path, State, WebSocketUpgrade,
    },
    http::HeaderMap,
    response::Response,
};
use otto_core::{api::*, Error, Result};
use std::time::{Duration, Instant};

pub async fn upgrade(
    State(ctx): State<ServerCtx>,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: axum::http::Uri,
    ws: WebSocketUpgrade,
) -> ApiResult<Response> {
    if uri.query().is_some() {
        return Err(Error::Unauthorized.into());
    }
    let token = socket_token(&headers)?;
    let member = ctx.rooms.authenticate(&id, &token).await?;
    let room = ctx.rooms.get(&id).await?;
    Ok(ws
        .protocols(["otto-room"])
        .max_message_size(65536)
        .max_frame_size(65536)
        .on_upgrade(move |socket| run(ctx, room, member, socket)))
}
pub(super) async fn send(socket: &mut WebSocket, event: &RoomEvent) -> Result<()> {
    let text = serde_json::to_string(event).map_err(|e| Error::Internal(e.to_string()))?;
    send_frame(socket, Message::Text(text.into())).await
}
pub(super) async fn send_frame(socket: &mut WebSocket, frame: Message) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(2), socket.send(frame))
        .await
        .map_err(|_| Error::Conflict("Room connection is too slow".into()))?
        .map_err(|_| Error::Conflict("Room connection closed".into()))
}
pub(super) async fn sync_authority(ctx: &ServerCtx, room: &mut Room) -> Result<()> {
    let authority = ctx
        .manager
        .room_authority_snapshot(&room.session)
        .await
        .ok_or(Error::Unauthorized)?;
    if authority.room_id != room.id {
        return Err(Error::Unauthorized);
    }
    room.driver = authority.driver.unwrap_or_else(|| room.host.clone());
    room.grant_epoch = authority.epoch;
    Ok(())
}
pub(super) async fn end_room(ctx: &ServerCtx, room: &mut Room, reason: &str) {
    ctx.manager
        .end_room_authority(&room.session, &room.id)
        .await;
    room.end(reason);
    super::audit(ctx, &room.id, None, None, "closed").await;
}
async fn effect(ctx: &ServerCtx, room: &mut Room, effect: Effect) -> Result<()> {
    match effect {
        Effect::None => {}
        Effect::Driver(member) => {
            room.grant_epoch = ctx
                .manager
                .set_room_driver(&room.session, &room.id, member.as_deref())
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
async fn disconnect(ctx: &ServerCtx, shared: &SharedRoom, member: &str, generation: u64) {
    let mut room = shared.lock().await;
    if room.ended {
        return;
    }
    if room.disconnect(member, generation)
        && ctx
            .manager
            .set_room_driver(&room.session, &room.id, None)
            .await
            .is_err()
    {
        end_room(ctx, &mut room, "Terminal control ended").await;
        return;
    }
    let _ = sync_authority(ctx, &mut room).await;
    room.changed();
}
async fn run(ctx: ServerCtx, shared: SharedRoom, member: String, mut socket: WebSocket) {
    let (generation, mut notices, initial) = {
        let mut room = shared.lock().await;
        let Ok(generation) = room.connect(&member) else {
            return;
        };
        // Reconnect cannot restore a previous grant. Advance even when the same
        // member is regranted, fencing old terminal sockets and queued frames.
        if ctx
            .manager
            .fence_room_connection(&room.session, &room.id, &member)
            .await
            .is_err()
            || sync_authority(&ctx, &mut room).await.is_err()
        {
            end_room(&ctx, &mut room, "Terminal control ended").await;
            return;
        }
        let rx = room.signal.subscribe();
        room.changed();
        let Ok(initial) = room.snapshot(&member) else {
            return;
        };
        (generation, rx, initial)
    };
    if send(
        &mut socket,
        &RoomEvent::Snapshot {
            room: Box::new(initial),
        },
    )
    .await
    .is_err()
    {
        disconnect(&ctx, &shared, &member, generation).await;
        return;
    }
    let mut wire_rate = Rate::new(120.0);
    let mut tick = tokio::time::interval(Duration::from_secs(5));
    loop {
        tokio::select! {
            incoming=socket.recv()=>{
                if wire_rate.take(64.0,120.0).is_err(){break;}
                let action=match incoming {
                    Some(Ok(Message::Text(text)))=>match serde_json::from_str::<RoomAction>(&text){Ok(action) if !matches!(action,RoomAction::Annotation{..}) || text.len()<=8192=>action,Ok(_)=>{let _=send(&mut socket,&RoomEvent::Error{code:"invalid".into(),message:"Annotation exceeds 8 KiB".into()}).await;continue;},Err(_)=>{if send(&mut socket,&RoomEvent::Error{code:"invalid".into(),message:"Unknown or malformed room action".into()}).await.is_err(){break;}continue;}},
                    Some(Ok(Message::Pong(_)))=>RoomAction::Heartbeat,
                    Some(Ok(Message::Ping(data)))=>{if send_frame(&mut socket,Message::Pong(data)).await.is_err(){break;}continue;},
                    _=>break,
                };
                let audit=super::audit_action(&action);
                let is_ice=matches!(action,RoomAction::RequestIce);
                // Drawing has its own small events. Do not retransmit the full
                // room/chat snapshot at pointer frequency.
                let mut state_change=!matches!(action,RoomAction::Heartbeat|RoomAction::Signal{..}|RoomAction::Subscribe{..}|RoomAction::Annotation{..}|RoomAction::RequestIce);
                let result={let mut room=shared.lock().await;
                    if room.connected(&member,generation).is_err(){break;}
                    if let RoomAction::AudioApplied{epoch}=&action {state_change=*epoch!=room.audio_enforced_epoch;}
                    match room.apply(&member,action){Ok(change)=>{let result=effect(&ctx,&mut room,change).await;if result.is_ok()&&state_change{room.changed();}result},Err(e)=>Err(e)}
                };
                if let Err(error)=result {if send(&mut socket,&RoomEvent::Error{code:error.code().into(),message:error.to_string()}).await.is_err(){break;}}
                else {
                    if let Some((action,target))=audit {let (id,user)={let room=shared.lock().await;(room.id.clone(),(member==room.host).then(||room.owner.clone()))};super::audit(&ctx,&id,user,target.or_else(||Some(member.clone())),action).await;}
                    if is_ice {
                    let response=super::ice::configuration(&ctx,&member).await;
                    let room=shared.lock().await;if room.connected(&member,generation).is_err() || room.admitted(&member).is_err(){break;}drop(room);
                    let event=response.unwrap_or_else(|e|RoomEvent::Error{code:e.code().into(),message:e.to_string()});if send(&mut socket,&event).await.is_err(){break;}
                    }
                }
            }
            notice=notices.recv()=>{
                let event={let mut room=shared.lock().await;
                    if room.ended{Some(RoomEvent::Ended{reason:"Room ended".into()})}
                    else if room.connected(&member,generation).is_err(){Some(RoomEvent::Ended{reason:"Membership ended or reconnected elsewhere".into()})}
                    else {match notice {
                        Ok(Notice::State)=>{if sync_authority(&ctx,&mut room).await.is_err(){break;}room.snapshot(&member).ok().map(|room|RoomEvent::Snapshot{room:Box::new(room)})},
                        Ok(Notice::Event{to,event})=>{
                            if to.as_deref().is_some_and(|target|target!=member) || room.admitted(&member).is_err(){None}
                            else if let RoomEvent::Annotation{annotation}= &event {
                                // A revoke/clear can overtake a queued notice;
                                // only still-live marks may leave the server.
                                if room.marks.iter().any(|m|m.id==annotation.id){Some(event)}else{None}
                            }else{Some(event)}
                        },
                        Err(_)=>break,
                    }}
                };
                if let Some(event)=event {let ended=matches!(event,RoomEvent::Ended{..});if send(&mut socket,&event).await.is_err()||ended{break;}}
            }
            _=tick.tick()=>{
                {let room=shared.lock().await;if room.connected(&member,generation).is_err() || room.members[&member].heartbeat.elapsed()>=Duration::from_secs(10){break;}}
                if send(&mut socket,&RoomEvent::Heartbeat).await.is_err() || send_frame(&mut socket,Message::Ping(Vec::new().into())).await.is_err(){break;}
            }
        }
    }
    disconnect(&ctx, &shared, &member, generation).await;
}
/// One bounded maintenance task per room, including when no sockets remain.
/// It never resumes/kills a PTY or broadcasts through ordinary session streams.
pub(super) fn maintain(ctx: ServerCtx, shared: SharedRoom) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        loop {
            tick.tick().await;
            let mut room = shared.lock().await;
            if room.ended {
                let id = room.id.clone();
                drop(room);
                ctx.rooms.forget(&id).await;
                break;
            }
            let live = ctx
                .manager
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
            let mut changed = false;
            let mut reclaim = false;
            let disconnected: Vec<_> = room
                .members
                .values()
                .filter(|m| m.view.connected && m.heartbeat.elapsed() >= Duration::from_secs(10))
                .map(|m| (m.view.id.clone(), m.view.generation))
                .collect();
            for (id, generation) in disconnected {
                reclaim |= room.disconnect(&id, generation);
                changed = true;
            }
            let expired: Vec<_> = room
                .members
                .values()
                .filter(|m| {
                    !m.valid()
                        || (m.view.id != room.host
                            && m.disconnected.is_some_and(|t| t.elapsed() >= GRACE)
                            && m.view.admission == RoomAdmission::Admitted)
                })
                .map(|m| m.view.id.clone())
                .collect();
            for id in expired {
                room.withdraw_media(&id);
                room.members.remove(&id);
                if room.driver == id {
                    room.driver = room.host.clone();
                    reclaim = true;
                }
                changed = true;
            }
            for member in room.members.values_mut() {
                if member.present_until.is_some_and(|t| t < Instant::now()) {
                    member.view.presenter_allowed = false;
                    member.present_until = None;
                    changed = true;
                }
            }
            room.marks.retain(|m| {
                chrono::DateTime::parse_from_rfc3339(&m.expires_at)
                    .is_ok_and(|t| t > chrono::Utc::now())
            });
            if reclaim
                && ctx
                    .manager
                    .set_room_driver(&room.session, &room.id, None)
                    .await
                    .is_err()
            {
                end_room(&ctx, &mut room, "Terminal control ended").await;
                continue;
            }
            if room
                .audio_pending_since
                .is_some_and(|t| t.elapsed() >= Duration::from_secs(3))
            {
                let active = room.members.values().any(|m| m.view.audio_joined);
                room.audio_pending_since = None;
                if active {
                    for member in room.members.values_mut() {
                        member.view.audio_joined = false;
                        member.view.muted = true;
                    }
                    room.audio_epoch += 1;
                    changed = true;
                    room.event(None,RoomEvent::Error{code:"audio_unavailable".into(),message:"Room audio stopped because the host did not confirm the audio change".into()});
                } else {
                    room.audio_enforced_epoch = room.audio_epoch;
                }
            }
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
