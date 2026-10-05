//! Terminal WebSocket — `GET /ws/term/{session_id}` per docs/contracts/ws.md.
//!
//! Auth: `?token=` validated BEFORE the upgrade via a `route_layer` middleware,
//! so the 403 path is exercisable in tests even without a real WS connection.
//! Owners (session creator), workspace Admins, and root may attach. Editors and
//! Viewers who are not the owner are rejected (#L9).  Input/resize capability
//! is determined post-auth by whether the caller holds at least Editor role in
//! the session's workspace.
//!
//! A **scoped (share-link) token** is a separate path (mobile plan Task 1.6): it
//! bypasses the owner-or-admin gate (the scope IS the authority) but may attach
//! ONLY to its one pinned session — the path `session_id` must equal the scope's
//! `session_id` (else 403, no upgrade) — and its write capability is the share's
//! capped role (`Editor` may input/resize; a `Viewer` share is read-only).

use std::sync::{Arc, LazyLock};
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use bytes::Bytes;
use otto_core::api::Problem;
use otto_core::auth::{session_owner_or_admin, AuthUser, TokenAuthenticator};
use otto_core::domain::WorkspaceRole;
use otto_core::{Error, Id};
use otto_pty::PtyHandle;
use serde::Deserialize;
use tokio::sync::{broadcast, watch, Semaphore};

use crate::share_throttle;

use crate::http::SessionsCtx;

/// Interval for server-initiated pings (keeps idle sockets alive).
const PING_INTERVAL: Duration = Duration::from_secs(30);

/// Cadence of the off-loop re-authorization pass for a PLAIN session.
///
/// The pass is a SQLite round-trip plus an auth lookup. It used to run on this
/// socket's `select!` loop (and, worse, ahead of every client frame), so a
/// congested pool was felt as 1–2 s of typing lag with no CPU to show for it.
/// It now lives in [`reauth_loop`] and only publishes its verdict here.
const REAUTH_INTERVAL: Duration = Duration::from_secs(5);

/// Cadence for a RESOURCE-BOUND terminal (k8s / AWS / a connection). A revoked
/// grant must drop those sockets promptly, so they keep the original 1 s beat.
/// Which one applies is decided ONCE at attach (the binding lives in the
/// session row's `meta`/`connection_id` and never changes mid-connection).
const REAUTH_INTERVAL_RESOURCE: Duration = Duration::from_secs(1);

/// A re-check slower than this means the state pool is congested — exactly the
/// condition that used to surface as a frozen terminal.
const REAUTH_SLOW: Duration = Duration::from_millis(100);

/// A PTY write slower than this means the child stopped draining its tty (the
/// write runs on the PTY's writer thread; this task only awaits its delivery).
const INPUT_SLOW: Duration = Duration::from_millis(20);

/// How often a viewer whose process is gone looks for a respawned one (a chat
/// send, a channel follow-up, a workflow step or a restart from another client
/// can bring the session back while this tab stays open). Only armed while
/// the viewer has no live process; the check is one in-memory map lookup.
const REVIVE_POLL: Duration = Duration::from_secs(1);

#[derive(Clone)]
struct WsState<S> {
    auth: Arc<dyn TokenAuthenticator>,
    ctx: S,
}

#[derive(Deserialize)]
struct TokenQuery {
    token: Option<String>,
}

fn user_input_default() -> bool {
    true
}

/// Attach intent, from the upgrade URL (`/ws/term/{id}?view=1`).
///
/// A VIEW-ONLY attach (a grid tile, an embedded agent-output viewer, a pane
/// reconnecting on its own after a dropped socket) must not spawn the
/// session's CLI: `ensure_live` on every attach brought back `claude --resume`
/// for each session merely looked at (r3-05-01; 150–400 MB each). Such a
/// socket resumes a dormant session only on the user's first real keystroke
/// (see [`wakes_on_input`]); an explicit Resume (the restart route, or a
/// non-view re-attach) is unchanged. Unknown/absent → the classic attach.
#[derive(Deserialize, Default)]
struct AttachQuery {
    #[serde(default)]
    view: Option<String>,
}

impl AttachQuery {
    fn view_only(&self) -> bool {
        matches!(self.view.as_deref(), Some("1" | "true"))
    }
}

/// Does this attach resume an exited-but-resumable session? Read-only viewers
/// (shares) never do; view-only attaches wait for real input.
fn resume_on_attach(can_input: bool, view_only: bool) -> bool {
    can_input && !view_only
}

/// Should an `input` frame wake the session first? Only on a view-only socket,
/// only for a real keystroke (`user`: emulator DA/CPR replies never wake a
/// CLI), and only while no process is live behind this viewer.
fn wakes_on_input(view_only: bool, user: bool, live: bool) -> bool {
    view_only && user && !live
}

/// Client → server control frames.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientFrame {
    Input {
        data: String,
        #[serde(default = "user_input_default")]
        user: bool,
    },
    Resize {
        cols: u16,
        rows: u16,
    },
    // Request a history-inclusive snapshot: up to `lines` rows of scrollback
    // history (rows that scrolled off above the visible screen) followed by a
    // coherent current-screen frame. Honors the requested `lines`. The client
    // treats every snapshot as a full rebuild (reset + repaint), so this must
    // always carry the complete retained history.
    //
    // Optional `cols`/`rows` (perf 01 F1): the client's measured grid. When
    // this viewer may resize, the PTY + emulator are resized to it BEFORE the
    // capture, so the snapshot is already at the client's width (no second
    // "compact" snapshot after the resize confirms) and the TUI's SIGWINCH
    // repaint arrives as ordinary live bytes after it.
    Scrollback {
        lines: usize,
        #[serde(default)]
        cols: Option<u16>,
        #[serde(default)]
        rows: Option<u16>,
    },
    // Server-side search: grep the ring-buffer scrollback for `query` (plain
    // substring, case-insensitive). The server replies with a JSON
    // `{"type":"search_result","query":"…","matches":[…]}` frame containing
    // up to `MAX_SEARCH_RESULTS` matching line objects. This keeps results
    // across WS reconnects (the ring survives), unlike the xterm SearchAddon
    // which only searches the emulator's current viewport.
    Search {
        query: String,
    },
    // Claim size authority without typing — sent when the terminal gains
    // FOCUS, so clicking into a pane reclaims the PTY size from a stale
    // viewer (e.g. a phone tab that typed once and stayed attached).
    Claim,
    // Flow control (SA-02). The client's renderer is behind: its pending
    // (received-but-unparsed) bytes crossed the high watermark. Stop
    // forwarding output to THIS viewer — the PTY, the emulator and every other
    // viewer carry on — until `resume` or [`FLOW_AUTO_RESUME`]. A repeated
    // `pause` while paused re-arms the auto-resume deadline (keep-alive).
    Pause,
    // The client drained below its low watermark. If output was held back
    // meanwhile, the server discards it and pushes ONE `scrollback` snapshot
    // (the lagging-viewer resync) instead of the megabytes it skipped.
    Resume,
    // The user typed (typically ^C) while the client still had a large
    // backlog queued in front of its emulator: the client DROPPED that queue
    // and needs the current screen. Unlike `scrollback`, the server first
    // discards this viewer's own queued chunks (already reflected in the
    // snapshot — forwarding them after it would double-apply them), then
    // sends ONE snapshot of up to `lines` rows, and leaves any flow-control
    // pause (the dropped backlog was the reason for it).
    Resync {
        #[serde(default)]
        lines: usize,
        #[serde(default)]
        cols: Option<u16>,
        #[serde(default)]
        rows: Option<u16>,
    },
    // Credit-based flow control (supersedes `pause`/`resume` for clients that
    // opt in). Sent once, first thing on a new socket: from the server's
    // `{"type":"credit","window":W}` reply on, it sends at most W binary bytes
    // this viewer has not acknowledged. See [`CreditGate`].
    Credit {
        #[serde(default)]
        window: u64,
        /// The client takes snapshots as a header + ONE binary frame (perf
        /// 01 N3, [`Snap`]). Absent (older clients) = base64-in-JSON.
        #[serde(default)]
        binary_snapshots: bool,
    },
    // Cumulative binary bytes (since the `credit` reply) the client's emulator
    // has parsed OR the client dropped. Sent every ~64 KB consumed.
    Ack {
        bytes: u64,
    },
    // Latency probe (terminal latency HUD, review 01 L1). Answered right away
    // by this loop with `probe_ack` echoing `id`, so the client's round trip
    // is websocket + daemon loop without the child; the ack also carries the
    // PTY's keystroke-echo statistics (see `otto_pty::EchoStats`). Read-only
    // safe: it touches neither the PTY nor any shared state.
    Probe {
        #[serde(default)]
        id: u64,
    },
}

/// A paused viewer that never sends `resume` (renderer wedged, buggy client)
/// is resumed after this long without a fresh `pause`, so a flow-control bug
/// can at worst reproduce the pre-flow-control flood — never a frozen pane.
const FLOW_AUTO_RESUME: Duration = Duration::from_secs(2);

/// Per-connection output gate for client-driven flow control (SA-02).
///
/// A browser WebSocket drains the socket eagerly into the JS task queue, so
/// the daemon never feels TCP backpressure from a slow RENDERER: xterm parses
/// 5.5–8 MB/s in WebKit while `cat`/`yes` produce far more, and the backlog
/// (up to xterm's 50 MB discard watermark) kept the screen scrolling for
/// seconds after `^C`. While paused the output arm of the socket loop is
/// disabled — this viewer's broadcast receiver just stops reading (the ring is
/// bounded; the sender never blocks) — and on resume the held-back output is
/// replaced by one emulator snapshot.
#[derive(Debug, Default)]
struct FlowGate {
    /// `Some(deadline)` while paused; the deadline is the auto-resume instant.
    paused_until: Option<tokio::time::Instant>,
}

impl FlowGate {
    fn pause(&mut self, now: tokio::time::Instant) {
        self.paused_until = Some(now + FLOW_AUTO_RESUME);
    }

    fn is_paused(&self) -> bool {
        self.paused_until.is_some()
    }

    /// Leave the paused state. `true` when the gate WAS paused (the caller
    /// then checks whether anything was held back and resyncs).
    fn resume(&mut self) -> bool {
        self.paused_until.take().is_some()
    }
}

