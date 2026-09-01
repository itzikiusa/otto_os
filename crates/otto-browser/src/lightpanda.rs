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
}

impl Lightpanda {
    /// Resolve the `lightpanda` binary in priority order: an explicit
    /// configured path, then `PATH`, then well-known install locations
    /// (including the slot Otto would auto-download a binary into).
    pub fn locate(configured: Option<&str>) -> Option<PathBuf> {
        if let Some(p) = configured.map(str::trim).filter(|s| !s.is_empty()) {
            let pb = PathBuf::from(p);
            if pb.is_file() {
                return Some(pb);
            }
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
        let supervisor =
            spawn_supervisor(bin, data_dir, port, child.clone(), shutting_down.clone());

        tracing::info!("browser: lightpanda sidecar ready on port {port}");
        Ok(Self {
            port,
            child,
            shutting_down,
            supervisor: StdMutex::new(Some(supervisor)),
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
        if let Ok(mut g) = self.supervisor.lock() {
            if let Some(h) = g.take() {
                h.abort();
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
        if let Ok(mut g) = self.supervisor.lock() {
            if let Some(h) = g.take() {
                h.abort();
            }
        }
        if let Ok(mut g) = self.child.try_lock() {
            if let Some(mut c) = g.take() {
                let _ = c.start_kill();
            }
        }
    }
}

fn spawn_lightpanda(bin: &Path, data_dir: &Path, port: u16) -> Result<Child, EngineError> {
    let log_path = data_dir.join("lightpanda.log");
    let stdout = std::fs::File::create(&log_path)
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

/// Poll a loopback TCP connect to `port` until it succeeds or `timeout`
/// elapses — lightpanda has no HTTP `/ping`, so "accepts a connection" is
/// the readiness signal.
async fn wait_ready(port: u16, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return true;
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
                        backoff = (backoff * 2).min(Duration::from_secs(30));
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
    fn locate_rejects_missing_configured_path() {
        assert!(
            Lightpanda::locate(Some("/definitely/not/a/real/lightpanda/binary")).is_none()
                || Lightpanda::locate(None).is_some()
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
