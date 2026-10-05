//! The daemon side of a held PTY: one connection to a PTY holder
//! ([`crate::holder`]), feeding the handle's local mirror and carrying its
//! input, resizes and kills.
//!
//! Threads: one reader per connection (output → mirror, acks → the waiting
//! writer, exit → the handle's exit watch). Input goes through the handle's
//! ordinary writer thread, whose `Write` target ([`HeldWriter`]) sends one
//! INPUT frame and blocks until the holder acknowledges that the bytes reached
//! the tty — so `PtyHandle::write*` keeps exactly its local semantics
//! (ordering, the bounded queue, `Conflict` on a child that stops reading).
//!
//! A dropped connection is re-established (the holder replaced us, or kicked a
//! lagging stream); a holder that is really gone counts as the child exiting
//! (`-1`) — its PTY master closed, which hung the child up. A holder that is
//! alive but unreachable is NOT reported as exited (that would let a resume
//! start a second CLI on the same conversation): it is ended over a fresh
//! connection ([`crate::holder::terminate`]) first, and only a confirmed
//! terminate — or a holder that is provably gone — fires the exit. KILL /
//! RELEASE that cannot be sent on the current connection (mid-reconnect) take
//! the same fallback instead of being silently lost.

use std::io::{self, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::watch;

use crate::holder::{
    frame, AdoptError, Hello, HolderInfo, InputAck, HANDSHAKE_TIMEOUT, PROTO_MAJOR,
};
use crate::ring::RingBuffer;
use crate::{lock_unpoisoned, ChildState, Mirror, PtyHandle};

/// Largest INPUT frame payload; a bigger write is split by the writer loop.
const MAX_INPUT_CHUNK: usize = 1 << 20;
/// Reconnect attempts after an unexpected disconnect, and the pause between.
const RECONNECT_ATTEMPTS: u32 = 10;
const RECONNECT_PAUSE: Duration = Duration::from_millis(300);
/// Pause before another reconnect/terminate round when the holder is alive
/// but could be neither reconnected nor terminated.
const UNREACHABLE_RETRY: Duration = Duration::from_secs(5);

type Ack = io::Result<()>;

pub(crate) struct HeldConn {
    path: PathBuf,
    info: HolderInfo,
    /// Write half of the CURRENT connection (`None` while reconnecting).
    write: Mutex<Option<UnixStream>>,
    /// The one in-flight input job awaiting its INPUT_ACK.
    pending: Mutex<Option<(u64, SyncSender<Ack>)>>,
    seq: AtomicU64,
    /// Let go without ending the session (daemon shutdown for a restart).
    detached: AtomicBool,
    /// The handle is being dropped: stop reading, never reconnect.
    closed: AtomicBool,
    /// The holder reported the child's exit (or vanished).
    exited: AtomicBool,
    /// Scrollback cap last delivered to the holder (`0` = unknown: a fresh
    /// adoption, or a send that failed mid-reconnect). The daemon's status
    /// tick re-asserts the cap every 2 s; only a change is sent.
    history_cap: AtomicUsize,
    /// The holder was ended over a fresh connection ([`Self::terminate_fallback`]).
    terminated: AtomicBool,
    /// Publishes the child's exit to the handle (reader thread, or a
    /// confirmed terminate — which supersedes our connection, so the reader
    /// would never see an EXITED frame for it).
    exit: ExitSignal,
}

impl HeldConn {
    pub(crate) fn info(&self) -> &HolderInfo {
        &self.info
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn detach(&self) {
        self.detached.store(true, Ordering::SeqCst);
    }

    pub(crate) fn is_detached(&self) -> bool {
        self.detached.load(Ordering::SeqCst)
    }

    fn send(&self, kind: u8, payload: &[u8]) -> io::Result<()> {
        let mut guard = lock_unpoisoned(&self.write);
        let Some(stream) = guard.as_mut() else {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionReset,
                "pty holder connection is being re-established",
            ));
        };
        stream.write_all(&frame::encode(kind, payload))
    }

    pub(crate) fn resize(&self, cols: u16, rows: u16) -> io::Result<()> {
        self.send(frame::RESIZE, &frame::grid(cols, rows))
    }

    /// Mirror a scrollback-cap change into the holder's own emulator (perf
    /// 01 N1). Best-effort: a failed send is retried on the next change or
    /// re-assertion.
    pub(crate) fn set_history_cap(&self, lines: usize) {
        if self.history_cap.swap(lines, Ordering::SeqCst) == lines {
            return;
        }
        let payload = (lines.min(u32::MAX as usize) as u32).to_be_bytes();
        if self.send(frame::HISTORY_CAP, &payload).is_err() {
            self.history_cap.store(0, Ordering::SeqCst);
        }
    }

    /// End the child. If the KILL can't go out on the current connection (it
    /// is being re-established, or just broke), end the holder over a fresh
    /// one instead — a lost kill orphaned the holder's child for up to a day
    /// while the session looked dead and could be resumed a second time.
    pub(crate) fn kill(&self) -> io::Result<()> {
        if self.terminated.load(Ordering::SeqCst) {
            return Ok(());
        }
        match self.send(frame::KILL, &[]) {
            Ok(()) => Ok(()),
            Err(_) => self.terminate_fallback(),
        }
    }

    /// Tell the holder it may exit once the child is gone. Best-effort, with
    /// the same fresh-connection fallback as [`Self::kill`].
    pub(crate) fn release(&self) {
        if self.terminated.load(Ordering::SeqCst) {
            return;
        }
        if self.send(frame::RELEASE, &[]).is_err() {
            let _ = self.terminate_fallback();
        }
    }

    /// [`crate::holder::terminate`] over a fresh connection (KILL + RELEASE).
    /// The exit is reported only once that is confirmed — the frames were
    /// delivered, or nobody listens on the socket any more (holder gone).
    /// That connection supersedes ours, so no EXITED frame would follow.
    fn terminate_fallback(&self) -> io::Result<()> {
        let res = match crate::holder::terminate(&self.path) {
            Ok(()) => Ok(()),
            Err(e) if holder_gone(&e) => Ok(()),
            Err(e) => Err(e),
        };
        if res.is_ok() {
            self.terminated.store(true, Ordering::SeqCst);
            self.mark_exited(-1);
        }
        res
    }

    /// The child is gone (confirmed): fail the waiting writer and fire the
    /// handle's exit (idempotent — the first code wins).
    fn mark_exited(&self, code: i32) {
        self.exited.store(true, Ordering::SeqCst);
        self.fail_pending(io::ErrorKind::BrokenPipe);
        self.exit.fire(code);
    }

    /// Forget the write half (see `PtyHandle::simulate_holder_reconnecting`).
    pub(crate) fn drop_writer(&self) {
        lock_unpoisoned(&self.write).take();
    }

    /// Hang up for good (the handle is dropping).
    pub(crate) fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        if let Some(stream) = lock_unpoisoned(&self.write).take() {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
        self.fail_pending(io::ErrorKind::BrokenPipe);
    }

    fn fail_pending(&self, kind: io::ErrorKind) {
        if let Some((_, tx)) = lock_unpoisoned(&self.pending).take() {
            let _ = tx.send(Err(io::Error::new(kind, "pty holder connection lost")));
        }
    }

    fn complete(&self, ack: InputAck) {
        let mut pending = lock_unpoisoned(&self.pending);
        if pending.as_ref().is_some_and(|(seq, _)| *seq == ack.seq) {
            if let Some((_, tx)) = pending.take() {
                let res = match ack.error {
                    None => Ok(()),
                    // The child is gone: end the writer like a local EIO.
                    Some(msg) if ack.closed => Err(io::Error::new(io::ErrorKind::BrokenPipe, msg)),
                    Some(msg) => Err(io::Error::other(msg)),
                };
                let _ = tx.send(res);
            }
        }
    }

    /// Send one input job and block until the holder wrote it to the tty.
    fn request_input(&self, data: &[u8]) -> Ack {
        if self.exited.load(Ordering::SeqCst) || self.closed.load(Ordering::SeqCst) {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "the process is gone",
            ));
        }
        let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let (tx, rx) = sync_channel::<Ack>(1);
        *lock_unpoisoned(&self.pending) = Some((seq, tx));
        let mut payload = Vec::with_capacity(8 + data.len());
        payload.extend_from_slice(&seq.to_be_bytes());
        payload.extend_from_slice(data);
        if let Err(e) = self.send(frame::INPUT, &payload) {
            lock_unpoisoned(&self.pending).take();
            return Err(io::Error::new(io::ErrorKind::ConnectionReset, e));
        }
        match rx.recv() {
            Ok(res) => res,
            Err(_) => Err(io::Error::new(
                io::ErrorKind::ConnectionReset,
                "pty holder connection lost",
            )),
        }
    }
}

