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

pub mod diagnose;
pub mod sftp;
pub use diagnose::{diagnose as diagnose_ssh_stderr, SshDiagnosis};
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

/// A bastion host `ssh` can only read as a DESTINATION: a hostname / IPv4
/// literal ([`otto_core::network_profiles::valid_host`] — no leading `-`, no
/// spaces or shell/option syntax) or an IPv6 literal. Profiles can come from
/// imported third-party connection files, and a "host" that starts with `-`
/// would otherwise reach `ssh`'s argv as an option.
pub fn valid_ssh_host(host: &str) -> bool {
    otto_core::network_profiles::valid_host(host) || host.parse::<std::net::Ipv6Addr>().is_ok()
}

/// An SSH user name that cannot be read as an option or split the argument:
/// empty (let ssh resolve it), or no leading `-` and no whitespace / control
/// characters.
pub fn valid_ssh_user(user: &str) -> bool {
    user.is_empty()
        || (!user.starts_with('-') && !user.chars().any(|c| c.is_whitespace() || c.is_control()))
}

impl SshTunnelConfig {
    /// Refuse a host / user `ssh` could misread (see [`valid_ssh_host`],
    /// [`valid_ssh_user`]). Checked before every tunnel is opened and when a
    /// profile carrying a tunnel is saved.
    pub fn validate(&self) -> Result<()> {
        if !valid_ssh_host(&self.host) {
            return Err(Error::Invalid(format!(
                "ssh tunnel host '{}' is not a valid hostname or IP address",
                self.host
            )));
        }
        if !valid_ssh_user(&self.user) {
            return Err(Error::Invalid(format!(
                "ssh tunnel user '{}' is not a valid user name",
                self.user
            )));
        }
        Ok(())
    }
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
        cfg.validate()?;
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
        cfg.validate()?;
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

    /// Spawn `ssh` with the prepared args and wait for the forward to come up.
    /// Shared by both forwarding modes.
    ///
    /// Readiness is ssh's OWN "Local forwarding listening on 127.0.0.1 port N"
    /// line (`LogLevel=DEBUG1`, see [`base_args`]), not a bare TCP probe. The
    /// reserved port is released before ssh binds it, and OpenSSH binds `-L` /
    /// `-D` only AFTER authentication — so for the whole connect, host-key and
    /// auth phase a foreign process that grabbed the port would answer a probe
    /// while ssh is still alive, and the tunnel's first client (a DB driver,
    /// with its password) would talk to that process. Once ssh has logged the
    /// bind, the listener on the port is ssh's (a lost bind exits instead,
    /// `ExitOnForwardFailure`); the probe then only confirms it accepts.
    async fn launch(args: Vec<String>, local_port: u16) -> Result<SshTunnel> {
        let mut cmd = Command::new("ssh");
        cmd.args(&args).kill_on_drop(true);
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| Error::Upstream(format!("failed to start ssh tunnel: {e}")))?;
        // Drain stderr from the start, for the tunnel's lifetime: it carries
        // the readiness line, and ssh warnings (rekey, keepalive, channel
        // errors, debug1) otherwise fill the ~64 KiB pipe buffer and BLOCK ssh
        // — wedging the tunnel while `is_alive()` still reports true.
        let mut tap = StderrTap::spawn(child.stderr.take(), local_port);

        let deadline = Instant::now() + Duration::from_secs(12);
        loop {
            if let Ok(Some(status)) = child.try_wait() {
                // ssh exited early — surface its stderr.
                return Err(tap.exit_error(&format!("exited early ({status})")).await);
            }
            if tap.forward_ready() && TcpStream::connect(("127.0.0.1", local_port)).await.is_ok() {
                return Ok(SshTunnel {
                    child: Mutex::new(child),
                    local_port,
                    last_stderr: tap.last,
                });
            }
            if Instant::now() >= deadline {
                // Killing ssh closes its stderr, so whatever it said about the
                // stall (a hung jump host, an unanswered DNS lookup) is readable.
                let _ = child.start_kill();
                return Err(tap.exit_error("did not become ready within 12s").await);
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    }
}

/// True for ssh's debug1 line announcing its own bind of `127.0.0.1:port`
/// (`Local forwarding listening on 127.0.0.1 port N.`, logged for both `-L` and
/// `-D`). The trailing `.` keeps port 80 from matching a line about port 8080.
fn is_forward_ready_line(line: &str, port: u16) -> bool {
    line.contains(&format!(
        "Local forwarding listening on 127.0.0.1 port {port}."
    ))
}

/// ssh's stderr, read line by line from spawn on. Keeps the non-debug text for
/// the early-exit diagnosis (bounded), the last non-debug line for a death
/// report, and flips `ready` on the forward's readiness line.
struct StderrTap {
    ready: std::sync::Arc<std::sync::atomic::AtomicBool>,
    text: std::sync::Arc<Mutex<String>>,
    last: std::sync::Arc<Mutex<String>>,
    task: Option<tokio::task::JoinHandle<()>>,
}

/// Cap on the stderr text kept for a diagnosis — the explanation is in the
/// first lines; a chatty ssh must not grow the buffer for the tunnel's life.
const STDERR_KEEP: usize = 16 * 1024;

impl StderrTap {
    fn spawn(err: Option<tokio::process::ChildStderr>, port: u16) -> StderrTap {
        use std::sync::atomic::Ordering;
        let ready = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let text = std::sync::Arc::new(Mutex::new(String::new()));
        let last = std::sync::Arc::new(Mutex::new(String::new()));
        let task = err.map(|err| {
            let (ready, text, last) = (ready.clone(), text.clone(), last.clone());
            tokio::spawn(async move {
                use tokio::io::AsyncBufReadExt;
                let mut lines = tokio::io::BufReader::new(err).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let line = line.trim().to_string();
                    if line.is_empty() {
                        continue;
                    }
                    if is_forward_ready_line(&line, port) {
                        ready.store(true, Ordering::Release);
                    }
                    if line.starts_with("debug") {
                        tracing::trace!(target: "otto_ssh", "ssh tunnel stderr: {line}");
                        continue;
                    }
                    tracing::debug!(target: "otto_ssh", "ssh tunnel stderr: {line}");
                    if let Ok(mut t) = text.lock() {
                        if t.len() < STDERR_KEEP {
                            t.push_str(&line);
                            t.push('\n');
                        }
                    }
                    if let Ok(mut slot) = last.lock() {
                        *slot = line;
                    }
                }
            })
        });
        StderrTap {
            ready,
            text,
            last,
            task,
        }
    }

