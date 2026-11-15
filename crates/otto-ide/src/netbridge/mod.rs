//! IDE Environment Network Bridge (`crates/otto-ide/src/netbridge`).
//!
//! Attaches a Local IDE Run or Debug environment to a **Kubernetes Cluster +
//! Namespace** (`kubectl` service/ingress discovery + on-demand pooled
//! `port-forward`) and/or an **SSH Bastion Tunnel** (`otto_ssh::SshTunnel`
//! dynamic SOCKS5), with zero `sudo` or `/etc/hosts` modifications:
//!
//! 1. **Native POSIX Interposer (`DYLD_INSERT_LIBRARIES` / `LD_PRELOAD`)**:
//!    intercepts `getaddrinfo` (allocating `127.88.x.y` synthetic VIPs with
//!    instant `AF_INET6` short-circuiting so macOS `.local` mDNS never stalls)
//!    and `connect` (rewriting `127.88.x.y` and ClusterIPs to `127.0.0.1:<pf>`).
//! 2. **JVM / SOCKS5 Loopback Proxy (`JAVA_TOOL_OPTIONS`)**:
//!    covers Apple `/usr/bin/java` SIP-stub launches transparently.
//! 3. **Kubernetes Service Environment Variables**:
//!    populates `OTTO_K8S_NAMESPACE`, `POD_NAMESPACE`, optional
//!    `CONTAINERIZED=true`, and `<SVC>_SERVICE_HOST` / `<SVC>_SERVICE_PORT`.

pub mod interposer;
pub mod k8s_discover;
pub mod socks5;
pub mod tunnel;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dashmap::DashMap;
use otto_core::domain::User;
use otto_core::{Error, Id, Result};
use otto_ssh::{SshTunnel, SshTunnelConfig};
use sha2::{Digest, Sha256};

pub use interposer::wrap_shell_script_for_interposer;

use crate::types::{
    IdeHostMapping, IdeNetBridgeStatusResp, IdeNetBridgeTestReq, IdeNetworkBridgeConfig,
    IdeSshSpec, IdeTargetKind, IdeTargetSpec,
};
use crate::IdeCtx;

const WARM_IDLE_TTL: Duration = Duration::from_secs(300);
const MAX_HOST_MAPPINGS: usize = 128;

/// Resolved Kubernetes cluster & namespace execution parameters supplied by
/// [`crate::IdeCtx::resolve_k8s_cluster`].
#[derive(Debug, Clone)]
pub struct IdeK8sSpec {
    pub cluster_id: Id,
    pub cluster_name: String,
    pub namespace: String,
    pub allow_cross_namespace: bool,
    pub kubectl_program: String,
    /// Args including `--request-timeout` for fast JSON `get services/ingresses`.
    pub kubectl_query_args: Vec<String>,
    /// Args WITHOUT `--request-timeout` for long-running `port-forward`.
    pub kubectl_base_args: Vec<String>,
    pub kubectl_env: Vec<(String, String)>,
}

