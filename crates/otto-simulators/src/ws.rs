//! WebSocket protocols and socket loops for:
//! - `WS /ws/simulators/{id}/stream` (live screencast binary frames + bidirectional input/control)
//! - `WS /ws/simulators/{id}/logs` (live device log stream: `simctl log stream` / `adb logcat`)
//!
//! Both screenshot capture and the 5s grant recheck run on dedicated background
//! tasks off the socket `select!` loop so slow captures never stall token
//! revocation or client control messages.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, watch, RwLock};

use crate::service::{BootedTarget, SimulatorsService};
use crate::types::{
    encode_frame, Appearance, InputAction, Orientation, ScreenshotData, ScreenshotFormat,
    SimDevice, SimFrameHeader, MAX_CLIENT_FRAME_BYTES,
};

/// Maximum unacknowledged screencast frames allowed in flight before capture
/// pauses to avoid hammering `simctl`/`adb` for a stalled viewer.
const MAX_IN_FLIGHT_FRAMES: u64 = 4;

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamControlMsg {
    Ack {
        seq: u64,
    },
    SetFps {
        fps: u8,
    },
    Pause,
    Resume,
    Snapshot,
    OpenUrl {
        url: String,
        #[serde(default)]
        auto_forward_localhost: Option<bool>,
    },
    Orientation {
        orientation: Orientation,
    },
    Appearance {
        appearance: Appearance,
    },
}

/// Wire-compatible client message reusing [`InputAction`] directly.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ClientStreamMsg {
    Control(StreamControlMsg),
    Input(InputAction),
}

impl ClientStreamMsg {
    pub fn parse(raw: &str) -> Result<Self, String> {
        if raw.len() > MAX_CLIENT_FRAME_BYTES {
            return Err("client frame exceeds size limit".into());
        }
        serde_json::from_str(raw).map_err(|e| format!("invalid client frame: {e}"))
    }

    pub fn is_drive(&self) -> bool {
        !matches!(
            self,
            ClientStreamMsg::Control(
                StreamControlMsg::Ack { .. }
                    | StreamControlMsg::SetFps { .. }
                    | StreamControlMsg::Pause
                    | StreamControlMsg::Resume
                    | StreamControlMsg::Snapshot
            )
        )
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerStreamMsg {
    Hello {
        device: SimDevice,
        fps: u8,
        can_drive: bool,
    },
    State {
        device: SimDevice,
    },
    Error {
        code: String,
        message: String,
    },
    Closed {
        reason: String,
    },
}

impl ServerStreamMsg {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| {
            r#"{"type":"error","code":"internal","message":"serialization failed"}"#.into()
        })
    }

