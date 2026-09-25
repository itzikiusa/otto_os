//! Agent UI control — the daemon half of "the agent drives Otto, visibly".
//!
//! An agent running in an Otto session calls a governed `otto.ui_*` tool
//! (catalog: [`crate::ui_commands`]). `mcp_outward::governed_invoke` hands it
//! to [`run`], which:
//!
//! 1. binds the call to the CALLING SESSION (`managed_session_id` /
//!    `mcp_session_id` — never an argument) and its owner;
//! 2. requires the session's grant (`session.meta.ui_control.enabled`, set only
//!    by the human-only `POST /sessions/{id}/ui-control`), raising an
//!    owner-scoped `ui_control_requested` event and waiting briefly when it is
//!    missing → `pending_grant`;
//! 3. validates the arguments against the catalog schema and resolves a
//!    `connection_id` (id OR name) as the owner — the daemon-side RBAC
//!    pre-check (the UI then runs the action with the owner's own login token,
//!    so the endpoint's native RBAC runs again);
//! 4. picks ONE Otto document of the owner ([`pick`]: the session's own
//!    device, a document already showing the module, focused > visible >
//!    recent), opening the module in the side pane first when none shows it;
//! 5. sends a per-connection `ui_command` frame to that document only and
//!    waits for `POST /ui/commands/{id}/result` from the SAME user AND the
//!    SAME connection (`X-Otto-Ui-Conn`), with a deadline that a human-confirm
//!    `progress` may extend; on timeout / Stop / revoke / window close the
//!    document gets `ui_command_cancel`;
//! 6. redacts + row-caps the result before it reaches the agent.
//!
//! With no Otto window, a `read` command with a `headless` twin runs in the
//! daemon instead (`ui_visible:false`); everything else is `no_ui_client`.
//!
//! The registry here is in-memory and per daemon: documents register over
//! `/ws/events` ([`UiBridge::register`] + the `hello` / `presence` frames) —
//! and ONLY a human credential's socket may register, so an agent holding a
//! session token cannot pose as the window it is driving.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use otto_core::auth::AuthContext;
use otto_core::domain::Session;
use otto_core::event::Event;
use otto_core::{Error, Id};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot, Notify};
use tokio::time::Instant;

use crate::state::ServerCtx;
use crate::ui_commands::{self, UiCommandSpec};

/// Largest client→server text frame accepted on `/ws/events`.
pub const MAX_CLIENT_FRAME: usize = 64 * 1024;
/// Rows of any list returned to the agent.
pub const MAX_AGENT_ROWS: usize = 200;
/// How long an ungranted call waits for the human to click Allow.
const GRANT_WAIT: Duration = Duration::from_secs(15);
/// An explicit Deny this recent answers `pending_grant` without re-asking.
const DENY_QUIET: Duration = Duration::from_secs(60);
/// `ui_control_requested` is re-raised at most this often per session.
const REQUEST_EVERY: Duration = Duration::from_secs(10);
/// After an auto-open, how long to wait for the module's document to report.
const OPEN_READY_WAIT: Duration = Duration::from_secs(10);
/// `progress {awaiting_human}` extends the deadline to now + this…
const HUMAN_WAIT: Duration = Duration::from_millis(ui_commands::MAX_TIMEOUT_MS);
/// …but never past dispatch + this (inside stdio's governed-call budget).
const HUMAN_WAIT_CAP: Duration = Duration::from_secs(150);
/// Per-connection outbound frame queue.
const CONN_QUEUE: usize = 64;

// ===========================================================================
// Registry types
// ===========================================================================

/// Which pane a document is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pane {
    Main,
    Side,
}

/// What one registered Otto document reported (hello + later presence).
#[derive(Debug, Clone, Serialize)]
pub struct DocInfo {
    pub conn_id: String,
    pub user_id: Id,
    pub client_id: String,
    pub window_id: String,
    pub pane: Pane,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_window_id: Option<String>,
    pub route: String,
    pub module: String,
    pub focused: bool,
    pub visible: bool,
    #[serde(skip)]
    pub capabilities: HashSet<String>,
    /// Monotonic activity stamp (bumped on hello / presence) — "most recent".
    #[serde(skip)]
    pub activity: u64,
}

struct DocEntry {
    user_id: Id,
    tx: mpsc::Sender<String>,
    info: Option<DocInfo>,
}

/// How a pending command ended, as delivered to the waiting dispatcher.
#[derive(Debug)]
enum Outcome {
    Ok(Value),
    Err(UiError),
}

struct Pending {
    session_id: Id,
    user_id: Id,
    conn_id: String,
    dispatched: Instant,
    deadline: Instant,
    note: Option<String>,
    tx: Option<oneshot::Sender<Outcome>>,
}

/// A UI-control failure: a stable `code` the agent (and the tests) key on,
/// plus a human message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiError {
    pub code: String,
    pub message: String,
}

impl UiError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

/// The per-daemon registry of Otto documents + in-flight commands.
pub struct UiBridge {
    docs: Mutex<HashMap<String, DocEntry>>,
    pending: Mutex<HashMap<Id, Pending>>,
    presence_changed: Notify,
    grant_changed: Notify,
    last_request: Mutex<HashMap<Id, Instant>>,
    seq: AtomicU64,
}

impl Default for UiBridge {
    fn default() -> Self {
        Self {
            docs: Mutex::new(HashMap::new()),
            pending: Mutex::new(HashMap::new()),
            presence_changed: Notify::new(),
            grant_changed: Notify::new(),
            last_request: Mutex::new(HashMap::new()),
            seq: AtomicU64::new(1),
        }
    }
}

impl std::fmt::Debug for UiBridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UiBridge").finish_non_exhaustive()
    }
}

// ===========================================================================
// Client frames (`/ws/events`, client → server)
// ===========================================================================

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientFrame {
    Hello(HelloFrame),
    Presence(PresenceFrame),
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Deserialize)]
struct HelloFrame {
    client_id: String,
    window_id: String,
    pane: Pane,
    #[serde(default)]
    host_window_id: Option<String>,
    #[serde(default)]
    route: String,
    #[serde(default)]
    module: String,
    #[serde(default)]
    focused: bool,
    #[serde(default)]
    visible: bool,
    #[serde(default)]
    capabilities: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct PresenceFrame {
    #[serde(default)]
    route: String,
    #[serde(default)]
    module: String,
    #[serde(default)]
    focused: bool,
    #[serde(default)]
    visible: bool,
}

const MAX_ID_LEN: usize = 128;
const MAX_ROUTE_LEN: usize = 2048;
const MAX_CAPS: usize = 1024;

fn short_ok(s: &str, max: usize) -> bool {
    s.len() <= max && !s.chars().any(char::is_control)
}

/// Parse + bound one client frame. `None` = ignore it (oversized, malformed,
/// unknown type, or a field out of bounds). Pure.
fn parse_client_frame(text: &str) -> Option<ClientFrame> {
    if text.len() > MAX_CLIENT_FRAME {
        return None;
    }
    let frame: ClientFrame = serde_json::from_str(text).ok()?;
    let ok = match &frame {
        ClientFrame::Hello(h) => {
            !h.client_id.is_empty()
                && !h.window_id.is_empty()
                && short_ok(&h.client_id, MAX_ID_LEN)
                && short_ok(&h.window_id, MAX_ID_LEN)
                && h
                    .host_window_id
                    .as_deref()
                    .is_none_or(|w| short_ok(w, MAX_ID_LEN))
                && short_ok(&h.route, MAX_ROUTE_LEN)
                && short_ok(&h.module, MAX_ID_LEN)
                && h.capabilities.len() <= MAX_CAPS
                && h.capabilities.iter().all(|c| short_ok(c, MAX_ID_LEN))
        }
        ClientFrame::Presence(p) => {
            short_ok(&p.route, MAX_ROUTE_LEN) && short_ok(&p.module, MAX_ID_LEN)
        }
        ClientFrame::Unknown => false,
    };
    ok.then_some(frame)
}

// ===========================================================================
// Target picking — pure
// ===========================================================================

/// Where a command should go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pick {
    /// Send it to this document.
    Direct(String),
    /// No document shows the module: open it via this MAIN document (in its
    /// side pane), then send it to whichever document of `window_id` reports it.
    Open { via: String, window_id: String },
    /// Nothing can run it.
    None,
}

