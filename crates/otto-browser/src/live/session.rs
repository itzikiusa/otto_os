//! One remote live session: a page target (in its own browser context when
//! ephemeral) driven over the process's CDP pipe. Owns the screencast (with
//! per-viewer ack backpressure + adaptive quality), viewer input dispatch,
//! the human/agent control lock, navigation, screenshots, JS dialogs, and the
//! screenshot-then-approve hold for an agent's outward actions.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, Weak};
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::conn::CdpEvent;
use super::control::{ControlError, ControlLock};
use super::flow::{Adaptive, ViewerFlow};
use super::guard::{self, Paused};
use super::hooks::{LiveAudit, LiveHooks, OutwardAction};
use super::input;
use super::process::{download_behavior, ephemeral_downloads_dir, ChromeProcess, ProcessKey};
use super::protocol::{
    encode_frame, jpeg_size, ClientMsg, ControlAction, FrameHeader, NavAction, ServerMsg,
};
use super::runtime::{cdp_live, LiveError};
use super::types::{
    safe_segment, ControllerKind, LiveSessionInfo, SessionState, Viewport, MAX_VIEWERS,
};

/// Per-input CDP call budget (a page stuck in a JS dialog never answers).
const INPUT_TIMEOUT: Duration = Duration::from_secs(5);
/// Mouse moves closer together than this are coalesced (dropped).
const MOVE_COALESCE: Duration = Duration::from_millis(8);
/// Cursor-shape probes at most this often.
const CURSOR_PROBE_EVERY: Duration = Duration::from_millis(100);
/// An unanswered JS dialog is dismissed after this long.
const DIALOG_AUTO_DISMISS: Duration = Duration::from_secs(60);
/// How long an agent's outward action waits for a human decision.
pub const OUTWARD_APPROVAL_TIMEOUT: Duration = Duration::from_secs(600);
/// Outbound queue per viewer (frames are window-limited, so this only fills
/// when the viewer's socket is wedged — it is then dropped).
const VIEWER_QUEUE: usize = 64;
/// Full-page captures are clipped to this many CSS px per side.
pub const MAX_CAPTURE_PX: f64 = 16_384.0;

/// What a viewer's WebSocket task sends.
#[derive(Debug, Clone)]
pub enum ViewerOut {
    Text(String),
    Frame(Arc<Vec<u8>>),
}

struct Viewer {
    user_id: String,
    can_drive: bool,
    warned: bool,
    tx: mpsc::Sender<ViewerOut>,
    flow: ViewerFlow<Arc<Vec<u8>>>,
}

struct Inner {
    state: SessionState,
    url: String,
    title: String,
    loading: bool,
    can_go_back: bool,
    can_go_forward: bool,
    viewport: Viewport,
    control: ControlLock,
    created_at: String,
    last_activity: Instant,
    last_activity_at: String,
    screencast_on: bool,
    adaptive: Adaptive,
    pending_cdp_ack: Option<i64>,
    last_move: Option<Instant>,
    last_probe: Option<Instant>,
    cursor: String,
    dialog_seq: u64,
}

/// What opening a session needs.
#[derive(Debug, Clone)]
pub struct OpenParams {
    pub tab_id: String,
    pub workspace_id: String,
    pub owner_id: String,
    pub profile: String,
    pub viewport: Viewport,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ScreenshotMode {
    #[default]
    Viewport,
    FullPage,
    Element,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ImageFormat {
    #[default]
    Png,
    Jpeg,
}

impl ImageFormat {
    pub fn mime(self) -> &'static str {
        match self {
            ImageFormat::Png => "image/png",
            ImageFormat::Jpeg => "image/jpeg",
        }
    }
}

/// `BrowserScreenshotReq`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ScreenshotRequest {
    #[serde(default)]
    pub mode: ScreenshotMode,
    #[serde(default)]
    pub selector: Option<String>,
    #[serde(default)]
    pub format: ImageFormat,
    #[serde(default)]
    pub quality: Option<u8>,
}

pub struct Screenshot {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
    pub url: String,
}

/// A clip rectangle in CSS px.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Clip {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// `Page.captureScreenshot` params. Pure — unit-tested.
pub fn screenshot_params(format: ImageFormat, quality: Option<u8>, clip: Option<Clip>) -> Value {
    let mut p = json!({
        "format": match format { ImageFormat::Png => "png", ImageFormat::Jpeg => "jpeg" },
    });
    if format == ImageFormat::Jpeg {
        p["quality"] = json!(quality.unwrap_or(80).clamp(1, 100));
    }
    if let Some(c) = clip {
        let w = c.width.clamp(1.0, MAX_CAPTURE_PX);
        let h = c.height.clamp(1.0, MAX_CAPTURE_PX);
        p["clip"] =
            json!({"x": c.x.max(0.0), "y": c.y.max(0.0), "width": w, "height": h, "scale": 1});
        p["captureBeyondViewport"] = json!(true);
    }
    p
}

/// JS returning the document-relative box of the first `selector` match
/// (`null` when nothing matches). The selector is JSON-escaped.
pub fn element_box_expr(selector: &str) -> String {
    let sel = serde_json::to_string(selector).unwrap_or_else(|_| "\"\"".into());
    format!(
        "(function(){{var e;try{{e=document.querySelector({sel});}}catch(_){{return null;}}\
         if(!e)return null;e.scrollIntoView({{block:'nearest',inline:'nearest'}});\
         var r=e.getBoundingClientRect();\
         return {{x:r.left+window.scrollX,y:r.top+window.scrollY,w:r.width,h:r.height}};}})()"
    )
}