    pub fn error(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Error {
            code: code.into(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Verbose,
    Debug,
    Info,
    Warn,
    Error,
    Fatal,
    Unknown,
}

impl LogLevel {
    pub fn rank(self) -> u8 {
        match self {
            LogLevel::Verbose => 0,
            LogLevel::Debug => 1,
            LogLevel::Info => 2,
            LogLevel::Warn => 3,
            LogLevel::Error => 4,
            LogLevel::Fatal => 5,
            LogLevel::Unknown => 2,
        }
    }

    pub fn parse_filter(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "v" | "verbose" => Some(LogLevel::Verbose),
            "d" | "debug" => Some(LogLevel::Debug),
            "i" | "info" => Some(LogLevel::Info),
            "w" | "warn" | "warning" => Some(LogLevel::Warn),
            "e" | "error" => Some(LogLevel::Error),
            "f" | "fatal" | "fault" => Some(LogLevel::Fatal),
            _ => None,
        }
    }
}

/// Longest `?filter=` the logs socket accepts.
pub const MAX_LOG_FILTER_CHARS: usize = 256;

/// `?fps=&format=` of `WS /ws/simulators/{id}/stream`.
#[derive(Debug, Default, Deserialize)]
pub struct StreamQuery {
    #[serde(default)]
    pub fps: Option<u8>,
    #[serde(default)]
    pub format: ScreenshotFormat,
}

/// `?filter=&level=` of `WS /ws/simulators/{id}/logs`.
#[derive(Debug, Default, Deserialize)]
pub struct LogsQuery {
    #[serde(default)]
    pub filter: Option<String>,
    #[serde(default)]
    pub level: Option<String>,
}

impl LogsQuery {
    /// The filter and minimum level; `Invalid` (400) for an unknown non-empty
    /// level or a filter over [`MAX_LOG_FILTER_CHARS`].
    pub fn validate(self) -> otto_core::Result<(Option<String>, Option<LogLevel>)> {
        let min_level = match self.level.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            None => None,
            Some(raw) => Some(LogLevel::parse_filter(raw).ok_or_else(|| {
                otto_core::Error::Invalid(format!(
                    "unknown log level {raw:?}; expected verbose, debug, info, warn, error, or fatal"
                ))
            })?),
        };
        if let Some(ref f) = self.filter {
            if f.chars().count() > MAX_LOG_FILTER_CHARS {
                return Err(otto_core::Error::Invalid(format!(
                    "log filter exceeds {MAX_LOG_FILTER_CHARS} characters"
                )));
            }
        }
        Ok((self.filter, min_level))
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RecentLogEntry {
    pub ts: String,
    pub level: LogLevel,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecentLogsResp {
    pub device_id: String,
    pub lines: Vec<RecentLogEntry>,
    pub truncated: bool,
}

/// Parse raw stdout from `adb logcat -d -v time` or `simctl spawn log show`,
/// apply optional level/substring filters, and keep the newest `max_lines`.
pub fn parse_and_filter_logs(
    raw: &str,
    filter: Option<&str>,
    min_level: Option<LogLevel>,
    max_lines: usize,
) -> (Vec<RecentLogEntry>, bool) {
    let filter_lc = filter
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty());
    let mut out = Vec::new();
    for line in raw.lines() {
        // Blank lines, logcat's `--------- beginning of main`, log show's column header.
        if line.trim().is_empty() || line.starts_with("---------") || line.starts_with("Timestamp ")
        {
            continue;
        }
        let entry = parse_log_line(line);
        if !keep_log(&entry, min_level, filter_lc.as_deref()) {
            continue;
        }
        if let LogServerMsg::Log {
            ts,
            level,
            tag,
            pid,
            message,
        } = entry
        {
            out.push(RecentLogEntry {
                ts,
                level,
                tag,
                pid,
                message,
            });
        }
    }
    let truncated = out.len() > max_lines;
    out.drain(..out.len().saturating_sub(max_lines));
    (out, truncated)
}

/// The level floor and case-insensitive tag/message match shared by the logs
/// socket and recent logs (`filter_lc` already trimmed and lowercased).
fn keep_log(entry: &LogServerMsg, min_level: Option<LogLevel>, filter_lc: Option<&str>) -> bool {
    let LogServerMsg::Log {
        level,
        tag,
        message,
        ..
    } = entry
    else {
        return true;
    };
    min_level.is_none_or(|min| level.rank() >= min.rank())
        && filter_lc.is_none_or(|f| {
            tag.as_deref()
                .is_some_and(|t| t.to_ascii_lowercase().contains(f))
                || message.to_ascii_lowercase().contains(f)
        })
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LogServerMsg {
    Log {
        ts: String,
        level: LogLevel,
        #[serde(skip_serializing_if = "Option::is_none")]
        tag: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pid: Option<u32>,
        message: String,
    },
    Status {
        state: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
    Error {
        code: String,
        message: String,
    },
}

impl LogServerMsg {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| {
            r#"{"type":"error","code":"internal","message":"serialization failed"}"#.into()
        })
    }
}

/// Optional periodic grant re-check callback provided by the server host.
/// Returning `None` revokes the socket; `Some(can_drive)` updates drive rights.
pub type GrantRecheckFn =
    Box<dyn Fn() -> otto_core::auth::BoxFuture<'static, Option<bool>> + Send + Sync>;

#[derive(Debug, Clone, Copy)]
struct CaptureConfig {
    fps: u8,
    paused: bool,
}

fn encode_screenshot_frame(
    seq: &AtomicU64,
    shot: ScreenshotData,
    last_orientation: &mut Orientation,
) -> Message {
    let next_seq = seq.fetch_add(1, Ordering::Relaxed) + 1;
    if shot.width > 0 && shot.height > 0 {
        *last_orientation = if shot.width > shot.height {
            Orientation::Landscape
        } else {
            Orientation::Portrait
        };
    }
    let hdr = SimFrameHeader {
        seq: next_seq,
        mime: shot.mime,
        width: shot.width,
        height: shot.height,
        orientation: *last_orientation,
        timestamp_ms: chrono::Utc::now().timestamp_millis(),
    };
    Message::Binary(encode_frame(&hdr, &shot.bytes).into())
}

fn spawn_recheck_task(
    recheck: Option<GrantRecheckFn>,
) -> (mpsc::Receiver<Option<bool>>, tokio::task::AbortHandle) {
    let (tx, rx) = mpsc::channel::<Option<bool>>(4);
    let task = tokio::spawn(async move {
        let Some(check) = recheck else {
            std::future::pending::<()>().await;
            return;
        };
        let now = tokio::time::Instant::now();
        let mut tick =
            tokio::time::interval_at(now + Duration::from_secs(5), Duration::from_secs(5));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            let verdict = check().await;
            if tx.send(verdict).await.is_err() || verdict.is_none() {
                break;
            }
        }
    });
    (rx, task.abort_handle())
}

async fn resolve_cached_target(
    service: &SimulatorsService,
    device_id: &str,
    cached_target: &RwLock<Option<BootedTarget>>,
) -> otto_core::Result<BootedTarget> {
    if let Some(target) = cached_target.read().await.clone() {
        return Ok(target);
    }
    let resolved = service.resolve_booted_target(device_id).await?;
    *cached_target.write().await = Some(resolved.clone());
    Ok(resolved)
}

async fn capture_with_cached_target(
    service: &SimulatorsService,
    device_id: &str,
    cached_target: &RwLock<Option<BootedTarget>>,
    format: ScreenshotFormat,
) -> otto_core::Result<ScreenshotData> {
    let target = resolve_cached_target(service, device_id, cached_target).await?;
    match service.screenshot_target(&target, format).await {
        Ok(shot) => Ok(shot),
        Err(_) => {
            *cached_target.write().await = None;
            let refreshed = resolve_cached_target(service, device_id, cached_target).await?;
            service.screenshot_target(&refreshed, format).await
        }
    }
}

async fn send_input_with_cached_target(
    service: &SimulatorsService,
    device_id: &str,
    cached_target: &RwLock<Option<BootedTarget>>,
    action: &InputAction,
) -> otto_core::Result<()> {
    let target = resolve_cached_target(service, device_id, cached_target).await?;
    match service.send_input_target(&target, action).await {
        Ok(_) => Ok(()),
        Err(_) => {
            *cached_target.write().await = None;
            let refreshed = resolve_cached_target(service, device_id, cached_target).await?;
            service
                .send_input_target(&refreshed, action)
                .await
                .map(|_| ())
        }
    }
}

struct CaptureTaskArgs {
    service: Arc<SimulatorsService>,
    device_id: String,
    cached_target: Arc<RwLock<Option<BootedTarget>>>,
    format: ScreenshotFormat,
    initial_orientation: Orientation,
    cfg_rx: watch::Receiver<CaptureConfig>,
    snap_rx: mpsc::Receiver<()>,
    acked_seq: Arc<AtomicU64>,
    out_tx: mpsc::Sender<Message>,
}

fn spawn_capture_task(args: CaptureTaskArgs) -> tokio::task::AbortHandle {
    let CaptureTaskArgs {
        service,
        device_id,
        cached_target,
        format,
        initial_orientation,
        mut cfg_rx,
        mut snap_rx,
        acked_seq,
        out_tx,
    } = args;
    let task = tokio::spawn(async move {
        let frame_seq = AtomicU64::new(0);
        let mut last_orientation = initial_orientation;
        let mut cfg = *cfg_rx.borrow();
        let mut tick = tokio::time::interval(Duration::from_millis(1000 / u64::from(cfg.fps)));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                changed = cfg_rx.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    let next = *cfg_rx.borrow();
                    if next.fps != cfg.fps {
                        tick = tokio::time::interval(Duration::from_millis(1000 / u64::from(next.fps)));
                        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                    }
                    cfg = next;
                }
                snap = snap_rx.recv() => {
                    if snap.is_none() {
                        break;
                    }
                    if let Ok(shot) = capture_with_cached_target(&service, &device_id, &cached_target, format).await {
                        let msg = encode_screenshot_frame(&frame_seq, shot, &mut last_orientation);
                        if out_tx.send(msg).await.is_err() {
                            break;
                        }
                    }
                }
                _ = tick.tick() => {
                    if cfg.paused {
                        continue;
                    }
                    // Gate periodic capture on client ACK credit budget so stalled
                    // viewers do not keep spawning screenshot subprocesses.
                    let sent = frame_seq.load(Ordering::Relaxed);
                    let acked = acked_seq.load(Ordering::Acquire);
                    if sent.saturating_sub(acked) >= MAX_IN_FLIGHT_FRAMES {
                        continue;
                    }
                    match capture_with_cached_target(&service, &device_id, &cached_target, format).await {
                        Ok(shot) => {
                            let msg = encode_screenshot_frame(&frame_seq, shot, &mut last_orientation);
                            let _ = out_tx.try_send(msg);
                        }
                        Err(e) => {
                            let _ = out_tx.try_send(Message::Text(
                                ServerStreamMsg::error("capture_failed", e.to_string()).to_json().into()
                            ));
                        }
                    }
                }
            }
        }
    });
    task.abort_handle()
}

