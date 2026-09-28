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
use otto_sessions::ws::{CreditGate, CreditStep};
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;

const FLOW_AUTO_RESUME: Duration = Duration::from_secs(2);
const RESYNC_INTERVAL: Duration = Duration::from_millis(500);
/// Credit `ack` budget. A viewer acks every 64 KB it parses — ~100/s while
/// it drains a flood at WebKit's 5–8 MB/s — which would trip the 60/s
/// general frame limit and drop the socket. An ack only moves this viewer's
/// own window, so it gets its own budget and skips the room lock.
const ACK_RATE: f64 = 400.0;
const ACK_BURST: f64 = 800.0;

/// Recovery cannot be rejected after the client discards its renderer backlog.
/// Keep one pending request, coalescing bursts without delaying its deadline.
#[derive(Default)]
struct ResyncQueue {
    pending: Option<(usize, tokio::time::Instant)>,
    next_allowed: Option<tokio::time::Instant>,
}
impl ResyncQueue {
    fn request(&mut self, lines: usize, now: tokio::time::Instant) {
        let due = self.next_allowed.unwrap_or(now).max(now);
        self.pending = Some((lines.min(10_000), self.pending.map_or(due, |(_, at)| at)));
    }
    fn deadline(&self) -> Option<tokio::time::Instant> {
        self.pending.map(|(_, at)| at)
    }
    fn take_ready(&mut self, now: tokio::time::Instant) -> Option<usize> {
        let (lines, at) = self.pending?;
        if now < at {
            return None;
        }
        self.pending = None;
        self.next_allowed = Some(now + RESYNC_INTERVAL);
        Some(lines)
    }
}

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
    Resync {
        #[serde(default)]
        lines: Option<usize>,
    },
    Pause,
    Resume,
    /// Credit flow control, same contract as `/ws/term` (docs/contracts/
    /// ws.md): at most `window` unacknowledged binary bytes to this viewer.
    /// Like pause/resume it needs no driver grant.
    Credit {
        #[serde(default)]
        window: u64,
    },
    /// Cumulative credited bytes this viewer parsed or dropped.
    Ack {
        bytes: u64,
    },
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
/// Replace this viewer's stream with one snapshot. The emulator is locked only
/// to copy its state and open the new subscription (atomically); formatting
/// and base64 run on the blocking pool (r3-06-02). The snapshot supersedes
/// any output the credit window was holding.
async fn snapshot(
    socket: &mut WebSocket,
    handle: &Arc<otto_pty::PtyHandle>,
    lines: usize,
    output: &mut tokio::sync::broadcast::Receiver<axum::body::Bytes>,
    credit: &mut Option<CreditGate>,
) -> Result<()> {
    let h = Arc::clone(handle);
    let lines = lines.min(10_000);
    let (text, fresh) = tokio::task::spawn_blocking(move || {
        let replay = h.snapshot_and_subscribe(lines);
        let value = serde_json::json!({"type":"scrollback","data":STANDARD.encode(&replay.data),"cols":replay.cols,"rows":replay.rows,"epoch":h.spawn_seq()});
        (value.to_string(), replay.output)
    })
    .await
    .map_err(|e| Error::Internal(format!("terminal snapshot: {e}")))?;
    *output = fresh;
    if let Some(c) = credit.as_mut() {
        c.superseded();
    }
    send_frame(socket, Message::Text(text.into())).await
}

