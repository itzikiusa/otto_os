//! Live per-environment network bridge instance: Unix control socket for the
//! POSIX `getaddrinfo`/`connect` interposer, on-demand `kubectl port-forward`
//! pool, SSH SOCKS5 forwarders, fixed `localhost:<port>` listeners, and JVM
//! SOCKS5 loopback server.

use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU16, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dashmap::DashMap;
use otto_core::{Error, Result};
use otto_ssh::{SshTunnel, SshTunnelConfig};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream, UnixListener, UnixStream};
use tokio::process::{Child, Command};

use super::k8s_discover::{
    resolve_destination, should_intercept_dns, BridgeDestination, DiscoveryCache,
    K8sDiscoverySnapshot,
};
use super::socks5::{connect_via_socks5, spawn_local_socks5_server, SocksUpstreamResolver};
use super::IdeK8sSpec;
use crate::types::IdeNetworkBridgeConfig;

const PORT_FORWARD_READY_TIMEOUT: Duration = Duration::from_secs(12);
const MAX_INJECTED_K8S_ENV_SERVICES: usize = 128;

struct K8sPortForward {
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

struct SshLocalRelay {
    local_port: u16,
    accept_abort: tokio::task::AbortHandle,
}

impl Drop for SshLocalRelay {
    fn drop(&mut self) {
        self.accept_abort.abort();
    }
}

type K8sForwardKey = (String, String, u16);
type K8sForwardSlot = Arc<tokio::sync::Mutex<Option<K8sPortForward>>>;
type SshRelaySlot = Arc<tokio::sync::Mutex<Option<SshLocalRelay>>>;

pub struct NetBridgeInstance {
    cfg: IdeNetworkBridgeConfig,
    k8s_spec: Option<IdeK8sSpec>,
    ssh_cfg: Option<SshTunnelConfig>,
    discovery_cache: Arc<DiscoveryCache>,
    cached_k8s: tokio::sync::RwLock<Option<K8sDiscoverySnapshot>>,
    interposer_lib: Option<PathBuf>,
    auth_token: String,
    sock_path: PathBuf,
    _sock_dir: tempfile::TempDir,
    socks_port: AtomicU16,
    host_to_vip: DashMap<String, Ipv4Addr>,
    vip_to_host: DashMap<Ipv4Addr, String>,
    next_vip_seq: AtomicU32,
    k8s_forwards: DashMap<K8sForwardKey, K8sForwardSlot>,
    ssh_tunnel: tokio::sync::Mutex<Option<Arc<SshTunnel>>>,
    ssh_relays: DashMap<(String, u16), SshRelaySlot>,
    abort_handles: Mutex<Vec<tokio::task::AbortHandle>>,
}

impl Drop for NetBridgeInstance {
    fn drop(&mut self) {
        if let Ok(handles) = self.abort_handles.lock() {
            for h in handles.iter() {
                h.abort();
            }
        }
    }
}

impl NetBridgeInstance {
    pub async fn start(
        data_dir: &Path,
        cfg: IdeNetworkBridgeConfig,
        k8s_spec: Option<IdeK8sSpec>,
        ssh_cfg: Option<SshTunnelConfig>,
        discovery_cache: Arc<DiscoveryCache>,
    ) -> Result<Arc<Self>> {
        let initial_snap = if let Some(spec) = &k8s_spec {
            Some(discovery_cache.get_or_fetch(spec, false).await?)
        } else {
            None
        };

        let initial_ssh = if let Some(scfg) = &ssh_cfg {
            Some(Arc::new(SshTunnel::open_socks(scfg).await?))
        } else {
            None
        };

        let interposer_lib = super::interposer::ensure_interposer_lib(data_dir)
            .await
            .unwrap_or(None);

        let sock_dir = tempfile::Builder::new()
            .prefix("otto-nb-")
            .tempdir_in("/tmp")
            .or_else(|_| tempfile::Builder::new().prefix("otto-nb-").tempdir())
            .map_err(|e| Error::Internal(format!("create netbridge socket dir: {e}")))?;
        let sock_path = sock_dir.path().join("b.sock");
        let unix_listener = UnixListener::bind(&sock_path)
            .map_err(|e| Error::Internal(format!("bind netbridge unix socket: {e}")))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&sock_path, std::fs::Permissions::from_mode(0o600));
        }

