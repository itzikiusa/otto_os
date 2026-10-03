//! PTY holders — a PTY that outlives the daemon.
//!
//! A *holder* is a small detached process (`ottod pty-holder`, a re-exec of
//! the daemon binary) that owns ONE session's PTY master and child process,
//! keeps its own screen emulator (so it can repaint a fresh attach), and serves
//! a single client over a unix socket. The daemon spawns sessions through a
//! holder ([`crate::PtyHandle::spawn_held`]) instead of owning the PTY itself,
//! so restarting the daemon — a deploy (`launchctl kickstart`), a crash, an
//! app relaunch — no longer closes the PTY master (which used to SIGHUP every
//! shell and agent CLI). On startup the new daemon lists the holder sockets
//! ([`HolderConfig::sockets`]) and re-adopts each one
//! ([`crate::PtyHandle::adopt`]): it gets the same live process, its screen
//! and its scrollback, and streams on from there.
//!
//! ## Process model
//!
//! - The daemon launches `<launcher.program> <launcher.args…>` with
//!   [`HOLDER_ENV`]`=1`, in a NEW SESSION (`setsid`), so neither launchd's
//!   process-group kill of the daemon job nor a terminal hangup reaches it.
//!   The launch request ([`HolderSpawn`]: socket path, command, grid, opaque
//!   meta) travels on stdin — never argv, which `ps` shows to every user.
//! - The holder binds its socket (0600, in a 0700 directory), spawns the child
//!   in a fresh PTY, prints one [`READY_PREFIX`] line on stdout and detaches
//!   its stdio. The daemon reaps it if it exits while the daemon lives; after
//!   a daemon restart launchd (pid 1) does.
//! - One client at a time: a new connection replaces the previous one (a
//!   restarted daemon adopting it while the old one is still shutting down);
//!   the replaced client is told so ([`frame::SUPERSEDED`]) and lets go
//!   instead of fighting back.
//! - The holder exits once its child has exited AND either the client released
//!   it ([`frame::RELEASE`], sent by the daemon once it saw the exit, or when it
//!   deliberately closed the session) or no client came back within
//!   [`HolderConfig::exit_linger`]. A holder no daemon has attached to for
//!   [`HolderConfig::orphan_ttl`] kills its child — Otto uninstalled or stopped
//!   for good must not leave processes running forever — and one whose socket
//!   file disappeared (its directory was wiped) does the same at once: nobody
//!   can ever reach it again.
//!
//! ## Wire protocol (version handshake)
//!
//! Frames are `[u8 kind][u32 big-endian length][payload]`. The client opens
//! with [`frame::HELLO`] (`{proto_major, proto_minor, client}`); the holder
//! answers [`frame::HELLO_ACK`] with its [`HolderInfo`] (JSON, unknown fields
//! ignored) and, when the majors match, a [`frame::SNAPSHOT`] of its screen +
//! history followed by live [`frame::OUTPUT`]. Compatibility rules:
//!
//! - [`PROTO_MAJOR`] changes only for an incompatible frame layout. A daemon
//!   adopts only holders of its own major ([`AdoptError::Incompatible`]
//!   otherwise — the caller then ends that holder and falls back to resuming
//!   the session the old way).
//! - Minor bumps add frames/fields; both sides ignore frame kinds they do not
//!   know, and JSON payloads ignore unknown fields.
//! - FROZEN forever, whatever the major: the frame header, `HELLO`,
//!   `HELLO_ACK`, `KILL` and `RELEASE`. A daemon can therefore always tell an
//!   older or newer holder to end its child and exit ([`terminate`]).

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use otto_core::{Error, Result};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot, watch, Notify};

use crate::ring::RingBuffer;
use crate::{CommandSpec, PtyHandle, EMULATOR_SCROLLBACK_LINES};

/// Incompatible-layout version of the wire protocol (see the module docs).
pub const PROTO_MAJOR: u32 = 1;
/// Additive revision of the wire protocol.
pub const PROTO_MINOR: u32 = 2;
// 1.1: `HolderInfo::last_output_unix_ms` (additive; older holders send none).

/// Environment marker the launcher sets on the holder process. A holder entry
/// point refuses to run without it; it never reaches the session's child.
pub const HOLDER_ENV: &str = "OTTO_PTY_HOLDER";
/// Stdout line prefix a holder prints once its socket is bound and its child
/// is running (followed by a JSON object).
pub const READY_PREFIX: &str = "OTTO-PTY-HOLDER-READY ";
/// Stdout line prefix a holder prints when it could not start.
pub const ERROR_PREFIX: &str = "OTTO-PTY-HOLDER-ERROR ";

