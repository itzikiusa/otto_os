//! Smart virtual-host, Kubernetes DNS/ClusterIP/Ingress, and SSH internal-domain
//! routing rules for [`super::NetBridgeManager`].

use otto_core::{Error, Id, Result};

use super::k8s_discover::{BridgeDestination, K8sDiscoverySnapshot};
use crate::types::{IdeHostMapping, IdeNetworkBridgeConfig};

const MAX_HOST_MAPPINGS: usize = 128;

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

fn match_explicit_mapping<'a>(
    host_lc: &str,
    port: u16,
    mappings: &'a [IdeHostMapping],
) -> Option<&'a IdeHostMapping> {
    mappings.iter().find(|m| {
        m.from_host
            .trim()
            .trim_end_matches('.')
            .eq_ignore_ascii_case(host_lc)
            && (port == 0 || m.from_port.is_none() || m.from_port == Some(port))
    })
}

fn is_mdns_machine_local(host_lc: &str) -> bool {
    host_lc.ends_with(".local") && host_lc.matches('.').count() == 1
}

fn is_cloud_managed_host(host_lc: &str) -> bool {
    host_lc.ends_with(".amazonaws.com")
        || host_lc.contains(".rds.amazonaws.com")
        || host_lc.contains(".cache.amazonaws.com")
        || host_lc.ends_with(".azure.com")
        || host_lc.ends_with(".database.windows.net")
        || host_lc.ends_with(".googleapis.com")
}

fn is_internal_domain_host(host_lc: &str) -> bool {
    if is_mdns_machine_local(host_lc) {
        return false;
    }
    !host_lc.contains('.')
        || host_lc.ends_with(".local")
        || host_lc.ends_with(".internal")
        || host_lc.ends_with(".svc")
        || host_lc.ends_with(".cluster.local")
        || host_lc.ends_with(".lan")
        || host_lc.ends_with(".corp")
        || host_lc.contains(".rds.amazonaws.com")
        || host_lc.contains(".cache.amazonaws.com")
        || host_lc.contains(".kafka.")
}

pub fn should_intercept_dns(
    raw_host: &str,
    cfg: &IdeNetworkBridgeConfig,
    k8s: Option<&K8sDiscoverySnapshot>,
    has_ssh_tunnel: bool,
) -> bool {
    let host_lc = raw_host.trim().trim_end_matches('.').to_ascii_lowercase();
    if host_lc.is_empty()
        || matches!(
            host_lc.as_str(),
            "localhost" | "127.0.0.1" | "::1" | "0.0.0.0" | "::"
        )
        || otto_netguard::is_metadata_hostname(&host_lc)
    {
        return false;
    }
    resolve_destination(&host_lc, 0, cfg, k8s, has_ssh_tunnel).is_some()
}