pub struct LiveSession {
    pub tab_id: String,
    pub workspace_id: String,
    pub owner_id: String,
    pub profile: String,
    proc: Arc<ChromeProcess>,
    target_id: String,
    sid: String,
    context_id: Option<String>,
    downloads_dir: PathBuf,
    data_dir: PathBuf,
    hooks: Arc<dyn LiveHooks>,
    inner: StdMutex<Inner>,
    viewers: StdMutex<HashMap<u64, Viewer>>,
    next_viewer: AtomicU64,
    frame_seq: AtomicU64,
    closed: AtomicBool,
    event_task: StdMutex<Option<JoinHandle<()>>>,
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

impl LiveSession {
    /// Create the context/target, attach, enable the domains, and navigate.
    pub async fn create(
        proc: Arc<ChromeProcess>,
        p: OpenParams,
        data_dir: PathBuf,
        hooks: Arc<dyn LiveHooks>,
    ) -> Result<Arc<Self>, LiveError> {
        let conn = proc.conn.clone();
        let vp = p.viewport.clamped();
        let ephemeral = proc.key == ProcessKey::Ephemeral;

        let context_id = if ephemeral {
            let r = conn
                .call(
                    "Target.createBrowserContext",
                    json!({"disposeOnDetach": true}),
                    None,
                )
                .await
                .map_err(cdp_live)?;
            Some(
                r["browserContextId"]
                    .as_str()
                    .ok_or_else(|| LiveError::Engine("no browserContextId".into()))?
                    .to_string(),
            )
        } else {
            None
        };
        let downloads_dir = if ephemeral {
            ephemeral_downloads_dir(&data_dir, &p.workspace_id, &p.owner_id)
        } else {
            proc.downloads_dir.clone()
        };

        let target = Self::create_target(&proc, context_id.as_deref(), &downloads_dir, vp).await;
        let (target_id, sid) = match target {
            Ok(t) => t,
            Err(e) => {
                if let Some(ctx) = &context_id {
                    let _ = conn
                        .call(
                            "Target.disposeBrowserContext",
                            json!({"browserContextId": ctx}),
                            None,
                        )
                        .await;
                }
                return Err(e);
            }
        };

        let (tx, rx) = mpsc::unbounded_channel::<CdpEvent>();
        conn.route(&sid, tx);
        let now = Instant::now();
        let session = Arc::new(Self {
            tab_id: p.tab_id.clone(),
            workspace_id: p.workspace_id.clone(),
            owner_id: p.owner_id.clone(),
            profile: p.profile.clone(),
            proc: proc.clone(),
            target_id: target_id.clone(),
            sid: sid.clone(),
            context_id,
            downloads_dir,
            data_dir,
            hooks,
            inner: StdMutex::new(Inner {
                state: SessionState::Starting,
                url: "about:blank".into(),
                title: String::new(),
                loading: false,
                can_go_back: false,
                can_go_forward: false,
                viewport: vp,
                control: ControlLock::default(),
                created_at: now_rfc3339(),
                last_activity: now,
                last_activity_at: now_rfc3339(),
                screencast_on: false,
                adaptive: Adaptive::new(now),
                pending_cdp_ack: None,
                last_move: None,
                last_probe: None,
                cursor: "default".into(),
                dialog_seq: 0,
            }),
            viewers: StdMutex::new(HashMap::new()),
            next_viewer: AtomicU64::new(1),
            frame_seq: AtomicU64::new(0),
            closed: AtomicBool::new(false),
            event_task: StdMutex::new(None),
        });
        proc.register(&target_id, &session);
        let task = tokio::spawn(session_loop(Arc::downgrade(&session), rx));
        if let Ok(mut t) = session.event_task.lock() {
            *t = Some(task);
        }

        if let Err(e) = session.enable_domains(vp).await {
            session.close("closed").await;
            return Err(e);
        }
        session.set_state(SessionState::Ready);
        if let Some(url) = &p.url {
            if let Err(e) = session.navigate(NavAction::Goto, Some(url.clone())).await {
                tracing::info!("browser live: initial navigation failed: {e}");
            }
        }
        session.hooks.session_changed(&session.info());
        Ok(session)
    }

    async fn create_target(
        proc: &Arc<ChromeProcess>,
        context_id: Option<&str>,
        downloads_dir: &std::path::Path,
        vp: Viewport,
    ) -> Result<(String, String), LiveError> {
        let conn = &proc.conn;
        if let Some(ctx) = context_id {
            conn.call(
                "Browser.setDownloadBehavior",
                download_behavior(proc.download_policy, downloads_dir, Some(ctx)),
                None,
            )
            .await
            .map_err(cdp_live)?;
        }
        let mut create = json!({"url": "about:blank", "width": vp.width, "height": vp.height});
        if let Some(ctx) = context_id {
            create["browserContextId"] = json!(ctx);
        }
        let r = conn
            .call("Target.createTarget", create, None)
            .await
            .map_err(cdp_live)?;
        let target_id = r["targetId"]
            .as_str()
            .ok_or_else(|| LiveError::Engine("Target.createTarget: no targetId".into()))?
            .to_string();
        let r = conn
            .call(
                "Target.attachToTarget",
                json!({"targetId": target_id, "flatten": true}),
                None,
            )
            .await
            .map_err(cdp_live)?;
        let sid = r["sessionId"]
            .as_str()
            .ok_or_else(|| LiveError::Engine("Target.attachToTarget: no sessionId".into()))?
            .to_string();
        Ok((target_id, sid))
    }

    async fn enable_domains(&self, vp: Viewport) -> Result<(), LiveError> {
        let sid = Some(self.sid.as_str());
        let conn = &self.proc.conn;
        if !self.proc.browser_fetch {
            // Per-target fallback: without interception this page would browse
            // unguarded by the Fetch pump — refuse (fail closed).
            conn.call(
                "Fetch.enable",
                json!({"patterns": [{"urlPattern": "*", "requestStage": "Request"}]}),
                sid,
            )
            .await
            .map_err(|e| {
                LiveError::Engine(format!("cannot intercept this page's requests: {e}"))
            })?;
        }
        conn.call("Page.enable", json!({}), sid)
            .await
            .map_err(cdp_live)?;
        let _ = conn
            .call(
                "Page.setInterceptFileChooserDialog",
                json!({"enabled": true}),
                sid,
            )
            .await;
        let _ = conn
            .call(
                "Emulation.setFocusEmulationEnabled",
                json!({"enabled": true}),
                sid,
            )
            .await;
        conn.call(
            "Emulation.setDeviceMetricsOverride",
            metrics_params(vp),
            sid,
        )
        .await
        .map_err(cdp_live)?;
        Ok(())
    }

    // ---------------------------------------------------------------- state

    pub fn info(&self) -> LiveSessionInfo {
        let viewers = self.viewers.lock().map(|v| v.len()).unwrap_or(0) as u32;
        let i = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        LiveSessionInfo {
            tab_id: self.tab_id.clone(),
            workspace_id: self.workspace_id.clone(),
            owner_id: self.owner_id.clone(),
            engine: "remote",
            build: self.proc.build,
            version: self.proc.version.clone(),
            profile: self.profile.clone(),
            headed: self.proc.headed,
            state: i.state,
            url: i.url.clone(),
            title: i.title.clone(),
            loading: i.loading,
            can_go_back: i.can_go_back,
            can_go_forward: i.can_go_forward,
            viewport: i.viewport,
            controller: i.control.kind(),
            controller_user_id: i.control.human_user().map(str::to_string),
            viewers,
            created_at: i.created_at.clone(),
            last_activity_at: i.last_activity_at.clone(),
        }
    }