/// Default [`HolderConfig::orphan_ttl`]: a day without any daemon attached.
pub const DEFAULT_ORPHAN_TTL: Duration = Duration::from_secs(24 * 60 * 60);
/// Default [`HolderConfig::exit_linger`]: how long a holder whose child exited
/// keeps the exit code + final screen for a daemon that is not connected.
pub const DEFAULT_EXIT_LINGER: Duration = Duration::from_secs(10 * 60);

/// How long the launcher waits for a holder's READY line.
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(10);
/// Handshake (HELLO → HELLO_ACK → SNAPSHOT) and control-write timeout.
pub(crate) const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
/// `sun_path` is 104 bytes on macOS (108 on Linux), NUL included. Keep a
/// margin so a socket path never silently truncates.
const MAX_SOCKET_PATH: usize = 100;

/// Frame kinds and the codec. See the module docs for which are frozen.
pub(crate) mod frame {
    use std::io::{self, Read};

    use tokio::io::{AsyncRead, AsyncReadExt};

    /// client → holder, JSON [`super::Hello`]. FROZEN.
    pub const HELLO: u8 = 0x01;
    /// holder → client, JSON [`super::HolderInfo`]. FROZEN.
    pub const HELLO_ACK: u8 = 0x02;
    /// holder → client: `[u16 cols][u16 rows][screen + history bytes]`.
    pub const SNAPSHOT: u8 = 0x03;
    /// holder → client: raw PTY output.
    pub const OUTPUT: u8 = 0x04;
    /// holder → client, JSON `{code}`: the child exited.
    pub const EXITED: u8 = 0x05;
    /// holder → client: another client took over (a newer daemon adopted us);
    /// this connection is about to close. The old client must neither
    /// reconnect (that would kick the new owner back) nor report an exit.
    pub const SUPERSEDED: u8 = 0x06;
    /// client → holder: `[u64 seq][bytes]` to write to the PTY.
    pub const INPUT: u8 = 0x10;
    /// holder → client, JSON [`super::InputAck`].
    pub const INPUT_ACK: u8 = 0x11;
    /// client → holder: `[u16 cols][u16 rows]`.
    pub const RESIZE: u8 = 0x12;
    /// client → holder: `[u32 lines]` — the emulator's scrollback cap (minor
    /// 2, perf 01 N1). The daemon shrinks it for a long-unviewed session and
    /// restores it on attach, so the holder's copy of the history is bounded
    /// like the daemon's mirror. Older holders ignore it (unknown kind).
    pub const HISTORY_CAP: u8 = 0x13;
    /// client → holder: no longer needed — exit once the child is gone. FROZEN.
    pub const RELEASE: u8 = 0x7E;
    /// client → holder: end the child (HUP → TERM → KILL escalation). FROZEN.
    pub const KILL: u8 = 0x7F;

    /// Largest accepted payload (a 4000-row history snapshot at 500 columns
    /// formats to a few MiB).
    pub const MAX_FRAME: usize = 64 << 20;

    pub fn encode(kind: u8, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(5 + payload.len());
        out.push(kind);
        out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        out.extend_from_slice(payload);
        out
    }

    fn check_len(len: usize) -> io::Result<()> {
        if len > MAX_FRAME {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "pty holder frame too large",
            ));
        }
        Ok(())
    }

    pub fn read_sync(r: &mut impl Read) -> io::Result<(u8, Vec<u8>)> {
        let mut hdr = [0u8; 5];
        r.read_exact(&mut hdr)?;
        let len = u32::from_be_bytes([hdr[1], hdr[2], hdr[3], hdr[4]]) as usize;
        check_len(len)?;
        let mut buf = vec![0u8; len];
        r.read_exact(&mut buf)?;
        Ok((hdr[0], buf))
    }

    pub async fn read_async(r: &mut (impl AsyncRead + Unpin)) -> io::Result<(u8, Vec<u8>)> {
        let mut hdr = [0u8; 5];
        r.read_exact(&mut hdr).await?;
        let len = u32::from_be_bytes([hdr[1], hdr[2], hdr[3], hdr[4]]) as usize;
        check_len(len)?;
        let mut buf = vec![0u8; len];
        r.read_exact(&mut buf).await?;
        Ok((hdr[0], buf))
    }

    /// `[u16 cols][u16 rows]` prefix of SNAPSHOT / RESIZE payloads.
    pub fn grid(cols: u16, rows: u16) -> [u8; 4] {
        let c = cols.to_be_bytes();
        let r = rows.to_be_bytes();
        [c[0], c[1], r[0], r[1]]
    }

    pub fn parse_grid(p: &[u8]) -> Option<(u16, u16, &[u8])> {
        if p.len() < 4 {
            return None;
        }
        Some((
            u16::from_be_bytes([p[0], p[1]]),
            u16::from_be_bytes([p[2], p[3]]),
            &p[4..],
        ))
    }
}

/// The client's opening frame.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Hello {
    pub proto_major: u32,
    #[serde(default)]
    pub proto_minor: u32,
    #[serde(default)]
    pub client: String,
}

