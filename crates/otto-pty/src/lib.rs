//! otto-pty — portable-pty wrapper: spawn, resize, ring-buffer scrollback,
//! broadcast of output chunks, kill, exit watch.

pub mod ring;

use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
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
    let c = if (MIN_COLS..=MAX_COLS).contains(&c) { c } else { DEFAULT_COLS };
    let r = if (MIN_ROWS..=MAX_ROWS).contains(&r) { r } else { DEFAULT_ROWS };
    (c, r)
}
/// Capacity of the output broadcast channel (chunks).
const BROADCAST_CAPACITY: usize = 1024;

/// Scrollback rows retained by the vt100 emulator — the replay depth clients
/// rebuild from on reconnect. See the comment at the `Parser` construction in
/// [`PtyHandle::spawn_sized`] for the memory tradeoff.
pub const EMULATOR_SCROLLBACK_LINES: usize = 4000;

/// A fully-resolved command to run inside a PTY.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub env: Vec<(String, String)>,
}

/// A live PTY child process: write input, watch output, observe exit.
pub struct PtyHandle {
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    ring: Arc<Mutex<RingBuffer>>,
    /// Headless terminal emulator tracking the CURRENT screen, so a fresh
    /// attach can reproduce the live screen in one coherent frame (no replay
    /// flicker, no clipped TUI). This is what tmux does.
    parser: Arc<Mutex<vt100::Parser>>,
    tx: broadcast::Sender<Bytes>,
    exit_rx: watch::Receiver<Option<i32>>,
    /// Instant the handle was created; `last_output_ms` is relative to it.
    epoch: Instant,
    /// Daemon-unique spawn counter. A client that sees this change between
    /// attaches knows the process was respawned (resume/restart) and its local
    /// scrollback belongs to a dead process — rebuild from the snapshot instead
    /// of appending to stale content.
    spawn_seq: u64,
    last_output_ms: Arc<AtomicU64>,
    /// OS pid of the direct child, captured at spawn (None if the OS didn't
    /// report one). Used by the idle-suspend sweep to check for live descendant
    /// processes before killing a "quiet" session.
    child_pid: Option<u32>,
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

        let (tx, _) = broadcast::channel::<Bytes>(BROADCAST_CAPACITY);
        let (exit_tx, exit_rx) = watch::channel::<Option<i32>>(None);
        let ring = Arc::new(Mutex::new(RingBuffer::default()));
        // Scrollback history kept by the emulator — this is the depth a client
        // gets back on every reconnect rebuild, so it IS the user-visible
        // scrollback for reopened sessions. Rows cost cols×32 bytes, so the cap
        // trades replay depth against per-live-session memory (~25 MiB worst
        // case at 200 cols when a long session fills it). Initialise at the
        // requested grid size so the emulator agrees with the PTY from the
        // very first byte — avoids a spurious SIGWINCH on reconnect when the
        // client echoes back the same dimensions we already reported.
        let parser = Arc::new(Mutex::new(vt100::Parser::new(rows, cols, EMULATOR_SCROLLBACK_LINES)));
        let epoch = Instant::now();
        static SPAWN_SEQ: AtomicU64 = AtomicU64::new(1);
        let spawn_seq = SPAWN_SEQ.fetch_add(1, Ordering::Relaxed);
        let last_output_ms = Arc::new(AtomicU64::new(0));