        let token_bytes: [u8; 16] = rand::random();
        let auth_token = hex::encode(token_bytes);

        let inst = Arc::new(Self {
            cfg,
            k8s_spec,
            ssh_cfg,
            discovery_cache,
            cached_k8s: tokio::sync::RwLock::new(initial_snap.clone()),
            interposer_lib,
            auth_token,
            sock_path,
            _sock_dir: sock_dir,
            socks_port: AtomicU16::new(0),
            host_to_vip: DashMap::new(),
            vip_to_host: DashMap::new(),
            next_vip_seq: AtomicU32::new(1),
            k8s_forwards: DashMap::new(),
            ssh_tunnel: tokio::sync::Mutex::new(initial_ssh),
            ssh_relays: DashMap::new(),
            abort_handles: Mutex::new(Vec::new()),
        });

        if let Some(snap) = &initial_snap {
            for svc_name in snap.services.keys().take(MAX_INJECTED_K8S_ENV_SERVICES) {
                if svc_name != "kubernetes" {
                    let _ = inst.allocate_vip(svc_name);
                }
            }
        }

        let resolver_arc: Arc<dyn SocksUpstreamResolver> =
            Arc::new(NetBridgeSocksAdapter(Arc::downgrade(&inst)));
        let (socks_port, socks_abort) = spawn_local_socks5_server(resolver_arc).await?;
        inst.socks_port.store(socks_port, Ordering::Release);