/// Carry out a credit step on the room terminal socket.
async fn apply_credit(
    step: CreditStep,
    socket: &mut WebSocket,
    handle: &Arc<otto_pty::PtyHandle>,
    output: &mut tokio::sync::broadcast::Receiver<axum::body::Bytes>,
    credit: &mut Option<CreditGate>,
) -> Result<()> {
    match step {
        CreditStep::Idle => Ok(()),
        CreditStep::Send(bytes) => send_frame(socket, Message::Binary(bytes)).await,
        CreditStep::Resync => snapshot(socket, handle, 10_000, output, credit).await,
    }
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
    handle: Arc<otto_pty::PtyHandle>,
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
    // Credit flow control, once this viewer offers it (else pause/resume).
    let mut credit: Option<CreditGate> = None;
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
        || snapshot(&mut socket, &handle, 10_000, &mut output, &mut credit)
            .await
            .is_err()
    {
        return;
    }
    let mut frame_rate = Rate::new(120.0);
    let mut ack_rate = Rate::new(ACK_BURST);
    let mut history_rate = Rate::new(2.0);
    let mut flow = FlowGate::default();
    let mut recovery = ResyncQueue::default();
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
    loop {
        tokio::select! {
            _=flow_deadline(recovery.deadline()), if recovery.pending.is_some()=>{
                if !valid(&shared,&member,generation).await{break;}
                if let Some(lines)=recovery.take_ready(tokio::time::Instant::now()) {
                    if snapshot(&mut socket,&handle,lines,&mut output,&mut credit).await.is_err(){break;}
                }
            }
            _=flow_deadline(flow.paused_until), if flow.paused_until.is_some() && recovery.pending.is_none()=>{
                if !valid(&shared,&member,generation).await{break;}
                if flow.resume(&mut output) && snapshot(&mut socket,&handle,10_000,&mut output,&mut credit).await.is_err(){break;}
            }
            // Credit: no ack progress for 2 s while output waits (see
            // CreditGate::forgive — the window stays bounded, one snapshot is
            // owed once the viewer drains).
            _=flow_deadline(credit.as_ref().and_then(CreditGate::stall_deadline)), if credit.as_ref().is_some_and(|c|c.stall_deadline().is_some())=>{
                if let Some(c)=credit.as_mut(){c.forgive();}
            }
            chunk=output.recv(), if flow.paused_until.is_none() && recovery.pending.is_none()=>{
                if !valid(&shared,&member,generation).await{break;}
                match chunk {
                    Ok(chunk)=>{
                        let step=match credit.as_mut(){Some(c)=>c.push(chunk.to_vec(),tokio::time::Instant::now()),None=>CreditStep::Send(chunk)};
                        if apply_credit(step,&mut socket,&handle,&mut output,&mut credit).await.is_err(){break;}
                    },
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_))=>{if snapshot(&mut socket,&handle,10_000,&mut output,&mut credit).await.is_err(){break;}},
                    Err(_)=>break,
                }
            }
            incoming=socket.recv()=>{
                let parsed=match &incoming {Some(Ok(Message::Text(text)))=>Some(serde_json::from_str::<Frame>(text)),_=>None};
                // Acks: own budget, no room lock (see ACK_RATE).
                if let Some(Ok(Frame::Ack{bytes}))=parsed {
                    if ack_rate.take(ACK_RATE,ACK_BURST).is_err(){break;}
                    if let Some(c)=credit.as_mut(){
                        let step=c.ack(bytes,tokio::time::Instant::now());
                        if apply_credit(step,&mut socket,&handle,&mut output,&mut credit).await.is_err(){break;}
                    }
                    continue;
                }
                if frame_rate.take(60.0,120.0).is_err(){break;}
                if !valid(&shared,&member,generation).await{break;}
                let action=match (parsed,incoming) {
                    (Some(Ok(frame)),_)=>frame,
                    (Some(Err(_)),_)=>{let _=send(&mut socket,&RoomEvent::Error{code:"invalid".into(),message:"Invalid room terminal frame".into()}).await;continue;},
                    (None,Some(Ok(Message::Ping(data))))=>{if send_frame(&mut socket,Message::Pong(data)).await.is_err(){break;}continue;},
                    (None,Some(Ok(Message::Pong(_))))=>continue,
                    _=>break,
                };
                let room_id={let room=shared.lock().await;if room.connected(&member,generation).is_err(){break;}room.id.clone()};
                // No room lock over a potentially backpressured PTY write.
                // Grant changes invalidate the manager's actual writer queue.
                let result=match action {
                    Frame::Input{data,grant_epoch}=>match decode_input(&data){Ok(data)=>{
                        // Typed while more than a window behind: the held backlog becomes one snapshot.
                        if let Some(c)=credit.as_mut(){c.skip_on_input(true,tokio::time::Instant::now());}
                        ctx.manager.room_input(&session,&room_id,&member,grant_epoch,&data).await
                    },Err(error)=>Err(error)},
                    Frame::Resize{cols,rows,grant_epoch}=>ctx.manager.room_resize(&session,&room_id,&member,grant_epoch,cols,rows).await,
                    Frame::Scrollback{lines}=>match history_rate.take(1.0,2.0){Ok(())=>snapshot(&mut socket,&handle,lines.unwrap_or(10_000),&mut output,&mut credit).await,Err(e)=>Err(e)},
                    Frame::Snapshot=>match history_rate.take(1.0,2.0){Ok(())=>snapshot(&mut socket,&handle,0,&mut output,&mut credit).await,Err(e)=>Err(e)},
                    Frame::Resync{lines}=>{
                        flow.paused_until=None;
                        let now=tokio::time::Instant::now();
                        recovery.request(lines.unwrap_or(10_000),now);
                        if let Some(lines)=recovery.take_ready(now){snapshot(&mut socket,&handle,lines,&mut output,&mut credit).await}else{Ok(())}
                    },
                    Frame::Pause=>{flow.pause();Ok(())},
                    Frame::Resume=>{if flow.resume(&mut output) && recovery.pending.is_none(){snapshot(&mut socket,&handle,10_000,&mut output,&mut credit).await}else{Ok(())}},
                    Frame::Credit{window}=>{
                        let gate=CreditGate::new(window);
                        let sent=send_frame(&mut socket,Message::Text(gate.grant_frame().into())).await;
                        credit=Some(gate);
                        sent
                    },
                    // Routed above (before the general limiter).
                    Frame::Ack{..}=>Ok(()),
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
        assert!(matches!(
            serde_json::from_str::<Frame>(r#"{"type":"resync","lines":1000}"#),
            Ok(Frame::Resync { lines: Some(1000) })
        ));
    }

    /// r3-10-06: room viewers use the same credit window as `/ws/term`.
    #[test]
    fn terminal_credit_frames_parse_without_a_driver_epoch() {
        assert!(matches!(
            serde_json::from_str::<Frame>(r#"{"type":"credit","window":1048576}"#),
            Ok(Frame::Credit { window: 1_048_576 })
        ));
        assert!(matches!(
            serde_json::from_str::<Frame>(r#"{"type":"credit"}"#),
            Ok(Frame::Credit { window: 0 })
        ));
        assert!(matches!(
            serde_json::from_str::<Frame>(r#"{"type":"ack","bytes":65536}"#),
            Ok(Frame::Ack { bytes: 65_536 })
        ));
        assert!(
            serde_json::from_str::<Frame>(r#"{"type":"ack","bytes":1,"grant_epoch":3}"#).is_err(),
            "still deny_unknown_fields"
        );
    }

    #[test]
    fn resync_bursts_coalesce_without_starving_recovery() {
        let now = tokio::time::Instant::now();
        let mut recovery = ResyncQueue::default();
        recovery.request(100, now);
        assert_eq!(
            recovery.take_ready(now),
            Some(100),
            "first recovery is immediate"
        );
        recovery.request(200, now + Duration::from_millis(10));
        for ms in 11..500 {
            recovery.request(usize::MAX, now + Duration::from_millis(ms));
            assert_eq!(recovery.take_ready(now + Duration::from_millis(ms)), None);
            assert_eq!(recovery.deadline(), Some(now + RESYNC_INTERVAL));
        }
        assert_eq!(recovery.take_ready(now + RESYNC_INTERVAL), Some(10_000));
        assert_eq!(
            recovery.take_ready(now + RESYNC_INTERVAL),
            None,
            "one response per coalesced burst"
        );
        recovery.request(0, now + RESYNC_INTERVAL);
        assert_eq!(recovery.deadline(), Some(now + 2 * RESYNC_INTERVAL));
        assert_eq!(recovery.take_ready(now + 2 * RESYNC_INTERVAL), Some(0));
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