impl Hello {
    pub(crate) fn ours() -> Self {
        Self {
            proto_major: PROTO_MAJOR,
            proto_minor: PROTO_MINOR,
            client: format!("otto-pty {}", env!("CARGO_PKG_VERSION")),
        }
    }
}

/// What a holder says about itself on every attach ([`frame::HELLO_ACK`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HolderInfo {
    pub proto_major: u32,
    #[serde(default)]
    pub proto_minor: u32,
    /// The holder build (`otto-pty <version>`), for logs.
    #[serde(default)]
    pub holder_version: String,
    pub holder_pid: u32,
    /// The child's pid (the holder is its parent).
    #[serde(default)]
    pub child_pid: Option<u32>,
    /// Unix epoch milliseconds when the child was spawned.
    #[serde(default)]
    pub started_at_ms: u64,
    /// The holder emulator's current grid.
    pub cols: u16,
    pub rows: u16,
    /// `Some(code)` once the child has exited (the holder lingers so a
    /// reconnecting daemon can learn the exit; see the module docs).
    #[serde(default)]
    pub exited: Option<i32>,
    /// Opaque caller metadata given at spawn ([`crate::PtyHandle::spawn_held`]).
    #[serde(default)]
    pub meta: serde_json::Value,
    /// Unix epoch milliseconds of the child's most recent output (its spawn
    /// time when it has printed nothing). An adopting daemon back-dates its
    /// last-output clock from this, so a restart does not reset the session's
    /// idle clock. `0` = unknown (a protocol 1.0 holder).
    #[serde(default)]
    pub last_output_unix_ms: u64,
}

/// [`frame::INPUT_ACK`] payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct InputAck {
    pub seq: u64,
    #[serde(default)]
    pub error: Option<String>,
    /// The child is gone: no later input can succeed either.
    #[serde(default)]
    pub closed: bool,
}

/// The launch request a holder reads from stdin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HolderSpawn {
    pub socket: PathBuf,
    pub command: CommandSpec,
    pub cols: u16,
    pub rows: u16,
    #[serde(default)]
    pub meta: serde_json::Value,
    #[serde(default)]
    pub orphan_ttl_secs: Option<u64>,
    #[serde(default)]
    pub exit_linger_secs: Option<u64>,
}

/// How to start a holder process.
#[derive(Debug, Clone)]
pub struct HolderLauncher {
    pub program: PathBuf,
    pub args: Vec<String>,
}

impl HolderLauncher {
    pub fn new(program: impl Into<PathBuf>, args: Vec<String>) -> Self {
        Self {
            program: program.into(),
            args,
        }
    }

    /// Re-run THIS executable with `args` — the daemon's `ottod pty-holder`.
    pub fn current_exe(args: Vec<String>) -> std::io::Result<Self> {
        Ok(Self::new(std::env::current_exe()?, args))
    }
}

/// Where holders live and how they are started.
#[derive(Debug, Clone)]
pub struct HolderConfig {
    /// Private (0700) directory holding one `<random>.sock` per holder.
    pub dir: PathBuf,
    pub launcher: HolderLauncher,
    pub orphan_ttl: Duration,
    pub exit_linger: Duration,
}

impl HolderConfig {
    pub fn new(dir: impl Into<PathBuf>, launcher: HolderLauncher) -> Self {
        Self {
            dir: dir.into(),
            launcher,
            orphan_ttl: DEFAULT_ORPHAN_TTL,
            exit_linger: DEFAULT_EXIT_LINGER,
        }
    }

    /// The holder directory for a daemon data dir: `<data_dir>/pty-holders`,
    /// or — when that is too long for a unix socket path (`sun_path` is ~104
    /// bytes) — a per-data-dir directory under the per-user temp dir. `None`
    /// when neither fits (the caller then spawns PTYs locally).
    pub fn for_data_dir(data_dir: &Path, launcher: HolderLauncher) -> Option<Self> {
        let preferred = data_dir.join("pty-holders");
        if socket_path_fits(&preferred) {
            return Some(Self::new(preferred, launcher));
        }
        let fallback = std::env::temp_dir().join(format!("otto-ptyh-{:08x}", fnv32(data_dir)));
        socket_path_fits(&fallback).then(|| Self::new(fallback, launcher))
    }

    /// Create the directory (0700) and make sure it is ours and private —
    /// anyone who can connect to a holder socket can type into its session.
    pub fn ensure_dir(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        std::fs::set_permissions(&self.dir, std::fs::Permissions::from_mode(0o700))?;
        let meta = std::fs::metadata(&self.dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            // SAFETY: plain getter syscall.
            let uid = unsafe { libc::geteuid() };
            if meta.uid() != uid {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    format!("{} is not owned by this user", self.dir.display()),
                ));
            }
        }
        let _ = meta;
        Ok(())
    }

    /// A fresh, unique socket path for a new holder.
    pub fn new_socket_path(&self) -> PathBuf {
        self.dir.join(format!("{:016x}.sock", random_u64()))
    }

    /// Every holder socket currently in the directory (live or stale).
    pub fn sockets(&self) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut out: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "sock"))
            .collect();
        out.sort();
        out
    }
}