fn spawn_input_task(
    service: Arc<SimulatorsService>,
    device_id: String,
    cached_target: Arc<RwLock<Option<BootedTarget>>>,
    can_drive_flag: Arc<AtomicBool>,
    out_tx: mpsc::Sender<Message>,
) -> (mpsc::Sender<ClientStreamMsg>, tokio::task::AbortHandle) {
    let (in_tx, mut in_rx) = mpsc::channel::<ClientStreamMsg>(64);
    let task = tokio::spawn(async move {
        while let Some(msg) = in_rx.recv().await {
            // Re-check drive permission before dispatching each queued action so a
            // grant downgrade immediately drops buffered commands.
            if !can_drive_flag.load(Ordering::Acquire) {
                continue;
            }
            let res: otto_core::Result<()> = match msg {
                ClientStreamMsg::Input(action) => {
                    send_input_with_cached_target(&service, &device_id, &cached_target, &action)
                        .await
                }
                ClientStreamMsg::Control(StreamControlMsg::OpenUrl {
                    url,
                    auto_forward_localhost,
                }) => service
                    .open_url(&device_id, &url, auto_forward_localhost.unwrap_or(true))
                    .await
                    .map(|_| ()),
                ClientStreamMsg::Control(StreamControlMsg::Orientation { orientation }) => {
                    let r = service
                        .set_orientation(&device_id, orientation)
                        .await
                        .map(|_| ());
                    // Refresh cached target so Android orientation changes take effect on next input.
                    *cached_target.write().await = None;
                    r
                }
                ClientStreamMsg::Control(StreamControlMsg::Appearance { appearance }) => service
                    .set_appearance(&device_id, appearance)
                    .await
                    .map(|_| ()),
                _ => Ok(()),
            };
            if let Err(e) = res {
                let _ = out_tx
                    .send(Message::Text(
                        ServerStreamMsg::error("action_failed", e.to_string())
                            .to_json()
                            .into(),
                    ))
                    .await;
            }
        }
    });
    (in_tx, task.abort_handle())
}