/// Sleep until the gate's auto-resume deadline; pending forever when open.
async fn flow_deadline(paused_until: Option<tokio::time::Instant>) {
    match paused_until {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

/// Discard every chunk queued for this viewer. `true` when anything was
/// discarded — including a lag notice (chunks the ring already overwrote).
fn drain_backlog(rx: &mut broadcast::Receiver<Bytes>) -> bool {
    let mut any = false;
    // Ok / Lagged keep draining; Empty / Closed stop.
    while matches!(
        rx.try_recv(),
        Ok(_) | Err(broadcast::error::TryRecvError::Lagged(_))
    ) {
        any = true;
    }
    any
}

/// The `{"type":"scrollback","data":…,"epoch":…}` snapshot frame.
fn scrollback_frame(data: &[u8], epoch: u64) -> String {
    format!(
        r#"{{"type":"scrollback","data":"{}","epoch":{}}}"#,
        B64.encode(data),
        epoch
    )
}

/// A built `scrollback` reply (perf 01 N3). A client that offered
/// `binary_snapshots` on its `credit` frame gets the bytes as ONE binary WS
/// frame behind a small JSON header — no +33 % base64, no `format!` copy, and
/// no multi-MB `JSON.parse` on the client's main thread. Everyone else (and an
/// empty snapshot) gets the original base64-in-JSON frame.
enum Snap {
    Json(String),
    Binary { data: Bytes, epoch: u64 },
}

impl Snap {
    /// Encode `data` for this connection. Runs inside [`off_worker`] so the
    /// base64 pass of the JSON form stays off the async workers.
    fn build(data: Vec<u8>, epoch: u64, binary: bool) -> Self {
        if binary && !data.is_empty() {
            Snap::Binary {
                data: Bytes::from(data),
                epoch,
            }
        } else {
            Snap::Json(scrollback_frame(&data, epoch))
        }
    }

    /// The binary form's header: `{"type":"scrollback","epoch":E,"binary":true,"len":L}`.
    /// The very next frame on the socket is the binary payload of `len` bytes;
    /// the client must not count it against the credit window (it never went
    /// through the [`CreditGate`]).
    fn header(len: usize, epoch: u64) -> String {
        format!(r#"{{"type":"scrollback","epoch":{epoch},"binary":true,"len":{len}}}"#)
    }

    /// Send it. Header and payload go out back to back from the socket's own
    /// loop, so no live output can land between them. `Err` = socket gone.
    async fn send(self, socket: &mut WebSocket) -> std::result::Result<(), ()> {
        match self {
            Snap::Json(frame) => socket.send(Message::Text(frame.into())).await,
            Snap::Binary { data, epoch } => {
                let header = Self::header(data.len(), epoch);
                if socket.send(Message::Text(header.into())).await.is_err() {
                    return Err(());
                }
                socket.send(Message::Binary(data)).await
            }
        }
        .map_err(|_| ())
    }

    /// The JSON form (tests that inspect a reply built without the capability).
    #[cfg(test)]
    fn json(&self) -> &str {
        match self {
            Snap::Json(frame) => frame,
            Snap::Binary { .. } => panic!("binary snapshot"),
        }
    }
}

/// Snapshot builds allowed at once (r3-06-02). Each copies the emulator
/// state (up to ~25 MB for 4000 × 200 cells) and formats + base64-encodes it
/// on the blocking pool; a tiled overview attaching 15 terminals at once must
/// not turn that into 15 concurrent copies.
const SNAPSHOT_CONCURRENCY: usize = 4;
static SNAPSHOT_PERMITS: LazyLock<Semaphore> =
    LazyLock::new(|| Semaphore::new(SNAPSHOT_CONCURRENCY));

/// Run snapshot work on the blocking pool, bounded by [`SNAPSHOT_PERMITS`].
/// Formatting walks every retained cell (4000 rows × cols) and the frame is
/// base64 of 1–2 MB: tens of ms of CPU that used to run on the socket's async
/// worker — and with the emulator lock held, stalling that session's PTY
/// reader too. `None` only if the job panicked.
async fn off_worker<T: Send + 'static>(job: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let _permit = SNAPSHOT_PERMITS.acquire().await.ok();
    tokio::task::spawn_blocking(job).await.ok()
}

/// A `(snapshot bytes, epoch, subscription)` source for [`resync_frame`]: the
/// subscription must start exactly where the snapshot ends.
type Capture = (Vec<u8>, u64, broadcast::Receiver<Bytes>);

/// [`resync_frame`]'s capture for a live PTY: emulator copy + new
/// subscription under one lock hold, formatted after it is released.
fn pty_capture(h: &Arc<PtyHandle>, lines: usize) -> impl FnOnce() -> Capture + Send + 'static {
    let h = Arc::clone(h);
    move || {
        let (screen, output) = h.capture_and_subscribe();
        (screen.format(lines), h.spawn_seq(), output)
    }
}

/// A `scrollback` reply built off the async worker, for a viewer with no live
/// subscription to swap (normally [`resync_frame`] answers `scrollback`).
async fn snapshot_frame(h: &Arc<PtyHandle>, lines: usize, binary: bool) -> Snap {
    let h = Arc::clone(h);
    let epoch = h.spawn_seq();
    off_worker(move || Snap::build(h.snapshot_with_history(lines), epoch, binary))
        .await
        .unwrap_or_else(|| Snap::build(Vec::new(), epoch, false))
}

/// Replace this viewer's backlog with a fresh full snapshot. The capture
/// takes the snapshot and a NEW subscription under one emulator lock
/// (`PtyHandle::capture_and_subscribe`), and the old receiver — with every
/// chunk still queued in it, all already absorbed by the emulator — is
/// dropped. So each chunk is either in the snapshot or on the new receiver:
/// never double-applied, and (unlike the old drain → snapshot → drain) never
/// lost when parsed just after the snapshot released the lock. Copying,
/// formatting and encoding run on the blocking pool ([`off_worker`]).
async fn resync_frame(
    rx: &mut broadcast::Receiver<Bytes>,
    capture: impl FnOnce() -> Capture + Send + 'static,
    binary: bool,
) -> Snap {
    let built = off_worker(move || {
        let (data, epoch, output) = capture();
        (Snap::build(data, epoch, binary), output)
    })
    .await;
    match built {
        Some((frame, output)) => {
            *rx = output;
            frame
        }
        // The formatter panicked: drop the backlog anyway; an empty snapshot
        // is ignored by the client, which keeps its screen.
        None => {
            drain_backlog(rx);
            Snap::build(Vec::new(), 0, false)
        }
    }
}

/// Flow-control resume: when output was held back while paused, the
/// resync snapshot to send; `None` when nothing was skipped (the stream just
/// continues — no rebuild, no flicker).
async fn resume_frame(
    rx: &mut broadcast::Receiver<Bytes>,
    capture: impl FnOnce() -> Capture + Send + 'static,
    binary: bool,
) -> Option<Snap> {
    if drain_backlog(rx) {
        Some(resync_frame(rx, capture, binary).await)
    } else {
        None
    }
}

/// Client `resync`: open the flow gate (the backlog it guarded was dropped
/// client-side) and replace this viewer's queued output with one snapshot.
/// Always answers — the client already discarded bytes and relies on it.
async fn client_resync_frame(
    flow: &mut FlowGate,
    rx: &mut broadcast::Receiver<Bytes>,
    capture: impl FnOnce() -> Capture + Send + 'static,
    binary: bool,
) -> Snap {
    flow.resume();
    resync_frame(rx, capture, binary).await
}

/// Default / bounds for a client-proposed credit window (`credit` frame).
const CREDIT_WINDOW_DEFAULT: u64 = 1024 * 1024;
const CREDIT_WINDOW_MIN: u64 = 64 * 1024;
const CREDIT_WINDOW_MAX: u64 = 8 * 1024 * 1024;

/// Per-connection credit window (the successor of [`FlowGate`] for clients
/// that send `credit`).
///
/// `pause`/`resume` cannot bound the client's backlog: every byte sent during
/// the pause frame's round trip still lands, so the overshoot grew with the
/// send rate (2.5 MB budget → 3.0–4.4 MB peak at ~26 MB/s, 14 MB at ~100 MB/s).
/// Here the SERVER counts: it sends at most `window` bytes the client has not
/// acknowledged (`ack` = cumulative bytes parsed or dropped), so the client's
/// backlog of live output is ≤ `window` whatever the producer's rate.
///
/// Output that arrives while the window is closed is held here — up to one
/// more window, so any burst up to 2 × window is delivered losslessly (the
/// same 2 MB a pause-mode client absorbed before pausing). Beyond that the
/// held bytes are dropped (`skipped`) and, once the client has drained to a
/// quarter window, replaced by ONE snapshot: the lagging-viewer resync. The
/// broadcast receiver keeps being read throughout, so the ring never lags.
///
/// Public so other terminal transports (the room terminal socket in
/// otto-server) enforce the same window with the same rules.
#[derive(Debug)]
pub struct CreditGate {
    window: u64,
    /// Binary bytes sent since the grant.
    sent: u64,
    /// Cumulative bytes the client reported consumed (≤ `sent`).
    acked: u64,
    /// Output waiting for credit (≤ `window` bytes).
    held: bytes::BytesMut,
    /// Held output overflowed and was dropped: next reopen sends a snapshot.
    skipped: bool,
    /// Since when this gate has waited on the client without progress.
    stalled_since: Option<tokio::time::Instant>,
    /// The stall deadline fired and nothing moved since: don't re-arm it
    /// until the client acks again (see [`CreditGate::forgive`]).
    forgiven: bool,
}

/// What the socket loop must do after a [`CreditGate`] step.
#[derive(Debug, PartialEq)]
pub enum CreditStep {
    Idle,
    /// Send these bytes as one binary frame (already counted as sent).
    Send(Bytes),
    /// Replace the skipped output with one snapshot ([`resync_frame`]).
    Resync,
}

impl CreditGate {
    /// A gate for the client's `credit` offer (clamped; 0 = default).
    pub fn new(requested: u64) -> Self {
        let window = if requested == 0 {
            CREDIT_WINDOW_DEFAULT
        } else {
            requested.clamp(CREDIT_WINDOW_MIN, CREDIT_WINDOW_MAX)
        };
        Self {
            window,
            sent: 0,
            acked: 0,
            held: bytes::BytesMut::new(),
            skipped: false,
            stalled_since: None,
            forgiven: false,
        }
    }

    /// The `{"type":"credit","window":W}` grant; binary frames sent after it
    /// are counted (WS frames are ordered, so both sides agree where).
    pub fn grant_frame(&self) -> String {
        format!(r#"{{"type":"credit","window":{}}}"#, self.window)
    }

    fn unacked(&self) -> u64 {
        self.sent - self.acked
    }

    fn available(&self) -> usize {
        self.window.saturating_sub(self.unacked()) as usize
    }

    /// Waiting on the client: output is held with no credit, or a skip waits
    /// for the client to drain.
    fn waiting(&self) -> bool {
        self.skipped || (!self.held.is_empty() && self.available() == 0)
    }

    fn track_stall(&mut self, now: tokio::time::Instant) {
        if !self.waiting() {
            self.stalled_since = None;
        } else if self.stalled_since.is_none() && !self.forgiven {
            self.stalled_since = Some(now);
        }
    }

    /// Hand out as much held output as the window allows.
    fn take(&mut self) -> CreditStep {
        let n = self.available().min(self.held.len());
        if n == 0 {
            return CreditStep::Idle;
        }
        self.sent += n as u64;
        CreditStep::Send(self.held.split_to(n).freeze())
    }

    /// New live output for this viewer (one coalesced chunk).
    pub fn push(&mut self, chunk: impl Into<Bytes>, now: tokio::time::Instant) -> CreditStep {
        let chunk: Bytes = chunk.into();
        let step = if self.skipped {
            CreditStep::Idle
        } else if self.held.is_empty() && chunk.len() <= self.available() {
            // Fast path (all normal output): straight through, no copy.
            self.sent += chunk.len() as u64;
            CreditStep::Send(chunk)
        } else {
            self.held.extend_from_slice(&chunk);
            if self.held.len() as u64 > self.window {
                // More than a window behind on top of a full window in the
                // client: stop buffering, rebuild from a snapshot later.
                self.held = bytes::BytesMut::new();
                self.skipped = true;
                CreditStep::Idle
            } else {
                self.take()
            }
        };
        self.track_stall(now);
        step
    }

    /// The window (re)opened: send held output, or the pending snapshot once
    /// the client is down to a quarter window.
    fn reopen(&mut self) -> CreditStep {
        if self.skipped {
            if self.unacked() > self.window / 4 {
                return CreditStep::Idle;
            }
            self.skipped = false;
            return CreditStep::Resync;
        }
        self.take()
    }

    /// Client `ack` (cumulative). Stale or out-of-range values are clamped.
    pub fn ack(&mut self, cumulative: u64, now: tokio::time::Instant) -> CreditStep {
        let c = cumulative.min(self.sent);
        if c > self.acked {
            self.acked = c;
            self.forgiven = false;
            if self.waiting() {
                // Progress: the client is draining, restart the stall clock.
                self.stalled_since = Some(now);
            }
        }
        let step = self.reopen();
        self.track_stall(now);
        step
    }

    /// The user typed while this viewer is more than a window behind: they
    /// want the present, not the held backlog (the server-side twin of the
    /// client's `resync` on input). The held output becomes a snapshot.
    pub fn skip_on_input(&mut self, user: bool, now: tokio::time::Instant) {
        // Emulator replies (for example cursor reports) are not a request to
        // interrupt a backlog; only explicit typing may discard held output.
        if user && !self.held.is_empty() {
            self.held = bytes::BytesMut::new();
            self.skipped = true;
            self.track_stall(now);
        }
    }

    /// A snapshot just went out (lag / client resync / revive / request): it
    /// already reflects everything held here — sending that after it would
    /// double-apply it.
    pub fn superseded(&mut self) {
        self.held = bytes::BytesMut::new();
        self.skipped = false;
        self.stalled_since = None;
        self.forgiven = false;
    }

    /// No `ack` progress for [`FLOW_AUTO_RESUME`] while output waits.
    pub fn stall_deadline(&self) -> Option<tokio::time::Instant> {
        self.stalled_since.map(|t| t + FLOW_AUTO_RESUME)
    }

    /// The stall deadline passed: the client's renderer is wedged (a blocked
    /// main thread, a napped or hidden window) and whatever is held is going
    /// stale. Drop it and owe the client ONE snapshot once it has drained to a
    /// quarter window — exactly the overflow path. The window is NOT reopened:
    /// `acked` stays truthful, so however long or often the client stalls it
    /// never has more than `window` unacknowledged bytes (r3-10-07: this used
    /// to set `acked = sent` and send up to another window per 2 s stall,
    /// piling megabytes into a renderer that could not parse them).
    ///
    /// No deadlock: a live client acks everything it parses AND everything it
    /// drops (termFlow.ts), and the unreported remainder is < an ack step ≤
    /// `window / 4`, so its acks always bring the gate to the snapshot. Only a
    /// client that stops executing waits — and it is sent nothing meanwhile.
    pub fn forgive(&mut self) {
        self.held = bytes::BytesMut::new();
        self.skipped = true;
        self.stalled_since = None;
        self.forgiven = true;
    }
}

/// Carry out a [`CreditStep`] on the socket. `Err` = the socket is gone.
async fn apply_credit_step(
    step: CreditStep,
    socket: &mut WebSocket,
    out_rx: &mut Option<broadcast::Receiver<Bytes>>,
    handle: Option<&Arc<PtyHandle>>,
    history: usize,
    binary: bool,
) -> std::result::Result<(), ()> {
    match step {
        CreditStep::Idle => Ok(()),
        CreditStep::Send(bytes) => socket.send(Message::Binary(bytes)).await.map_err(|_| ()),
        CreditStep::Resync => {
            let (Some(rx), Some(h)) = (out_rx.as_mut(), handle) else {
                return Ok(());
            };
            resync_frame(rx, pty_capture(h, history), binary)
                .await
                .send(socket)
                .await
        }
    }
}

/// The `search_result` reply. Built with serde_json: matched lines are
/// ANSI-stripped but keep tabs, BEL, backspace and other C0 bytes, which JSON
/// forbids raw — the old hand-rolled escaper emitted them verbatim, the
/// client's `JSON.parse` threw, and the find bar spun on "…" forever (SA-11).
/// `{"type":"probe_ack","id":N,"echo":{last_ms,avg_ms,max_ms,samples}|null}`.
fn probe_ack_frame(id: u64, echo: Option<otto_pty::EchoStats>) -> String {
    serde_json::json!({ "type": "probe_ack", "id": id, "echo": echo }).to_string()
}

fn search_result_frame(query: &str, matches: Vec<(usize, String)>) -> String {
    let matches: Vec<serde_json::Value> = matches
        .into_iter()
        .map(|(line, text)| serde_json::json!({ "line": line, "text": text }))
        .collect();
    serde_json::json!({
        "type": "search_result",
        "query": query,
        "matches": matches,
    })
    .to_string()
}

/// Maximum number of matches returned per `Search` request.
const MAX_SEARCH_RESULTS: usize = 200;

/// Default scrollback depth used for server-initiated snapshot pushes (dead
/// session revival, lagged-stream resync) and for a `lines: 0` request. Full
/// emulator depth: clients rebuild from snapshots, so anything less silently
/// truncates the user's visible scrollback.
const DEFAULT_ATTACH_HISTORY_LINES: usize = otto_pty::EMULATOR_SCROLLBACK_LINES;

/// The history depth a `scrollback`/`resync` request asks for: `0` means the
/// full retained depth; anything larger than the emulator keeps is clamped.
fn requested_history(lines: usize) -> usize {
    if lines == 0 {
        DEFAULT_ATTACH_HISTORY_LINES
    } else {
        lines.min(DEFAULT_ATTACH_HISTORY_LINES)
    }
}

/// Fixed first subprotocol the browser offers alongside the token; the gate
/// echoes it back on a successful upgrade so the handshake completes. Mirrors
/// the same constant in `otto-server`'s `ws_events.rs`.
const BEARER_SUBPROTOCOL: &str = "otto-bearer";

/// Extract the bearer token from a `Sec-WebSocket-Protocol: otto-bearer, <token>`
/// request header. Returns `None` when the header is absent or not in that form.
fn token_from_subprotocol(headers: &HeaderMap) -> Option<String> {
    let raw = headers
        .get(axum::http::header::SEC_WEBSOCKET_PROTOCOL)?
        .to_str()
        .ok()?;
    // The header is a comma-separated list; the browser sends the fixed marker
    // first and the token second.
    let mut parts = raw.split(',').map(str::trim);
    if parts.next()? != BEARER_SUBPROTOCOL {
        return None;
    }
    let token = parts.next()?;
    (!token.is_empty()).then(|| token.to_string())
}

/// Build the standalone terminal-WS router (carries its own state).
///
/// The route is layered with [`ws_auth_gate`]: token validation, session
/// lookup, and owner-or-admin check all run BEFORE axum attempts the WS
/// upgrade, so a forbidden caller receives a plain 403 JSON response without
/// the connection ever being promoted to WebSocket.
pub fn ws_router<S: SessionsCtx>(authenticator: Arc<dyn TokenAuthenticator>, ctx: S) -> Router {
    let state = WsState {
        auth: authenticator,
        ctx,
    };
    Router::new()
        .route("/ws/term/{session_id}", get(term_ws::<S>))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            ws_auth_gate::<S>,
        ))
        .with_state(state)
}

fn problem(status: StatusCode, e: &Error) -> Response {
    let body = Problem {
        code: e.code().to_string(),
        message: e.to_string(),
    };
    (status, Json(body)).into_response()
}

/// Route-layer middleware: authenticate the token (from `Sec-WebSocket-Protocol`
/// or `?token=`), look up the session, and enforce the owner-or-admin gate (#L9).
///
/// Token extraction order (Task 1.10):
///  1. `Sec-WebSocket-Protocol: otto-bearer, <token>` — preferred; keeps the
///     token out of the URL (which is logged everywhere). On a successful
///     subprotocol auth, the upgrade response echoes `otto-bearer` back.
///  2. `?token=<bearer token>` query param — backward-compatible fallback.
///
/// On success, inserts [`AuthUser`], [`CanInput`], and [`UsedSubprotocol`]
/// extensions so `term_ws` can read them. On failure, returns a 401/403/404
/// JSON problem BEFORE the WebSocket upgrade extractor runs — this is the
/// property the isolation tests rely on.
///
/// Rate limiting (Task 1.8, S8-02): the token is authenticated FIRST and a
/// valid one always passes. A failed auth records a failure against the
/// tunnel-aware client IP and, once that IP is locked, is answered 429 instead
/// of 401. Store errors (`Internal`) are 503 and never counted.
async fn ws_auth_gate<S: SessionsCtx>(
    State(st): State<WsState<S>>,
    Path(session_id): Path<Id>,
    Query(q): Query<TokenQuery>,
    mut req: Request,
    next: Next,
) -> Response {
    // 0. The client IP: the host guard's tunnel-aware [`share_throttle::ClientIp`]
    //    when present, else the raw `ConnectInfo` peer (absent only in unit-test
    //    harnesses that wire neither).
    let peer_ip = share_throttle::client_ip(req.extensions());

    // 1. Token source resolution (Task 1.10): subprotocol first, then query.
    let subprotocol_token = token_from_subprotocol(req.headers());
    let used_subprotocol = subprotocol_token.is_some();
    let token = match subprotocol_token.or(q.token) {
        Some(t) => t,
        None => return problem(StatusCode::UNAUTHORIZED, &Error::Unauthorized),
    };

    // 2. Token auth FIRST (S8-02 / S1-05). A token that verifies is never
    //    refused by the lockout: on the desktop (and behind a tunnel without
    //    `CF-Connecting-IP`) every client shares 127.0.0.1, so a lock checked
    //    before auth let any web page — or a stale-token reconnect loop — keep
    //    every terminal from attaching. Tokens are 256-bit; the lockout only
    //    ever needs to slow down guesses, so it gates failures alone.
    let auth = match st.auth.authenticate(&token).await {
        Ok(auth) => auth,
        // A store hiccup is not a guess: never counted, never a lockout.
        Err(e @ Error::Internal(_)) => return problem(StatusCode::SERVICE_UNAVAILABLE, &e),
        Err(_) => {
            if let Some(ip) = peer_ip {
                if let Err(locked) = share_throttle::global().check(ip) {
                    let secs = locked.retry_after.as_secs().max(1);
                    let body = otto_core::api::Problem {
                        code: "too_many_requests".to_string(),
                        message: "too many failed share-token attempts; try again later"
                            .to_string(),
                    };
                    return (
                        StatusCode::TOO_MANY_REQUESTS,
                        [("retry-after", secs.to_string())],
                        Json(body),
                    )
                        .into_response();
                }
                share_throttle::global().record_failure(ip);
            }
            return problem(StatusCode::UNAUTHORIZED, &Error::Unauthorized);
        }
    };
    // 2. Session lookup.
    let session = match st.ctx.manager().get(&session_id).await {
        Ok(s) => s,
        Err(e) => return problem(StatusCode::NOT_FOUND, &e),
    };

    // 2b. Agent-credential confinement. This root-mounted route never passes
    // the `/api/v1` feature guard, so the agent-token rules are applied here.
    let agent_input = match agent_attach_rule(&auth, &session) {
        Ok(allowed) => allowed,
        Err(e) => return problem(StatusCode::FORBIDDEN, &e),
    };
    // Authorize attach against the effective user (== real for a normal token);
    // the owner-or-admin gate below runs on the effective identity.
    let user = auth.effective_user;

    // 3. Authorize the attach + decide write capability. Two disjoint paths:
    //
    //  - **Scoped (share-link) token** (`scope == Some`): the scope IS the
    //    authority, so the owner-or-admin gate is BYPASSED. The single guarantee
    //    a share carries is its one pinned `session_id`, so the path id MUST
    //    equal `scope.session_id` (a share for S1 must never attach to S2, even
    //    if the same owner created S2) → otherwise 403, no upgrade. Write
    //    capability is the share's capped role: `Editor` may type/resize, a
    //    `Viewer` share is strictly read-only. The workspace-role probe is
    //    ignored entirely (the synthetic share principal holds no membership).
    //
    //  - **Unscoped token** (`scope == None`): unchanged behaviour — the
    //    owner-or-admin gate (#L9) plus the Editor probe for write capability.
    let scoped = auth.scope.is_some();
    let can_input = match auth.scope {
        Some(scope) => {
            if scope.session_id != session_id {
                return problem(
                    StatusCode::FORBIDDEN,
                    &Error::Forbidden("share token is scoped to a different session".into()),
                );
            }
            // EMAIL-OTP GATE (mobile plan Task 7.3). A share locked to a recipient
            // email may NOT attach until the guest redeems the emailed OTP via
            // `POST /api/v1/share/verify` — and once verified, only inside the
            // ≤12h window. `otp_pending` captures both: it is `true` while the
            // code is unredeemed AND once the `max_expires_at` window has elapsed,
            // so this single check rejects an unverified OR an expired-window share
            // before the WS upgrade. Fail closed (no socket for a leaked link).
            if scope.otp_pending {
                return problem(
                    StatusCode::FORBIDDEN,
                    &Error::Forbidden(
                        "share requires email-OTP verification before attaching".into(),
                    ),
                );
            }
            scope.role == WorkspaceRole::Editor
        }
        None => {
            // Owner-or-admin gate (#L9): only the creator, a workspace Admin, or
            // root may attach. Workspace Viewers/Editors who are not the owner
            // are refused here, before the WS upgrade.
            if !session_owner_or_admin(st.ctx.roles().as_ref(), &user, &session).await {
                return problem(
                    StatusCode::FORBIDDEN,
                    &Error::Forbidden("not the session owner or a workspace admin".into()),
                );
            }
            // Write capability (owner/ws-admin/root all pass the Editor check; a
            // viewer-owner would fail it and stay read-only — unchanged).
            st.ctx
                .roles()
                .check(&user, &session.workspace_id, WorkspaceRole::Editor)
                .await
                .is_ok()
        }
    } && agent_input;

    if let Err(e) = st.ctx.check_resource(&user, &session).await {
        return problem(StatusCode::FORBIDDEN, &e);
    }

    // Propagate auth results to the handler via extensions.
    req.extensions_mut().insert(LiveTerminalAuth {
        user: user.clone(),
        scoped,
        token,
        auth: st.auth.clone(),
    });
    req.extensions_mut().insert(AuthUser(user));
    req.extensions_mut().insert(CanInput(can_input));
    // Tell term_ws whether the client used the subprotocol path so it can echo
    // `otto-bearer` back in the upgrade response (Task 1.10).
    req.extensions_mut()
        .insert(UsedSubprotocol(used_subprotocol));

    next.run(req).await
}