/// Choose the document a `command` for `module` runs in, among `docs`, for
/// the session owner `user_id` whose session was started on `device`
/// (`session.meta.client_id`; `None` = unknown → any device of the owner).
///
/// 1. Only the owner's documents that implement `command` (`capabilities`).
/// 2. The session's own device. Another device only when the session's
///    device has NO Otto document at all — and then only a document already
///    showing the module AND focused (never an auto-open elsewhere).
/// 3. A document showing the module (`shell` = any document, main pane
///    preferred), ranked focused > visible > most recent.
/// 4. None showing it: open it through the device's best MAIN document.
pub fn pick(docs: &[DocInfo], user_id: &str, device: Option<&str>, module: &str, command: &str) -> Pick {
    let mine: Vec<&DocInfo> = docs.iter().filter(|d| d.user_id == user_id).collect();
    let on_device = |d: &&DocInfo| device.is_none_or(|dev| d.client_id == dev);
    let showing = |d: &&DocInfo| module == "shell" || d.module == module;
    let can = |d: &&DocInfo, c: &str| d.capabilities.contains(c);
    let rank = |d: &DocInfo| {
        (
            d.focused,
            d.visible,
            module == "shell" && d.pane == Pane::Main,
            d.activity,
        )
    };
    let best = |it: Vec<&DocInfo>| it.into_iter().max_by_key(|d| rank(d)).map(|d| d.conn_id.clone());

    let device_docs: Vec<&DocInfo> = mine.iter().copied().filter(on_device).collect();
    if device_docs.is_empty() {
        let elsewhere: Vec<&DocInfo> = mine
            .iter()
            .copied()
            .filter(|d| showing(d) && d.focused && can(d, command))
            .collect();
        return best(elsewhere).map_or(Pick::None, Pick::Direct);
    }
    let direct: Vec<&DocInfo> = device_docs
        .iter()
        .copied()
        .filter(|d| showing(d) && can(d, command))
        .collect();
    if let Some(id) = best(direct) {
        return Pick::Direct(id);
    }
    if module == "shell" {
        return Pick::None;
    }
    let openers: Vec<&DocInfo> = device_docs
        .iter()
        .copied()
        .filter(|d| d.pane == Pane::Main && can(d, "open"))
        .collect();
    match openers.into_iter().max_by_key(|d| rank(d)) {
        Some(d) => Pick::Open {
            via: d.conn_id.clone(),
            window_id: d.window_id.clone(),
        },
        None => Pick::None,
    }
}

/// After an auto-open through window `window_id`: the document of that window
/// (main or its side pane) now showing `module` and implementing `command`.
fn opened_target(docs: &[DocInfo], user_id: &str, window_id: &str, module: &str, command: &str) -> Option<String> {
    docs.iter()
        .filter(|d| {
            d.user_id == user_id
                && d.module == module
                && d.capabilities.contains(command)
                && (d.window_id == window_id || d.host_window_id.as_deref() == Some(window_id))
        })
        .max_by_key(|d| (d.focused, d.visible, d.activity))
        .map(|d| d.conn_id.clone())
}

// ===========================================================================
// Grants — pure helpers over `session.meta.ui_control`
// ===========================================================================

/// The server-owned grant record in `session.meta`.
pub const META_KEY: &str = "ui_control";

/// `Some(true)` granted, `Some(false)` explicitly denied/revoked, `None` never decided.
pub fn grant_state(meta: &Value) -> Option<bool> {
    meta.get(META_KEY)?.get("enabled")?.as_bool()
}

/// When the last grant decision was made (RFC 3339), if recorded.
fn grant_decided_at(meta: &Value) -> Option<chrono::DateTime<chrono::Utc>> {
    meta.get(META_KEY)?
        .get("granted_at")?
        .as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.with_timezone(&chrono::Utc))
}

/// A Deny/Stop within [`DENY_QUIET`] — answer without prompting again.
fn recently_denied(meta: &Value, now: chrono::DateTime<chrono::Utc>) -> bool {
    grant_state(meta) == Some(false)
        && grant_decided_at(meta).is_some_and(|t| {
            now.signed_duration_since(t)
                .to_std()
                .map(|d| d < DENY_QUIET)
                .unwrap_or(true)
        })
}

/// The grant record written by `POST /sessions/{id}/ui-control`.
pub fn grant_record(enabled: bool, by: &str) -> Value {
    json!({ META_KEY: {
        "enabled": enabled,
        "granted_at": chrono::Utc::now().to_rfc3339(),
        "granted_by": by,
    }})
}

// ===========================================================================
// Result shaping — pure
// ===========================================================================

/// Cap every array at [`MAX_AGENT_ROWS`] (marking `<key>_truncated: <n>` on the
/// parent object), then redact. What the agent sees of any UI result.
pub fn shape_result(v: Value) -> Value {
    fn cap(v: Value) -> Value {
        match v {
            Value::Array(a) => Value::Array(a.into_iter().take(MAX_AGENT_ROWS).map(cap).collect()),
            Value::Object(m) => {
                let mut out = serde_json::Map::with_capacity(m.len());
                let mut marks = Vec::new();
                for (k, val) in m {
                    if let Value::Array(a) = &val {
                        if a.len() > MAX_AGENT_ROWS {
                            marks.push((format!("{k}_truncated"), json!(a.len())));
                        }
                    }
                    out.insert(k, cap(val));
                }
                for (k, n) in marks {
                    out.entry(k).or_insert(n);
                }
                Value::Object(out)
            }
            other => other,
        }
    }
    otto_core::redact::redact_json(&cap(v)).value
}

/// `{ui_visible, …result}` — the result object with the visibility flag
/// merged in (a non-object result nests under `result`).
fn with_visibility(v: Value, visible: bool) -> Value {
    match v {
        Value::Object(mut m) => {
            m.insert("ui_visible".into(), json!(visible));
            Value::Object(m)
        }
        other => json!({"ui_visible": visible, "result": other}),
    }
}

/// The deadline the daemon gives the UI for `spec` with these arguments:
/// the catalog default, or for a statement carrying `timeout_ms` that + 15 s;
/// capped at [`ui_commands::MAX_TIMEOUT_MS`].
pub fn deadline_for(spec: &UiCommandSpec, args: &Value) -> Duration {
    let ms = match args.get("timeout_ms").and_then(Value::as_u64) {
        Some(t) if spec.input_schema["properties"].get("timeout_ms").is_some() => {
            t.saturating_add(15_000).max(spec.timeout_ms)
        }
        _ => spec.timeout_ms,
    };
    Duration::from_millis(ms.min(ui_commands::MAX_TIMEOUT_MS))
}

/// The deadline after a `progress {awaiting_human}` at `now`: now + 120 s,
/// never past dispatch + 150 s, never earlier than the current one. Pure.
fn extended_deadline(now: Instant, dispatched: Instant, current: Instant) -> Instant {
    (now + HUMAN_WAIT).min(dispatched + HUMAN_WAIT_CAP).max(current)
}

/// Normalize a UI-reported error code to the contract's set.
fn normalize_code(code: &str) -> &'static str {
    match code {
        "cancelled_by_user" => "cancelled_by_user",
        "invalid_args" => "invalid_args",
        "not_found" => "not_found",
        "forbidden" => "forbidden",
        _ => "failed",
    }
}

// ===========================================================================
// The registry
// ===========================================================================

impl UiBridge {
    fn next_seq(&self) -> u64 {
        self.seq.fetch_add(1, Ordering::Relaxed)
    }

