//! Multiplexed DAP wire client and per-session state entry (`SessionEntry`).

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use dashmap::DashMap;
use otto_core::{Error, Result};
use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{broadcast, mpsc, oneshot, watch, Notify};

use super::session::{DapStartContext, MAX_DEBUG_OUTPUT, MAX_DEBUG_OUTPUT_BYTES};
use super::session_inspect::BpStopKey;
use super::session_transport::evict_terminated_sessions;
use super::wire::{DapEvent, DapMessage, DapReadItem, DapReader, DapResponse, DapWriter};
use super::BridgeGuard;
use crate::types::{IdeDebugOutputEntry, IdeDebugSessionSnapshot, IdeDebugState, IdeStartDebugReq};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
/// How long debuggee output gathers before watchers get one snapshot of it: a
/// snapshot carries the whole output ring (up to 256 KiB), so one per line
/// cost a full clone + JSON encode per line per watcher.
pub(super) const OUTPUT_COALESCE: Duration = Duration::from_millis(100);

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
            loop {
                let msg = match reader.next_item().await {
                    Ok(Some(DapReadItem::Message(msg))) => msg,
                    // One unreadable or oversized reply (a huge `variables`
                    // body) fails the request it answered, not the session.
                    Ok(Some(DapReadItem::Skipped {
                        len,
                        request_seq,
                        reason,
                    })) => {
                        tracing::warn!(len, reason = %reason, "DAP: skipped an adapter message");
                        let tx = request_seq.and_then(|seq| {
                            client_ref
                                .pending
                                .lock()
                                .ok()
                                .and_then(|mut map| map.remove(&seq))
                                .map(|tx| (seq, tx))
                        });
                        if let Some((seq, tx)) = tx {
                            let _ = tx.send(DapResponse {
                                seq: 0,
                                request_seq: seq,
                                success: false,
                                command: String::new(),
                                message: Some(format!(
                                    "the debug adapter's reply could not be used ({reason})"
                                )),
                                body: None,
                            });
                        }
                        continue;
                    }
                    Ok(None) | Err(_) => break,
                };
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

    pub(super) fn is_retired(&self) -> bool {
        self.retired.load(Ordering::SeqCst)
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

impl ActiveConnection {
    pub(super) fn shutdown_immediately(&self) {
        self.wire.retire();
        self.wire.abort_pending();
        self.guard.kill_all();
    }
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
    /// The last breakpoint stop: where it was and its instruction pointer
    /// (see `is_same_line_relocation`).
    pub(super) last_bp_stop: Mutex<Option<(BpStopKey, u64)>>,
    pub(super) break_on_exceptions: Mutex<bool>,
    pub(super) conn: RwLock<Option<Arc<ActiveConnection>>>,
    pub(super) events_tx: broadcast::Sender<IdeDebugSessionSnapshot>,
    pub(super) initialized_notify: Arc<Notify>,
    pub(super) halted_notify: Arc<Notify>,
    pub(super) suppress_transient_pause: AtomicBool,
    pub(super) restarting: AtomicBool,
    pub(super) boot_generation: AtomicU64,
    /// Mirrors `boot_generation` so an in-flight boot (build, adapter start,
    /// handshake) can be cancelled the moment Stop or Restart supersedes it.
    boot_gen_tx: watch::Sender<u64>,
    pub(super) lifecycle_lock: tokio::sync::Mutex<()>,
    /// A coalesced output snapshot is scheduled for watchers.
    output_flush_pending: AtomicBool,
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
            config_id: req.config_id.clone(),
            launch: Some(req.launch.clone()),
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
        let initial_break_exc = req.break_on_exceptions;
        Arc::new(Self {
            created_seq,
            owner_user_id,
            orig_req: RwLock::new(req),
            orig_ctx: RwLock::new(ctx),
            snapshot: RwLock::new(initial_snap),
            output_buf: Mutex::new(output_buf),
            configured_bp_files: Mutex::new(BTreeSet::new()),
            temp_run_to_cursor: Mutex::new(None),
            last_bp_stop: Mutex::new(None),
            break_on_exceptions: Mutex::new(initial_break_exc),
            conn: RwLock::new(None),
            events_tx,
            initialized_notify: Arc::new(Notify::new()),
            halted_notify: Arc::new(Notify::new()),
            suppress_transient_pause: AtomicBool::new(false),
            restarting: AtomicBool::new(false),
            boot_generation: AtomicU64::new(1),
            boot_gen_tx: watch::channel(1).0,
            lifecycle_lock: tokio::sync::Mutex::new(()),
            output_flush_pending: AtomicBool::new(false),
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

    /// Supersede any boot in flight (Stop, Restart) and return the new generation.
    pub(super) fn bump_generation(&self) -> u64 {
        let gen = self.boot_generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.boot_gen_tx.send_replace(gen);
        gen
    }

    /// Resolves once Stop or Restart has moved the session past `my_gen`.
    pub(super) async fn superseded(&self, my_gen: u64) {
        let mut rx = self.boot_gen_tx.subscribe();
        if rx.wait_for(|g| *g != my_gen).await.is_err() {
            std::future::pending::<()>().await;
        }
    }

    /// Run a boot step unless the boot is superseded first; the step future is
    /// then dropped, which kills its build's process group and releases the
    /// lifecycle lock instead of making Stop wait out a 15 s adapter timeout.
    pub(super) async fn unless_superseded<T>(
        &self,
        my_gen: u64,
        step: impl std::future::Future<Output = T>,
    ) -> Option<T> {
        if self.boot_generation.load(Ordering::SeqCst) != my_gen {
            return None;
        }
        tokio::select! {
            v = step => Some(v),
            _ = self.superseded(my_gen) => None,
        }
    }

    pub(super) fn is_terminal(&self) -> bool {
        let state = self
            .snapshot
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .state;
        matches!(state, IdeDebugState::Terminated | IdeDebugState::Error)
    }

    fn release_active_netbridge(&self) {
        let mut ctx_guard = self.orig_ctx.write().unwrap_or_else(|e| e.into_inner());
        if let Some(lease) = ctx_guard.netbridge.take() {
            if ctx_guard.dormant_netbridge.is_none() {
                ctx_guard.dormant_netbridge = Some(lease.dormant_handle());
            }
        }
    }

    pub(super) fn disconnect_and_kill(&self) {
        if self.restarting.load(Ordering::SeqCst) {
            return;
        }
        self.release_active_netbridge();
        if let Some(conn) = self.conn.write().unwrap_or_else(|e| e.into_inner()).take() {
            conn.shutdown_immediately();
        }
    }

    pub(super) fn mark_terminated(&self) {
        if self.restarting.load(Ordering::SeqCst) {
            return;
        }
        self.release_active_netbridge();
        self.mutate_and_broadcast(|snap| {
            if snap.state != IdeDebugState::Error {
                snap.state = IdeDebugState::Terminated;
            }
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

    pub(super) fn append_output(self: &Arc<Self>, category: &str, text: String) {
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
        {
            let mut snap = self.snapshot.write().unwrap_or_else(|e| e.into_inner());
            snap.revision = snap.revision.saturating_add(1);
        }
        self.schedule_output_flush();
    }

    /// Broadcast at most one snapshot per [`OUTPUT_COALESCE`] for a burst of
    /// output lines, and nothing at all while no one is watching.
    fn schedule_output_flush(self: &Arc<Self>) {
        if self.events_tx.receiver_count() == 0
            || self.output_flush_pending.swap(true, Ordering::AcqRel)
        {
            return;
        }
        let Ok(rt) = tokio::runtime::Handle::try_current() else {
            self.output_flush_pending.store(false, Ordering::Release);
            let _ = self.events_tx.send(self.read_snapshot());
            return;
        };
        let me = Arc::clone(self);
        rt.spawn(async move {
            tokio::time::sleep(OUTPUT_COALESCE).await;
            // Clear first: a line appended after this schedules the next flush.
            me.output_flush_pending.store(false, Ordering::Release);
            let _ = me.events_tx.send(me.read_snapshot());
        });
    }

    pub(super) fn active_conn(&self) -> Result<Arc<ActiveConnection>> {
        self.conn
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .ok_or_else(|| Error::Conflict("debug session is no longer connected".into()))
    }
}