    pub fn state(&self) -> SessionState {
        self.inner
            .lock()
            .map(|i| i.state)
            .unwrap_or(SessionState::Closed)
    }

    /// Usable (not crashed / closed).
    pub fn is_live(&self) -> bool {
        !self.closed.load(Ordering::SeqCst)
            && matches!(self.state(), SessionState::Starting | SessionState::Ready)
    }

    pub fn agent_drives(&self) -> bool {
        self.inner
            .lock()
            .map(|i| i.control.agent_drives())
            .unwrap_or(false)
    }

    pub fn downloads_dir(&self) -> PathBuf {
        self.downloads_dir.clone()
    }

    pub fn context_id(&self) -> Option<String> {
        self.context_id.clone()
    }

    pub fn viewer_count(&self) -> usize {
        self.viewers.lock().map(|v| v.len()).unwrap_or(0)
    }

    /// Idle time — `None` while a viewer is attached.
    pub fn idle_for(&self, now: Instant) -> Option<Duration> {
        if self.viewer_count() > 0 {
            return None;
        }
        self.inner
            .lock()
            .ok()
            .map(|i| now.saturating_duration_since(i.last_activity))
    }

    fn touch(&self) {
        if let Ok(mut i) = self.inner.lock() {
            i.last_activity = Instant::now();
            i.last_activity_at = now_rfc3339();
        }
    }

    fn set_state(&self, s: SessionState) {
        if let Ok(mut i) = self.inner.lock() {
            i.state = s;
        }
        self.broadcast_state();
    }

    // -------------------------------------------------------------- viewers

    fn send_to(&self, viewer_id: u64, msg: ServerMsg) {
        let tx = self
            .viewers
            .lock()
            .ok()
            .and_then(|v| v.get(&viewer_id).map(|v| v.tx.clone()));
        if let Some(tx) = tx {
            let _ = tx.try_send(ViewerOut::Text(msg.to_json()));
        }
    }

    fn broadcast_msg(&self, msg: ServerMsg) {
        let text = msg.to_json();
        let mut dead = Vec::new();
        if let Ok(mut vs) = self.viewers.lock() {
            for (id, v) in vs.iter() {
                if v.tx.try_send(ViewerOut::Text(text.clone())).is_err() {
                    dead.push(*id);
                }
            }
            for id in dead {
                vs.remove(&id); // wedged or gone — its socket task ends
            }
        }
    }

    pub fn broadcast_state(&self) {
        let info = self.info();
        self.broadcast_msg(ServerMsg::State {
            session: Box::new(info),
        });
    }

    pub fn notify_blocked(&self, host: &str) {
        self.broadcast_msg(ServerMsg::Blocked {
            host: host.to_string(),
            reason: "ssrf",
        });
    }

    pub fn notify_popup(&self, url: &str) {
        self.broadcast_msg(ServerMsg::Popup {
            url: url.to_string(),
        });
    }

    pub fn notify_download(&self, status: &'static str, filename: &str, bytes: Option<u64>) {
        self.broadcast_msg(ServerMsg::Download {
            status,
            filename: filename.to_string(),
            bytes,
        });
    }

    /// Attach a viewer (the WS task). `can_drive` = Edit + ws editor.
    pub fn attach(
        self: &Arc<Self>,
        user_id: &str,
        can_drive: bool,
    ) -> Result<ViewerHandle, LiveError> {
        if !self.is_live() {
            return Err(LiveError::NotFound);
        }
        let (tx, rx) = mpsc::channel(VIEWER_QUEUE);
        let id = self.next_viewer.fetch_add(1, Ordering::SeqCst);
        {
            let mut vs = self.viewers.lock().map_err(|_| LiveError::NotFound)?;
            if vs.len() >= MAX_VIEWERS {
                return Err(LiveError::Conflict(format!(
                    "at most {MAX_VIEWERS} viewers per live session"
                )));
            }
            vs.insert(
                id,
                Viewer {
                    user_id: user_id.to_string(),
                    can_drive,
                    warned: false,
                    tx,
                    flow: ViewerFlow::default(),
                },
            );
        }
        self.touch();
        self.broadcast_state();
        // (Re)start: a fresh screencast emits the current frame immediately,
        // so the new viewer isn't blank until the page next repaints.
        self.stop_screencast();
        self.start_screencast();
        Ok(ViewerHandle {
            id,
            rx,
            session: self.clone(),
        })
    }

    fn detach(&self, viewer_id: u64) {
        let (user, remaining_for_user, remaining) = match self.viewers.lock() {
            Ok(mut vs) => {
                let user = vs.remove(&viewer_id).map(|v| v.user_id);
                let remaining_for_user = user
                    .as_ref()
                    .map(|u| vs.values().filter(|v| &v.user_id == u).count())
                    .unwrap_or(0);
                (user, remaining_for_user, vs.len())
            }
            Err(_) => return,
        };
        let Some(user) = user else { return };
        if remaining_for_user == 0 {
            if let Ok(mut i) = self.inner.lock() {
                i.control.human_left(&user);
            }
        }
        if remaining == 0 {
            self.stop_screencast();
        }
        self.touch();
        self.broadcast_state();
    }

    fn set_can_drive(&self, viewer_id: u64, can_drive: bool) {
        if let Ok(mut vs) = self.viewers.lock() {
            if let Some(v) = vs.get_mut(&viewer_id) {
                v.can_drive = can_drive;
            }
        }
    }

    // ------------------------------------------------------------ screencast

    fn start_screencast(&self) {
        let (quality, nth, vp) = match self.inner.lock() {
            Ok(mut i) => {
                i.screencast_on = true;
                let (q, n) = i.adaptive.params();
                (q, n, i.viewport)
            }
            Err(_) => return,
        };
        let scale = vp.device_scale_factor.min(2.0);
        self.proc.conn.send(
            "Page.startScreencast",
            json!({
                "format": "jpeg",
                "quality": quality,
                "maxWidth": (vp.width as f64 * scale).round() as u32,
                "maxHeight": (vp.height as f64 * scale).round() as u32,
                "everyNthFrame": nth,
            }),
            Some(&self.sid),
        );
    }

    fn stop_screencast(&self) {
        if let Ok(mut i) = self.inner.lock() {
            i.screencast_on = false;
            i.pending_cdp_ack = None;
        }
        self.proc
            .conn
            .send("Page.stopScreencast", json!({}), Some(&self.sid));
    }