/// Agent-credential confinement for the terminal socket. The `/api/v1` feature
/// guard confines agent tokens (MCP-only, read-only sessions), but `/ws/term`
/// is root-mounted and a WS upgrade is a GET, so those rules are restated here:
///
/// - an MCP-restricted token (external `.mcp.json` or a session's internal MCP
///   credential) never attaches to a terminal;
/// - an Otto-minted agent-session token (`managed_session_id`) may attach only
///   to its OWN session — never type into another terminal of its owner;
/// - a read-only session (`meta.read_only = true`) gets a view-only socket.
///
/// `Ok(input_allowed)` caps the caller's write capability; `Err` is a 403.
fn agent_attach_rule(
    auth: &otto_core::auth::AuthContext,
    session: &otto_core::domain::Session,
) -> otto_core::Result<bool> {
    if auth.mcp_only {
        return Err(Error::Forbidden(
            "mcp-restricted token cannot attach to a terminal".into(),
        ));
    }
    let Some(own) = auth.managed_session_id.as_ref() else {
        return Ok(true);
    };
    if *own != session.id {
        return Err(Error::Forbidden(
            "an agent session token may only attach to its own session".into(),
        ));
    }
    Ok(session
        .meta
        .get("read_only")
        .and_then(serde_json::Value::as_bool)
        != Some(true))
}

/// Newtype extension carrying the write-capability flag set by [`ws_auth_gate`].
#[derive(Clone, Copy)]
struct CanInput(bool);

#[derive(Clone)]
struct LiveTerminalAuth {
    scoped: bool,
    user: otto_core::domain::User,
    token: String,
    auth: Arc<dyn TokenAuthenticator>,
}
impl LiveTerminalAuth {
    async fn check<S: SessionsCtx>(
        &self,
        ctx: &S,
        session: &otto_core::domain::Session,
    ) -> otto_core::Result<bool> {
        let auth = self.auth.authenticate(&self.token).await?;
        if auth.effective_user.id != self.user.id || auth.mcp_only {
            return Err(Error::Unauthorized);
        }
        let agent_input = agent_attach_rule(&auth, session)?;
        let can_input = if let Some(scope) = auth.scope {
            if scope.session_id != session.id || scope.otp_pending {
                return Err(Error::Unauthorized);
            }
            scope.role == WorkspaceRole::Editor
        } else {
            if !session_owner_or_admin(ctx.roles().as_ref(), &auth.effective_user, session).await {
                return Err(Error::Forbidden("session access revoked".into()));
            }
            ctx.roles()
                .check(
                    &auth.effective_user,
                    &session.workspace_id,
                    WorkspaceRole::Editor,
                )
                .await
                .is_ok()
        } && agent_input;
        ctx.check_resource(&auth.effective_user, session).await?;
        Ok(can_input)
    }
}

/// Newtype extension: true iff the client presented the token via the
/// `Sec-WebSocket-Protocol: otto-bearer, <token>` header. When set, `term_ws`
/// echoes the `otto-bearer` subprotocol in the upgrade response so the browser
/// handshake completes; a bare `?token=` client gets a plain upgrade.
#[derive(Clone, Copy)]
struct UsedSubprotocol(bool);

async fn term_ws<S: SessionsCtx>(
    ws: WebSocketUpgrade,
    Path(session_id): Path<Id>,
    State(st): State<WsState<S>>,
    axum::Extension(live_auth): axum::Extension<LiveTerminalAuth>,
    axum::Extension(CanInput(can_input)): axum::Extension<CanInput>,
    axum::Extension(UsedSubprotocol(used_subprotocol)): axum::Extension<UsedSubprotocol>,
    Query(attach): Query<AttachQuery>,
) -> Response {
    let view_only = attach.view_only();
    // Auth and owner-gate already enforced by ws_auth_gate middleware.
    let session = match st.ctx.manager().get(&session_id).await {
        Ok(s) => s,
        Err(e) => return problem(StatusCode::NOT_FOUND, &e),
    };
    let initial_status = session.status;
    // Echo `otto-bearer` only when the client used the subprotocol path (Task
    // 1.10): the browser rejects an unsolicited subprotocol in the upgrade
    // response, so we must not echo it for legacy `?token=` clients.
    if used_subprotocol {
        ws.protocols([BEARER_SUBPROTOCOL])
            .on_upgrade(move |socket| async move {
                serve_terminal(
                    socket,
                    st.ctx,
                    session_id,
                    initial_status,
                    can_input,
                    view_only,
                    live_auth,
                )
                .await;
            })
    } else {
        ws.on_upgrade(move |socket| async move {
            serve_terminal(
                socket,
                st.ctx,
                session_id,
                initial_status,
                can_input,
                view_only,
                live_auth,
            )
            .await;
        })
    }
}

/// Receive the next live output chunk, pending forever without a handle.
async fn next_output(
    rx: &mut Option<broadcast::Receiver<Bytes>>,
) -> Result<Bytes, broadcast::error::RecvError> {
    match rx {
        Some(r) => r.recv().await,
        None => std::future::pending().await,
    }
}

/// Wait for the child exit code, pending forever without a handle.
async fn next_exit(rx: &mut Option<watch::Receiver<Option<i32>>>) -> i32 {
    match rx {
        Some(r) => match crate::manager::wait_exit_code(r).await {
            Some(code) => code,
            None => std::future::pending().await,
        },
        None => std::future::pending().await,
    }
}

/// Point a terminal viewer at the session's CURRENT live PTY when that is not
/// the one it is showing — its process exited and something respawned the
/// session. Resubscribes to the new output/exit streams, then pushes a
/// `status` frame and a full `scrollback` snapshot (new `epoch`) so the client
/// drops its exited state and rebuilds from the new process. Without this a
/// tab left open across an exit stayed on the dead screen while its
/// keystrokes (routed by session id) went, unseen, into the new process.
///
/// `Ok(true)` = swapped, `Ok(false)` = nothing newer is live, `Err(())` = the
/// socket is gone.
#[allow(clippy::too_many_arguments)]
async fn revive_viewer<S: SessionsCtx>(
    ctx: &S,
    session_id: &Id,
    socket: &mut WebSocket,
    handle: &mut Option<Arc<PtyHandle>>,
    out_rx: &mut Option<broadcast::Receiver<Bytes>>,
    exit_rx: &mut Option<watch::Receiver<Option<i32>>>,
    history: usize,
    binary: bool,
) -> std::result::Result<bool, ()> {
    let Some(fresh) = ctx.manager().live_handle(session_id) else {
        return Ok(false);
    };
    if handle.as_ref().is_some_and(|old| Arc::ptr_eq(old, &fresh)) {
        return Ok(false);
    }
    // Snapshot + subscription in one emulator lock (exact hand-over, see
    // `resync_frame`), built off the async worker. `epoch` = PTY spawn
    // counter: the client resets its local buffer when it changes (see the
    // Scrollback arm).
    let capture = pty_capture(&fresh, history);
    let (frame, output) = match off_worker(move || {
        let (data, epoch, output) = capture();
        (Snap::build(data, epoch, binary), output)
    })
    .await
    {
        Some(built) => built,
        None => (
            Snap::build(Vec::new(), fresh.spawn_seq(), false),
            fresh.subscribe(),
        ),
    };
    *out_rx = Some(output);
    *exit_rx = Some(fresh.on_exit());
    let status = r#"{"type":"status","status":"running"}"#;
    if socket.send(Message::Text(status.into())).await.is_err() {
        return Err(());
    }
    frame.send(socket).await?;
    *handle = Some(fresh);
    Ok(true)
}

/// Await the next capability verdict from the off-loop re-auth task.
/// `Some(can_input)` is a fresh (monotonically narrowing) capability;
/// `None` means the task returned — access was revoked or the session row is
/// gone — and this viewer must be evicted. Mirrors the other `next_*` helpers:
/// the receiver borrow stays INSIDE the future, so the `select!` arm may touch
/// the receiver again in its handler.
async fn next_can_input(rx: &mut watch::Receiver<bool>) -> Option<bool> {
    match rx.changed().await {
        Ok(()) => Some(*rx.borrow_and_update()),
        Err(_) => None,
    }
}

/// Periodic re-authorization for one attached terminal, run OFF the socket's
/// `select!` loop so no arm of that loop ever awaits SQLite (investigation H2).
///
/// It owns the `can_input` watch and publishes each pass's raw verdict; every
/// subscribed socket narrows its own capability with it monotonically (a
/// share downgraded mid-connection loses input and can never regain it,
/// exactly as the old inline check did). Production sockets share one pass
/// per token + session ([`shared_reauth`]); this wrapper is the unshared form.
/// Revocation is signalled by RETURNING: dropping `can_tx` closes the channel,
/// which the select loop reads as "evict this viewer". The task also stops the
/// moment the socket goes away (`can_tx.closed()`), so a detached session never
/// leaves a timer — or a DB query — behind.
#[cfg(test)]
async fn reauth_loop<S: SessionsCtx>(
    ctx: S,
    session_id: Id,
    live_auth: LiveTerminalAuth,
    can_tx: watch::Sender<bool>,
    period: Duration,
) {
    reauth_loop_shared(ctx, session_id, live_auth, can_tx, period, None).await;
}

/// Key of one shared re-auth pass: the verdict is a pure function of the
/// token and the session (and the cadence decides how stale it may get), so
/// every socket presenting the same token to the same session can share it.
type ReauthKey = (Id, String, Duration);

/// One running re-auth pass per [`ReauthKey`] (perf 01 F8). A pane, its
/// tile in the overview and a parked copy of it used to run three identical
/// SQLite passes every 5 s; now they subscribe to one.
static SHARED_REAUTH: LazyLock<dashmap::DashMap<ReauthKey, (u64, watch::Sender<bool>)>> =
    LazyLock::new(dashmap::DashMap::new);

/// Subscribe this socket to the shared re-auth pass for its token +
/// session, starting one when none runs. The watch carries the latest raw
/// verdict (`true` = may input); each socket narrows its own capability
/// with it, so sharing never widens what any one socket may do. The channel
/// closing still means "revoked: evict".
fn shared_reauth<S: SessionsCtx>(
    ctx: &S,
    session_id: &Id,
    live_auth: LiveTerminalAuth,
    period: Duration,
) -> watch::Receiver<bool> {
    static GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let key: ReauthKey = (session_id.clone(), live_auth.token.clone(), period);
    match SHARED_REAUTH.entry(key.clone()) {
        dashmap::mapref::entry::Entry::Occupied(e) => e.get().1.subscribe(),
        dashmap::mapref::entry::Entry::Vacant(v) => {
            let (tx, rx) = watch::channel(true);
            let generation = GEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            v.insert((generation, tx.clone()));
            tokio::spawn(reauth_loop_shared(
                ctx.clone(),
                session_id.clone(),
                live_auth,
                tx,
                period,
                Some((key, generation)),
            ));
            rx
        }
    }
}

/// Drop this pass's registry entry: unconditionally on revocation (sockets
/// that arrive later must start a fresh check, not join a dying one), or —
/// `only_if_unwatched` — only while nobody is subscribed, decided under the
/// map's shard lock so a socket joining at that instant keeps the pass alive.
fn unregister_reauth(
    shared: &Option<(ReauthKey, u64)>,
    can_tx: &watch::Sender<bool>,
    only_if_unwatched: bool,
) -> bool {
    let Some((key, generation)) = shared else {
        return true;
    };
    SHARED_REAUTH
        .remove_if(key, |_, (g, _)| {
            *g == *generation && (!only_if_unwatched || can_tx.receiver_count() == 0)
        })
        .is_some()
        || !SHARED_REAUTH.get(key).is_some_and(|e| e.0 == *generation)
}

async fn reauth_loop_shared<S: SessionsCtx>(
    ctx: S,
    session_id: Id,
    live_auth: LiveTerminalAuth,
    can_tx: watch::Sender<bool>,
    period: Duration,
    shared: Option<(ReauthKey, u64)>,
) {
    let mut tick = tokio::time::interval(period);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    tick.tick().await; // consume the immediate first tick — attach just authorized
    loop {
        tokio::select! {
            _ = can_tx.closed() => {
                // Every socket left. Stop — unless one joined just now.
                if unregister_reauth(&shared, &can_tx, true) {
                    return;
                }
                continue;
            }
            _ = tick.tick() => {}
        }
        let started = std::time::Instant::now();
        let current = match ctx.manager().get(&session_id).await {
            Ok(current) => current,
            // A transient store error (SQLITE_BUSY, pool timeout) is not a
            // revocation: keep the last verdict and look again next tick.
            Err(Error::Internal(_)) => continue,
            Err(_) => {
                unregister_reauth(&shared, &can_tx, false);
                return;
            }
        };
        let get_ms = started.elapsed().as_millis() as u64;
        let verdict = live_auth.check(&ctx, &current).await;
        let elapsed = started.elapsed();
        if elapsed > REAUTH_SLOW {
            tracing::warn!(
                session = %session_id,
                elapsed_ms = elapsed.as_millis() as u64,
                get_ms,
                "terminal ws: auth re-check slow"
            );
        }
        match verdict {
            // The raw verdict; each socket narrows its own capability with
            // it (capability only ever narrows for a live connection).
            Ok(allowed) => {
                can_tx.send_if_modified(|cur| {
                    let changed = allowed != *cur;
                    *cur = allowed;
                    changed
                });
            }
            // Transient (DB busy / pool timeout): keep the last verdict, retry
            // next tick. Only a definite auth/authz failure evicts.
            Err(Error::Internal(_)) => continue,
            Err(_) => {
                // Access revoked mid-connection: evict THIS viewer only (S1-01).
                // The session itself is never killed from here — a share or
                // impersonation token resolves to the OWNER, so "the revoked
                // user created it" used to take the owner's live agent down when
                // a guest's share lapsed, an impersonation expired, or the owner
                // logged out on another device.
                unregister_reauth(&shared, &can_tx, false);
                return;
            }
        }
    }
}

/// Wait for the per-session forced-disconnect signal. `Ok` (fired) and `Lagged`
/// both mean "evict"; `Closed` (the sender was dropped without ever firing,
/// e.g. the session row was removed) clears the receiver and pends forever so
/// this branch stops competing in the `select!` without busy-looping.
async fn next_evict(rx: &mut Option<broadcast::Receiver<()>>) {
    match rx {
        Some(r) => match r.recv().await {
            Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => {}
            Err(broadcast::error::RecvError::Closed) => {
                *rx = None;
                std::future::pending().await
            }
        },
        None => std::future::pending().await,
    }
}

/// Bytes of keystrokes / pastes one connection may have queued for the PTY
/// (perf 01 N7). Past this further `input` frames are deferred in order on
/// the socket loop ([`DeferredInput`]) instead of dropped. Bounded by bytes,
/// not frames: 256 one-byte keystrokes behind a slow child used to overflow a
/// frame-count queue and lose the rest of the line.
const INPUT_BUDGET_BYTES: usize = 1024 * 1024;

/// Bytes of `input` frames the socket loop holds back while the budget above
/// is full (perf3 R6). Below it the loop keeps READING client frames, so
/// `ack` / `resize` / `probe` still land and output keeps flowing while a
/// child is not reading its tty; only past it does the loop stop reading
/// (TCP backpressure to the sender, nothing dropped).
const INPUT_DEFER_BYTES: usize = 1024 * 1024;

/// Upper bound on one coalesced PTY write: the writer joins consecutive
/// queued frames of the same kind up to this many bytes per `human_input`.
const INPUT_COALESCE_BYTES: usize = 64 * 1024;

/// One queued `input` frame. `cost` is the budget it holds (released after
/// the write); `prev_owner` is the size owner its optimistic authority claim
/// replaced (user typing only), restored if the write fails.
struct InputJob {
    bytes: Vec<u8>,
    user: bool,
    cost: u32,
    prev_owner: Option<u64>,
}

/// Per-connection input queue (perf 01 F10 + N7): `input` frames are written
/// to the PTY in order on their own task, off the socket's `select!` loop, so
/// a paste into a TUI that is slow to read its tty no longer freezes output.
/// The queue never drops: it is bounded by [`INPUT_BUDGET_BYTES`], and a frame
/// that does not fit waits in the loop's [`DeferredInput`].
struct InputQueue {
    tx: tokio::sync::mpsc::UnboundedSender<InputJob>,
    budget: Arc<tokio::sync::Semaphore>,
}

impl InputQueue {
    /// Budget a frame of `len` bytes holds. A frame larger than the whole
    /// budget holds all of it (it waits for an empty queue, then goes alone).
    fn cost(len: usize) -> u32 {
        len.clamp(1, INPUT_BUDGET_BYTES) as u32
    }

    /// A job for one `input` frame, holding no budget yet.
    fn job(bytes: Vec<u8>, user: bool, prev_owner: Option<u64>) -> InputJob {
        InputJob {
            cost: Self::cost(bytes.len()),
            bytes,
            user,
            prev_owner,
        }
    }

    /// Queue a frame if its budget is free now; `Err` hands it back. (The
    /// socket loop goes through [`DeferredInput::push`], which keeps order.)
    #[cfg(test)]
    fn try_push(
        &self,
        bytes: Vec<u8>,
        user: bool,
        prev_owner: Option<u64>,
    ) -> Result<(), InputJob> {
        self.try_queue(Self::job(bytes, user, prev_owner))
    }

    /// [`InputQueue::try_push`] for a job that is already built.
    fn try_queue(&self, job: InputJob) -> Result<(), InputJob> {
        match self.budget.try_acquire_many(job.cost) {
            Ok(permit) => {
                permit.forget();
                self.send(job);
                Ok(())
            }
            Err(_) => Err(job),
        }
    }

    /// Wait until `cost` bytes of budget are free and take them. Cancel-safe
    /// (a dropped wait holds nothing), so it can sit in a `select!` arm.
    async fn reserve(budget: Arc<tokio::sync::Semaphore>, cost: u32) {
        if let Ok(permit) = budget.acquire_many_owned(cost).await {
            permit.forget();
        }
    }

    /// Hand a job whose budget is already reserved to the writer.
    fn send(&self, job: InputJob) {
        // Fails only once the writer task ended (connection teardown).
        let _ = self.tx.send(job);
    }
}

/// `input` frames waiting for [`InputQueue`] budget, in arrival order (perf3
/// R6). The socket loop used to park ONE such frame and stop reading the
/// socket until it fit — which also stopped reading `ack` frames, so once the
/// credit window ran out output stalled behind a child that was not reading
/// its stdin. Now the loop keeps reading: input frames queue here (bounded by
/// [`INPUT_DEFER_BYTES`]) while every other frame is handled as it arrives.
#[derive(Default)]
struct DeferredInput {
    jobs: std::collections::VecDeque<InputJob>,
    /// Payload bytes held in `jobs`.
    bytes: usize,
}

impl DeferredInput {
    /// Whether the loop may read client frames: until the deferred bytes
    /// reach their bound (then TCP backpressure, never a drop).
    fn reading(&self) -> bool {
        self.bytes < INPUT_DEFER_BYTES
    }