pub fn is_valid_dns1123_label(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 63
        && !s.starts_with('-')
        && !s.ends_with('-')
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn is_valid_bridge_host(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 253
        && !s.starts_with('-')
        && !s.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// Validate and normalize an optional [`IdeNetworkBridgeConfig`].
pub fn sanitize_network_bridge(
    raw: Option<IdeNetworkBridgeConfig>,
) -> Result<Option<IdeNetworkBridgeConfig>> {
    let Some(mut cfg) = raw else {
        return Ok(None);
    };
    cfg.k8s_cluster_id = cfg
        .k8s_cluster_id
        .map(|s| Id::from(s.as_str().trim()))
        .filter(|s| !s.as_str().is_empty());
    cfg.k8s_namespace = cfg
        .k8s_namespace
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty());
    if let Some(ns) = cfg.k8s_namespace.as_deref() {
        if !is_valid_dns1123_label(ns) {
            return Err(Error::Invalid(format!(
                "invalid Kubernetes namespace '{ns}' (must be a lowercase DNS-1123 label)"
            )));
        }
    }
    cfg.ssh_tunnel_connection_id = cfg
        .ssh_tunnel_connection_id
        .map(|s| Id::from(s.as_str().trim()))
        .filter(|s| !s.as_str().is_empty());

    if cfg.host_mappings.len() > MAX_HOST_MAPPINGS {
        return Err(Error::Invalid(format!(
            "at most {MAX_HOST_MAPPINGS} host mappings are allowed"
        )));
    }
    let mut cleaned_mappings = Vec::with_capacity(cfg.host_mappings.len());
    for m in cfg.host_mappings {
        let from_host = m
            .from_host
            .trim()
            .trim_end_matches('.')
            .to_ascii_lowercase();
        let to_host = m.to_host.trim().trim_end_matches('.').to_string();
        if from_host.is_empty() && to_host.is_empty() {
            continue;
        }
        if !is_valid_bridge_host(&from_host) || !is_valid_bridge_host(&to_host) {
            return Err(Error::Invalid(format!(
                "invalid host mapping '{from_host}' -> '{to_host}'"
            )));
        }
        cleaned_mappings.push(IdeHostMapping {
            from_host,
            from_port: m.from_port.filter(|&p| p > 0),
            to_host,
            to_port: m.to_port.filter(|&p| p > 0),
        });
    }
    cfg.host_mappings = cleaned_mappings;

    if !cfg.has_any_content() {
        return Ok(None);
    }
    if cfg.enabled
        && cfg.k8s_cluster_id.is_none()
        && cfg.ssh_tunnel_connection_id.is_none()
        && cfg.host_mappings.is_empty()
    {
        return Err(Error::Invalid(
            "Network bridge is enabled: select a Kubernetes cluster, an SSH tunnel, or add a host mapping."
                .into(),
        ));
    }
    Ok(Some(cfg))
}

fn ssh_spec_to_tunnel_cfg(spec: IdeSshSpec) -> SshTunnelConfig {
    let port = spec.effective_port().unwrap_or(22);
    SshTunnelConfig {
        host: spec.host,
        port,
        user: spec.user,
        identity_file: spec.identity_file,
        proxy_jump: spec.proxy_jump,
    }
}

struct LeaseCounters {
    active_leases: usize,
    last_released: Instant,
}

struct PooledBridge {
    instance: Arc<tunnel::NetBridgeInstance>,
    counters: Arc<Mutex<LeaseCounters>>,
}

/// Active reference-counted lease keeping a [`tunnel::NetBridgeInstance`] alive
/// for the duration of a local IDE Run or Debug session.
pub struct NetBridgeLease {
    instance: Arc<tunnel::NetBridgeInstance>,
    counters: Arc<Mutex<LeaseCounters>>,
}

impl std::fmt::Debug for NetBridgeLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NetBridgeLease(..)")
    }
}

impl NetBridgeLease {
    pub fn apply_to_env(&self, env: &mut BTreeMap<String, String>) {
        self.instance.apply_to_env(env);
    }

    pub async fn summary_banner(&self) -> String {
        self.instance.summary_banner().await
    }
}

impl Drop for NetBridgeLease {
    fn drop(&mut self) {
        if let Ok(mut c) = self.counters.lock() {
            c.active_leases = c.active_leases.saturating_sub(1);
            c.last_released = Instant::now();
        }
    }
}

type PooledBridgeSlot = Arc<tokio::sync::Mutex<Option<PooledBridge>>>;
type BridgePoolMap = DashMap<(String, String), PooledBridgeSlot>;

/// Warm pool and status prober for IDE K8s + SSH network bridges.
#[derive(Clone)]
pub struct NetBridgeManager {
    data_dir: PathBuf,
    discovery_cache: Arc<k8s_discover::DiscoveryCache>,
    pool: Arc<BridgePoolMap>,
}

