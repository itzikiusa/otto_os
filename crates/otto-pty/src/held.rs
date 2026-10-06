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
//! connection ([`crate::holder::terminate`]) first, and only a holder that is
//! provably GONE afterwards (nobody listens on its socket — it exits only
//! once its child has) fires the exit. KILL / RELEASE that cannot be sent on
//! the current connection (mid-reconnect) take the same fallback on a
//! detached thread instead of being silently lost, and are re-sent on the
//! next connection a reconnect establishes (review S1-08).

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
/// How long a fallback terminate waits for the holder to disappear: the
/// holder's HUP → TERM → KILL escalation ([`crate::KILL_GRACE`] per step)
/// plus a second for it to reap the child and exit.
const TERMINATE_CONFIRM: Duration = Duration::from_secs(2 * crate::KILL_GRACE.as_secs() + 1);
/// Fallback rounds a detached kill/release fallback thread makes.
const FALLBACK_ROUNDS: u32 = 3;

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
    /// A KILL / RELEASE was requested: re-sent on every new connection until
    /// the exit is confirmed (a frame lost to a reconnect race orphaned the
    /// child while the session looked dead).
    kill_requested: AtomicBool,
    release_requested: AtomicBool,
    /// A detached fallback thread is running (at most one).
    fallback_running: AtomicBool,
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
        let res = stream.write_all(&frame::encode(kind, payload));
        if res.is_err() {
            // A write that failed part-way (the 5 s write timeout on a frame
            // of up to 1 MiB) leaves the stream MISFRAMED: the holder could
            // decode a payload byte as a frame kind (0x7F = KILL). Never
            // write to it again — hang it up so the reader reconnects on a
            // clean stream (review S1-21).
            if let Some(stream) = guard.take() {
                let _ = stream.shutdown(std::net::Shutdown::Both);
            }
        }
        res
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
    ///
    /// Never blocks on the fallback: it runs on a detached thread (it waits
    /// for the holder's whole kill escalation, and callers sit on async
    /// workers — review S1-22). The exit fires once the end is confirmed;
    /// until then the request is re-sent on any new connection.
    pub(crate) fn kill(self: &Arc<Self>) -> io::Result<()> {
        if self.terminated.load(Ordering::SeqCst) {
            return Ok(());
        }
        self.kill_requested.store(true, Ordering::SeqCst);
        if self.send(frame::KILL, &[]).is_err() {
            self.spawn_fallback();
        }
        Ok(())
    }

    /// Tell the holder it may exit once the child is gone. Best-effort, with
    /// the same fresh-connection fallback as [`Self::kill`].
    pub(crate) fn release(self: &Arc<Self>) {
        if self.terminated.load(Ordering::SeqCst) {
            return;
        }
        self.release_requested.store(true, Ordering::SeqCst);
        if self.send(frame::RELEASE, &[]).is_err() {
            self.spawn_fallback();
        }
    }

    /// Run [`Self::terminate_fallback`] on a detached thread (at most one at
    /// a time), retrying a few rounds while the holder is unreachable.
    fn spawn_fallback(self: &Arc<Self>) {
        if self.fallback_running.swap(true, Ordering::SeqCst) {
            return;
        }
        let conn = Arc::clone(self);
        std::thread::spawn(move || {
            for round in 0..FALLBACK_ROUNDS {
                if conn.terminated.load(Ordering::SeqCst) || conn.exited.load(Ordering::SeqCst) {
                    break;
                }
                if round > 0 {
                    std::thread::sleep(UNREACHABLE_RETRY);
                }
                if conn.terminate_fallback().is_ok() {
                    break;
                }
            }
            conn.fallback_running.store(false, Ordering::SeqCst);
        });
    }

    /// [`crate::holder::terminate`] over a fresh connection (KILL + RELEASE),
    /// then wait (bounded by [`TERMINATE_CONFIRM`]) until the holder is
    /// provably GONE — it exits only after its child, so that is the proof
    /// the child is dead. Writing the frames is NOT proof: the escalation
    /// may still be running, or a racing reconnect may have superseded the
    /// terminating connection. Only then is the exit reported; that
    /// connection superseded ours, so no EXITED frame would follow.
    fn terminate_fallback(&self) -> io::Result<()> {
        let res = match crate::holder::terminate(&self.path) {
            Ok(()) => wait_holder_gone(&self.path, TERMINATE_CONFIRM),
            Err(e) if holder_gone(&e) => Ok(()),
            Err(e) => Err(e),
        };
        if res.is_ok() {
            self.terminated.store(true, Ordering::SeqCst);
            self.mark_exited(-1);
        }
        res
    }

    /// After a reconnect: re-send a KILL / RELEASE that may have been lost
    /// with the previous connection (or with a superseded terminate).
    fn resend_requests(&self) {
        if self.kill_requested.load(Ordering::SeqCst) {
            let _ = self.send(frame::KILL, &[]);
        }
        if self.release_requested.load(Ordering::SeqCst) {
            let _ = self.send(frame::RELEASE, &[]);
        }
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
            // Never (fully) on the wire: the stream was gone or is hung up
            // after a partial write, so the holder cannot ack it — NOT
            // delivered, safe to send again.
            lock_unpoisoned(&self.pending).take();
            return Err(io::Error::new(io::ErrorKind::ConnectionReset, e));
        }
        settle_sent_input(rx.recv())
    }
}