/// Serve a live simulator screencast + bidirectional input WebSocket.
pub async fn serve_stream_socket(
    socket: WebSocket,
    service: Arc<SimulatorsService>,
    device_id: String,
    initial_fps: u8,
    format: ScreenshotFormat,
    mut can_drive: bool,
    recheck: Option<GrantRecheckFn>,
) {
    let (mut sink, mut stream) = socket.split();
    let fps = initial_fps.clamp(1, 10);

    let initial_dev = match service.get_device(&device_id).await {
        Ok(d) => d,
        Err(e) => {
            let _ = sink
                .send(Message::Text(
                    ServerStreamMsg::error("not_found", e.to_string())
                        .to_json()
                        .into(),
                ))
                .await;
            let _ = sink.send(Message::Close(None)).await;
            return;
        }
    };

    if sink
        .send(Message::Text(
            ServerStreamMsg::Hello {
                device: initial_dev.clone(),
                fps,
                can_drive,
            }
            .to_json()
            .into(),
        ))
        .await
        .is_err()
    {
        return;
    }

    let (out_tx, mut out_rx) = mpsc::channel::<Message>(16);
    let writer = tokio::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            if sink.send(msg).await.is_err() {
                break;
            }
        }
    });

    let initial_target = service.resolve_booted_target(&device_id).await.ok();
    let cached_target = Arc::new(RwLock::new(initial_target));
    let can_drive_flag = Arc::new(AtomicBool::new(can_drive));
    let acked_seq = Arc::new(AtomicU64::new(0));

    let (in_tx, input_abort) = spawn_input_task(
        service.clone(),
        device_id.clone(),
        cached_target.clone(),
        can_drive_flag.clone(),
        out_tx.clone(),
    );
    let (cfg_tx, cfg_rx) = watch::channel(CaptureConfig { fps, paused: false });
    let (snap_tx, snap_rx) = mpsc::channel::<()>(4);
    let capture_abort = spawn_capture_task(CaptureTaskArgs {
        service,
        device_id,
        cached_target,
        format,
        initial_orientation: initial_dev.orientation,
        cfg_rx,
        snap_rx,
        acked_seq: acked_seq.clone(),
        out_tx: out_tx.clone(),
    });
    let (mut recheck_rx, recheck_abort) = spawn_recheck_task(recheck);

    let now = tokio::time::Instant::now();
    let mut ping_tick =
        tokio::time::interval_at(now + Duration::from_secs(30), Duration::from_secs(30));
    ping_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = ping_tick.tick() => {
                if out_tx.send(Message::Ping(Vec::new().into())).await.is_err() {
                    break;
                }
            }
            verdict = recheck_rx.recv() => {
                match verdict {
                    None | Some(None) => {
                        can_drive_flag.store(false, Ordering::Release);
                        let _ = out_tx.send(Message::Text(
                            ServerStreamMsg::Closed { reason: "revoked".into() }.to_json().into()
                        )).await;
                        break;
                    }
                    Some(Some(still_drive)) => {
                        can_drive = can_drive && still_drive;
                        can_drive_flag.store(can_drive, Ordering::Release);
                    }
                }
            }
            maybe = stream.next() => {
                let Some(Ok(msg)) = maybe else {
                    break;
                };
                match msg {
                    Message::Text(txt) => {
                        handle_client_text(
                            &txt,
                            can_drive,
                            &acked_seq,
                            &cfg_tx,
                            &snap_tx,
                            &in_tx,
                            &out_tx,
                        )
                        .await;
                    }
                    Message::Close(_) => break,
                    Message::Pong(_) | Message::Ping(_) | Message::Binary(_) => {}
                }
            }
        }
    }

    capture_abort.abort();
    recheck_abort.abort();
    drop(in_tx);
    input_abort.abort();
    drop(out_tx);
    let _ = writer.await;
}

