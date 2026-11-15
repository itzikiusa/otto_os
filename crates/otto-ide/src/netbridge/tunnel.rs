//! Live per-environment network bridge instance: Unix control socket for the
//! POSIX `getaddrinfo`/`connect` interposer, on-demand `kubectl port-forward`
//! pool, multiplexed SSH SOCKS5 relays, fixed `localhost:<port>` listeners, and
//! RFC 1929 authenticated SOCKS5 loopback server.

use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU16, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use dashmap::DashMap;
use otto_core::{Error, Result};
use otto_ssh::{SshTunnel, SshTunnelConfig};
use tokio::net::{TcpListener, UnixListener};

use super::k8s_discover::{DiscoveryCache, K8sDiscoverySnapshot};
use super::socks5::{spawn_local_socks5_server, SocksUpstreamResolver};
use super::tunnel_forward::{K8sForwardKey, K8sForwardSlot, NetBridgeSocksAdapter, SshRelaySlot};
use super::IdeK8sSpec;
use crate::types::IdeNetworkBridgeConfig;

pub(crate) const MAX_VIP_ENTRIES: usize = 4096;
const MAX_INJECTED_K8S_ENV_SERVICES: usize = 128;

/// Map 1-based sequence `seq` into `127.88.<1..=254>.<1..=254>` using base-254
/// arithmetic so `.0` (network) and `.255` (broadcast) octets are never emitted.
pub(crate) fn vip_for_seq(seq: u32) -> Ipv4Addr {
    let idx = ((u64::from(seq).saturating_sub(1)) % (254 * 254)) as usize;
    let hi = ((idx / 254) + 1) as u8;
    let lo = ((idx % 254) + 1) as u8;
    Ipv4Addr::new(127, 88, hi, lo)
}

pub struct NetBridgeInstance {
    pub(super) cfg: IdeNetworkBridgeConfig,
    pub(super) k8s_spec: Option<IdeK8sSpec>,
    pub(super) ssh_cfg: Option<SshTunnelConfig>,
    pub(super) discovery_cache: Arc<DiscoveryCache>,
    pub(super) cached_k8s: tokio::sync::RwLock<Option<Arc<K8sDiscoverySnapshot>>>,
    pub(super) interposer_lib: Option<PathBuf>,
    pub(super) auth_token: String,
    pub(super) sock_path: PathBuf,
    _sock_dir: tempfile::TempDir,
    pub(super) socks_port: AtomicU16,
    pub(super) host_to_vip: DashMap<String, Ipv4Addr>,
    pub(super) vip_to_host: DashMap<Ipv4Addr, String>,
    next_vip_seq: AtomicU32,
    pub(super) k8s_forwards: DashMap<K8sForwardKey, K8sForwardSlot>,
    pub(super) ssh_tunnel: tokio::sync::Mutex<Option<Arc<SshTunnel>>>,
    pub(super) ssh_relays: DashMap<(String, u16), SshRelaySlot>,
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
        let auth_tok_arc: Arc<str> = Arc::from(inst.auth_token.as_str());
        let (socks_port, socks_abort) =
            spawn_local_socks5_server(resolver_arc, auth_tok_arc).await?;
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

    pub fn auth_token(&self) -> &str {
        &self.auth_token
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

    /// Inject `OTTO_NETBRIDGE_*`, `DYLD_INSERT_LIBRARIES` / `LD_PRELOAD`, and
    /// Kubernetes metadata / `*_SERVICE_HOST` variables into a Run or Debug
    /// child environment. Explicit user variables win.
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
                format!("socks5h://otto:{}@127.0.0.1:{socks_port}", self.auth_token),
            );
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

    pub(crate) fn allocate_vip(&self, raw_host: &str) -> Ipv4Addr {
        let host_lc = raw_host.trim().trim_end_matches('.').to_ascii_lowercase();
        if let Some(existing) = self.host_to_vip.get(&host_lc) {
            return *existing.value();
        }
        while self.host_to_vip.len() >= MAX_VIP_ENTRIES {
            let victim = self
                .host_to_vip
                .iter()
                .next()
                .map(|e| (e.key().clone(), *e.value()));
            let Some((old_host, old_vip)) = victim else {
                break;
            };
            self.host_to_vip.remove(&old_host);
            self.vip_to_host.remove(&old_vip);
        }
        let seq = self.next_vip_seq.fetch_add(1, Ordering::Relaxed);
        let vip = vip_for_seq(seq);
        if let Some((_, prev_host)) = self.vip_to_host.remove(&vip) {
            self.host_to_vip.remove(&prev_host);
        }
        self.vip_to_host.insert(vip, host_lc.clone());
        self.host_to_vip.insert(host_lc, vip);
        vip
    }

    #[cfg(test)]
    pub(crate) fn vip_count(&self) -> usize {
        self.host_to_vip.len()
    }

    pub(super) async fn current_k8s_snapshot(&self) -> Option<Arc<K8sDiscoverySnapshot>> {
        let spec = self.k8s_spec.as_ref()?;
        if let Ok(fresh) = self.discovery_cache.get_or_fetch(spec, false).await {
            let mut guard = self.cached_k8s.write().await;
            *guard = Some(Arc::clone(&fresh));
            return Some(fresh);
        }
        self.cached_k8s.read().await.clone()
    }
}