    /// Register a `/ws/events` connection of a HUMAN credential. Returns its
    /// ephemeral `conn_id` and the queue of per-connection frames the socket
    /// loop must forward. It is not a target until its `hello` arrives.
    pub fn register(&self, user_id: &Id) -> (String, mpsc::Receiver<String>) {
        let conn_id = otto_core::new_id();
        let (tx, rx) = mpsc::channel(CONN_QUEUE);
        self.docs.lock().unwrap().insert(
            conn_id.clone(),
            DocEntry {
                user_id: user_id.clone(),
                tx,
                info: None,
            },
        );
        (conn_id, rx)
    }

    /// Drop a closed connection; its in-flight commands fail with `no_ui_client`.
    pub fn unregister(&self, conn_id: &str) {
        self.docs.lock().unwrap().remove(conn_id);
        let gone: Vec<Pending> = {
            let mut p = self.pending.lock().unwrap();
            let ids: Vec<Id> = p
                .iter()
                .filter(|(_, v)| v.conn_id == conn_id)
                .map(|(k, _)| k.clone())
                .collect();
            ids.into_iter().filter_map(|id| p.remove(&id)).collect()
        };
        for mut g in gone {
            if let Some(tx) = g.tx.take() {
                let _ = tx.send(Outcome::Err(UiError::new(
                    "no_ui_client",
                    "the Otto window running this command was closed",
                )));
            }
        }
        self.presence_changed.notify_waiters();
    }

    /// Handle one client text frame from `conn_id`. Unknown / malformed /
    /// oversized frames are ignored.
    pub fn on_client_frame(&self, conn_id: &str, text: &str) {
        let Some(frame) = parse_client_frame(text) else {
            return;
        };
        let activity = self.next_seq();
        let mut docs = self.docs.lock().unwrap();
        let Some(entry) = docs.get_mut(conn_id) else {
            return;
        };
        match frame {
            ClientFrame::Hello(h) => {
                entry.info = Some(DocInfo {
                    conn_id: conn_id.to_string(),
                    user_id: entry.user_id.clone(),
                    client_id: h.client_id,
                    window_id: h.window_id,
                    pane: h.pane,
                    host_window_id: h.host_window_id.filter(|w| !w.is_empty()),
                    route: h.route,
                    module: h.module,
                    focused: h.focused,
                    visible: h.visible,
                    capabilities: h.capabilities.into_iter().collect(),
                    activity,
                });
                let _ = entry
                    .tx
                    .try_send(json!({"type":"hello_ack","conn_id":conn_id}).to_string());
            }
            ClientFrame::Presence(p) => {
                if let Some(info) = entry.info.as_mut() {
                    info.route = p.route;
                    info.module = p.module;
                    info.focused = p.focused;
                    info.visible = p.visible;
                    info.activity = activity;
                }
            }
            ClientFrame::Unknown => return,
        }
        drop(docs);
        self.presence_changed.notify_waiters();
    }

    /// Snapshot of the registered (hello'd) documents.
    pub fn docs(&self) -> Vec<DocInfo> {
        self.docs
            .lock()
            .unwrap()
            .values()
            .filter_map(|e| e.info.clone())
            .collect()
    }

    fn user_has_docs(&self, user_id: &str) -> bool {
        self.docs
            .lock()
            .unwrap()
            .values()
            .any(|e| e.user_id == user_id && e.info.is_some())
    }

    /// Queue a frame to one connection. `false` when it is gone or saturated.
    fn send_to(&self, conn_id: &str, frame: String) -> bool {
        self.docs
            .lock()
            .unwrap()
            .get(conn_id)
            .is_some_and(|e| e.tx.try_send(frame).is_ok())
    }

    /// Put a command on the wire to `conn_id` and register it as pending.
    fn send_command(
        &self,
        conn_id: &str,
        session: &Session,
        command: &str,
        args: &Value,
        budget: Duration,
    ) -> Result<(Id, oneshot::Receiver<Outcome>), UiError> {
        let id = otto_core::new_id();
        let (tx, rx) = oneshot::channel();
        let now = Instant::now();
        self.pending.lock().unwrap().insert(
            id.clone(),
            Pending {
                session_id: session.id.clone(),
                user_id: session.created_by.clone(),
                conn_id: conn_id.to_string(),
                dispatched: now,
                deadline: now + budget,
                note: None,
                tx: Some(tx),
            },
        );
        let frame = json!({
            "type": "ui_command",
            "id": id,
            "session_id": session.id,
            "agent": {
                "session_id": session.id,
                "title": session.title,
                "provider": session.provider,
            },
            "command": command,
            "args": args,
            "deadline_ms": budget.as_millis() as u64,
        })
        .to_string();
        if !self.send_to(conn_id, frame) {
            self.pending.lock().unwrap().remove(&id);
            return Err(UiError::new(
                "no_ui_client",
                "the Otto window could not be reached (it closed or is not responding)",
            ));
        }
        Ok((id, rx))
    }

    /// Cancel one pending command: tell its document, fail its waiter.
    fn cancel(&self, id: &str, reason: &str, err: Option<UiError>) {
        let Some(mut p) = self.pending.lock().unwrap().remove(id) else {
            return;
        };
        self.send_to(
            &p.conn_id,
            json!({"type":"ui_command_cancel","id":id,"reason":reason}).to_string(),
        );
        if let (Some(tx), Some(e)) = (p.tx.take(), err) {
            let _ = tx.send(Outcome::Err(e));
        }
    }