    fn forward_ready(&self) -> bool {
        self.ready.load(std::sync::atomic::Ordering::Acquire)
    }

    /// The error for a tunnel whose `ssh` exited (or was killed) before the
    /// forward came up: ssh's own explanation, de-noised and with a fix hint
    /// ([`diagnose::diagnose`]), else `fallback`. Waiting for stderr's end is
    /// bounded — a grandchild (ProxyCommand) can keep the pipe open after ssh
    /// itself is gone.
    async fn exit_error(&mut self, fallback: &str) -> Error {
        if let Some(task) = self.task.take() {
            let _ = tokio::time::timeout(Duration::from_secs(2), task).await;
        }
        let buf = self.text.lock().map(|t| t.clone()).unwrap_or_default();
        let diagnosis = diagnose::diagnose(&buf);
        if diagnosis.is_empty() {
            Error::Upstream(format!("ssh tunnel {fallback}"))
        } else {
            Error::Upstream(format!("ssh tunnel failed: {diagnosis}"))
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
        // debug1 carries "Local forwarding listening on … port N", the only
        // proof the forwarded port is ssh's own listener (see `launch`).
        "-o".into(),
        "LogLevel=DEBUG1".into(),
        "-p".into(),
        cfg.port.to_string(),
    ];
    if let Some(identity) = cfg.identity_file.as_deref().filter(|s| !s.is_empty()) {
        args.push("-i".into());
        args.push(identity.to_string());
    }
    args
}

/// `ssh` args for a local port-forward: `… -L 127.0.0.1:<local>:<remote>:<port> -- user@host`.
fn local_forward_args(
    cfg: &SshTunnelConfig,
    local_port: u16,
    remote_host: &str,
    remote_port: u16,
) -> Vec<String> {
    let mut args = base_args(cfg);
    args.push("-L".into());
    args.push(format!(
        "127.0.0.1:{local_port}:{}:{remote_port}",
        forward_host(remote_host)
    ));
    // `--` ends option parsing: the destination is never read as an option.
    args.push("--".into());
    args.push(ssh_target(cfg));
    args
}

/// The remote host as ssh's `-L` spec needs it: an IPv6 literal must be
/// bracketed (`[fd00::5]`), or its colons are read as field separators and the
/// forward points at the wrong host/port. Already-bracketed and non-IPv6 hosts
/// pass through unchanged.
fn forward_host(host: &str) -> String {
    if host.parse::<std::net::Ipv6Addr>().is_ok() {
        format!("[{host}]")
    } else {
        host.to_string()
    }
}

/// `ssh` args for a dynamic SOCKS5 forward: `… -D 127.0.0.1:<local> -- user@host`.
fn socks_forward_args(cfg: &SshTunnelConfig, local_port: u16) -> Vec<String> {
    let mut args = base_args(cfg);
    args.push("-D".into());
    args.push(format!("127.0.0.1:{local_port}"));
    args.push("--".into());
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
        assert_eq!(args[args.len() - 3], "127.0.0.1:54321:db.internal:3306");
        assert_eq!(args[args.len() - 2], "--");
        assert_eq!(args.last().unwrap(), "itziklavon@bastion.example.com");
        let l = args.iter().position(|a| a == "-L").unwrap();
        assert_eq!(args[l + 1], "127.0.0.1:54321:db.internal:3306");
        assert!(!args.iter().any(|a| a == "-D"));
        assert!(args.iter().any(|a| a == "BatchMode=yes"));
        assert!(args.iter().any(|a| a == "StrictHostKeyChecking=accept-new"));
        assert!(args.iter().any(|a| a == "ExitOnForwardFailure=yes"));
        assert!(args.iter().any(|a| a == "ServerAliveCountMax=2"));
        assert_eq!(
            args[args.iter().position(|a| a == "-p").unwrap() + 1],
            "2222"
        );
        assert_eq!(
            args[args.iter().position(|a| a == "-i").unwrap() + 1],
            "/home/me/.ssh/id_rsa"
        );
    }

