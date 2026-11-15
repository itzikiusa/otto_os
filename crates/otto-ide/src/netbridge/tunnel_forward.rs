//! On-demand `kubectl port-forward` pool, multiplexed SSH SOCKS5 local relays,
//! private-IP SSH routing, and `otto_netguard`-guarded outbound streams for
//! [`super::tunnel::NetBridgeInstance`].

use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use otto_core::{Error, Result};
use otto_ssh::SshTunnel;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::{TcpStream, UnixListener};
use tokio::process::{Child, Command};

use super::k8s_discover::{resolve_destination, BridgeDestination};
use super::socks5::{connect_via_socks5, SocksUpstreamResolver};
use super::tunnel::NetBridgeInstance;
use super::IdeK8sSpec;

const PORT_FORWARD_READY_TIMEOUT: Duration = Duration::from_secs(12);

pub(super) struct K8sPortForward {
    local_port: u16,
    child: Mutex<Child>,
    _drain_aborts: Vec<tokio::task::AbortHandle>,
}

impl Drop for K8sPortForward {
    fn drop(&mut self) {
        for h in &self._drain_aborts {
            h.abort();
        }
    }
}

impl K8sPortForward {
    fn is_alive(&self) -> bool {
        match self.child.lock() {
            Ok(mut c) => matches!(c.try_wait(), Ok(None)),
            Err(_) => false,
        }
    }
}

pub(super) enum LocalForwardTarget {
    TcpPort(u16),
    UnixSocket(PathBuf),
}

pub(super) struct SshLocalRelay {
    sock_path: PathBuf,
    _weak_inst: Weak<NetBridgeInstance>,
    accept_abort: tokio::task::AbortHandle,
}

impl Drop for SshLocalRelay {
    #[allow(clippy::disallowed_methods)]
    fn drop(&mut self) {
        self.accept_abort.abort();
        let _ = std::fs::remove_file(&self.sock_path);
    }
}

pub(super) type K8sForwardKey = (String, String, u16);
pub(super) type K8sForwardSlot = Arc<tokio::sync::Mutex<Option<K8sPortForward>>>;
pub(super) type SshRelaySlot = Arc<tokio::sync::Mutex<Option<SshLocalRelay>>>;

/// True when `ip` is a private/RFC1918/CGNAT/ULA literal that should ride an
/// active SSH bastion tunnel rather than local TCP (excludes loopback `127.0.0.0/8`
/// incl. synthetic `127.88.x.y` VIPs, `::1`, and cloud-metadata/link-local IPs).
pub(crate) fn is_ssh_routable_private_ip(ip: IpAddr) -> bool {
    if ip.is_loopback() || otto_netguard::is_metadata_ip(ip) {
        return false;
    }
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            if o[0] == 127 {
                return false;
            }
            v4.is_private() || (o[0] == 100 && (o[1] & 0xC0) == 0x40)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_ssh_routable_private_ip(IpAddr::V4(v4));
            }
            (v6.segments()[0] & 0xfe00) == 0xfc00
        }
    }
}

pub(super) fn ensure_not_metadata_target(host: &str) -> Result<()> {
    let bare = host.trim().trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = bare.parse::<IpAddr>() {
        if otto_netguard::is_metadata_ip(ip) {
            return Err(Error::Forbidden(format!(
                "blocked cloud-metadata address {ip} (SSRF guard)"
            )));
        }
    } else if otto_netguard::is_metadata_hostname(bare) {
        return Err(Error::Forbidden(format!(
            "blocked cloud-metadata host {bare} (SSRF guard)"
        )));
    }
    Ok(())
}

impl NetBridgeInstance {
    pub(super) async fn ensure_local_forward_target(
        self: &Arc<Self>,
        dest: BridgeDestination,
    ) -> Result<LocalForwardTarget> {
        match dest {
            BridgeDestination::K8sService {
                namespace,
                service,
                port,
            } => self
                .ensure_k8s_port_forward(&namespace, &service, port)
                .await
                .map(LocalForwardTarget::TcpPort),
            BridgeDestination::SshHost { host, port } => {
                ensure_not_metadata_target(&host)?;
                self.ensure_ssh_local_relay(&host, port)
                    .await
                    .map(LocalForwardTarget::UnixSocket)
            }
        }
    }