    /// Cancel every pending command of a session (grant revoked / session gone).
    pub fn cancel_session(&self, session_id: &str, reason: &str, err: UiError) {
        let ids: Vec<Id> = self
            .pending
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, p)| p.session_id == session_id)
            .map(|(k, _)| k.clone())
            .collect();
        for id in ids {
            self.cancel(&id, reason, Some(err.clone()));
        }
    }

    /// Wait for a pending command's outcome, honouring (extendable) deadlines.
    /// Dropping this future (the agent's call went away) cancels the command.
    async fn await_outcome(&self, id: &Id, mut rx: oneshot::Receiver<Outcome>) -> Result<Value, UiError> {
        struct Guard<'a> {
            bridge: &'a UiBridge,
            id: &'a Id,
            armed: bool,
        }
        impl Drop for Guard<'_> {
            fn drop(&mut self) {
                if self.armed {
                    self.bridge.cancel(self.id, "caller_gone", None);
                }
            }
        }
        let mut guard = Guard {
            bridge: self,
            id,
            armed: true,
        };
        loop {
            let deadline = match self.pending.lock().unwrap().get(id) {
                Some(p) => p.deadline,
                // Already resolved/cancelled — the outcome is in the channel.
                None => Instant::now() + Duration::from_secs(1),
            };
            tokio::select! {
                out = &mut rx => {
                    guard.armed = false;
                    return match out {
                        Ok(Outcome::Ok(v)) => Ok(v),
                        Ok(Outcome::Err(e)) => Err(e),
                        Err(_) => Err(UiError::new("failed", "the command was dropped")),
                    };
                }
                _ = tokio::time::sleep_until(deadline) => {
                    let expired = self
                        .pending
                        .lock()
                        .unwrap()
                        .get(id)
                        .map(|p| (p.deadline <= Instant::now(), p.note.clone()));
                    match expired {
                        Some((true, note)) => {
                            guard.armed = false;
                            self.cancel(id, "timeout", None);
                            let mut msg = "the Otto window did not finish the command in time".to_string();
                            if let Some(n) = note {
                                msg.push_str(&format!(" (last progress: {n})"));
                            }
                            return Err(UiError::new("timeout", msg));
                        }
                        Some((false, _)) => continue,
                        None => {
                            // Resolved between the wake-up and the lock: the
                            // outcome is (or will immediately be) in `rx`.
                            if let Ok(Ok(out)) = tokio::time::timeout(Duration::from_secs(1), &mut rx).await {
                                guard.armed = false;
                                return match out {
                                    Outcome::Ok(v) => Ok(v),
                                    Outcome::Err(e) => Err(e),
                                };
                            }
                            guard.armed = false;
                            return Err(UiError::new("failed", "the command was dropped"));
                        }
                    }
                }
            }
        }
    }

    /// Dispatch to `conn_id` and wait for the outcome.
    async fn dispatch(
        &self,
        conn_id: &str,
        session: &Session,
        command: &str,
        args: &Value,
        budget: Duration,
    ) -> Result<Value, UiError> {
        let (id, rx) = self.send_command(conn_id, session, command, args, budget)?;
        self.await_outcome(&id, rx).await
    }

    /// Check the caller of the result/progress routes against a pending command.
    fn authorize_reply(&self, id: &str, user_id: &str, conn_id: Option<&str>) -> Result<(), Error> {
        let p = self.pending.lock().unwrap();
        let Some(p) = p.get(id) else {
            return Err(Error::NotFound("ui command (finished, cancelled or unknown)".into()));
        };
        if p.user_id != user_id {
            return Err(Error::Forbidden("this UI command belongs to another user".into()));
        }
        if conn_id != Some(p.conn_id.as_str()) {
            return Err(Error::Forbidden(
                "this UI command was sent to a different Otto window (X-Otto-Ui-Conn)".into(),
            ));
        }
        Ok(())
    }

    /// `POST /ui/commands/{id}/result` — deliver the outcome.
    pub fn complete(&self, id: &str, user_id: &str, conn_id: Option<&str>, reply: UiReply) -> Result<(), Error> {
        self.authorize_reply(id, user_id, conn_id)?;
        let Some(mut p) = self.pending.lock().unwrap().remove(id) else {
            return Err(Error::NotFound("ui command".into()));
        };
        let outcome = match reply {
            UiReply { ok: true, result, .. } => Outcome::Ok(result.unwrap_or(Value::Null)),
            UiReply { error, .. } => {
                let e = error.unwrap_or_default();
                Outcome::Err(UiError::new(
                    normalize_code(&e.code),
                    if e.message.trim().is_empty() {
                        "the Otto window reported a failure".to_string()
                    } else {
                        e.message.chars().take(2000).collect()
                    },
                ))
            }
        };
        if let Some(tx) = p.tx.take() {
            let _ = tx.send(outcome);
        }
        Ok(())
    }

    /// `POST /ui/commands/{id}/progress` — record a note; `awaiting_human`
    /// extends the deadline (bounded, see [`HUMAN_WAIT`] / [`HUMAN_WAIT_CAP`]).
    pub fn progress(&self, id: &str, user_id: &str, conn_id: Option<&str>, note: &str, awaiting_human: bool) -> Result<(), Error> {
        self.authorize_reply(id, user_id, conn_id)?;
        let mut all = self.pending.lock().unwrap();
        let Some(p) = all.get_mut(id) else {
            return Err(Error::NotFound("ui command".into()));
        };
        let note: String = note.chars().take(500).collect();
        if !note.trim().is_empty() {
            p.note = Some(note);
        }
        if awaiting_human {
            p.deadline = extended_deadline(Instant::now(), p.dispatched, p.deadline);
        }
        Ok(())
    }

    /// Wake every call waiting for a grant decision.
    pub fn grant_changed(&self) {
        self.grant_changed.notify_waiters();
    }

    /// Rate-limit `ui_control_requested`: `true` when it may be raised now.
    fn should_request(&self, session_id: &str) -> bool {
        let mut m = self.last_request.lock().unwrap();
        let now = Instant::now();
        match m.get(session_id) {
            Some(t) if now.duration_since(*t) < REQUEST_EVERY => false,
            _ => {
                m.insert(session_id.to_string(), now);
                true
            }
        }
    }

    /// Wait up to `wait` for a document matching `f`.
    async fn wait_for_doc(&self, wait: Duration, f: impl Fn(&[DocInfo]) -> Option<String>) -> Option<String> {
        let until = Instant::now() + wait;
        loop {
            let notified = self.presence_changed.notified();
            if let Some(id) = f(&self.docs()) {
                return Some(id);
            }
            if Instant::now() >= until {
                return None;
            }
            let _ = tokio::time::timeout_at(until.min(Instant::now() + Duration::from_millis(250)), notified).await;
        }
    }
}

/// Body of `POST /ui/commands/{id}/result`.
#[derive(Debug, Deserialize)]
pub struct UiReply {
    pub ok: bool,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<UiReplyError>,
}

#[derive(Debug, Default, Deserialize)]
pub struct UiReplyError {
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub message: String,
}

// ===========================================================================
// The dispatcher (called from `mcp_outward::governed_invoke`)
// ===========================================================================

/// The calling session of an Otto-minted credential, if any.
pub fn calling_session(auth: &AuthContext) -> Option<&Id> {
    auth.managed_session_id
        .as_ref()
        .or(auth.mcp_session_id.as_ref())
}

/// True iff `auth` is a person's own credential (not an agent session's, not
/// an MCP-restricted one, not a share link) — the only kind allowed to act as
/// "the UI" or to grant UI control.
pub fn is_human(auth: &AuthContext) -> bool {
    auth.managed_session_id.is_none() && auth.mcp_session_id.is_none() && !auth.mcp_only && !auth.is_scoped()
}

/// The message the agent gets while the user has not allowed UI control.
pub const PENDING_GRANT_MSG: &str = "The user hasn't allowed UI control for this session yet — they were asked in Otto; retry after they allow it";

/// Run one `otto.ui_*` tool for `auth`. `tool` is the bare governed name.
pub async fn run(ctx: &ServerCtx, auth: &AuthContext, tool: &str, args: &Value) -> Result<Value, UiError> {
    let spec = ui_commands::by_tool(tool)
        .ok_or_else(|| UiError::new("not_found", format!("unknown UI command '{tool}'")))?;
    let sid = calling_session(auth).ok_or_else(|| {
        UiError::new(
            "forbidden",
            "UI control is for agents running in an Otto session (this credential has no session)",
        )
    })?;
    let session = ctx
        .manager
        .get(sid)
        .await
        .map_err(|_| UiError::new("not_found", "the calling session no longer exists"))?;
    if session.created_by != auth.effective_user.id {
        return Err(UiError::new(
            "forbidden",
            "UI control drives the session owner's own Otto window",
        ));
    }
    if let Some(pin) = crate::agent_refs::pin_of(auth) {
        if pin != session.workspace_id {
            return Err(UiError::new(
                "forbidden",
                format!("this token is scoped to workspace '{pin}'"),
            ));
        }
    }
    let args = if args.is_null() { json!({}) } else { args.clone() };
    ui_commands::validate_args(&spec.input_schema, &args).map_err(|m| UiError::new("invalid_args", m))?;

    let session = ensure_grant(ctx, session, spec).await?;
    let args = resolve_refs(ctx, auth, &session, args).await?;

    let bridge = &ctx.ui_bridge;
    let device = session.meta.get("client_id").and_then(Value::as_str).map(str::to_string);
    let budget = deadline_for(spec, &args);
    let target = match pick(&bridge.docs(), &session.created_by, device.as_deref(), &spec.module, &spec.name) {
        Pick::Direct(conn) => Some(conn),
        Pick::Open { via, window_id } => {
            let open_args = json!({"module": spec.module, "route": spec.route, "placement": "side"});
            let open_budget = ui_commands::get("open")
                .map(|o| Duration::from_millis(o.timeout_ms))
                .unwrap_or(Duration::from_secs(15));
            bridge.dispatch(&via, &session, "open", &open_args, open_budget).await?;
            let (user, module, command) = (session.created_by.clone(), spec.module.clone(), spec.name.clone());
            let found = bridge
                .wait_for_doc(OPEN_READY_WAIT, |docs| opened_target(docs, &user, &window_id, &module, &command))
                .await;
            if found.is_none() {
                return Err(UiError::new(
                    "no_ui_client",
                    format!("opened {} in Otto, but it did not report ready in time — retry", spec.module),
                ));
            }
            found
        }
        Pick::None => None,
    };
    match target {
        Some(conn) => {
            let v = bridge.dispatch(&conn, &session, &spec.name, &args, budget).await?;
            Ok(with_visibility(shape_result(v), true))
        }
        None => match spec.headless.as_deref() {
            Some(h) => {
                let v = headless(ctx, auth, &session, h, &args).await?;
                let mut v = with_visibility(shape_result(v), false);
                v["note"] = json!("no Otto window can show this right now — ran it in the daemon instead");
                Ok(v)
            }
            None => Err(UiError::new(
                "no_ui_client",
                "no Otto window is open on the device this session was started from (or it is \
                 too old to support this command) — ask the user to open Otto",
            )),
        },
    }
}

