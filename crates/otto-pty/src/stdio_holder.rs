//! Stdio holders — a structured agent's stdio child that outlives the daemon.
//!
//! The same detached `ottod pty-holder` process as [`crate::holder`] (same
//! launcher, `setsid`, private 0600 socket, frame header, version handshake,
//! one client, [`frame::SUPERSEDED`], `KILL` / `RELEASE`, exit linger and
//! orphan TTL — [`crate::holder::terminate`] works on it unchanged), started
//! with [`HolderSpawn::stdio`] set: the child runs on PIPES instead of a PTY.
//! The structured runtime (`otto-agent-protocol`) speaks newline-delimited
//! JSON to it, so the holder deals in lines:
//!
//! - every stdout line gets the next sequence number (`seq`, from 1) and is
//!   kept until the client acknowledges it ([`kind::ACK`]) — the daemon acks
//!   only once it has durably checkpointed its protocol state past that
//!   line, so a daemon that restarts (or crashes) asks for everything after
//!   its checkpoint ([`kind::REPLAY`]) and loses nothing;
//! - unacknowledged lines are bounded by bytes ([`DEFAULT_LOG_CAP`]): at the
//!   cap the holder stops reading stdout, so the child blocks on its pipe
//!   until a daemon comes back — it pauses, nothing is dropped;
//! - stdin lines ([`kind::STDIN`]) and EOF ([`kind::CLOSE_STDIN`]) go to the
//!   child; its exit (code + stderr tail) is the log's last entry.
//!
//! On attach the holder answers `HELLO` with a [`HolderInfo`] whose `stdio`
//! is a [`StdioLog`] (`acked`, `last`); the client then sends `REPLAY {after}`
//! (which also acks `..=after`) and gets every entry after it, then live ones.
//! A client whose `after` is below `acked` would miss lines: it must not
//! adopt ([`attach`] fails, the caller terminates the holder).

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use otto_core::Result;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, watch, Notify};

use crate::holder::{
    self, frame, lock, AdoptError, Hello, HolderConfig, HolderInfo, HolderSpawn,
    DEFAULT_EXIT_LINGER, DEFAULT_ORPHAN_TTL, HANDSHAKE_TIMEOUT, PROTO_MAJOR, PROTO_MINOR,
    READY_PREFIX,
};
use crate::CommandSpec;

/// A stdout line longer than this is dropped (an [`StdioEvent::Overlong`]).
pub const MAX_LINE_BYTES: usize = 16 * 1024 * 1024;
/// Unacknowledged log bytes at which the holder stops reading stdout.
pub const DEFAULT_LOG_CAP: usize = 64 * 1024 * 1024;
const STDERR_TAIL: usize = 4096;
/// After `KILL`: SIGTERM, then SIGKILL this much later.
const KILL_GRACE: Duration = Duration::from_secs(2);

/// Stdio frame kinds (minor additions; the header and the frozen frames are
/// [`crate::holder`]'s).
pub mod kind {
    /// client → holder: `[u64 after]` — stream every entry after `after`
    /// (and drop `..=after`). The first frame after `HELLO_ACK`.
    pub const REPLAY: u8 = 0x20;
    /// holder → client: `[u64 seq][line bytes]`.
    pub const LINE: u8 = 0x21;
    /// holder → client: `[u64 seq][u64 len]` — a line over the size cap.
    pub const OVERLONG: u8 = 0x22;
    /// holder → client: `[u64 seq]{code, stderr_tail}` — the child exited.
    pub const EXITED: u8 = 0x23;
    /// client → holder: one stdin line (no newline).
    pub const STDIN: u8 = 0x24;
    /// client → holder: close the child's stdin (EOF).
    pub const CLOSE_STDIN: u8 = 0x25;
    /// client → holder: `[u64 seq]` — entries up to `seq` are durable on
    /// the client's side; drop them.
    pub const ACK: u8 = 0x26;
}