        let mut aborts = vec![socks_abort];
        let weak_for_unix = Arc::downgrade(&inst);
        let unix_task = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = unix_listener.accept().await else {
                    break;
                };
                let Some(strong) = weak_for_unix.upgrade() else {
                    break;
                };
                tokio::spawn(async move {
                    let _ = strong.handle_control_client(stream).await;
                });
            }
        });
        aborts.push(unix_task.abort_handle());

        for mapping in &inst.cfg.host_mappings {
            let fh = mapping.from_host.trim().to_ascii_lowercase();
            let Some(from_port) = mapping.from_port.filter(|&p| p > 0) else {
                continue;
            };
            if !matches!(fh.as_str(), "localhost" | "127.0.0.1") {
                continue;
            }
            let to_host = mapping.to_host.trim().to_string();
            let to_port = mapping.to_port.unwrap_or(from_port);
            if to_host.is_empty() {
                continue;
            }
            match TcpListener::bind(("127.0.0.1", from_port)).await {
                Ok(listener) => {
                    let weak_l = Arc::downgrade(&inst);
                    let join = tokio::spawn(async move {
                        loop {
                            let Ok((mut inbound, _)) = listener.accept().await else {
                                break;
                            };
                            let _ = inbound.set_nodelay(true);
                            let Some(strong) = weak_l.upgrade() else {
                                break;
                            };
                            let th = to_host.clone();
                            tokio::spawn(async move {
                                if let Ok(mut up) = strong.open_routed_stream(&th, to_port).await {
                                    let _ =
                                        tokio::io::copy_bidirectional(&mut inbound, &mut up).await;
                                }
                            });
                        }
                    });
                    aborts.push(join.abort_handle());
                }
                Err(e) => {
                    tracing::warn!(
                        from_port,
                        to_host,
                        error = %e,
                        "otto-ide netbridge: localhost port mapping already in use"
                    );
                }
            }
        }

        if let Ok(mut guard) = inst.abort_handles.lock() {
            guard.extend(aborts);
        }

        Ok(inst)
    }

    pub fn is_healthy(&self) -> bool {
        self.sock_path.exists()
    }

    pub async fn summary_banner(&self) -> String {
        let mut parts = Vec::new();
        if let Some(spec) = &self.k8s_spec {
            let svc_count = self
                .cached_k8s
                .read()
                .await
                .as_ref()
                .map_or(0, |s| s.services.len());
            parts.push(format!(
                "K8s {}/{} ({} services)",
                spec.cluster_name, spec.namespace, svc_count
            ));
        }
        if let Some(ssh) = &self.ssh_cfg {
            parts.push(format!("SSH tunnel {}:{}", ssh.host, ssh.port));
        }
        if !self.cfg.host_mappings.is_empty() {
            parts.push(format!("{} host mappings", self.cfg.host_mappings.len()));
        }
        format!(
            "[NetBridge] Active ({}) · SOCKS5 127.0.0.1:{}",
            parts.join(" + "),
            self.socks_port.load(Ordering::Acquire)
        )
    }

    /// Inject `OTTO_NETBRIDGE_*`, `DYLD_INSERT_LIBRARIES` / `LD_PRELOAD`, JVM
    /// `JAVA_TOOL_OPTIONS`, and Kubernetes metadata / `*_SERVICE_HOST` variables
    /// into a Run or Debug child environment. Explicit user variables win.
    pub fn apply_to_env(&self, env: &mut BTreeMap<String, String>) {
        env.insert("OTTO_NETBRIDGE_ACTIVE".into(), "1".into());
        env.insert(
            "OTTO_NETBRIDGE_SOCK".into(),
            self.sock_path.to_string_lossy().into_owned(),
        );
        env.insert("OTTO_NETBRIDGE_TOKEN".into(), self.auth_token.clone());
        if self.k8s_spec.is_some() || self.ssh_cfg.is_some() {
            env.insert("OTTO_NETBRIDGE_PRIVATE_IPS".into(), "1".into());
        }

        let socks_port = self.socks_port.load(Ordering::Acquire);
        if socks_port > 0 {
            env.insert(
                "OTTO_NETBRIDGE_SOCKS5".into(),
                format!("127.0.0.1:{socks_port}"),
            );
            let jvm_flags = format!("-DsocksProxyHost=127.0.0.1 -DsocksProxyPort={socks_port}");
            match env.get_mut("JAVA_TOOL_OPTIONS") {
                Some(existing) if !existing.contains("-DsocksProxyHost=") => {
                    if !existing.trim().is_empty() {
                        existing.push(' ');
                    }
                    existing.push_str(&jvm_flags);
                }
                None => {
                    env.insert("JAVA_TOOL_OPTIONS".into(), jvm_flags);
                }
                _ => {}
            }
        }

        if let Some(lib_path) = &self.interposer_lib {
            let lib_str = lib_path.to_string_lossy().into_owned();
            env.insert("OTTO_NETBRIDGE_DYLIB".into(), lib_str.clone());
            let preload_key = if cfg!(target_os = "macos") {
                Some("DYLD_INSERT_LIBRARIES")
            } else if cfg!(target_os = "linux") {
                Some("LD_PRELOAD")
            } else {
                None
            };
            if let Some(key) = preload_key {
                match env.get_mut(key) {
                    Some(existing) if !existing.split(':').any(|p| p == lib_str) => {
                        if existing.trim().is_empty() {
                            *existing = lib_str;
                        } else {
                            *existing = format!("{lib_str}:{existing}");
                        }
                    }
                    None => {
                        env.insert(key.into(), lib_str);
                    }
                    _ => {}
                }
            }
        }

        if let Some(spec) = &self.k8s_spec {
            env.entry("OTTO_K8S_CLUSTER".into())
                .or_insert_with(|| spec.cluster_name.clone());
            env.entry("OTTO_K8S_NAMESPACE".into())
                .or_insert_with(|| spec.namespace.clone());
            env.entry("KUBERNETES_NAMESPACE".into())
                .or_insert_with(|| spec.namespace.clone());
            env.entry("POD_NAMESPACE".into())
                .or_insert_with(|| spec.namespace.clone());
        }
        if self.cfg.containerized {
            env.entry("CONTAINERIZED".into())
                .or_insert_with(|| "true".into());
        }
        if self.cfg.inject_k8s_service_env {
            if let Ok(guard) = self.cached_k8s.try_read() {
                if let Some(snap) = guard.as_ref() {
                    for svc in snap.services.values().take(MAX_INJECTED_K8S_ENV_SERVICES) {
                        if svc.name == "kubernetes" {
                            continue;
                        }
                        let Some(first_port) = svc.ports.first().map(|p| p.port) else {
                            continue;
                        };
                        let prefix: String = svc
                            .name
                            .chars()
                            .map(|c| {
                                if c.is_ascii_alphanumeric() {
                                    c.to_ascii_uppercase()
                                } else {
                                    '_'
                                }
                            })
                            .collect();
                        if prefix.is_empty() {
                            continue;
                        }
                        let vip = self.allocate_vip(&svc.name).to_string();
                        env.entry(format!("{prefix}_SERVICE_HOST"))
                            .or_insert_with(|| vip.clone());
                        env.entry(format!("{prefix}_SERVICE_PORT"))
                            .or_insert_with(|| first_port.to_string());
                    }
                }
            }
        }
    }

    fn allocate_vip(&self, raw_host: &str) -> Ipv4Addr {
        let host_lc = raw_host.trim().trim_end_matches('.').to_ascii_lowercase();
        if let Some(existing) = self.host_to_vip.get(&host_lc) {
            return *existing.value();
        }
        let seq = (self.next_vip_seq.fetch_add(1, Ordering::Relaxed) % 65_000) + 1;
        let hi = ((seq >> 8) & 0xFF) as u8;
        let lo = (seq & 0xFF) as u8;
        let vip = Ipv4Addr::new(127, 88, hi, lo.max(1));
        self.vip_to_host.insert(vip, host_lc.clone());
        self.host_to_vip.insert(host_lc, vip);
        vip
    }

    async fn current_k8s_snapshot(&self) -> Option<K8sDiscoverySnapshot> {
        let spec = self.k8s_spec.as_ref()?;
        if let Ok(fresh) = self.discovery_cache.get_or_fetch(spec, false).await {
            let mut guard = self.cached_k8s.write().await;
            *guard = Some(fresh.clone());
            return Some(fresh);
        }
        self.cached_k8s.read().await.clone()
    }

    fn check_token(&self, candidate: &str) -> bool {
        let a = self.auth_token.as_bytes();
        let b = candidate.as_bytes();
        if a.len() != b.len() {
            return false;
        }
        let mut diff: u8 = 0;
        for (&x, &y) in a.iter().zip(b.iter()) {
            diff |= x ^ y;
        }
        std::hint::black_box(diff) == 0
    }

    async fn handle_control_client(&self, stream: UnixStream) -> std::io::Result<()> {
        let (rd, mut wr) = stream.into_split();
        let mut reader = BufReader::new(rd);
        let mut line = String::new();
        let n = tokio::time::timeout(Duration::from_secs(5), reader.read_line(&mut line))
            .await
            .map_err(std::io::Error::other)??;
        if n == 0 || line.len() > 1024 {
            return Ok(());
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        match parts.as_slice() {
            ["RESOLVE", token, host] => {
                if !self.check_token(token) {
                    wr.write_all(b"ERR unauthorized\n").await?;
                    return Ok(());
                }
                let snap = self.current_k8s_snapshot().await;
                if should_intercept_dns(host, &self.cfg, snap.as_ref(), self.ssh_cfg.is_some()) {
                    let vip = self.allocate_vip(host);
                    wr.write_all(format!("VIP {vip}\n").as_bytes()).await?;
                } else {
                    wr.write_all(b"MISS\n").await?;
                }
            }
            ["CONNECT", token, ip_str, port_str] => {
                if !self.check_token(token) {
                    wr.write_all(b"ERR unauthorized\n").await?;
                    return Ok(());
                }
                let Ok(port) = port_str.parse::<u16>() else {
                    wr.write_all(b"ERR bad_port\n").await?;
                    return Ok(());
                };
                match self.resolve_connect_port(ip_str, port).await {
                    Ok(Some(lport)) => {
                        wr.write_all(format!("PORT {lport}\n").as_bytes()).await?;
                    }
                    Ok(None) => {
                        wr.write_all(b"PASSTHROUGH\n").await?;
                    }
                    Err(e) => {
                        tracing::debug!(ip = ip_str, port, error = %e, "netbridge connect failed");
                        wr.write_all(b"ERR upstream\n").await?;
                    }
                }
            }
            _ => {
                wr.write_all(b"ERR syntax\n").await?;
            }
        }
        wr.flush().await
    }

    async fn resolve_connect_port(&self, ip_str: &str, req_port: u16) -> Result<Option<u16>> {
        let snap = self.current_k8s_snapshot().await;
        if let Ok(v4) = ip_str.parse::<Ipv4Addr>() {
            if let Some(host_entry) = self.vip_to_host.get(&v4) {
                let host = host_entry.value().clone();
                drop(host_entry);
                let Some(dest) = resolve_destination(
                    &host,
                    req_port,
                    &self.cfg,
                    snap.as_ref(),
                    self.ssh_cfg.is_some(),
                ) else {
                    return Err(Error::NotFound(format!(
                        "no bridge route for {host}:{req_port}"
                    )));
                };
                return self.ensure_local_forward_port(dest).await.map(Some);
            }
        }

        // Direct ClusterIP or RFC1918 IP connection (`OTTO_NETBRIDGE_PRIVATE_IPS=1`).
        if let Some(s) = &snap {
            if let Some(svc_name) = s.cluster_ips.get(ip_str) {
                if let Some(entry) = s.services.get(svc_name) {
                    let port = entry.resolve_service_port(req_port);
                    return self
                        .ensure_k8s_port_forward(&s.namespace, &entry.name, port)
                        .await
                        .map(Some);
                }
            }
        }
        if self.ssh_cfg.is_some() {
            return self
                .ensure_ssh_local_relay(ip_str, req_port)
                .await
                .map(Some);
        }
        Ok(None)
    }

    async fn ensure_local_forward_port(&self, dest: BridgeDestination) -> Result<u16> {
        match dest {
            BridgeDestination::K8sService {
                namespace,
                service,
                port,
            } => {
                self.ensure_k8s_port_forward(&namespace, &service, port)
                    .await
            }
            BridgeDestination::SshHost { host, port } => {
                self.ensure_ssh_local_relay(&host, port).await
            }
        }
    }

    async fn ensure_k8s_port_forward(
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

    async fn ensure_ssh_socks_port(&self) -> Result<u16> {
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

    async fn ensure_ssh_local_relay(&self, remote_host: &str, remote_port: u16) -> Result<u16> {
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
            return Ok(existing.local_port);
        }

        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|e| Error::Internal(format!("bind SSH local relay: {e}")))?;
        let local_port = listener
            .local_addr()
            .map_err(|e| Error::Internal(format!("SSH relay local_addr: {e}")))?
            .port();
        let ssh_cfg = self
            .ssh_cfg
            .clone()
            .ok_or_else(|| Error::Invalid("SSH tunnel missing".into()))?;
        let rhost = remote_host.to_string();
        let tunnel_cell_initial = self.ssh_tunnel.lock().await.clone();
        let shared_tunnel = Arc::new(tokio::sync::Mutex::new(tunnel_cell_initial));

        let join = tokio::spawn(async move {
            loop {
                let Ok((mut inbound, _)) = listener.accept().await else {
                    break;
                };
                let _ = inbound.set_nodelay(true);
                let scfg = ssh_cfg.clone();
                let target_h = rhost.clone();
                let tcell = Arc::clone(&shared_tunnel);
                tokio::spawn(async move {
                    let socks_port = {
                        let mut g = tcell.lock().await;
                        if let Some(t) = g.as_ref().filter(|t| t.is_alive()) {
                            t.local_port()
                        } else if let Ok(fresh) = SshTunnel::open_socks(&scfg).await {
                            let p = fresh.local_port();
                            *g = Some(Arc::new(fresh));
                            p
                        } else {
                            return;
                        }
                    };
                    if let Ok(mut up) = connect_via_socks5(socks_port, &target_h, remote_port).await
                    {
                        let _ = tokio::io::copy_bidirectional(&mut inbound, &mut up).await;
                    }
                });
            }
        });

        *guard = Some(SshLocalRelay {
            local_port,
            accept_abort: join.abort_handle(),
        });
        Ok(local_port)
    }

    pub async fn open_routed_stream(&self, raw_host: &str, port: u16) -> Result<TcpStream> {
        let translated_host = if let Ok(v4) = raw_host.parse::<Ipv4Addr>() {
            self.vip_to_host
                .get(&v4)
                .map(|e| e.value().clone())
                .unwrap_or_else(|| raw_host.to_string())
        } else {
            raw_host.to_string()
        };

        let snap = self.current_k8s_snapshot().await;
        if let Some(dest) = resolve_destination(
            &translated_host,
            port,
            &self.cfg,
            snap.as_ref(),
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
                    let socks_port = self.ensure_ssh_socks_port().await?;
                    connect_via_socks5(socks_port, &shost, sport).await
                }
            };
        }

        TcpStream::connect((translated_host.as_str(), port))
            .await
            .map_err(|e| Error::Upstream(format!("connect {translated_host}:{port}: {e}")))
    }
}

struct NetBridgeSocksAdapter(std::sync::Weak<NetBridgeInstance>);

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
