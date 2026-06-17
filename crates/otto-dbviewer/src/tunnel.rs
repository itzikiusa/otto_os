//! SSH tunneling via the system `ssh` client (local port-forward).
//!
//! We shell out to `ssh -N -L <local>:<remote_host>:<remote_port> <user@host>`
//! rather than embedding an SSH stack, so the tunnel honours the user's
//! ssh-agent, `~/.ssh/config`, and known_hosts — the same model the rest of
//! Otto already uses for connection profiles. The native driver then connects
//! to the local end of the forward.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use otto_core::{Error, Result};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::{Child, Command};

use crate::types::SshTunnelConfig;

/// A live SSH local port-forward. The `ssh` child is killed on drop, tearing
/// down the tunnel. The `child` is behind a `Mutex` so a shared, cached tunnel
/// (held as `Arc<SshTunnel>`) can be liveness-probed via `&self`.
pub struct SshTunnel {
    child: Mutex<Child>,
    local_port: u16,
}

impl SshTunnel {
    pub fn local_host(&self) -> &'static str {
        "127.0.0.1"
    }
    pub fn local_port(&self) -> u16 {
        self.local_port
    }

    /// Whether the underlying `ssh` child is still running (the forward is
    /// usable). `try_wait()` returning `Ok(None)` means the process hasn't
    /// exited yet — the tunnel is alive. Anything else (it exited, or the probe
    /// failed) is treated as not-alive so the caller re-opens a fresh tunnel.
    pub fn is_alive(&self) -> bool {
        match self.child.lock() {
            Ok(mut child) => matches!(child.try_wait(), Ok(None)),
            // A poisoned lock means a prior holder panicked; treat as dead so we
            // re-open rather than reuse an unknown-state tunnel.
            Err(_) => false,
        }
    }

    /// Open a forward from an ephemeral local port to `remote_host:remote_port`
    /// through the SSH server in `cfg`. Returns once the local port accepts a
    /// TCP connection (or errors after ~12s).
    pub async fn open(
        cfg: &SshTunnelConfig,
        remote_host: &str,
        remote_port: u16,
    ) -> Result<SshTunnel> {
        let local_port = free_local_port().await?;
        let target = format!("{}@{}", cfg.user, cfg.host);
        let forward = format!("127.0.0.1:{local_port}:{remote_host}:{remote_port}");

        let mut cmd = Command::new("ssh");
        cmd.arg("-N")
            .arg("-o")
            .arg("BatchMode=yes")
            .arg("-o")
            .arg("ExitOnForwardFailure=yes")
            .arg("-o")
            .arg("ConnectTimeout=10")
            .arg("-o")
            .arg("ServerAliveInterval=15")
            .arg("-p")
            .arg(cfg.port.to_string());
        if let Some(identity) = cfg.identity_file.as_deref().filter(|s| !s.is_empty()) {
            cmd.arg("-i").arg(identity);
        }
        cmd.arg("-L").arg(&forward).arg(&target).kill_on_drop(true);
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| Error::Upstream(format!("failed to start ssh tunnel: {e}")))?;

        // Wait for the local end to accept connections (the forward is live).
        let deadline = Instant::now() + Duration::from_secs(12);
        loop {
            if let Ok(Some(status)) = child.try_wait() {
                // ssh exited early — surface its stderr.
                let mut msg = format!("ssh tunnel exited early ({status})");
                if let Some(mut err) = child.stderr.take() {
                    use tokio::io::AsyncReadExt;
                    let mut buf = String::new();
                    let _ = err.read_to_string(&mut buf).await;
                    let first = buf.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
                    if !first.is_empty() {
                        msg = format!("ssh tunnel failed: {}", first.trim());
                    }
                }
                return Err(Error::Upstream(msg));
            }
            if TcpStream::connect(("127.0.0.1", local_port)).await.is_ok() {
                return Ok(SshTunnel {
                    child: Mutex::new(child),
                    local_port,
                });
            }
            if Instant::now() >= deadline {
                let _ = child.start_kill();
                return Err(Error::Upstream(
                    "ssh tunnel did not become ready within 12s".into(),
                ));
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    }
}

impl Drop for SshTunnel {
    fn drop(&mut self) {
        // `&mut self` gives us exclusive access; `get_mut` avoids needing to
        // handle lock poisoning here.
        if let Ok(child) = self.child.get_mut() {
            let _ = child.start_kill();
        }
    }
}

/// Ask the OS for a free local TCP port by binding to :0, then releasing it.
async fn free_local_port() -> Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|e| Error::Internal(format!("reserve local port: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| Error::Internal(format!("read local port: {e}")))?
        .port();
    drop(listener);
    Ok(port)
}