/// Require the session's grant; raise the owner-scoped request and wait
/// briefly when it is missing. Returns the (re-read) session on success.
async fn ensure_grant(ctx: &ServerCtx, session: Session, spec: &UiCommandSpec) -> Result<Session, UiError> {
    if grant_state(&session.meta) == Some(true) {
        return Ok(session);
    }
    let pending = |extra: &str| UiError::new("pending_grant", format!("{PENDING_GRANT_MSG}{extra}"));
    if recently_denied(&session.meta, chrono::Utc::now()) {
        return Err(pending(" (they declined or stopped it moments ago)."));
    }
    let bridge = &ctx.ui_bridge;
    if !bridge.user_has_docs(&session.created_by) {
        return Err(pending(" (no Otto window is open right now, so they could not be asked)."));
    }
    if bridge.should_request(&session.id) {
        let _ = ctx.events.send(Event::UiControlRequested {
            user_id: session.created_by.clone(),
            workspace_id: session.workspace_id.clone(),
            session_id: session.id.clone(),
            session_title: session.title.clone(),
            module: spec.module.clone(),
            command: spec.name.clone(),
        });
    }
    let asked = chrono::Utc::now();
    let until = Instant::now() + GRANT_WAIT;
    loop {
        let notified = bridge.grant_changed.notified();
        let _ = tokio::time::timeout_at(until.min(Instant::now() + Duration::from_millis(500)), notified).await;
        let s = ctx
            .manager
            .get(&session.id)
            .await
            .map_err(|_| UiError::new("not_found", "the calling session no longer exists"))?;
        match grant_state(&s.meta) {
            Some(true) => return Ok(s),
            Some(false) if grant_decided_at(&s.meta).is_some_and(|t| t >= asked) => {
                return Err(pending(" (they declined it)."));
            }
            _ => {}
        }
        if Instant::now() >= until {
            return Err(pending("."));
        }
    }
}

/// Resolve `connection_id` (id OR name) as the session owner — the daemon's
/// RBAC pre-check: a connection the owner cannot list is `not_found` /
/// `forbidden` before anything reaches the window.
async fn resolve_refs(ctx: &ServerCtx, auth: &AuthContext, session: &Session, mut args: Value) -> Result<Value, UiError> {
    let Some(reference) = args.get("connection_id").and_then(Value::as_str).map(str::to_string) else {
        return Ok(args);
    };
    let kind = crate::agent_refs::kind_of("connection")
        .ok_or_else(|| UiError::new("failed", "connection resolver unavailable"))?;
    let (c, _) = crate::agent_refs::resolve(
        ctx,
        auth,
        kind,
        "connection_id",
        Some(&reference),
        None,
        Some(session.workspace_id.as_str()),
    )
    .await
    .map_err(|e| match e {
        Error::NotFound(m) => UiError::new("not_found", m),
        Error::Forbidden(m) => UiError::new("forbidden", m),
        Error::Conflict(m) | Error::Invalid(m) => UiError::new("invalid_args", m),
        other => UiError::new("failed", otto_core::redact::redact_text(&other.to_string()).value),
    })?;
    args["connection_id"] = json!(c.id);
    Ok(args)
}

/// The daemon-side twin of a `read` command, used when no window can run it.
async fn headless(ctx: &ServerCtx, auth: &AuthContext, session: &Session, which: &str, args: &Value) -> Result<Value, UiError> {
    let fail = |e: Error| match e {
        Error::NotFound(m) => UiError::new("not_found", m),
        Error::Forbidden(m) => UiError::new("forbidden", m),
        Error::Invalid(m) | Error::Conflict(m) => UiError::new("invalid_args", m),
        other => UiError::new("failed", otto_core::redact::redact_text(&other.to_string()).value),
    };
    match which {
        "presence" => {
            let device = session.meta.get("client_id").and_then(Value::as_str);
            let docs: Vec<Value> = ctx
                .ui_bridge
                .docs()
                .into_iter()
                .filter(|d| d.user_id == session.created_by)
                .map(|d| {
                    let same = device.is_none_or(|dev| dev == d.client_id);
                    json!({"pane": d.pane, "route": d.route, "module": d.module,
                           "focused": d.focused, "visible": d.visible,
                           "window_id": d.window_id, "session_device": same})
                })
                .collect();
            Ok(json!({"documents": docs}))
        }
        "db_list_connections" => {
            let kind = crate::agent_refs::kind_of("connection")
                .ok_or_else(|| UiError::new("failed", "connection directory unavailable"))?;
            let mut v = crate::agent_refs::directory_json(ctx, auth, kind, None, Some(&session.workspace_id))
                .await
                .map_err(fail)?;
            if let Some(q) = args.get("query").and_then(Value::as_str).map(str::to_lowercase).filter(|q| !q.is_empty()) {
                if let Some(items) = v.get_mut("items").and_then(Value::as_array_mut) {
                    items.retain(|it| {
                        ["name", "kind"].iter().any(|k| {
                            it.get(*k).and_then(Value::as_str).is_some_and(|s| s.to_lowercase().contains(&q))
                        })
                    });
                }
            }
            Ok(v)
        }
        "db_mcp_query" => {
            let conn = args.get("connection_id").and_then(Value::as_str);
            let stmt = args.get("statement").and_then(Value::as_str).filter(|s| !s.trim().is_empty());
            let (Some(conn), Some(stmt)) = (conn, stmt) else {
                return Err(UiError::new(
                    "no_ui_client",
                    "no Otto window is open, and running headless needs `connection_id` + `statement` \
                     (a `tab_id` only exists in a window)",
                ));
            };
            // The engine-aware read-only path (`run_read_only`: classifier +
            // the engine's read-only mode + masking + row cap) — the same
            // route stdio `otto_db_query` uses, so a Redis `GET` / Mongo
            // `find` works headless too. `connection_id` was already resolved
            // (and RBAC pre-checked) as the owner; the route re-checks Viewer.
            let mut q = json!({"statement": stmt, "max_rows": MAX_AGENT_ROWS});
            if let Some(db) = args.get("database").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                q["node"] = json!(format!("db:{db}"));
            }
            let path = format!(
                "/api/v1/connections/{}/db/mcp-query",
                crate::mcp_outward::seg(conn)
            );
            let timeout = deadline_for(
                ui_commands::get("db_run_query").unwrap_or_else(|| &ui_commands::catalog()[0]),
                args,
            );
            self_post(ctx, &auth.effective_user, &path, &q, timeout).await.map_err(fail)
        }
        other => Err(UiError::new("failed", format!("unknown headless fallback '{other}'"))),
    }
}

/// POST `path` (under the daemon's own base URL) AS `user`, with a
/// short-lived token revoked on the way out — so the route's native RBAC
/// decides, exactly like the governed tools' self-calls.
async fn self_post(
    ctx: &ServerCtx,
    user: &otto_core::domain::User,
    path: &str,
    body: &Value,
    timeout: Duration,
) -> Result<Value, Error> {
    let (token, _) = otto_rbac::AuthRepo::new(ctx.pool.clone())
        .issue_api_token(&user.id, Some("mcp-otto-ui"))
        .await?;
    let result = async {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| Error::Internal(format!("http client: {e}")))?;
        let resp = client
            .post(format!("{}{path}", ctx.base_url.trim_end_matches('/')))
            .bearer_auth(&token)
            .header("X-Otto-Agent", "mcp-outward")
            .json(body)
            .send()
            .await
            .map_err(|e| Error::Upstream(format!("self-call: {e}")))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            let msg = crate::mcp_outward::self_call_error(status, &text);
            return Err(match status.as_u16() {
                400 | 422 => Error::Invalid(msg),
                403 => Error::Forbidden(msg),
                404 => Error::NotFound(msg),
                409 => Error::Conflict(msg),
                _ => Error::Upstream(msg),
            });
        }
        Ok(crate::mcp_outward::parse_self_ok(&text))
    }
    .await;
    let _ = otto_rbac::AuthRepo::new(ctx.pool.clone()).revoke(&token).await;
    result
}