async fn handle_client_text(
    txt: &str,
    can_drive: bool,
    acked_seq: &AtomicU64,
    cfg_tx: &watch::Sender<CaptureConfig>,
    snap_tx: &mpsc::Sender<()>,
    in_tx: &mpsc::Sender<ClientStreamMsg>,
    out_tx: &mpsc::Sender<Message>,
) {
    match ClientStreamMsg::parse(txt) {
        Ok(ClientStreamMsg::Control(StreamControlMsg::Ack { seq })) => {
            acked_seq.fetch_max(seq, Ordering::Release);
        }
        Ok(ClientStreamMsg::Control(StreamControlMsg::Pause)) => {
            cfg_tx.send_modify(|c| c.paused = true);
        }
        Ok(ClientStreamMsg::Control(StreamControlMsg::Resume)) => {
            cfg_tx.send_modify(|c| c.paused = false);
        }
        Ok(ClientStreamMsg::Control(StreamControlMsg::SetFps { fps })) => {
            let clamped = fps.clamp(1, 10);
            cfg_tx.send_modify(|c| c.fps = clamped);
        }
        Ok(ClientStreamMsg::Control(StreamControlMsg::Snapshot)) => {
            let _ = snap_tx.try_send(());
        }
        Ok(drive_msg) if drive_msg.is_drive() => {
            if !can_drive {
                let _ = out_tx
                    .send(Message::Text(
                        ServerStreamMsg::error("forbidden", "drive requires Browser:Edit")
                            .to_json()
                            .into(),
                    ))
                    .await;
            } else {
                let _ = in_tx.try_send(drive_msg);
            }
        }
        Ok(_) => {}
        Err(e) => {
            let _ = out_tx
                .send(Message::Text(
                    ServerStreamMsg::error("invalid", e).to_json().into(),
                ))
                .await;
        }
    }
}

/// Serve a live device log WebSocket (`simctl spawn log stream` or `adb logcat`).
pub async fn serve_logs_socket(
    socket: WebSocket,
    service: Arc<SimulatorsService>,
    device_id: String,
    filter: Option<String>,
    min_level: Option<LogLevel>,
    recheck: Option<GrantRecheckFn>,
) {
    let (mut sink, mut stream) = socket.split();
    let filter_lc = filter
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty());

    let mut handle = match service.spawn_logs(&device_id).await {
        Ok(h) => h,
        Err(e) => {
            let _ = sink
                .send(Message::Text(
                    LogServerMsg::Error {
                        code: "unavailable".into(),
                        message: e.to_string(),
                    }
                    .to_json()
                    .into(),
                ))
                .await;
            let _ = sink.send(Message::Close(None)).await;
            return;
        }
    };

    if sink
        .send(Message::Text(
            LogServerMsg::Status {
                state: "streaming".into(),
                detail: Some(device_id.clone()),
            }
            .to_json()
            .into(),
        ))
        .await
        .is_err()
    {
        return;
    }

    let (mut recheck_rx, recheck_abort) = spawn_recheck_task(recheck);
    let now = tokio::time::Instant::now();
    let mut ping_tick =
        tokio::time::interval_at(now + Duration::from_secs(30), Duration::from_secs(30));
    ping_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            line = handle.rx.recv() => {
                let Some(raw) = line else {
                    let _ = sink.send(Message::Text(
                        LogServerMsg::Status {
                            state: "ended".into(),
                            detail: None,
                        }
                        .to_json()
                        .into(),
                    )).await;
                    break;
                };
                let entry = parse_log_line(&raw);
                if !keep_log(&entry, min_level, filter_lc.as_deref()) {
                    continue;
                }
                if sink.send(Message::Text(entry.to_json().into())).await.is_err() {
                    break;
                }
            }
            _ = ping_tick.tick() => {
                if sink.send(Message::Ping(Vec::new().into())).await.is_err() {
                    break;
                }
            }
            verdict = recheck_rx.recv() => {
                if matches!(verdict, None | Some(None)) {
                    break;
                }
            }
            maybe = stream.next() => {
                match maybe {
                    None | Some(Err(_)) | Some(Ok(Message::Close(_))) => break,
                    _ => {}
                }
            }
        }
    }
    recheck_abort.abort();
    let _ = sink.send(Message::Close(None)).await;
}