/// The handle writer thread's target for a held PTY.
pub(crate) struct HeldWriter(Arc<HeldConn>);

impl HeldWriter {
    pub(crate) fn new(conn: Arc<HeldConn>) -> Self {
        Self(conn)
    }
}

impl Write for HeldWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = buf.len().min(MAX_INPUT_CHUNK);
        self.0.request_input(&buf[..n])?;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Connect and run the handshake: HELLO → HELLO_ACK (+ version check) →
/// SNAPSHOT. Returns the stream (no read timeout) and what the holder said.
/// A completed handshake: the stream, the holder's self-description and its
/// `(cols, rows, bytes)` snapshot.
type Handshake = (UnixStream, HolderInfo, (u16, u16, Vec<u8>));

fn handshake(path: &Path) -> Result<Handshake, AdoptError> {
    let mut stream = match UnixStream::connect(path) {
        Ok(s) => s,
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound
            ) =>
        {
            // Nobody listens: the holder is gone (killed, crashed). Its socket
            // file is litter now.
            let _ = std::fs::remove_file(path);
            return Err(AdoptError::Stale);
        }
        Err(e) => {
            return Err(AdoptError::Failed(format!(
                "connect {}: {e}",
                path.display()
            )))
        }
    };
    let failed = |what: &str, e: io::Error| AdoptError::Failed(format!("pty holder {what}: {e}"));
    stream
        .set_read_timeout(Some(HANDSHAKE_TIMEOUT))
        .map_err(|e| failed("timeout", e))?;
    stream
        .set_write_timeout(Some(HANDSHAKE_TIMEOUT))
        .map_err(|e| failed("timeout", e))?;
    let hello = serde_json::to_vec(&Hello::ours()).unwrap_or_default();
    stream
        .write_all(&frame::encode(frame::HELLO, &hello))
        .map_err(|e| failed("hello", e))?;
    let (kind, payload) = frame::read_sync(&mut stream).map_err(|e| failed("hello ack", e))?;
    if kind != frame::HELLO_ACK {
        return Err(AdoptError::Failed(format!(
            "pty holder: unexpected frame {kind:#x}"
        )));
    }
    let info: HolderInfo = serde_json::from_slice(&payload)
        .map_err(|e| AdoptError::Failed(format!("pty holder hello ack: {e}")))?;
    if info.proto_major != PROTO_MAJOR {
        return Err(AdoptError::Incompatible(Box::new(info)));
    }
    // Skip anything a newer minor sends before its snapshot.
    let snapshot = loop {
        let (kind, payload) = frame::read_sync(&mut stream).map_err(|e| failed("snapshot", e))?;
        if kind == frame::SNAPSHOT {
            let Some((cols, rows, data)) = frame::parse_grid(&payload) else {
                return Err(AdoptError::Failed("pty holder: malformed snapshot".into()));
            };
            break (cols, rows, data.to_vec());
        }
    };
    stream
        .set_read_timeout(None)
        .map_err(|e| failed("timeout", e))?;
    Ok((stream, info, snapshot))
}