pub fn resolve_destination(
    raw_host: &str,
    req_port: u16,
    cfg: &IdeNetworkBridgeConfig,
    k8s: Option<&K8sDiscoverySnapshot>,
    has_ssh_tunnel: bool,
) -> Option<BridgeDestination> {
    let host_lc = raw_host.trim().trim_end_matches('.').to_ascii_lowercase();
    if host_lc.is_empty() || otto_netguard::is_metadata_hostname(&host_lc) {
        return None;
    }

    if let Some(mapped) = match_explicit_mapping(&host_lc, req_port, &cfg.host_mappings) {
        let target_port = mapped.to_port.unwrap_or_else(|| req_port.max(80));
        let raw_to = mapped.to_host.trim().trim_end_matches('.');
        let svc_ref = raw_to.strip_prefix("service/").unwrap_or(raw_to);
        if let Some(snap) = k8s {
            if let Some((svc, ns)) = parse_k8s_svc_and_ns(svc_ref, &snap.namespace) {
                if ns == snap.namespace {
                    if let Some(entry) = snap.services.get(&svc) {
                        return Some(BridgeDestination::K8sService {
                            namespace: ns,
                            service: svc,
                            port: entry.resolve_service_port(target_port),
                        });
                    }
                }
                if (ns == snap.namespace || snap.allow_cross_namespace)
                    && (raw_to.starts_with("service/")
                        || !svc_ref.contains('.')
                        || svc_ref.ends_with(".svc")
                        || svc_ref.ends_with(".cluster.local"))
                {
                    return Some(BridgeDestination::K8sService {
                        namespace: ns,
                        service: svc,
                        port: target_port,
                    });
                }
            }
        }
        if has_ssh_tunnel {
            return Some(BridgeDestination::SshHost {
                host: raw_to.to_string(),
                port: target_port,
            });
        }
        if let Some(snap) = k8s {
            if is_valid_dns1123_label(svc_ref) {
                return Some(BridgeDestination::K8sService {
                    namespace: snap.namespace.clone(),
                    service: svc_ref.to_ascii_lowercase(),
                    port: target_port,
                });
            }
        }
    }

    if matches!(
        host_lc.as_str(),
        "localhost" | "127.0.0.1" | "::1" | "0.0.0.0" | "::"
    ) || is_mdns_machine_local(&host_lc)
    {
        return None;
    }

    if let Some(snap) = k8s {
        if let Some(svc_name) = snap.cluster_ips.get(&host_lc) {
            if let Some(entry) = snap.services.get(svc_name) {
                return Some(BridgeDestination::K8sService {
                    namespace: snap.namespace.clone(),
                    service: entry.name.clone(),
                    port: entry.resolve_service_port(req_port),
                });
            }
        }
        if let Some((svc_name, port)) = snap.resolve_ingress_target(&host_lc, req_port) {
            return Some(BridgeDestination::K8sService {
                namespace: snap.namespace.clone(),
                service: svc_name,
                port,
            });
        }
        if let Some((svc_name, target_ns)) = parse_k8s_svc_and_ns(&host_lc, &snap.namespace) {
            if target_ns == snap.namespace {
                if let Some(entry) = snap.services.get(&svc_name) {
                    return Some(BridgeDestination::K8sService {
                        namespace: target_ns,
                        service: entry.name.clone(),
                        port: entry.resolve_service_port(req_port),
                    });
                }
                if !host_lc.contains('.') {
                    if !snap.loaded && !has_ssh_tunnel {
                        return Some(BridgeDestination::K8sService {
                            namespace: target_ns,
                            service: svc_name,
                            port: req_port.max(80),
                        });
                    }
                } else if host_lc.ends_with(".svc") || host_lc.ends_with(".cluster.local") {
                    return Some(BridgeDestination::K8sService {
                        namespace: target_ns,
                        service: svc_name,
                        port: req_port.max(80),
                    });
                }
            } else if snap.allow_cross_namespace
                && (host_lc.ends_with(".svc") || host_lc.ends_with(".cluster.local"))
            {
                return Some(BridgeDestination::K8sService {
                    namespace: target_ns,
                    service: svc_name,
                    port: req_port.max(80),
                });
            }
        }
        // Smart `.local` / `.internal` prefix match: e.g. `confsrv.stgco.local` -> `service/confsrv`.
        // Exclude cloud-managed DB/cache suffixes (`.amazonaws.com`, `.azure.com`, `.googleapis.com`)
        // so e.g. `redis.prod.use1.cache.amazonaws.com` routes via SSH tunnel rather than colliding
        // with an in-cluster `redis` Service.
        if is_internal_domain_host(&host_lc) && !is_cloud_managed_host(&host_lc) {
            if let Some(first_label) = host_lc.split('.').next() {
                if let Some(entry) = snap.services.get(first_label) {
                    return Some(BridgeDestination::K8sService {
                        namespace: snap.namespace.clone(),
                        service: entry.name.clone(),
                        port: entry.resolve_service_port(req_port),
                    });
                }
            }
        }
    }

    if has_ssh_tunnel && is_internal_domain_host(&host_lc) {
        return Some(BridgeDestination::SshHost {
            host: host_lc,
            port: req_port.max(80),
        });
    }

    None
}

fn parse_k8s_svc_and_ns(host_lc: &str, default_ns: &str) -> Option<(String, String)> {
    let parts: Vec<&str> = host_lc.split('.').collect();
    match parts.as_slice() {
        [svc] if is_valid_dns1123_label(svc) && *svc != "localhost" => {
            Some(((*svc).to_string(), default_ns.to_string()))
        }
        [svc, ns]
            if is_valid_dns1123_label(svc) && is_valid_dns1123_label(ns) && *ns != "local" =>
        {
            Some(((*svc).to_string(), (*ns).to_string()))
        }
        [svc, ns, "svc"] if is_valid_dns1123_label(svc) && is_valid_dns1123_label(ns) => {
            Some(((*svc).to_string(), (*ns).to_string()))
        }
        [svc, ns, "svc", "cluster", "local"]
            if is_valid_dns1123_label(svc) && is_valid_dns1123_label(ns) =>
        {
            Some(((*svc).to_string(), (*ns).to_string()))
        }
        _ => None,
    }
}
