//! js-debug child sessions. js-debug debugs every process of a run (the
//! Playwright CLI and its forked worker, a Karma Chrome page) as a target of
//! its own and announces each with a `startDebugging` reverse request, on the
//! connection of the target's parent. Otto opens one more DAP connection per
//! target to the same js-debug server and launches it with the request's
//! `configuration` (which carries `__pendingTargetId`).
//!
//! One target is in focus: commands and the Debugger panel use it. A stop of
//! another target takes the focus when the focus is running; while the focus
//! is paused it waits in a queue, and the focus moves to it when the focused
//! target resumes or ends. Breakpoints go to every connection; one counts as
//! verified once any connection verified it.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use otto_core::{Error, Result};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

use super::session::{mark_resumed, ActiveConnection, DapWireClient, SessionEntry};
use super::session_inspect::{apply_breakpoint_event, handle_stopped_event, sync_all_breakpoints};
use super::wire::DapEvent;
use crate::types::IdeDebugTarget;

/// The child targets of one js-debug session.
#[derive(Default)]
pub(super) struct ChildTargets {
    /// Where js-debug listens, as Otto reaches it (the SSH tunnel's local end
    /// on a remote target); `None` for every other adapter.
    endpoint: Mutex<Option<(String, u16)>>,
    state: Mutex<TargetsState>,
}

#[derive(Default)]
struct TargetsState {
    live: Vec<ChildTarget>,
    focus: Option<String>,
    focus_paused: bool,
    /// Targets stopped while the focus was paused, oldest first.
    queue: VecDeque<String>,
    failed: Vec<IdeDebugTarget>,
}

struct ChildTarget {
    id: String,
    name: String,
    conn: Arc<ActiveConnection>,
    /// A stop waiting for the focus: `(reason, thread id)`.
    pending_stop: Option<(String, Option<i64>)>,
}

impl ChildTargets {
    pub(super) fn enable(&self, host: String, port: u16) {
        *self.endpoint.lock().unwrap_or_else(|e| e.into_inner()) = Some((host, port));
    }

    /// This session runs on js-debug and debugs its processes as child targets.
    pub(super) fn enabled(&self) -> bool {
        self.endpoint
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }

    fn endpoint(&self) -> Option<(String, u16)> {
        self.endpoint
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, TargetsState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(super) fn focused(&self) -> Option<Arc<ActiveConnection>> {
        let st = self.lock();
        let id = st.focus.as_deref()?;
        st.live.iter().find(|c| c.id == id).map(|c| c.conn.clone())
    }

    pub(super) fn connections(&self) -> Vec<Arc<ActiveConnection>> {
        self.lock().live.iter().map(|c| c.conn.clone()).collect()
    }

    pub(super) fn snapshot(&self) -> Vec<IdeDebugTarget> {
        let st = self.lock();
        let mut out: Vec<IdeDebugTarget> = st
            .live
            .iter()
            .map(|c| IdeDebugTarget {
                id: c.id.clone(),
                name: c.name.clone(),
                paused: c.pending_stop.is_some()
                    || (st.focus_paused && st.focus.as_deref() == Some(c.id.as_str())),
                error: None,
            })
            .collect();
        out.extend(st.failed.iter().cloned());
        out
    }

    /// Teardown: every child's wire is retired so its driver stops acting.
    pub(super) fn clear(&self) {
        let mut st = self.lock();
        for child in st.live.drain(..) {
            child.conn.wire.retire();
            child.conn.wire.abort_pending();
        }
        *st = TargetsState::default();
        *self.endpoint.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

fn broadcast_targets(entry: &SessionEntry) {
    let targets = entry.targets.snapshot();
    entry.mutate_and_broadcast(|snap| snap.targets = targets);
}

/// js-debug names a Node target after its script (`workerProcessEntry.js [4242]`)
/// and a page after its title (`about:blank` when it attaches).
fn display_name(configuration: &Value) -> String {
    let raw = configuration
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("process");
    let pid = raw.rfind(" [").map_or("", |i| &raw[i..]);
    if configuration.get("type").and_then(Value::as_str) == Some("pwa-chrome") {
        "Karma: Chrome".into()
    } else if raw.starts_with("workerProcessEntry.js") {
        format!("Playwright worker{pid}")
    } else if raw.starts_with("cli.js") {
        format!("Playwright{pid}")
    } else {
        raw.to_string()
    }
}

/// A `startDebugging` reverse request from `parent`: attach the child, then answer.
pub(super) fn on_start_debugging(
    entry: &Arc<SessionEntry>,
    parent: Arc<DapWireClient>,
    ev: DapEvent,
) {
    let entry = entry.clone();
    tokio::spawn(async move {
        let configuration = ev
            .body
            .as_ref()
            .and_then(|b| b.get("configuration"))
            .cloned()
            .unwrap_or_else(|| json!({}));
        let name = display_name(&configuration);
        let result = attach_child(&entry, configuration).await;
        let reply = result.as_ref().map(|_| None).map_err(ToString::to_string);
        let _ = parent.respond(ev.seq, "startDebugging", reply).await;
        if let Err(err) = result {
            if parent.is_retired() || !entry.targets.enabled() {
                return;
            }
            entry.append_output("stderr", format!("{name} not attached: {err}\n"));
            entry.targets.lock().failed.push(IdeDebugTarget {
                id: format!("failed-{}", ev.seq),
                name,
                paused: false,
                error: Some(err.to_string()),
            });
            broadcast_targets(&entry);
        }
    });
}

async fn attach_child(entry: &Arc<SessionEntry>, configuration: Value) -> Result<()> {
    let (host, port) = entry
        .targets
        .endpoint()
        .ok_or_else(|| Error::Conflict("debug session is no longer connected".into()))?;
    let guard = entry.active_root()?.guard.clone();
    let tcp = TcpStream::connect((host.as_str(), port))
        .await
        .map_err(|e| Error::Upstream(format!("connect js-debug {host}:{port}: {e}")))?;
    let _ = tcp.set_nodelay(true);
    let (r, w) = tcp.into_split();
    let (ev_tx, ev_rx) = mpsc::unbounded_channel();
    let wire = DapWireClient::spawn(Box::new(r), Box::new(w), ev_tx);
    let adapter_id = configuration
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("pwa-node");
    let init = wire
        .request(
            "initialize",
            Some(json!({
                "adapterID": adapter_id,
                "linesStartAt1": true,
                "columnsStartAt1": true,
                "pathFormat": "path",
                "supportsStartDebuggingRequest": true,
            })),
        )
        .await?;
    let caps = init.body.unwrap_or_default();
    let flag = |k: &str| caps.get(k).and_then(Value::as_bool).unwrap_or(false);
    let conn = Arc::new(ActiveConnection {
        wire: wire.clone(),
        guard,
        supports_terminate: flag("supportsTerminateRequest"),
        supports_set_variable: flag("supportsSetVariable"),
        exception_filters: caps
            .get("exceptionBreakpointFilters")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|f| f.get("filter").and_then(Value::as_str).map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        attached: false,
    });
    let id = configuration
        .get("__pendingTargetId")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let name = display_name(&configuration);
    // js-debug answers `launch` once the target is configured, so it is sent
    // first and awaited after `configurationDone`.
    let launch = wire
        .send_request_nowait("launch", Some(configuration))
        .await?;
    sync_all_breakpoints(entry, &conn).await?;
    wire.request("configurationDone", Some(json!({}))).await?;
    if !entry.targets.enabled() {
        wire.retire();
        return Err(Error::Conflict("debug session ended".into()));
    }
    {
        let mut st = entry.targets.lock();
        st.live.push(ChildTarget {
            id: id.clone(),
            name: name.clone(),
            conn: conn.clone(),
            pending_stop: None,
        });
        // The newest process takes the focus (Pause reaches the Playwright
        // worker, not its idle CLI) unless the focus is stopped.
        if !st.focus_paused {
            st.focus = Some(id.clone());
        }
    }
    entry.append_output("console", format!("Attached to {name}\n"));
    broadcast_targets(entry);
    spawn_child_driver(entry.clone(), id, conn, ev_rx);
    tokio::spawn(async move {
        let _ = launch.await;
    });
    Ok(())
}

fn spawn_child_driver(
    entry: Arc<SessionEntry>,
    id: String,
    conn: Arc<ActiveConnection>,
    mut ev_rx: mpsc::UnboundedReceiver<DapEvent>,
) {
    tokio::spawn(async move {
        while let Some(ev) = ev_rx.recv().await {
            if conn.wire.is_retired() {
                break;
            }
            let body = ev.body.as_ref();
            match ev.event.as_str() {
                "__startDebugging__" => on_start_debugging(&entry, conn.wire.clone(), ev),
                "breakpoint" => {
                    if let Some(body) = body {
                        apply_breakpoint_event(&entry, &conn.wire.bp_ids, body);
                    }
                }
                "output" => {
                    let cat = body
                        .and_then(|b| b.get("category"))
                        .and_then(Value::as_str)
                        .unwrap_or("stdout");
                    if let Some(text) = body
                        .and_then(|b| b.get("output"))
                        .and_then(Value::as_str)
                        .filter(|_| cat != "telemetry")
                    {
                        entry.append_output(cat, text.to_string());
                        if text.contains("Executable doesn't exist") {
                            entry.append_output(
                                "console",
                                "Install browsers with npx playwright install.\n".into(),
                            );
                        }
                    }
                }
                "stopped" => {
                    let reason = body
                        .and_then(|b| b.get("reason"))
                        .and_then(Value::as_str)
                        .unwrap_or("breakpoint")
                        .to_string();
                    let tid = body.and_then(|b| b.get("threadId")).and_then(Value::as_i64);
                    on_child_stopped(&entry, &id, reason, tid).await;
                }
                "continued" => on_child_continued(&entry, &id).await,
                "terminated" | "__eof__" => {
                    on_child_ended(&entry, &id).await;
                    break;
                }
                _ => {}
            }
        }
    });
}

async fn on_child_stopped(entry: &SessionEntry, id: &str, reason: String, tid: Option<i64>) {
    let take_focus = {
        let mut st = entry.targets.lock();
        let others_paused = st.focus_paused && st.focus.as_deref() != Some(id);
        let st = &mut *st;
        let Some(child) = st.live.iter_mut().find(|c| c.id == id) else {
            return;
        };
        if others_paused {
            child.pending_stop = Some((reason.clone(), tid));
            st.queue.push_back(id.to_string());
            None
        } else {
            child.pending_stop = None;
            st.focus = Some(id.to_string());
            st.focus_paused = true;
            Some(child.conn.clone())
        }
    };
    broadcast_targets(entry);
    if let Some(conn) = take_focus {
        handle_stopped_event(entry, &conn, reason, tid).await;
    }
}

/// The next queued stop takes the focus: `(connection, reason, thread id)`.
fn next_queued_stop(st: &mut TargetsState) -> Option<(Arc<ActiveConnection>, String, Option<i64>)> {
    while let Some(next) = st.queue.pop_front() {
        if let Some(child) = st.live.iter_mut().find(|c| c.id == next) {
            if let Some((reason, tid)) = child.pending_stop.take() {
                st.focus = Some(next);
                st.focus_paused = true;
                return Some((child.conn.clone(), reason, tid));
            }
        }
    }
    None
}

async fn on_child_continued(entry: &SessionEntry, id: &str) {
    let next = {
        let mut st = entry.targets.lock();
        if st.focus.as_deref() != Some(id) {
            // A queued target that resumed on its own no longer waits.
            if let Some(child) = st.live.iter_mut().find(|c| c.id == id) {
                child.pending_stop = None;
            }
            st.queue.retain(|q| q != id);
            return;
        }
        if !st.focus_paused {
            return;
        }
        st.focus_paused = false;
        next_queued_stop(&mut st)
    };
    mark_resumed(entry);
    broadcast_targets(entry);
    if let Some((conn, reason, tid)) = next {
        handle_stopped_event(entry, &conn, reason, tid).await;
    }
}

async fn on_child_ended(entry: &SessionEntry, id: &str) {
    let (was_paused, next) = {
        let mut st = entry.targets.lock();
        if let Some(pos) = st.live.iter().position(|c| c.id == id) {
            let child = st.live.remove(pos);
            child.conn.wire.retire();
        }
        st.queue.retain(|q| q != id);
        if st.focus.as_deref() != Some(id) {
            (false, None)
        } else {
            let was_paused = std::mem::take(&mut st.focus_paused);
            st.focus = st.live.last().map(|c| c.id.clone());
            (was_paused, next_queued_stop(&mut st))
        }
    };
    if was_paused {
        mark_resumed(entry);
    }
    broadcast_targets(entry);
    if let Some((conn, reason, tid)) = next {
        handle_stopped_event(entry, &conn, reason, tid).await;
    }
}

#[cfg(test)]
#[path = "session_children_tests.rs"]
mod tests;
