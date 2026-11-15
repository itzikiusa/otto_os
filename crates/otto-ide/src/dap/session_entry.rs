//! Multiplexed DAP wire client and per-session state entry (`SessionEntry`).

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use dashmap::DashMap;
use otto_core::{Error, Result};
use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{broadcast, mpsc, oneshot, Notify};

use super::session::{DapStartContext, MAX_DEBUG_OUTPUT, MAX_DEBUG_OUTPUT_BYTES};
use super::session_transport::evict_terminated_sessions;
use super::wire::{DapEvent, DapMessage, DapReader, DapResponse, DapWriter};
use super::BridgeGuard;
use crate::types::{IdeDebugOutputEntry, IdeDebugSessionSnapshot, IdeDebugState, IdeStartDebugReq};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

pub(super) type BoxRead = Box<dyn AsyncRead + Send + Unpin>;
pub(super) type BoxWrite = Box<dyn AsyncWrite + Send + Unpin>;

/// Multiplexed async DAP request/response client decoupled from event streaming.
pub(super) struct DapWireClient {
    writer: tokio::sync::Mutex<DapWriter<BoxWrite>>,
    pending: Mutex<HashMap<i64, oneshot::Sender<DapResponse>>>,
    retired: AtomicBool,
}

impl DapWireClient {
    pub(super) fn spawn(
        reader_io: BoxRead,
        writer_io: BoxWrite,
        event_tx: mpsc::UnboundedSender<DapEvent>,
    ) -> Arc<Self> {
        let client = Arc::new(Self {
            writer: tokio::sync::Mutex::new(DapWriter::new(writer_io)),
            pending: Mutex::new(HashMap::new()),
            retired: AtomicBool::new(false),
        });
        let client_ref = client.clone();
        tokio::spawn(async move {
            let mut reader = DapReader::new(reader_io);
            while let Ok(Some(msg)) = reader.next_message().await {
                match msg {
                    DapMessage::Response(resp) => {
                        let tx = client_ref
                            .pending
                            .lock()
                            .ok()
                            .and_then(|mut map| map.remove(&resp.request_seq));
                        if let Some(tx) = tx {
                            let _ = tx.send(resp);
                        }
                    }
                    DapMessage::Event(ev) => {
                        if !client_ref.retired.load(Ordering::SeqCst) {
                            let _ = event_tx.send(ev);
                        }
                    }
                    DapMessage::Request(rev_req) => {
                        let mut w = client_ref.writer.lock().await;
                        let _ = w
                            .send_response(rev_req.seq, &rev_req.command, Ok(Some(json!({}))))
                            .await;
                    }
                }
            }
            if let Ok(mut map) = client_ref.pending.lock() {
                map.clear();
            }
            if !client_ref.retired.load(Ordering::SeqCst) {
                let _ = event_tx.send(DapEvent::new(0, "__eof__", None));
            }
        });
        client
    }

    pub(super) fn retire(&self) {
        self.retired.store(true, Ordering::SeqCst);
    }

    pub(super) fn abort_pending(&self) {
        if let Ok(mut map) = self.pending.lock() {
            map.clear();
        }
    }

    pub(super) async fn send_request_nowait(
        &self,
        command: &str,
        arguments: Option<Value>,
    ) -> Result<oneshot::Receiver<DapResponse>> {
        let (tx, rx) = oneshot::channel();
        let mut w = self.writer.lock().await;
        let seq = w.alloc_seq();
        if let Ok(mut map) = self.pending.lock() {
            map.insert(seq, tx);
        }
        let msg = DapMessage::Request(super::wire::DapRequest::new(seq, command, arguments));
        if let Err(e) = w.write_message(&msg).await {
            if let Ok(mut map) = self.pending.lock() {
                map.remove(&seq);
            }
            return Err(Error::Upstream(format!(
                "DAP write '{command}' failed: {e}"
            )));
        }
        Ok(rx)
    }