/// A stdio holder's log position, in its [`HolderInfo`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StdioLog {
    /// Everything up to here was acknowledged (and dropped).
    pub acked: u64,
    /// The last sequence assigned.
    pub last: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct ExitPayload {
    code: Option<i32>,
    #[serde(default)]
    stderr_tail: String,
}

// ── the holder process ──────────────────────────────────────────────────────

struct Entry {
    seq: u64,
    kind: u8,
    /// The whole frame payload (`[u64 seq]…`).
    payload: Vec<u8>,
}

#[derive(Default)]
struct Log {
    entries: VecDeque<Entry>,
    bytes: usize,
    last: u64,
    acked: u64,
    /// The child exited (`Some(code)`; a signal is `None` inside).
    exited: Option<Option<i32>>,
}

enum StdinOp {
    Line(Vec<u8>),
    Close,
}

struct Shared {
    socket: PathBuf,
    holder_pid: u32,
    child_pid: u32,
    started_at_ms: u64,
    meta: serde_json::Value,
    log: Mutex<Log>,
    cap: usize,
    /// New entries.
    data: Notify,
    /// Entries were acknowledged (room under the cap).
    space: Notify,
    last_output_ms: AtomicU64,
    stdin: mpsc::UnboundedSender<StdinOp>,
    released: AtomicBool,
    killed: AtomicBool,
    /// Nobody will ever read the log again (orphaned / unreachable): drop
    /// output instead of pausing the child at the cap, so it can exit.
    discard: AtomicBool,
    gen: AtomicU64,
    kick: watch::Sender<u64>,
    active: Mutex<Option<u64>>,
    last_client: Mutex<Instant>,
    exited_at: Mutex<Option<Instant>>,
    wake: Notify,
    orphan_ttl: Duration,
    exit_linger: Duration,
}

impl Shared {
    fn info(&self) -> HolderInfo {
        let log = lock(&self.log);
        HolderInfo {
            proto_major: PROTO_MAJOR,
            proto_minor: PROTO_MINOR,
            holder_version: format!("otto-pty {} (stdio)", env!("CARGO_PKG_VERSION")),
            holder_pid: self.holder_pid,
            child_pid: Some(self.child_pid),
            started_at_ms: self.started_at_ms,
            cols: 0,
            rows: 0,
            exited: log.exited.map(|c| c.unwrap_or(-1)),
            meta: self.meta.clone(),
            last_output_unix_ms: self.last_output_ms.load(Ordering::Relaxed),
            stdio: Some(StdioLog {
                acked: log.acked,
                last: log.last,
            }),
        }
    }

    /// Append one entry, waiting while the unacknowledged log is at its cap.
    async fn push(&self, kind: u8, body: &[u8]) {
        if kind != kind::EXITED && self.discard.load(Ordering::SeqCst) {
            return;
        }
        loop {
            let room = self.space.notified();
            tokio::pin!(room);
            room.as_mut().enable();
            if lock(&self.log).bytes < self.cap
                || kind == kind::EXITED
                || self.discard.load(Ordering::SeqCst)
            {
                break;
            }
            room.await;
        }
        let mut log = lock(&self.log);
        log.last += 1;
        let seq = log.last;
        let mut payload = seq.to_be_bytes().to_vec();
        payload.extend_from_slice(body);
        log.bytes += payload.len();
        log.entries.push_back(Entry { seq, kind, payload });
        drop(log);
        self.last_output_ms
            .store(holder::unix_ms(), Ordering::Relaxed);
        self.data.notify_waiters();
    }

    fn ack(&self, upto: u64) {
        let mut log = lock(&self.log);
        while log.entries.front().is_some_and(|e| e.seq <= upto) {
            let e = log.entries.pop_front().expect("checked");
            log.bytes -= e.payload.len();
        }
        log.acked = log.acked.max(upto.min(log.last));
        drop(log);
        self.space.notify_waiters();
    }

