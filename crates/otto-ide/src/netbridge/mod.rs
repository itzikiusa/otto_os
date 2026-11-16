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
//! 2. **Authenticated SOCKS5 Loopback Proxy (`OTTO_NETBRIDGE_SOCKS5`)**:
//!    RFC 1929 (`otto:{token}`) loopback listener with `otto_netguard` SSRF defense.
//! 3. **Kubernetes Service Environment Variables**:
//!    populates `OTTO_K8S_NAMESPACE`, `POD_NAMESPACE`, optional
//!    `CONTAINERIZED=true`, and `<SVC>_SERVICE_HOST` / `<SVC>_SERVICE_PORT`.

pub mod interposer;
pub mod k8s_discover;
mod k8s_routing;
pub mod socks5;
pub mod tunnel;
mod tunnel_forward;
mod tunnel_rpc;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use dashmap::DashMap;
use otto_core::domain::User;
use otto_core::{Error, Id, Result};
use otto_ssh::{SshTunnel, SshTunnelConfig};
use sha2::{Digest, Sha256};

pub use interposer::wrap_shell_script_for_interposer;
pub use k8s_routing::{is_valid_dns1123_label, sanitize_network_bridge};

use crate::types::{
    IdeNetBridgeStatusResp, IdeNetBridgeTestReq, IdeNetworkBridgeConfig, IdeTargetKind,
    IdeTargetSpec,
};
use crate::IdeCtx;

const WARM_IDLE_TTL: Duration = Duration::from_secs(300);

/// Resolved Kubernetes cluster & namespace execution parameters supplied by
/// [`crate::IdeCtx::resolve_k8s_cluster`].
#[derive(Clone)]
pub struct IdeK8sSpec {
    pub cluster_id: Id,
    pub cluster_name: String,
    pub environment: Option<String>,
    pub namespace: String,
    pub allow_cross_namespace: bool,
    pub kubectl_program: String,
    /// Args including `--request-timeout` for fast JSON `get services/ingresses`.
    pub kubectl_query_args: Vec<String>,
    /// Args WITHOUT `--request-timeout` for long-running `port-forward`.
    pub kubectl_base_args: Vec<String>,
    pub kubectl_env: Vec<(String, String)>,
}

impl std::fmt::Debug for IdeK8sSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let env_keys: Vec<&str> = self.kubectl_env.iter().map(|(k, _)| k.as_str()).collect();
        f.debug_struct("IdeK8sSpec")
            .field("cluster_id", &self.cluster_id)
            .field("cluster_name", &self.cluster_name)
            .field("environment", &self.environment)
            .field("namespace", &self.namespace)
            .field("allow_cross_namespace", &self.allow_cross_namespace)
            .field("kubectl_program", &self.kubectl_program)
            .field("kubectl_query_args", &self.kubectl_query_args)
            .field("kubectl_base_args", &self.kubectl_base_args)
            .field("kubectl_env_keys", &env_keys)
            .finish()
    }
}

impl IdeK8sSpec {
    pub fn is_production(&self) -> bool {
        crate::types::is_production_label(self.environment.as_deref(), &self.cluster_name)
    }
}

pub const PROD_BRIDGE_CONFIRM_MSG: &str =
    "Production Network Bridge target requires confirm_prod_targets: true";

struct LeaseCounters {
    active_leases: usize,
    last_released: Instant,
}

struct PooledBridge {
    instance: Arc<tunnel::NetBridgeInstance>,
    counters: Arc<Mutex<LeaseCounters>>,
}

type PooledBridgeSlot = Arc<tokio::sync::Mutex<Option<PooledBridge>>>;
type BridgePoolMap = DashMap<(String, String), PooledBridgeSlot>;

fn evict_key_if_idle(pool: &BridgePoolMap, key: &(String, String), ttl: Duration) {
    let should_remove = if let Some(entry) = pool.get(key) {
        if let Ok(guard) = entry.value().try_lock() {
            guard.as_ref().is_some_and(|pb| {
                pb.counters
                    .lock()
                    .is_ok_and(|c| c.active_leases == 0 && c.last_released.elapsed() >= ttl)
            })
        } else {
            false
        }
    } else {
        false
    };
    if should_remove {
        pool.remove(key);
    }
}

/// Active reference-counted lease keeping a [`tunnel::NetBridgeInstance`] alive
/// for the duration of a local IDE Run or Debug session.
pub struct NetBridgeLease {
    instance: Arc<tunnel::NetBridgeInstance>,
    counters: Arc<Mutex<LeaseCounters>>,
    pool: Weak<BridgePoolMap>,
    pool_key: (String, String),
    idle_ttl: Duration,
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

