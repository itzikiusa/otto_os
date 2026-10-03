//! otto-pty — portable-pty wrapper: spawn, resize, ring-buffer scrollback,
//! broadcast of output chunks, kill, exit watch.

pub mod ring;

pub mod input_authority;
pub use input_authority::InputAuthorization;

mod echo;
pub use echo::EchoStats;
mod held;
pub mod holder;
pub use holder::{AdoptError, HolderConfig, HolderInfo, HolderLauncher};

use std::io::Read;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, SyncSender, TrySendError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use bytes::Bytes;
use otto_core::{Error, Result};
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, watch};

use crate::ring::RingBuffer;

/// Default terminal size on spawn.
pub const DEFAULT_COLS: u16 = 80;
pub const DEFAULT_ROWS: u16 = 24;

/// Sane column bounds for a restored grid: outside this range we fall back to
/// the default to guard against corrupt or zero-valued metadata.
pub const MIN_COLS: u16 = 20;
pub const MAX_COLS: u16 = 500;

/// Sane row bounds for a restored grid.
pub const MIN_ROWS: u16 = 5;
pub const MAX_ROWS: u16 = 200;

/// Clamp and validate caller-supplied grid dimensions, falling back to
/// `DEFAULT_COLS × DEFAULT_ROWS` when either value is out of range.
///
/// Used by `PtyHandle::spawn_sized` so the caller can pass raw metadata values
/// directly and always get a safe result.
pub fn resolve_grid(cols: Option<u16>, rows: Option<u16>) -> (u16, u16) {
    let c = cols.unwrap_or(DEFAULT_COLS);
    let r = rows.unwrap_or(DEFAULT_ROWS);
    let c = if (MIN_COLS..=MAX_COLS).contains(&c) {
        c
    } else {
        DEFAULT_COLS
    };
    let r = if (MIN_ROWS..=MAX_ROWS).contains(&r) {
        r
    } else {
        DEFAULT_ROWS
    };
    (c, r)
}
/// Capacity of the output broadcast channel (chunks).
const BROADCAST_CAPACITY: usize = 1024;

/// Pause between kill-escalation steps: `SIGHUP` → grace → `SIGTERM` → grace
/// → `SIGKILL`, each step skipped once the child has exited.
pub const KILL_GRACE: Duration = Duration::from_secs(2);

/// Depth (in write jobs) of a PTY's input queue. The macOS tty input queue is
/// ~1 KiB, so a child that is not reading its terminal (TUI still booting, a
/// hung event loop, a stopped job) stalls the writer; jobs beyond this depth
/// are refused instead of parking more callers behind it.
const INPUT_QUEUE_DEPTH: usize = 256;

/// Signalling state of the direct child, shared with the waiter thread.
///
/// The waiter observes the exit WITHOUT reaping (`waitid(WNOWAIT)`), flips
/// `exited` under the mutex, and only then reaps. A signaller checks `exited`
/// under the same mutex, so it only ever signals a pid that still belongs to
/// our (possibly zombie) child — never a recycled pid.
#[derive(Default)]
struct ChildState {
    exited: Mutex<bool>,
    cv: Condvar,
}

impl ChildState {
    fn mark_exited(&self) {
        *lock_unpoisoned(&self.exited) = true;
        self.cv.notify_all();
    }

    fn has_exited(&self) -> bool {
        *lock_unpoisoned(&self.exited)
    }

    /// Block up to `timeout` for the child to exit; true once it has.
    fn wait_exited(&self, timeout: Duration) -> bool {
        let guard = lock_unpoisoned(&self.exited);
        let (guard, _) = self
            .cv
            .wait_timeout_while(guard, timeout, |exited| !*exited)
            .unwrap_or_else(|e| e.into_inner());
        *guard
    }

    /// Send `sig` to the child's process group (the child is a session leader,
    /// so its pgid is its pid) and to `fg_pgrp` — the terminal's foreground
    /// job, when that is a different group — unless the child already exited.
    /// Returns whether anything was signalled.
    fn signal(&self, pid: u32, fg_pgrp: Option<i32>, sig: i32) -> bool {
        let exited = lock_unpoisoned(&self.exited);
        if *exited {
            return false;
        }
        signal_tree(pid as i32, fg_pgrp, sig);
        drop(exited);
        true
    }
}

#[cfg(unix)]
fn signal_tree(pid: i32, fg_pgrp: Option<i32>, sig: i32) {
    // SAFETY: plain signal syscalls on ids owned by this PTY's session; the
    // caller guarantees `pid` is our not-yet-reaped child.
    unsafe {
        if libc::killpg(pid, sig) != 0 {
            libc::kill(pid, sig);
        }
        if let Some(pg) = fg_pgrp {
            if pg > 0 && pg != pid {
                libc::killpg(pg, sig);
            }
        }
    }
}

#[cfg(not(unix))]
fn signal_tree(_pid: i32, _fg_pgrp: Option<i32>, _sig: i32) {}

#[cfg(unix)]
const SIGHUP: i32 = libc::SIGHUP;
#[cfg(unix)]
const SIGTERM: i32 = libc::SIGTERM;
#[cfg(unix)]
const SIGKILL: i32 = libc::SIGKILL;
#[cfg(not(unix))]
const SIGHUP: i32 = 1;
#[cfg(not(unix))]
const SIGTERM: i32 = 15;
#[cfg(not(unix))]
const SIGKILL: i32 = 9;

/// Block until `pid` has exited, WITHOUT reaping it (it stays a zombie, so its
/// pid cannot be recycled until the caller reaps it). Returns on any error too
/// — the caller then reaps as before.
#[cfg(unix)]
fn wait_exit_unreaped(pid: u32) {
    loop {
        // SAFETY: `siginfo_t` is plain data; all-zero is a valid out-param.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let rc = unsafe {
            libc::waitid(
                libc::P_PID,
                pid as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOWAIT,
            )
        };
        if rc == 0 {
            return;
        }
        if std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
            return;
        }
    }
}

#[cfg(not(unix))]
fn wait_exit_unreaped(_pid: u32) {}

/// Who is waiting for a queued input write to reach the PTY.
enum WriteDone {
    /// A synchronous caller blocked on the write (old `write` semantics).
    Blocking(SyncSender<std::io::Result<()>>),
    /// An async caller awaiting it with a timeout (never parks a worker).
    Async(tokio::sync::oneshot::Sender<std::io::Result<()>>),
}

struct WriteJob {
    data: Vec<u8>,
    authorization: Option<InputAuthorization>,
    done: WriteDone,
}

/// Scrollback rows retained by the vt100 emulator — the replay depth clients
/// rebuild from on reconnect. See the comment at the `Parser` construction in
/// [`PtyHandle::spawn_sized`] for the memory tradeoff.
pub const EMULATOR_SCROLLBACK_LINES: usize = 4000;

/// Scrollback rows kept while NOBODY has viewed a terminal for a while
/// (daemon-core perf F9): the session manager lowers the emulator's cap to
/// this for long-unviewed sessions ([`PtyHandle::set_history_cap`]) and
/// restores [`EMULATOR_SCROLLBACK_LINES`] on the next attach. Rows beyond it
/// are dropped (the raw ring still holds recent bytes for search).
pub const UNVIEWED_SCROLLBACK_LINES: usize = 1000;

/// Emulator memory accounting ([`PtyHandle::emulator_stats`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct EmulatorStats {
    /// Scrollback rows retained.
    pub scrollback_rows: usize,
    /// The current scrollback cap.
    pub scrollback_cap: usize,
    /// Approximate heap held by scrollback cells, in bytes.
    pub scrollback_bytes: usize,
}

/// A fully-resolved command to run inside a PTY.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub env: Vec<(String, String)>,
}

/// Where the PTY master and the child actually live.
enum Backend {
    /// In this process: the classic spawn. The child dies with this process
    /// (its PTY master closes → SIGHUP), and [`PtyHandle`]'s `Drop` kills it.
    Local {
        master: Mutex<Box<dyn MasterPty + Send>>,
        /// Fallback killer for a child whose pid the OS didn't report.
        killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    },
    /// In a detached PTY-holder process ([`holder`]) reached over a unix
    /// socket: the child survives a restart of this process, and a new one
    /// re-adopts it ([`PtyHandle::adopt`]). This handle keeps a local mirror
    /// (emulator + ring + broadcast) fed by the holder's output stream, so
    /// every read API behaves exactly as for a local PTY.
    Held(Arc<held::HeldConn>),
}

/// A live PTY child process: write input, watch output, observe exit.
pub struct PtyHandle {
    backend: Backend,
    /// Bounded input queue drained by a dedicated writer thread, so a child
    /// that stops reading its tty blocks that thread — never a daemon worker.
    input_tx: SyncSender<WriteJob>,
    /// Exit state shared with the waiter thread (pid-reuse-safe signalling).
    child_state: Arc<ChildState>,
    /// Ring + headless terminal emulator tracking the CURRENT screen (so a
    /// fresh attach can reproduce the live screen in one coherent frame — no
    /// replay flicker, no clipped TUI; this is what tmux does) + the live
    /// output broadcast + the last-output clock (relative to `mirror.epoch`,
    /// the instant the handle was created).
    mirror: Mirror,
    exit_rx: watch::Receiver<Option<i32>>,
    /// Daemon-unique spawn counter. A client that sees this change between
    /// attaches knows the process was respawned (resume/restart) and its local
    /// scrollback belongs to a dead process — rebuild from the snapshot instead
    /// of appending to stale content. An adopted (re-attached) held PTY gets a
    /// fresh one too: clients reconnecting after a daemon restart rebuild from
    /// the adoption snapshot.
    spawn_seq: u64,
    /// OS pid of the direct child, captured at spawn (None if the OS didn't
    /// report one). Used by the idle-suspend sweep to check for live descendant
    /// processes before killing a "quiet" session. For a held PTY this is the
    /// child's pid as the holder reported it (the holder is its parent).
    child_pid: Option<u32>,
    /// Flips to `true` once the output stream has ended (local: the reader
    /// thread saw EOF; held: the holder reported the exit). Lets a holder send
    /// its final output before the exit notice.
    reader_done: watch::Receiver<bool>,
}