/// [`PtyHandle::adopt`].
pub(crate) fn adopt(path: &Path) -> Result<PtyHandle, AdoptError> {
    let (stream, info, (cols, rows, snapshot)) = handshake(path)?;
    let write = stream
        .try_clone()
        .map_err(|e| AdoptError::Failed(format!("pty holder socket: {e}")))?;

    // The holder's emulator is authoritative; only guard against nonsense.
    let cols = cols.clamp(2, crate::MAX_COLS);
    let rows = rows.clamp(2, crate::MAX_ROWS);
    let mut mirror = Mirror::new(cols, rows, RingBuffer::default());
    mirror.reset_to(cols, rows, &snapshot);
    // Carry the holder's last-output clock over: the snapshot replay is not
    // new output, and a daemon restart must not make every re-adopted session
    // look freshly active to the idle sweep (review A14).
    mirror.backdate(last_output_age(info.last_output_unix_ms));

    let child_state = Arc::new(ChildState::default());
    let (exit_tx, exit_rx) = watch::channel::<Option<i32>>(None);
    let (done_tx, done_rx) = watch::channel(false);
    let exit = ExitSignal {
        child_state: Arc::clone(&child_state),
        exit_tx,
        done_tx,
    };
    let conn = Arc::new(HeldConn {
        path: path.to_path_buf(),
        info: info.clone(),
        write: Mutex::new(Some(write)),
        pending: Mutex::new(None),
        seq: AtomicU64::new(0),
        detached: AtomicBool::new(false),
        closed: AtomicBool::new(false),
        exited: AtomicBool::new(false),
        history_cap: AtomicUsize::new(0),
        terminated: AtomicBool::new(false),
        exit,
    });
    if let Some(code) = info.exited {
        // Dead on arrival (it exited while nobody was connected): visible to
        // the caller before this returns, not only once the reader runs.
        conn.exited.store(true, Ordering::SeqCst);
        conn.exit.fire(code);
    }
    {
        let conn = Arc::clone(&conn);
        let mirror = mirror.clone();
        std::thread::spawn(move || reader_loop(stream, conn, mirror));
    }
    Ok(PtyHandle::from_held(
        conn,
        mirror,
        child_state,
        exit_rx,
        done_rx,
    ))
}