    fn restart_screencast(&self) {
        let on = self.inner.lock().map(|i| i.screencast_on).unwrap_or(false);
        if on {
            self.stop_screencast();
            self.start_screencast();
        }
    }

    fn ack_chrome(&self, screencast_sid: i64) {
        self.proc.conn.send(
            "Page.screencastFrameAck",
            json!({"sessionId": screencast_sid}),
            Some(&self.sid),
        );
    }

    fn on_frame(&self, params: &Value) {
        let Some(sc_sid) = params["sessionId"].as_i64() else {
            return;
        };
        let Ok(bytes) =
            base64::engine::general_purpose::STANDARD.decode(params["data"].as_str().unwrap_or(""))
        else {
            self.ack_chrome(sc_sid);
            return;
        };
        let md = &params["metadata"];
        let f = |k: &str| md[k].as_f64().unwrap_or(0.0);
        let (w, h) =
            jpeg_size(&bytes).unwrap_or((f("deviceWidth") as u32, f("deviceHeight") as u32));
        let seq = self.frame_seq.fetch_add(1, Ordering::SeqCst) + 1;
        let header = FrameHeader {
            seq,
            mime: "image/jpeg".into(),
            width: w,
            height: h,
            device_width: f("deviceWidth"),
            device_height: f("deviceHeight"),
            page_scale_factor: md["pageScaleFactor"].as_f64().unwrap_or(1.0),
            offset_top: f("offsetTop"),
            scroll_x: f("scrollOffsetX"),
            scroll_y: f("scrollOffsetY"),
            timestamp: f("timestamp"),
        };
        let frame = Arc::new(encode_frame(&header, &bytes));
        let now = Instant::now();
        let mut any_capacity = false;
        let mut none = true;
        if let Ok(mut vs) = self.viewers.lock() {
            let mut dead = Vec::new();
            for (id, v) in vs.iter_mut() {
                none = false;
                if let Some(fr) = v.flow.offer(seq, frame.clone(), now) {
                    if v.tx.try_send(ViewerOut::Frame(fr)).is_err() {
                        dead.push(*id);
                        continue;
                    }
                }
                if v.flow.has_capacity() {
                    any_capacity = true;
                }
            }
            for id in dead {
                vs.remove(&id);
            }
        }
        if none || any_capacity {
            self.ack_chrome(sc_sid);
        } else if let Ok(mut i) = self.inner.lock() {
            // Every viewer is saturated: hold Chrome's ack until one acks us.
            i.pending_cdp_ack = Some(sc_sid);
        }
    }

    fn on_ack(&self, viewer_id: u64, seq: u64) {
        let now = Instant::now();
        let mut sample = None;
        if let Ok(mut vs) = self.viewers.lock() {
            let mut drop_it = false;
            if let Some(v) = vs.get_mut(&viewer_id) {
                if let Some(fr) = v.flow.ack(seq, now) {
                    drop_it = v.tx.try_send(ViewerOut::Frame(fr)).is_err();
                }
                let (sent, dropped) = v.flow.take_counters();
                sample = Some((v.flow.rtt_ms(), sent, dropped));
            }
            if drop_it {
                vs.remove(&viewer_id);
            }
        }
        let (pending, change) = match self.inner.lock() {
            Ok(mut i) => {
                if let Some((rtt, sent, dropped)) = sample {
                    i.adaptive.observe(rtt, sent, dropped);
                }
                (i.pending_cdp_ack.take(), i.adaptive.decide(now))
            }
            Err(_) => (None, None),
        };
        if let Some(sc) = pending {
            self.ack_chrome(sc);
        }
        if change.is_some() {
            self.restart_screencast();
        }
    }

    // ----------------------------------------------------------------- input

    /// One frame from a viewer.
    pub async fn handle_client(self: &Arc<Self>, viewer_id: u64, msg: ClientMsg) {
        if let ClientMsg::Ack { seq } = msg {
            self.on_ack(viewer_id, seq);
            return;
        }
        let who = self.viewers.lock().ok().and_then(|mut vs| {
            vs.get_mut(&viewer_id).map(|v| {
                let first_refusal = !v.can_drive && !v.warned;
                if first_refusal {
                    v.warned = true;
                }
                (v.user_id.clone(), v.can_drive, first_refusal)
            })
        });
        let Some((user, can_drive, first_refusal)) = who else {
            return;
        };
        if !can_drive {
            if first_refusal {
                self.send_to(
                    viewer_id,
                    ServerMsg::error("forbidden", "you can watch this session but not drive it"),
                );
            }
            return;
        }
        self.touch();
        match msg {
            ClientMsg::Control { action } => {
                let changed = match self.inner.lock() {
                    Ok(mut i) => match action {
                        ControlAction::TakeOver => i.control.take_over(&user),
                        ControlAction::HandBack => i.control.hand_back(),
                    },
                    Err(_) => false,
                };
                if changed {
                    self.broadcast_state();
                    let what = match action {
                        ControlAction::TakeOver => "take_over",
                        ControlAction::HandBack => "hand_back",
                    };
                    self.audit(
                        "browser.live.control",
                        Some(user.clone()),
                        json!({ "action": what }),
                    );
                }
            }
            other => {
                let claim = match self.inner.lock() {
                    Ok(mut i) => i.control.human_input(&user),
                    Err(_) => Ok(false),
                };
                match claim {
                    Err(_) => {
                        self.send_to(
                            viewer_id,
                            ServerMsg::error(
                                "not_driver",
                                "the agent is driving — take over first",
                            ),
                        );
                        return;
                    }
                    Ok(true) => self.broadcast_state(),
                    Ok(false) => {}
                }
                if let Err(e) = self.dispatch_input(other).await {
                    let code = match e {
                        LiveError::Blocked(_) | LiveError::Invalid(_) => "nav_failed",
                        _ => "input_failed",
                    };
                    self.send_to(viewer_id, ServerMsg::error(code, e.to_string()));
                }
            }
        }
    }