    /// Entries after `cursor` (copies: the log stays the source of truth).
    fn after(&self, cursor: u64) -> Vec<(u64, u8, Vec<u8>)> {
        lock(&self.log)
            .entries
            .iter()
            .filter(|e| e.seq > cursor)
            .map(|e| (e.seq, e.kind, e.payload.clone()))
            .collect()
    }

    fn exited(&self) -> bool {
        lock(&self.log).exited.is_some()
    }

    /// EOF, then SIGTERM the child's group and SIGKILL it after a grace.
    fn kill(self: &Arc<Self>) {
        let _ = self.stdin.send(StdinOp::Close);
        if self.exited() || self.killed.swap(true, Ordering::SeqCst) {
            return;
        }
        signal_group(self.child_pid, libc::SIGTERM);
        let sh = Arc::clone(self);
        tokio::spawn(async move {
            tokio::time::sleep(KILL_GRACE).await;
            if !sh.exited() {
                signal_group(sh.child_pid, libc::SIGKILL);
            }
        });
    }

    fn claim(&self) -> u64 {
        let gen = self.gen.fetch_add(1, Ordering::SeqCst) + 1;
        *lock(&self.active) = Some(gen);
        self.kick.send_replace(gen);
        gen
    }

    fn release_claim(&self, gen: u64) {
        let mut active = lock(&self.active);
        if *active == Some(gen) {
            *active = None;
            *lock(&self.last_client) = Instant::now();
        }
        drop(active);
        self.wake.notify_one();
    }

    /// [`crate::holder`]'s lifecycle rules. True = exit now.
    fn should_exit(self: &Arc<Self>) -> bool {
        let connected = lock(&self.active).is_some();
        let reachable = self.socket.exists();
        if self.exited() {
            let exited_at = *lock(&self.exited_at).get_or_insert_with(Instant::now);
            if self.released.load(Ordering::SeqCst) || !reachable {
                return true;
            }
            return !connected && exited_at.elapsed() >= self.exit_linger;
        }
        if !connected && (!reachable || lock(&self.last_client).elapsed() >= self.orphan_ttl) {
            // Unreachable for good, or no daemon for a whole TTL: end the
            // child; exit once it is gone.
            self.discard.store(true, Ordering::SeqCst);
            self.ack(u64::MAX);
            self.kill();
            self.released.store(true, Ordering::SeqCst);
        }
        false
    }

    fn control(self: &Arc<Self>, kind: u8) {
        match kind {
            frame::KILL => self.kill(),
            frame::RELEASE => {
                self.released.store(true, Ordering::SeqCst);
                self.wake.notify_one();
            }
            _ => {}
        }
    }
}

fn signal_group(pid: u32, sig: i32) {
    let Ok(pgid) = libc::pid_t::try_from(pid) else {
        return;
    };
    if pgid > 1 {
        // SAFETY: plain syscall on the group our child leads.
        unsafe {
            libc::killpg(pgid, sig);
        }
    }
}

/// A stdio holder process (see the module docs). Returns its exit code.
pub(crate) fn run(request: HolderSpawn) -> i32 {
    let socket = request.socket.clone();
    let listener = match holder::bind_private(&socket) {
        Ok(l) => l,
        Err(code) => return code,
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => return holder::fail(Some(&socket), &format!("runtime: {e}")),
    };
    let code = runtime.block_on(serve(listener, request));
    let _ = std::fs::remove_file(&socket);
    drop(runtime);
    code
}

