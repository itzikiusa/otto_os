//! Shared SSH tunnelling via the system `ssh` client.
//!
//! We shell out to `ssh` rather than embedding an SSH stack, so the tunnel
//! honours the user's ssh-agent, `~/.ssh/config`, and known_hosts — the same
//! model the rest of Otto already uses for connection profiles. Two forwarding
//! modes share the same liveness/teardown machinery:
//!
//! - **Local forward** (`ssh -N -L <local>:<remote_host>:<remote_port>`): a
//!   fixed local port maps to one remote endpoint. Used for the single-endpoint
//!   database engines (MySQL, Redis, ClickHouse), whose driver connects to the
//!   local end.
//! - **Dynamic SOCKS5** (`ssh -N -D <local>`): a local SOCKS5 proxy through
//!   which a SOCKS-aware client resolves + dials arbitrary hosts from the SSH
//!   server's network. Used for MongoDB (Atlas SRV/replica-set topology) and
//!   for the Kafka brokers proxy (MSK advertises per-broker private DNS names a
//!   single local forward can't represent).
//!
//! This crate is intentionally dependency-light (just `otto-core` + `tokio` +
//! `serde` + `tracing`) so any feature crate can reuse it without pulling in a
//! driver stack.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use otto_core::{Error, Result};
use serde::{Deserialize, Serialize};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::{Child, Command};

pub mod sftp;
pub use sftp::{SftpEntry, SftpParams, SftpSession};

/// SSH tunnel config. Auth uses the system ssh client, so it honours the
/// ssh-agent, `~/.ssh/config`, and known_hosts. Provide an `identity_file` for
/// key auth, or rely on the agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshTunnelConfig {
    pub host: String,
    #[serde(default = "default_ssh_port")]
    pub port: u16,
    /// SSH username. Empty/absent = let ssh resolve it (`~/.ssh/config` `User`,
    /// else the local login name) — the target is then a bare `host`.
    #[serde(default)]
    pub user: String,
    /// Path to a private key on disk (optional; agent is used otherwise).
    #[serde(default)]
    pub identity_file: Option<String>,
}

fn default_ssh_port() -> u16 {
    22
}

/// A live SSH port-forward. The `ssh` child is killed on drop, tearing down the
/// tunnel. The `child` is behind a `Mutex` so a shared, cached tunnel (held as
/// `Arc<SshTunnel>`) can be liveness-probed via `&self`.
pub struct SshTunnel {
    child: Mutex<Child>,
    local_port: u16,
    /// Last line ssh wrote to stderr, kept by the drain task so a tunnel that
    /// dies mid-use still has a diagnostic to report. Empty until ssh says
    /// something.
    last_stderr: std::sync::Arc<Mutex<String>>,
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

    /// The last line ssh wrote to stderr (drained continuously after launch),
    /// for the death report when a tunnel dies mid-use. `None` until ssh has
    /// said anything.
    pub fn last_stderr(&self) -> Option<String> {
        self.last_stderr
            .lock()
            .ok()
            .map(|s| s.clone())
            .filter(|s| !s.is_empty())
    }

    /// Open a local port-forward (`ssh -L`) from an ephemeral local port to
    /// `remote_host:remote_port` through the SSH server in `cfg`. Returns once
    /// the local port accepts a TCP connection (or errors after ~12s). A lost
    /// bind race on the reserved port (another process grabbed it between
    /// reserve and ssh's bind) is retried on a fresh port.
    pub async fn open(
        cfg: &SshTunnelConfig,
        remote_host: &str,
        remote_port: u16,
    ) -> Result<SshTunnel> {
        Self::launch_with_retry(|local_port| {
            local_forward_args(cfg, local_port, remote_host, remote_port)
        })
        .await
    }

    /// Open a dynamic SOCKS5 forward (`ssh -D`) on an ephemeral local port
    /// through the SSH server in `cfg`. A SOCKS-aware client then resolves +
    /// dials arbitrary hosts from the SSH server's network. Returns once the
    /// local SOCKS port accepts a TCP connection.
    pub async fn open_socks(cfg: &SshTunnelConfig) -> Result<SshTunnel> {
        Self::launch_with_retry(|local_port| socks_forward_args(cfg, local_port)).await
    }