/// How long ago `last_output_unix_ms` was (zero when unknown or in the
/// future — a clock step never makes a session look idle for longer).
fn last_output_age(last_output_unix_ms: u64) -> Duration {
    if last_output_unix_ms == 0 {
        return Duration::ZERO;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    Duration::from_millis(now.saturating_sub(last_output_unix_ms))
}

struct ExitSignal {
    child_state: Arc<ChildState>,
    exit_tx: watch::Sender<Option<i32>>,
    done_tx: watch::Sender<bool>,
}

impl ExitSignal {
    fn fire(&self, code: i32) {
        self.child_state.mark_exited();
        let _ = self.done_tx.send(true);
        if self.exit_tx.borrow().is_none() {
            let _ = self.exit_tx.send(Some(code));
        }
    }
}

/// Nobody listens on the holder's socket: it is gone (killed, crashed).
fn holder_gone(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound
    )
}

fn reader_loop(mut stream: UnixStream, conn: Arc<HeldConn>, mirror: Mirror) {
    loop {
        match frame::read_sync(&mut stream) {
            Ok((frame::OUTPUT, data)) => mirror.feed_bytes(data.into()),
            Ok((frame::SNAPSHOT, payload)) => {
                if let Some((cols, rows, data)) = frame::parse_grid(&payload) {
                    mirror.reset_to(cols, rows, data);
                }
            }
            Ok((frame::INPUT_ACK, payload)) => {
                if let Ok(ack) = serde_json::from_slice::<InputAck>(&payload) {
                    conn.complete(ack);
                }
            }
            Ok((frame::EXITED, payload)) => {
                let code = serde_json::from_slice::<serde_json::Value>(&payload)
                    .ok()
                    .and_then(|v| v.get("code").and_then(|c| c.as_i64()))
                    .unwrap_or(-1) as i32;
                conn.mark_exited(code);
                // We hold the whole final state: the holder may go — unless
                // this handle was detached (daemon handing over for a
                // restart): then the next daemon adopts the exited holder and
                // learns the exit from it, so it must linger.
                if !conn.is_detached() {
                    conn.release();
                }
            }
            Ok((frame::SUPERSEDED, _)) => {
                // Another daemon adopted this session: it is theirs now. Let
                // go without ending it (our Drop must not kill it) and
                // without reporting an exit.
                conn.detach();
                conn.fail_pending(io::ErrorKind::ConnectionReset);
                lock_unpoisoned(&conn.write).take();
                return;
            }
            // Unknown kinds: a newer holder's additions — ignore.
            Ok(_) => {}
            Err(_) => {
                conn.fail_pending(io::ErrorKind::ConnectionReset);
                if conn.closed.load(Ordering::SeqCst)
                    || conn.is_detached()
                    || conn.exited.load(Ordering::SeqCst)
                {
                    return;
                }
                lock_unpoisoned(&conn.write).take();
                match reconnect_or_end(&conn, &mirror) {
                    Some(fresh) => stream = fresh,
                    None => return,
                }
            }
        }
    }
}