impl NetBridgeManager {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            discovery_cache: Arc::new(k8s_discover::DiscoveryCache::default()),
            pool: Arc::new(DashMap::new()),
        }
    }

    /// Authorize `cfg` against `user` without starting listeners (called when
    /// saving environment configs so unauthorized K8s/SSH IDs are rejected).
    pub async fn authorize_config<S: IdeCtx>(
        &self,
        ctx: &S,
        user: &User,
        cfg: &IdeNetworkBridgeConfig,
    ) -> Result<()> {
        if let Some(cid) = &cfg.k8s_cluster_id {
            let _ = ctx
                .resolve_k8s_cluster(user, cid, cfg.k8s_namespace.as_deref())
                .await?;
        }
        if let Some(sid) = &cfg.ssh_tunnel_connection_id {
            let _ = ctx.resolve_ssh_connection(user, sid).await?;
        }
        Ok(())
    }

    /// Acquire (or reuse a warm) [`NetBridgeLease`] for a local Run or Debug launch.
    pub async fn acquire_for_launch<S: IdeCtx>(
        &self,
        ctx: &S,
        user: &User,
        target: &IdeTargetSpec,
        raw_cfg: &IdeNetworkBridgeConfig,
    ) -> Result<Option<Arc<NetBridgeLease>>> {
        if target.kind != IdeTargetKind::Local || !raw_cfg.enabled {
            return Ok(None);
        }
        let Some(cfg) = sanitize_network_bridge(Some(raw_cfg.clone()))? else {
            return Ok(None);
        };

        // Re-authorize on every launch even when reusing a warm bridge.
        let k8s_spec = if let Some(cid) = &cfg.k8s_cluster_id {
            Some(
                ctx.resolve_k8s_cluster(user, cid, cfg.k8s_namespace.as_deref())
                    .await?,
            )
        } else {
            None
        };
        let ssh_cfg = if let Some(sid) = &cfg.ssh_tunnel_connection_id {
            Some(ssh_spec_to_tunnel_cfg(
                ctx.resolve_ssh_connection(user, sid).await?,
            ))
        } else {
            None
        };

        self.evict_expired_idle();

        let key = (
            user.id.to_string(),
            config_fingerprint(&cfg, k8s_spec.as_ref(), ssh_cfg.as_ref()),
        );
        let slot = self
            .pool
            .entry(key)
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(None)))
            .value()
            .clone();

        let mut guard = slot.lock().await;
        if let Some(existing) = guard.as_ref() {
            if existing.instance.is_healthy() {
                if let Ok(mut c) = existing.counters.lock() {
                    c.active_leases += 1;
                }
                return Ok(Some(Arc::new(NetBridgeLease {
                    instance: Arc::clone(&existing.instance),
                    counters: Arc::clone(&existing.counters),
                })));
            }
        }

        let instance = tunnel::NetBridgeInstance::start(
            &self.data_dir,
            cfg,
            k8s_spec,
            ssh_cfg,
            Arc::clone(&self.discovery_cache),
        )
        .await?;
        let counters = Arc::new(Mutex::new(LeaseCounters {
            active_leases: 1,
            last_released: Instant::now(),
        }));
        *guard = Some(PooledBridge {
            instance: Arc::clone(&instance),
            counters: Arc::clone(&counters),
        });

        Ok(Some(Arc::new(NetBridgeLease { instance, counters })))
    }

    /// Test connectivity & discover Kubernetes `Service` + `Ingress` endpoints
    /// for the "Test & Discover Services" button in the IDE UI.
    pub async fn probe_status<S: IdeCtx>(
        &self,
        ctx: &S,
        user: &User,
        req: &IdeNetBridgeTestReq,
    ) -> Result<IdeNetBridgeStatusResp> {
        let started = Instant::now();
        let mut probe_cfg = req.bridge.clone();
        probe_cfg.enabled = true;
        let cfg = sanitize_network_bridge(Some(probe_cfg))?.ok_or_else(|| {
            Error::Invalid(
                "Select a Kubernetes cluster, an SSH tunnel, or add a host mapping to test.".into(),
            )
        })?;

        let mut cluster_name = None;
        let mut namespace = None;
        let mut services = Vec::new();
        let mut ingress_hosts = Vec::new();

        if let Some(cid) = &cfg.k8s_cluster_id {
            let spec = ctx
                .resolve_k8s_cluster(user, cid, cfg.k8s_namespace.as_deref())
                .await?;
            cluster_name = Some(spec.cluster_name.clone());
            namespace = Some(spec.namespace.clone());
            let snap = self.discovery_cache.get_or_fetch(&spec, true).await?;
            let (svcs, ings) = snap.summaries();
            services = svcs;
            ingress_hosts = ings;
        }

        let mut ssh_tunnel_ready = false;
        if let Some(sid) = &cfg.ssh_tunnel_connection_id {
            let ssh_spec = ctx.resolve_ssh_connection(user, sid).await?;
            let tcfg = ssh_spec_to_tunnel_cfg(ssh_spec);
            let tunnel = SshTunnel::open_socks(&tcfg).await?;
            ssh_tunnel_ready = tunnel.is_alive();
        }

        let interposer_ready = interposer::ensure_interposer_lib(&self.data_dir)
            .await?
            .is_some();

        let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let mut msg_parts = Vec::new();
        if let (Some(cn), Some(ns)) = (&cluster_name, &namespace) {
            msg_parts.push(format!(
                "Connected to {cn}/{ns}: {} services, {} ingress hosts",
                services.len(),
                ingress_hosts.len()
            ));
        }
        if cfg.ssh_tunnel_connection_id.is_some() {
            msg_parts.push(if ssh_tunnel_ready {
                "SSH SOCKS5 bastion tunnel verified".to_string()
            } else {
                "SSH tunnel did not stay alive".to_string()
            });
        }
        if !cfg.host_mappings.is_empty() {
            msg_parts.push(format!("{} custom host mappings", cfg.host_mappings.len()));
        }

        Ok(IdeNetBridgeStatusResp {
            ok: true,
            cluster_name,
            namespace,
            ssh_tunnel_ready,
            interposer_ready,
            services,
            ingress_hosts,
            latency_ms,
            message: msg_parts.join(" · "),
        })
    }

    fn evict_expired_idle(&self) {
        let keys: Vec<(String, String)> = self.pool.iter().map(|e| e.key().clone()).collect();
        for key in keys {
            let should_remove = if let Some(entry) = self.pool.get(&key) {
                if let Ok(guard) = entry.value().try_lock() {
                    guard.as_ref().is_some_and(|pb| {
                        pb.counters.lock().is_ok_and(|c| {
                            c.active_leases == 0 && c.last_released.elapsed() > WARM_IDLE_TTL
                        })
                    })
                } else {
                    false
                }
            } else {
                false
            };
            if should_remove {
                self.pool.remove(&key);
            }
        }
    }
}

fn config_fingerprint(
    cfg: &IdeNetworkBridgeConfig,
    k8s: Option<&IdeK8sSpec>,
    ssh: Option<&SshTunnelConfig>,
) -> String {
    let mut hasher = Sha256::new();
    if let Ok(bytes) = serde_json::to_vec(cfg) {
        hasher.update(&bytes);
    }
    if let Some(k) = k8s {
        hasher.update(k.cluster_id.as_bytes());
        hasher.update(b":");
        hasher.update(k.namespace.as_bytes());
        hasher.update(if k.allow_cross_namespace {
            b":1"
        } else {
            b":0"
        });
    }
    if let Some(s) = ssh {
        hasher.update(s.host.as_bytes());
        hasher.update(s.port.to_be_bytes());
        hasher.update(s.user.as_bytes());
    }
    hex::encode(hasher.finalize())[..24].to_string()
}