/// The output side shared by both backends: the screen emulator, the raw
/// ring, the live broadcast and the last-output clock. A local PTY's reader
/// thread and a held PTY's socket reader feed it the same way.
#[derive(Clone)]
pub(crate) struct Mirror {
    ring: Arc<Mutex<RingBuffer>>,
    parser: Arc<Mutex<vt100::Parser>>,
    tx: broadcast::Sender<Bytes>,
    epoch: Instant,
    last_output_ms: Arc<AtomicU64>,
    /// Keystroke → first-output clock shared with the writer thread.
    echo: Arc<echo::EchoClock>,
}

impl Mirror {
    fn new(cols: u16, rows: u16, ring: RingBuffer) -> Self {
        let (tx, _) = broadcast::channel::<Bytes>(BROADCAST_CAPACITY);
        Self {
            ring: Arc::new(Mutex::new(ring)),
            // Scrollback history kept by the emulator — this is the depth a
            // client gets back on every reconnect rebuild, so it IS the
            // user-visible scrollback for reopened sessions. Rows cost
            // cols×32 bytes, so the cap trades replay depth against
            // per-live-session memory (~25 MiB worst case at 200 cols when a
            // long session fills it). Initialise at the requested grid size so
            // the emulator agrees with the PTY from the very first byte —
            // avoids a spurious SIGWINCH on reconnect when the client echoes
            // back the same dimensions we already reported.
            parser: Arc::new(Mutex::new(vt100::Parser::new(
                rows,
                cols,
                EMULATOR_SCROLLBACK_LINES,
            ))),
            tx,
            epoch: Instant::now(),
            last_output_ms: Arc::new(AtomicU64::new(0)),
            echo: Arc::new(echo::EchoClock::new()),
        }
    }

    /// Make the last-output clock read `age` ago (an adopted holder's real
    /// last output). Moves the epoch back — saturating at what `Instant` can
    /// represent — so [`PtyHandle::uptime`] grows by the same amount, which
    /// stays truthful: the child has been running at least that long.
    pub(crate) fn backdate(&mut self, age: Duration) {
        if let Some(epoch) = Instant::now().checked_sub(age) {
            self.epoch = epoch;
            self.last_output_ms.store(0, Ordering::Relaxed);
        }
    }

    /// One chunk of child output → emulator + ring + broadcast.
    pub(crate) fn feed(&self, data: &[u8]) {
        self.last_output_ms
            .store(self.epoch.elapsed().as_millis() as u64, Ordering::Relaxed);
        self.echo.output();
        // Publish under the same lock as the emulator update:
        // snapshot_and_subscribe must never include a chunk in its replay and
        // then receive that chunk again.
        let mut parser = lock_unpoisoned(&self.parser);
        parser.process(data);
        // No receivers is fine — the screen state still records.
        let _ = self.tx.send(Bytes::copy_from_slice(data));
        drop(parser);
        // The raw ring is not part of the snapshot hand-over (only emulator
        // + broadcast must be atomic), so it is filled AFTER the parser lock
        // is released (perf 01 F3): its line split + allocations no longer
        // lengthen the window in which captures and resizes wait.
        lock_unpoisoned(&self.ring).push(data);
    }

    /// Replace the emulator with one rebuilt from a holder snapshot (a fresh
    /// adoption, or a resync after the holder dropped a lagging stream). Live
    /// viewers get the snapshot bytes too — it is a full repaint.
    pub(crate) fn reset_to(&self, cols: u16, rows: u16, snapshot: &[u8]) {
        let mut parser = lock_unpoisoned(&self.parser);
        let mut fresh = vt100::Parser::new(rows, cols, EMULATOR_SCROLLBACK_LINES);
        fresh.process(snapshot);
        *parser = fresh;
        lock_unpoisoned(&self.ring).push(snapshot);
        let _ = self.tx.send(Bytes::copy_from_slice(snapshot));
    }
}

/// Emulator state copied under the parser lock, to be formatted AFTER the
/// lock is released (r3-06-02). Formatting a snapshot walks every retained
/// cell and emits escape codes; doing that under the lock stalled the PTY
/// reader thread — i.e. that session's live output — for the whole walk, and
/// callers did it on async workers. The copy is a flat memcpy of the grid
/// (32-byte cells), a fraction of the formatting cost.
pub struct ScreenCapture {
    screen: vt100::Screen,
}

impl ScreenCapture {
    /// Emulator grid at capture time as `(cols, rows)`.
    pub fn size(&self) -> (u16, u16) {
        let (rows, cols) = self.screen.size();
        (cols, rows)
    }

    /// The snapshot bytes [`PtyHandle::snapshot_with_history`] would return
    /// for this state. CPU-heavy for deep histories: call it off the async
    /// workers (`spawn_blocking`).
    pub fn format(&self, lines: usize) -> Vec<u8> {
        PtyHandle::format_snapshot(&self.screen, lines)
    }
}

/// A coherent replay followed by only the output produced after that replay.
pub struct OutputSnapshot {
    pub data: Vec<u8>,
    pub cols: u16,
    pub rows: u16,
    pub output: broadcast::Receiver<Bytes>,
}