    /// Budget the oldest deferred frame needs, if any frame waits.
    fn front_cost(&self) -> Option<u32> {
        self.jobs.front().map(|j| j.cost)
    }

    /// Hand one `input` frame to the queue — behind anything already
    /// deferred, so order is kept — or defer it when the budget is full.
    fn push(&mut self, q: &InputQueue, bytes: Vec<u8>, user: bool, prev_owner: Option<u64>) {
        let job = InputQueue::job(bytes, user, prev_owner);
        let job = if self.jobs.is_empty() {
            match q.try_queue(job) {
                Ok(()) => return,
                Err(job) => job,
            }
        } else {
            job
        };
        self.bytes += job.bytes.len();
        self.jobs.push_back(job);
    }

    /// The oldest frame's budget was just reserved ([`InputQueue::reserve`]
    /// with [`DeferredInput::front_cost`]): send it, then every frame behind
    /// it that fits now.
    fn release(&mut self, q: &InputQueue) {
        let Some(job) = self.jobs.pop_front() else {
            return;
        };
        self.bytes -= job.bytes.len();
        q.send(job);
        while let Some(job) = self.jobs.pop_front() {
            let len = job.bytes.len();
            if let Err(job) = q.try_queue(job) {
                self.jobs.push_front(job);
                break;
            }
            self.bytes -= len;
        }
    }
}

/// Spawn the writer behind an [`InputQueue`]. `write` delivers one (possibly
/// coalesced) chunk; `revert` undoes a failed chunk's optimistic size claim.
/// Each outcome is reported on the returned receiver (`Err` = the message for
/// the one-per-stretch `input_failed` notice). Ends when the queue is dropped.
fn spawn_input_queue<W, Fut, R>(
    write: W,
    revert: R,
) -> (
    InputQueue,
    tokio::sync::mpsc::Receiver<std::result::Result<(), String>>,
)
where
    W: Fn(Vec<u8>, bool) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = std::result::Result<(), String>> + Send,
    R: Fn(Option<u64>) + Send + 'static,
{
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<InputJob>();
    let budget = Arc::new(tokio::sync::Semaphore::new(INPUT_BUDGET_BYTES));
    let (res_tx, res_rx) = tokio::sync::mpsc::channel(64);
    let task_budget = budget.clone();
    tokio::spawn(async move {
        let mut carry: Option<InputJob> = None;
        loop {
            let first = match carry.take() {
                Some(job) => job,
                None => match rx.recv().await {
                    Some(job) => job,
                    None => break,
                },
            };
            // Coalesce whatever queued up behind it while the last write ran:
            // a burst of keystrokes becomes one PTY write, not N lock round
            // trips. Only frames of the same kind join (an emulator reply is
            // never billed as typing), and order is kept.
            let user = first.user;
            let prev_owner = first.prev_owner;
            let mut cost = first.cost;
            let mut bytes = first.bytes;
            while bytes.len() < INPUT_COALESCE_BYTES {
                match rx.try_recv() {
                    Ok(job) if job.user == user => {
                        bytes.extend_from_slice(&job.bytes);
                        cost += job.cost;
                    }
                    Ok(job) => {
                        carry = Some(job);
                        break;
                    }
                    Err(_) => break,
                }
            }
            let res = write(bytes, user).await;
            task_budget.add_permits(cost as usize);
            if res.is_err() && user {
                revert(prev_owner);
            }
            // Outcomes only drive a notice: never block on a busy loop.
            let _ = res_tx.try_send(res);
        }
    });
    (InputQueue { tx, budget }, res_rx)
}

/// [`spawn_input_queue`] wired to the session manager for one connection.
fn spawn_input_writer<S: SessionsCtx>(
    ctx: S,
    session_id: Id,
    user: Id,
    scoped: bool,
    conn_id: u64,
) -> (
    InputQueue,
    tokio::sync::mpsc::Receiver<std::result::Result<(), String>>,
) {
    let revert_ctx = ctx.clone();
    let revert_id = session_id.clone();
    spawn_input_queue(
        move |bytes: Vec<u8>, user_input: bool| {
            let ctx = ctx.clone();
            let session_id = session_id.clone();
            let user = user.clone();
            async move {
                let started = std::time::Instant::now();
                let res = ctx
                    .manager()
                    .human_input(&session_id, &user, scoped, user_input, &bytes)
                    .await;
                let elapsed = started.elapsed();
                if elapsed > INPUT_SLOW {
                    tracing::debug!(
                        session = %session_id,
                        elapsed_ms = elapsed.as_millis() as u64,
                        bytes = bytes.len(),
                        "terminal ws: slow PTY write (child not draining its tty?)"
                    );
                }
                res.map_err(|e| e.to_string())
            }
        },
        move |prev| {
            revert_ctx
                .manager()
                .revert_input_authority(&revert_id, conn_id, prev)
        },
    )
}

