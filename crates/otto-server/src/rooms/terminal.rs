//! Room-only terminal transport. It never calls ensure_live and never follows
//! a replacement process. All input/resize crosses SessionManager authority.
use super::{
    auth::socket_token,
    registry::{Rate, SharedRoom},
    socket::{send, send_frame},
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
use base64::{engine::general_purpose::STANDARD, Engine};
use otto_core::{api::RoomEvent, Error, Result};
use serde::Deserialize;
use std::time::Duration;
use tokio::sync::broadcast;

const FLOW_AUTO_RESUME: Duration = Duration::from_secs(2);

/// Output flow belongs to this viewer, independently of the room driver.
#[derive(Default)]
struct FlowGate {
    paused_until: Option<tokio::time::Instant>,
}
impl FlowGate {
    fn pause(&mut self) {
        self.paused_until = Some(tokio::time::Instant::now() + FLOW_AUTO_RESUME);
    }
    fn resume(&mut self, output: &mut broadcast::Receiver<axum::body::Bytes>) -> bool {
        // Inspect a fixed backlog count, never chase a continuously producing
        // PTY with a drain loop. The snapshot replaces this receiver atomically.
        self.paused_until.take().is_some() && !output.is_empty()
    }
}
async fn flow_deadline(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Frame {
    Input {
        data: String,
        grant_epoch: u64,
    },
    Resize {
        cols: u16,
        rows: u16,
        grant_epoch: u64,
    },
    Scrollback {
        #[serde(default)]
        lines: Option<usize>,
    },
    Snapshot,
    Pause,
    Resume,
}
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
    let shared = ctx.rooms.get(&id).await?;
    let (generation, session, spawn) = {
        let room = shared.lock().await;
        let member = room.admitted(&member)?;
        if member.terminal_active {
            return Err(
                Error::Conflict("This member already has a terminal connection".into()).into(),
            );
        }
        if !member.view.connected {
            return Err(
                Error::Forbidden("Connect the room before viewing the terminal".into()).into(),
            );
        }
        (member.view.generation, room.session.clone(), room.spawn_seq)
    };
    let handle = ctx
        .manager
        .live_handle(&session)
        .filter(|h| !h.has_exited() && h.spawn_seq() == spawn)
        .ok_or(Error::Unauthorized)?;
    Ok(ws
        .protocols(["otto-room"])
        .max_message_size(65536)
        .max_frame_size(65536)
        .on_upgrade(move |socket| run(ctx, shared, member, generation, handle, socket)))
}
async fn snapshot(
    socket: &mut WebSocket,
    handle: &otto_pty::PtyHandle,
    lines: usize,
    output: &mut tokio::sync::broadcast::Receiver<axum::body::Bytes>,
) -> Result<()> {
    let replay = handle.snapshot_and_subscribe(lines.min(10_000));
    *output = replay.output;
    let value = serde_json::json!({"type":"scrollback","data":STANDARD.encode(replay.data),"cols":replay.cols,"rows":replay.rows,"epoch":handle.spawn_seq()});
    send_frame(socket, Message::Text(value.to_string().into())).await
}
async fn valid(shared: &SharedRoom, member: &str, generation: u64) -> bool {
    let room = shared.lock().await;
    room.connected(member, generation).is_ok() && room.admitted(member).is_ok()
}
async fn run(
    ctx: ServerCtx,
    shared: SharedRoom,
    member: String,
    generation: u64,
    handle: std::sync::Arc<otto_pty::PtyHandle>,
    socket: WebSocket,
) {
    {
        let mut room = shared.lock().await;
        if room.connected(&member, generation).is_err() || room.admitted(&member).is_err() {
            return;
        }
        let m = room.members.get_mut(&member).unwrap();
        if m.terminal_active {
            return;
        }
        m.terminal_active = true;
    }
    serve(
        ctx,
        shared.clone(),
        member.clone(),
        generation,
        handle,
        socket,
    )
    .await;
    let mut room = shared.lock().await;
    if let Some(m) = room.members.get_mut(&member) {
        if m.view.generation == generation {
            m.terminal_active = false;
        }
    }
}
async fn serve(
    ctx: ServerCtx,
    shared: SharedRoom,
    member: String,
    generation: u64,
    handle: std::sync::Arc<otto_pty::PtyHandle>,
    mut socket: WebSocket,
) {
    if !valid(&shared, &member, generation).await {
        return;
    }
    // Keep the existing session alive while a room viewer watches it. This
    // guard does not start a process and does not claim ordinary input.
    let session = shared.lock().await.session.clone();
    let _attach = ctx.manager.attach(&session);
    let mut output = handle.subscribe();
    let mut notices = shared.lock().await.signal.subscribe();
    let mut exit = handle.on_exit();
    if send_frame(
        &mut socket,
        Message::Text(
            serde_json::json!({"type":"status","status":"running","epoch":handle.spawn_seq()})
                .to_string()
                .into(),
        ),
    )
    .await
    .is_err()
        || !valid(&shared, &member, generation).await
        || handle.has_exited()
        || snapshot(&mut socket, &handle, 10_000, &mut output)
            .await
            .is_err()
    {
        return;
    }
    let mut frame_rate = Rate::new(120.0);
    let mut history_rate = Rate::new(2.0);
    let mut flow = FlowGate::default();
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
    loop {
        tokio::select! {
            _=flow_deadline(flow.paused_until), if flow.paused_until.is_some()=>{
                if !valid(&shared,&member,generation).await{break;}
                if flow.resume(&mut output) && snapshot(&mut socket,&handle,10_000,&mut output).await.is_err(){break;}
            }
            chunk=output.recv(), if flow.paused_until.is_none()=>{
                if !valid(&shared,&member,generation).await{break;}
                match chunk {
                    Ok(chunk)=>if send_frame(&mut socket,Message::Binary(chunk)).await.is_err(){break;},
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_))=>{if snapshot(&mut socket,&handle,10_000,&mut output).await.is_err(){break;}},
                    Err(_)=>break,
                }
            }
            incoming=socket.recv()=>{
                if frame_rate.take(60.0,120.0).is_err(){break;}
                if !valid(&shared,&member,generation).await{break;}
                let action=match incoming {
                    Some(Ok(Message::Text(text)))=>match serde_json::from_str::<Frame>(&text){Ok(frame)=>frame,Err(_)=>{let _=send(&mut socket,&RoomEvent::Error{code:"invalid".into(),message:"Invalid room terminal frame".into()}).await;continue;}},
                    Some(Ok(Message::Ping(data)))=>{if send_frame(&mut socket,Message::Pong(data)).await.is_err(){break;}continue;},
                    Some(Ok(Message::Pong(_)))=>continue,
                    _=>break,
                };
                let room_id={let room=shared.lock().await;if room.connected(&member,generation).is_err(){break;}room.id.clone()};
                // No room lock over a potentially backpressured PTY write.
                // Grant changes invalidate the manager's actual writer queue.
                let result=match action {
                    Frame::Input{data,grant_epoch}=>match decode_input(&data){Ok(data)=>ctx.manager.room_input(&session,&room_id,&member,grant_epoch,&data).await,Err(error)=>Err(error)},
                    Frame::Resize{cols,rows,grant_epoch}=>ctx.manager.room_resize(&session,&room_id,&member,grant_epoch,cols,rows).await,
                    Frame::Scrollback{lines}=>match history_rate.take(1.0,2.0){Ok(())=>snapshot(&mut socket,&handle,lines.unwrap_or(10_000),&mut output).await,Err(e)=>Err(e)},
                    Frame::Snapshot=>match history_rate.take(1.0,2.0){Ok(())=>snapshot(&mut socket,&handle,0,&mut output).await,Err(e)=>Err(e)},
                    Frame::Pause=>{flow.pause();Ok(())},
                    Frame::Resume=>{if flow.resume(&mut output){snapshot(&mut socket,&handle,10_000,&mut output).await}else{Ok(())}},
                };
                if let Err(error)=result {if send(&mut socket,&RoomEvent::Error{code:error.code().into(),message:error.to_string()}).await.is_err(){break;}}
            }
            _=notices.recv()=>{if !valid(&shared,&member,generation).await{break;}},
            _=exit.changed()=>break,
            _=tick.tick()=>{if !valid(&shared,&member,generation).await || handle.has_exited() || ctx.manager.live_handle(&session).is_none_or(|h|h.spawn_seq()!=handle.spawn_seq()){break;}},
        }
    }
    let _ = send(
        &mut socket,
        &RoomEvent::Ended {
            reason: "Terminal sharing ended".into(),
        },
    )
    .await;
}