    /// Dispatch one input/nav/resize/dialog frame (human or agent).
    pub async fn dispatch_input(self: &Arc<Self>, msg: ClientMsg) -> Result<(), LiveError> {
        let conn = &self.proc.conn;
        let sid = Some(self.sid.as_str());
        let call = |cmd: input::Command| async move {
            conn.call_with_timeout(cmd.0, cmd.1, sid, INPUT_TIMEOUT)
                .await
                .map(|_| ())
                .map_err(cdp_live)
        };
        match msg {
            ClientMsg::Ack { .. } | ClientMsg::Control { .. } => Ok(()),
            ClientMsg::Mouse {
                action,
                x,
                y,
                button,
                buttons,
                click_count,
                delta_x,
                delta_y,
                modifiers,
            } => {
                let is_move = action == super::protocol::MouseAction::Move;
                if is_move && !self.move_due() {
                    return Ok(());
                }
                call(input::mouse(
                    action,
                    x,
                    y,
                    button,
                    buttons,
                    click_count,
                    delta_x,
                    delta_y,
                    modifiers,
                ))
                .await?;
                if is_move {
                    self.maybe_probe_cursor(x, y);
                }
                Ok(())
            }
            ClientMsg::Key {
                action,
                key,
                code,
                text,
                key_code,
                location,
                repeat,
                modifiers,
            } => {
                call(input::key(
                    action,
                    &key,
                    &code,
                    text.as_deref(),
                    key_code,
                    location,
                    repeat,
                    modifiers,
                ))
                .await
            }
            ClientMsg::Text { text } | ClientMsg::Paste { text } => {
                call(input::insert_text(&text)).await
            }
            ClientMsg::Ime {
                text,
                selection_start,
                selection_end,
            } => {
                call(input::ime_composition(
                    &text,
                    selection_start,
                    selection_end,
                ))
                .await
            }
            ClientMsg::Nav { action, url } => self.navigate(action, url).await,
            ClientMsg::Resize {
                width,
                height,
                device_scale_factor,
            } => {
                self.resize(Viewport {
                    width,
                    height,
                    device_scale_factor: device_scale_factor.unwrap_or(1.0),
                })
                .await
            }
            ClientMsg::Dialog {
                accept,
                prompt_text,
            } => {
                let mut p = json!({"accept": accept});
                if let Some(t) = prompt_text {
                    p["promptText"] = json!(t.chars().take(10_000).collect::<String>());
                }
                if let Ok(mut i) = self.inner.lock() {
                    i.dialog_seq += 1;
                }
                call(("Page.handleJavaScriptDialog", p)).await
            }
        }
    }

    fn move_due(&self) -> bool {
        let now = Instant::now();
        match self.inner.lock() {
            Ok(mut i) => {
                let due = i
                    .last_move
                    .is_none_or(|t| now.saturating_duration_since(t) >= MOVE_COALESCE);
                if due {
                    i.last_move = Some(now);
                }
                due
            }
            Err(_) => true,
        }
    }

    fn maybe_probe_cursor(self: &Arc<Self>, x: f64, y: f64) {
        let now = Instant::now();
        let due = match self.inner.lock() {
            Ok(mut i) => {
                let due = i
                    .last_probe
                    .is_none_or(|t| now.saturating_duration_since(t) >= CURSOR_PROBE_EVERY);
                if due {
                    i.last_probe = Some(now);
                }
                due
            }
            Err(_) => false,
        };
        if !due {
            return;
        }
        let me = Arc::downgrade(self);
        tokio::spawn(async move {
            let Some(s) = me.upgrade() else { return };
            let r = s
                .proc
                .conn
                .call_with_timeout(
                    "Runtime.evaluate",
                    json!({"expression": input::cursor_probe_expr(x, y), "returnByValue": true}),
                    Some(&s.sid),
                    Duration::from_secs(1),
                )
                .await;
            let Ok(v) = r else { return };
            let css = v
                .pointer("/result/value")
                .and_then(Value::as_str)
                .unwrap_or("default");
            let cursor = input::normalize_cursor(css);
            let changed = match s.inner.lock() {
                Ok(mut i) if i.cursor != cursor => {
                    i.cursor = cursor.to_string();
                    true
                }
                _ => false,
            };
            if changed {
                s.broadcast_msg(ServerMsg::Cursor {
                    cursor: cursor.to_string(),
                });
            }
        });
    }

    // ------------------------------------------------------------ navigation

    pub async fn navigate(&self, action: NavAction, url: Option<String>) -> Result<(), LiveError> {
        let conn = &self.proc.conn;
        let sid = Some(self.sid.as_str());
        match action {
            NavAction::Goto => {
                let url =
                    url.ok_or_else(|| LiveError::Invalid("url is required for goto".into()))?;
                let url = validate_nav_url(&url).await?;
                let r = conn
                    .call("Page.navigate", json!({"url": url}), sid)
                    .await
                    .map_err(cdp_live)?;
                if let Some(err) = r["errorText"].as_str().filter(|s| !s.is_empty()) {
                    if err.contains("ERR_BLOCKED_BY_CLIENT") {
                        return Err(LiveError::Blocked(guard::host_of(&url)));
                    }
                    return Err(LiveError::Engine(format!("navigation failed: {err}")));
                }
                Ok(())
            }
            NavAction::Back | NavAction::Forward => {
                let h = conn
                    .call("Page.getNavigationHistory", json!({}), sid)
                    .await
                    .map_err(cdp_live)?;
                let idx = h["currentIndex"].as_i64().unwrap_or(0);
                let entries = h["entries"].as_array().cloned().unwrap_or_default();
                let target = if action == NavAction::Back {
                    idx - 1
                } else {
                    idx + 1
                };
                if target < 0 || target as usize >= entries.len() {
                    return Ok(()); // nothing there — a no-op, like the browser
                }
                let entry_id = entries[target as usize]["id"].as_i64().unwrap_or(0);
                conn.call(
                    "Page.navigateToHistoryEntry",
                    json!({"entryId": entry_id}),
                    sid,
                )
                .await
                .map(|_| ())
                .map_err(cdp_live)
            }
            NavAction::Reload => conn
                .call("Page.reload", json!({}), sid)
                .await
                .map(|_| ())
                .map_err(cdp_live),
            NavAction::Stop => conn
                .call("Page.stopLoading", json!({}), sid)
                .await
                .map(|_| ())
                .map_err(cdp_live),
        }
    }

    pub async fn resize(&self, vp: Viewport) -> Result<(), LiveError> {
        let vp = vp.clamped();
        self.proc
            .conn
            .call(
                "Emulation.setDeviceMetricsOverride",
                metrics_params(vp),
                Some(&self.sid),
            )
            .await
            .map_err(cdp_live)?;
        if let Ok(mut i) = self.inner.lock() {
            i.viewport = vp;
        }
        self.restart_screencast();
        self.broadcast_state();
        Ok(())
    }