fn socket_path_fits(dir: &Path) -> bool {
    // "<16 hex>.sock" + the separator.
    dir.as_os_str().len() + 1 + 21 <= MAX_SOCKET_PATH
}

fn fnv32(p: &Path) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in p.as_os_str().as_encoded_bytes() {
        h ^= u32::from(*b);
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

fn random_u64() -> u64 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut h = RandomState::new().build_hasher();
    h.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
    h.write_u32(std::process::id());
    h.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    );
    h.finish()
}

fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ── process-wide detach ─────────────────────────────────────────────────────

static DETACH_ALL: AtomicBool = AtomicBool::new(false);

/// From now on, dropping a held [`PtyHandle`] never kills its child or
/// releases its holder (as if every handle were [`PtyHandle::detach`]ed). The
/// daemon calls this as it begins shutting down, so no teardown path — a
/// dropped map, an unwinding task — can end a session that is meant to survive
/// the restart.
pub fn detach_all_on_drop() {
    DETACH_ALL.store(true, Ordering::SeqCst);
}

pub(crate) fn detaching_all() -> bool {
    DETACH_ALL.load(Ordering::SeqCst)
}

// ── launcher (daemon side) ──────────────────────────────────────────────────

/// Start a holder for `spec` and attach to it (see [`PtyHandle::spawn_held`]).
pub(crate) fn spawn_and_attach(
    config: &HolderConfig,
    spec: &CommandSpec,
    cols: u16,
    rows: u16,
    meta: serde_json::Value,
) -> Result<PtyHandle> {
    config
        .ensure_dir()
        .map_err(|e| Error::Internal(format!("pty holder dir {}: {e}", config.dir.display())))?;
    let socket = config.new_socket_path();
    let request = HolderSpawn {
        socket: socket.clone(),
        command: spec.clone(),
        cols,
        rows,
        meta,
        orphan_ttl_secs: Some(config.orphan_ttl.as_secs()),
        exit_linger_secs: Some(config.exit_linger.as_secs()),
    };
    let payload = serde_json::to_vec(&request)
        .map_err(|e| Error::Internal(format!("pty holder request: {e}")))?;

    let mut cmd = std::process::Command::new(&config.launcher.program);
    cmd.args(&config.launcher.args)
        .env(HOLDER_ENV, "1")
        .current_dir("/")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: setsid is async-signal-safe; it runs in the forked child
        // before exec. A new session takes the holder out of the daemon's
        // process group (launchd kills that group when the job exits).
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    let mut child = cmd.spawn().map_err(|e| {
        Error::Internal(format!(
            "start pty holder {}: {e}",
            config.launcher.program.display()
        ))
    })?;
    let holder_pid = child.id();
    if let Some(mut stdin) = child.stdin.take() {
        if let Err(e) = stdin.write_all(&payload) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::Internal(format!("pty holder request: {e}")));
        }
    }
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(Error::Internal("pty holder: no stdout".into()));
    };

    // One thread per holder this daemon launched: read its verdict, then
    // reap it whenever it exits (after a daemon restart launchd does that).
    let (tx, rx) = std::sync::mpsc::channel::<std::result::Result<(), String>>();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        let mut verdict = Err("pty holder exited before it was ready".to_string());
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    // Matched anywhere in the line: a test harness may have
                    // printed an unterminated banner ("test x ... ") first.
                    if line.contains(READY_PREFIX) {
                        verdict = Ok(());
                        break;
                    }
                    if let Some(at) = line.find(ERROR_PREFIX) {
                        verdict = Err(line[at + ERROR_PREFIX.len()..].trim().to_string());
                        break;
                    }
                    // Anything else (a test harness banner) is noise.
                }
            }
        }
        let _ = tx.send(verdict);
        drop(reader);
        let _ = child.wait();
    });
    match rx.recv_timeout(LAUNCH_TIMEOUT) {
        Ok(Ok(())) => {}
        Ok(Err(msg)) => return Err(Error::Internal(format!("pty holder: {msg}"))),
        Err(_) => {
            // Still not ready, so not reaped either: its pid is still ours.
            #[cfg(unix)]
            // SAFETY: signalling our own unreaped child.
            unsafe {
                libc::kill(holder_pid as i32, libc::SIGKILL);
            }
            let _ = std::fs::remove_file(&socket);
            return Err(Error::Internal(format!(
                "pty holder did not start within {}s",
                LAUNCH_TIMEOUT.as_secs()
            )));
        }
    }
    match crate::held::adopt(&socket) {
        Ok(handle) => Ok(handle),
        Err(e) => {
            // Running but unreachable: end it rather than leak its child.
            let _ = terminate(&socket);
            Err(Error::Internal(format!("attach to new pty holder: {e}")))
        }
    }
}

