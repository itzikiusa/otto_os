//! Managed `lightpanda serve` sidecar: spawn the headless-browser CDP
//! server, poll it healthy, keep it alive with a restart-with-backoff
//! supervisor, and hand back the CDP WebSocket URL. Mirrors the ClickHouse
//! sidecar pattern in `otto-usage::clickhouse` (`ClickHouse::locate`/`start`).
//!
//! Install: `brew install lightpanda-io/tap/lightpanda`, or download a
//! release from <https://github.com/lightpanda-io/browser>.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use tokio::process::{Child, Command};
use tokio::sync::Mutex as AsyncMutex;
use tokio::task::JoinHandle;

use crate::engine::EngineError;

/// Handle to a running `lightpanda serve` sidecar. Dropping it (or calling
/// [`Lightpanda::shutdown`]) stops the supervisor and kills the child.
pub struct Lightpanda {
    port: u16,
    child: Arc<AsyncMutex<Option<Child>>>,
    shutting_down: Arc<AtomicBool>,
    supervisor: StdMutex<Option<JoinHandle<()>>>,
    /// Periodic copy-truncate of `lightpanda.log` (see [`spawn_log_rotator`]).
    log_rotator: StdMutex<Option<JoinHandle<()>>>,
}

impl Lightpanda {
    /// Resolve the `lightpanda` binary in priority order: an explicit
    /// configured path, then `PATH`, then well-known install locations
    /// (including the slot Otto would auto-download a binary into).
    ///
    /// An explicit `configured` path is authoritative: if it's set but does
    /// not point at a real file, `locate` returns `None` rather than
    /// silently falling through to autodetection — a misconfigured path
    /// should surface as "not found", not paper over the mistake with
    /// whatever happens to be on `PATH`.
    pub fn locate(configured: Option<&str>) -> Option<PathBuf> {
        if let Some(p) = configured.map(str::trim).filter(|s| !s.is_empty()) {
            let pb = PathBuf::from(p);
            return pb.is_file().then_some(pb);
        }
        if let Ok(out) = std::process::Command::new("which")
            .arg("lightpanda")
            .output()
        {
            if out.status.success() {
                let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !s.is_empty() && Path::new(&s).is_file() {
                    return Some(PathBuf::from(s));
                }
            }
        }
        let mut candidates = vec![
            PathBuf::from("/usr/local/bin/lightpanda"),
            PathBuf::from("/opt/homebrew/bin/lightpanda"),
        ];
        if let Some(home) = dirs::home_dir() {
            candidates.push(home.join(".local/bin/lightpanda"));
            // Slot Otto would place an auto-downloaded binary into.
            candidates.push(home.join("Library/Application Support/Otto/bin/lightpanda"));
        }
        candidates.into_iter().find(|p| p.is_file())
    }

    /// Spawn `lightpanda serve` bound to a free loopback port, poll it
    /// healthy, and start the restart-with-backoff supervisor. `data_dir`
    /// holds the sidecar's log file.
    pub async fn start(bin: PathBuf, data_dir: PathBuf) -> Result<Self, EngineError> {
        std::fs::create_dir_all(&data_dir)
            .map_err(|e| EngineError::Unavailable(format!("create lightpanda data dir: {e}")))?;

        let port = free_loopback_port()
            .map_err(|e| EngineError::Unavailable(format!("pick port: {e}")))?;
        let first_child = spawn_lightpanda(&bin, &data_dir, port)?;
        let child = Arc::new(AsyncMutex::new(Some(first_child)));

        if !wait_ready(port, Duration::from_secs(20)).await {
            if let Some(mut c) = child.lock().await.take() {
                let _ = c.start_kill();
            }
            return Err(EngineError::Unavailable(format!(
                "lightpanda did not open port {port} in time"
            )));
        }

        let shutting_down = Arc::new(AtomicBool::new(false));
        let log_rotator = spawn_log_rotator(data_dir.join("lightpanda.log"), LOG_CAP_BYTES);
        let supervisor =
            spawn_supervisor(bin, data_dir, port, child.clone(), shutting_down.clone());

        tracing::info!("browser: lightpanda sidecar ready on port {port}");
        Ok(Self {
            port,
            child,
            shutting_down,
            supervisor: StdMutex::new(Some(supervisor)),
            log_rotator: StdMutex::new(Some(log_rotator)),
        })
    }