    async fn refresh_history(&self) {
        let Ok(h) = self
            .proc
            .conn
            .call("Page.getNavigationHistory", json!({}), Some(&self.sid))
            .await
        else {
            return;
        };
        let idx = h["currentIndex"].as_i64().unwrap_or(0);
        let len = h["entries"].as_array().map(|e| e.len()).unwrap_or(0) as i64;
        let (back, fwd) = (idx > 0, idx + 1 < len);
        let changed = match self.inner.lock() {
            Ok(mut i) if i.can_go_back != back || i.can_go_forward != fwd => {
                i.can_go_back = back;
                i.can_go_forward = fwd;
                true
            }
            _ => false,
        };
        if changed {
            self.broadcast_state();
        }
    }

    // ------------------------------------------------------------ screenshots

    pub async fn screenshot(&self, req: &ScreenshotRequest) -> Result<Screenshot, LiveError> {
        let conn = &self.proc.conn;
        let sid = Some(self.sid.as_str());
        let clip = match req.mode {
            ScreenshotMode::Viewport => None,
            ScreenshotMode::FullPage => {
                let m = conn
                    .call("Page.getLayoutMetrics", json!({}), sid)
                    .await
                    .map_err(cdp_live)?;
                let size = if m["cssContentSize"].is_object() {
                    &m["cssContentSize"]
                } else {
                    &m["contentSize"]
                };
                Some(Clip {
                    x: 0.0,
                    y: 0.0,
                    width: size["width"].as_f64().unwrap_or(1280.0),
                    height: size["height"].as_f64().unwrap_or(800.0),
                })
            }
            ScreenshotMode::Element => {
                let sel = req
                    .selector
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| {
                        LiveError::Invalid("selector is required for an element screenshot".into())
                    })?;
                if sel.len() > 1000 {
                    return Err(LiveError::Invalid("selector too long".into()));
                }
                let r = conn
                    .call(
                        "Runtime.evaluate",
                        json!({"expression": element_box_expr(sel), "returnByValue": true}),
                        sid,
                    )
                    .await
                    .map_err(cdp_live)?;
                let b = r.pointer("/result/value").cloned().unwrap_or(Value::Null);
                if b.is_null() {
                    return Err(LiveError::NotFound);
                }
                Some(Clip {
                    x: b["x"].as_f64().unwrap_or(0.0),
                    y: b["y"].as_f64().unwrap_or(0.0),
                    width: b["w"].as_f64().unwrap_or(1.0),
                    height: b["h"].as_f64().unwrap_or(1.0),
                })
            }
        };
        let r = conn
            .call_with_timeout(
                "Page.captureScreenshot",
                screenshot_params(req.format, req.quality, clip),
                sid,
                Duration::from_secs(60),
            )
            .await
            .map_err(cdp_live)?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(r["data"].as_str().unwrap_or(""))
            .map_err(|e| LiveError::Engine(format!("bad screenshot data: {e}")))?;
        let url = self.inner.lock().map(|i| i.url.clone()).unwrap_or_default();
        Ok(Screenshot {
            bytes,
            mime: req.format.mime(),
            url,
        })
    }

    // ------------------------------------------------------------------ agent

    /// The agent wants to drive. Refused (paused) while a human drives.
    pub fn agent_acquire(&self) -> Result<(), LiveError> {
        let r = match self.inner.lock() {
            Ok(mut i) => i.control.agent_acquire(),
            Err(_) => return Err(LiveError::NotFound),
        };
        match r {
            Ok(changed) => {
                if changed {
                    self.broadcast_state();
                }
                Ok(())
            }
            Err(ControlError::AgentPaused) | Err(ControlError::NotDriver) => {
                Err(LiveError::AgentPaused)
            }
        }
    }

    pub fn agent_release(&self) {
        let changed = self
            .inner
            .lock()
            .map(|mut i| i.control.agent_release())
            .unwrap_or(false);
        if changed {
            self.broadcast_state();
        }
    }

    /// Agent input — only while the agent holds control.
    pub async fn agent_input(self: &Arc<Self>, msg: ClientMsg) -> Result<(), LiveError> {
        if !self.agent_drives() {
            return Err(LiveError::AgentPaused);
        }
        self.touch();
        self.dispatch_input(msg).await
    }

    pub fn controller(&self) -> ControllerKind {
        self.inner
            .lock()
            .map(|i| i.control.kind())
            .unwrap_or(ControllerKind::None)
    }

    /// REST take-over / hand-back (same rules as the WS frame).
    pub fn control(&self, user_id: &str, action: ControlAction) -> bool {
        let changed = match self.inner.lock() {
            Ok(mut i) => match action {
                ControlAction::TakeOver => i.control.take_over(user_id),
                ControlAction::HandBack => i.control.hand_back(),
            },
            Err(_) => false,
        };
        if changed {
            self.touch();
            self.broadcast_state();
            let what = match action {
                ControlAction::TakeOver => "take_over",
                ControlAction::HandBack => "hand_back",
            };
            self.audit(
                "browser.live.control",
                Some(user_id.to_string()),
                json!({ "action": what }),
            );
        }
        changed
    }

    /// An agent's outward document request is paused at the Fetch stage:
    /// screenshot the page as it is, file an approval, and continue the
    /// request only when a human approves it.
    pub async fn hold_outward(self: &Arc<Self>, req: Paused, via: Option<String>) {
        let conn = self.proc.conn.clone();
        let shot = self
            .screenshot(&ScreenshotRequest {
                mode: ScreenshotMode::Viewport,
                selector: None,
                format: ImageFormat::Jpeg,
                quality: Some(70),
            })
            .await
            .ok();
        let screenshot_path = shot.and_then(|s| self.store_approval_shot(&s.bytes));
        let (page_url, page_title) = self
            .inner
            .lock()
            .map(|i| (i.url.clone(), i.title.clone()))
            .unwrap_or_default();
        let action = OutwardAction {
            workspace_id: self.workspace_id.clone(),
            tab_id: self.tab_id.clone(),
            owner_id: self.owner_id.clone(),
            page_origin: guard::display_origin(&page_url),
            page_title,
            method: req.method.clone(),
            target_host: guard::host_of(&req.url),
            screenshot_path,
        };
        let title = action.title();
        let approved = match self.hooks.request_approval(&action).await {
            Ok(id) => {
                self.broadcast_msg(ServerMsg::Approval {
                    approval_id: id.clone(),
                    status: "pending",
                    title: title.clone(),
                });
                let decided = self
                    .hooks
                    .await_approval(&id, OUTWARD_APPROVAL_TIMEOUT)
                    .await;
                let ok = decided == Some(true) && self.is_live();
                self.broadcast_msg(ServerMsg::Approval {
                    approval_id: id,
                    status: if ok { "approved" } else { "denied" },
                    title,
                });
                ok
            }
            Err(e) => {
                tracing::warn!("browser live: could not file an approval: {e}");
                self.broadcast_msg(ServerMsg::error(
                    "input_failed",
                    "an outward action needs approval, but the approval queue is unavailable",
                ));
                false
            }
        };
        if approved {
            conn.send(
                "Fetch.continueRequest",
                json!({"requestId": req.request_id}),
                via.as_deref(),
            );
        } else {
            conn.send(
                "Fetch.failRequest",
                json!({"requestId": req.request_id, "errorReason": "BlockedByClient"}),
                via.as_deref(),
            );
        }
        self.audit(
            "browser.live.outward",
            None,
            json!({
                "method": action.method,
                "host": action.target_host,
                "approved": approved,
                "driver": "agent",
            }),
        );
    }