/// Why [`PtyHandle::adopt`] failed.
#[derive(Debug)]
pub enum AdoptError {
    /// Nothing is listening on the socket any more (the holder is gone); the
    /// stale socket file was removed.
    Stale,
    /// A holder answered with a protocol major this build cannot drive. Only
    /// the frozen frames work with it: [`terminate`] it.
    Incompatible(Box<HolderInfo>),
    /// Anything else (I/O, a malformed handshake).
    Failed(String),
}

impl std::fmt::Display for AdoptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AdoptError::Stale => write!(f, "pty holder is gone (stale socket)"),
            AdoptError::Incompatible(info) => write!(
                f,
                "pty holder speaks protocol {}.{} ({}), this build {PROTO_MAJOR}.{PROTO_MINOR}",
                info.proto_major, info.proto_minor, info.holder_version
            ),
            AdoptError::Failed(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for AdoptError {}

/// Tell the holder at `socket` to end its child and exit, using only the
/// frozen frames — works with a holder of ANY protocol version. Best-effort.
pub fn terminate(socket: &Path) -> std::io::Result<()> {
    let mut s = std::os::unix::net::UnixStream::connect(socket)?;
    s.set_read_timeout(Some(HANDSHAKE_TIMEOUT))?;
    s.set_write_timeout(Some(HANDSHAKE_TIMEOUT))?;
    let hello = serde_json::to_vec(&Hello::ours()).unwrap_or_default();
    s.write_all(&frame::encode(frame::HELLO, &hello))?;
    // Wait for the ack so our KILL is processed by THIS connection's handler
    // (not dropped as pre-handshake noise).
    let _ = frame::read_sync(&mut s);
    s.write_all(&frame::encode(frame::KILL, &[]))?;
    s.write_all(&frame::encode(frame::RELEASE, &[]))?;
    // Give the holder a moment to read them before we hang up.
    let mut sink = [0u8; 4096];
    let deadline = Instant::now() + Duration::from_millis(300);
    let _ = s.set_read_timeout(Some(Duration::from_millis(100)));
    while Instant::now() < deadline {
        match s.read(&mut sink) {
            Ok(0) => break,
            _ => continue,
        }
    }
    Ok(())
}

// ── the holder process ──────────────────────────────────────────────────────

/// Entry point of a holder process (`ottod pty-holder`): read the
/// [`HolderSpawn`] request from stdin, start the child, serve until done.
/// Returns the process exit code. Refuses to run unless [`HOLDER_ENV`] is set
/// (it is not a command for humans).
pub fn run_from_stdin() -> i32 {
    if std::env::var_os(HOLDER_ENV).is_none() {
        eprintln!("pty-holder: internal command, started by the Otto daemon");
        return 2;
    }
    let mut raw = Vec::new();
    if let Err(e) = std::io::stdin().read_to_end(&mut raw) {
        return fail(None, &format!("read request: {e}"));
    }
    let request: HolderSpawn = match serde_json::from_slice(&raw) {
        Ok(r) => r,
        Err(e) => return fail(None, &format!("parse request: {e}")),
    };
    run(request)
}

/// Run a holder in-process for an explicit test-harness entry point: when the
/// current process was launched as a holder ([`HOLDER_ENV`] is set) this runs
/// it and EXITS the process; otherwise it returns immediately. Lets a test
/// binary serve as its own holder executable (see [`HolderLauncher`]).
pub fn run_if_holder_process() {
    if std::env::var_os(HOLDER_ENV).is_some() {
        std::process::exit(run_from_stdin());
    }
}

fn fail(socket: Option<&Path>, msg: &str) -> i32 {
    if let Some(s) = socket {
        let _ = std::fs::remove_file(s);
    }
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{ERROR_PREFIX}{}", msg.replace('\n', " "));
    let _ = out.flush();
    1
}

fn run(request: HolderSpawn) -> i32 {
    // Private by construction: the socket is created 0600.
    #[cfg(unix)]
    // SAFETY: plain syscall.
    unsafe {
        libc::umask(0o077);
    }
    let socket = request.socket.clone();
    if socket.exists() {
        let _ = std::fs::remove_file(&socket);
    }
    let listener = match std::os::unix::net::UnixListener::bind(&socket) {
        Ok(l) => l,
        Err(e) => return fail(None, &format!("bind {}: {e}", socket.display())),
    };
    let _ = std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600));

    // The holder never reads its raw ring (attach snapshots come from the
    // emulator; search is daemon-side), so it keeps none at all (N1).
    let handle = match PtyHandle::spawn_local(
        &request.command,
        request.cols,
        request.rows,
        RingBuffer::disabled(),
    ) {
        Ok(h) => Arc::new(h),
        Err(e) => return fail(Some(&socket), &e.to_string()),
    };
    let started_at_ms = unix_ms();
    {
        let ready = serde_json::json!({
            "holder_pid": std::process::id(),
            "child_pid": handle.pid(),
        });
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{READY_PREFIX}{ready}");
        let _ = out.flush();
    }
    detach_stdio();

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(_) => {
            let _ = handle.kill();
            let _ = std::fs::remove_file(&socket);
            return 1;
        }
    };
    let shared = Arc::new(Shared {
        handle: Arc::clone(&handle),
        socket: socket.clone(),
        holder_pid: std::process::id(),
        started_at_ms,
        meta: request.meta,
        released: AtomicBool::new(false),
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
    runtime.block_on(serve(listener, Arc::clone(&shared)));
    let _ = std::fs::remove_file(&socket);
    drop(runtime);
    0
}

/// Point stdin/stdout/stderr at /dev/null: the launcher's pipes close (it has
/// its READY line) and nothing later can block on them.
fn detach_stdio() {
    #[cfg(unix)]
    // SAFETY: plain fd syscalls on a freshly opened descriptor.
    unsafe {
        let fd = libc::open(c"/dev/null".as_ptr(), libc::O_RDWR);
        if fd >= 0 {
            libc::dup2(fd, 0);
            libc::dup2(fd, 1);
            libc::dup2(fd, 2);
            if fd > 2 {
                libc::close(fd);
            }
        }
    }
}

struct Shared {
    handle: Arc<PtyHandle>,
    /// Our own socket path: once it is gone (its directory was wiped — a test
    /// harness tearing down a throwaway data dir, an uninstall) no daemon can
    /// ever reach us again, so we end the child instead of waiting out the
    /// orphan TTL.
    socket: PathBuf,
    holder_pid: u32,
    started_at_ms: u64,
    meta: serde_json::Value,
    /// The client said it no longer needs us: exit once the child is gone.
    released: AtomicBool,
    gen: AtomicU64,
    /// Current client generation: a client whose generation is no longer
    /// current stops (it was replaced by a newer connection).
    kick: watch::Sender<u64>,
    active: Mutex<Option<u64>>,
    /// When the last client left (holder start until the first one comes).
    last_client: Mutex<Instant>,
    exited_at: Mutex<Option<Instant>>,
    wake: Notify,
    orphan_ttl: Duration,
    exit_linger: Duration,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Shared {
    fn exit_code(&self) -> Option<i32> {
        *self.handle.on_exit().borrow()
    }

    fn info(&self) -> HolderInfo {
        let (cols, rows) = self.handle.size();
        HolderInfo {
            proto_major: PROTO_MAJOR,
            proto_minor: PROTO_MINOR,
            holder_version: format!("otto-pty {}", env!("CARGO_PKG_VERSION")),
            holder_pid: self.holder_pid,
            child_pid: self.handle.pid(),
            started_at_ms: self.started_at_ms,
            cols,
            rows,
            exited: self.exit_code(),
            meta: self.meta.clone(),
            last_output_unix_ms: unix_ms()
                .saturating_sub(self.handle.last_output_at().elapsed().as_millis() as u64),
        }
    }

    /// Make a new connection THE client, replacing any previous one.
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

    /// The lifecycle rules from the module docs. True = exit now.
    fn should_exit(&self) -> bool {
        let connected = lock(&self.active).is_some();
        let reachable = self.socket.exists();
        if self.exit_code().is_some() {
            let exited_at = *lock(&self.exited_at).get_or_insert_with(Instant::now);
            if self.released.load(Ordering::SeqCst) || !reachable {
                return true;
            }
            return !connected && exited_at.elapsed() >= self.exit_linger;
        }
        if !reachable && !connected {
            // Unreachable for good: end the child; exit once it is gone.
            let _ = self.handle.kill();
            self.released.store(true, Ordering::SeqCst);
            return false;
        }
        if !connected && lock(&self.last_client).elapsed() >= self.orphan_ttl {
            // No daemon for a whole TTL: end the child; exit once it is gone.
            let _ = self.handle.kill();
            self.released.store(true, Ordering::SeqCst);
        }
        false
    }
}

async fn serve(listener: std::os::unix::net::UnixListener, sh: Arc<Shared>) {
    let _ = listener.set_nonblocking(true);
    let Ok(listener) = tokio::net::UnixListener::from_std(listener) else {
        let _ = sh.handle.kill();
        return;
    };
    let mut exit_rx = sh.handle.on_exit();
    let mut exit_seen = false;
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                if let Ok((stream, _)) = accepted {
                    tokio::spawn(serve_client(stream, Arc::clone(&sh)));
                }
            }
            _ = sh.wake.notified() => {}
            // Once: the sender is dropped right after publishing the code, and
            // a closed watch would resolve `changed()` in a busy loop.
            _ = wait_exit(&mut exit_rx), if !exit_seen => exit_seen = true,
            _ = tokio::time::sleep(Duration::from_millis(500)) => {}
        }
        if sh.should_exit() {
            return;
        }
    }
}