    pub(super) async fn request(
        &self,
        command: &str,
        arguments: Option<Value>,
    ) -> Result<DapResponse> {
        let rx = self.send_request_nowait(command, arguments).await?;
        let resp = tokio::time::timeout(REQUEST_TIMEOUT, rx)
            .await
            .map_err(|_| Error::Upstream(format!("DAP '{command}' timed out")))?
            .map_err(|_| Error::Upstream(format!("DAP adapter closed during '{command}'")))?;
        if !resp.success {
            let reason = resp
                .message
                .unwrap_or_else(|| format!("DAP '{command}' failed"));
            return Err(Error::Invalid(reason));
        }
        Ok(resp)
    }
}

pub(super) struct ActiveConnection {
    pub(super) wire: Arc<DapWireClient>,
    pub(super) guard: Arc<BridgeGuard>,
    pub(super) supports_terminate: bool,
    pub(super) exception_filters: Vec<String>,
}

pub(super) struct SessionEntry {
    pub(super) created_seq: u64,
    pub(super) owner_user_id: String,
    pub(super) orig_req: RwLock<IdeStartDebugReq>,
    pub(super) orig_ctx: RwLock<DapStartContext>,
    pub(super) snapshot: RwLock<IdeDebugSessionSnapshot>,
    pub(super) output_buf: Mutex<VecDeque<IdeDebugOutputEntry>>,
    pub(super) configured_bp_files: Mutex<BTreeSet<String>>,
    pub(super) temp_run_to_cursor: Mutex<Option<(String, u32)>>,
    pub(super) break_on_exceptions: Mutex<bool>,
    pub(super) conn: RwLock<Option<Arc<ActiveConnection>>>,
    pub(super) events_tx: broadcast::Sender<IdeDebugSessionSnapshot>,
    pub(super) initialized_notify: Arc<Notify>,
    pub(super) halted_notify: Arc<Notify>,
    pub(super) suppress_transient_pause: AtomicBool,
    pub(super) restarting: AtomicBool,
}

impl SessionEntry {
    pub(super) fn new(
        created_seq: u64,
        (id, title, language, adapter): (String, String, String, String),
        initial_output: Vec<IdeDebugOutputEntry>,
        req: IdeStartDebugReq,
        ctx: DapStartContext,
    ) -> Arc<Self> {
        let owner_user_id = ctx.owner_user_id.clone();
        let (events_tx, _) = broadcast::channel(128);
        let mut output_buf = VecDeque::with_capacity(initial_output.len().min(MAX_DEBUG_OUTPUT));
        for item in initial_output {
            if output_buf.len() >= MAX_DEBUG_OUTPUT {
                output_buf.pop_front();
            }
            output_buf.push_back(item);
        }
        let initial_snap = IdeDebugSessionSnapshot {
            id,
            revision: 1,
            title,
            language,
            adapter,
            state: IdeDebugState::Starting,
            stopped_reason: None,
            active_thread_id: None,
            active_frame_id: None,
            paused_file: None,
            paused_line: None,
            threads: Vec::new(),
            stack_frames: Vec::new(),
            scopes: Vec::new(),
            inline_values: Vec::new(),
            breakpoints: req.breakpoints.clone(),
            output: Vec::new(),
            error: None,
        };
        Arc::new(Self {
            created_seq,
            owner_user_id,
            orig_req: RwLock::new(req),
            orig_ctx: RwLock::new(ctx),
            snapshot: RwLock::new(initial_snap),
            output_buf: Mutex::new(output_buf),
            configured_bp_files: Mutex::new(BTreeSet::new()),
            temp_run_to_cursor: Mutex::new(None),
            break_on_exceptions: Mutex::new(false),
            conn: RwLock::new(None),
            events_tx,
            initialized_notify: Arc::new(Notify::new()),
            halted_notify: Arc::new(Notify::new()),
            suppress_transient_pause: AtomicBool::new(false),
            restarting: AtomicBool::new(false),
        })
    }