    /// The CDP WebSocket URL to connect a [`crate::LightpandaEngine`] to.
    pub fn cdp_url(&self) -> String {
        format!("ws://127.0.0.1:{}", self.port)
    }

    /// Stop the supervisor and the sidecar: SIGTERM, bounded wait, SIGKILL
    /// fallback.
    pub async fn shutdown(&self) {
        self.shutting_down.store(true, Ordering::SeqCst);
        for task in [&self.supervisor, &self.log_rotator] {
            if let Ok(mut g) = task.lock() {
                if let Some(h) = g.take() {
                    h.abort();
                }
            }
        }
        let mut guard = self.child.lock().await;
        let Some(mut child) = guard.take() else {
            return;
        };
        if let Some(pid) = child.id() {
            let _ = Command::new("kill")
                .arg("-TERM")
                .arg(pid.to_string())
                .output()
                .await;
        }
        match tokio::time::timeout(Duration::from_secs(5), child.wait()).await {
            Ok(_) => {}
            Err(_) => {
                let _ = child.start_kill();
                let _ = child.wait().await;
            }
        }
    }
}

impl Drop for Lightpanda {
    fn drop(&mut self) {
        // Safety net if `shutdown()` wasn't called (e.g. a panic). `kill_on_drop`
        // already arms SIGKILL on the child; this makes it explicit and stops
        // the supervisor from trying to respawn a sidecar nobody wants anymore.
        self.shutting_down.store(true, Ordering::SeqCst);
        for task in [&self.supervisor, &self.log_rotator] {
            if let Ok(mut g) = task.lock() {
                if let Some(h) = g.take() {
                    h.abort();
                }
            }
        }
        if let Ok(mut g) = self.child.try_lock() {
            if let Some(mut c) = g.take() {
                let _ = c.start_kill();
            }
        }
    }
}

/// `lightpanda.log` past this size is rotated to `lightpanda.log.1` on the
/// next (re)spawn, so appending across restarts stays bounded — and, while the
/// sidecar runs, by a copy-truncate every [`LOG_CHECK_EVERY`].
const LOG_CAP_BYTES: u64 = 4 * 1024 * 1024;
/// How often a running sidecar's log is checked against [`LOG_CAP_BYTES`]. It
/// lives as long as the daemon (days) and logs per connection and request, so
/// a check only at spawn left that growth unbounded.
const LOG_CHECK_EVERY: Duration = Duration::from_secs(600);

/// Copy-truncate `log_path` once it passes `cap`: its contents replace
/// `lightpanda.log.1` and the live file is cut to zero IN PLACE. The child
/// holds `O_APPEND` fds, so its next write lands at the new end (a rename
/// would leave it writing to the unlinked old file forever). Lines written
/// between the copy and the truncate can be lost — a bounded log is worth
/// that. Returns whether it rotated.
fn copy_truncate_if_over(log_path: &Path, cap: u64) -> std::io::Result<bool> {
    let len = match std::fs::metadata(log_path) {
        Ok(m) => m.len(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    if len <= cap {
        return Ok(false);
    }
    std::fs::copy(log_path, log_path.with_extension("log.1"))?;
    std::fs::OpenOptions::new()
        .write(true)
        .open(log_path)?
        .set_len(0)?;
    Ok(true)
}

/// Check `log_path` every [`LOG_CHECK_EVERY`] and copy-truncate it past `cap`.
/// Survives respawns (the path is stable); aborted by `shutdown`/`Drop`.
fn spawn_log_rotator(log_path: PathBuf, cap: u64) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(LOG_CHECK_EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        tick.tick().await; // immediate: open_log just checked at spawn
        loop {
            tick.tick().await;
            let path = log_path.clone();
            match tokio::task::spawn_blocking(move || copy_truncate_if_over(&path, cap)).await {
                Ok(Ok(true)) => tracing::debug!("browser: rotated lightpanda.log past {cap} bytes"),
                Ok(Err(e)) => tracing::debug!("browser: lightpanda log check failed: {e}"),
                _ => {}
            }
        }
    })
}