async fn serve_terminal<S: SessionsCtx>(
    mut socket: WebSocket,
    ctx: S,
    session_id: Id,
    initial_status: otto_core::domain::SessionStatus,
    mut can_input: bool,
    view_only: bool,
    live_auth: LiveTerminalAuth,
) {
    // Track this viewer for the whole connection. The guard decrements the
    // session's attached-viewer count on EVERY return path below (clean close,
    // send error, recv end, drop), so the idle-suspend sweep never frees the
    // PTY of a session someone is actively watching.
    let _attach = ctx.manager().attach(&session_id);
    // This connection's id for the size-authority policy: typing claims the
    // session's PTY size; resizes from passive viewers (tiles, previews, an
    // idle phone tab) are ignored while a typing owner is attached.
    let conn_id = _attach.conn_id();

    // Auto-resume: if the session is an exited-but-resumable agent session,
    // spawn it now so the reconnect yields a live terminal instead of a black
    // screen.  Errors are logged and ignored — the WS stays open (no handle).
    // Read-only viewers (shares) never trigger a resume: watching must not
    // spawn a process on the host. (`ensure_live` itself also refuses archived
    // sessions, so an archived row can no longer come back live via attach.)
    // Neither do view-only attaches (`?view=1`, see [`AttachQuery`]): looking
    // at a session must not spawn its CLI; the first real keystroke does.
    let Ok(current) = ctx.manager().get(&session_id).await else {
        return;
    };
    match live_auth.check(&ctx, &current).await {
        Ok(allowed) => can_input &= allowed,
        Err(_) => return,
    }
    if resume_on_attach(can_input, view_only) {
        if let Err(e) = ctx.manager().ensure_live(&session_id).await {
            tracing::warn!(session = %session_id, "ensure_live on ws attach: {e}");
        }
    }

    // Re-read the current status (it may have changed from Exited → Running
    // if ensure_live just resumed the session).
    let current_status = ctx
        .manager()
        .get(&session_id)
        .await
        .map(|s| s.status)
        .unwrap_or(initial_status);

    // On attach: current status first.
    let status_frame = format!(
        r#"{{"type":"status","status":"{}"}}"#,
        current_status.as_str()
    );
    if socket
        .send(Message::Text(status_frame.into()))
        .await
        .is_err()
    {
        return;
    }

    let mut handle: Option<Arc<PtyHandle>> = ctx.manager().live_handle(&session_id);
    let mut out_rx = handle.as_ref().map(|h| h.subscribe());
    let mut exit_rx = handle.as_ref().map(|h| h.on_exit());
    // Forced-disconnect signal: admin terminate / share-link revoke fire this
    // (via SessionManager::evict) to immediately kick attached viewers, even
    // before the PTY broadcast closes.
    let mut evict_rx = Some(ctx.manager().evict_signal(&session_id));
    let mut warned_forbidden = false;
    // One visible notice per stretch of failing input (reset on success).
    let mut warned_input = false;
    let mut ping = tokio::time::interval(PING_INTERVAL);
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    ping.reset(); // skip the immediate first tick
    let mut revive_tick = tokio::time::interval(REVIVE_POLL);
    revive_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // Client-driven flow control (`pause` / `resume` frames).
    let mut flow = FlowGate::default();
    // Credit-based flow control, once the client sends `credit` (else the
    // legacy pause gate above is all there is).
    let mut credit: Option<CreditGate> = None;
    // Snapshot encoding this client asked for on its `credit` frame (N3).
    let mut binary_snapshots = false;
    // History depth this client asked for in its last `scrollback`/`resync`
    // (perf 01 F4): every server-initiated snapshot (credit skip, lag,
    // resume, revive) honours it instead of always sending the full 4000
    // rows to a 2000-row embed that then trims them.
    let mut history = DEFAULT_ATTACH_HISTORY_LINES;

    // Re-authorization runs OFF this loop (investigation H2): every arm here
    // must stay free of SQLite, or a slow statement elsewhere in the daemon
    // freezes the terminal in BOTH directions. The task publishes `can_input`
    // through the watch and signals revocation by dropping its sender. The
    // resource binding is read once, from the session we already have in hand.
    let reauth_period = if ctx.resource_bound(&current) {
        REAUTH_INTERVAL_RESOURCE
    } else {
        REAUTH_INTERVAL
    };
    let input_user = live_auth.user.id.clone();
    let input_scoped = live_auth.scoped;
    let (input_q, mut input_res_rx) = spawn_input_writer(
        ctx.clone(),
        session_id.clone(),
        input_user.clone(),
        input_scoped,
        conn_id,
    );
    // Input frames waiting for byte budget (perf 01 N7), in order. The socket
    // keeps being read meanwhile (perf3 R6): acks, resizes and probes are
    // never stuck behind a child that is not reading its stdin.
    let mut deferred = DeferredInput::default();
    let mut can_rx = shared_reauth(&ctx, &session_id, live_auth, reauth_period);
    // Joining a pass that already narrowed: apply its latest verdict now.
    can_input &= *can_rx.borrow_and_update();

    loop {
        tokio::select! {
            // Capability / revocation verdict from the re-auth task. `None` =
            // the task returned: access was revoked (or the session row is
            // gone) → tell the client and drop the socket.
            update = next_can_input(&mut can_rx) => {
                match update {
                    Some(allowed) => can_input &= allowed,
                    None => {
                        let _ = socket.send(Message::Close(None)).await;
                        return;
                    }
                }
            }

            // Outcome of a keystroke delivered by the input task: one visible
            // notice per stretch of failing input (reset on success).
            Some(res) = input_res_rx.recv() => {
                match res {
                    Ok(()) => warned_input = false,
                    Err(message) if !warned_input => {
                        warned_input = true;
                        let frame = serde_json::json!({
                            "type": "error",
                            "code": "input_failed",
                            "message": message,
                        })
                        .to_string();
                        if socket.send(Message::Text(frame.into())).await.is_err() {
                            return;
                        }
                    }
                    Err(_) => {}
                }
            }

            // Flow control: a paused viewer that never resumed. Resume it
            // anyway (see FLOW_AUTO_RESUME) — held-back output becomes one
            // snapshot, exactly as for an explicit `resume`.
            _ = flow_deadline(flow.paused_until), if flow.is_paused() => {
                flow.resume();
                tracing::debug!(session = %session_id, "terminal ws flow auto-resume (no resume within {FLOW_AUTO_RESUME:?})");
                if let (Some(rx), Some(h)) = (out_rx.as_mut(), handle.as_ref()) {
                    if let Some(frame) = resume_frame(rx, pty_capture(h, history), binary_snapshots).await {
                        if frame.send(&mut socket).await.is_err() {
                            return;
                        }
                        if let Some(c) = credit.as_mut() {
                            c.superseded();
                        }
                    }
                }
            }

            // Credit flow control: the client stopped acknowledging while
            // output waits (see CreditGate::stall_deadline).
            _ = flow_deadline(credit.as_ref().and_then(CreditGate::stall_deadline)),
                if credit.as_ref().is_some_and(|c| c.stalled_since.is_some()) =>
            {
                let Some(c) = credit.as_mut() else { continue };
                tracing::debug!(session = %session_id, "terminal ws credit stalled {FLOW_AUTO_RESUME:?} without an ack; dropping held output for one snapshot on recovery");
                c.forgive();
            }

            // Live PTY output → binary frames. Coalesce rapid bursts into one
            // frame by draining any immediately-available chunks with try_recv.
            // Disabled while the client has paused this stream (flow control):
            // the receiver simply stops reading until resume.
            chunk = next_output(&mut out_rx), if !flow.is_paused() => {
                match chunk {
                    Ok(first) => {
                        // Attempt a non-blocking drain to merge back-to-back
                        // chunks into one WS frame. Lagged errors are harmless
                        // (data is still in the ring buffer).
                        // A lone chunk (the common case) is forwarded as is —
                        // no copy (perf 01 F11); only a burst is coalesced.
                        let mut out = first;
                        if let Some(rx) = out_rx.as_mut() {
                            if let Ok(more) = rx.try_recv() {
                                let mut buf = bytes::BytesMut::with_capacity(out.len() + more.len());
                                buf.extend_from_slice(&out);
                                buf.extend_from_slice(&more);
                                // Cap at ~64 KiB to bound latency.
                                while buf.len() < 64 * 1024 {
                                    let Ok(more) = rx.try_recv() else { break };
                                    buf.extend_from_slice(&more);
                                }
                                out = buf.freeze();
                            }
                        }
                        // Credit mode: send only what the window allows (the
                        // rest is held, or skipped → one snapshot later).
                        let step = match credit.as_mut() {
                            Some(c) => c.push(out, tokio::time::Instant::now()),
                            None => CreditStep::Send(out),
                        };
                        if apply_credit_step(step, &mut socket, &mut out_rx, handle.as_ref(), history, binary_snapshots).await.is_err() {
                            return;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        // This viewer fell behind the broadcast ring and chunks
                        // were dropped. Dropped bytes are not just missing text:
                        // TUI erase/repaint sequences vanish with them, leaving
                        // stale frames on screen until the user reconnects.
                        // Recover by discarding the entire backlog (its effects
                        // are already absorbed by the emulator) and pushing a
                        // fresh full snapshot; the client rebuilds from it.
                        tracing::debug!(session = %session_id, "terminal ws lagged by {n} chunks; resyncing from snapshot");
                        if let (Some(rx), Some(h)) = (out_rx.as_mut(), handle.as_ref()) {
                            let frame = resync_frame(rx, pty_capture(h, history), binary_snapshots).await;
                            if frame.send(&mut socket).await.is_err() {
                                return;
                            }
                            if let Some(c) = credit.as_mut() {
                                c.superseded();
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        out_rx = None;
                    }
                }
            }
            // Child exit → {"type":"exit","code":N}; socket stays open.
            code = next_exit(&mut exit_rx) => {
                exit_rx = None;
                let frame = format!(r#"{{"type":"exit","code":{code}}}"#);
                if socket.send(Message::Text(frame.into())).await.is_err() {
                    return;
                }
            }
            _ = ping.tick() => {
                if socket.send(Message::Ping(Bytes::new())).await.is_err() {
                    return;
                }
            }
            // Revival: a viewer attached while the session was dead, or whose
            // process exited mid-watch, would otherwise stay on that dead
            // screen even after another client / engine respawned the session.
            // (It used to wait for the output broadcast to close, which never
            // happens while this loop holds the dead handle — `exit_rx` is the
            // reliable "my process is gone" signal.) Armed only while dead.
            _ = revive_tick.tick(), if exit_rx.is_none() || out_rx.is_none() => {
                match revive_viewer(&ctx, &session_id, &mut socket, &mut handle, &mut out_rx, &mut exit_rx, history, binary_snapshots).await {
                    Err(()) => return,
                    Ok(true) => {
                        if let Some(c) = credit.as_mut() {
                            c.superseded();
                        }
                    }
                    Ok(false) => {}
                }
            }
            // Forced disconnect: the session was terminated (admin terminate or
            // share-link revoke). Tell the client and close cleanly. `Lagged`
            // also resolves here (signal was sent) → evict; only `Closed`
            // (handled in next_evict) is a non-event that stops this branch.
            _ = next_evict(&mut evict_rx) => {
                let frame = r#"{"type":"terminated"}"#;
                let _ = socket.send(Message::Text(frame.into())).await;
                return;
            }
            // Client control frames.
            // NOTE: no auth work here. `can_input` is whatever the attach
            // decided, narrowed by the re-auth task's watch above — a keystroke
            // must never wait on the state DB (investigation H2).
            // Deferred input frames (budget full): wait for the writer to free
            // room for the oldest, then queue it and whatever fits behind it.
            // Every other arm keeps running, including reading client frames
            // (below) — `ack`s must land or output stalls once credit runs out
            // (perf3 R6). Only past INPUT_DEFER_BYTES does reading pause.
            _ = InputQueue::reserve(
                input_q.budget.clone(),
                deferred.front_cost().unwrap_or(1),
            ), if deferred.front_cost().is_some() => {
                deferred.release(&input_q);
            }

            msg = socket.recv(), if deferred.reading() => {
                let Some(Ok(msg)) = msg else { return };
                let Message::Text(text) = msg else {
                    if matches!(msg, Message::Close(_)) { return; }
                    continue;
                };
                let Ok(frame) = serde_json::from_str::<ClientFrame>(text.as_str()) else {
                    continue;
                };
                match frame {
                    ClientFrame::Input { data, user } => {
                        if !can_input {
                            if !warned_forbidden {
                                warned_forbidden = true;
                                let err = r#"{"type":"error","code":"forbidden","message":"viewers cannot send input"}"#;
                                if socket.send(Message::Text(err.into())).await.is_err() {
                                    return;
                                }
                            }
                            continue;
                        }
                        if let Ok(bytes) = B64.decode(data.as_bytes()) {
                            // The process this viewer shows is gone: never type
                            // blind into whatever replaced it. Swap onto the
                            // respawn first (the user then sees where the
                            // keystroke lands); with nothing live, `input`
                            // fails and the notice below says so.
                            if exit_rx.is_none() {
                                match revive_viewer(&ctx, &session_id, &mut socket, &mut handle, &mut out_rx, &mut exit_rx, history, binary_snapshots).await {
                                    Err(()) => return,
                                    Ok(true) => {
                                        if let Some(c) = credit.as_mut() {
                                            c.superseded();
                                        }
                                    }
                                    Ok(false) => {}
                                }
                            }
                            // A view-only viewer typed into a dormant session:
                            // this keystroke is the explicit "resume". Wake it
                            // and move onto the new process; the keystroke
                            // itself is NOT delivered (the CLI is still
                            // starting — a stray key or Enter would land in
                            // its startup, not in the prompt the user saw).
                            if wakes_on_input(view_only, user, exit_rx.is_some()) {
                                match ctx.manager().ensure_live(&session_id).await {
                                    Ok(()) => match revive_viewer(&ctx, &session_id, &mut socket, &mut handle, &mut out_rx, &mut exit_rx, history, binary_snapshots).await {
                                        Err(()) => return,
                                        Ok(true) => {
                                            warned_input = false;
                                            if let Some(c) = credit.as_mut() {
                                                c.superseded();
                                            }
                                            continue;
                                        }
                                        // Not resumable: fall through, `input`
                                        // fails and the notice says so.
                                        Ok(false) => {}
                                    },
                                    Err(e) => tracing::warn!(session = %session_id, "ensure_live on view-only wake: {e}"),
                                }
                            }
                            // Typed while more than a window behind (^C mid-
                            // flood): the held backlog becomes one snapshot.
                            if let Some(c) = credit.as_mut() {
                                c.skip_on_input(user, tokio::time::Instant::now());
                            }
                            // Delivery runs on this connection's input task
                            // (perf 01 F10): a large paste into a TUI that is
                            // slow to read its tty no longer freezes this
                            // loop's output, acks and resizes. Order is kept
                            // (one queue); results come back on `input_res_rx`.
                            // Typing claims size authority NOW (perf 01 N7),
                            // not when the write lands: a `resize` right behind
                            // the first keystroke must see this viewer as owner.
                            // A failed write reverts it.
                            let prev_owner = if user {
                                ctx.manager().claim_input_authority(&session_id, conn_id)
                            } else {
                                None
                            };
                            // Never dropped (N7): a frame that does not fit the
                            // byte budget (or arrives behind one that did not)
                            // is deferred in order until the writer frees room.
                            deferred.push(&input_q, bytes, user, prev_owner);
                        }
                    }
                    ClientFrame::Resize { cols, rows } => {
                        if can_input && ctx.manager().may_resize(&session_id, conn_id) {
                            let _ = ctx.manager().human_resize(&session_id, &input_user, input_scoped, cols, rows).await;
                        } else if can_input {
                            // Forensic trail for the half-width bug class: a
                            // denied resize is a viewer that WOULD have re-pinned
                            // the grid under the old policy.
                            tracing::info!(
                                session = %session_id,
                                conn = conn_id,
                                "terminal resize denied (not size owner): {cols}x{rows}"
                            );
                        }
                    }
                    ClientFrame::Claim => {
                        if can_input {
                            let _ = ctx.manager().human_claim_size(&session_id, &input_user, input_scoped, conn_id).await;
                        }
                    }
                    // Flow control is per-viewer and read-only safe: it only
                    // gates what THIS socket is sent, never the PTY.
                    ClientFrame::Pause => flow.pause(tokio::time::Instant::now()),
                    ClientFrame::Resume => {
                        if flow.resume() {
                            if let (Some(rx), Some(h)) = (out_rx.as_mut(), handle.as_ref()) {
                                if let Some(frame) = resume_frame(rx, pty_capture(h, history), binary_snapshots).await {
                                    if frame.send(&mut socket).await.is_err() {
                                        return;
                                    }
                                    if let Some(c) = credit.as_mut() {
                                        c.superseded();
                                    }
                                }
                            }
                        }
                    }
                    ClientFrame::Resync { lines, cols, rows } => {
                        let want = requested_history(lines);
                        history = want;
                        if let (Some(c), Some(r)) = (cols, rows) {
                            if can_input && ctx.manager().may_resize(&session_id, conn_id) {
                                let _ = ctx.manager().human_resize(&session_id, &input_user, input_scoped, c, r).await;
                            }
                        }
                        if let (Some(rx), Some(h)) = (out_rx.as_mut(), handle.as_ref()) {
                            let frame = client_resync_frame(&mut flow, rx, pty_capture(h, want), binary_snapshots).await;
                            if frame.send(&mut socket).await.is_err() {
                                return;
                            }
                            if let Some(c) = credit.as_mut() {
                                c.superseded();
                            }
                        } else {
                            flow.resume();
                        }
                    }
                    ClientFrame::Credit { window, binary_snapshots: bin } => {
                        binary_snapshots = bin;
                        let gate = CreditGate::new(window);
                        if socket.send(Message::Text(gate.grant_frame().into())).await.is_err() {
                            return;
                        }
                        credit = Some(gate);
                    }
                    ClientFrame::Ack { bytes } => {
                        if let Some(c) = credit.as_mut() {
                            let step = c.ack(bytes, tokio::time::Instant::now());
                            if apply_credit_step(step, &mut socket, &mut out_rx, handle.as_ref(), history, binary_snapshots).await.is_err() {
                                return;
                            }
                        }
                    }
                    ClientFrame::Scrollback { lines, cols, rows } => {
                        // Reproduce the live screen as one coherent frame (what
                        // tmux does on attach) PRECEDED by up to `lines` rows of
                        // scrollback history, so reconnecting restores history
                        // above the viewport instead of losing it. A `lines` of
                        // 0 falls back to the full retained depth.
                        let want = requested_history(lines);
                        history = want;
                        // Attach-with-grid (perf 01 F1): reflow to the client's
                        // grid FIRST, so this one snapshot is already at its
                        // width (same size-authority rules as `resize`; a
                        // same-size grid is a no-op end to end).
                        if let (Some(c), Some(r)) = (cols, rows) {
                            if can_input && ctx.manager().may_resize(&session_id, conn_id) {
                                let _ = ctx.manager().human_resize(&session_id, &input_user, input_scoped, c, r).await;
                            }
                        }
                        // `epoch` = PTY spawn counter. When the process was
                        // respawned since the client's last attach (suspend →
                        // resume, restart, daemon restart) the client's local
                        // scrollback belongs to the DEAD process — it resets and
                        // rebuilds from this snapshot instead of appending the
                        // fresh screen under stale (possibly narrow-painted)
                        // history. Omitted (0) when no live handle exists.
                        // Built off the async worker (r3-06-02).
                        //
                        // Like `resync`, the snapshot and a NEW subscription
                        // are taken under one emulator lock and swapped in for
                        // `out_rx`: every chunk already queued on the old
                        // receiver is in the snapshot, so streaming it after
                        // the snapshot double-applied output (duplicated lines
                        // on every attach/reattach).
                        let frame = match (out_rx.as_mut(), handle.as_ref()) {
                            (Some(rx), Some(h)) => resync_frame(rx, pty_capture(h, want), binary_snapshots).await,
                            (None, Some(h)) => snapshot_frame(h, want, binary_snapshots).await,
                            (_, None) => Snap::build(Vec::new(), 0, false),
                        };
                        // Sent inline, i.e. before any subsequent live bytes.
                        if frame.send(&mut socket).await.is_err() {
                            return;
                        }
                        // Output held for credit is already in this snapshot.
                        if let Some(c) = credit.as_mut() {
                            c.superseded();
                        }
                    }
                    ClientFrame::Probe { id } => {
                        let frame = probe_ack_frame(id, handle.as_ref().map(|h| h.echo_stats()));
                        if socket.send(Message::Text(frame.into())).await.is_err() {
                            return;
                        }
                    }
                    ClientFrame::Search { query } => {
                        // Server-side search: grep the ring buffer (survives WS
                        // reconnects, unlike the xterm SearchAddon which only
                        // sees the current emulator viewport). Skip empty queries
                        // to avoid spamming the socket with all-lines results.
                        if query.trim().is_empty() {
                            continue;
                        }
                        // The scan runs on the blocking pool (perf 01 F3):
                        // up to 10k ring lines are copied under a brief lock
                        // and searched after it is released, so neither this
                        // worker nor the session's PTY reader waits on it.
                        let matches = match handle.as_ref() {
                            Some(h) => {
                                let lines = h.search_lines();
                                let q = query.clone();
                                off_worker(move || otto_pty::ring::search_lines(&lines, &q, MAX_SEARCH_RESULTS))
                                    .await
                                    .unwrap_or_default()
                            }
                            None => Vec::new(),
                        };
                        let frame = search_result_frame(&query, matches);
                        if socket.send(Message::Text(frame.into())).await.is_err() {
                            return;
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! In-crate gate tests (mobile plan Task 1.6). These drive the private
    //! [`ws_auth_gate`] middleware directly through a probe handler that echoes
    //! the [`CanInput`] extension into a response header, so the read-only vs
    //! read-write capability a scoped share confers is observable WITHOUT a real
    //! WebSocket upgrade (which the integration harness in `tests/isolation.rs`
    //! cannot inspect — it only sees the pre-upgrade status).

    use super::*;
    use crate::{ProviderRegistry, SessionManager};
    use chrono::Utc;
    use otto_core::auth::RoleChecker;
    use otto_core::domain::{SessionKind, WorkspaceRole};
    use otto_rbac::{tokens::AuthRepo, RbacAuthenticator, RbacRoleChecker};
    use otto_state::{SessionsRepo, SqlitePool, WorkspacesRepo};
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::sync::Arc;
    use tokio::sync::broadcast;
    use tower::ServiceExt; // for `oneshot`

    #[derive(Clone)]
    struct Ctx {
        manager: Arc<SessionManager>,
        roles: Arc<dyn RoleChecker>,
        workspaces: WorkspacesRepo,
    }

    impl SessionsCtx for Ctx {
        fn manager(&self) -> &Arc<SessionManager> {
            &self.manager
        }
        fn roles(&self) -> &Arc<dyn RoleChecker> {
            &self.roles
        }
        fn workspaces(&self) -> &WorkspacesRepo {
            &self.workspaces
        }
    }

    async fn mem_pool() -> SqlitePool {
        let opts = SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .expect("connect in-memory sqlite");
        sqlx::migrate!("../otto-state/migrations")
            .run(&pool)
            .await
            .expect("run migrations");
        pool
    }

    async fn seed_user(pool: &SqlitePool, id: &str) {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, display_name, is_root, created_at)
             VALUES (?, ?, 'x', ?, 0, ?)",
        )
        .bind(id)
        .bind(id)
        .bind(id)
        .bind(&now)
        .execute(pool)
        .await
        .expect("seed user");
    }

    async fn seed_workspace(pool: &SqlitePool, ws_id: &str) {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO workspaces (id, name, root_path, settings_json, archived, created_at)
             VALUES (?, 'ws', '/tmp', '{}', 0, ?)",
        )
        .bind(ws_id)
        .bind(&now)
        .execute(pool)
        .await
        .expect("seed workspace");
    }

    async fn insert_session(repo: &SessionsRepo, ws: &str, created_by: &str) -> Id {
        repo.create(otto_state::NewSession {
            workspace_id: ws.into(),
            kind: SessionKind::Agent,
            provider: "shell".into(),
            title: "t".into(),
            cwd: "/tmp".into(),
            provider_session_id: None,
            connection_id: None,
            created_by: created_by.into(),
            meta: serde_json::Value::Null,
        })
        .await
        .expect("insert session")
        .id
    }

    /// Build a router that layers the REAL [`ws_auth_gate`] over a probe handler
    /// echoing the gate-set [`CanInput`] and [`UsedSubprotocol`] flags into
    /// response headers. This lets the test read the exact capability decision
    /// and subprotocol detection pre-upgrade (without a real WS upgrade).
    fn probe_app(state: WsState<Ctx>) -> Router {
        async fn probe(
            axum::Extension(CanInput(can_input)): axum::Extension<CanInput>,
            axum::Extension(UsedSubprotocol(used_subprotocol)): axum::Extension<UsedSubprotocol>,
        ) -> Response {
            let mut resp = StatusCode::OK.into_response();
            resp.headers_mut().insert(
                "x-can-input",
                axum::http::HeaderValue::from_static(if can_input { "1" } else { "0" }),
            );
            resp.headers_mut().insert(
                "x-used-subprotocol",
                axum::http::HeaderValue::from_static(if used_subprotocol { "1" } else { "0" }),
            );
            resp
        }
        Router::new()
            .route("/ws/term/{session_id}", get(probe))
            .route_layer(axum::middleware::from_fn_with_state(
                state.clone(),
                ws_auth_gate::<Ctx>,
            ))
            .with_state(state)
    }

    async fn build(pool: &SqlitePool) -> WsState<Ctx> {
        let repo = SessionsRepo::new(pool.clone());
        let (events, _rx) = broadcast::channel(64);
        let providers = ProviderRegistry::new(None);
        let manager = Arc::new(SessionManager::new(repo, events, providers));
        let ctx = Ctx {
            manager,
            roles: Arc::new(RbacRoleChecker::new(pool.clone())),
            workspaces: WorkspacesRepo::new(pool.clone()),
        };
        WsState {
            auth: Arc::new(RbacAuthenticator::new(pool.clone())),
            ctx,
        }
    }

    async fn mint_share(pool: &SqlitePool, owner: &str, sid: &Id, role: WorkspaceRole) -> String {
        AuthRepo::new(pool.clone())
            .issue_share_token(&owner.into(), sid, role, 3600, None)
            .await
            .expect("issue share")
            .0
    }

    /// Drive the gate via the legacy `?token=` query param.
    async fn gate(app: &Router, sid: &Id, token: &str) -> Response {
        let req = Request::builder()
            .method("GET")
            .uri(format!("/ws/term/{sid}?token={token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        app.clone().oneshot(req).await.unwrap()
    }

    /// Drive the gate via the `Sec-WebSocket-Protocol: otto-bearer, <token>` header.
    async fn gate_subprotocol(app: &Router, sid: &Id, token: &str) -> Response {
        let req = Request::builder()
            .method("GET")
            .uri(format!("/ws/term/{sid}"))
            .header(
                axum::http::header::SEC_WEBSOCKET_PROTOCOL,
                format!("otto-bearer, {token}"),
            )
            .body(axum::body::Body::empty())
            .unwrap();
        app.clone().oneshot(req).await.unwrap()
    }

    /// A **viewer** share attaches to its session but is read-only: the gate sets
    /// `CanInput = false`.
    #[tokio::test]
    async fn viewer_share_is_read_only() {
        let pool = mem_pool().await;
        seed_user(&pool, "alice").await;
        seed_workspace(&pool, "ws1").await;
        let s1 = insert_session(&SessionsRepo::new(pool.clone()), "ws1", "alice").await;
        let app = probe_app(build(&pool).await);

        let token = mint_share(&pool, "alice", &s1, WorkspaceRole::Viewer).await;
        let resp = gate(&app, &s1, &token).await;
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "viewer share must pass the gate"
        );
        assert_eq!(
            resp.headers().get("x-can-input").unwrap(),
            "0",
            "a viewer share must be read-only (CanInput=false)"
        );
    }

    /// An **editor** share attaches AND may type/resize: `CanInput = true`.
    #[tokio::test]
    async fn editor_share_can_input() {
        let pool = mem_pool().await;
        seed_user(&pool, "alice").await;
        seed_workspace(&pool, "ws1").await;
        let s1 = insert_session(&SessionsRepo::new(pool.clone()), "ws1", "alice").await;
        let app = probe_app(build(&pool).await);

        let token = mint_share(&pool, "alice", &s1, WorkspaceRole::Editor).await;
        let resp = gate(&app, &s1, &token).await;
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "editor share must pass the gate"
        );
        assert_eq!(
            resp.headers().get("x-can-input").unwrap(),
            "1",
            "an editor share may input (CanInput=true)"
        );
    }

    /// A share scoped to S1 is REFUSED (403) on S2 — even though the same owner
    /// created both — and never reaches the probe handler.
    #[tokio::test]
    async fn share_for_s1_denied_on_s2() {
        let pool = mem_pool().await;
        seed_user(&pool, "alice").await;
        seed_workspace(&pool, "ws1").await;
        let repo = SessionsRepo::new(pool.clone());
        let s1 = insert_session(&repo, "ws1", "alice").await;
        let s2 = insert_session(&repo, "ws1", "alice").await;
        let app = probe_app(build(&pool).await);

        let token = mint_share(&pool, "alice", &s1, WorkspaceRole::Viewer).await;
        assert_eq!(
            gate(&app, &s1, &token).await.status(),
            StatusCode::OK,
            "share must pass on its pinned session S1"
        );
        assert_eq!(
            gate(&app, &s2, &token).await.status(),
            StatusCode::FORBIDDEN,
            "a share scoped to S1 must be 403 on S2"
        );
    }

    /// A normal (unscoped) owner token still passes the owner gate and gets the
    /// Editor-probe capability — unchanged behaviour for non-scoped tokens.
    #[tokio::test]
    async fn unscoped_owner_token_unchanged() {
        let pool = mem_pool().await;
        seed_user(&pool, "alice").await;
        seed_workspace(&pool, "ws1").await;
        sqlx::query("INSERT INTO workspace_members (workspace_id, user_id, role) VALUES ('ws1','alice','editor')")
            .execute(&pool)
            .await
            .expect("set member");
        let s1 = insert_session(&SessionsRepo::new(pool.clone()), "ws1", "alice").await;
        let app = probe_app(build(&pool).await);

        let token = AuthRepo::new(pool.clone())
            .issue(&"alice".into())
            .await
            .expect("issue token");
        let resp = gate(&app, &s1, &token).await;
        assert_eq!(resp.status(), StatusCode::OK, "owner must pass the gate");
        assert_eq!(
            resp.headers().get("x-can-input").unwrap(),
            "1",
            "owner-editor keeps input capability (unscoped path unchanged)"
        );
    }

    // ---- Agent-credential confinement on /ws/term ---------------------------

    async fn seed_editor(pool: &SqlitePool, user: &str) {
        sqlx::query("INSERT INTO workspace_members (workspace_id, user_id, role) VALUES ('ws1', ?, 'editor')")
            .bind(user)
            .execute(pool)
            .await
            .expect("set member");
    }

    /// An agent session's own API token may NOT attach to another terminal of
    /// its owner (it would type into it): the WS upgrade is a GET, so the
    /// `/api/v1` read-only guard never saw it — the gate must refuse it.
    #[tokio::test]
    async fn agent_session_token_refused_on_other_session() {
        let pool = mem_pool().await;
        seed_user(&pool, "alice").await;
        seed_workspace(&pool, "ws1").await;
        seed_editor(&pool, "alice").await;
        let repo = SessionsRepo::new(pool.clone());
        let agent = insert_session(&repo, "ws1", "alice").await;
        let other = insert_session(&repo, "ws1", "alice").await;
        let app = probe_app(build(&pool).await);

        let (token, _) = AuthRepo::new(pool.clone())
            .issue_session_api_token(&"alice".into(), &agent)
            .await
            .expect("issue session token");
        assert_eq!(
            gate(&app, &other, &token).await.status(),
            StatusCode::FORBIDDEN,
            "a managed-session token must be 403 on a sibling session"
        );
        let own = gate(&app, &agent, &token).await;
        assert_eq!(own.status(), StatusCode::OK, "own session still attaches");
        assert_eq!(own.headers().get("x-can-input").unwrap(), "1");
    }

    /// A read-only agent session attaching to its own terminal is view-only.
    #[tokio::test]
    async fn read_only_agent_session_cannot_input() {
        let pool = mem_pool().await;
        seed_user(&pool, "alice").await;
        seed_workspace(&pool, "ws1").await;
        seed_editor(&pool, "alice").await;
        let repo = SessionsRepo::new(pool.clone());
        let agent = repo
            .create(otto_state::NewSession {
                workspace_id: "ws1".into(),
                kind: SessionKind::Agent,
                provider: "shell".into(),
                title: "t".into(),
                cwd: "/tmp".into(),
                provider_session_id: None,
                connection_id: None,
                created_by: "alice".into(),
                meta: serde_json::json!({ "read_only": true }),
            })
            .await
            .expect("insert session")
            .id;
        let app = probe_app(build(&pool).await);
        let (token, _) = AuthRepo::new(pool.clone())
            .issue_session_api_token(&"alice".into(), &agent)
            .await
            .expect("issue session token");
        let resp = gate(&app, &agent, &token).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get("x-can-input").unwrap(),
            "0",
            "a read-only session must never get input on the terminal socket"
        );
    }

    /// An MCP-restricted token never attaches to a terminal (its only routes
    /// are the governed MCP endpoints).
    #[tokio::test]
    async fn mcp_token_refused_on_terminal() {
        let pool = mem_pool().await;
        seed_user(&pool, "alice").await;
        seed_workspace(&pool, "ws1").await;
        seed_editor(&pool, "alice").await;
        let s1 = insert_session(&SessionsRepo::new(pool.clone()), "ws1", "alice").await;
        let app = probe_app(build(&pool).await);
        let token = AuthRepo::new(pool.clone())
            .issue_mcp_token(&"alice".into(), None)
            .await
            .expect("issue mcp token");
        assert_eq!(
            gate(&app, &s1, &token).await.status(),
            StatusCode::FORBIDDEN
        );
    }

    // ---- Task 1.10: otto-bearer subprotocol on /ws/term -------------------

    /// A share token presented via `Sec-WebSocket-Protocol: otto-bearer, <token>`
    /// is accepted: the gate sets 200 and marks `UsedSubprotocol = true`.
    #[tokio::test]
    async fn subprotocol_token_accepted() {
        let pool = mem_pool().await;
        seed_user(&pool, "alice").await;
        seed_workspace(&pool, "ws1").await;
        let s1 = insert_session(&SessionsRepo::new(pool.clone()), "ws1", "alice").await;
        let app = probe_app(build(&pool).await);

        let token = mint_share(&pool, "alice", &s1, WorkspaceRole::Viewer).await;
        let resp = gate_subprotocol(&app, &s1, &token).await;
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "subprotocol token must pass the gate"
        );
        assert_eq!(
            resp.headers().get("x-used-subprotocol").unwrap(),
            "1",
            "gate must detect the subprotocol path"
        );
    }

    /// A bad token via subprotocol is rejected with 401.
    #[tokio::test]
    async fn bad_subprotocol_token_rejected() {
        let pool = mem_pool().await;
        seed_user(&pool, "alice").await;
        seed_workspace(&pool, "ws1").await;
        let s1 = insert_session(&SessionsRepo::new(pool.clone()), "ws1", "alice").await;
        let app = probe_app(build(&pool).await);

        let resp = gate_subprotocol(&app, &s1, "not-a-real-token").await;
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "bad subprotocol token must return 401"
        );
    }

    /// Legacy `?token=` is still accepted and `UsedSubprotocol` is false.
    #[tokio::test]
    async fn query_param_token_marks_no_subprotocol() {
        let pool = mem_pool().await;
        seed_user(&pool, "alice").await;
        seed_workspace(&pool, "ws1").await;
        let s1 = insert_session(&SessionsRepo::new(pool.clone()), "ws1", "alice").await;
        let app = probe_app(build(&pool).await);

        let token = mint_share(&pool, "alice", &s1, WorkspaceRole::Viewer).await;
        let resp = gate(&app, &s1, &token).await;
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "?token= must still work (backward compat)"
        );
        assert_eq!(
            resp.headers().get("x-used-subprotocol").unwrap(),
            "0",
            "legacy ?token= path must NOT set UsedSubprotocol"
        );
    }

    // ---- Task 1.8: share redemption rate limiter --------------------------

    /// The `ShareThrottle` unit tests live in `share_throttle.rs`. Here we
    /// assert that the gate wiring is correct: a request carrying an invalid
    /// token from an IP that has been pre-locked returns 429 (not 401), and a
    /// valid token still works when the IP is not locked.
    #[tokio::test]
    async fn rate_limit_blocks_locked_ip() {
        use crate::share_throttle::{ShareThrottle, FAILURE_THRESHOLD};
        use axum::extract::ConnectInfo;
        use std::net::{IpAddr, Ipv4Addr, SocketAddr};

        let pool = mem_pool().await;
        seed_user(&pool, "alice").await;
        seed_workspace(&pool, "ws1").await;
        let s1 = insert_session(&SessionsRepo::new(pool.clone()), "ws1", "alice").await;
        let app = probe_app(build(&pool).await);

        // Use an isolated throttle instance to pre-lock an IP.
        let throttle = ShareThrottle::default();
        let loopback = IpAddr::V4(Ipv4Addr::new(127, 42, 42, 1));
        for _ in 0..FAILURE_THRESHOLD {
            throttle.record_failure(loopback);
        }
        assert!(throttle.check(loopback).is_err(), "IP should be locked");

        // The global throttle (used by the gate) is separate; verify the gate
        // itself does NOT lock a clean IP and still accepts a valid token.
        let token = mint_share(&pool, "alice", &s1, WorkspaceRole::Viewer).await;
        let req = Request::builder()
            .method("GET")
            .uri(format!("/ws/term/{s1}?token={token}"))
            // Inject a clean ConnectInfo — the loopback already has N failures
            // in the isolated `throttle` above, but the GLOBAL store is clean.
            .extension(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 12345))))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "a valid token from a clean IP must still pass through the global throttle"
        );
    }

    /// S8-02 / S1-05: once an IP is locked by junk tokens, a token that
    /// VERIFIES still attaches (the lock only refuses further failures), and
    /// the tunnel-aware [`share_throttle::ClientIp`] keys tunnelled clients
    /// apart so one visitor's junk never locks another.
    #[tokio::test]
    async fn locked_ip_still_admits_a_valid_token() {
        use crate::share_throttle::{ClientIp, FAILURE_THRESHOLD};
        use std::net::{IpAddr, Ipv4Addr};

        let pool = mem_pool().await;
        seed_user(&pool, "alice").await;
        seed_workspace(&pool, "ws1").await;
        let s1 = insert_session(&SessionsRepo::new(pool.clone()), "ws1", "alice").await;
        let app = probe_app(build(&pool).await);
        // Unique per-test addresses: the gate uses the process-global throttle.
        let attacker = IpAddr::V4(Ipv4Addr::new(198, 18, 77, 1));
        let other = IpAddr::V4(Ipv4Addr::new(198, 18, 77, 2));
        let req = |token: &str, ip: IpAddr| {
            Request::builder()
                .method("GET")
                .uri(format!("/ws/term/{s1}?token={token}"))
                .extension(ClientIp { ip, local: false })
                .body(axum::body::Body::empty())
                .unwrap()
        };

        for _ in 0..FAILURE_THRESHOLD {
            let resp = app.clone().oneshot(req("junk", attacker)).await.unwrap();
            assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        }
        // Locked: further junk is 429 …
        let resp = app.clone().oneshot(req("junk", attacker)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
        // … but a valid token from the SAME address still attaches.
        let token = mint_share(&pool, "alice", &s1, WorkspaceRole::Viewer).await;
        let resp = app.clone().oneshot(req(&token, attacker)).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "a token that verifies must never be refused by the lockout"
        );
        // A different (tunnel-resolved) client is unaffected by the lock.
        let resp = app.clone().oneshot(req("junk", other)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // ---- Task 7.3: email-OTP gate before attach ---------------------------

    /// An OTP-gated share that has NOT been verified is refused at the gate
    /// (403, no upgrade) — the email-OTP second factor must be passed first.
    /// After redeeming the OTP it passes the gate.
    #[tokio::test]
    async fn otp_pending_share_denied_until_verified() {
        let pool = mem_pool().await;
        seed_user(&pool, "alice").await;
        seed_workspace(&pool, "ws1").await;
        let s1 = insert_session(&SessionsRepo::new(pool.clone()), "ws1", "alice").await;
        let app = probe_app(build(&pool).await);

        // Mint an OTP-gated share (recipient locked). It is OTP-pending.
        let repo = AuthRepo::new(pool.clone());
        let (token, otp, _info) = repo
            .issue_share_otp_token(
                &"alice".into(),
                &s1,
                WorkspaceRole::Viewer,
                3600,
                None,
                "guest@example.com",
            )
            .await
            .expect("issue otp share");

        // Pre-verify: the gate refuses the attach with 403 (no socket).
        let resp = gate(&app, &s1, &token).await;
        assert_eq!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "an OTP-pending share must be denied at the WS gate"
        );

        // Redeem the code, then the gate lets the attach through.
        assert!(repo.verify_share_otp(&token, &otp).await.unwrap());
        let resp = gate(&app, &s1, &token).await;
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "after OTP verification the share may attach"
        );
        assert_eq!(
            resp.headers().get("x-can-input").unwrap(),
            "0",
            "a viewer OTP share is still read-only"
        );
    }

    /// A request with no token at all is 401 (no change to throttle behaviour
    /// — we only rate-limit actual token attempts, not missing-token probes).
    #[tokio::test]
    async fn no_token_is_401() {
        let pool = mem_pool().await;
        seed_user(&pool, "alice").await;
        seed_workspace(&pool, "ws1").await;
        let s1 = insert_session(&SessionsRepo::new(pool.clone()), "ws1", "alice").await;
        let app = probe_app(build(&pool).await);

        let req = Request::builder()
            .method("GET")
            .uri(format!("/ws/term/{s1}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    /// A share token revoked MID-CONNECTION still evicts that viewer promptly.
    ///
    /// The re-auth used to run inline in the socket's `select!` loop (ahead of
    /// every client frame); it now runs in [`reauth_loop`] and speaks to the
    /// loop through a `watch`. Revocation is signalled by the task RETURNING —
    /// the sender drops, the watch closes, and the loop sends `Close`. This
    /// test drives that task directly with a short period, so "within one tick"
    /// is measured, not assumed. (Share tokens are never auth-cached, so the
    /// very next pass sees the revocation.)
    #[tokio::test]
    async fn revoked_share_is_evicted_within_a_tick() {
        let pool = mem_pool().await;
        seed_user(&pool, "alice").await;
        seed_workspace(&pool, "ws1").await;
        let sid = insert_session(&SessionsRepo::new(pool.clone()), "ws1", "alice").await;
        let before = SessionsRepo::new(pool.clone())
            .get(&sid)
            .await
            .expect("session row");
        let st = build(&pool).await;
        let token = mint_share(&pool, "alice", &sid, WorkspaceRole::Editor).await;
        let user = st
            .auth
            .authenticate(&token)
            .await
            .expect("share token authenticates")
            .effective_user;
        let live_auth = LiveTerminalAuth {
            scoped: true,
            user,
            token: token.clone(),
            auth: st.auth.clone(),
        };

        let (can_tx, mut can_rx) = watch::channel(true);
        tokio::spawn(reauth_loop(
            st.ctx.clone(),
            sid.clone(),
            live_auth,
            can_tx,
            Duration::from_millis(25),
        ));

        // While the share is valid: several ticks pass, the verdict never moves
        // (an editor share keeps input) and the watch stays open.
        assert!(
            tokio::time::timeout(Duration::from_millis(200), next_can_input(&mut can_rx))
                .await
                .is_err(),
            "a valid editor share must not change the capability"
        );
        assert!(*can_rx.borrow(), "editor share still has input");

        AuthRepo::new(pool.clone())
            .revoke(&token)
            .await
            .expect("revoke the share");

        // One tick later the task is gone and the loop learns it must evict.
        let verdict = tokio::time::timeout(Duration::from_secs(2), next_can_input(&mut can_rx))
            .await
            .expect("the revoked viewer must be evicted within a tick");
        assert_eq!(
            verdict, None,
            "a revoked share closes the capability watch (= evict this socket)"
        );

        // S1-01: evicting the guest must NOT kill the owner's session. A share
        // token resolves to the owner, so the old "revoked user created it ⇒
        // kill" rule took the owner's live agent down with the guest.
        tokio::time::sleep(Duration::from_millis(100)).await;
        let after = SessionsRepo::new(pool.clone())
            .get(&sid)
            .await
            .expect("session row survives the eviction");
        assert_eq!(
            after.status, before.status,
            "re-auth eviction must leave the session status untouched"
        );
        assert!(st.ctx.manager().get(&sid).await.is_ok());
    }

    /// Perf 01 F8: sockets presenting the same token to the same session
    /// share ONE re-auth pass; it stops once the last socket leaves, a later
    /// socket starts a fresh one, and a revocation still evicts every
    /// subscriber within a tick.
    #[tokio::test]
    async fn sockets_share_one_reauth_pass_per_token_and_session() {
        let pool = mem_pool().await;
        seed_user(&pool, "alice").await;
        seed_workspace(&pool, "ws1").await;
        let sid = insert_session(&SessionsRepo::new(pool.clone()), "ws1", "alice").await;
        let st = build(&pool).await;
        let token = mint_share(&pool, "alice", &sid, WorkspaceRole::Editor).await;
        let user = st
            .auth
            .authenticate(&token)
            .await
            .expect("share token authenticates")
            .effective_user;
        let live_auth = LiveTerminalAuth {
            scoped: true,
            user,
            token: token.clone(),
            auth: st.auth.clone(),
        };
        let period = Duration::from_millis(25);
        let key: ReauthKey = (sid.clone(), token.clone(), period);
        let entries = || SHARED_REAUTH.iter().filter(|e| *e.key() == key).count();

        let a = shared_reauth(&st.ctx, &sid, live_auth.clone(), period);
        let b = shared_reauth(&st.ctx, &sid, live_auth.clone(), period);
        assert_eq!(entries(), 1, "one pass for both sockets");
        assert_eq!(SHARED_REAUTH.get(&key).unwrap().1.receiver_count(), 2);
        drop(a);
        drop(b);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while entries() > 0 {
            assert!(
                tokio::time::Instant::now() < deadline,
                "the unwatched pass never stopped"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        let mut c = shared_reauth(&st.ctx, &sid, live_auth.clone(), period);
        let mut d = shared_reauth(&st.ctx, &sid, live_auth, period);
        assert!(*c.borrow_and_update() && *d.borrow_and_update());
        AuthRepo::new(pool.clone())
            .revoke(&token)
            .await
            .expect("revoke the share");
        for rx in [&mut c, &mut d] {
            let verdict = tokio::time::timeout(Duration::from_secs(2), next_can_input(rx))
                .await
                .expect("every subscriber is evicted within a tick");
            assert_eq!(verdict, None);
        }
        assert_eq!(entries(), 0, "a revoked pass leaves no registry entry");
    }

    // ── Flow control + search_result framing (SA-02 / SA-11) ──────────────

    /// SA-11: a matched line with a TAB (make output, Go/Java stack traces)
    /// or any other C0 byte used to produce invalid JSON, so the client's
    /// `JSON.parse` threw and the find bar spun forever.
    /// r3-05-01 follow-up: a view-only attach (`?view=1`) never resumes the
    /// CLI; the first REAL keystroke on it does, emulator replies never do, and
    /// a live process is never "woken" twice. Classic attaches keep resuming.
    #[test]
    fn view_only_attach_resumes_only_on_real_input() {
        let parse = |q: &str| {
            let uri: axum::http::Uri = format!("/ws/term/s{q}").parse().unwrap();
            Query::<AttachQuery>::try_from_uri(&uri)
                .unwrap()
                .0
                .view_only()
        };
        assert!(parse("?view=1"));
        assert!(parse("?token=abc&view=true"));
        assert!(!parse(""), "absent = classic attach");
        assert!(!parse("?token=abc"));
        assert!(!parse("?view=0"));

        assert!(resume_on_attach(true, false), "classic attach resumes");
        assert!(
            !resume_on_attach(true, true),
            "view-only attach never spawns"
        );
        assert!(
            !resume_on_attach(false, false),
            "read-only shares never spawn"
        );

        assert!(
            wakes_on_input(true, true, false),
            "typing wakes a dormant view"
        );
        assert!(
            !wakes_on_input(true, false, false),
            "DA/CPR replies never wake"
        );
        assert!(
            !wakes_on_input(true, true, true),
            "already live: plain input"
        );
        assert!(
            !wakes_on_input(false, true, false),
            "classic socket: unchanged"
        );
    }

    #[test]
    fn probe_frame_parses_and_ack_carries_echo_stats() {
        assert!(matches!(
            serde_json::from_str::<ClientFrame>(r#"{"type":"probe","id":7}"#),
            Ok(ClientFrame::Probe { id: 7 })
        ));
        assert!(matches!(
            serde_json::from_str::<ClientFrame>(r#"{"type":"probe"}"#),
            Ok(ClientFrame::Probe { id: 0 })
        ));
        let ack: serde_json::Value = serde_json::from_str(&probe_ack_frame(7, None)).unwrap();
        assert_eq!(ack["type"], "probe_ack");
        assert_eq!(ack["id"], 7);
        assert!(ack["echo"].is_null());
        let stats = otto_pty::EchoStats {
            last_ms: 1.5,
            avg_ms: 2.0,
            max_ms: 9.25,
            samples: 3,
        };
        let ack: serde_json::Value =
            serde_json::from_str(&probe_ack_frame(8, Some(stats))).unwrap();
        assert_eq!(ack["echo"]["last_ms"], 1.5);
        assert_eq!(ack["echo"]["max_ms"], 9.25);
        assert_eq!(ack["echo"]["samples"], 3);
    }

    #[test]
    fn search_result_frame_is_valid_json_for_tabs_and_control_bytes() {
        let text = "at\tmain.go:12\t\"quoted\" back\\slash \u{7} bell \u{8} bs \u{1b}esc\r\n";
        let query = "tab\t\"q\"";
        let frame = search_result_frame(query, vec![(42, text.to_string()), (7, "plain".into())]);
        let v: serde_json::Value =
            serde_json::from_str(&frame).expect("search_result must be valid JSON");
        assert_eq!(v["type"], "search_result");
        assert_eq!(v["query"], query);
        assert_eq!(v["matches"][0]["line"], 42);
        assert_eq!(v["matches"][0]["text"], text, "text round-trips byte-exact");
        assert_eq!(v["matches"][1]["line"], 7);
        assert_eq!(v["matches"][1]["text"], "plain");
        // Empty result set is still a well-formed frame.
        let empty: serde_json::Value =
            serde_json::from_str(&search_result_frame("x", Vec::new())).unwrap();
        assert_eq!(empty["matches"], serde_json::json!([]));
    }

    #[test]
    fn client_pause_resume_frames_parse() {
        assert!(matches!(
            serde_json::from_str::<ClientFrame>(r#"{"type":"pause"}"#),
            Ok(ClientFrame::Pause)
        ));
        assert!(matches!(
            serde_json::from_str::<ClientFrame>(r#"{"type":"resume"}"#),
            Ok(ClientFrame::Resume)
        ));
    }

    #[test]
    fn flow_gate_pause_arms_auto_resume_and_resume_reports_state() {
        let mut gate = FlowGate::default();
        assert!(!gate.is_paused());
        assert!(!gate.resume(), "resume while open is a no-op");
        let t0 = tokio::time::Instant::now();
        gate.pause(t0);
        assert!(gate.is_paused());
        assert_eq!(gate.paused_until, Some(t0 + FLOW_AUTO_RESUME));
        // A repeated pause (client keep-alive) re-arms the deadline.
        let t1 = t0 + Duration::from_millis(1500);
        gate.pause(t1);
        assert_eq!(gate.paused_until, Some(t1 + FLOW_AUTO_RESUME));
        assert!(gate.resume(), "resume reports it was paused");
        assert!(!gate.is_paused());
    }

    /// The auto-resume arm fires once the deadline passes (a client that
    /// never sends `resume` cannot wedge the pane).
    #[tokio::test]
    async fn flow_deadline_fires_and_open_gate_never_does() {
        let at = tokio::time::Instant::now() + Duration::from_millis(20);
        tokio::time::timeout(Duration::from_secs(2), flow_deadline(Some(at)))
            .await
            .expect("deadline must fire");
        assert!(
            tokio::time::timeout(Duration::from_millis(50), flow_deadline(None))
                .await
                .is_err(),
            "an open gate has no deadline"
        );
    }

    /// pause → output skipped → resume ⇒ ONE snapshot replaces the backlog,
    /// then live output flows again. Mirrors the socket loop: while paused the
    /// output arm is disabled, so chunks pile up in this viewer's receiver.
    /// A test capture: a fixed snapshot plus a fresh subscription on `tx`
    /// (the real one takes both under the emulator lock), counting calls.
    fn test_capture(
        tx: &broadcast::Sender<Bytes>,
        data: &'static [u8],
        epoch: u64,
        calls: &Arc<std::sync::atomic::AtomicUsize>,
    ) -> impl FnOnce() -> Capture + Send + 'static {
        let tx = tx.clone();
        let calls = Arc::clone(calls);
        move || {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            (data.to_vec(), epoch, tx.subscribe())
        }
    }

    #[tokio::test]
    async fn pause_skip_resume_sends_one_snapshot_then_streams() {
        let (tx, mut rx) = broadcast::channel::<Bytes>(8);
        let mut gate = FlowGate::default();
        gate.pause(tokio::time::Instant::now());
        // A flood while paused — more than the ring holds, so it also lags.
        for i in 0..20 {
            tx.send(Bytes::from(format!("y{i}\n"))).unwrap();
        }
        assert!(gate.resume());
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let frame = resume_frame(
            &mut rx,
            test_capture(&tx, b"\x1b[Hsnapshot", 3, &calls),
            false,
        )
        .await
        .expect("skipped output must be replaced by a snapshot");
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        let v: serde_json::Value = serde_json::from_str(frame.json()).unwrap();
        assert_eq!(v["type"], "scrollback");
        assert_eq!(v["epoch"], 3);
        assert_eq!(
            B64.decode(v["data"].as_str().unwrap()).unwrap(),
            b"\x1b[Hsnapshot"
        );
        // The backlog is gone — none of it is forwarded after the snapshot…
        assert!(matches!(
            rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
        // …and new output streams normally again.
        tx.send(Bytes::from_static(b"live")).unwrap();
        assert_eq!(rx.try_recv().unwrap(), Bytes::from_static(b"live"));
    }

    /// A client `resync` (typed while its queue was dropped) always gets
    /// exactly one snapshot — even with nothing queued server-side — opens a
    /// paused gate, and nothing that was queued before it is forwarded after
    /// (it is already in the snapshot). A later `resume` is then a no-op.
    #[tokio::test]
    async fn client_resync_opens_the_gate_and_replaces_the_backlog_with_one_snapshot() {
        assert!(matches!(
            serde_json::from_str::<ClientFrame>(r#"{"type":"resync","lines":2000}"#),
            Ok(ClientFrame::Resync {
                lines: 2000,
                cols: None,
                rows: None
            })
        ));
        assert!(matches!(
            serde_json::from_str::<ClientFrame>(r#"{"type":"resync"}"#),
            Ok(ClientFrame::Resync { lines: 0, .. })
        ));
        let (tx, mut rx) = broadcast::channel::<Bytes>(64);
        let mut gate = FlowGate::default();
        gate.pause(tokio::time::Instant::now());
        for i in 0..10 {
            tx.send(Bytes::from(format!("flood{i}\n"))).unwrap();
        }
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let frame = client_resync_frame(
            &mut gate,
            &mut rx,
            test_capture(&tx, b"screen", 9, &calls),
            false,
        )
        .await;
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(!gate.is_paused(), "resync leaves the paused state");
        assert!(!gate.resume(), "a trailing resume finds the gate open");
        let v: serde_json::Value = serde_json::from_str(frame.json()).unwrap();
        assert_eq!(v["type"], "scrollback");
        assert_eq!(v["epoch"], 9);
        assert!(matches!(
            rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
        // Nothing queued and not paused: still answers with a snapshot.
        let frame = client_resync_frame(
            &mut gate,
            &mut rx,
            test_capture(&tx, b"s2", 9, &calls),
            false,
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(frame.json()).unwrap();
        assert_eq!(B64.decode(v["data"].as_str().unwrap()).unwrap(), b"s2");
        tx.send(Bytes::from_static(b"live")).unwrap();
        assert_eq!(rx.try_recv().unwrap(), Bytes::from_static(b"live"));
    }

    /// Pausing with nothing produced meanwhile must NOT rebuild the client
    /// (a rebuild resets selection/scroll position for no reason).
    #[tokio::test]
    async fn resume_without_skipped_output_sends_nothing() {
        let (tx, mut rx) = broadcast::channel::<Bytes>(8);
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        assert!(
            resume_frame(&mut rx, test_capture(&tx, b"", 0, &calls), false)
                .await
                .is_none()
        );
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "no snapshot is taken when nothing was skipped"
        );
    }

    /// The resync hand-over is exact: output published after the capture
    /// arrives on the NEW receiver (never lost), and nothing queued before
    /// it is forwarded after the snapshot (never double-applied).
    #[tokio::test]
    async fn resync_swaps_to_the_capture_subscription() {
        let (tx, mut rx) = broadcast::channel::<Bytes>(64);
        tx.send(Bytes::from_static(b"before")).unwrap();
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let frame = resync_frame(&mut rx, test_capture(&tx, b"snap", 4, &calls), false).await;
        let v: serde_json::Value = serde_json::from_str(frame.json()).unwrap();
        assert_eq!(v["epoch"], 4);
        assert!(matches!(
            rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
        tx.send(Bytes::from_static(b"after")).unwrap();
        assert_eq!(rx.try_recv().unwrap(), Bytes::from_static(b"after"));
    }

    /// The attach `scrollback` reply against a REAL PTY: output that was
    /// queued on the viewer's attach-time subscription is already in the
    /// snapshot, so after the reply (built like `resync`) it must not arrive
    /// on the live stream again — it used to be painted twice.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn scrollback_reply_never_double_applies_queued_output() {
        let spec = otto_pty::CommandSpec {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), "sleep 0.3; printf QUEUED-MARK; sleep 5".into()],
            cwd: None,
            env: vec![],
        };
        let h = Arc::new(PtyHandle::spawn(&spec).expect("spawn"));
        // The attach-time subscription (`out_rx`).
        let mut rx = h.subscribe();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while !String::from_utf8_lossy(&h.snapshot_with_history(10)).contains("QUEUED-MARK") {
            assert!(
                tokio::time::Instant::now() < deadline,
                "marker never printed"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let frame = resync_frame(&mut rx, pty_capture(&h, 100), false).await;
        let v: serde_json::Value = serde_json::from_str(frame.json()).unwrap();
        let data = B64.decode(v["data"].as_str().unwrap()).unwrap();
        assert!(String::from_utf8_lossy(&data).contains("QUEUED-MARK"));
        // Nothing already in the snapshot is streamed after it.
        let mut streamed = Vec::new();
        while let Ok(chunk) = rx.try_recv() {
            streamed.extend_from_slice(&chunk);
        }
        assert!(
            !String::from_utf8_lossy(&streamed).contains("QUEUED-MARK"),
            "queued output was delivered in both the snapshot and the live stream"
        );
        let _ = h.kill();
    }

    // ── Credit-based flow control ─────────────────────────────────────────

    const KB: u64 = 1024;

    fn sent_len(step: &CreditStep) -> usize {
        match step {
            CreditStep::Send(b) => b.len(),
            _ => 0,
        }
    }

    #[test]
    fn credit_and_ack_frames_parse_and_the_window_is_clamped() {
        assert!(matches!(
            serde_json::from_str::<ClientFrame>(r#"{"type":"credit","window":1048576}"#),
            Ok(ClientFrame::Credit {
                window: 1_048_576,
                binary_snapshots: false
            })
        ));
        assert!(matches!(
            serde_json::from_str::<ClientFrame>(r#"{"type":"credit"}"#),
            Ok(ClientFrame::Credit {
                window: 0,
                binary_snapshots: false
            })
        ));
        assert!(matches!(
            serde_json::from_str::<ClientFrame>(
                r#"{"type":"credit","window":1048576,"binary_snapshots":true}"#
            ),
            Ok(ClientFrame::Credit {
                binary_snapshots: true,
                ..
            })
        ));
        assert!(matches!(
            serde_json::from_str::<ClientFrame>(r#"{"type":"ack","bytes":65536}"#),
            Ok(ClientFrame::Ack { bytes: 65_536 })
        ));
        assert_eq!(CreditGate::new(0).window, CREDIT_WINDOW_DEFAULT);
        assert_eq!(CreditGate::new(1).window, CREDIT_WINDOW_MIN);
        assert_eq!(CreditGate::new(u64::MAX).window, CREDIT_WINDOW_MAX);
        let grant: serde_json::Value =
            serde_json::from_str(&CreditGate::new(512 * KB).grant_frame()).unwrap();
        assert_eq!(grant["type"], "credit");
        assert_eq!(grant["window"], 512 * KB);
    }

    /// Normal output (the client keeps up) passes straight through, frame for
    /// frame — identical to the no-credit path.
    #[test]
    fn credit_normal_output_is_untouched() {
        let now = tokio::time::Instant::now();
        let mut g = CreditGate::new(1024 * KB);
        let mut acked = 0u64;
        for i in 0..10_000u64 {
            let chunk = format!("line {i}\r\n").into_bytes();
            let n = chunk.len();
            assert_eq!(
                g.push(chunk.clone(), now),
                CreditStep::Send(Bytes::from(chunk))
            );
            acked += n as u64;
            if i % 100 == 0 {
                assert_eq!(g.ack(acked, now), CreditStep::Idle);
            }
        }
        assert!(!g.waiting());
        assert!(g.stall_deadline().is_none());
    }

    /// THE property: whatever the producer's rate, the client never has more
    /// than `window` unacknowledged bytes, and a burst up to 2 × window is
    /// delivered losslessly and in order.
    #[test]
    fn credit_bounds_unacked_bytes_independent_of_send_rate() {
        let now = tokio::time::Instant::now();
        for burst in [1usize, 2, 8, 64] {
            let mut g = CreditGate::new(1024 * KB);
            let mut delivered: Vec<u8> = Vec::new();
            let mut produced: Vec<u8> = Vec::new();
            let mut client_backlog = 0u64;
            let mut consumed = 0u64;
            let mut peak = 0u64;
            // 2 MB total arrives `burst` 64 KB chunks per client parse step.
            let mut seq = 0u8;
            while produced.len() < 2 * 1024 * 1024 {
                for _ in 0..burst {
                    if produced.len() >= 2 * 1024 * 1024 {
                        break;
                    }
                    let chunk: Vec<u8> =
                        (0..64 * 1024).map(|i| seq.wrapping_add(i as u8)).collect();
                    seq = seq.wrapping_add(1);
                    produced.extend_from_slice(&chunk);
                    if let CreditStep::Send(b) = g.push(chunk, now) {
                        client_backlog += b.len() as u64;
                        delivered.extend_from_slice(&b);
                    }
                    peak = peak.max(client_backlog);
                    assert!(
                        g.unacked() <= g.window,
                        "burst {burst}: unacked {} > window",
                        g.unacked()
                    );
                }
                // The client parses 64 KB and acks.
                let parsed = client_backlog.min(64 * KB);
                client_backlog -= parsed;
                consumed += parsed;
                if let CreditStep::Send(b) = g.ack(consumed, now) {
                    client_backlog += b.len() as u64;
                    delivered.extend_from_slice(&b);
                }
                peak = peak.max(client_backlog);
            }
            // Drain the rest.
            while client_backlog > 0 || !g.held.is_empty() {
                consumed += client_backlog;
                client_backlog = 0;
                match g.ack(consumed, now) {
                    CreditStep::Send(b) => {
                        client_backlog += b.len() as u64;
                        delivered.extend_from_slice(&b);
                    }
                    CreditStep::Idle => {}
                    CreditStep::Resync => panic!("burst {burst}: a 2 × window burst must not skip"),
                }
            }
            assert!(
                peak <= 1024 * KB,
                "burst {burst}: client backlog peaked at {peak}"
            );
            assert_eq!(delivered, produced, "burst {burst}: lossless and in order");
        }
    }

    /// Past window + one held window the backlog is skipped and replaced by
    /// ONE snapshot once the client drained to a quarter window.
    #[test]
    fn credit_flood_skips_to_one_snapshot_at_a_quarter_window() {
        let now = tokio::time::Instant::now();
        let mut g = CreditGate::new(1024 * KB);
        let chunk = || vec![b'y'; 64 * 1024];
        let mut sent = 0usize;
        for _ in 0..16 {
            sent += sent_len(&g.push(chunk(), now));
        }
        assert_eq!(sent as u64, g.window, "the first window goes out");
        for _ in 0..16 {
            assert_eq!(g.push(chunk(), now), CreditStep::Idle, "held");
        }
        assert_eq!(g.held.len() as u64, g.window);
        assert!(!g.skipped);
        assert_eq!(g.push(chunk(), now), CreditStep::Idle);
        assert!(
            g.skipped && g.held.is_empty(),
            "overflow drops the held bytes"
        );
        // Everything after the skip is discarded without buffering.
        for _ in 0..100 {
            assert_eq!(g.push(chunk(), now), CreditStep::Idle);
            assert!(g.held.is_empty());
        }
        // The client drains: no snapshot until ≤ window/4 unacked, then one.
        assert_eq!(g.ack(512 * KB, now), CreditStep::Idle);
        assert_eq!(g.ack(768 * KB, now), CreditStep::Resync);
        assert!(!g.skipped);
        assert_eq!(g.ack(1024 * KB, now), CreditStep::Idle, "exactly one");
        // …and live output streams again.
        assert_eq!(sent_len(&g.push(b"live".to_vec(), now)), 4);
    }

    /// ^C while more than a window behind: the held backlog becomes a
    /// snapshot instead of making the user watch it scroll past.
    #[test]
    fn credit_input_while_behind_skips_the_held_backlog() {
        let now = tokio::time::Instant::now();
        let mut g = CreditGate::new(256 * KB);
        g.push(vec![0; 256 * 1024], now);
        g.push(vec![1; 100 * 1024], now);
        assert_eq!(g.held.len(), 100 * 1024);
        g.skip_on_input(true, now);
        assert!(g.skipped && g.held.is_empty());
        // The client's resync (queue dropped + acked) answers it; the socket
        // loop marks the gate superseded after sending that snapshot.
        g.superseded();
        assert_eq!(g.ack(256 * KB, now), CreditStep::Idle);
        assert_eq!(sent_len(&g.push(b"^C".to_vec(), now)), 2);
        // Input while caught up changes nothing.
        let mut h = CreditGate::new(256 * KB);
        h.push(vec![0; 1024], now);
        h.skip_on_input(true, now);
        assert!(!h.skipped);
    }

    /// Automatic terminal responses preserve pending output; legacy clients
    /// without the flag still represent explicit input.
    #[test]
    fn credit_emulator_reply_preserves_held_output() {
        let now = tokio::time::Instant::now();
        let mut gate = CreditGate::new(64 * KB);
        gate.push(vec![0; 64 * 1024], now);
        gate.push(b"held output".to_vec(), now);
        let reply = serde_json::from_str::<ClientFrame>(
            r#"{"type":"input","data":"G1swOzBS","user":false}"#,
        )
        .unwrap();
        let ClientFrame::Input { user, .. } = reply else {
            panic!("input expected")
        };
        gate.skip_on_input(user, now);
        assert!(!gate.skipped);
        assert_eq!(
            gate.ack(64 * KB, now),
            CreditStep::Send(Bytes::from_static(b"held output"))
        );
        assert!(matches!(
            serde_json::from_str::<ClientFrame>(r#"{"type":"input","data":"Aw=="}"#),
            Ok(ClientFrame::Input { user: true, .. })
        ));
    }

    /// Stale / duplicate / future acks are harmless.
    #[test]
    fn credit_ack_is_cumulative_and_clamped() {
        let now = tokio::time::Instant::now();
        let mut g = CreditGate::new(64 * KB);
        g.push(vec![0; 64 * 1024], now);
        assert_eq!(g.push(vec![1; 10], now), CreditStep::Idle);
        assert_eq!(
            g.ack(u64::MAX, now),
            CreditStep::Send(Bytes::from(vec![1u8; 10]))
        );
        assert_eq!(g.acked, 64 * KB, "clamped to what was sent");
        assert_eq!(g.ack(5, now), CreditStep::Idle, "a regression is ignored");
        assert_eq!(g.acked, 64 * KB);
    }

    /// A stalled client is never sent more than the window: the stall
    /// deadline drops the held output and owes ONE snapshot, and the client's
    /// own acks (it acks what it parses and drops) bring it — no deadlock.
    #[test]
    fn credit_stall_deadline_degrades_to_a_snapshot_without_reopening() {
        let t0 = tokio::time::Instant::now();
        let mut g = CreditGate::new(64 * KB);
        g.push(vec![0; 64 * 1024], t0);
        assert!(g.stall_deadline().is_none(), "full but nothing waiting");
        g.push(vec![1; 10], t0);
        assert_eq!(g.stall_deadline(), Some(t0 + FLOW_AUTO_RESUME));
        // Progress restarts the clock.
        let t1 = t0 + Duration::from_millis(1500);
        assert_eq!(
            g.ack(1024, t1),
            CreditStep::Send(Bytes::from(vec![1u8; 10]))
        );
        assert!(g.stall_deadline().is_none(), "nothing held any more");
        // 1014 bytes of credit left: those go out, the rest waits.
        assert_eq!(sent_len(&g.push(vec![2; 2048], t1)), 1014);
        assert_eq!(g.stall_deadline(), Some(t1 + FLOW_AUTO_RESUME));
        let unacked = g.unacked();
        g.forgive();
        assert_eq!(
            g.unacked(),
            unacked,
            "acked stays truthful: the window is not reopened"
        );
        assert!(
            g.held.is_empty() && g.skipped,
            "held output becomes one owed snapshot"
        );
        assert!(
            g.stall_deadline().is_none(),
            "not re-armed while nothing moves"
        );
        // More output while stalled: nothing is sent or buffered.
        for _ in 0..50 {
            assert_eq!(g.push(vec![3; 4096], t1), CreditStep::Idle);
        }
        assert!(g.held.is_empty() && g.stall_deadline().is_none());
        // The client wakes and acks what it parsed: one snapshot at ≤ window/4.
        assert_eq!(g.ack(g.sent - 32 * 1024, t1), CreditStep::Idle);
        assert_eq!(g.ack(g.sent - 1000, t1), CreditStep::Resync);
        assert_eq!(g.ack(g.sent, t1), CreditStep::Idle, "exactly one");
        assert_eq!(sent_len(&g.push(b"live".to_vec(), t1)), 4, "streams again");
    }

    /// r3-10-07: repeated ≥ 2 s stalls under a continuous producer used to
    /// forgive a window each time; the unacked backlog must stay ≤ window.
    #[test]
    fn credit_repeated_stalls_never_exceed_the_window() {
        let mut now = tokio::time::Instant::now();
        let mut g = CreditGate::new(256 * KB);
        let mut client_backlog = 0u64;
        let mut consumed = 0u64;
        for cycle in 0..20 {
            // A burst while the client is wedged (acks nothing).
            for _ in 0..40 {
                if let CreditStep::Send(b) = g.push(vec![b'x'; 16 * 1024], now) {
                    client_backlog += b.len() as u64;
                }
                assert!(
                    g.unacked() <= g.window,
                    "cycle {cycle}: unacked {} > window",
                    g.unacked()
                );
            }
            if let Some(at) = g.stall_deadline() {
                now = at;
                g.forgive();
            }
            assert!(
                client_backlog <= g.window,
                "cycle {cycle}: client holds {client_backlog}"
            );
            // Every other cycle the client drains everything it holds.
            if cycle % 2 == 1 {
                consumed += client_backlog;
                client_backlog = 0;
                match g.ack(consumed, now) {
                    CreditStep::Send(b) => client_backlog += b.len() as u64,
                    CreditStep::Resync => g.superseded(),
                    CreditStep::Idle => {}
                }
            }
        }
    }

    /// Any snapshot supersedes held output: nothing held is sent after it.
    #[test]
    fn credit_snapshot_supersedes_held_output() {
        let now = tokio::time::Instant::now();
        let mut g = CreditGate::new(64 * KB);
        g.push(vec![0; 64 * 1024], now);
        g.push(vec![1; 1000], now);
        g.superseded();
        assert!(g.held.is_empty() && !g.skipped && g.stall_deadline().is_none());
        assert_eq!(g.ack(64 * KB, now), CreditStep::Idle);
    }

    /// Attach-with-grid (perf 01 F1): `scrollback`/`resync` carry the client's
    /// grid optionally; old clients (no grid) still parse. The requested depth
    /// (perf 01 F4) is what later server-initiated snapshots reuse: `0` = the
    /// full retained depth, larger values are clamped to it.
    #[test]
    fn scrollback_frame_carries_an_optional_grid_and_depth() {
        assert!(matches!(
            serde_json::from_str::<ClientFrame>(
                r#"{"type":"scrollback","lines":2000,"cols":132,"rows":40}"#
            ),
            Ok(ClientFrame::Scrollback {
                lines: 2000,
                cols: Some(132),
                rows: Some(40)
            })
        ));
        assert!(matches!(
            serde_json::from_str::<ClientFrame>(r#"{"type":"scrollback","lines":10000}"#),
            Ok(ClientFrame::Scrollback {
                lines: 10000,
                cols: None,
                rows: None
            })
        ));
        assert!(matches!(
            serde_json::from_str::<ClientFrame>(
                r#"{"type":"resync","lines":500,"cols":80,"rows":24}"#
            ),
            Ok(ClientFrame::Resync {
                lines: 500,
                cols: Some(80),
                rows: Some(24)
            })
        ));
        assert_eq!(requested_history(0), DEFAULT_ATTACH_HISTORY_LINES);
        assert_eq!(requested_history(2000), 2000);
        assert_eq!(requested_history(10_000), DEFAULT_ATTACH_HISTORY_LINES);
    }
}

#[cfg(test)]
mod input_queue_tests {
    //! Perf 01 N7: the per-connection input queue never loses keystrokes. It
    //! coalesces what queued behind a slow write and bounds the backlog by
    //! BYTES with backpressure, where round 1 dropped every frame past 256.

    use super::*;
    use std::sync::Mutex;

    type Writes = Arc<Mutex<Vec<(Vec<u8>, bool)>>>;
    type Reverts = Arc<Mutex<Vec<Option<u64>>>>;
    type Results = tokio::sync::mpsc::Receiver<std::result::Result<(), String>>;

    /// A queue whose writer blocks until `gate` has a permit per write.
    fn gated(gate: Arc<Semaphore>, fail: bool) -> (InputQueue, Results, Writes, Reverts) {
        let writes: Writes = Arc::default();
        let reverts: Reverts = Arc::default();
        let (w, r) = (writes.clone(), reverts.clone());
        let (q, res) = spawn_input_queue(
            move |bytes: Vec<u8>, user: bool| {
                let gate = gate.clone();
                let w = w.clone();
                async move {
                    gate.acquire().await.unwrap().forget();
                    w.lock().unwrap().push((bytes, user));
                    if fail {
                        Err("not live".to_string())
                    } else {
                        Ok(())
                    }
                }
            },
            move |prev| r.lock().unwrap().push(prev),
        );
        (q, res, writes, reverts)
    }

    /// What the socket loop does per frame: queue it, or park it and wait.
    async fn push(q: &InputQueue, bytes: Vec<u8>, user: bool, prev: Option<u64>) {
        if let Err(job) = q.try_push(bytes, user, prev) {
            InputQueue::reserve(q.budget.clone(), job.cost).await;
            q.send(job);
        }
    }

    async fn settle(writes: &Writes, total: usize) {
        for _ in 0..500 {
            if writes
                .lock()
                .unwrap()
                .iter()
                .map(|w| w.0.len())
                .sum::<usize>()
                >= total
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("writer never delivered {total} bytes");
    }

    #[tokio::test]
    async fn thousands_of_keystrokes_behind_a_stuck_write_are_all_delivered_in_order() {
        let gate = Arc::new(Semaphore::new(0));
        let (q, _res, writes, _) = gated(gate.clone(), false);
        // 5000 one-byte keystrokes while the child is not reading (the first
        // write is stuck): 20× the old 256-frame cap. None may be dropped.
        let expected: Vec<u8> = (0..5000u32).map(|i| b'a' + (i % 26) as u8).collect();
        for b in &expected {
            push(&q, vec![*b], true, None).await;
        }
        gate.add_permits(Semaphore::MAX_PERMITS / 2);
        settle(&writes, expected.len()).await;
        let writes = writes.lock().unwrap();
        let got: Vec<u8> = writes.iter().flat_map(|w| w.0.clone()).collect();
        assert_eq!(got, expected, "every keystroke arrives, in order");
        // Coalesced: the stuck first write, then the backlog in few chunks.
        assert!(
            writes.len() <= 1 + 5000usize.div_ceil(INPUT_COALESCE_BYTES) + 1,
            "the backlog is coalesced, got {} writes",
            writes.len()
        );
    }

    #[tokio::test]
    async fn a_full_byte_budget_waits_instead_of_dropping() {
        let gate = Arc::new(Semaphore::new(0));
        let (q, _res, writes, _) = gated(gate.clone(), false);
        let chunk = INPUT_BUDGET_BYTES / 4;
        // Four quarter-budget pastes fill the budget (the first sits in the
        // stuck write and still holds its share).
        for i in 0..4u8 {
            assert!(q.try_push(vec![i; chunk], true, None).is_ok());
        }
        let Err(job) = q.try_push(vec![9; chunk], true, None) else {
            panic!("a fifth quarter must not fit the byte budget");
        };
        // …so it waits, it is not dropped:
        let wait = InputQueue::reserve(q.budget.clone(), job.cost);
        tokio::pin!(wait);
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut wait)
                .await
                .is_err(),
            "the parked frame waits while the budget is full"
        );
        // The child reads: budget frees, the parked frame goes through.
        gate.add_permits(Semaphore::MAX_PERMITS / 2);
        tokio::time::timeout(Duration::from_secs(5), wait)
            .await
            .expect("budget frees once the writer drains");
        q.send(job);
        settle(&writes, 5 * chunk).await;
        let got: Vec<u8> = writes
            .lock()
            .unwrap()
            .iter()
            .flat_map(|w| w.0.clone())
            .collect();
        assert_eq!(got.len(), 5 * chunk);
        assert_eq!(got[4 * chunk], 9, "the waited frame lands last, in order");
    }

    /// Perf3 R6: with the input budget full behind a child that is not
    /// reading its stdin, the loop must keep reading client frames. A paste
    /// interleaved with `ack`s still frees credit, so held output flows, and
    /// once the child reads every deferred byte arrives, in order.
    ///
    /// This drives the socket loop's own input/read arms (`DeferredInput`
    /// guard + reserve/release) over a scripted client; before the fix the
    /// read arm was disabled while a frame was parked, so the `ack` behind
    /// the paste was never read and this timed out.
    #[tokio::test]
    async fn a_full_input_budget_still_reads_acks_so_output_keeps_flowing() {
        let gate = Arc::new(Semaphore::new(0));
        let (q, _res, writes, _) = gated(gate.clone(), false);
        let now = tokio::time::Instant::now();
        let window = CREDIT_WINDOW_MIN;
        let mut credit = CreditGate::new(window);
        // Output producer: two windows of output, the second held for credit.
        let mut delivered = 0usize;
        for chunk in [vec![b'o'; window as usize], vec![b'p'; window as usize]] {
            if let CreditStep::Send(b) = credit.push(chunk, now) {
                delivered += b.len();
            }
        }
        assert_eq!(delivered, window as usize, "one window out, one held");

        // Client: six quarter-budget paste frames (four fill the budget, two
        // are deferred), then the ack for the first window, a probe-like
        // no-op resize, and one more keystroke.
        let quarter = INPUT_BUDGET_BYTES / 4;
        let mut expected: Vec<u8> = Vec::new();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        for i in 0..6u8 {
            let data = vec![b'a' + i; quarter];
            expected.extend_from_slice(&data);
            let frame = serde_json::json!({"type": "input", "data": B64.encode(&data)});
            tx.send(frame.to_string()).unwrap();
        }
        tx.send(format!(r#"{{"type":"ack","bytes":{window}}}"#))
            .unwrap();
        tx.send(r#"{"type":"resize","cols":80,"rows":24}"#.into())
            .unwrap();
        tx.send(serde_json::json!({"type": "input", "data": B64.encode(b"z")}).to_string())
            .unwrap();
        expected.push(b'z');
        drop(tx);

        // The loop's two input-related arms, as in `serve_terminal`.
        let mut deferred = DeferredInput::default();
        let (mut acks, mut resizes) = (0, 0);
        let run = async {
            loop {
                tokio::select! {
                    _ = InputQueue::reserve(q.budget.clone(), deferred.front_cost().unwrap_or(1)),
                        if deferred.front_cost().is_some() => deferred.release(&q),
                    msg = rx.recv(), if deferred.reading() => {
                        let Some(text) = msg else { break };
                        match serde_json::from_str::<ClientFrame>(&text).unwrap() {
                            ClientFrame::Input { data, user } => {
                                let bytes = B64.decode(data.as_bytes()).unwrap();
                                deferred.push(&q, bytes, user, None);
                            }
                            ClientFrame::Ack { bytes } => {
                                acks += 1;
                                if let CreditStep::Send(b) = credit.ack(bytes, now) {
                                    delivered += b.len();
                                }
                            }
                            ClientFrame::Resize { .. } => resizes += 1,
                            _ => {}
                        }
                    }
                }
            }
        };
        tokio::time::timeout(Duration::from_secs(5), run)
            .await
            .expect("client frames behind a full input budget are still read");
        assert_eq!((acks, resizes), (1, 1), "ack and resize processed");
        assert_eq!(
            delivered,
            2 * window as usize,
            "the ack freed credit: the held window went out"
        );
        assert!(
            deferred.front_cost().is_some(),
            "input past the budget is deferred, not dropped"
        );
        assert!(
            writes.lock().unwrap().is_empty(),
            "the child has not read yet"
        );

        // The child starts reading stdin: every deferred frame drains, in order.
        gate.add_permits(Semaphore::MAX_PERMITS / 2);
        tokio::time::timeout(Duration::from_secs(5), async {
            while let Some(cost) = deferred.front_cost() {
                InputQueue::reserve(q.budget.clone(), cost).await;
                deferred.release(&q);
            }
        })
        .await
        .expect("deferred input drains once the child reads");
        settle(&writes, expected.len()).await;
        let got: Vec<u8> = writes
            .lock()
            .unwrap()
            .iter()
            .flat_map(|w| w.0.clone())
            .collect();
        assert_eq!(got.len(), expected.len(), "nothing dropped");
        assert!(got == expected, "all input arrives in order");
        assert_eq!(deferred.bytes, 0);
    }

    /// The deferral itself is bounded: past INPUT_DEFER_BYTES the loop stops
    /// reading (TCP backpressure) rather than buffering without limit.
    #[tokio::test]
    async fn deferred_input_is_bounded_and_keeps_order() {
        let gate = Arc::new(Semaphore::new(0));
        let (q, _res, _writes, _) = gated(gate, false);
        let mut d = DeferredInput::default();
        d.push(&q, vec![0; INPUT_BUDGET_BYTES], true, None); // takes the whole budget
        assert!(d.front_cost().is_none() && d.reading());
        // Everything behind a deferred frame is deferred too: order is kept.
        d.push(&q, vec![1; INPUT_DEFER_BYTES - 1], true, None);
        d.push(&q, vec![2], true, None);
        assert_eq!(d.jobs.len(), 2);
        assert!(!d.reading(), "the deferral bound pauses reading");
    }

    #[tokio::test]
    async fn emulator_replies_never_coalesce_with_typing_and_a_failed_typing_write_reverts() {
        let gate = Arc::new(Semaphore::new(0));
        let (q, mut res, writes, reverts) = gated(gate.clone(), true);
        push(&q, b"x".to_vec(), true, Some(7)).await; // stuck write
                                                      // Let the writer pick it up and block in the write.
        tokio::time::sleep(Duration::from_millis(30)).await;
        push(&q, b"a".to_vec(), true, Some(7)).await;
        push(&q, b"b".to_vec(), true, Some(7)).await;
        push(&q, b"\x1b[1;1R".to_vec(), false, None).await; // DSR reply
        push(&q, b"c".to_vec(), true, Some(7)).await;
        gate.add_permits(100);
        settle(&writes, 3 + 6 + 1).await;
        let kinds: Vec<(Vec<u8>, bool)> = writes.lock().unwrap().clone();
        assert_eq!(
            kinds,
            vec![
                (b"x".to_vec(), true),
                (b"ab".to_vec(), true),
                (b"\x1b[1;1R".to_vec(), false),
                (b"c".to_vec(), true),
            ]
        );
        for _ in 0..4 {
            assert!(res.recv().await.unwrap().is_err());
        }
        // Only the three failed TYPING writes revert the size claim.
        assert_eq!(*reverts.lock().unwrap(), vec![Some(7); 3]);
    }
}

#[cfg(test)]
mod snapshot_encoding_tests {
    //! Perf 01 N3: snapshots as a header + ONE binary frame for clients that
    //! offered `binary_snapshots`; the base64-in-JSON form otherwise.

    use super::*;

    #[test]
    fn a_binary_capable_client_gets_raw_bytes_behind_a_header() {
        let data = b"\x1b[2J\x1b[Hhello \xff\x00 world".to_vec();
        let Snap::Binary { data: raw, epoch } = Snap::build(data.clone(), 7, true) else {
            panic!("binary form expected");
        };
        assert_eq!(&raw[..], &data[..], "bytes travel unencoded");
        assert_eq!(epoch, 7);
        let v: serde_json::Value = serde_json::from_str(&Snap::header(raw.len(), epoch)).unwrap();
        assert_eq!(v["type"], "scrollback");
        assert_eq!(v["binary"], true);
        assert_eq!(v["len"], data.len());
        assert_eq!(v["epoch"], 7);
        assert!(v.get("data").is_none(), "no base64 copy in the header");
    }

    #[test]
    fn older_clients_and_empty_snapshots_keep_the_json_form() {
        let v: serde_json::Value =
            serde_json::from_str(Snap::build(b"abc".to_vec(), 2, false).json()).unwrap();
        assert_eq!(B64.decode(v["data"].as_str().unwrap()).unwrap(), b"abc");
        // An empty snapshot (no live PTY) is ignored by clients: no payload frame.
        let v: serde_json::Value =
            serde_json::from_str(Snap::build(Vec::new(), 0, true).json()).unwrap();
        assert_eq!(v["data"], "");
    }

    /// The point of N3: a 4000-row snapshot costs its own size on the wire,
    /// not +33 % of base64 inside a JSON string the client must parse.
    #[test]
    fn the_binary_form_saves_the_base64_overhead() {
        let data = vec![b'x'; 1_500_000];
        let json = Snap::build(data.clone(), 1, false).json().len();
        let Snap::Binary { data: raw, epoch } = Snap::build(data, 1, true) else {
            panic!("binary form expected");
        };
        let binary = raw.len() + Snap::header(raw.len(), epoch).len();
        let ratio = json as f64 / binary as f64;
        assert!(
            ratio > 1.33,
            "base64-in-JSON is {ratio:.3}× the binary form (binary {binary} B, json {json} B)"
        );
    }
}