fn lock_unpoisoned<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl PtyHandle {
    /// Spawn `spec` in a fresh 80×24 PTY (default size). A blocking reader
    /// thread pumps output into the scrollback ring and the broadcast channel;
    /// a waiter thread reaps the child and publishes the exit code.
    ///
    /// When you have a previously-saved grid (e.g. from session metadata) use
    /// [`PtyHandle::spawn_sized`] to restore the exact dimensions instead.
    pub fn spawn(spec: &CommandSpec) -> Result<PtyHandle> {
        Self::spawn_sized(spec, DEFAULT_COLS, DEFAULT_ROWS)
    }

    /// Spawn `spec` at the given `cols × rows` grid size, restoring a
    /// previously-saved terminal size on resume so the session reopens at
    /// exactly the dimensions the user had. Values are **not** clamped here —
    /// call [`resolve_grid`] first to sanitise raw metadata.
    pub fn spawn_sized(spec: &CommandSpec, cols: u16, rows: u16) -> Result<PtyHandle> {
        Self::spawn_local(spec, cols, rows, RingBuffer::default())
    }

    /// [`Self::spawn_sized`] with an explicit raw-ring size (a PTY holder
    /// never reads its ring, so it keeps a token one).
    pub(crate) fn spawn_local(
        spec: &CommandSpec,
        cols: u16,
        rows: u16,
        ring: RingBuffer,
    ) -> Result<PtyHandle> {
        let pty = native_pty_system();
        let pair = pty
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| Error::Internal(format!("openpty: {e}")))?;

        let mut cmd = CommandBuilder::new(&spec.program);
        cmd.args(&spec.args);
        if let Some(cwd) = &spec.cwd {
            cmd.cwd(cwd);
        }
        // The holder's own launch marker must never leak into the session.
        cmd.env_remove(holder::HOLDER_ENV);

        // Baseline terminal environment. The daemon is launched by launchd with
        // a minimal env (often no TERM/COLORTERM/LANG), which makes full-screen
        // TUIs like claude/codex fall back to a degraded inline renderer with no
        // input composer. Provide sane defaults unless the caller overrides them.
        let has = |key: &str| spec.env.iter().any(|(k, _)| k == key);
        if !has("TERM") && std::env::var_os("TERM").is_none() {
            cmd.env("TERM", "xterm-256color");
        }
        if !has("COLORTERM") && std::env::var_os("COLORTERM").is_none() {
            cmd.env("COLORTERM", "truecolor");
        }
        if !has("LANG") && std::env::var_os("LANG").is_none() {
            cmd.env("LANG", "en_US.UTF-8");
        }

        for (k, v) in &spec.env {
            cmd.env(k, v);
        }

        let mut child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| Error::Internal(format!("spawn {}: {e}", spec.program)))?;
        // Close our copy of the slave so the master sees EOF when the child exits.
        drop(pair.slave);

        let killer = child.clone_killer();
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| Error::Internal(format!("pty reader: {e}")))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| Error::Internal(format!("pty writer: {e}")))?;

        let (exit_tx, exit_rx) = watch::channel::<Option<i32>>(None);
        let (done_tx, reader_done) = watch::channel(false);
        let mirror = Mirror::new(cols, rows, ring);

        // Blocking reader thread: PTY output -> screen emulator + ring + broadcast.
        {
            let mirror = mirror.clone();
            std::thread::spawn(move || {
                let mut buf = [0u8; 8192];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => mirror.feed(&buf[..n]),
                    }
                }
                let _ = done_tx.send(true);
            });
        }

        // Capture the child's OS pid before the waiter thread takes ownership.
        let child_pid = child.process_id();
        let child_state = Arc::new(ChildState::default());

        // Waiter thread: observe the exit, mark it (so no signal can target a
        // recycled pid), THEN reap the child and publish the exit code.
        {
            let child_state = Arc::clone(&child_state);
            std::thread::spawn(move || {
                if let Some(pid) = child_pid {
                    wait_exit_unreaped(pid);
                }
                child_state.mark_exited();
                let code = match child.wait() {
                    Ok(status) => status.exit_code() as i32,
                    Err(_) => -1,
                };
                let _ = exit_tx.send(Some(code));
            });
        }

        let input_tx = spawn_writer(Box::new(writer), Arc::clone(&mirror.echo));

        Ok(PtyHandle {
            backend: Backend::Local {
                master: Mutex::new(pair.master),
                killer: Mutex::new(killer),
            },
            input_tx,
            child_state,
            mirror,
            exit_rx,
            spawn_seq: next_spawn_seq(),
            child_pid,
            reader_done,
        })
    }

    /// Spawn `spec` inside a detached PTY holder ([`holder`]) so the child
    /// survives a restart of this process, then attach to it. `meta` is opaque
    /// to the holder and handed back verbatim to whoever adopts it later
    /// ([`HolderInfo::meta`]) — the caller's key to which session it is.
    ///
    /// Blocking (process launch + handshake): call it off the async workers.
    pub fn spawn_held(
        config: &HolderConfig,
        spec: &CommandSpec,
        cols: u16,
        rows: u16,
        meta: serde_json::Value,
    ) -> Result<PtyHandle> {
        holder::spawn_and_attach(config, spec, cols, rows, meta)
    }

    /// Re-attach to a live PTY holder at `socket` (e.g. after this process
    /// restarted): the returned handle replays the holder's screen + history
    /// into its emulator and streams from there on, exactly like the handle
    /// that spawned it. Blocking: call it off the async workers.
    pub fn adopt(socket: &std::path::Path) -> std::result::Result<PtyHandle, AdoptError> {
        held::adopt(socket)
    }

    /// Assemble a held handle (see [`held::adopt`]).
    pub(crate) fn from_held(
        conn: Arc<held::HeldConn>,
        mirror: Mirror,
        child_state: Arc<ChildState>,
        exit_rx: watch::Receiver<Option<i32>>,
        reader_done: watch::Receiver<bool>,
    ) -> PtyHandle {
        let input_tx = spawn_writer(
            Box::new(held::HeldWriter::new(Arc::clone(&conn))),
            Arc::clone(&mirror.echo),
        );
        let child_pid = conn.info().child_pid;
        PtyHandle {
            backend: Backend::Held(conn),
            input_tx,
            child_state,
            mirror,
            exit_rx,
            spawn_seq: next_spawn_seq(),
            child_pid,
            reader_done,
        }
    }

    /// The holder behind this PTY, when it is held ([`Self::spawn_held`] /
    /// [`Self::adopt`]); `None` for a local PTY.
    pub fn holder(&self) -> Option<&HolderInfo> {
        match &self.backend {
            Backend::Held(conn) => Some(conn.info()),
            Backend::Local { .. } => None,
        }
    }

    /// The holder's socket path, when held.
    pub fn holder_socket(&self) -> Option<&std::path::Path> {
        match &self.backend {
            Backend::Held(conn) => Some(conn.path()),
            Backend::Local { .. } => None,
        }
    }

    /// Let go of a held PTY WITHOUT ending it: dropping this handle afterwards
    /// closes the connection but neither kills the child nor releases the
    /// holder, so the next daemon run can [`Self::adopt`] it. A no-op for a
    /// local PTY (which cannot outlive this process anyway).
    pub fn detach(&self) {
        if let Backend::Held(conn) = &self.backend {
            conn.detach();
        }
    }

    /// Wait (blocking, at most `timeout`) until the output stream has ended.
    /// True once it has.
    pub fn wait_output_closed_blocking(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if *self.reader_done.borrow() {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Watch the end of the output stream (see the `reader_done` field).
    pub fn output_closed(&self) -> watch::Receiver<bool> {
        self.reader_done.clone()
    }

    /// OS pid of the direct child process (None when the OS didn't report one).
    pub fn pid(&self) -> Option<u32> {
        self.child_pid
    }

    /// Write bytes to the child's stdin, blocking until they reached the PTY
    /// (the synchronous API; use [`Self::write_async`] from async code).
    pub fn write(&self, data: &[u8]) -> Result<()> {
        let (tx, rx) = sync_channel(1);
        self.input_tx
            .send(WriteJob {
                data: data.to_vec(),
                authorization: None,
                done: WriteDone::Blocking(tx),
            })
            .map_err(|_| input_closed())?;
        match rx.recv() {
            Ok(res) => res.map_err(|e| Error::Internal(format!("pty write: {e}"))),
            Err(_) => Err(input_closed()),
        }
    }

    /// Queue bytes for the child's stdin and await their delivery for at most
    /// `timeout`, without ever blocking the calling thread. Writes keep their
    /// order (one queue per PTY, shared with [`Self::write`]).
    ///
    /// Errors: `Conflict` when the child is not accepting input — the queue is
    /// full, or the bytes weren't drained within `timeout` (they then stay
    /// queued and are delivered if the child resumes reading); `Internal` when
    /// the PTY is gone.
    pub async fn write_async(&self, data: &[u8], timeout: Duration) -> Result<()> {
        self.write_async_authorized(data, timeout, None).await
    }

    /// Carry revocable room authority through the queue to the actual writer.
    /// Timeout retains legacy delivery semantics, but an epoch change discards
    /// a queued job (or any unwritten tail) when the writer next makes progress.
    pub async fn write_async_authorized(
        &self,
        data: &[u8],
        timeout: Duration,
        authorization: Option<InputAuthorization>,
    ) -> Result<()> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        match self.input_tx.try_send(WriteJob {
            data: data.to_vec(),
            authorization,
            done: WriteDone::Async(tx),
        }) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                return Err(Error::Conflict(
                    "session is not accepting input (its input queue is full — the process is not reading its terminal)".into(),
                ))
            }
            Err(TrySendError::Disconnected(_)) => return Err(input_closed()),
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(res)) => res.map_err(|e| {
                if e.kind() == std::io::ErrorKind::PermissionDenied {
                    Error::Forbidden("terminal control changed".into())
                } else {
                    Error::Internal(format!("pty write: {e}"))
                }
            }),
            Ok(Err(_)) => Err(input_closed()),
            Err(_) => Err(Error::Conflict(format!(
                "session is not accepting input (not drained within {}s; still queued)",
                timeout.as_secs()
            ))),
        }
    }

    /// Keystroke-echo statistics: input reaching the PTY → the child's first
    /// output after it (latency diagnostics, the terminal HUD's "child echo").
    pub fn echo_stats(&self) -> EchoStats {
        self.mirror.echo.stats()
    }

    /// Current emulator grid as `(cols, rows)`.
    pub fn size(&self) -> (u16, u16) {
        let parser = lock_unpoisoned(&self.mirror.parser);
        let (rows, cols) = parser.screen().size();
        (cols, rows)
    }

    /// Resize the terminal (PTY + the screen emulator).
    ///
    /// Same-size calls are dropped entirely — no emulator rewrap and no
    /// TIOCSWINSZ — so clients can re-push their grid unconditionally on
    /// focus/reconnect without triggering a TUI repaint (codex re-emits its
    /// transcript on every SIGWINCH; each spurious one pollutes scrollback).
    ///
    /// A real resize REFLOWS the emulator's whole retained history (up to
    /// [`EMULATOR_SCROLLBACK_LINES`] rows) under the parser lock — it must be
    /// atomic with the reader thread's `process`, or output parsed at the old
    /// width would land in a half-reflowed grid. That reflow is synchronous
    /// CPU work, so on a multi-threaded tokio runtime it runs inside
    /// `block_in_place`: the calling worker hands its other tasks to the pool
    /// instead of stalling them (r3-06-02).
    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        let mut parser = lock_unpoisoned(&self.mirror.parser);
        if parser.screen().size() == (rows, cols) {
            return Ok(());
        }
        let multi_thread = tokio::runtime::Handle::try_current()
            .is_ok_and(|h| h.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread);
        if multi_thread {
            tokio::task::block_in_place(|| parser.screen_mut().set_size(rows, cols));
        } else {
            parser.screen_mut().set_size(rows, cols);
        }
        drop(parser);
        match &self.backend {
            Backend::Local { master, .. } => lock_unpoisoned(master)
                .resize(PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .map_err(|e| Error::Internal(format!("pty resize: {e}"))),
            // The holder applies TIOCSWINSZ to the real PTY and reflows its
            // own emulator (the one future adoptions are rebuilt from).
            Backend::Held(conn) => conn
                .resize(cols, rows)
                .map_err(|e| Error::Internal(format!("pty resize (holder): {e}"))),
        }
    }

    /// Change the emulator's scrollback cap (see
    /// [`UNVIEWED_SCROLLBACK_LINES`]). Lowering drops the oldest rows now;
    /// raising lets history grow again. Clamped to
    /// `1..=`[`EMULATOR_SCROLLBACK_LINES`]; a no-op when unchanged.
    ///
    /// A held PTY forwards the cap to its holder too (perf 01 N1): the
    /// holder's emulator is the copy future adoptions are rebuilt from, and
    /// it used to keep the full history of every unviewed session forever.
    pub fn set_history_cap(&self, lines: usize) {
        let lines = lines.clamp(1, EMULATOR_SCROLLBACK_LINES);
        let mut parser = lock_unpoisoned(&self.mirror.parser);
        if parser.screen().scrollback_len() != lines {
            parser.screen_mut().set_scrollback_len(lines);
        }
        drop(parser);
        if let Backend::Held(conn) = &self.backend {
            conn.set_history_cap(lines);
        }
    }

    /// Scrollback rows / cap / approximate cell heap of the emulator.
    pub fn emulator_stats(&self) -> EmulatorStats {
        let parser = lock_unpoisoned(&self.mirror.parser);
        let screen = parser.screen();
        let (rows, cells) = screen.scrollback_stats();
        EmulatorStats {
            scrollback_rows: rows,
            scrollback_cap: screen.scrollback_len(),
            scrollback_bytes: cells * std::mem::size_of::<vt100::Cell>(),
        }
    }

    /// Kill the child process and its process group: `SIGHUP` now (to the
    /// child's group and the terminal's foreground job), then — on a helper
    /// thread, only while the child is still alive — `SIGTERM` after
    /// [`KILL_GRACE`] and `SIGKILL` after another. A CLI that traps or ignores
    /// HUP therefore can't outlive `kill_session` / suspend / archive.
    ///
    /// Idempotent and pid-reuse safe: once the child has exited nothing is
    /// signalled (its pid may already belong to an unrelated process).
    ///
    /// A held PTY delegates to its holder, which is the child's parent and
    /// runs this same escalation locally (pid-reuse safe there too).
    pub fn kill(&self) -> Result<()> {
        let (master, killer) = match &self.backend {
            Backend::Local { master, killer } => (master, killer),
            Backend::Held(conn) => {
                if self.child_state.has_exited() {
                    return Ok(());
                }
                return conn
                    .kill()
                    .map_err(|e| Error::Internal(format!("pty kill (holder): {e}")));
            }
        };
        let Some(pid) = self.child_pid else {
            return lock_unpoisoned(killer)
                .kill()
                .map_err(|e| Error::Internal(format!("pty kill: {e}")));
        };
        if self.child_state.has_exited() {
            return Ok(());
        }
        // The foreground job (e.g. a command run from a shell session) may sit
        // in its own process group; capture it now, while the tty exists.
        #[cfg(unix)]
        let fg_pgrp = lock_unpoisoned(master).process_group_leader();
        #[cfg(not(unix))]
        let fg_pgrp: Option<i32> = None;
        if !self.child_state.signal(pid, fg_pgrp, SIGHUP) {
            return Ok(());
        }
        let state = Arc::clone(&self.child_state);
        std::thread::spawn(move || {
            for sig in [SIGTERM, SIGKILL] {
                if state.wait_exited(KILL_GRACE) || !state.signal(pid, fg_pgrp, sig) {
                    return;
                }
            }
        });
        Ok(())
    }

    /// True once the direct child has exited (it may not be reaped yet).
    pub fn has_exited(&self) -> bool {
        self.child_state.has_exited()
    }

    /// Subscribe to live output chunks.
    pub fn subscribe(&self) -> broadcast::Receiver<Bytes> {
        self.mirror.tx.subscribe()
    }

    /// Last `lines` lines of scrollback as raw bytes (legacy/raw history).
    pub fn scrollback(&self, lines: usize) -> Vec<u8> {
        lock_unpoisoned(&self.mirror.ring).tail(lines)
    }

    /// Search the scrollback ring for `query` (plain substring, case-insensitive).
    /// Returns up to `limit` `(line_index, plain_text)` pairs in buffer order.
    /// The scan runs after the ring lock is released (it would otherwise
    /// stall the PTY reader, which pushes into the ring for every chunk);
    /// async callers should use [`Self::search_lines`] + [`ring::search_lines`]
    /// on the blocking pool instead.
    pub fn search(&self, query: &str, limit: usize) -> Vec<(usize, String)> {
        ring::search_lines(&self.search_lines(), query, limit)
    }

    /// A consistent copy of the ring's lines (refcount bumps under a brief
    /// lock) for [`ring::search_lines`].
    pub fn search_lines(&self) -> Vec<Arc<Vec<u8>>> {
        lock_unpoisoned(&self.mirror.ring).lines()
    }

    /// A coherent snapshot of the CURRENT screen as escape sequences. Writing
    /// it to a fresh xterm reproduces exactly what a user attached now would
    /// see — including a full-screen TUI's input box — in one frame, with no
    /// replay flicker and no clipped bottom. Used on every (re)attach.
    pub fn screen_snapshot(&self) -> Vec<u8> {
        let parser = lock_unpoisoned(&self.mirror.parser);
        let screen = parser.screen();
        // Reset + home, then the formatted contents (incl. cursor + attrs).
        let mut out = b"\x1b[2J\x1b[H".to_vec();
        out.extend_from_slice(&screen.state_formatted());
        out
    }

    /// A snapshot that prepends up to `lines` rows of scrollback *history*
    /// (the rows that have scrolled off above the visible screen) before the
    /// coherent current-screen frame, so reconnecting doesn't lose history.
    ///
    /// History rows are emitted FORMATTED (colors/attributes preserved), at
    /// their full stored width, with soft-wrapped rows joined into logical
    /// lines the client re-wraps at its own width (reflowable on resize).
    /// Writing them scrolls them into the client xterm's scrollback; padding
    /// CRLFs then push the tail off the grid before `\x1b[2J\x1b[H` clears it
    /// (2J erases in place — without the padding the last screenful of
    /// history vanished). The live viewport is then drawn with full
    /// formatting. Visible rows are rendered exactly once — history holds
    /// only rows that scrolled off above the live screen, never the visible
    /// ones.
    ///
    /// `lines` is the cap on how many history rows to include (in addition to
    /// the current screen); it is further clamped to the emulator's retained
    /// history. `lines == 0` is equivalent to [`screen_snapshot`].
    ///
    /// [`screen_snapshot`]: Self::screen_snapshot
    /// Daemon-unique spawn counter for THIS process incarnation (see field doc).
    pub fn spawn_seq(&self) -> u64 {
        self.spawn_seq
    }

    ///
    /// The parser lock is held only to copy the emulator state; formatting
    /// runs after it is released (see [`ScreenCapture`]).
    pub fn snapshot_with_history(&self, lines: usize) -> Vec<u8> {
        self.capture().format(lines)
    }

    /// Copy the emulator state (brief lock). Format it with
    /// [`ScreenCapture::format`], ideally off the async workers.
    pub fn capture(&self) -> ScreenCapture {
        let parser = lock_unpoisoned(&self.mirror.parser);
        ScreenCapture {
            screen: parser.screen().clone(),
        }
    }

    /// Copy the emulator state and open a new output subscription under ONE
    /// lock hold. The reader thread parses and publishes under that same
    /// lock, so every chunk is either reflected in the capture or delivered
    /// to the receiver — never both, never neither.
    pub fn capture_and_subscribe(&self) -> (ScreenCapture, broadcast::Receiver<Bytes>) {
        let parser = lock_unpoisoned(&self.mirror.parser);
        let capture = ScreenCapture {
            screen: parser.screen().clone(),
        };
        (capture, self.mirror.tx.subscribe())
    }

    /// Atomically replace a viewer's backlog with emulator state and a new
    /// output subscription. Nothing is drained after unlocking: subsequent
    /// chunks are absent from the replay and must reach this receiver.
    /// Formats after releasing the lock ([`Self::capture_and_subscribe`]).
    pub fn snapshot_and_subscribe(&self, lines: usize) -> OutputSnapshot {
        let (capture, output) = self.capture_and_subscribe();
        let (cols, rows) = capture.size();
        OutputSnapshot {
            data: capture.format(lines),
            cols,
            rows,
            output,
        }
    }

    fn format_snapshot(screen: &vt100::Screen, lines: usize) -> Vec<u8> {
        if lines == 0 {
            let mut out = b"\x1b[2J\x1b[H".to_vec();
            out.extend_from_slice(&screen.state_formatted());
            return out;
        }

        // History first: formatted (colors/attributes preserved), each row at
        // its FULL stored width, soft-wrapped rows joined into one logical
        // line — the receiving terminal re-wraps at ITS current width and can
        // reflow on later resizes, instead of inheriting hard breaks from
        // whatever width this emulator had when the row scrolled off. See the
        // OTTO PATCH notes in vendor/vt100.
        let mut out = screen.scrollback_rows_formatted(lines);

        if !out.is_empty() {
            // Scroll the tail of the replayed history off the client's grid
            // BEFORE clearing: xterm's `CSI 2J` erases in place, so without
            // this the last screenful of history (the rows still on the grid
            // after replay) simply vanished — seen as "the content just above
            // the viewport is missing after reconnect". After the final CRLF
            // the cursor is at most `rows - 1` lines above the bottom, so
            // `rows - 1` CRLFs push every remaining history row into the
            // client's scrollback and never push a blank one.
            let (rows, _) = screen.size();
            for _ in 1..rows {
                out.extend_from_slice(b"\r\n");
            }
        }

        // Then the coherent current-screen frame on a clean grid.
        out.extend_from_slice(b"\x1b[2J\x1b[H");
        out.extend_from_slice(&screen.state_formatted());
        out
    }

    /// The CURRENT screen as plain text rows (no attributes, trailing blanks
    /// trimmed per row, no scrollback). Cheap: one grid walk under the parser
    /// lock. Used by the conversation view's live draft and by prompt probes.
    pub fn screen_rows(&self) -> Vec<String> {
        let parser = lock_unpoisoned(&self.mirror.parser);
        let screen = parser.screen();
        let (_, cols) = screen.size();
        screen
            .rows(0, cols)
            .map(|r| r.trim_end().to_string())
            .collect()
    }

    /// Current emulator size (rows, cols) — clients sync their xterm to this.
    pub fn screen_size(&self) -> (u16, u16) {
        lock_unpoisoned(&self.mirror.parser).screen().size()
    }

    /// Watch the child's exit: `None` while running, `Some(code)` after exit.
    pub fn on_exit(&self) -> watch::Receiver<Option<i32>> {
        self.exit_rx.clone()
    }

    /// Time since the PTY was spawned.
    pub fn uptime(&self) -> Duration {
        self.mirror.epoch.elapsed()
    }

    /// Instant of the most recent output chunk (spawn time when none yet).
    pub fn last_output_at(&self) -> Instant {
        self.mirror.epoch
            + Duration::from_millis(self.mirror.last_output_ms.load(Ordering::Relaxed))
    }
}