async fn serve(listener: std::os::unix::net::UnixListener, request: HolderSpawn) -> i32 {
    let socket = request.socket.clone();
    let spec = &request.command;
    let mut cmd = tokio::process::Command::new(&spec.program);
    cmd.args(&spec.args)
        .envs(spec.env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .env_remove(holder::HOLDER_ENV)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    if let Some(cwd) = &spec.cwd {
        cmd.current_dir(cwd);
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return holder::fail(Some(&socket), &format!("spawn {}: {e}", spec.program)),
    };
    let child_pid = child.id().unwrap_or(0);
    let (Some(mut stdin), Some(stdout), Some(stderr)) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        let _ = child.start_kill();
        return holder::fail(Some(&socket), "child stdio missing");
    };
    {
        use std::io::Write;
        let ready = serde_json::json!({
            "holder_pid": std::process::id(),
            "child_pid": child_pid,
        });
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{READY_PREFIX}{ready}");
        let _ = out.flush();
    }
    holder::detach_stdio();

    let (stdin_tx, mut stdin_rx) = mpsc::unbounded_channel::<StdinOp>();
    let started_at_ms = holder::unix_ms();
    let sh = Arc::new(Shared {
        socket,
        holder_pid: std::process::id(),
        child_pid,
        started_at_ms,
        meta: request.meta,
        log: Mutex::new(Log::default()),
        cap: DEFAULT_LOG_CAP,
        data: Notify::new(),
        space: Notify::new(),
        last_output_ms: AtomicU64::new(started_at_ms),
        stdin: stdin_tx,
        released: AtomicBool::new(false),
        killed: AtomicBool::new(false),
        discard: AtomicBool::new(false),
        gen: AtomicU64::new(0),
        kick: watch::channel(0).0,
        active: Mutex::new(None),
        last_client: Mutex::new(Instant::now()),
        exited_at: Mutex::new(None),
        wake: Notify::new(),
        orphan_ttl: request
            .orphan_ttl_secs
            .map(Duration::from_secs)
            .unwrap_or(DEFAULT_ORPHAN_TTL),
        exit_linger: request
            .exit_linger_secs
            .map(Duration::from_secs)
            .unwrap_or(DEFAULT_EXIT_LINGER),
    });

    tokio::spawn(async move {
        while let Some(StdinOp::Line(mut line)) = stdin_rx.recv().await {
            line.push(b'\n');
            if stdin.write_all(&line).await.is_err() || stdin.flush().await.is_err() {
                break;
            }
        }
        // Close (or the pipe broke): EOF for the child.
    });
    let tail = Arc::new(Mutex::new(Vec::<u8>::new()));
    let stderr_task = tokio::spawn(drain_stderr(stderr, Arc::clone(&tail)));
    let reader = tokio::spawn(read_lines(stdout, Arc::clone(&sh)));
    {
        let sh = Arc::clone(&sh);
        tokio::spawn(async move {
            let status = child.wait().await;
            let _ = reader.await;
            let _ = stderr_task.await;
            let payload = ExitPayload {
                code: status.ok().and_then(|s| s.code()),
                stderr_tail: String::from_utf8_lossy(&lock(&tail)).into_owned(),
            };
            let body = serde_json::to_vec(&payload).unwrap_or_default();
            sh.push(kind::EXITED, &body).await;
            lock(&sh.log).exited = Some(payload.code);
            sh.wake.notify_one();
        });
    }

    let _ = listener.set_nonblocking(true);
    let Ok(listener) = tokio::net::UnixListener::from_std(listener) else {
        sh.kill();
        return 1;
    };
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                if let Ok((stream, _)) = accepted {
                    tokio::spawn(serve_client(stream, Arc::clone(&sh)));
                }
            }
            _ = sh.wake.notified() => {}
            _ = tokio::time::sleep(Duration::from_millis(500)) => {}
        }
        if sh.should_exit() {
            return 0;
        }
    }
}

async fn read_lines<R: AsyncRead + Unpin>(stdout: R, sh: Arc<Shared>) {
    let mut reader = BufReader::new(stdout);
    let mut line: Vec<u8> = Vec::new();
    let mut overlong = 0usize;
    while let Ok(buf) = reader.fill_buf().await {
        if buf.is_empty() {
            break;
        }
        let (chunk, done, used) = match buf.iter().position(|&b| b == b'\n') {
            Some(i) => (&buf[..i], true, i + 1),
            None => (buf, false, buf.len()),
        };
        if overlong > 0 || line.len() + chunk.len() > MAX_LINE_BYTES {
            overlong += line.len() + chunk.len();
            line.clear();
        } else {
            line.extend_from_slice(chunk);
        }
        reader.consume(used);
        if done {
            if overlong > 0 {
                let n = std::mem::take(&mut overlong) as u64;
                sh.push(kind::OVERLONG, &n.to_be_bytes()).await;
            } else {
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                if !line.iter().all(u8::is_ascii_whitespace) {
                    sh.push(kind::LINE, &line).await;
                }
                line.clear();
            }
        }
    }
    // A final unterminated line still counts.
    if overlong == 0 && !line.iter().all(u8::is_ascii_whitespace) {
        sh.push(kind::LINE, &line).await;
    }
}

