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
    last_output_ms: Arc<AtomicU64>,
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
        // 1000 lines of scrollback history kept by the emulator. Initialise at
        // the requested grid size so the emulator agrees with the PTY from the
        // very first byte — avoids a spurious SIGWINCH on reconnect when the
        // client echoes back the same dimensions we already reported.
        let parser = Arc::new(Mutex::new(vt100::Parser::new(rows, cols, 1000)));
        let epoch = Instant::now();
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
            last_output_ms,
        })
    }

    /// Write bytes to the child's stdin.
    pub fn write(&self, data: &[u8]) -> Result<()> {
        let mut w = lock_unpoisoned(&self.writer);
        w.write_all(data)
            .and_then(|()| w.flush())
            .map_err(|e| Error::Internal(format!("pty write: {e}")))
    }

    /// Resize the terminal (PTY + the screen emulator).
    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        lock_unpoisoned(&self.parser)
            .screen_mut()
            .set_size(rows, cols);
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
    /// The history rows are emitted as plain text (one `\r\n`-terminated line
    /// each); writing them scrolls them up into the client xterm's own
    /// scrollback. The current screen is then drawn by [`screen_snapshot`],
    /// whose leading `\x1b[2J\x1b[H` clears the grid (NOT the client's
    /// scrollback) and redraws the live viewport with full formatting. The
    /// visible rows are therefore rendered exactly once — history holds only
    /// rows that scrolled off above the live screen, never the visible ones.
    ///
    /// `lines` is the cap on how many history rows to include (in addition to
    /// the current screen); it is further clamped to the emulator's retained
    /// history. `lines == 0` is equivalent to [`screen_snapshot`].
    ///
    /// [`screen_snapshot`]: Self::screen_snapshot
    pub fn snapshot_with_history(&self, lines: usize) -> Vec<u8> {
        let mut parser = lock_unpoisoned(&self.parser);
        if lines == 0 {
            let screen = parser.screen();
            let mut out = b"\x1b[2J\x1b[H".to_vec();
            out.extend_from_slice(&screen.state_formatted());
            return out;
        }

        let (_, cols) = parser.screen().size();
        let screen = parser.screen_mut();
        let saved_offset = screen.scrollback();

        // Probe the retained history depth: set_scrollback clamps to the
        // actual number of rows that have scrolled off, so reading it back
        // tells us how far up we can go.
        screen.set_scrollback(usize::MAX);
        let total_history = screen.scrollback();
        let take = lines.min(total_history);

        // Read history rows from oldest to newest. At scrollback offset `d`
        // the first visible row is exactly the row `d` positions above the
        // live screen's top, so iterating d = take..=1 yields the most recent
        // `take` history rows in display order.
        let mut out = Vec::new();
        for d in (1..=take).rev() {
            screen.set_scrollback(d);
            let line = screen.rows(0, cols).next().unwrap_or_default();
            out.extend_from_slice(line.as_bytes());
            out.extend_from_slice(b"\r\n");
        }

        // Restore the viewport, then append the coherent current-screen frame.
        screen.set_scrollback(saved_offset);
        out.extend_from_slice(b"\x1b[2J\x1b[H");
        out.extend_from_slice(&screen.state_formatted());
        out
    }

    /// Current emulator size (rows, cols) — clients sync their xterm to this.
    pub fn screen_size(&self) -> (u16, u16) {
        lock_unpoisoned(&self.parser).screen().size()
    }

    /// Watch the child's exit: `None` while running, `Some(code)` after exit.
    pub fn on_exit(&self) -> watch::Receiver<Option<i32>> {
        self.exit_rx.clone()
    }

    /// Instant of the most recent output chunk (spawn time when none yet).
    pub fn last_output_at(&self) -> Instant {
        self.epoch + Duration::from_millis(self.last_output_ms.load(Ordering::Relaxed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