    fn snapshot_output(&self) -> Vec<IdeDebugOutputEntry> {
        self.output_buf
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .cloned()
            .collect()
    }

    pub(super) fn is_terminal(&self) -> bool {
        let state = self
            .snapshot
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .state;
        matches!(state, IdeDebugState::Terminated | IdeDebugState::Error)
    }

    pub(super) fn disconnect_and_kill(&self) {
        if self.restarting.load(Ordering::SeqCst) {
            return;
        }
        self.orig_ctx
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .netbridge = None;
        if let Some(conn) = self.conn.write().unwrap_or_else(|e| e.into_inner()).take() {
            conn.wire.retire();
            conn.wire.abort_pending();
            conn.guard.kill_all();
        }
    }

    pub(super) fn mark_terminated(&self) {
        if self.restarting.load(Ordering::SeqCst) {
            return;
        }
        self.orig_ctx
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .netbridge = None;
        self.mutate_and_broadcast(|snap| {
            snap.state = IdeDebugState::Terminated;
            snap.stopped_reason = None;
            snap.paused_line = None;
            snap.threads.clear();
            snap.stack_frames.clear();
            snap.scopes.clear();
            snap.inline_values.clear();
        });
    }

    pub(super) fn fail_and_evict(
        &self,
        sessions: &DashMap<String, Arc<SessionEntry>>,
        msg: String,
    ) {
        let session_id = self
            .snapshot
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .id
            .clone();
        tracing::warn!(
            session_id = %session_id,
            error = %msg,
            "IDE debug session failed"
        );
        self.disconnect_and_kill();
        self.mutate_and_broadcast(|snap| {
            snap.state = IdeDebugState::Error;
            snap.threads.clear();
            snap.stack_frames.clear();
            snap.scopes.clear();
            snap.inline_values.clear();
            snap.error = Some(msg);
        });
        evict_terminated_sessions(sessions);
    }

    pub(super) fn read_snapshot(&self) -> IdeDebugSessionSnapshot {
        let mut snap = self
            .snapshot
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        snap.output = self.snapshot_output();
        snap
    }

    pub(super) fn mutate_and_broadcast(
        &self,
        f: impl FnOnce(&mut IdeDebugSessionSnapshot),
    ) -> IdeDebugSessionSnapshot {
        let mut updated = {
            let mut snap = self.snapshot.write().unwrap_or_else(|e| e.into_inner());
            f(&mut snap);
            snap.revision = snap.revision.saturating_add(1);
            snap.clone()
        };
        updated.output = self.snapshot_output();
        let _ = self.events_tx.send(updated.clone());
        updated
    }

    pub(super) fn append_output(&self, category: &str, text: String) {
        if text.is_empty() {
            return;
        }
        let clipped = if text.len() > 16_384 {
            let mut end = 16_384;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            format!("{}…\n", &text[..end])
        } else {
            text
        };
        {
            let mut buf = self.output_buf.lock().unwrap_or_else(|e| e.into_inner());
            if buf.len() >= MAX_DEBUG_OUTPUT {
                buf.pop_front();
            }
            buf.push_back(IdeDebugOutputEntry {
                category: category.to_string(),
                text: clipped,
            });
            let mut total_bytes: usize = buf.iter().map(|e| e.text.len()).sum();
            while total_bytes > MAX_DEBUG_OUTPUT_BYTES && buf.len() > 1 {
                if let Some(removed) = buf.pop_front() {
                    total_bytes = total_bytes.saturating_sub(removed.text.len());
                }
            }
        }
        self.mutate_and_broadcast(|_| {});
    }

    pub(super) fn active_conn(&self) -> Result<Arc<ActiveConnection>> {
        self.conn
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .ok_or_else(|| Error::Conflict("debug session is no longer connected".into()))
    }
}