    fn store_approval_shot(&self, bytes: &[u8]) -> Option<String> {
        let dir = self.data_dir.join("browser").join("approvals");
        std::fs::create_dir_all(&dir).ok()?;
        super::chrome::restrict_dir(&dir);
        let path = dir.join(format!(
            "{}-{}.jpg",
            safe_segment(&self.tab_id),
            chrono::Utc::now().timestamp_millis()
        ));
        std::fs::write(&path, bytes).ok()?;
        Some(path.display().to_string())
    }

    // ---------------------------------------------------------------- events

    fn audit(&self, action: &'static str, user_id: Option<String>, mut detail: Value) {
        detail["workspace_id"] = json!(self.workspace_id);
        detail["profile"] = json!(self.profile);
        let entry = LiveAudit {
            action,
            user_id: user_id.or_else(|| Some(self.owner_id.clone())),
            target: self.tab_id.clone(),
            detail,
        };
        let hooks = self.hooks.clone();
        tokio::spawn(async move { hooks.audit(entry).await });
    }

    pub fn on_target_info(&self, title: &str, url: &str) {
        let changed = match self.inner.lock() {
            Ok(mut i) if i.title != title || (!url.is_empty() && i.url != url) => {
                i.title = title.chars().take(500).collect();
                if !url.is_empty() {
                    i.url = url.to_string();
                }
                true
            }
            _ => false,
        };
        if changed {
            self.broadcast_state();
        }
    }

    /// The page target died (`crashed`) or was closed by the page itself.
    pub fn on_target_gone(&self, crashed: bool) {
        let first = match self.inner.lock() {
            Ok(mut i) if matches!(i.state, SessionState::Starting | SessionState::Ready) => {
                i.state = if crashed {
                    SessionState::Crashed
                } else {
                    SessionState::Closed
                };
                true
            }
            _ => false,
        };
        if !first {
            return;
        }
        self.broadcast_state();
        self.broadcast_msg(ServerMsg::Closed {
            reason: if crashed { "crashed" } else { "closed" },
        });
        if let Ok(mut vs) = self.viewers.lock() {
            vs.clear();
        }
        self.hooks.session_changed(&self.info());
    }

    async fn on_event(self: &Arc<Self>, ev: CdpEvent) {
        let p = &ev.params;
        match ev.method.as_str() {
            "Page.screencastFrame" => self.on_frame(p),
            "Page.frameNavigated" => {
                let frame = &p["frame"];
                if frame.get("parentId").and_then(Value::as_str).is_some() {
                    return;
                }
                let url = frame["url"].as_str().unwrap_or("").to_string();
                let driver = match self.controller() {
                    ControllerKind::Agent => "agent",
                    ControllerKind::Human => "human",
                    ControllerKind::None => "none",
                };
                let user = self
                    .inner
                    .lock()
                    .ok()
                    .and_then(|i| i.control.human_user().map(str::to_string));
                if let Ok(mut i) = self.inner.lock() {
                    i.url = url.clone();
                }
                self.broadcast_state();
                if url.starts_with("http://") || url.starts_with("https://") {
                    self.audit(
                        "browser.live.navigate",
                        user,
                        json!({"host": guard::host_of(&url), "driver": driver}),
                    );
                }
                // Off the event loop: frames and paused requests keep flowing.
                let me = self.clone();
                tokio::spawn(async move { me.refresh_history().await });
            }
            "Page.navigatedWithinDocument" => {
                if p["frameId"].as_str() != Some(self.target_id.as_str()) {
                    return;
                }
                if let Some(url) = p["url"].as_str() {
                    if let Ok(mut i) = self.inner.lock() {
                        i.url = url.to_string();
                    }
                }
                self.broadcast_state();
                // Off the event loop: frames and paused requests keep flowing.
                let me = self.clone();
                tokio::spawn(async move { me.refresh_history().await });
            }
            "Page.frameStartedLoading" | "Page.frameStoppedLoading" => {
                if p["frameId"].as_str() != Some(self.target_id.as_str()) {
                    return;
                }
                let loading = ev.method == "Page.frameStartedLoading";
                let changed = match self.inner.lock() {
                    Ok(mut i) if i.loading != loading => {
                        i.loading = loading;
                        true
                    }
                    _ => false,
                };
                if changed {
                    self.broadcast_state();
                }
            }
            "Page.javascriptDialogOpening" => {
                let seq = match self.inner.lock() {
                    Ok(mut i) => {
                        i.dialog_seq += 1;
                        i.dialog_seq
                    }
                    Err(_) => 0,
                };
                self.broadcast_msg(ServerMsg::Dialog {
                    dialog_type: p["type"].as_str().unwrap_or("alert").to_string(),
                    message: p["message"]
                        .as_str()
                        .unwrap_or("")
                        .chars()
                        .take(4000)
                        .collect(),
                    default_prompt: p["defaultPrompt"].as_str().unwrap_or("").to_string(),
                    url: p["url"].as_str().unwrap_or("").to_string(),
                });
                let me = Arc::downgrade(self);
                tokio::spawn(async move {
                    tokio::time::sleep(DIALOG_AUTO_DISMISS).await;
                    let Some(s) = me.upgrade() else { return };
                    let still = s.inner.lock().map(|i| i.dialog_seq == seq).unwrap_or(false);
                    if still {
                        let _ = s
                            .proc
                            .conn
                            .call(
                                "Page.handleJavaScriptDialog",
                                json!({"accept": false}),
                                Some(&s.sid),
                            )
                            .await;
                    }
                });
            }
            "Page.javascriptDialogClosed" => {
                if let Ok(mut i) = self.inner.lock() {
                    i.dialog_seq += 1;
                }
            }
            "Page.fileChooserOpened" => {
                self.broadcast_msg(ServerMsg::error(
                    "input_failed",
                    "file uploads aren't supported in the remote live view yet",
                ));
            }
            "Inspector.targetCrashed" => self.on_target_gone(true),
            "Fetch.requestPaused" => {
                let proc = self.proc.clone();
                let params = ev.params.clone();
                let via = Some(self.sid.clone());
                tokio::spawn(async move { proc.on_paused(params, via).await });
            }
            _ => {}
        }
    }