    /// Reserve a port + launch, retrying (with a fresh port) when ssh loses the
    /// bind race for the reserved port — the one failure mode where trying
    /// again genuinely helps. Auth/network errors surface on the first attempt.
    async fn launch_with_retry(build_args: impl Fn(u16) -> Vec<String>) -> Result<SshTunnel> {
        const ATTEMPTS: usize = 3;
        let mut last_err = None;
        for _ in 0..ATTEMPTS {
            let local_port = free_local_port().await?;
            match Self::launch(build_args(local_port), local_port).await {
                Ok(t) => return Ok(t),
                Err(e) => {
                    let msg = e.to_string().to_ascii_lowercase();
                    let bind_race = msg.contains("bind") || msg.contains("address already in use");
                    last_err = Some(e);
                    if !bind_race {
                        break;
                    }
                }
            }
        }
        Err(last_err.unwrap_or_else(|| Error::Upstream("ssh tunnel failed".into())))
    }

    /// Spawn `ssh` with the prepared args and wait for the local port to become
    /// usable. Shared by both forwarding modes.
    async fn launch(args: Vec<String>, local_port: u16) -> Result<SshTunnel> {
        let mut cmd = Command::new("ssh");
        cmd.args(&args).kill_on_drop(true);
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
                // TOCTOU guard: the reserved port was released before ssh bound
                // it, so the successful probe could have reached a FOREIGN
                // process that grabbed the port first (ssh then exits — it uses
                // ExitOnForwardFailure). Only bless the tunnel if the ssh child
                // is still running after the probe; an exit here is a failure,
                // never a live tunnel pointing at someone else's listener.
                if let Ok(Some(status)) = child.try_wait() {
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
                // Drain stderr for the tunnel's lifetime: ssh warnings (rekey,
                // keepalive, channel errors) otherwise fill the ~64 KiB pipe
                // buffer and BLOCK ssh — wedging the tunnel while `is_alive()`
                // still reports true. Keep the last line for the death report.
                let last_stderr = std::sync::Arc::new(Mutex::new(String::new()));
                if let Some(err) = child.stderr.take() {
                    let sink = std::sync::Arc::clone(&last_stderr);
                    tokio::spawn(async move {
                        use tokio::io::AsyncBufReadExt;
                        let mut lines = tokio::io::BufReader::new(err).lines();
                        while let Ok(Some(line)) = lines.next_line().await {
                            let line = line.trim().to_string();
                            if line.is_empty() {
                                continue;
                            }
                            tracing::debug!(target: "otto_ssh", "ssh tunnel stderr: {line}");
                            if let Ok(mut slot) = sink.lock() {
                                *slot = line;
                            }
                        }
                    });
                }
                return Ok(SshTunnel {
                    child: Mutex::new(child),
                    local_port,
                    last_stderr,
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
pub async fn free_local_port() -> Result<u16> {
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

/// The shared `ssh` options every tunnel uses: no remote command (`-N`),
/// non-interactive auth (`BatchMode`), trust-on-first-use host keys
/// (`StrictHostKeyChecking=accept-new`), fail fast if the forward can't bind
/// (`ExitOnForwardFailure`), bounded connect, and a keep-alive probe. The
/// forward flag and target are appended by the per-mode builders.
///
/// `BatchMode=yes` disables the interactive "authenticity of host … can't be
/// established" prompt, so without an explicit policy a first-time host fails
/// with "Host key verification failed". `accept-new` adds an *unknown* host's
/// key to `~/.ssh/known_hosts` on first connect (what a user does by answering
/// "yes"), while still refusing to connect if a *known* host's key later
/// changes — the actual MITM signal. This is strictly safer than `=no`.
fn base_args(cfg: &SshTunnelConfig) -> Vec<String> {
    let mut args = vec![
        "-N".into(),
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        "StrictHostKeyChecking=accept-new".into(),
        "-o".into(),
        "ExitOnForwardFailure=yes".into(),
        "-o".into(),
        "ConnectTimeout=10".into(),
        "-o".into(),
        "ServerAliveInterval=15".into(),
        // Two missed keep-alives (~30s) declare the path dead — the default of
        // three left a network-dead tunnel reporting "alive" for ~45s.
        "-o".into(),
        "ServerAliveCountMax=2".into(),
        "-p".into(),
        cfg.port.to_string(),
    ];
    if let Some(identity) = cfg.identity_file.as_deref().filter(|s| !s.is_empty()) {
        args.push("-i".into());
        args.push(identity.to_string());
    }
    args
}

/// `ssh` args for a local port-forward: `… -L 127.0.0.1:<local>:<remote>:<port> user@host`.
fn local_forward_args(
    cfg: &SshTunnelConfig,
    local_port: u16,
    remote_host: &str,
    remote_port: u16,
) -> Vec<String> {
    let mut args = base_args(cfg);
    args.push("-L".into());
    args.push(format!("127.0.0.1:{local_port}:{remote_host}:{remote_port}"));
    args.push(ssh_target(cfg));
    args
}

/// `ssh` args for a dynamic SOCKS5 forward: `… -D 127.0.0.1:<local> user@host`.
fn socks_forward_args(cfg: &SshTunnelConfig, local_port: u16) -> Vec<String> {
    let mut args = base_args(cfg);
    args.push("-D".into());
    args.push(format!("127.0.0.1:{local_port}"));
    args.push(ssh_target(cfg));
    args
}

/// `user@host`, or a bare `host` when no user is set so ssh's own resolution
/// (`~/.ssh/config` `User`, else the local login name) applies.
fn ssh_target(cfg: &SshTunnelConfig) -> String {
    if cfg.user.is_empty() {
        cfg.host.clone()
    } else {
        format!("{}@{}", cfg.user, cfg.host)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> SshTunnelConfig {
        SshTunnelConfig {
            host: "bastion.example.com".into(),
            port: 2222,
            user: "itziklavon".into(),
            identity_file: Some("/home/me/.ssh/id_rsa".into()),
        }
    }

    #[test]
    fn local_forward_args_shape() {
        let args = local_forward_args(&cfg(), 54321, "db.internal", 3306);
        assert_eq!(args[args.len() - 2], "127.0.0.1:54321:db.internal:3306");
        assert_eq!(args.last().unwrap(), "itziklavon@bastion.example.com");
        let l = args.iter().position(|a| a == "-L").unwrap();
        assert_eq!(args[l + 1], "127.0.0.1:54321:db.internal:3306");
        assert!(!args.iter().any(|a| a == "-D"));
        assert!(args.iter().any(|a| a == "BatchMode=yes"));
        assert!(args.iter().any(|a| a == "StrictHostKeyChecking=accept-new"));
        assert!(args.iter().any(|a| a == "ExitOnForwardFailure=yes"));
        assert!(args.iter().any(|a| a == "ServerAliveCountMax=2"));
        assert_eq!(args[args.iter().position(|a| a == "-p").unwrap() + 1], "2222");
        assert_eq!(
            args[args.iter().position(|a| a == "-i").unwrap() + 1],
            "/home/me/.ssh/id_rsa"
        );
    }

    #[test]
    fn socks_forward_args_shape() {
        let args = socks_forward_args(&cfg(), 1080);
        let d = args.iter().position(|a| a == "-D").unwrap();
        assert_eq!(args[d + 1], "127.0.0.1:1080");
        assert_eq!(args.last().unwrap(), "itziklavon@bastion.example.com");
        assert!(!args.iter().any(|a| a == "-L"));
    }

    #[test]
    fn identity_omitted_when_absent() {
        let mut c = cfg();
        c.identity_file = None;
        let args = socks_forward_args(&c, 1080);
        assert!(!args.iter().any(|a| a == "-i"));
    }

    #[test]
    fn ssh_config_default_port() {
        let c: SshTunnelConfig =
            serde_json::from_str(r#"{"host":"h","user":"u"}"#).unwrap();
        assert_eq!(c.port, 22);
        assert!(c.identity_file.is_none());
    }
}