/// Parse an Android `logcat -v time` or iOS `log stream --style compact` line
/// into a structured [`LogServerMsg::Log`].
pub fn parse_log_line(raw: &str) -> LogServerMsg {
    let trimmed = raw.trim_end_matches(['\r', '\n']);
    // Android logcat `-v time`: `11-19 17:30:01.123 D/MyTag( 1234): hello world`
    // Guard UTF-8 boundary + ASCII timestamp prefix before slicing at byte 18.
    if trimmed.len() > 22
        && trimmed.is_char_boundary(18)
        && trimmed.as_bytes()[..18].is_ascii()
        && trimmed.as_bytes()[2] == b'-'
        && trimmed.as_bytes()[5] == b' '
    {
        let ts = trimmed[..18].to_string();
        let rest = trimmed[18..].trim_start();
        if let Some((meta, msg)) = rest.split_once(": ") {
            if let Some((lvl_ch, tag_pid)) = meta.split_once('/') {
                let level = match lvl_ch.trim() {
                    "V" => LogLevel::Verbose,
                    "D" => LogLevel::Debug,
                    "I" => LogLevel::Info,
                    "W" => LogLevel::Warn,
                    "E" => LogLevel::Error,
                    "F" | "A" => LogLevel::Fatal,
                    _ => LogLevel::Unknown,
                };
                let (tag, pid) = if let Some((t, p)) = tag_pid.rsplit_once('(') {
                    let pid = p.trim_end_matches(')').trim().parse::<u32>().ok();
                    (Some(t.trim().to_string()), pid)
                } else {
                    (Some(tag_pid.trim().to_string()), None)
                };
                return LogServerMsg::Log {
                    ts,
                    level,
                    tag,
                    pid,
                    message: msg.to_string(),
                };
            }
        }
    }

    // iOS `log stream --style compact`: `2026-11-19 17:30:01.123 Df SpringBoard[456:789] message`
    let parts: Vec<&str> = trimmed.splitn(5, ' ').collect();
    if parts.len() == 5 && parts[0].len() == 10 && parts[0].contains('-') {
        let ts = format!("{} {}", parts[0], parts[1]);
        let level = match parts[2] {
            "Db" | "Debug" => LogLevel::Debug,
            "Df" | "Info" | "Default" => LogLevel::Info,
            "E" | "Error" => LogLevel::Error,
            "F" | "Fault" => LogLevel::Fatal,
            _ => LogLevel::Info,
        };
        let (tag, pid) = if let Some((proc_name, rest)) = parts[3].split_once('[') {
            let pid = rest
                .split([':', ']'])
                .next()
                .and_then(|s| s.parse::<u32>().ok());
            (Some(proc_name.to_string()), pid)
        } else {
            (Some(parts[3].to_string()), None)
        };
        return LogServerMsg::Log {
            ts,
            level,
            tag,
            pid,
            message: parts[4].to_string(),
        };
    }

    LogServerMsg::Log {
        ts: chrono::Utc::now().to_rfc3339(),
        level: LogLevel::Info,
        tag: None,
        pid: None,
        message: trimmed.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_logs_keep_the_newest_lines_matching_level_and_tag_or_message() {
        let raw = "--------- beginning of main\n\
            11-19 17:30:01.100 E/App( 200): first boom\n\
            11-19 17:30:01.200 W/BoomTag( 300): tag-only match\n\
            11-19 17:30:01.300 E/App( 200): unrelated error\n\
            11-19 17:30:01.400 D/Net( 100): boom below the floor\n\
            11-19 17:30:01.500 F/App( 200): last BOOM\n";
        let (lines, truncated) =
            parse_and_filter_logs(raw, Some(" Boom "), Some(LogLevel::Warn), 2);
        let msgs: Vec<&str> = lines.iter().map(|l| l.message.as_str()).collect();
        assert_eq!(msgs, ["tag-only match", "last BOOM"]);
        assert!(truncated, "three lines matched, the newest two kept");
        assert_eq!((lines[1].level, lines[1].pid), (LogLevel::Fatal, Some(200)));
        let (all, truncated) = parse_and_filter_logs(raw, None, None, 10);
        assert_eq!(
            (all.len(), truncated),
            (5, false),
            "the separator is no log line"
        );
    }

    #[test]
    fn logs_query_rejects_unknown_level_and_oversized_filter() {
        let q = |level: &str, filter: String| LogsQuery {
            filter: Some(filter),
            level: Some(level.into()),
        };
        let (f, lvl) = q(" E ", "boom".into()).validate().unwrap();
        assert_eq!((f.as_deref(), lvl), (Some("boom"), Some(LogLevel::Error)));
        assert_eq!(q("", "x".into()).validate().unwrap().1, None);
        assert!(q("bogus", "x".into()).validate().is_err());
        let long = "é".repeat(MAX_LOG_FILTER_CHARS + 1);
        assert!(q("info", long).validate().is_err());
        assert!(q("info", "é".repeat(MAX_LOG_FILTER_CHARS))
            .validate()
            .is_ok());
    }

    #[test]
    fn parses_android_ios_and_non_ascii_log_lines_without_panicking() {
        let and = parse_log_line("11-19 17:30:01.123 E/ReactNativeJS( 4210): TypeError: boom");
        match and {
            LogServerMsg::Log {
                ts,
                level,
                tag,
                pid,
                message,
            } => {
                assert_eq!(ts, "11-19 17:30:01.123");
                assert_eq!(level, LogLevel::Error);
                assert_eq!(tag.as_deref(), Some("ReactNativeJS"));
                assert_eq!(pid, Some(4210));
                assert_eq!(message, "TypeError: boom");
            }
            _ => panic!("expected Log"),
        }

        let ios = parse_log_line("2026-11-19 17:30:01.123 E MyApp[812:1234] Failed to fetch");
        match ios {
            LogServerMsg::Log {
                level,
                tag,
                pid,
                message,
                ..
            } => {
                assert_eq!(level, LogLevel::Error);
                assert_eq!(tag.as_deref(), Some("MyApp"));
                assert_eq!(pid, Some(812));
                assert_eq!(message, "Failed to fetch");
            }
            _ => panic!("expected Log"),
        }

        // Byte 18 falls inside a multi-byte UTF-8 char; must not panic.
        let utf8 = parse_log_line("01-02 x你好世界你好世界");
        assert!(matches!(utf8, LogServerMsg::Log { .. }));
    }

    #[test]
    fn parses_client_stream_messages() {
        assert!(!ClientStreamMsg::parse(r#"{"type":"ack","seq":1}"#)
            .unwrap()
            .is_drive());
        assert!(!ClientStreamMsg::parse(r#"{"type":"set_fps","fps":5}"#)
            .unwrap()
            .is_drive());
        assert!(
            ClientStreamMsg::parse(r#"{"type":"tap","x":0.5,"y":0.5,"normalized":true}"#)
                .unwrap()
                .is_drive()
        );
        assert!(
            ClientStreamMsg::parse(r#"{"type":"open_url","url":"http://localhost:5173"}"#)
                .unwrap()
                .is_drive()
        );
        assert!(ClientStreamMsg::parse(r#"{"type":"unknown"}"#).is_err());
    }

    fn setup_ws_ios_service() -> (
        Arc<crate::exec::fake::FakeCommandRunner>,
        Arc<SimulatorsService>,
    ) {
        use crate::exec::fake::{ok_out, FakeCommandRunner, FakeHostProbe};
        let runner = Arc::new(FakeCommandRunner::new());
        let probe = Arc::new(FakeHostProbe::new());
        probe.add_path("/Applications/Xcode.app");
        probe.add_which("xcrun", "/usr/bin/xcrun");
        probe.add_which("idb", "/opt/homebrew/bin/idb");
        runner.on_exact(
            "xcode-select",
            &["-p"],
            ok_out(b"/Applications/Xcode.app/Contents/Developer\n"),
        );
        runner.on_exact("xcodebuild", &["-version"], ok_out(b"Xcode 16.2\n"));
        runner.on_exact(
            "xcrun",
            &["simctl", "list", "runtimes", "-j"],
            ok_out(
                br#"{"runtimes":[{"identifier":"com.apple.CoreSimulator.SimRuntime.iOS-18-2","name":"iOS 18.2","isAvailable":true}]}"#,
            ),
        );
        runner.on_exact(
            "xcrun",
            &["simctl", "list", "devices", "-j"],
            ok_out(
                br#"{"devices":{"com.apple.CoreSimulator.SimRuntime.iOS-18-2":[{"udid":"IOS-WS-1","name":"iPhone 16 Pro","state":"Booted","isAvailable":true}]}}"#,
            ),
        );
        runner.on_exact(
            "xcrun",
            &["simctl", "list", "-j"],
            ok_out(
                br#"{"runtimes":[{"identifier":"com.apple.CoreSimulator.SimRuntime.iOS-18-2","name":"iOS 18.2","version":"18.2","isAvailable":true}],"devicetypes":[],"devices":{"com.apple.CoreSimulator.SimRuntime.iOS-18-2":[{"udid":"IOS-WS-1","name":"iPhone 16 Pro","state":"Booted","isAvailable":true,"deviceTypeIdentifier":"com.apple.CoreSimulator.SimDeviceType.iPhone-16-Pro"}]}}"#,
            ),
        );
        runner.on(|spec| {
            if spec.program.ends_with("xcrun")
                && spec.args.len() == 6
                && spec.args[0] == "simctl"
                && spec.args[1] == "io"
                && spec.args[2] == "IOS-WS-1"
                && spec.args[3] == "screenshot"
            {
                let mut png = Vec::new();
                png.extend_from_slice(b"\x89PNG\r\n\x1a\n");
                png.extend_from_slice(&13u32.to_be_bytes());
                png.extend_from_slice(b"IHDR");
                png.extend_from_slice(&1179u32.to_be_bytes());
                png.extend_from_slice(&2556u32.to_be_bytes());
                png.extend_from_slice(&[8, 6, 0, 0, 0]);
                std::fs::write(&spec.args[5], png).unwrap();
                Some(ok_out(b""))
            } else {
                None
            }
        });
        runner.on_exact(
            "idb",
            &["ui", "tap", "--udid", "IOS-WS-1", "197", "426"],
            ok_out(b""),
        );
        let svc = Arc::new(SimulatorsService::with_deps(runner.clone(), probe));
        (runner, svc)
    }

    #[tokio::test]
    async fn queued_drive_actions_are_dropped_when_can_drive_downgrades() {
        let (runner, svc) = setup_ws_ios_service();
        let target = svc.resolve_booted_target("ios:IOS-WS-1").await.unwrap();
        let cached_target = Arc::new(RwLock::new(Some(target)));
        let can_drive_flag = Arc::new(AtomicBool::new(false));
        let (out_tx, _out_rx) = mpsc::channel(8);

        runner.clear_recorded();
        let (in_tx, abort) = spawn_input_task(
            svc,
            "ios:IOS-WS-1".into(),
            cached_target,
            can_drive_flag,
            out_tx,
        );
        for _ in 0..5 {
            in_tx
                .send(ClientStreamMsg::Input(InputAction::Tap {
                    x: 0.5,
                    y: 0.5,
                    normalized: true,
                }))
                .await
                .unwrap();
        }
        tokio::time::sleep(Duration::from_millis(80)).await;
        abort.abort();

        assert!(
            runner.recorded().is_empty(),
            "queued drive actions must not execute after can_drive becomes false"
        );
    }

    #[tokio::test]
    async fn capture_task_gates_ticks_on_ack_budget_and_skips_reenumeration() {
        let (runner, svc) = setup_ws_ios_service();
        let target = svc.resolve_booted_target("ios:IOS-WS-1").await.unwrap();
        let cached_target = Arc::new(RwLock::new(Some(target)));
        let (_cfg_tx, cfg_rx) = watch::channel(CaptureConfig {
            fps: 10,
            paused: false,
        });
        let (_snap_tx, snap_rx) = mpsc::channel(4);
        let (out_tx, mut out_rx) = mpsc::channel(16);
        let acked_seq = Arc::new(AtomicU64::new(0));

        runner.clear_recorded();
        let abort = spawn_capture_task(CaptureTaskArgs {
            service: svc,
            device_id: "ios:IOS-WS-1".into(),
            cached_target,
            format: ScreenshotFormat::Png,
            initial_orientation: Orientation::Portrait,
            cfg_rx,
            snap_rx,
            acked_seq: acked_seq.clone(),
            out_tx,
        });

        // Wait for MAX_IN_FLIGHT_FRAMES (4) frames to arrive.
        let mut received = 0u64;
        while received < MAX_IN_FLIGHT_FRAMES {
            let msg = tokio::time::timeout(Duration::from_secs(2), out_rx.recv())
                .await
                .unwrap()
                .unwrap();
            if matches!(msg, Message::Binary(_)) {
                received += 1;
            }
        }
        // Sleep past several 100ms ticks without ACKing — no extra frames should be captured.
        tokio::time::sleep(Duration::from_millis(350)).await;
        assert!(
            out_rx.try_recv().is_err(),
            "capture must pause once MAX_IN_FLIGHT_FRAMES are unacknowledged"
        );
        let before_ack_shots = runner
            .recorded()
            .iter()
            .filter(|(_, args)| args.get(1).map(String::as_str) == Some("io"))
            .count();
        assert_eq!(before_ack_shots as u64, MAX_IN_FLIGHT_FRAMES);

        // Advance ACK and verify capture resumes with 0 per-frame `simctl list` calls.
        acked_seq.store(MAX_IN_FLIGHT_FRAMES, Ordering::Release);
        let next_msg = tokio::time::timeout(Duration::from_secs(2), out_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(next_msg, Message::Binary(_)));
        abort.abort();

        let list_calls = runner
            .recorded()
            .iter()
            .filter(|(_, args)| args.get(1).map(String::as_str) == Some("list"))
            .count();
        assert_eq!(list_calls, 0, "capture task must not re-enumerate devices");
    }
}