    /// Tear down: tell viewers, close the target, dispose an ephemeral
    /// context (its cookies go with it), unregister. Idempotent.
    pub async fn close(&self, reason: &'static str) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        if let Ok(mut i) = self.inner.lock() {
            if i.state != SessionState::Crashed {
                i.state = SessionState::Closed;
            }
        }
        self.broadcast_msg(ServerMsg::Closed { reason });
        if let Ok(mut vs) = self.viewers.lock() {
            vs.clear();
        }
        let conn = &self.proc.conn;
        if !conn.is_closed() {
            conn.send("Page.stopScreencast", json!({}), Some(&self.sid));
            let _ = conn
                .call_with_timeout(
                    "Target.closeTarget",
                    json!({"targetId": self.target_id}),
                    None,
                    Duration::from_secs(5),
                )
                .await;
            if let Some(ctx) = &self.context_id {
                let _ = conn
                    .call_with_timeout(
                        "Target.disposeBrowserContext",
                        json!({"browserContextId": ctx}),
                        None,
                        Duration::from_secs(5),
                    )
                    .await;
            }
        }
        conn.unroute(&self.sid);
        self.proc.unregister(&self.target_id);
        if let Ok(mut t) = self.event_task.lock() {
            if let Some(h) = t.take() {
                h.abort();
            }
        }
        self.hooks.session_changed(&self.info());
    }
}

async fn session_loop(me: Weak<LiveSession>, mut rx: mpsc::UnboundedReceiver<CdpEvent>) {
    while let Some(ev) = rx.recv().await {
        let Some(s) = me.upgrade() else { return };
        s.on_event(ev).await;
    }
}

/// `Emulation.setDeviceMetricsOverride` params.
pub fn metrics_params(vp: Viewport) -> Value {
    json!({
        "width": vp.width,
        "height": vp.height,
        "deviceScaleFactor": vp.device_scale_factor,
        "mobile": false,
    })
}

/// A navigation target must be http(s) and pass the SSRF pre-check (the
/// Fetch pump + guard proxy re-vet every hop afterwards).
pub async fn validate_nav_url(raw: &str) -> Result<String, LiveError> {
    let trimmed = raw.trim();
    let parsed =
        reqwest::Url::parse(trimmed).map_err(|_| LiveError::Invalid("not a valid URL".into()))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(LiveError::Invalid("only http(s) URLs can be opened".into()));
    }
    otto_netguard::check_url(trimmed)
        .await
        .map_err(|_| LiveError::Blocked(guard::host_of(trimmed)))?;
    Ok(parsed.to_string())
}

/// A viewer's end: receive frames/messages, forward client frames.
/// Dropping it detaches the viewer.
pub struct ViewerHandle {
    pub id: u64,
    rx: mpsc::Receiver<ViewerOut>,
    session: Arc<LiveSession>,
}

impl ViewerHandle {
    pub async fn recv(&mut self) -> Option<ViewerOut> {
        self.rx.recv().await
    }

    pub async fn handle(&self, msg: ClientMsg) {
        self.session.handle_client(self.id, msg).await;
    }

    /// Narrow (or restore) the drive right after a periodic re-check.
    pub fn set_can_drive(&self, can_drive: bool) {
        self.session.set_can_drive(self.id, can_drive);
    }

    pub fn session(&self) -> &Arc<LiveSession> {
        &self.session
    }
}

impl Drop for ViewerHandle {
    fn drop(&mut self) {
        self.session.detach(self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screenshot_params_per_mode() {
        let p = screenshot_params(ImageFormat::Png, Some(10), None);
        assert_eq!(p["format"], "png");
        assert!(p.get("quality").is_none());
        assert!(p.get("clip").is_none());
        let p = screenshot_params(
            ImageFormat::Jpeg,
            Some(250),
            Some(Clip {
                x: -3.0,
                y: 10.0,
                width: 1280.0,
                height: 99_999.0,
            }),
        );
        assert_eq!(p["quality"], 100);
        assert_eq!(p["clip"]["x"], 0.0);
        assert_eq!(p["clip"]["height"], MAX_CAPTURE_PX);
        assert_eq!(p["captureBeyondViewport"], true);
    }

    #[test]
    fn element_box_expression_escapes_the_selector() {
        let js = element_box_expr("a[href=\"x\"]'); alert(1); ('");
        assert!(js.contains(r#"document.querySelector("a[href=\"x\"]'); alert(1); ('")"#));
    }

    #[test]
    fn screenshot_request_defaults() {
        let r: ScreenshotRequest = serde_json::from_str("{}").unwrap();
        assert_eq!(r.mode, ScreenshotMode::Viewport);
        assert_eq!(r.format, ImageFormat::Png);
        let r: ScreenshotRequest =
            serde_json::from_str(r#"{"mode":"full_page","format":"jpeg","quality":50}"#).unwrap();
        assert_eq!(r.mode, ScreenshotMode::FullPage);
        assert_eq!(r.format.mime(), "image/jpeg");
    }

    #[tokio::test]
    async fn navigation_targets_are_vetted() {
        assert!(matches!(
            validate_nav_url("file:///etc/passwd").await,
            Err(LiveError::Invalid(_))
        ));
        assert!(matches!(
            validate_nav_url("javascript:alert(1)").await,
            Err(LiveError::Invalid(_))
        ));
        assert!(matches!(
            validate_nav_url("http://127.0.0.1:7700/api/v1/sessions").await,
            Err(LiveError::Blocked(_))
        ));
        assert!(matches!(
            validate_nav_url("http://169.254.169.254/").await,
            Err(LiveError::Blocked(_))
        ));
        assert!(validate_nav_url("https://8.8.8.8/").await.is_ok());
    }

    #[test]
    fn metrics_follow_the_viewport() {
        let p = metrics_params(Viewport {
            width: 800,
            height: 600,
            device_scale_factor: 2.0,
        });
        assert_eq!(p["width"], 800);
        assert_eq!(p["deviceScaleFactor"], 2.0);
        assert_eq!(p["mobile"], false);
    }
}