/// Open the sidecar log for APPEND: a respawn after a crash must not truncate
/// the dead instance's output — the only record of why it died. One previous
/// generation is kept once the log passes [`LOG_CAP_BYTES`].
fn open_log(log_path: &Path) -> std::io::Result<std::fs::File> {
    if std::fs::metadata(log_path).is_ok_and(|m| m.len() > LOG_CAP_BYTES) {
        let _ = std::fs::rename(log_path, log_path.with_extension("log.1"));
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)?;
    use std::io::Write;
    let _ = writeln!(
        file,
        "--- otto: lightpanda starting at {} ---",
        chrono::Utc::now().to_rfc3339()
    );
    Ok(file)
}

fn spawn_lightpanda(bin: &Path, data_dir: &Path, port: u16) -> Result<Child, EngineError> {
    let log_path = data_dir.join("lightpanda.log");
    let stdout = open_log(&log_path)
        .map_err(|e| EngineError::Unavailable(format!("open lightpanda log: {e}")))?;
    let stderr = stdout
        .try_clone()
        .map_err(|e| EngineError::Unavailable(format!("clone lightpanda log handle: {e}")))?;
    tracing::info!(
        "browser: starting lightpanda serve (binary {}, port {port})",
        bin.display()
    );
    Command::new(bin)
        .arg("serve")
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| EngineError::Unavailable(format!("spawn lightpanda: {e}")))
}

/// Poll until a CDP websocket handshake on `port` succeeds or `timeout`
/// elapses. lightpanda has no HTTP `/ping`, and a bare TCP accept lands a
/// beat before the websocket upgrade is served — the first navigate then
/// failed with `Connect` and silently degraded to plain fetch — so the
/// readiness signal is the handshake itself.
async fn wait_ready(port: u16, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    let url = format!("ws://127.0.0.1:{port}");
    while tokio::time::Instant::now() < deadline {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            let hs = tokio::time::timeout(
                Duration::from_secs(2),
                tokio_tungstenite::connect_async(&url),
            )
            .await;
            if matches!(hs, Ok(Ok(_))) {
                return true; // dropping the stream closes the probe socket
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    false
}

/// Watches the child; on an unexpected exit (not a `shutdown()`), respawns
/// it on the same port with exponential backoff (capped at 30s), resetting
/// the backoff once a respawn comes up healthy.
fn spawn_supervisor(
    bin: PathBuf,
    data_dir: PathBuf,
    port: u16,
    child: Arc<AsyncMutex<Option<Child>>>,
    shutting_down: Arc<AtomicBool>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut backoff = Duration::from_millis(500);
        loop {
            let had_child = {
                let mut guard = child.lock().await;
                match guard.as_mut() {
                    Some(c) => {
                        let _ = c.wait().await;
                        true
                    }
                    None => false,
                }
            };
            if shutting_down.load(Ordering::SeqCst) {
                return;
            }
            if had_child {
                tracing::warn!(
                    "browser: lightpanda sidecar exited unexpectedly; restarting in {backoff:?}"
                );
                tokio::time::sleep(backoff).await;
            }
            match spawn_lightpanda(&bin, &data_dir, port) {
                Ok(new_child) => {
                    *child.lock().await = Some(new_child);
                    if wait_ready(port, Duration::from_secs(20)).await {
                        backoff = Duration::from_millis(500);
                    } else {
                        // Alive but never serving CDP: waiting on it would
                        // park the supervisor forever. Kill it and retry.
                        backoff = (backoff * 2).min(Duration::from_secs(30));
                        tracing::warn!(
                            "browser: restarted lightpanda never became ready; retrying in {backoff:?}"
                        );
                        if let Some(mut c) = child.lock().await.take() {
                            let _ = c.start_kill();
                            let _ = c.wait().await;
                        }
                        tokio::time::sleep(backoff).await;
                    }
                }
                Err(e) => {
                    tracing::error!("browser: lightpanda restart spawn failed: {e}");
                    backoff = (backoff * 2).min(Duration::from_secs(30));
                    tokio::time::sleep(backoff).await;
                }
            }
        }
    })
}