    pub(super) async fn ensure_k8s_port_forward(
        &self,
        namespace: &str,
        service: &str,
        port: u16,
    ) -> Result<u16> {
        let Some(spec) = &self.k8s_spec else {
            return Err(Error::Invalid(
                "Kubernetes cluster is not configured".into(),
            ));
        };
        if namespace != spec.namespace && !spec.allow_cross_namespace {
            return Err(Error::Forbidden(format!(
                "Kubernetes namespace '{namespace}' is outside bridged namespace '{}'",
                spec.namespace
            )));
        }
        let key = (namespace.to_string(), service.to_string(), port);
        let slot = self
            .k8s_forwards
            .entry(key)
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(None)))
            .value()
            .clone();

        let mut guard = slot.lock().await;
        if let Some(existing) = guard.as_ref() {
            if existing.is_alive() {
                return Ok(existing.local_port);
            }
        }

        let spawned = spawn_kubectl_port_forward(spec, namespace, service, port).await?;
        let local_port = spawned.local_port;
        *guard = Some(spawned);
        Ok(local_port)
    }

    pub(super) async fn ensure_ssh_socks_port(&self) -> Result<u16> {
        let Some(cfg) = &self.ssh_cfg else {
            return Err(Error::Invalid(
                "SSH bastion tunnel is not configured".into(),
            ));
        };
        let mut guard = self.ssh_tunnel.lock().await;
        if let Some(existing) = guard.as_ref() {
            if existing.is_alive() {
                return Ok(existing.local_port());
            }
        }
        let fresh = Arc::new(SshTunnel::open_socks(cfg).await?);
        let port = fresh.local_port();
        *guard = Some(fresh);
        Ok(port)
    }

    /// Multiplex a local `0o600` Unix domain socket relay inside `sock_dir` through
    /// [`Self::ensure_ssh_socks_port`] (`Weak<NetBridgeInstance>` -> `connect_via_socks5`),
    /// never binding unauthenticated `127.0.0.1:0` TCP ports or spawning extra `ssh` children.
    #[allow(clippy::disallowed_methods)]
    pub(super) async fn ensure_ssh_local_relay(
        self: &Arc<Self>,
        remote_host: &str,
        remote_port: u16,
    ) -> Result<PathBuf> {
        ensure_not_metadata_target(remote_host)?;
        let _ = self.ensure_ssh_socks_port().await?;
        let key = (remote_host.to_string(), remote_port);
        let slot = self
            .ssh_relays
            .entry(key)
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(None)))
            .value()
            .clone();

        let mut guard = slot.lock().await;
        if let Some(existing) = guard.as_ref() {
            if existing.sock_path.exists() {
                return Ok(existing.sock_path.clone());
            }
        }

        let seq = self.next_relay_seq.fetch_add(1, Ordering::Relaxed);
        let sock_path = self.sock_dir.path().join(format!("s{seq}.sock"));
        let _ = std::fs::remove_file(&sock_path);
        let listener = UnixListener::bind(&sock_path)
            .map_err(|e| Error::Internal(format!("bind SSH unix relay: {e}")))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&sock_path, std::fs::Permissions::from_mode(0o600));
        }
        let rhost = remote_host.to_string();
        let weak_inst = Arc::downgrade(self);
        let weak_for_loop = weak_inst.clone();

        let join = tokio::spawn(async move {
            loop {
                let Ok((mut inbound, _)) = listener.accept().await else {
                    break;
                };
                let Some(strong) = weak_for_loop.upgrade() else {
                    break;
                };
                let target_h = rhost.clone();
                tokio::spawn(async move {
                    let Ok(socks_port) = strong.ensure_ssh_socks_port().await else {
                        return;
                    };
                    if let Ok(mut up) = connect_via_socks5(socks_port, &target_h, remote_port).await
                    {
                        let _ = tokio::io::copy_bidirectional(&mut inbound, &mut up).await;
                    }
                });
            }
        });

        *guard = Some(SshLocalRelay {
            sock_path: sock_path.clone(),
            _weak_inst: weak_inst,
            accept_abort: join.abort_handle(),
        });
        Ok(sock_path)
    }

    /// Bind a `0o600` Unix domain socket relay inside `sock_dir` for VIP connections
    /// on unmapped ports (e.g. when DNS interception was triggered by a port-scoped
    /// `HostMapping` on a different port), connecting directly to `(host, port)`.
    #[allow(clippy::disallowed_methods)]
    pub(super) async fn ensure_direct_local_relay(
        self: &Arc<Self>,
        host: &str,
        port: u16,
    ) -> Result<PathBuf> {
        ensure_not_metadata_target(host)?;
        let key = (host.to_string(), port);
        let slot = self
            .direct_relays
            .entry(key)
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(None)))
            .value()
            .clone();

        let mut guard = slot.lock().await;
        if let Some(existing) = guard.as_ref() {
            if existing.sock_path.exists() {
                return Ok(existing.sock_path.clone());
            }
        }

        let seq = self.next_relay_seq.fetch_add(1, Ordering::Relaxed);
        let sock_path = self.sock_dir.path().join(format!("d{seq}.sock"));
        let _ = std::fs::remove_file(&sock_path);
        let listener = UnixListener::bind(&sock_path)
            .map_err(|e| Error::Internal(format!("bind direct unix relay: {e}")))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&sock_path, std::fs::Permissions::from_mode(0o600));
        }
        let target_host = host.to_string();
        let weak_inst = Arc::downgrade(self);
        let weak_for_loop = weak_inst.clone();

        let join = tokio::spawn(async move {
            loop {
                let Ok((mut inbound, _)) = listener.accept().await else {
                    break;
                };
                if weak_for_loop.upgrade().is_none() {
                    break;
                }
                let th = target_host.clone();
                tokio::spawn(async move {
                    if ensure_not_metadata_target(&th).is_err() {
                        return;
                    }
                    if let Ok(mut up) = TcpStream::connect((th.as_str(), port)).await {
                        let _ = up.set_nodelay(true);
                        let _ = tokio::io::copy_bidirectional(&mut inbound, &mut up).await;
                    }
                });
            }
        });

        *guard = Some(SshLocalRelay {
            sock_path: sock_path.clone(),
            _weak_inst: weak_inst,
            accept_abort: join.abort_handle(),
        });
        Ok(sock_path)
    }

    pub async fn open_routed_stream(&self, raw_host: &str, port: u16) -> Result<TcpStream> {
        let bare_raw = raw_host
            .trim()
            .trim_start_matches('[')
            .trim_end_matches(']');
        let (translated_host, was_vip) = if let Ok(v4) = bare_raw.parse::<Ipv4Addr>() {
            if let Some(entry) = self.vip_to_host.get(&v4) {
                (entry.value().clone(), true)
            } else {
                (bare_raw.to_string(), false)
            }
        } else {
            (bare_raw.to_string(), false)
        };

        ensure_not_metadata_target(&translated_host)?;

        let snap = self.current_k8s_snapshot().await;
        if let Some(dest) = resolve_destination(
            &translated_host,
            port,
            &self.cfg,
            snap.as_deref(),
            self.ssh_cfg.is_some(),
        ) {
            return match dest {
                BridgeDestination::K8sService {
                    namespace,
                    service,
                    port: svc_port,
                } => {
                    let lport = self
                        .ensure_k8s_port_forward(&namespace, &service, svc_port)
                        .await?;
                    TcpStream::connect(("127.0.0.1", lport)).await.map_err(|e| {
                        Error::Upstream(format!("connect k8s forward 127.0.0.1:{lport}: {e}"))
                    })
                }
                BridgeDestination::SshHost {
                    host: shost,
                    port: sport,
                } => {
                    ensure_not_metadata_target(&shost)?;
                    let socks_port = self.ensure_ssh_socks_port().await?;
                    connect_via_socks5(socks_port, &shost, sport).await
                }
            };
        }

        if !was_vip && self.ssh_cfg.is_some() {
            if let Ok(ip) = translated_host.parse::<IpAddr>() {
                if is_ssh_routable_private_ip(ip) {
                    let socks_port = self.ensure_ssh_socks_port().await?;
                    return connect_via_socks5(socks_port, &ip.to_string(), port).await;
                }
            }
        }

        let vetted_addrs = otto_netguard::resolve_host_checked(&translated_host, port)
            .await
            .map_err(Error::Forbidden)?;
        TcpStream::connect(vetted_addrs.as_slice())
            .await
            .map_err(|e| Error::Upstream(format!("connect {translated_host}:{port}: {e}")))
    }
}

