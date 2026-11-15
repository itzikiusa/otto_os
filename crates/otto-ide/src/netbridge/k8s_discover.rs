//! Kubernetes `Service` + `Ingress` discovery and SWR snapshot cache for
//! [`super::NetBridgeManager`].

use std::collections::{BTreeMap, HashMap};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use otto_core::{Error, Result};
use serde_json::Value;
use tokio::process::Command;

pub use super::k8s_routing::{resolve_destination, should_intercept_dns};
use super::{is_valid_dns1123_label, IdeK8sSpec};
use crate::types::IdeBridgedServiceSummary;

const FRESH_TTL: Duration = Duration::from_secs(45);
const STALE_TTL: Duration = Duration::from_secs(600);
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(6);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct K8sServicePort {
    pub name: Option<String>,
    pub port: u16,
    pub target_port: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct K8sServiceEntry {
    pub name: String,
    pub namespace: String,
    pub cluster_ip: Option<String>,
    pub ports: Vec<K8sServicePort>,
    pub ingress_hosts: Vec<String>,
}

impl K8sServiceEntry {
    /// Choose the `Service` port to pass to `kubectl port-forward service/<name> 0:<port>`.
    pub fn resolve_service_port(&self, requested_port: u16) -> u16 {
        if self.ports.iter().any(|p| p.port == requested_port) {
            return requested_port;
        }
        if let Some(found) = self
            .ports
            .iter()
            .find(|p| p.target_port == Some(requested_port))
        {
            return found.port;
        }
        if matches!(requested_port, 0 | 80 | 443) && self.ports.len() == 1 {
            return self.ports[0].port;
        }
        if requested_port == 0 {
            return self.ports.first().map(|p| p.port).unwrap_or(80);
        }
        requested_port
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngressBackendTarget {
    pub service: String,
    pub port: Option<u16>,
    pub port_name: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct K8sDiscoverySnapshot {
    pub namespace: String,
    pub allow_cross_namespace: bool,
    pub services: BTreeMap<String, K8sServiceEntry>,
    pub cluster_ips: HashMap<String, String>,
    pub ingress_exact: HashMap<String, IngressBackendTarget>,
    pub ingress_wildcards: Vec<(String, IngressBackendTarget)>,
}

impl K8sDiscoverySnapshot {
    pub fn summaries(&self) -> (Vec<IdeBridgedServiceSummary>, Vec<String>) {
        let mut services = Vec::with_capacity(self.services.len());
        for svc in self.services.values() {
            let mut hosts = vec![
                svc.name.clone(),
                format!("{}.{}", svc.name, svc.namespace),
                format!("{}.{}.svc.cluster.local", svc.name, svc.namespace),
            ];
            hosts.extend(svc.ingress_hosts.iter().cloned());
            let mut ports: Vec<u16> = svc.ports.iter().map(|p| p.port).collect();
            ports.sort_unstable();
            ports.dedup();
            services.push(IdeBridgedServiceSummary {
                name: svc.name.clone(),
                namespace: svc.namespace.clone(),
                cluster_ip: svc.cluster_ip.clone(),
                ports,
                hosts,
            });
        }
        let mut ing_hosts: Vec<String> = self.ingress_exact.keys().cloned().collect();
        for (suffix, _) in &self.ingress_wildcards {
            ing_hosts.push(format!("*{suffix}"));
        }
        ing_hosts.sort();
        ing_hosts.dedup();
        (services, ing_hosts)
    }

    pub(super) fn resolve_ingress_target(
        &self,
        host_lc: &str,
        req_port: u16,
    ) -> Option<(String, u16)> {
        let backend = self.ingress_exact.get(host_lc).or_else(|| {
            self.ingress_wildcards
                .iter()
                .find(|(suffix, _)| {
                    host_lc.ends_with(suffix.as_str()) && host_lc.len() > suffix.len()
                })
                .map(|(_, b)| b)
        })?;
        if let Some(svc) = self.services.get(&backend.service) {
            if let Some(num) = backend.port {
                let port = if matches!(req_port, 0 | 80 | 443) || req_port == num {
                    svc.resolve_service_port(num)
                } else {
                    svc.resolve_service_port(req_port)
                };
                return Some((svc.name.clone(), port));
            }
            if let Some(pname) = backend.port_name.as_deref() {
                if let Some(sp) = svc.ports.iter().find(|p| p.name.as_deref() == Some(pname)) {
                    return Some((svc.name.clone(), sp.port));
                }
            }
            return Some((svc.name.clone(), svc.resolve_service_port(req_port)));
        }
        Some((
            backend.service.clone(),
            backend.port.unwrap_or(req_port.max(80)),
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum BridgeDestination {
    K8sService {
        namespace: String,
        service: String,
        port: u16,
    },
    SshHost {
        host: String,
        port: u16,
    },
}

type CacheKey = (String, String);

struct BgRefreshGuard {
    map: Arc<DashMap<CacheKey, ()>>,
    key: CacheKey,
}

impl Drop for BgRefreshGuard {
    fn drop(&mut self) {
        self.map.remove(&self.key);
    }
}

#[derive(Clone, Default)]
pub struct DiscoveryCache {
    entries: Arc<DashMap<CacheKey, (Instant, Arc<K8sDiscoverySnapshot>)>>,
    inflight: Arc<DashMap<CacheKey, Arc<tokio::sync::Mutex<()>>>>,
    bg_refreshing: Arc<DashMap<CacheKey, ()>>,
}

impl DiscoveryCache {
    pub async fn get_or_fetch(
        &self,
        spec: &IdeK8sSpec,
        force_refresh: bool,
    ) -> Result<Arc<K8sDiscoverySnapshot>> {
        let key = (spec.cluster_id.to_string(), spec.namespace.clone());
        if !force_refresh {
            let cached = self
                .entries
                .get(&key)
                .map(|hit| (hit.value().0.elapsed(), Arc::clone(&hit.value().1)));
            if let Some((age, snap)) = cached {
                if age < FRESH_TTL {
                    return Ok(snap);
                }
                if age < STALE_TTL {
                    self.spawn_background_refresh(key, spec);
                    return Ok(snap);
                }
            }
        }

        let lock = self
            .inflight
            .entry(key.clone())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .value()
            .clone();
        let _lk = lock.lock().await;
        if !force_refresh {
            if let Some(hit) = self.entries.get(&key) {
                if hit.value().0.elapsed() < FRESH_TTL {
                    return Ok(Arc::clone(&hit.value().1));
                }
            }
        }
        let snap = Arc::new(fetch_k8s_discovery(spec).await?);
        self.entries
            .insert(key, (Instant::now(), Arc::clone(&snap)));
        Ok(snap)
    }

    fn spawn_background_refresh(&self, key: CacheKey, spec: &IdeK8sSpec) {
        if self.bg_refreshing.insert(key.clone(), ()).is_some() {
            return;
        }
        let cache = self.clone();
        let spec_bg = spec.clone();
        tokio::spawn(async move {
            let _bg_guard = BgRefreshGuard {
                map: Arc::clone(&cache.bg_refreshing),
                key: key.clone(),
            };
            let lock = cache
                .inflight
                .entry(key.clone())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .value()
                .clone();
            let _lk = lock.lock().await;
            if let Ok(fresh) = fetch_k8s_discovery(&spec_bg).await {
                cache.entries.insert(key, (Instant::now(), Arc::new(fresh)));
            }
        });
    }

    #[cfg(test)]
    pub(crate) fn insert_with_age_for_test(
        &self,
        spec: &IdeK8sSpec,
        snap: K8sDiscoverySnapshot,
        age: Duration,
    ) {
        let key = (spec.cluster_id.to_string(), spec.namespace.clone());
        let ts = Instant::now().checked_sub(age).unwrap_or_else(Instant::now);
        self.entries.insert(key, (ts, Arc::new(snap)));
    }
}

async fn run_kubectl_json(spec: &IdeK8sSpec, resource: &str) -> Result<Value> {
    let mut args = spec.kubectl_query_args.clone();
    args.extend([
        "-n".to_string(),
        spec.namespace.clone(),
        "get".to_string(),
        resource.to_string(),
        "-o".to_string(),
        "json".to_string(),
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

    let child = cmd
        .spawn()
        .map_err(|e| Error::Invalid(format!("failed to run '{}': {e}", spec.kubectl_program)))?;
    let out = tokio::time::timeout(DISCOVERY_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| Error::Upstream(format!("kubectl get {resource} timed out")))?
        .map_err(|e| Error::Upstream(format!("kubectl get {resource}: {e}")))?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let first = stderr.lines().next().unwrap_or("kubectl failed").trim();
        if stderr.to_ascii_lowercase().contains("forbidden") {
            return Err(Error::Forbidden(format!("cluster RBAC: {first}")));
        }
        return Err(Error::Upstream(format!("kubectl get {resource}: {first}")));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    if stdout.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(stdout.trim())
        .map_err(|e| Error::Upstream(format!("invalid kubectl {resource} JSON: {e}")))
}

async fn fetch_k8s_discovery(spec: &IdeK8sSpec) -> Result<K8sDiscoverySnapshot> {
    let (svc_res, ing_res) = tokio::join!(
        run_kubectl_json(spec, "services"),
        run_kubectl_json(spec, "ingresses"),
    );
    let svc_val = svc_res?;
    let ing_val = ing_res.unwrap_or(Value::Null);
    let mut snap = parse_k8s_discovery_json(&spec.namespace, &svc_val, &ing_val);
    snap.allow_cross_namespace = spec.allow_cross_namespace;
    Ok(snap)
}

pub fn parse_k8s_discovery_json(
    namespace: &str,
    svc_json: &Value,
    ing_json: &Value,
) -> K8sDiscoverySnapshot {
    let mut snap = K8sDiscoverySnapshot {
        namespace: namespace.to_string(),
        ..Default::default()
    };
    if let Some(items) = svc_json.get("items").and_then(Value::as_array) {
        for item in items {
            let Some(name) = item
                .pointer("/metadata/name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|n| is_valid_dns1123_label(n))
            else {
                continue;
            };
            let cluster_ip = item
                .pointer("/spec/clusterIP")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|ip| !ip.is_empty() && !ip.eq_ignore_ascii_case("none"))
                .filter(|ip| ip.parse::<std::net::Ipv4Addr>().is_ok())
                .map(str::to_string);
            let mut ports = Vec::new();
            if let Some(parr) = item.pointer("/spec/ports").and_then(Value::as_array) {
                for p in parr {
                    let proto = p.get("protocol").and_then(Value::as_str).unwrap_or("TCP");
                    if !proto.eq_ignore_ascii_case("TCP") {
                        continue;
                    }
                    let Some(port_u64) = p.get("port").and_then(Value::as_u64) else {
                        continue;
                    };
                    let Ok(port) = u16::try_from(port_u64) else {
                        continue;
                    };
                    if port == 0 {
                        continue;
                    }
                    let target_port = p
                        .get("targetPort")
                        .and_then(Value::as_u64)
                        .and_then(|tp| u16::try_from(tp).ok())
                        .filter(|&tp| tp > 0);
                    let pname = p
                        .get("name")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string);
                    ports.push(K8sServicePort {
                        name: pname,
                        port,
                        target_port,
                    });
                }
            }
            if let Some(ip) = &cluster_ip {
                snap.cluster_ips.insert(ip.clone(), name.to_string());
            }
            snap.services.insert(
                name.to_string(),
                K8sServiceEntry {
                    name: name.to_string(),
                    namespace: namespace.to_string(),
                    cluster_ip,
                    ports,
                    ingress_hosts: Vec::new(),
                },
            );
        }
    }

    if let Some(items) = ing_json.get("items").and_then(Value::as_array) {
        for item in items {
            let Some(rules) = item.pointer("/spec/rules").and_then(Value::as_array) else {
                continue;
            };
            for rule in rules {
                let Some(raw_host) = rule
                    .get("host")
                    .and_then(Value::as_str)
                    .map(|h| h.trim().trim_end_matches('.').to_ascii_lowercase())
                    .filter(|h| !h.is_empty())
                else {
                    continue;
                };
                let Some(paths) = rule.pointer("/http/paths").and_then(Value::as_array) else {
                    continue;
                };
                for path_item in paths {
                    let Some(svc_name) = path_item
                        .pointer("/backend/service/name")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|n| is_valid_dns1123_label(n))
                    else {
                        continue;
                    };
                    let port_num = path_item
                        .pointer("/backend/service/port/number")
                        .and_then(Value::as_u64)
                        .and_then(|p| u16::try_from(p).ok())
                        .filter(|&p| p > 0);
                    let port_name = path_item
                        .pointer("/backend/service/port/name")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string);
                    let target = IngressBackendTarget {
                        service: svc_name.to_string(),
                        port: port_num,
                        port_name,
                    };
                    if let Some(svc) = snap.services.get_mut(svc_name) {
                        if !svc.ingress_hosts.contains(&raw_host) {
                            svc.ingress_hosts.push(raw_host.clone());
                        }
                    }
                    if let Some(suffix) = raw_host.strip_prefix('*') {
                        if suffix.starts_with('.') && suffix.len() > 2 {
                            snap.ingress_wildcards.push((suffix.to_string(), target));
                        }
                    } else {
                        snap.ingress_exact.entry(raw_host.clone()).or_insert(target);
                    }
                    break;
                }
            }
        }
    }
    snap
}