fn decode_input(data: &str) -> Result<Vec<u8>> {
    let bytes = STANDARD
        .decode(data)
        .map_err(|_| Error::Invalid("Input must use base64 encoding".into()))?;
    if bytes.len() > 32768 {
        return Err(Error::Invalid("Input exceeds 32 KiB".into()));
    }
    Ok(bytes)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn input_decodes_the_existing_terminal_wire_format() {
        assert_eq!(decode_input("aGVsbG8NCg==").unwrap(), b"hello\r\n");
        assert!(decode_input("%invalid").is_err());
    }

    #[test]
    fn terminal_flow_frames_need_no_driver_epoch() {
        assert!(matches!(
            serde_json::from_str::<Frame>(r#"{"type":"pause"}"#),
            Ok(Frame::Pause)
        ));
        assert!(matches!(
            serde_json::from_str::<Frame>(r#"{"type":"resume"}"#),
            Ok(Frame::Resume)
        ));
    }

    #[test]
    fn resume_requests_one_snapshot_only_when_output_was_skipped() {
        let (tx, mut output) = broadcast::channel(2);
        let mut flow = FlowGate::default();
        flow.pause();
        assert!(
            !flow.resume(&mut output),
            "quiet pause must not rebuild the screen"
        );
        flow.pause();
        for _ in 0..8 {
            tx.send(axum::body::Bytes::from_static(b"output")).unwrap();
        }
        assert!(
            flow.resume(&mut output),
            "lagged receiver must resynchronize"
        );
        assert!(
            !flow.resume(&mut output),
            "duplicate resume must not send another snapshot"
        );
        // The caller replaces the old receiver with the snapshot subscription.
        // Flow detection itself never drains a live producer.
        assert!(!output.is_empty());
        output = tx.subscribe();
        tx.send(axum::body::Bytes::from_static(b"live")).unwrap();
        assert_eq!(output.try_recv().unwrap(), b"live"[..]);
    }

    #[tokio::test]
    async fn repeated_pause_rearms_auto_resume() {
        let (tx, mut output) = broadcast::channel(2);
        let mut flow = FlowGate::default();
        flow.pause();
        let first = flow.paused_until.unwrap();
        tokio::time::sleep(Duration::from_millis(1)).await;
        flow.pause();
        assert!(flow.paused_until.unwrap() > first);
        assert!(flow.paused_until.unwrap() > tokio::time::Instant::now() + Duration::from_secs(1));
        // Expire the deadline without spending two wall-clock seconds in a unit test.
        flow.paused_until = Some(tokio::time::Instant::now());
        tx.send(axum::body::Bytes::from_static(b"held")).unwrap();
        tokio::time::timeout(Duration::from_millis(100), flow_deadline(flow.paused_until))
            .await
            .unwrap();
        assert!(flow.resume(&mut output));
        assert!(flow.paused_until.is_none());
    }
}
