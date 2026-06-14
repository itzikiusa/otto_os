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
const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;
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
    /// Spawn `spec` in a fresh 80x24 PTY. A blocking reader thread pumps
    /// output into the scrollback ring and the broadcast channel; a waiter
    /// thread reaps the child and publishes the exit code.
    pub fn spawn(spec: &CommandSpec) -> Result<PtyHandle> {
        let pty = native_pty_system();
        let pair = pty
            .openpty(PtySize {
                rows: DEFAULT_ROWS,
                cols: DEFAULT_COLS,
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
        // 1000 lines of scrollback history kept by the emulator.
        let parser = Arc::new(Mutex::new(vt100::Parser::new(
            DEFAULT_ROWS,
            DEFAULT_COLS,
            1000,
        )));
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
}