/// Outcome of one [`reconnect`] round.
enum Reconnect {
    /// Back on the SAME holder.
    Connected(UnixStream),
    /// Our holder is provably gone: nobody listens on the socket, or a
    /// different holder (pid / protocol) owns it now.
    Gone,
    /// The holder may still be alive but every attempt failed.
    Unreachable,
    /// The handle is closing or was detached: stop quietly.
    Stop,
}

/// Reconnect after a dropped connection; on give-up decide whether the child
/// really is gone. `None` = the reader is done (exit fired if confirmed).
///
/// An unreachable-but-maybe-alive holder is ended over a fresh connection
/// before the exit is reported: reporting `-1` while its child still ran let
/// a resume start a second CLI on the same conversation, and left the holder
/// (and child) orphaned for up to its 24 h idle limit.
fn reconnect_or_end(conn: &HeldConn, mirror: &Mirror) -> Option<UnixStream> {
    loop {
        match reconnect(conn, mirror) {
            Reconnect::Connected(stream) => return Some(stream),
            Reconnect::Stop => return None,
            Reconnect::Gone => {
                // With its PTY master gone the child was hung up.
                conn.mark_exited(-1);
                return None;
            }
            Reconnect::Unreachable => {
                if conn.terminate_fallback().is_ok() {
                    return None;
                }
                // Neither reachable nor terminable: try again later rather
                // than declare a possibly-live child dead.
                std::thread::sleep(UNREACHABLE_RETRY);
            }
        }
    }
}

/// Re-establish a dropped connection to the SAME holder (it replaced a stale
/// client, or cut a lagging stream). The new snapshot resyncs the mirror.
fn reconnect(conn: &HeldConn, mirror: &Mirror) -> Reconnect {
    for attempt in 0..RECONNECT_ATTEMPTS {
        if conn.closed.load(Ordering::SeqCst) || conn.is_detached() {
            return Reconnect::Stop;
        }
        if conn.exited.load(Ordering::SeqCst) {
            // A fallback terminate (kill/release mid-reconnect) already
            // confirmed and reported the end.
            return Reconnect::Stop;
        }
        if attempt > 0 {
            std::thread::sleep(RECONNECT_PAUSE);
        }
        match handshake(&conn.path) {
            Ok((stream, info, (cols, rows, snapshot))) => {
                if info.holder_pid != conn.info.holder_pid || info.child_pid != conn.info.child_pid
                {
                    return Reconnect::Gone;
                }
                let Ok(write) = stream.try_clone() else {
                    continue;
                };
                mirror.reset_to(cols, rows, &snapshot);
                *lock_unpoisoned(&conn.write) = Some(write);
                // An exited child's EXITED frame follows on this connection.
                return Reconnect::Connected(stream);
            }
            Err(AdoptError::Stale) | Err(AdoptError::Incompatible(_)) => return Reconnect::Gone,
            Err(AdoptError::Failed(_)) => continue,
        }
    }
    Reconnect::Unreachable
}