    #[test]
    fn ipv6_forward_target_is_bracketed() {
        let args = local_forward_args(&cfg(), 5000, "fd00::5", 5432);
        let l = args.iter().position(|a| a == "-L").unwrap();
        assert_eq!(args[l + 1], "127.0.0.1:5000:[fd00::5]:5432");
        // Already bracketed / hostnames / IPv4 are left alone.
        assert_eq!(forward_host("[fd00::5]"), "[fd00::5]");
        assert_eq!(forward_host("db.internal"), "db.internal");
        assert_eq!(forward_host("10.0.0.5"), "10.0.0.5");
    }

    /// A tunnel whose ssh dies before the forward is up reports ssh's real
    /// reason (not the known_hosts notice that precedes it) plus a fix.
    #[tokio::test]
    async fn early_exit_reports_the_diagnosed_reason() {
        let mut child = Command::new("sh")
            .args([
                "-c",
                "echo \"Warning: Permanently added 'h' (ED25519) to the list of known hosts.\" >&2; \
                 echo 'me@h: Permission denied (publickey).' >&2; exit 255",
            ])
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let _ = child.wait().await;
        let mut tap = StderrTap::spawn(child.stderr.take(), 1);
        let msg = tap.exit_error("exited early").await.to_string();
        assert!(msg.contains("Permission denied (publickey)"), "{msg}");
        assert!(msg.contains("ssh-add"), "{msg}");
        assert!(!msg.contains("Permanently added"), "{msg}");
    }

    /// Only ssh's own bind line for THIS port marks the forward ready — the
    /// TCP probe alone could reach a foreign listener during ssh's auth phase.
    #[test]
    fn forward_ready_line_matches_only_this_port() {
        let line = "debug1: Local forwarding listening on 127.0.0.1 port 54321.";
        assert!(is_forward_ready_line(line, 54321));
        assert!(!is_forward_ready_line(line, 5432));
        assert!(!is_forward_ready_line(
            "debug1: Local connections to 127.0.0.1:54321 forwarded to remote address db:3306",
            54321
        ));
        assert!(local_forward_args(&cfg(), 1, "h", 2)
            .iter()
            .any(|a| a == "LogLevel=DEBUG1"));
    }

    /// debug1 chatter (on for the readiness line) neither reaches the
    /// diagnosis nor wins the death report's last line.
    #[tokio::test]
    async fn stderr_tap_flags_ready_and_drops_debug_lines() {
        let mut child = Command::new("sh")
            .args([
                "-c",
                "echo 'debug1: Local forwarding listening on 127.0.0.1 port 7001.' >&2; \
                 echo 'channel 0: open failed' >&2; echo 'debug1: keepalive' >&2",
            ])
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let _ = child.wait().await;
        let mut tap = StderrTap::spawn(child.stderr.take(), 7001);
        let _ = tap.exit_error("x").await;
        assert!(tap.forward_ready());
        assert_eq!(*tap.last.lock().unwrap(), "channel 0: open failed");
        assert!(!tap.text.lock().unwrap().contains("debug1"));
    }

    #[test]
    fn socks_forward_args_shape() {
        let args = socks_forward_args(&cfg(), 1080);
        let d = args.iter().position(|a| a == "-D").unwrap();
        assert_eq!(args[d + 1], "127.0.0.1:1080");
        assert_eq!(args[args.len() - 2], "--");
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

    /// A profile (possibly imported) can never smuggle an option into ssh's
    /// argv through its host or user.
    #[test]
    fn tunnel_host_and_user_are_validated() {
        assert!(cfg().validate().is_ok());
        let mut bare = cfg();
        bare.user = String::new();
        assert!(bare.validate().is_ok());
        for host in ["10.0.0.5", "bastion-1.internal", "fe80::1", "[fe80::1]"] {
            let mut c = cfg();
            c.host = host.into();
            assert!(c.validate().is_ok(), "{host}");
        }
        for host in ["", "-oFoo=bar", "host name", "a;b"] {
            let mut c = cfg();
            c.host = host.into();
            assert!(c.validate().is_err(), "{host:?}");
        }
        for user in ["-oFoo=bar", "a b", "a\nb"] {
            let mut c = cfg();
            c.user = user.into();
            assert!(c.validate().is_err(), "{user:?}");
        }
    }

    #[test]
    fn ssh_config_default_port() {
        let c: SshTunnelConfig = serde_json::from_str(r#"{"host":"h","user":"u"}"#).unwrap();
        assert_eq!(c.port, 22);
        assert!(c.identity_file.is_none());
    }
}