/// Daemon-unique spawn counter (see [`PtyHandle::spawn_seq`]).
fn next_spawn_seq() -> u64 {
    static SPAWN_SEQ: AtomicU64 = AtomicU64::new(1);
    SPAWN_SEQ.fetch_add(1, Ordering::Relaxed)
}

/// Writer thread: drains the bounded input queue in order. A failed write
/// (child gone → EIO) ends it; queued and later jobs then fail fast with
/// "input closed" instead of blocking. Two error kinds do NOT end it: a
/// revoked room authority (`PermissionDenied`) rejects only that job, and a
/// held PTY's transient loss of its holder connection (`ConnectionReset`)
/// fails only the in-flight job — the connection is re-established and later
/// input flows again.
fn spawn_writer(
    writer: Box<dyn std::io::Write + Send>,
    echo: Arc<echo::EchoClock>,
) -> SyncSender<WriteJob> {
    let (input_tx, input_rx) = sync_channel::<WriteJob>(INPUT_QUEUE_DEPTH);
    std::thread::spawn(move || {
        let mut writer = writer;
        while let Ok(job) = input_rx.recv() {
            let res = input_authority::write_authorized(
                writer.as_mut(),
                &job.data,
                job.authorization.as_ref(),
            );
            if res.is_ok() {
                echo.input_written();
            }
            let failed = res.as_ref().is_err_and(|e| {
                !matches!(
                    e.kind(),
                    std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::ConnectionReset
                )
            });
            match job.done {
                WriteDone::Blocking(tx) => {
                    let _ = tx.send(res);
                }
                WriteDone::Async(tx) => {
                    let _ = tx.send(res);
                }
            }
            if failed {
                break;
            }
        }
    });
    input_tx
}