async fn drain_stderr<R: AsyncRead + Unpin>(mut stderr: R, tail: Arc<Mutex<Vec<u8>>>) {
    let mut buf = [0u8; 4096];
    loop {
        match stderr.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let mut t = lock(&tail);
                t.extend_from_slice(&buf[..n]);
                if t.len() > STDERR_TAIL {
                    let cut = t.len() - STDERR_TAIL;
                    t.drain(..cut);
                }
            }
        }
    }
}

fn read_u64(p: &[u8]) -> Option<u64> {
    p.get(..8)
        .map(|b| u64::from_be_bytes(b.try_into().expect("8 bytes")))
}

async fn serve_client(stream: tokio::net::UnixStream, sh: Arc<Shared>) {
    let (mut rd, mut wr) = stream.into_split();
    let hello = match tokio::time::timeout(HANDSHAKE_TIMEOUT, frame::read_async(&mut rd)).await {
        Ok(Ok((frame::HELLO, payload))) => payload,
        _ => return,
    };
    let client_major = serde_json::from_slice::<Hello>(&hello)
        .map(|h| h.proto_major)
        .unwrap_or(0);
    let gen = sh.claim();
    let mut kick = sh.kick.subscribe();
    let info = serde_json::to_vec(&sh.info()).unwrap_or_default();
    if wr
        .write_all(&frame::encode(frame::HELLO_ACK, &info))
        .await
        .is_err()
    {
        sh.release_claim(gen);
        return;
    }
    // A client of another major, or a terminate()-style one (KILL right
    // after the ack): only the frozen control frames count.
    let first = if client_major == PROTO_MAJOR {
        tokio::time::timeout(HANDSHAKE_TIMEOUT, frame::read_async(&mut rd)).await
    } else {
        Ok(Err(std::io::ErrorKind::Unsupported.into()))
    };
    let mut cursor = match first {
        Ok(Ok((kind::REPLAY, p))) if p.len() >= 8 => read_u64(&p).unwrap_or(0),
        other => {
            if let Ok(Ok((k, _))) = other {
                sh.control(k);
            }
            while let Ok((k, _)) = frame::read_async(&mut rd).await {
                sh.control(k);
            }
            sh.release_claim(gen);
            return;
        }
    };
    sh.ack(cursor);

    let superseded = Arc::new(AtomicBool::new(false));
    let (eof_tx, mut eof_rx) = tokio::sync::oneshot::channel::<()>();
    let reader = {
        let sh = Arc::clone(&sh);
        let superseded = Arc::clone(&superseded);
        tokio::spawn(async move {
            while let Ok((k, p)) = frame::read_async(&mut rd).await {
                if superseded.load(Ordering::SeqCst) {
                    sh.control(k);
                    continue;
                }
                match k {
                    kind::STDIN => {
                        let _ = sh.stdin.send(StdinOp::Line(p));
                    }
                    kind::CLOSE_STDIN => {
                        let _ = sh.stdin.send(StdinOp::Close);
                    }
                    kind::ACK => {
                        if let Some(seq) = read_u64(&p) {
                            sh.ack(seq);
                        }
                    }
                    k => sh.control(k),
                }
            }
            let _ = eof_tx.send(());
        })
    };
    'serve: loop {
        let more = sh.data.notified();
        tokio::pin!(more);
        more.as_mut().enable();
        for (seq, k, payload) in sh.after(cursor) {
            if wr.write_all(&frame::encode(k, &payload)).await.is_err() {
                break 'serve;
            }
            cursor = seq;
        }
        tokio::select! {
            _ = &mut more => {}
            changed = kick.changed() => {
                if changed.is_err() || *kick.borrow() != gen {
                    superseded.store(true, Ordering::SeqCst);
                    let _ = wr.write_all(&frame::encode(frame::SUPERSEDED, &[])).await;
                    break;
                }
            }
            _ = &mut eof_rx => break,
        }
    }
    // As the PTY holder: read the control frames the client still sent
    // (a RELEASE racing the end), bounded, then let go.
    superseded.store(true, Ordering::SeqCst);
    let _ = wr.shutdown().await;
    let mut reader = reader;
    let _ = tokio::time::timeout(Duration::from_millis(500), &mut reader).await;
    reader.abort();
    sh.release_claim(gen);
}