/// The error kind of a held write whose frame WAS sent but whose INPUT_ACK
/// was lost with the holder connection (S1-23): the holder may or may not
/// have written it to the tty. Distinct from `ConnectionReset` (not
/// delivered — a retry is safe), so a caller never resends input that may
/// already be in the terminal. Like `ConnectionReset`, it fails only that
/// job: the writer keeps running across the reconnect.
pub(crate) const DELIVERY_UNKNOWN: io::ErrorKind = io::ErrorKind::ConnectionAborted;

/// Outcome of an input job already on the wire: a connection loss before its
/// ack (`fail_pending(ConnectionReset)`, or the job's channel dropped) means
/// delivery is unknown, not "not delivered".
fn settle_sent_input(ack: Result<Ack, std::sync::mpsc::RecvError>) -> Ack {
    match ack {
        Ok(Err(e)) if e.kind() == io::ErrorKind::ConnectionReset => Err(io::Error::new(
            DELIVERY_UNKNOWN,
            "pty holder connection lost before the input was acknowledged — it may or may not have reached the terminal",
        )),
        Ok(res) => res,
        Err(_) => Err(io::Error::new(
            DELIVERY_UNKNOWN,
            "pty holder connection lost before the input was acknowledged — it may or may not have reached the terminal",
        )),
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

    // The holder's emulator is authoritative; only guard against nonsense
    // (the LIVE bounds — a holder may be up to 300 rows tall).
    let (cols, rows) = crate::clamp_live_grid(cols, rows);
    let mut mirror = Mirror::new(cols, rows, RingBuffer::default());
    mirror.reset_to(cols, rows, &snapshot, true);
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
        kill_requested: AtomicBool::new(false),
        release_requested: AtomicBool::new(false),
        fallback_running: AtomicBool::new(false),
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

/// Poll until nobody listens on `path` (the holder exited — which it does only
/// once its child is gone), up to `within`. A bare connect that never sends
/// HELLO is harmless to the holder: it drops it after the handshake timeout
/// without claiming the session.
pub(crate) fn wait_holder_gone(path: &Path, within: Duration) -> io::Result<()> {
    let deadline = std::time::Instant::now() + within;
    loop {
        match UnixStream::connect(path) {
            Err(e) if holder_gone(&e) => return Ok(()),
            _ => {}
        }
        if std::time::Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "pty holder still running after terminate",
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn reader_loop(mut stream: UnixStream, conn: Arc<HeldConn>, mirror: Mirror) {
    loop {
        match frame::read_sync(&mut stream) {
            Ok((frame::OUTPUT, data)) => mirror.feed_bytes(data.into()),
            Ok((frame::SNAPSHOT, payload)) => {
                if let Some((cols, rows, data)) = frame::parse_grid(&payload) {
                    mirror.reset_to(cols, rows, data, false);
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
                conn.fail_pending(io::ErrorKind::ConnectionReset);
                lock_unpoisoned(&conn.write).take();
                if conn.kill_requested.load(Ordering::SeqCst)
                    && !conn.closed.load(Ordering::SeqCst)
                    && !conn.exited.load(Ordering::SeqCst)
                {
                    // Superseded by our OWN fallback terminate, not by
                    // another daemon: keep supervising until the exit is
                    // confirmed (the reconnect re-sends the KILL).
                    match reconnect_or_end(&conn, &mirror) {
                        Some(fresh) => {
                            stream = fresh;
                            continue;
                        }
                        None => return,
                    }
                }
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
                mirror.reset_to(cols, rows, &snapshot, false);
                *lock_unpoisoned(&conn.write) = Some(write);
                conn.resend_requests();
                // An exited child's EXITED frame follows on this connection.
                return Reconnect::Connected(stream);
            }
            Err(AdoptError::Stale) | Err(AdoptError::Incompatible(_)) => return Reconnect::Gone,
            Err(AdoptError::Failed(_)) => continue,
        }
    }
    Reconnect::Unreachable
}

#[cfg(test)]
mod settle_tests {
    use super::*;

    /// S1-23: a sent job whose ack was lost reports "delivery unknown", never
    /// the retry-safe `ConnectionReset`; acks and a gone child pass through.
    #[test]
    fn a_lost_ack_is_delivery_unknown_not_a_reset() {
        let reset = Ok(Err(io::Error::new(io::ErrorKind::ConnectionReset, "lost")));
        assert_eq!(
            settle_sent_input(reset).unwrap_err().kind(),
            DELIVERY_UNKNOWN
        );
        assert_eq!(
            settle_sent_input(Err(std::sync::mpsc::RecvError))
                .unwrap_err()
                .kind(),
            DELIVERY_UNKNOWN
        );
        assert!(settle_sent_input(Ok(Ok(()))).is_ok());
        let gone = Ok(Err(io::Error::new(io::ErrorKind::BrokenPipe, "gone")));
        assert_eq!(
            settle_sent_input(gone).unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
    }
}