async fn write_frame(
    w: &mut (impl AsyncWrite + Unpin),
    kind: u8,
    payload: &[u8],
) -> std::io::Result<()> {
    w.write_all(&frame::encode(kind, payload)).await
}

async fn wait_exit(rx: &mut watch::Receiver<Option<i32>>) -> i32 {
    // Copy the code out first: the watch guard is not `Send` and must not
    // live across the `pending()` await below.
    let code = rx
        .wait_for(|v| v.is_some())
        .await
        .map(|v| (*v).unwrap_or(-1));
    match code {
        Ok(code) => code,
        Err(_) => std::future::pending().await,
    }
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
    if write_frame(&mut wr, frame::HELLO_ACK, &info).await.is_err() {
        sh.release_claim(gen);
        return;
    }
    if client_major != PROTO_MAJOR {
        // A client of another major: it can only use the frozen frames.
        frozen_only(rd, &sh).await;
        sh.release_claim(gen);
        return;
    }

    let snap = sh.handle.snapshot_and_subscribe(EMULATOR_SCROLLBACK_LINES);
    let mut payload = frame::grid(snap.cols, snap.rows).to_vec();
    payload.extend_from_slice(&snap.data);
    if write_frame(&mut wr, frame::SNAPSHOT, &payload)
        .await
        .is_err()
    {
        sh.release_claim(gen);
        return;
    }
    let mut out = snap.output;

    let (ack_tx, mut ack_rx) = mpsc::unbounded_channel::<InputAck>();
    let (in_tx, in_rx) = mpsc::unbounded_channel::<(u64, Vec<u8>)>();
    let input_task = tokio::spawn(input_loop(Arc::clone(&sh.handle), in_rx, ack_tx));
    let (eof_tx, mut eof_rx) = oneshot::channel::<()>();
    let reader_task = tokio::spawn(client_reader(rd, Arc::clone(&sh), in_tx, eof_tx));
    let mut exit_rx = sh.handle.on_exit();
    let mut exit_sent = false;

    loop {
        tokio::select! {
            changed = kick.changed() => {
                if changed.is_err() || *kick.borrow() != gen {
                    let _ = write_frame(&mut wr, frame::SUPERSEDED, &[]).await;
                    break;
                }
            }
            _ = &mut eof_rx => break,
            chunk = out.recv() => {
                match chunk {
                    Ok(bytes) => {
                        if write_frame(&mut wr, frame::OUTPUT, &bytes).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        // This client fell behind: replace the lost chunks
                        // with one fresh snapshot (it rebuilds from it).
                        let snap = sh.handle.snapshot_and_subscribe(EMULATOR_SCROLLBACK_LINES);
                        out = snap.output;
                        let mut payload = frame::grid(snap.cols, snap.rows).to_vec();
                        payload.extend_from_slice(&snap.data);
                        if write_frame(&mut wr, frame::SNAPSHOT, &payload).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            Some(ack) = ack_rx.recv() => {
                let payload = serde_json::to_vec(&ack).unwrap_or_default();
                if write_frame(&mut wr, frame::INPUT_ACK, &payload).await.is_err() {
                    break;
                }
            }
            code = wait_exit(&mut exit_rx), if !exit_sent => {
                // Final output first: wait (bounded — a grandchild may keep
                // the tty open) for the reader to drain, then forward what
                // the receiver still holds, then the exit itself.
                let mut done = sh.handle.output_closed();
                let _ = tokio::time::timeout(Duration::from_secs(1), done.wait_for(|d| *d)).await;
                let mut failed = false;
                loop {
                    match out.try_recv() {
                        Ok(bytes) => {
                            if write_frame(&mut wr, frame::OUTPUT, &bytes).await.is_err() {
                                failed = true;
                                break;
                            }
                        }
                        Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => continue,
                        Err(_) => break,
                    }
                }
                if failed {
                    break;
                }
                let payload = serde_json::json!({ "code": code }).to_string();
                if write_frame(&mut wr, frame::EXITED, payload.as_bytes()).await.is_err() {
                    break;
                }
                exit_sent = true;
                sh.wake.notify_one();
            }
        }
    }
    reader_task.abort();
    input_task.abort();
    let _ = wr.shutdown().await;
    sh.release_claim(gen);
}

/// Inbound frames of the current client.
async fn client_reader(
    mut rd: tokio::net::unix::OwnedReadHalf,
    sh: Arc<Shared>,
    in_tx: mpsc::UnboundedSender<(u64, Vec<u8>)>,
    eof_tx: oneshot::Sender<()>,
) {
    loop {
        match frame::read_async(&mut rd).await {
            Ok((frame::INPUT, payload)) if payload.len() >= 8 => {
                let mut seq = [0u8; 8];
                seq.copy_from_slice(&payload[..8]);
                if in_tx
                    .send((u64::from_be_bytes(seq), payload[8..].to_vec()))
                    .is_err()
                {
                    break;
                }
            }
            Ok((frame::RESIZE, payload)) => {
                if let Some((cols, rows, _)) = frame::parse_grid(&payload) {
                    let _ = sh.handle.resize(cols, rows);
                }
            }
            Ok((frame::HISTORY_CAP, payload)) if payload.len() >= 4 => {
                let lines = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
                sh.handle.set_history_cap(lines as usize);
            }
            // KILL / RELEASE; unknown kinds are ignored (forward compatibility).
            Ok((kind, _)) => control_frame(&sh, kind),
            Err(_) => break,
        }
    }
    let _ = eof_tx.send(());
}

/// Handle a frozen control frame (anything else is ignored).
fn control_frame(sh: &Shared, kind: u8) {
    match kind {
        frame::KILL => {
            let _ = sh.handle.kill();
        }
        frame::RELEASE => {
            sh.released.store(true, Ordering::SeqCst);
            sh.wake.notify_one();
        }
        _ => {}
    }
}

/// A client of a different protocol major: honour only KILL / RELEASE.
async fn frozen_only(mut rd: tokio::net::unix::OwnedReadHalf, sh: &Shared) {
    while let Ok((kind, _)) = frame::read_async(&mut rd).await {
        control_frame(sh, kind);
    }
}

/// Write the client's input to the PTY in order, acknowledging each job once
/// it reached the tty (the daemon-side writer thread waits for that, keeping
/// the old "delivered" semantics of a local write).
async fn input_loop(
    handle: Arc<PtyHandle>,
    mut rx: mpsc::UnboundedReceiver<(u64, Vec<u8>)>,
    acks: mpsc::UnboundedSender<InputAck>,
) {
    while let Some((seq, data)) = rx.recv().await {
        let res = handle
            .write_async(&data, Duration::from_secs(24 * 60 * 60))
            .await;
        let ack = InputAck {
            seq,
            error: res.err().map(|e| e.to_string()),
            closed: handle.has_exited(),
        };
        if acks.send(ack).is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_round_trip_and_size_guard() {
        let enc = frame::encode(frame::OUTPUT, b"hello");
        let (kind, payload) = frame::read_sync(&mut &enc[..]).unwrap();
        assert_eq!(kind, frame::OUTPUT);
        assert_eq!(payload, b"hello");
        let mut huge = vec![frame::OUTPUT];
        huge.extend_from_slice(&((frame::MAX_FRAME as u32) + 1).to_be_bytes());
        assert!(frame::read_sync(&mut &huge[..]).is_err());
        let (c, r, rest) = frame::parse_grid(&[0, 120, 0, 40, 9]).unwrap();
        assert_eq!((c, r, rest), (120, 40, &[9u8][..]));
        assert_eq!(frame::grid(120, 40), [0, 120, 0, 40]);
    }

    #[test]
    fn socket_dir_falls_back_when_the_data_dir_is_too_long() {
        let launcher = HolderLauncher::new("/bin/false", vec![]);
        let short = HolderConfig::for_data_dir(Path::new("/tmp/otto"), launcher.clone()).unwrap();
        assert_eq!(short.dir, Path::new("/tmp/otto/pty-holders"));
        let long = format!("/tmp/{}", "x".repeat(120));
        if let Some(cfg) = HolderConfig::for_data_dir(Path::new(&long), launcher) {
            assert!(cfg.dir.starts_with(std::env::temp_dir()));
            assert!(socket_path_fits(&cfg.dir));
        }
        let p = short.new_socket_path();
        assert!(p.as_os_str().len() <= MAX_SOCKET_PATH);
        assert_ne!(p, short.new_socket_path(), "socket names are unique");
    }

    /// Holder info stays readable by an older/newer peer: unknown fields are
    /// ignored and missing optional ones default (the version handshake).
    #[test]
    fn holder_info_is_forward_and_backward_compatible() {
        let newer = serde_json::json!({
            "proto_major": 1, "proto_minor": 7, "holder_pid": 42, "cols": 80, "rows": 24,
            "some_future_field": {"x": 1}
        });
        let info: HolderInfo = serde_json::from_value(newer).unwrap();
        assert_eq!(info.proto_major, 1);
        assert_eq!(info.child_pid, None);
        assert_eq!(info.exited, None);
        assert!(info.meta.is_null());
    }
}