pub(super) struct NetBridgeSocksAdapter(pub(super) Weak<NetBridgeInstance>);

impl SocksUpstreamResolver for NetBridgeSocksAdapter {
    fn open_upstream(
        &self,
        host: String,
        port: u16,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<TcpStream>> + Send>> {
        let weak = self.0.clone();
        Box::pin(async move {
            let strong = weak
                .upgrade()
                .ok_or_else(|| Error::Internal("network bridge shut down".into()))?;
            strong.open_routed_stream(&host, port).await
        })
    }
}

async fn spawn_kubectl_port_forward(
    spec: &IdeK8sSpec,
    namespace: &str,
    service: &str,
    port: u16,
) -> Result<K8sPortForward> {
    let mut args = spec.kubectl_base_args.clone();
    args.extend([
        "-n".to_string(),
        namespace.to_string(),
        "port-forward".to_string(),
        format!("service/{service}"),
        format!("0:{port}"),
    ]);

    let mut cmd = Command::new(&spec.kubectl_program);
    otto_core::aws_env::apply_aws_env(
        cmd.args(&args),
        spec.kubectl_env
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str())),
    )
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .kill_on_drop(true);

    let mut child = cmd.spawn().map_err(|e| {
        Error::Upstream(format!(
            "failed to spawn kubectl port-forward service/{service}:{port}: {e}"
        ))
    })?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::Internal("kubectl stdout pipe missing".into()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| Error::Internal("kubectl stderr pipe missing".into()))?;

    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<std::result::Result<u16, String>>();
    let ready_tx = Arc::new(Mutex::new(Some(ready_tx)));

    let tx_out = Arc::clone(&ready_tx);
    let out_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(port_num) = parse_forwarding_port(&line) {
                if let Ok(mut g) = tx_out.lock() {
                    if let Some(tx) = g.take() {
                        let _ = tx.send(Ok(port_num));
                    }
                }
            }
        }
    });

    let tx_err = Arc::clone(&ready_tx);
    let err_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        let mut first_err = String::new();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(port_num) = parse_forwarding_port(&line) {
                if let Ok(mut g) = tx_err.lock() {
                    if let Some(tx) = g.take() {
                        let _ = tx.send(Ok(port_num));
                    }
                }
            } else if first_err.is_empty() && !line.trim().is_empty() {
                first_err = line.trim().to_string();
            }
        }
        if let Ok(mut g) = tx_err.lock() {
            if let Some(tx) = g.take() {
                let msg = if first_err.is_empty() {
                    "kubectl port-forward exited before binding".to_string()
                } else {
                    first_err
                };
                let _ = tx.send(Err(msg));
            }
        }
    });

    let drain_aborts = vec![out_task.abort_handle(), err_task.abort_handle()];
    let local_port = match tokio::time::timeout(PORT_FORWARD_READY_TIMEOUT, ready_rx).await {
        Ok(Ok(Ok(p))) => p,
        Ok(Ok(Err(msg))) => {
            let _ = child.start_kill();
            return Err(Error::Upstream(format!(
                "kubectl port-forward service/{service}:{port}: {msg}"
            )));
        }
        Ok(Err(_)) => {
            let _ = child.start_kill();
            return Err(Error::Upstream(format!(
                "kubectl port-forward service/{service}:{port} closed unexpectedly"
            )));
        }
        Err(_) => {
            let _ = child.start_kill();
            return Err(Error::Upstream(format!(
                "kubectl port-forward service/{service}:{port} timed out after 12s"
            )));
        }
    };

    Ok(K8sPortForward {
        local_port,
        child: Mutex::new(child),
        _drain_aborts: drain_aborts,
    })
}

fn parse_forwarding_port(line: &str) -> Option<u16> {
    let idx = line.find("Forwarding from 127.0.0.1:")?;
    let rest = &line[idx + "Forwarding from 127.0.0.1:".len()..];
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse::<u16>().ok().filter(|&p| p > 0)
}