/// Grab a free loopback TCP port by binding to `:0` and reading it back.
/// TOCTOU: the listener is dropped before lightpanda binds the same port, so
/// in principle another process could grab it first between the two — same
/// risk `otto-usage::clickhouse`'s identical helper takes. Loopback-only,
/// desktop-single-user, so accepted as-is; a lost race surfaces as
/// `start`'s "did not open port in time" error (or the supervisor's
/// restart-with-backoff retrying a fresh spawn on the same port later).
fn free_loopback_port() -> std::io::Result<u16> {
    let l = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = l.local_addr()?.port();
    drop(l);
    Ok(port)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_respawn_appends_to_the_log_and_rotates_past_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("lightpanda.log");
        std::fs::write(&log, "panic: the crash we need\n").unwrap();
        drop(open_log(&log).unwrap());
        let kept = std::fs::read_to_string(&log).unwrap();
        assert!(kept.starts_with("panic: the crash we need"), "{kept}");
        let file = std::fs::OpenOptions::new().append(true).open(&log).unwrap();
        file.set_len(LOG_CAP_BYTES + 1).unwrap();
        drop(open_log(&log).unwrap());
        assert!(dir.path().join("lightpanda.log.1").exists());
        assert!(std::fs::metadata(&log).unwrap().len() < 1024);
    }

    #[test]
    fn a_running_sidecars_log_is_copy_truncated_in_place() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("lightpanda.log");
        // The child's fd: appending, never reopened.
        let mut child_fd = open_log(&log).unwrap();
        assert!(
            !copy_truncate_if_over(&log, 1024).unwrap(),
            "under the cap: untouched"
        );
        child_fd.write_all(&[b'x'; 2048]).unwrap();
        assert!(copy_truncate_if_over(&log, 1024).unwrap());
        assert!(
            std::fs::metadata(dir.path().join("lightpanda.log.1"))
                .unwrap()
                .len()
                > 2048
        );
        assert_eq!(std::fs::metadata(&log).unwrap().len(), 0);
        child_fd.write_all(b"after\n").unwrap();
        assert_eq!(
            std::fs::read(&log).unwrap(),
            b"after\n",
            "O_APPEND lands at the new end"
        );
        assert!(!copy_truncate_if_over(&dir.path().join("missing.log"), 1).unwrap());
    }

    #[test]
    fn locate_rejects_missing_configured_path() {
        // An explicit configured path is authoritative: an invalid one must
        // return None, never fall through to PATH/well-known locations —
        // deterministic regardless of what's installed on this machine.
        assert_eq!(
            Lightpanda::locate(Some("/definitely/not/a/real/lightpanda/binary")),
            None
        );
    }

    #[test]
    fn locate_accepts_a_real_file_as_configured() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap();
        assert_eq!(
            Lightpanda::locate(Some(path)),
            Some(tmp.path().to_path_buf())
        );
    }

    #[test]
    fn free_port_is_loopback() {
        let p = free_loopback_port().unwrap();
        assert!(p > 0);
    }

    #[tokio::test]
    async fn start_errors_when_the_binary_does_not_exist() {
        // `spawn()` itself fails (ENOENT) — fast, no health-poll wait involved.
        let tmp = tempfile::tempdir().unwrap();
        let result = Lightpanda::start(
            PathBuf::from("/definitely/not/a/real/lightpanda/binary"),
            tmp.path().into(),
        )
        .await;
        assert!(matches!(result, Err(EngineError::Unavailable(_))));
    }
}
