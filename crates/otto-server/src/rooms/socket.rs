//! The room socket: upgrade on a room capability, then relay actions and
//! notices. Authority sync, effects and maintenance are `otto_rooms::authority`.
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
use otto_rooms::{
    auth::socket_token,
    authority::{audit, audit_action, disconnect, effect, sync_authority},
    capability::TokenBucket,
    Notice, SharedRoom, RATE_LIMITED,
};
use std::time::Duration;

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
async fn run(ctx: ServerCtx, shared: SharedRoom, member: String, mut socket: WebSocket) {
    let (generation, mut notices, initial) = {
        let mut room = shared.lock().await;
        // Fences the member's previous socket (see `authority::connect`).
        let Some(generation) = otto_rooms::authority::connect(&ctx, &mut room, &member).await
        else {
            return;
        };
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
    let mut wire_rate = TokenBucket::new(120.0);
    let mut tick = tokio::time::interval(Duration::from_secs(5));
    loop {
        tokio::select! {
            incoming=socket.recv()=>{
                if wire_rate.take(64.0,120.0,RATE_LIMITED).is_err(){break;}
                let action=match incoming {
                    Some(Ok(Message::Text(text)))=>match serde_json::from_str::<RoomAction>(&text){Ok(action) if !matches!(action,RoomAction::Annotation{..}) || text.len()<=8192=>action,Ok(_)=>{let _=send(&mut socket,&RoomEvent::Error{code:"invalid".into(),message:"Annotation exceeds 8 KiB".into()}).await;continue;},Err(_)=>{if send(&mut socket,&RoomEvent::Error{code:"invalid".into(),message:"Unknown or malformed room action".into()}).await.is_err(){break;}continue;}},
                    Some(Ok(Message::Pong(_)))=>RoomAction::Heartbeat,
                    Some(Ok(Message::Ping(data)))=>{if send_frame(&mut socket,Message::Pong(data)).await.is_err(){break;}continue;},
                    _=>break,
                };
                let audit_entry=audit_action(&action);
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
                    if let Some((action,target))=audit_entry {let (id,user)={let room=shared.lock().await;(room.id.clone(),(member==room.host).then(||room.owner.clone()))};audit(&ctx,&id,user,target.or_else(||Some(member.clone())),action).await;}
                    if is_ice {
                    let response=otto_rooms::ice::configuration(&ctx,&member).await;
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