// ── the daemon side ─────────────────────────────────────────────────────────

/// What a held child reports, in order. After [`StdioEvent::Exited`] nothing
/// follows. The stream simply ends (no `Exited`) when a newer client took
/// the holder over or this side detached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StdioEvent {
    Line {
        seq: u64,
        text: String,
    },
    Overlong {
        seq: u64,
        len: usize,
    },
    Exited {
        seq: u64,
        code: Option<i32>,
        stderr_tail: String,
    },
}

/// What the daemon tells a held child. Dropping every sender without
/// [`StdioControl::Detach`] closes the child's stdin and releases the holder
/// (the session is over); `Detach` leaves both running for the next daemon.
#[derive(Debug, Clone)]
pub enum StdioControl {
    Line(String),
    Ack(u64),
    CloseStdin,
    Detach,
}

/// An attached stdio holder.
pub struct HeldStdio {
    pub socket: PathBuf,
    pub info: HolderInfo,
    pub events: mpsc::Receiver<StdioEvent>,
    pub control: mpsc::UnboundedSender<StdioControl>,
}

/// Start a stdio holder running `spec` (blocking: waits for its READY line)
/// and attach to it from the start.
pub async fn spawn(
    config: &HolderConfig,
    spec: &CommandSpec,
    meta: serde_json::Value,
) -> Result<HeldStdio> {
    let (config, spec) = (config.clone(), spec.clone());
    let socket = tokio::task::spawn_blocking(move || holder::launch(&config, &spec, None, meta))
        .await
        .map_err(|e| otto_core::Error::Internal(format!("stdio holder launch: {e}")))??;
    match attach(&socket, 0).await {
        Ok(h) => Ok(h),
        Err(e) => {
            let s = socket.clone();
            let _ = tokio::task::spawn_blocking(move || holder::terminate(&s)).await;
            Err(otto_core::Error::Internal(format!(
                "attach to new stdio holder: {e}"
            )))
        }
    }
}