    /// Create a weak dormant handle that does not hold an active lease count,
    /// allowing a terminated debug session to re-acquire the warm bridge on restart.
    pub fn dormant_handle(&self) -> DormantNetBridge {
        DormantNetBridge {
            instance: Arc::downgrade(&self.instance),
            counters: Arc::clone(&self.counters),
            pool: self.pool.clone(),
            pool_key: self.pool_key.clone(),
            idle_ttl: self.idle_ttl,
        }
    }
}

/// Weak handle to a warm [`tunnel::NetBridgeInstance`] retained across target exit
/// so `Restart` on a debug session can re-acquire the bridge lease without leaking
/// active tunnels while the session sits idle.
#[derive(Clone)]
pub struct DormantNetBridge {
    instance: Weak<tunnel::NetBridgeInstance>,
    counters: Arc<Mutex<LeaseCounters>>,
    pool: Weak<BridgePoolMap>,
    pool_key: (String, String),
    idle_ttl: Duration,
}

impl std::fmt::Debug for DormantNetBridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DormantNetBridge(..)")
    }
}

impl DormantNetBridge {
    pub async fn reacquire(&self) -> Option<Arc<NetBridgeLease>> {
        let instance = self.instance.upgrade()?;
        if !instance.is_healthy() {
            return None;
        }
        instance.ensure_host_mapping_listeners().await;
        if let Ok(mut c) = self.counters.lock() {
            c.active_leases += 1;
        }
        Some(Arc::new(NetBridgeLease {
            instance,
            counters: Arc::clone(&self.counters),
            pool: self.pool.clone(),
            pool_key: self.pool_key.clone(),
            idle_ttl: self.idle_ttl,
        }))
    }
}

impl Drop for NetBridgeLease {
    fn drop(&mut self) {
        let became_idle = if let Ok(mut c) = self.counters.lock() {
            c.active_leases = c.active_leases.saturating_sub(1);
            c.last_released = Instant::now();
            c.active_leases == 0
        } else {
            false
        };
        if became_idle {
            self.instance.stop_idle_listeners();
        }
        if became_idle && self.pool.strong_count() > 0 {
            let weak_pool = self.pool.clone();
            let key = self.pool_key.clone();
            let ttl = self.idle_ttl;
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(async move {
                    tokio::time::sleep(ttl).await;
                    if let Some(pool) = weak_pool.upgrade() {
                        evict_key_if_idle(&pool, &key, ttl);
                    }
                });
            }
        }
    }
}

/// Warm pool and status prober for IDE K8s + SSH network bridges.
#[derive(Clone)]
pub struct NetBridgeManager {
    data_dir: PathBuf,
    discovery_cache: Arc<k8s_discover::DiscoveryCache>,
    pool: Arc<BridgePoolMap>,
    idle_ttl: Duration,
}

impl NetBridgeManager {
    pub fn new(data_dir: PathBuf) -> Self {
        Self::with_idle_ttl(data_dir, WARM_IDLE_TTL)
    }

    #[doc(hidden)]
    pub fn with_idle_ttl(data_dir: PathBuf, idle_ttl: Duration) -> Self {
        Self {
            data_dir,
            discovery_cache: Arc::new(k8s_discover::DiscoveryCache::default()),
            pool: Arc::new(DashMap::new()),
            idle_ttl,
        }
    }

    /// Evict all pooled warm bridges owned by `user_id` (e.g. when their
    /// project `network_bridge` configuration changes or is disabled).
    pub fn evict_user(&self, user_id: &str) {
        let keys: Vec<(String, String)> = self
            .pool
            .iter()
            .filter(|e| e.key().0 == user_id)
            .map(|e| e.key().clone())
            .collect();
        for key in keys {
            self.pool.remove(&key);
        }
    }

    /// Tear down all pooled network bridge tunnels and port-forwards on daemon shutdown.
    pub async fn shutdown(&self) {
        let slots: Vec<PooledBridgeSlot> = self.pool.iter().map(|e| e.value().clone()).collect();
        self.pool.clear();
        for slot in slots {
            if let Some(pb) = slot.lock().await.take() {
                pb.instance.stop_idle_listeners();
            }
        }
    }

    /// Number of populated warm bridge slots currently cached for `user_id`.
    pub fn pooled_count_for_user(&self, user_id: &str) -> usize {
        self.pool
            .iter()
            .filter(|e| e.key().0 == user_id && e.value().try_lock().is_ok_and(|g| g.is_some()))
            .count()
    }