        // Blocking reader thread: PTY output -> screen emulator + ring + broadcast.
        {
            let tx = tx.clone();
            let ring = Arc::clone(&ring);
            let parser = Arc::clone(&parser);
            let last = Arc::clone(&last_output_ms);
            std::thread::spawn(move || {
                let mut buf = [0u8; 8192];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            last.store(epoch.elapsed().as_millis() as u64, Ordering::Relaxed);
                            lock_unpoisoned(&parser).process(&buf[..n]);
                            lock_unpoisoned(&ring).push(&buf[..n]);
                            // No receivers is fine — the screen state still records.
                            let _ = tx.send(Bytes::copy_from_slice(&buf[..n]));
                        }
                    }
                }
            });
        }

        // Capture the child's OS pid before the waiter thread takes ownership.
        let child_pid = child.process_id();

        // Waiter thread: reap the child and publish the exit code.
        std::thread::spawn(move || {
            let code = match child.wait() {
                Ok(status) => status.exit_code() as i32,
                Err(_) => -1,
            };
            let _ = exit_tx.send(Some(code));
        });

        Ok(PtyHandle {
            master: Mutex::new(pair.master),
            writer: Mutex::new(writer),
            killer: Mutex::new(killer),
            ring,
            parser,
            tx,
            exit_rx,
            epoch,
            spawn_seq,
            last_output_ms,
            child_pid,
        })
    }

    /// OS pid of the direct child process (None when the OS didn't report one).
    pub fn pid(&self) -> Option<u32> {
        self.child_pid
    }

    /// Write bytes to the child's stdin.
    pub fn write(&self, data: &[u8]) -> Result<()> {
        let mut w = lock_unpoisoned(&self.writer);
        w.write_all(data)
            .and_then(|()| w.flush())
            .map_err(|e| Error::Internal(format!("pty write: {e}")))
    }

    /// Current emulator grid as `(cols, rows)`.
    pub fn size(&self) -> (u16, u16) {
        let parser = lock_unpoisoned(&self.parser);
        let (rows, cols) = parser.screen().size();
        (cols, rows)
    }

    /// Resize the terminal (PTY + the screen emulator).
    ///
    /// Same-size calls are dropped entirely — no emulator rewrap and no
    /// TIOCSWINSZ — so clients can re-push their grid unconditionally on
    /// focus/reconnect without triggering a TUI repaint (codex re-emits its
    /// transcript on every SIGWINCH; each spurious one pollutes scrollback).
    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        let mut parser = lock_unpoisoned(&self.parser);
        if parser.screen().size() == (rows, cols) {
            return Ok(());
        }
        parser.screen_mut().set_size(rows, cols);
        drop(parser);
        lock_unpoisoned(&self.master)
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| Error::Internal(format!("pty resize: {e}")))
    }

    /// Kill the child process.
    pub fn kill(&self) -> Result<()> {
        lock_unpoisoned(&self.killer)
            .kill()
            .map_err(|e| Error::Internal(format!("pty kill: {e}")))
    }

    /// Subscribe to live output chunks.
    pub fn subscribe(&self) -> broadcast::Receiver<Bytes> {
        self.tx.subscribe()
    }

    /// Last `lines` lines of scrollback as raw bytes (legacy/raw history).
    pub fn scrollback(&self, lines: usize) -> Vec<u8> {
        lock_unpoisoned(&self.ring).tail(lines)
    }

    /// Search the scrollback ring for `query` (plain substring, case-insensitive).
    /// Returns up to `limit` `(line_index, plain_text)` pairs in buffer order.
    pub fn search(&self, query: &str, limit: usize) -> Vec<(usize, String)> {
        lock_unpoisoned(&self.ring).search(query, limit)
    }

    /// A coherent snapshot of the CURRENT screen as escape sequences. Writing
    /// it to a fresh xterm reproduces exactly what a user attached now would
    /// see — including a full-screen TUI's input box — in one frame, with no
    /// replay flicker and no clipped bottom. Used on every (re)attach.
    pub fn screen_snapshot(&self) -> Vec<u8> {
        let parser = lock_unpoisoned(&self.parser);
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

    pub fn snapshot_with_history(&self, lines: usize) -> Vec<u8> {
        let parser = lock_unpoisoned(&self.parser);
        let screen = parser.screen();
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
        let parser = lock_unpoisoned(&self.parser);
        let screen = parser.screen();
        let (_, cols) = screen.size();
        screen
            .rows(0, cols)
            .map(|r| r.trim_end().to_string())
            .collect()
    }

    /// Current emulator size (rows, cols) — clients sync their xterm to this.
    pub fn screen_size(&self) -> (u16, u16) {
        lock_unpoisoned(&self.parser).screen().size()
    }

    /// Watch the child's exit: `None` while running, `Some(code)` after exit.
    pub fn on_exit(&self) -> watch::Receiver<Option<i32>> {
        self.exit_rx.clone()
    }

    /// Time since the PTY was spawned.
    pub fn uptime(&self) -> Duration {
        self.epoch.elapsed()
    }

    /// Instant of the most recent output chunk (spawn time when none yet).
    pub fn last_output_at(&self) -> Instant {
        self.epoch + Duration::from_millis(self.last_output_ms.load(Ordering::Relaxed))
    }
}

/// RAII: a [`PtyHandle`] owns its child process, so terminate it when the handle
/// is fully dropped (its last `Arc` released). Without this, dropping a handle —
/// e.g. evicting it from a tracking map, or overwriting it on respawn — would
/// leave the child running forever (the leak that orphaned resumed agent
/// processes). The `SIGKILL` is best-effort and idempotent: on an already-exited
/// child the killer simply errors and we ignore it. Callers that want a tracked,
/// observable shutdown still call [`PtyHandle::kill`] explicitly; this only
/// catches the paths that drop a handle without an explicit kill.
impl Drop for PtyHandle {
    fn drop(&mut self) {
        let _ = lock_unpoisoned(&self.killer).kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Same-size resizes must be dropped before the ioctl: a shell trapping
    /// SIGWINCH sees NOTHING for repeats of the current grid and exactly one
    /// signal for a real change. (codex reprints its transcript per SIGWINCH —
    /// spurious ones from focus/reconnect re-pushes polluted scrollback.)
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
        assert!(gone, "child pid {pid} still alive after dropping the handle");
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
        assert!(handle.last_output_at() > handle.epoch);
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
        assert!(snap.contains("TSCPT_0040"), "latest transcript tail missing");
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
        for line in String::from_utf8_lossy(&p.screen().scrollback_rows_formatted(100))
            .split("\r\n")
        {
            text.push_str(line.trim_end_matches(['\u{1b}', '[', '0', 'm']));
        }
        text.push_str(&p.screen().contents());
        let count = text.chars().filter(|&c| c == 'A').count();
        assert_eq!(count, 200, "narrowing resize lost soft-wrapped content");
        // prompt survives at the cursor line
        assert!(p.screen().contents().contains("$ "), "prompt lost on narrow");
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
        assert!(hist.contains("line0"), "shrunk-off rows must enter scrollback");
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