/// Session removed → its pending commands end. Spawned once at boot.
pub fn spawn_session_watch(ctx: ServerCtx) {
    let mut rx = ctx.events.subscribe();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(Event::SessionRemoved { session_id, .. }) => ctx.ui_bridge.cancel_session(
                    &session_id,
                    "session_removed",
                    UiError::new("cancelled_by_user", "the session was removed"),
                ),
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(conn: &str, user: &str, device: &str, window: &str, pane: Pane, module: &str) -> DocInfo {
        DocInfo {
            conn_id: conn.into(),
            user_id: user.into(),
            client_id: device.into(),
            window_id: window.into(),
            pane,
            host_window_id: (pane == Pane::Side).then(|| window.to_string()),
            route: module.into(),
            module: module.into(),
            focused: false,
            visible: true,
            capabilities: ["open", "state", "db_run_query"].iter().map(|s| s.to_string()).collect(),
            activity: 0,
        }
    }

    // ---- pick ranking ------------------------------------------------------

    #[test]
    fn pick_prefers_the_document_showing_the_module() {
        let docs = vec![
            doc("main", "u", "dev", "w1", Pane::Main, "agents"),
            doc("side", "u", "dev", "w1", Pane::Side, "connections"),
        ];
        assert_eq!(pick(&docs, "u", Some("dev"), "connections", "db_run_query"), Pick::Direct("side".into()));
    }

    #[test]
    fn pick_opens_through_the_main_document_when_nothing_shows_it() {
        let mut a = doc("main-a", "u", "dev", "wa", Pane::Main, "agents");
        let mut b = doc("main-b", "u", "dev", "wb", Pane::Main, "git");
        a.activity = 5;
        b.focused = true;
        let docs = vec![a, b];
        assert_eq!(
            pick(&docs, "u", Some("dev"), "connections", "db_run_query"),
            Pick::Open { via: "main-b".into(), window_id: "wb".into() }
        );
    }

    #[test]
    fn pick_ranks_focused_then_visible_then_recent() {
        let mut a = doc("a", "u", "dev", "w1", Pane::Main, "connections");
        let mut b = doc("b", "u", "dev", "w2", Pane::Main, "connections");
        let mut c = doc("c", "u", "dev", "w3", Pane::Main, "connections");
        a.visible = false;
        a.activity = 100;
        b.activity = 1;
        c.activity = 2;
        let docs = vec![a.clone(), b.clone(), c.clone()];
        assert_eq!(pick(&docs, "u", Some("dev"), "connections", "db_run_query"), Pick::Direct("c".into()));
        b.focused = true;
        let docs = vec![a, b, c];
        assert_eq!(pick(&docs, "u", Some("dev"), "connections", "db_run_query"), Pick::Direct("b".into()));
    }

    #[test]
    fn pick_never_crosses_users() {
        let docs = vec![doc("theirs", "other", "dev", "w", Pane::Main, "connections")];
        assert_eq!(pick(&docs, "u", Some("dev"), "connections", "db_run_query"), Pick::None);
        assert_eq!(pick(&docs, "u", None, "connections", "db_run_query"), Pick::None);
    }

    #[test]
    fn pick_stays_on_the_sessions_device() {
        // The session's device has a window (not showing the module): open
        // there, even though another device shows the module focused.
        let mut elsewhere = doc("phone", "u", "phone", "wp", Pane::Main, "connections");
        elsewhere.focused = true;
        let docs = vec![doc("laptop", "u", "laptop", "wl", Pane::Main, "agents"), elsewhere.clone()];
        assert_eq!(
            pick(&docs, "u", Some("laptop"), "connections", "db_run_query"),
            Pick::Open { via: "laptop".into(), window_id: "wl".into() }
        );
        // No window at all on the session's device: another device only when
        // it already shows the module AND is focused…
        let docs = vec![elsewhere.clone()];
        assert_eq!(pick(&docs, "u", Some("laptop"), "connections", "db_run_query"), Pick::Direct("phone".into()));
        // …never when merely visible, and never an auto-open there.
        let mut unfocused = elsewhere.clone();
        unfocused.focused = false;
        assert_eq!(pick(&[unfocused], "u", Some("laptop"), "connections", "db_run_query"), Pick::None);
        let mut other_module = elsewhere;
        other_module.module = "git".into();
        assert_eq!(pick(&[other_module], "u", Some("laptop"), "connections", "db_run_query"), Pick::None);
    }

    #[test]
    fn pick_requires_the_capability() {
        let mut old = doc("old", "u", "dev", "w", Pane::Side, "connections");
        old.capabilities.remove("db_run_query");
        let main = doc("main", "u", "dev", "w", Pane::Main, "agents");
        // The side pane lacks the command; the main can open — but opening
        // would land on the same stale document, which `opened_target` skips.
        let docs = vec![old.clone(), main];
        assert!(matches!(pick(&docs, "u", Some("dev"), "connections", "db_run_query"), Pick::Open { .. }));
        assert_eq!(opened_target(&docs, "u", "w", "connections", "db_run_query"), None);
        // Nothing can open either → None.
        let mut main = doc("main", "u", "dev", "w", Pane::Main, "agents");
        main.capabilities.clear();
        assert_eq!(pick(&[old, main], "u", Some("dev"), "connections", "db_run_query"), Pick::None);
    }

    #[test]
    fn pick_shell_commands_prefer_the_main_pane() {
        let docs = vec![
            doc("side", "u", "dev", "w", Pane::Side, "connections"),
            doc("main", "u", "dev", "w", Pane::Main, "agents"),
        ];
        assert_eq!(pick(&docs, "u", Some("dev"), "shell", "state"), Pick::Direct("main".into()));
        let mut side_focused = docs.clone();
        side_focused[0].focused = true;
        assert_eq!(pick(&side_focused, "u", Some("dev"), "shell", "state"), Pick::Direct("side".into()));
        // A shell command never auto-opens.
        let mut none = docs;
        for d in &mut none {
            d.capabilities.clear();
        }
        assert_eq!(pick(&none, "u", Some("dev"), "shell", "state"), Pick::None);
    }

    #[test]
    fn pick_with_unknown_device_uses_any_of_the_owners_windows() {
        let docs = vec![doc("a", "u", "x", "w", Pane::Main, "agents")];
        assert!(matches!(pick(&docs, "u", None, "connections", "db_run_query"), Pick::Open { .. }));
    }

    #[test]
    fn opened_target_matches_the_window_or_its_side_pane() {
        let docs = vec![
            doc("main", "u", "dev", "w1", Pane::Main, "agents"),
            doc("side", "u", "dev", "w1", Pane::Side, "connections"),
            doc("other", "u", "dev", "w2", Pane::Main, "connections"),
        ];
        assert_eq!(opened_target(&docs, "u", "w1", "connections", "db_run_query"), Some("side".into()));
        assert_eq!(opened_target(&docs, "u", "w9", "connections", "db_run_query"), None);
        assert_eq!(opened_target(&docs, "x", "w1", "connections", "db_run_query"), None);
    }

    // ---- grants ------------------------------------------------------------

    #[test]
    fn grant_table() {
        assert_eq!(grant_state(&json!({})), None);
        assert_eq!(grant_state(&Value::Null), None);
        assert_eq!(grant_state(&json!({"ui_control": {"enabled": true}})), Some(true));
        assert_eq!(grant_state(&json!({"ui_control": {"enabled": false}})), Some(false));
        // Not a bool → not granted (fail closed).
        assert_eq!(grant_state(&json!({"ui_control": {"enabled": "yes"}})), None);
        assert_eq!(grant_state(&json!({"ui_control": true})), None);
        let rec = grant_record(true, "alice");
        assert_eq!(grant_state(&rec), Some(true));
        assert_eq!(rec["ui_control"]["granted_by"], json!("alice"));
        let now = chrono::Utc::now();
        let denied = grant_record(false, "alice");
        assert!(recently_denied(&denied, now));
        assert!(!recently_denied(&denied, now + chrono::Duration::seconds(61)));
        assert!(!recently_denied(&rec, now));
        assert!(!recently_denied(&json!({"ui_control": {"enabled": false}}), now));
    }

    #[test]
    fn human_credential_table() {
        let u = otto_core::domain::User {
            id: "u".into(),
            username: "u".into(),
            display_name: "u".into(),
            is_root: false,
            disabled: false,
            created_at: chrono::Utc::now(),
        };
        let base = AuthContext {
            real_user: u.clone(),
            effective_user: u,
            scope: None,
            mcp_only: false,
            mcp_scope: None,
            mcp_internal: false,
            mcp_session_id: None,
            managed_session_id: None,
        };
        assert!(is_human(&base));
        assert!(calling_session(&base).is_none());
        let mut managed = base.clone();
        managed.managed_session_id = Some("s".into());
        assert!(!is_human(&managed));
        assert_eq!(calling_session(&managed).map(String::as_str), Some("s"));
        let mut internal = base.clone();
        internal.mcp_only = true;
        internal.mcp_internal = true;
        internal.mcp_session_id = Some("s2".into());
        assert!(!is_human(&internal));
        assert_eq!(calling_session(&internal).map(String::as_str), Some("s2"));
        let mut mcp = base.clone();
        mcp.mcp_only = true;
        assert!(!is_human(&mcp));
        let mut share = base;
        share.scope = Some(otto_core::auth::SessionScope {
            session_id: "s".into(),
            role: otto_core::domain::WorkspaceRole::Viewer,
            otp_pending: false,
        });
        assert!(!is_human(&share));
    }

    // ---- frames --------------------------------------------------------------

    #[test]
    fn client_frames_parse_strictly() {
        let hello = r#"{"type":"hello","client_id":"c","window_id":"w","pane":"side","host_window_id":"h",
            "route":"database","module":"connections","focused":true,"visible":true,"capabilities":["db_run_query"]}"#;
        assert!(matches!(parse_client_frame(hello), Some(ClientFrame::Hello(_))));
        assert!(matches!(
            parse_client_frame(r#"{"type":"presence","route":"git","module":"git","focused":false,"visible":true}"#),
            Some(ClientFrame::Presence(_))
        ));
        // Unknown type, junk, wrong types, bad pane, missing ids → ignored.
        assert!(parse_client_frame(r#"{"type":"subscribe","x":1}"#).is_none());
        assert!(parse_client_frame("not json").is_none());
        assert!(parse_client_frame(r#"{"type":"hello","client_id":"c","window_id":"w","pane":"top"}"#).is_none());
        assert!(parse_client_frame(r#"{"type":"hello","client_id":"","window_id":"w","pane":"main"}"#).is_none());
        assert!(parse_client_frame(r#"{"type":"presence","focused":"yes"}"#).is_none());
        // Oversized / out-of-bounds fields.
        let big = format!(r#"{{"type":"presence","route":"{}"}}"#, "a".repeat(MAX_CLIENT_FRAME));
        assert!(parse_client_frame(&big).is_none());
        let long_id = format!(r#"{{"type":"hello","client_id":"{}","window_id":"w","pane":"main"}}"#, "c".repeat(200));
        assert!(parse_client_frame(&long_id).is_none());
        let ctrl = r#"{"type":"hello","client_id":"c\n","window_id":"w","pane":"main"}"#;
        assert!(parse_client_frame(ctrl).is_none());
    }

    // ---- shaping -------------------------------------------------------------

    #[test]
    fn results_are_row_capped_and_redacted() {
        let rows: Vec<Value> = (0..250).map(|i| json!([i])).collect();
        let v = shape_result(json!({"rows": rows, "note": "key AKIAIOSFODNN7EXAMPLE"}));
        assert_eq!(v["rows"].as_array().unwrap().len(), MAX_AGENT_ROWS);
        assert_eq!(v["rows_truncated"], json!(250));
        assert!(!v["note"].as_str().unwrap().contains("AKIAIOSFODNN7EXAMPLE"), "{v}");
        let small = shape_result(json!({"rows": [[1]]}));
        assert!(small.get("rows_truncated").is_none());
        assert_eq!(with_visibility(json!({"a":1}), true), json!({"a":1,"ui_visible":true}));
        assert_eq!(with_visibility(json!(5), false), json!({"ui_visible":false,"result":5}));
    }

    #[test]
    fn deadlines_follow_the_catalog_and_the_statement_timeout() {
        let run = ui_commands::get("db_run_query").unwrap();
        assert_eq!(deadline_for(run, &json!({})), Duration::from_millis(run.timeout_ms));
        assert_eq!(deadline_for(run, &json!({"timeout_ms": 60_000})), Duration::from_millis(75_000));
        assert_eq!(deadline_for(run, &json!({"timeout_ms": 1_000})), Duration::from_millis(run.timeout_ms));
        assert_eq!(deadline_for(run, &json!({"timeout_ms": 105_000})), Duration::from_millis(120_000));
        // A command without a timeout_ms property ignores a stray one.
        let st = ui_commands::get("state").unwrap();
        assert_eq!(deadline_for(st, &json!({"timeout_ms": 90_000})), Duration::from_millis(st.timeout_ms));
    }

    #[test]
    fn reply_codes_normalize_to_the_contract() {
        for c in ["cancelled_by_user", "invalid_args", "not_found", "forbidden"] {
            assert_eq!(normalize_code(c), c);
        }
        assert_eq!(normalize_code("pending_grant"), "failed");
        assert_eq!(normalize_code("boom"), "failed");
    }

    // ---- registry lifecycle ---------------------------------------------------

    fn session(id: &str, owner: &str) -> Session {
        Session {
            id: id.into(),
            workspace_id: "ws".into(),
            kind: otto_core::domain::SessionKind::Agent,
            provider: "claude".into(),
            title: "Fix the report".into(),
            status: otto_core::domain::SessionStatus::Running,
            cwd: "/tmp".into(),
            provider_session_id: None,
            connection_id: None,
            created_by: owner.into(),
            created_at: chrono::Utc::now(),
            last_active_at: chrono::Utc::now(),
            archived: false,
            meta: json!({}),
        }
    }

    fn hello(bridge: &UiBridge, conn: &str, rx: &mut mpsc::Receiver<String>) {
        bridge.on_client_frame(
            conn,
            r#"{"type":"hello","client_id":"dev","window_id":"w","pane":"side","route":"database",
                "module":"connections","focused":true,"visible":true,"capabilities":["db_run_query"]}"#,
        );
        let ack: Value = serde_json::from_str(&rx.try_recv().expect("hello_ack")).unwrap();
        assert_eq!(ack, json!({"type":"hello_ack","conn_id":conn}));
    }

    fn next_frame(rx: &mut mpsc::Receiver<String>) -> Value {
        serde_json::from_str(&rx.try_recv().expect("a frame")).unwrap()
    }

    #[tokio::test]
    async fn register_hello_and_unregister() {
        let b = UiBridge::default();
        let (conn, mut rx) = b.register(&"u".to_string());
        assert!(b.docs().is_empty(), "not a target before hello");
        hello(&b, &conn, &mut rx);
        assert_eq!(b.docs().len(), 1);
        b.on_client_frame(&conn, r#"{"type":"presence","route":"git","module":"git","focused":false,"visible":true}"#);
        assert_eq!(b.docs()[0].module, "git");
        assert!(b.user_has_docs("u"));
        b.unregister(&conn);
        assert!(b.docs().is_empty());
        assert!(!b.user_has_docs("u"));
        // Frames for an unknown connection are ignored.
        b.on_client_frame("nope", r#"{"type":"presence","route":"x","module":"x"}"#);
    }

    #[tokio::test]
    async fn command_round_trip_and_reply_auth() {
        let b = std::sync::Arc::new(UiBridge::default());
        let (conn, mut rx) = b.register(&"u".to_string());
        hello(&b, &conn, &mut rx);
        let s = session("s1", "u");
        let (id, orx) = b
            .send_command(&conn, &s, "db_run_query", &json!({"tab_id":"t"}), Duration::from_secs(5))
            .unwrap();
        let f = next_frame(&mut rx);
        assert_eq!(f["type"], "ui_command");
        assert_eq!(f["id"], json!(id));
        assert_eq!(f["session_id"], "s1");
        assert_eq!(f["agent"], json!({"session_id":"s1","title":"Fix the report","provider":"claude"}));
        assert_eq!(f["command"], "db_run_query");
        assert_eq!(f["args"], json!({"tab_id":"t"}));
        assert_eq!(f["deadline_ms"], 5000);

        let ok = || UiReply { ok: true, result: Some(json!({"rows":[[1]]})), error: None };
        // Wrong user, wrong / missing connection → refused, command still pending.
        assert!(matches!(b.complete(&id, "mallory", Some(&conn), ok()), Err(Error::Forbidden(_))));
        assert!(matches!(b.complete(&id, "u", Some("other-conn"), ok()), Err(Error::Forbidden(_))));
        assert!(matches!(b.complete(&id, "u", None, ok()), Err(Error::Forbidden(_))));
        assert!(matches!(b.progress(&id, "mallory", Some(&conn), "x", true), Err(Error::Forbidden(_))));
        // The right user + connection completes it.
        let waiter = {
            let b = b.clone();
            let id = id.clone();
            tokio::spawn(async move { b.await_outcome(&id, orx).await })
        };
        b.complete(&id, "u", Some(&conn), ok()).unwrap();
        assert_eq!(waiter.await.unwrap().unwrap(), json!({"rows":[[1]]}));
        // A second reply finds nothing.
        assert!(matches!(b.complete(&id, "u", Some(&conn), ok()), Err(Error::NotFound(_))));
    }

    #[tokio::test]
    async fn ui_errors_reach_the_agent_with_normalized_codes() {
        let b = std::sync::Arc::new(UiBridge::default());
        let (conn, mut rx) = b.register(&"u".to_string());
        hello(&b, &conn, &mut rx);
        let (id, orx) = b
            .send_command(&conn, &session("s", "u"), "db_run_query", &json!({}), Duration::from_secs(5))
            .unwrap();
        b.complete(
            &id,
            "u",
            Some(&conn),
            UiReply {
                ok: false,
                result: None,
                error: Some(UiReplyError { code: "cancelled_by_user".into(), message: "Cancelled".into() }),
            },
        )
        .unwrap();
        let e = b.await_outcome(&id, orx).await.unwrap_err();
        assert_eq!(e, UiError::new("cancelled_by_user", "Cancelled"));
    }

    #[tokio::test]
    async fn timeout_sends_a_cancel_frame() {
        let b = UiBridge::default();
        let (conn, mut rx) = b.register(&"u".to_string());
        hello(&b, &conn, &mut rx);
        let (id, orx) = b
            .send_command(&conn, &session("s", "u"), "db_run_query", &json!({}), Duration::from_millis(300))
            .unwrap();
        let _cmd = next_frame(&mut rx);
        b.progress(&id, "u", Some(&conn), "running", false).unwrap();
        let e = b.await_outcome(&id, orx).await.unwrap_err();
        assert_eq!(e.code, "timeout");
        assert!(e.message.contains("running"), "{}", e.message);
        let cancel = next_frame(&mut rx);
        assert_eq!(cancel, json!({"type":"ui_command_cancel","id":id,"reason":"timeout"}));
        assert!(b.pending.lock().unwrap().is_empty());
    }

    #[test]
    fn awaiting_human_extends_the_deadline_within_the_cap() {
        let t0 = Instant::now();
        let s = Duration::from_secs;
        // First confirm at dispatch: 10 s budget → now + 120 s.
        assert_eq!(extended_deadline(t0, t0, t0 + s(10)), t0 + s(120));
        // A later confirm cannot push past dispatch + 150 s…
        assert_eq!(extended_deadline(t0 + s(110), t0, t0 + s(120)), t0 + s(150));
        // …and never shortens a longer deadline.
        assert_eq!(extended_deadline(t0 + s(1), t0, t0 + s(140)), t0 + s(140));
    }

    #[tokio::test]
    async fn awaiting_human_progress_keeps_the_command_alive() {
        let b = std::sync::Arc::new(UiBridge::default());
        let (conn, mut rx) = b.register(&"u".to_string());
        hello(&b, &conn, &mut rx);
        let (id, orx) = b
            .send_command(&conn, &session("s", "u"), "db_run_query", &json!({}), Duration::from_millis(200))
            .unwrap();
        b.progress(&id, "u", Some(&conn), "confirm write", true).unwrap();
        let waiter = {
            let b = b.clone();
            let id = id.clone();
            tokio::spawn(async move { b.await_outcome(&id, orx).await })
        };
        // Well past the original 200 ms budget: still pending, then answered.
        tokio::time::sleep(Duration::from_millis(600)).await;
        assert!(b.pending.lock().unwrap().contains_key(&id));
        b.complete(&id, "u", Some(&conn), UiReply { ok: true, result: Some(json!({"done":true})), error: None })
            .unwrap();
        assert_eq!(waiter.await.unwrap().unwrap(), json!({"done":true}));
    }

    #[tokio::test]
    async fn revoke_and_close_end_pending_commands() {
        let b = std::sync::Arc::new(UiBridge::default());
        let (conn, mut rx) = b.register(&"u".to_string());
        hello(&b, &conn, &mut rx);
        let (id, orx) = b
            .send_command(&conn, &session("s", "u"), "db_run_query", &json!({}), Duration::from_secs(30))
            .unwrap();
        let _ = next_frame(&mut rx);
        b.cancel_session("s", "revoked", UiError::new("cancelled_by_user", "stopped"));
        assert_eq!(b.await_outcome(&id, orx).await.unwrap_err().code, "cancelled_by_user");
        assert_eq!(next_frame(&mut rx), json!({"type":"ui_command_cancel","id":id,"reason":"revoked"}));

        let (id2, orx2) = b
            .send_command(&conn, &session("s", "u"), "db_run_query", &json!({}), Duration::from_secs(30))
            .unwrap();
        b.unregister(&conn);
        let e = b.await_outcome(&id2, orx2).await.unwrap_err();
        assert_eq!(e.code, "no_ui_client");
        // Sending to a closed connection fails fast and leaves nothing pending.
        let e = b
            .send_command(&conn, &session("s", "u"), "db_run_query", &json!({}), Duration::from_secs(30))
            .unwrap_err();
        assert_eq!(e.code, "no_ui_client");
        assert!(b.pending.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn dropping_the_agent_call_cancels_the_command() {
        let b = std::sync::Arc::new(UiBridge::default());
        let (conn, mut rx) = b.register(&"u".to_string());
        hello(&b, &conn, &mut rx);
        let (id, orx) = b
            .send_command(&conn, &session("s", "u"), "db_run_query", &json!({}), Duration::from_secs(30))
            .unwrap();
        let _ = next_frame(&mut rx);
        let task = {
            let b = b.clone();
            let id = id.clone();
            tokio::spawn(async move { b.await_outcome(&id, orx).await })
        };
        tokio::task::yield_now().await;
        task.abort();
        let _ = task.await;
        assert_eq!(next_frame(&mut rx), json!({"type":"ui_command_cancel","id":id,"reason":"caller_gone"}));
        assert!(b.pending.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn request_event_is_rate_limited_per_session() {
        let b = UiBridge::default();
        assert!(b.should_request("s"));
        assert!(!b.should_request("s"));
        assert!(b.should_request("t"));
    }
}