    /// Authorize `cfg` against `user` without starting listeners (called when
    /// saving environment configs so unauthorized K8s/SSH IDs are rejected).
    pub async fn authorize_config<S: IdeCtx>(
        &self,
        ctx: &S,
        user: &User,
        cfg: &IdeNetworkBridgeConfig,
    ) -> Result<()> {
        self.authorize_config_with_confirm(ctx, user, cfg, !cfg.enabled)
            .await
    }

    /// Authorize `cfg` against `user` and enforce `confirm_prod_targets` when
    /// `cfg.enabled` targets a production Kubernetes cluster or SSH bastion.
    pub async fn authorize_config_with_confirm<S: IdeCtx>(
        &self,
        ctx: &S,
        user: &User,
        cfg: &IdeNetworkBridgeConfig,
        confirm_prod_targets: bool,
    ) -> Result<()> {
        let mut targets_prod = false;
        if let Some(cid) = &cfg.k8s_cluster_id {
            let k8s = ctx
                .resolve_k8s_cluster(user, cid, cfg.k8s_namespace.as_deref())
                .await?;
            targets_prod |= k8s.is_production();
        }
        if let Some(sid) = &cfg.ssh_tunnel_connection_id {
            let ssh = ctx.resolve_ssh_connection(user, sid).await?;
            let _ = ssh.to_ssh_tunnel_config()?;
            targets_prod |= ssh.is_production();
        }
        if cfg.enabled && targets_prod && !confirm_prod_targets {
            return Err(Error::Invalid(PROD_BRIDGE_CONFIRM_MSG.into()));
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
        self.acquire_for_launch_with_confirm(ctx, user, target, raw_cfg, false)
            .await
    }

    /// Acquire (or reuse a warm) [`NetBridgeLease`] with explicit `confirm_prod_targets`.
    pub async fn acquire_for_launch_with_confirm<S: IdeCtx>(
        &self,
        ctx: &S,
        user: &User,
        target: &IdeTargetSpec,
        raw_cfg: &IdeNetworkBridgeConfig,
        confirm_prod_targets: bool,
    ) -> Result<Option<Arc<NetBridgeLease>>> {
        if target.kind != IdeTargetKind::Local || !raw_cfg.enabled {
            return Ok(None);
        }
        let Some(cfg) = sanitize_network_bridge(Some(raw_cfg.clone()))? else {
            return Ok(None);
        };

        // Re-authorize on every launch even when reusing a warm bridge.
        let mut targets_prod = false;
        let k8s_spec = if let Some(cid) = &cfg.k8s_cluster_id {
            let spec = ctx
                .resolve_k8s_cluster(user, cid, cfg.k8s_namespace.as_deref())
                .await?;
            targets_prod |= spec.is_production();
            Some(spec)
        } else {
            None
        };
        let ssh_cfg = if let Some(sid) = &cfg.ssh_tunnel_connection_id {
            let ssh_spec = ctx.resolve_ssh_connection(user, sid).await?;
            targets_prod |= ssh_spec.is_production();
            Some(ssh_spec.to_ssh_tunnel_config()?)
        } else {
            None
        };
        if targets_prod && !confirm_prod_targets {
            return Err(Error::Invalid(PROD_BRIDGE_CONFIRM_MSG.into()));
        }

        self.evict_expired_idle();

        let key = (
            user.id.to_string(),
            config_fingerprint(&cfg, k8s_spec.as_ref(), ssh_cfg.as_ref()),
        );
        let slot = self
            .pool
            .entry(key.clone())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(None)))
            .value()
            .clone();

        let mut guard = slot.lock().await;
        if let Some(existing) = guard.as_ref() {
            if existing.instance.is_healthy() {
                existing.instance.ensure_host_mapping_listeners().await;
                if let Ok(mut c) = existing.counters.lock() {
                    c.active_leases += 1;
                }
                return Ok(Some(Arc::new(NetBridgeLease {
                    instance: Arc::clone(&existing.instance),
                    counters: Arc::clone(&existing.counters),
                    pool: Arc::downgrade(&self.pool),
                    pool_key: key,
                    idle_ttl: self.idle_ttl,
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

        Ok(Some(Arc::new(NetBridgeLease {
            instance,
            counters,
            pool: Arc::downgrade(&self.pool),
            pool_key: key,
            idle_ttl: self.idle_ttl,
        })))
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
            let tcfg = ssh_spec.to_ssh_tunnel_config()?;
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
            evict_key_if_idle(&self.pool, &key, self.idle_ttl);
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