fn input_closed() -> Error {
    Error::Internal("pty write: input closed (the process is gone)".into())
}

/// RAII: a [`PtyHandle`] owns its child process, so terminate it when the handle
/// is fully dropped (its last `Arc` released). Without this, dropping a handle —
/// e.g. evicting it from a tracking map, or overwriting it on respawn — would
/// leave the child running forever (the leak that orphaned resumed agent
/// processes). This is [`PtyHandle::kill`]: `SIGHUP` to the process group,
/// escalating to `SIGTERM`/`SIGKILL` while the child survives — and a no-op
/// once it has exited, which matters here because the last `Arc` (e.g. a
/// terminal tab left open) can outlive the child by hours, by which time its
/// pid may belong to an unrelated process. Callers that want a tracked,
/// observable shutdown still call [`PtyHandle::kill`] explicitly; this only
/// catches the paths that drop a handle without an explicit kill.
///
/// A held PTY follows the same rule, plus: unless it was [`PtyHandle::detach`]ed
/// (or the whole process is detaching, see [`holder::detach_all_on_drop`]),
/// the holder is told it is no longer needed and exits once the child is gone.
/// A detached handle just closes its connection — the child and its holder
/// keep running for the next daemon run to adopt.
impl Drop for PtyHandle {
    fn drop(&mut self) {
        match &self.backend {
            Backend::Local { .. } => {
                let _ = self.kill();
            }
            Backend::Held(conn) => {
                if !conn.is_detached() && !holder::detaching_all() {
                    let _ = self.kill();
                    conn.release();
                }
                conn.close();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backdate_moves_the_last_output_clock_not_the_feed() {
        let mut m = Mirror::new(80, 24, RingBuffer::default());
        m.backdate(Duration::from_secs(3600));
        let last = m.epoch + Duration::from_millis(m.last_output_ms.load(Ordering::Relaxed));
        assert!(last.elapsed() >= Duration::from_secs(3599));
        // Real output afterwards is "now" again.
        m.feed(b"x");
        let last = m.epoch + Duration::from_millis(m.last_output_ms.load(Ordering::Relaxed));
        assert!(last.elapsed() < Duration::from_secs(5));
    }

    /// Same-size resizes must be dropped before the ioctl: a shell trapping
    /// SIGWINCH sees NOTHING for repeats of the current grid and exactly one
    /// signal for a real change. (codex reprints its transcript per SIGWINCH —
    /// spurious ones from focus/reconnect re-pushes polluted scrollback.)
    /// On the daemon's multi-threaded runtime the reflow runs inside
    /// `block_in_place` (r3-06-02); the grid and PTY still change together.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn resize_on_multi_thread_runtime_reflows_in_place() {
        let spec = CommandSpec {
            program: "/bin/cat".into(),
            args: vec![],
            cwd: None,
            env: vec![],
        };
        let handle = Arc::new(PtyHandle::spawn_sized(&spec, 120, 40).expect("spawn"));
        let h = Arc::clone(&handle);
        tokio::spawn(async move { h.resize(90, 30).expect("resize") })
            .await
            .expect("resize task");
        assert_eq!(handle.size(), (90, 30));
        // Same size again: a no-op on every runtime flavour.
        handle.resize(90, 30).expect("same-size resize");
        assert_eq!(handle.size(), (90, 30));
    }

    #[tokio::test]
    async fn same_size_resize_sends_no_sigwinch() {
        let spec = CommandSpec {
            program: "/bin/sh".into(),
            args: vec![
                "-c".into(),
                "trap 'echo GOT_WINCH' WINCH; echo READY; while :; do sleep 1; done".into(),
            ],
            cwd: None,
            env: vec![],
        };
        let handle = PtyHandle::spawn_sized(&spec, 120, 40).expect("spawn");
        let wait_for = |needle: &'static str, h: &PtyHandle| {
            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline {
                if String::from_utf8_lossy(&h.scrollback(50)).contains(needle) {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            false
        };
        assert!(wait_for("READY", &handle), "shell never became ready");

        assert_eq!(handle.size(), (120, 40));
        // Re-push the exact same grid a few times (focus reclaim / reconnect).
        for _ in 0..3 {
            handle.resize(120, 40).expect("same-size resize");
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
        let out = String::from_utf8_lossy(&handle.scrollback(100)).to_string();
        assert!(
            !out.contains("GOT_WINCH"),
            "same-size resize must not SIGWINCH the child: {out}"
        );

        // A real change delivers the signal and updates the emulator grid.
        handle.resize(100, 30).expect("real resize");
        assert_eq!(handle.size(), (100, 30));
        assert!(
            wait_for("GOT_WINCH", &handle),
            "changed-size resize should SIGWINCH the child"
        );
    }

    /// True iff the OS still has a live (non-reaped) process for `pid`.
    fn pid_alive(pid: &str) -> bool {
        std::process::Command::new("/bin/kill")
            .args(["-0", pid])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// Dropping the handle SIGKILLs its child — the fix for the orphaned-process
    /// leak (a handle evicted/overwritten without an explicit `kill()` used to
    /// leave its child running forever).
    #[tokio::test]
    async fn drop_kills_the_child() {
        // `exec sleep` so the printed `$$` pid IS the long-lived process the
        // killer targets (no intermediate shell to leave behind).
        let spec = CommandSpec {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), "echo $$; exec sleep 30".into()],
            cwd: None,
            env: vec![],
        };
        let handle = PtyHandle::spawn(&spec).expect("spawn");

        // Read the pid the shell printed on the PTY.
        let mut pid = String::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            let out = String::from_utf8_lossy(&handle.scrollback(5)).to_string();
            if let Some(line) = out
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty() && l.bytes().all(|b| b.is_ascii_digit()))
            {
                pid = line.to_string();
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(!pid.is_empty(), "did not capture the child pid");
        assert!(pid_alive(&pid), "child should be alive before drop");

        drop(handle); // → Drop → SIGKILL; the waiter thread then reaps the zombie.

        let mut gone = false;
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if !pid_alive(&pid) {
                gone = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(
            gone,
            "child pid {pid} still alive after dropping the handle"
        );
    }

    /// Spawn `script` under /bin/sh and wait until it printed READY.
    fn spawn_ready(script: &str) -> PtyHandle {
        let spec = CommandSpec {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
            cwd: None,
            env: vec![],
        };
        let handle = PtyHandle::spawn(&spec).expect("spawn");
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if String::from_utf8_lossy(&handle.scrollback(20)).contains("READY") {
                return handle;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("child never printed READY");
    }

    async fn exits_within(handle: &PtyHandle, within: Duration) -> bool {
        let mut exit = handle.on_exit();
        let res = tokio::time::timeout(within, exit.wait_for(|v| v.is_some())).await;
        res.is_ok()
    }

    /// A CLI that ignores SIGHUP used to survive every kill path (suspend,
    /// archive, kill_session): kill must escalate to SIGTERM.
    #[tokio::test]
    async fn kill_escalates_past_an_ignored_sighup() {
        let handle = spawn_ready("trap '' HUP; echo READY; while :; do sleep 0.2; done");
        handle.kill().expect("kill");
        assert!(
            exits_within(&handle, KILL_GRACE + Duration::from_secs(3)).await,
            "HUP-ignoring child survived the SIGTERM escalation"
        );
    }

    /// …and to SIGKILL when TERM is ignored as well.
    #[tokio::test]
    async fn kill_escalates_to_sigkill() {
        let handle = spawn_ready("trap '' HUP TERM; echo READY; while :; do sleep 0.2; done");
        handle.kill().expect("kill");
        assert!(
            exits_within(&handle, KILL_GRACE * 2 + Duration::from_secs(3)).await,
            "HUP+TERM-ignoring child survived the SIGKILL escalation"
        );
    }

    /// Pid-reuse guard: once the child exited (and was reaped), kill must not
    /// signal anything — the pid may already belong to another process.
    #[tokio::test]
    async fn kill_after_exit_signals_nothing() {
        let spec = CommandSpec {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), "exit 0".into()],
            cwd: None,
            env: vec![],
        };
        let handle = PtyHandle::spawn(&spec).expect("spawn");
        assert!(exits_within(&handle, Duration::from_secs(10)).await);
        // The exit is published only after the child was marked exited.
        assert!(handle.has_exited());
        assert!(!handle
            .child_state
            .signal(handle.pid().expect("pid"), None, SIGHUP));
        handle.kill().expect("kill after exit is a no-op");
    }

    /// The async input path delivers in order alongside the blocking one.
    #[tokio::test]
    async fn write_async_delivers_in_order() {
        let handle = spawn_ready("echo READY; exec cat");
        handle
            .write_async(b"alpha-", Duration::from_secs(5))
            .await
            .expect("async write");
        handle.write(b"bravo-").expect("blocking write");
        handle
            .write_async(b"charlie\n", Duration::from_secs(5))
            .await
            .expect("async write");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if String::from_utf8_lossy(&handle.scrollback(20)).contains("alpha-bravo-charlie") {
                break;
            }
            assert!(Instant::now() < deadline, "input not echoed in order");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    #[tokio::test]
    async fn echo_output_lands_in_ring_and_exit_is_observed() {
        let spec = CommandSpec {
            program: "/bin/echo".into(),
            args: vec!["hello".into()],
            cwd: None,
            env: vec![],
        };
        let handle = PtyHandle::spawn(&spec).expect("spawn echo");

        let mut exit = handle.on_exit();
        let code = tokio::time::timeout(Duration::from_secs(10), async {
            let v = exit.wait_for(|v| v.is_some()).await.expect("exit watch");
            v.expect("code")
        })
        .await
        .expect("child exited in time");
        assert_eq!(code, 0);

        // Give the reader thread a moment to drain remaining buffered output.
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let out = handle.scrollback(100);
            if String::from_utf8_lossy(&out).contains("hello") {
                break;
            }
            assert!(Instant::now() < deadline, "no 'hello' in scrollback");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    #[tokio::test]
    async fn subscribe_receives_live_output() {
        // Delay output so the subscriber is attached before bytes flow.
        let spec = CommandSpec {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), "sleep 0.3; echo hello".into()],
            cwd: None,
            env: vec![],
        };
        let handle = PtyHandle::spawn(&spec).expect("spawn sh");
        let mut rx = handle.subscribe();

        let mut collected = Vec::new();
        let found = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                match rx.recv().await {
                    Ok(chunk) => {
                        collected.extend_from_slice(&chunk);
                        if String::from_utf8_lossy(&collected).contains("hello") {
                            return true;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return false,
                }
            }
        })
        .await
        .expect("output in time");
        assert!(found, "did not receive 'hello' via subscribe");
        assert!(handle.last_output_at() > handle.mirror.epoch);
    }

    #[tokio::test]
    async fn snapshot_with_history_keeps_offscreen_lines_without_duplicating_visible() {
        // Print far more than one 24-row screen so early lines scroll off into
        // the emulator's history. Each line is unique (LINE_0001..LINE_0080).
        let spec = CommandSpec {
            program: "/bin/sh".into(),
            args: vec![
                "-c".into(),
                "i=1; while [ $i -le 80 ]; do printf 'LINE_%04d\\n' $i; i=$((i+1)); done".into(),
            ],
            cwd: None,
            env: vec![],
        };
        let handle = PtyHandle::spawn(&spec).expect("spawn sh");

        // Wait for the child to finish emitting all lines.
        let mut exit = handle.on_exit();
        tokio::time::timeout(Duration::from_secs(10), async {
            exit.wait_for(|v| v.is_some()).await.expect("exit watch");
        })
        .await
        .expect("child exited in time");

        // Let the reader thread drain the last buffered output into the parser.
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let snap = String::from_utf8_lossy(&handle.snapshot_with_history(1000)).into_owned();
            if snap.contains("LINE_0080") {
                break;
            }
            assert!(Instant::now() < deadline, "LINE_0080 never appeared");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        let snap = String::from_utf8_lossy(&handle.snapshot_with_history(1000)).into_owned();

        // A line that scrolled off the top of a 24-row screen must survive in
        // the history-inclusive snapshot.
        assert!(
            snap.contains("LINE_0001"),
            "early off-screen line lost from history snapshot"
        );

        // The last line is in the visible viewport. It must appear exactly once
        // — the visible screen is rendered by the current-screen frame and must
        // NOT also be emitted as history (no double-render).
        let visible_count = snap.matches("LINE_0080").count();
        assert_eq!(
            visible_count, 1,
            "visible line was duplicated between history and the live screen"
        );

        // The bare current-screen snapshot must NOT contain the off-screen line:
        // confirms LINE_0001 is genuinely history, not part of the live screen.
        let screen_only = String::from_utf8_lossy(&handle.screen_snapshot()).into_owned();
        assert!(
            !screen_only.contains("LINE_0001"),
            "off-screen line unexpectedly present in the bare current screen"
        );

        // `lines == 0` is equivalent to the bare current-screen snapshot.
        assert_eq!(
            handle.snapshot_with_history(0),
            handle.screen_snapshot(),
            "lines == 0 must equal the bare screen snapshot"
        );
    }

    #[tokio::test]
    async fn snapshot_with_history_captures_scroll_region_history() {
        // Emulate how codex (ratatui `Terminal::insert_before`) emits finished
        // transcript lines: a TOP-ANCHORED scroll region (rows 1..5 of the
        // screen) is scrolled up by printing at its bottom row, pushing each
        // finished line off the top of the screen. Upstream vt100 discards
        // rows scrolled out while ANY region is active — which made reattach
        // snapshots lose the entire codex history. The vendored patch
        // (vendor/vt100/README-OTTO.md) keeps rows leaving the top of the
        // screen, matching what xterm.js shows live in the browser.
        let spec = CommandSpec {
            program: "/bin/sh".into(),
            args: vec![
                "-c".into(),
                // ESC[1;5r  → scroll region rows 1..5 (top-anchored)
                // ESC[5;1H  → cursor to region bottom
                // 30 lines  → 25+ of them scroll off the top of the screen
                // ESC[r     → reset region; DONE marks completion
                "printf '\\033[1;5r\\033[5;1H'; i=1; while [ $i -le 30 ]; do printf 'HIST_%04d\\n' $i; i=$((i+1)); done; printf '\\033[r\\033[24;1HDONE\\n'"
                    .into(),
            ],
            cwd: None,
            env: vec![],
        };
        let handle = PtyHandle::spawn(&spec).expect("spawn sh");

        let mut exit = handle.on_exit();
        tokio::time::timeout(Duration::from_secs(10), async {
            exit.wait_for(|v| v.is_some()).await.expect("exit watch");
        })
        .await
        .expect("child exited in time");

        // Let the reader thread drain the tail of the output into the parser.
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let snap = String::from_utf8_lossy(&handle.snapshot_with_history(1000)).into_owned();
            if snap.contains("DONE") {
                break;
            }
            assert!(Instant::now() < deadline, "DONE never appeared");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        let snap = String::from_utf8_lossy(&handle.snapshot_with_history(1000)).into_owned();
        // A line scrolled off the top of the screen through the region must
        // survive into the history-inclusive snapshot…
        assert!(
            snap.contains("HIST_0001"),
            "scroll-region history lost from snapshot (codex insert_before case)"
        );
        // …while the bare screen (region shows only the last 5 lines) must not
        // contain it — proving it was recovered from scrollback, not the grid.
        let screen_only = String::from_utf8_lossy(&handle.screen_snapshot()).into_owned();
        assert!(
            !screen_only.contains("HIST_0001"),
            "HIST_0001 unexpectedly still on the live screen; test premise broken"
        );
    }

    /// codex's SIGWINCH repaint: `ESC[r ESC[H ESC[2J ESC[3J` then a reprint of
    /// the ENTIRE transcript. `3J` (xterm "erase saved lines") must drop the
    /// emulator scrollback exactly like xterm.js does live — otherwise every
    /// resize banks one more full transcript copy into reattach snapshots
    /// (the "essay ×7/×20 after resizes" bug). OTTO PATCH 4 in vendor/vt100.
    #[tokio::test]
    async fn codex_3j_resize_repaint_keeps_single_transcript_copy() {
        let emit = "i=1; while [ $i -le 40 ]; do printf 'TSCPT_%04d\\n' $i; i=$((i+1)); done";
        let spec = CommandSpec {
            program: "/bin/sh".into(),
            args: vec![
                "-c".into(),
                // transcript → (codex repaint: full clear incl. scrollback →
                // reprint transcript) × 3 — as after three SIGWINCHes.
                format!(
                    "{emit}; n=1; while [ $n -le 3 ]; do printf '\\033[r\\033[H\\033[2J\\033[3J'; {emit}; n=$((n+1)); done; printf 'REPAINTS_DONE\\n'"
                ),
            ],
            cwd: None,
            env: vec![],
        };
        let handle = PtyHandle::spawn_sized(&spec, 80, 24).expect("spawn sh");
        let mut exit = handle.on_exit();
        tokio::time::timeout(Duration::from_secs(10), async {
            exit.wait_for(|v| v.is_some()).await.expect("exit watch");
        })
        .await
        .expect("child exited in time");

        let deadline = Instant::now() + Duration::from_secs(5);
        let snap = loop {
            let snap = String::from_utf8_lossy(&handle.snapshot_with_history(4000)).into_owned();
            if snap.contains("REPAINTS_DONE") {
                break snap;
            }
            assert!(Instant::now() < deadline, "REPAINTS_DONE never appeared");
            tokio::time::sleep(Duration::from_millis(50)).await;
        };
        let copies = snap.matches("TSCPT_0001").count();
        assert_eq!(
            copies, 1,
            "3J must wipe the previous transcript from scrollback; snapshot holds {copies} copies"
        );
        assert!(
            snap.contains("TSCPT_0040"),
            "latest transcript tail missing"
        );
    }

    /// A capture formats to exactly what the old under-lock snapshot produced,
    /// and the lock is held only for the copy (r3-06-02).
    #[test]
    fn capture_formats_like_the_locked_snapshot_and_holds_the_lock_briefly() {
        let mut parser = vt100::Parser::new(50, 160, EMULATOR_SCROLLBACK_LINES);
        // TUI-like rows: a colour/attribute change every 8 cells.
        let row: String = (0..19)
            .map(|k| {
                format!(
                    "\x1b[{};{}m{:<8}",
                    1 + (k % 2) * 21,
                    31 + (k % 7),
                    format!("seg{k:02}")
                )
            })
            .collect();
        for i in 0..(EMULATOR_SCROLLBACK_LINES + 200) {
            parser.process(format!("{i:05} {row}\x1b[0m\r\n").as_bytes());
        }
        let screen = parser.screen();
        let t0 = Instant::now();
        let direct = PtyHandle::format_snapshot(screen, EMULATOR_SCROLLBACK_LINES);
        let format_cost = t0.elapsed();
        let t1 = Instant::now();
        let capture = ScreenCapture {
            screen: screen.clone(),
        };
        let copy_cost = t1.elapsed();
        assert_eq!(capture.format(EMULATOR_SCROLLBACK_LINES), direct);
        assert_eq!(capture.size(), (160, 50));
        eprintln!(
            "snapshot of {} rows x 160: format {:?} (was under the lock), copy {:?} (now under the lock), {} bytes",
            EMULATOR_SCROLLBACK_LINES,
            format_cost,
            copy_cost,
            direct.len()
        );
    }

    /// OTTO PATCH 5 (perf 01 F2): scrollback rows shed trailing blanks, so a
    /// full 4000-row history of 20-char lines at 200 cols holds ~2.6 MB of
    /// cells instead of 25.6 MB — and the snapshot of it is unchanged.
    #[test]
    fn short_lines_at_wide_grid_keep_a_small_scrollback() {
        let mut parser = vt100::Parser::new(50, 200, EMULATOR_SCROLLBACK_LINES);
        for i in 0..(EMULATOR_SCROLLBACK_LINES + 100) {
            parser.process(format!("line {i:014}\r\n").as_bytes());
        }
        let (rows, cells) = parser.screen().scrollback_stats();
        let bytes = cells * std::mem::size_of::<vt100::Cell>();
        eprintln!("scrollback: {rows} rows, {cells} cells, {bytes} bytes");
        assert_eq!(rows, EMULATOR_SCROLLBACK_LINES);
        assert!(bytes < 3 * 1024 * 1024, "scrollback holds {bytes} bytes");
        let snap = String::from_utf8_lossy(&PtyHandle::format_snapshot(
            parser.screen(),
            EMULATOR_SCROLLBACK_LINES,
        ))
        .into_owned();
        assert!(snap.contains("line 00000000000100\x1b[0m\r\n"));
        assert!(snap.contains("line 00000000004099"));
    }

    /// A trimmed scrollback row pulled back onto a taller grid is full width
    /// again (cursor writes index cells by column), and a narrowing +
    /// widening reflow round-trips the text.
    #[test]
    fn trimmed_rows_survive_height_growth_and_reflow() {
        let mut parser = vt100::Parser::new(10, 120, EMULATOR_SCROLLBACK_LINES);
        for i in 0..60 {
            parser.process(format!("row-{i:03}\r\n").as_bytes());
        }
        parser.screen_mut().set_size(40, 120); // pulls 30 rows back
        parser.process(b"\x1b[1;100Hedge");
        assert!(parser.screen().contents().contains("edge"));
        parser.screen_mut().set_size(40, 30);
        parser.screen_mut().set_size(40, 160);
        let text = String::from_utf8_lossy(&PtyHandle::format_snapshot(parser.screen(), 4000))
            .into_owned();
        for i in [0, 29, 30, 59] {
            assert!(text.contains(&format!("row-{i:03}")), "row-{i:03} lost");
        }
    }

    /// The unviewed-session cap drops old rows and restoring it lets history
    /// grow again (daemon-core perf F9).
    #[test]
    fn history_cap_shrinks_and_regrows() {
        let handle = spawn_ready("echo READY; exec cat");
        handle.set_history_cap(UNVIEWED_SCROLLBACK_LINES);
        assert_eq!(
            handle.emulator_stats().scrollback_cap,
            UNVIEWED_SCROLLBACK_LINES
        );
        handle.set_history_cap(usize::MAX);
        assert_eq!(
            handle.emulator_stats().scrollback_cap,
            EMULATOR_SCROLLBACK_LINES
        );
        let mut parser = vt100::Parser::new(24, 80, EMULATOR_SCROLLBACK_LINES);
        for i in 0..3000 {
            parser.process(format!("{i}\r\n").as_bytes());
        }
        parser
            .screen_mut()
            .set_scrollback_len(UNVIEWED_SCROLLBACK_LINES);
        assert_eq!(
            parser.screen().scrollback_stats().0,
            UNVIEWED_SCROLLBACK_LINES
        );
        parser
            .screen_mut()
            .set_scrollback_len(EMULATOR_SCROLLBACK_LINES);
        for i in 0..500 {
            parser.process(format!("more {i}\r\n").as_bytes());
        }
        assert_eq!(
            parser.screen().scrollback_stats().0,
            UNVIEWED_SCROLLBACK_LINES + 500
        );
    }

    /// Daemon-side terminal budgets (perf 01 F7). Gated: `OTTO_PERF=1`
    /// (run with `--release` for the real budgets; debug builds get ×10).
    /// CI: the BLOCKING Rust job runs it in release with a ×3 scale (perf 01
    /// N4). Every timing is the best of several rounds, so one scheduler
    /// hiccup on a shared runner cannot fail a PR — a real regression moves
    /// every round.
    /// Full 4000 × 200 history of attribute-dense TUI rows:
    /// - capture (the parser-lock hold of every snapshot) < 2 ms,
    /// - format (off the lock, blocking pool) < 40 ms,
    /// - resize reflow of the whole history < 50 ms,
    /// - feed throughput ≥ 50 MB/s through the emulator + ring + broadcast
    ///   with 3 subscribers draining.
    #[test]
    fn perf_budgets_capture_format_reflow_feed() {
        if std::env::var("OTTO_PERF").ok().as_deref() != Some("1") {
            eprintln!("skipped: set OTTO_PERF=1 to enforce terminal perf budgets");
            return;
        }
        // Debug builds ×10; CI runners scale further with the same knob as
        // the Playwright perf gates (`OTTO_PERF_BUDGET_SCALE`, default 1).
        let scale: u32 = std::env::var("OTTO_PERF_BUDGET_SCALE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1)
            .max(1);
        let k: u32 = scale * if cfg!(debug_assertions) { 10 } else { 1 };
        let row: String = (0..24)
            .map(|j| format!("\x1b[{}m{:<8}", 31 + (j % 7), format!("seg{j:02}")))
            .collect();
        let mut parser = vt100::Parser::new(50, 200, EMULATOR_SCROLLBACK_LINES);
        for i in 0..(EMULATOR_SCROLLBACK_LINES + 100) {
            parser.process(format!("{i:05} {row}\x1b[0m\r\n").as_bytes());
        }
        const ROUNDS: usize = 5;
        let (mut capture_cost, mut format_cost, mut reflow_cost) =
            (Duration::MAX, Duration::MAX, Duration::MAX);
        let mut bytes = Vec::new();
        for round in 0..ROUNDS {
            let t = Instant::now();
            let capture = ScreenCapture {
                screen: parser.screen().clone(),
            };
            capture_cost = capture_cost.min(t.elapsed());
            let t = Instant::now();
            bytes = capture.format(EMULATOR_SCROLLBACK_LINES);
            format_cost = format_cost.min(t.elapsed());
            drop(capture);
            // Alternate narrow / wide: every round reflows the whole history.
            let cols = if round % 2 == 0 { 150 } else { 200 };
            let t = Instant::now();
            parser.screen_mut().set_size(50, cols);
            reflow_cost = reflow_cost.min(t.elapsed());
        }

        let m = Mirror::new(200, 50, RingBuffer::default());
        let mut subs: Vec<_> = (0..3).map(|_| m.tx.subscribe()).collect();
        let drained = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let drains: Vec<_> = subs
            .drain(..)
            .map(|mut rx| {
                let (drained, stop) = (Arc::clone(&drained), Arc::clone(&stop));
                std::thread::spawn(move || {
                    while !stop.load(Ordering::Relaxed) {
                        match rx.try_recv() {
                            Ok(b) => {
                                drained.fetch_add(b.len() as u64, Ordering::Relaxed);
                            }
                            Err(broadcast::error::TryRecvError::Lagged(_)) => {}
                            Err(_) => std::thread::yield_now(),
                        }
                    }
                })
            })
            .collect();
        let chunk: Vec<u8> = (0..8192)
            .map(|i| {
                if i % 120 == 119 {
                    b'\n'
                } else {
                    b'a' + (i % 26) as u8
                }
            })
            .collect();
        let total = 32 * 1024 * 1024;
        let mut feed_cost = Duration::MAX;
        for _ in 0..3 {
            let t = Instant::now();
            for _ in 0..(total / chunk.len()) {
                m.feed(&chunk);
            }
            feed_cost = feed_cost.min(t.elapsed());
        }
        stop.store(true, Ordering::Relaxed);
        for d in drains {
            d.join().unwrap();
        }
        let mbps = total as f64 / feed_cost.as_secs_f64() / 1e6;
        eprintln!(
            "terminal budgets: capture {capture_cost:?}, format {format_cost:?} ({} bytes), reflow {reflow_cost:?}, feed {mbps:.0} MB/s",
            bytes.len()
        );
        assert!(
            capture_cost < Duration::from_millis(2) * k,
            "capture {capture_cost:?}"
        );
        assert!(
            format_cost < Duration::from_millis(40) * k,
            "format {format_cost:?}"
        );
        assert!(
            reflow_cost < Duration::from_millis(50) * k,
            "reflow {reflow_cost:?}"
        );
        assert!(mbps * f64::from(k) >= 50.0, "feed {mbps:.0} MB/s");
    }

    /// Emulator update and broadcast publication are one atomic step under
    /// the parser lock: a snapshot either includes a chunk or its new
    /// receiver gets it — never both. The raw ring is filled AFTER that lock
    /// is released (perf 01 F3), so a ring held busy (a search copy, a slow
    /// push) must not block the snapshot either: hold the ring lock while the
    /// reader parses + publishes, then snapshot.
    #[test]
    fn snapshot_subscription_waits_for_output_publication() {
        let spec = CommandSpec {
            program: "/bin/cat".into(),
            args: vec![],
            cwd: None,
            env: vec![],
        };
        let handle = Arc::new(PtyHandle::spawn(&spec).unwrap());
        let ring = lock_unpoisoned(&handle.mirror.ring);
        handle.write(b"SNAPSHOT-BARRIER").unwrap();
        // The reader parses + publishes, releases the parser lock, then parks
        // on the ring lock we hold.
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let parsed = handle
                .mirror
                .parser
                .try_lock()
                .is_ok_and(|p| p.screen().contents().contains("SNAPSHOT-BARRIER"));
            if parsed {
                break;
            }
            assert!(Instant::now() < deadline, "reader never parsed the chunk");
            std::thread::sleep(Duration::from_millis(1));
        }
        let copy = handle.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let snapshot =
            std::thread::spawn(move || assert!(tx.send(copy.snapshot_and_subscribe(100)).is_ok()));
        // Not blocked by the ring.
        let mut replay = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("a busy ring must not block a snapshot");
        snapshot.join().unwrap();
        drop(ring);
        assert!(String::from_utf8_lossy(&replay.data).contains("SNAPSHOT-BARRIER"));
        assert!(matches!(
            replay.output.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
        handle.write(b"-NEXT").unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut received = Vec::new();
        loop {
            if let Ok(bytes) = replay.output.try_recv() {
                received.extend_from_slice(&bytes);
            }
            if String::from_utf8_lossy(&received).contains("-NEXT") {
                break;
            }
            assert!(Instant::now() < deadline, "post-snapshot output was lost");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(!String::from_utf8_lossy(&received).contains("SNAPSHOT-BARRIER"));
        handle.kill().unwrap();
    }

    /// Drive a PTY to completion and return the history-inclusive snapshot
    /// once `marker` shows up in it (the reader thread drains asynchronously).
    async fn settled_snapshot(spec: &CommandSpec, marker: &str) -> (PtyHandle, String) {
        let handle = PtyHandle::spawn(spec).expect("spawn sh");
        let mut exit = handle.on_exit();
        tokio::time::timeout(Duration::from_secs(10), async {
            exit.wait_for(|v| v.is_some()).await.expect("exit watch");
        })
        .await
        .expect("child exited in time");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let snap = String::from_utf8_lossy(&handle.snapshot_with_history(1000)).into_owned();
            if snap.contains(marker) {
                return (handle, snap);
            }
            assert!(Instant::now() < deadline, "{marker} never appeared");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    #[tokio::test]
    async fn history_replay_joins_soft_wrapped_rows_for_client_reflow() {
        // A 200-char line soft-wraps across three 80-col rows. Once it has
        // scrolled into history, the replay must emit it as ONE logical line
        // (no CR/LF between the segments) so the client terminal re-wraps it
        // at its own width — and can REFLOW it on a later resize — instead of
        // inheriting the emulator's historical hard breaks.
        let spec = CommandSpec {
            program: "/bin/sh".into(),
            args: vec![
                "-c".into(),
                "awk 'BEGIN{s=\"\";for(i=0;i<200;i++)s=s \"A\";print s}'; \
                 i=1; while [ $i -le 30 ]; do printf 'PAD_%04d\\n' $i; i=$((i+1)); done; echo DONE"
                    .into(),
            ],
            cwd: None,
            env: vec![],
        };
        let (_handle, snap) = settled_snapshot(&spec, "DONE").await;
        let run: String = std::iter::repeat_n('A', 200).collect();
        assert!(
            snap.contains(&run),
            "soft-wrapped history line was not joined into one contiguous logical line"
        );
    }

    // ---- vendored-vt100 reflow-on-resize (OTTO PATCH) unit tests ----------
    // Drive the emulator directly: these pin the reflow semantics every
    // terminal the user compares against (xterm.js, ghostty, iTerm) shares —
    // resize must re-wrap content, never truncate it.

    #[test]
    fn vt100_reflow_narrow_keeps_all_content() {
        let mut p = vt100::Parser::new(10, 80, 100);
        p.process(&[b"A".repeat(200), b"\r\n$ ".to_vec()].concat());
        p.screen_mut().set_size(10, 40);
        let mut text = String::new();
        for line in
            String::from_utf8_lossy(&p.screen().scrollback_rows_formatted(100)).split("\r\n")
        {
            text.push_str(line.trim_end_matches(['\u{1b}', '[', '0', 'm']));
        }
        text.push_str(&p.screen().contents());
        let count = text.chars().filter(|&c| c == 'A').count();
        assert_eq!(count, 200, "narrowing resize lost soft-wrapped content");
        // prompt survives at the cursor line
        assert!(
            p.screen().contents().contains("$ "),
            "prompt lost on narrow"
        );
    }

    #[test]
    fn vt100_reflow_widen_rejoins_wrapped_lines() {
        let mut p = vt100::Parser::new(10, 80, 100);
        p.process(&[b"B".repeat(200), b"\r\n$ ".to_vec()].concat());
        p.screen_mut().set_size(10, 120);
        // visual row 0 must now be FULL at the new width (120 B's), row 1
        // holds the remaining 80 — i.e. the wrapped rows re-joined and
        // re-split at 120, instead of keeping the old 80-col breaks.
        let row_b = |r: u16| {
            (0..120)
                .filter(|&c| {
                    p.screen()
                        .cell(r, c)
                        .is_some_and(|cell| cell.contents() == "B")
                })
                .count()
        };
        assert_eq!(row_b(0), 120, "row 0 not re-wrapped to the new width");
        assert_eq!(row_b(1), 80, "row 1 remainder wrong after widen");
        let contents = p.screen().contents();
        let count = contents.chars().filter(|&c| c == 'B').count();
        assert_eq!(count, 200, "widening resize lost content");
    }

    #[test]
    fn vt100_reflow_hard_lines_untouched() {
        let mut p = vt100::Parser::new(10, 80, 100);
        p.process(b"alpha\r\nbravo\r\ncharlie\r\n$ ");
        p.screen_mut().set_size(10, 40);
        let contents = p.screen().contents();
        for l in ["alpha", "bravo", "charlie", "$ "] {
            assert!(contents.contains(l), "hard line {l:?} damaged by reflow");
        }
        p.screen_mut().set_size(10, 100);
        let contents = p.screen().contents();
        for l in ["alpha", "bravo", "charlie"] {
            assert!(contents.contains(l), "hard line {l:?} damaged by widen");
        }
    }

    #[test]
    fn vt100_reflow_blank_lines_never_panic_snapshot() {
        // Blank transcript lines (claude prints them constantly) re-split to
        // empty cell runs during rewrap; a zero-width row in scrollback made
        // every subsequent snapshot panic on `cells[0]` — each WS replay
        // killed its connection handler (seen as a reconnect loop after the
        // first resize). Snapshot both directions and verify content.
        let mut p = vt100::Parser::new(10, 80, 4000);
        for i in 0..30 {
            p.process(
                format!("line {i} long enough to wrap once the pane narrows below it {i}\r\n\r\n")
                    .as_bytes(),
            );
        }
        p.screen_mut().set_size(10, 40);
        let snap = p.screen().scrollback_rows_formatted(4000);
        assert!(String::from_utf8_lossy(&snap).contains("line 0"));
        p.screen_mut().set_size(10, 120);
        let snap = p.screen().scrollback_rows_formatted(4000);
        assert!(String::from_utf8_lossy(&snap).contains("line 0"));
        // the newest line sits on the (bottom-anchored) visible grid
        assert!(p.screen().contents().contains("line 29"));
    }

    #[test]
    fn vt100_height_shrink_keeps_bottom_prompt() {
        let mut p = vt100::Parser::new(10, 80, 100);
        for i in 0..9 {
            p.process(format!("line{i}\r\n").as_bytes());
        }
        p.process(b"$ tail");
        p.screen_mut().set_size(5, 80);
        let contents = p.screen().contents();
        assert!(
            contents.contains("$ tail"),
            "height shrink must keep the bottom (prompt) — upstream truncated it: {contents:?}"
        );
        // pushed-off top rows live in scrollback, not the void
        let hist = String::from_utf8_lossy(&p.screen().scrollback_rows_formatted(100)).into_owned();
        assert!(
            hist.contains("line0"),
            "shrunk-off rows must enter scrollback"
        );
    }

    #[test]
    fn vt100_reflow_cursor_tracks_prompt() {
        let mut p = vt100::Parser::new(10, 80, 100);
        p.process(&[b"C".repeat(100), b"\r\n$ ".to_vec()].concat());
        let before = p.screen().cursor_position();
        assert_eq!(before.1, 2, "premise: cursor after '$ '");
        p.screen_mut().set_size(10, 40);
        let (r, c) = p.screen().cursor_position();
        // the prompt row must be the row the cursor is on, right after "$ "
        assert_eq!(c, 2, "cursor column lost through reflow");
        let cell = |col: u16| -> String {
            p.screen()
                .cell(r, col)
                .map(|x| x.contents().to_string())
                .unwrap_or_default()
        };
        assert_eq!(cell(0), "$", "cursor detached from prompt row");
        assert_eq!(cell(1), " ", "cursor detached from prompt row");
    }

    #[tokio::test]
    async fn history_replay_survives_narrowing_without_truncation() {
        // Emit a 60-char marker at the spawn width (80 cols), scroll it into
        // history, then NARROW the PTY to 40 cols. Scrollback rows keep their
        // captured width, and the replay must emit them untruncated — the old
        // per-row path clipped every history row to the CURRENT grid width,
        // literally cutting off replayed content after a narrowing resize.
        let marker: String = ('a'..='z').chain('A'..='Z').chain('0'..='7').collect();
        assert_eq!(marker.len(), 60);
        let spec = CommandSpec {
            program: "/bin/sh".into(),
            args: vec![
                "-c".into(),
                format!(
                    "echo {marker}; i=1; while [ $i -le 30 ]; do printf 'PAD_%04d\\n' $i; i=$((i+1)); done; echo DONE"
                ),
            ],
            cwd: None,
            env: vec![],
        };
        let (handle, _snap) = settled_snapshot(&spec, "DONE").await;
        handle.resize(40, 24).expect("resize narrower");
        let snap = String::from_utf8_lossy(&handle.snapshot_with_history(1000)).into_owned();
        assert!(
            snap.contains(&marker),
            "history row emitted at the narrowed width — replayed content truncated"
        );
    }
}