/// Attach to the stdio holder at `socket`, replaying every entry after
/// `after` (what this side has durably processed).
pub async fn attach(socket: &Path, after: u64) -> std::result::Result<HeldStdio, AdoptError> {
    let stream = match tokio::net::UnixStream::connect(socket).await {
        Ok(s) => s,
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
            ) =>
        {
            let _ = std::fs::remove_file(socket);
            return Err(AdoptError::Stale);
        }
        Err(e) => return Err(AdoptError::Failed(format!("connect: {e}"))),
    };
    let (mut rd, mut wr) = stream.into_split();
    let hello = serde_json::to_vec(&Hello::ours()).unwrap_or_default();
    let failed = |e: std::io::Error| AdoptError::Failed(format!("stdio holder handshake: {e}"));
    wr.write_all(&frame::encode(frame::HELLO, &hello))
        .await
        .map_err(failed)?;
    let (k, payload) = tokio::time::timeout(HANDSHAKE_TIMEOUT, frame::read_async(&mut rd))
        .await
        .map_err(|_| AdoptError::Failed("stdio holder handshake timed out".into()))?
        .map_err(failed)?;
    if k != frame::HELLO_ACK {
        return Err(AdoptError::Failed(format!("unexpected frame {k:#x}")));
    }
    let info: HolderInfo = serde_json::from_slice(&payload)
        .map_err(|e| AdoptError::Failed(format!("bad holder info: {e}")))?;
    if info.proto_major != PROTO_MAJOR {
        return Err(AdoptError::Incompatible(Box::new(info)));
    }
    let Some(log) = info.stdio.clone() else {
        return Err(AdoptError::Failed("not a stdio holder".into()));
    };
    if after < log.acked {
        return Err(AdoptError::Failed(format!(
            "the holder already dropped lines up to {} (asked for after {after})",
            log.acked
        )));
    }
    wr.write_all(&frame::encode(kind::REPLAY, &after.to_be_bytes()))
        .await
        .map_err(failed)?;

    let detached = Arc::new(AtomicBool::new(false));
    let (ev_tx, ev_rx) = mpsc::channel::<StdioEvent>(256);
    let (ctl_tx, mut ctl_rx) = mpsc::unbounded_channel::<StdioControl>();
    let reader = {
        let detached = Arc::clone(&detached);
        tokio::spawn(async move {
            loop {
                let ev = match frame::read_async(&mut rd).await {
                    Ok((kind::LINE, p)) if p.len() >= 8 => StdioEvent::Line {
                        seq: read_u64(&p).unwrap_or(0),
                        text: String::from_utf8_lossy(&p[8..]).into_owned(),
                    },
                    Ok((kind::OVERLONG, p)) if p.len() >= 16 => StdioEvent::Overlong {
                        seq: read_u64(&p).unwrap_or(0),
                        len: read_u64(&p[8..]).unwrap_or(0) as usize,
                    },
                    Ok((kind::EXITED, p)) if p.len() >= 8 => {
                        let x: ExitPayload =
                            serde_json::from_slice(&p[8..]).unwrap_or(ExitPayload {
                                code: None,
                                stderr_tail: String::new(),
                            });
                        let _ = ev_tx
                            .send(StdioEvent::Exited {
                                seq: read_u64(&p).unwrap_or(0),
                                code: x.code,
                                stderr_tail: x.stderr_tail,
                            })
                            .await;
                        return;
                    }
                    // A newer daemon took over: end quietly.
                    Ok((frame::SUPERSEDED, _)) => return,
                    Ok(_) => continue,
                    Err(_) => {
                        if !detached.load(Ordering::SeqCst) {
                            let _ = ev_tx
                                .send(StdioEvent::Exited {
                                    seq: 0,
                                    code: None,
                                    stderr_tail: "the agent's holder process ended unexpectedly"
                                        .into(),
                                })
                                .await;
                        }
                        return;
                    }
                };
                if ev_tx.send(ev).await.is_err() {
                    return;
                }
            }
        })
    };
    tokio::spawn(async move {
        let mut close = true;
        while let Some(op) = ctl_rx.recv().await {
            let f = match op {
                StdioControl::Line(l) => frame::encode(kind::STDIN, l.as_bytes()),
                StdioControl::Ack(seq) => frame::encode(kind::ACK, &seq.to_be_bytes()),
                StdioControl::CloseStdin => frame::encode(kind::CLOSE_STDIN, &[]),
                StdioControl::Detach => {
                    detached.store(true, Ordering::SeqCst);
                    close = false;
                    break;
                }
            };
            if wr.write_all(&f).await.is_err() {
                close = false;
                break;
            }
        }
        if close {
            // The session is over: EOF for the child, and the holder may go
            // once it has exited.
            let _ = wr.write_all(&frame::encode(kind::CLOSE_STDIN, &[])).await;
            let _ = wr.write_all(&frame::encode(frame::RELEASE, &[])).await;
        }
        let _ = wr.shutdown().await;
        reader.abort();
    });
    Ok(HeldStdio {
        socket: socket.to_path_buf(),
        info,
        events: ev_rx,
        control: ctl_tx,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shared(cap: usize) -> Arc<Shared> {
        Arc::new(Shared {
            socket: PathBuf::new(),
            holder_pid: 1,
            child_pid: 0,
            started_at_ms: 0,
            meta: serde_json::Value::Null,
            log: Mutex::new(Log::default()),
            cap,
            data: Notify::new(),
            space: Notify::new(),
            last_output_ms: AtomicU64::new(0),
            stdin: mpsc::unbounded_channel().0,
            released: AtomicBool::new(false),
            killed: AtomicBool::new(false),
            discard: AtomicBool::new(false),
            gen: AtomicU64::new(0),
            kick: watch::channel(0).0,
            active: Mutex::new(None),
            last_client: Mutex::new(Instant::now()),
            exited_at: Mutex::new(None),
            wake: Notify::new(),
            orphan_ttl: DEFAULT_ORPHAN_TTL,
            exit_linger: DEFAULT_EXIT_LINGER,
        })
    }

    #[tokio::test]
    async fn lines_are_numbered_kept_until_acked_and_replayed_after_a_cursor() {
        let sh = shared(usize::MAX);
        let (mut w, r) = tokio::io::duplex(1 << 16);
        let reader = tokio::spawn(read_lines(r, Arc::clone(&sh)));
        w.write_all(b"one\r\n\n  \ntwo\nthree").await.unwrap();
        drop(w);
        reader.await.unwrap();
        let all = sh.after(0);
        let lines: Vec<(u64, Vec<u8>)> =
            all.iter().map(|(s, _, p)| (*s, p[8..].to_vec())).collect();
        assert_eq!(
            lines,
            vec![
                (1, b"one".to_vec()),
                (2, b"two".to_vec()),
                (3, b"three".to_vec())
            ]
        );
        assert_eq!(sh.after(1).len(), 2, "replay after a cursor");
        sh.ack(2);
        assert_eq!(sh.after(0).iter().map(|e| e.0).collect::<Vec<_>>(), vec![3]);
        assert_eq!(
            sh.info().stdio,
            Some(StdioLog { acked: 2, last: 3 }),
            "acked lines are gone, the position is reported"
        );
    }

    #[tokio::test]
    async fn a_full_log_pauses_the_reader_until_acked_and_drops_nothing() {
        let sh = shared(10);
        let (mut w, r) = tokio::io::duplex(1 << 16);
        let reader = tokio::spawn(read_lines(r, Arc::clone(&sh)));
        w.write_all(b"aaaa\nbbbb\ncccc\n").await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        // 8-byte seq + 4 bytes = 12 >= cap after the first line.
        assert_eq!(sh.after(0).len(), 1, "paused at the cap");
        sh.ack(1);
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(sh.after(1).len(), 1);
        sh.ack(2);
        drop(w);
        reader.await.unwrap();
        let rest: Vec<Vec<u8>> = sh.after(2).into_iter().map(|e| e.2[8..].to_vec()).collect();
        assert_eq!(rest, vec![b"cccc".to_vec()]);
    }

    #[tokio::test]
    async fn an_overlong_line_is_reported_with_its_length() {
        let sh = shared(usize::MAX);
        let (mut w, r) = tokio::io::duplex(1 << 16);
        let reader = tokio::spawn(read_lines(r, Arc::clone(&sh)));
        let writer = tokio::spawn(async move {
            let chunk = vec![b'x'; 1 << 20];
            for _ in 0..17 {
                w.write_all(&chunk).await.unwrap();
            }
            w.write_all(b"\nafter\n").await.unwrap();
        });
        writer.await.unwrap();
        reader.await.unwrap();
        let all = sh.after(0);
        assert_eq!(all[0].1, kind::OVERLONG);
        assert!(read_u64(&all[0].2[8..]).unwrap() as usize > MAX_LINE_BYTES);
        assert_eq!(&all[1].2[8..], b"after");
    }
}
