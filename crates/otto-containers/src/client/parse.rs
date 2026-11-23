//! JSON-to-DTO parsers for Docker Engine API v1.47 payloads and Kiri VM state.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::types::{
    ComposeStackSummary, ContainerDetail, ContainerStatsResp, ContainerSummary,
    ContainersStatusResp, EngineKind, ImageInspectResp, ImageSummary, MountInfo, NetworkAttachment,
    NetworkContainerRef, NetworkSummary, PortMapping, VmResources, VolumeSummary,
};

use super::secrets::parse_env_entries;

/// Single authoritative classifier for Kiri VM state from Docker `/info` JSON.
/// Shared by both `DockerClient::status` and `runtime::runtime_status`.
pub(crate) fn classify_kiri_vm_state(info: &Value) -> &'static str {
    let stopped = info
        .get("KiriStopped")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if stopped {
        return "stopped";
    }
    let awake = info
        .get("KiriVmAwake")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    if awake {
        "running"
    } else {
        "asleep"
    }
}

pub(super) fn offline_status(
    context_name: &str,
    socket_path: String,
    engine_kind: EngineKind,
    docker_cli_available: bool,
    docker_cli_path: Option<String>,
    error: String,
) -> ContainersStatusResp {
    ContainersStatusResp {
        available: false,
        error: Some(error),
        active_context: context_name.to_string(),
        socket_path,
        engine_kind,
        engine_version: None,
        api_version: None,
        os: None,
        arch: None,
        containers_running: 0,
        containers_paused: 0,
        containers_stopped: 0,
        images_count: 0,
        mem_total_bytes: None,
        ncpu: None,
        docker_cli_available,
        docker_cli_path,
        vm: None,
    }
}

pub(super) fn extract_vm_resources(info: &Value) -> Option<VmResources> {
    if let Some(state) = str_field(info, "OttoVmState") {
        return Some(VmResources {
            state,
            cpus: info
                .get("OttoVmCpus")
                .and_then(|v| v.as_u64())
                .and_then(|n| u32::try_from(n).ok()),
            memory_used_bytes: info.get("OttoVmMemoryUsedBytes").and_then(|v| v.as_u64()),
            memory_target_bytes: info.get("OttoVmMemoryTargetBytes").and_then(|v| v.as_u64()),
            memory_ceiling_bytes: info
                .get("OttoVmMemoryCeilingBytes")
                .and_then(|v| v.as_u64()),
            cpu_percent: info.get("OttoVmCpuPercent").and_then(|v| v.as_f64()),
        });
    }
    if info.get("KiriVmAwake").is_some()
        || info.get("KiriVersion").is_some()
        || info.get("KiriStopped").is_some()
    {
        return Some(VmResources {
            state: classify_kiri_vm_state(info).to_string(),
            cpus: info
                .get("NCPU")
                .and_then(|v| v.as_u64())
                .and_then(|n| u32::try_from(n).ok()),
            memory_used_bytes: info.get("KiriMemoryUsed").and_then(|v| v.as_u64()),
            memory_target_bytes: info.get("KiriMemoryTarget").and_then(|v| v.as_u64()),
            memory_ceiling_bytes: info.get("KiriMemoryCeiling").and_then(|v| v.as_u64()),
            cpu_percent: info.get("KiriCpuPercent").and_then(|v| v.as_f64()),
        });
    }
    None
}

pub(super) fn parse_container_summary(v: &Value) -> ContainerSummary {
    let id = str_field(v, "Id").unwrap_or_default();
    let name = v
        .get("Names")
        .and_then(|n| n.as_array())
        .and_then(|a| a.first())
        .and_then(|s| s.as_str())
        .map(|s| s.trim_start_matches('/').to_string())
        .unwrap_or_else(|| short_id(&id));
    let labels = parse_labels(v.get("Labels"));
    let compose_project = labels.get("com.docker.compose.project").cloned();
    let compose_service = labels.get("com.docker.compose.service").cloned();
    let status = str_field(v, "Status").unwrap_or_default();
    let health = infer_health_from_status(&status);

    let mut ports = Vec::new();
    if let Some(arr) = v.get("Ports").and_then(|p| p.as_array()) {
        let mut seen = BTreeSet::new();
        for p in arr {
            let private_port = p
                .get("PrivatePort")
                .and_then(|n| n.as_u64())
                .and_then(|n| u16::try_from(n).ok())
                .unwrap_or(0);
            if private_port == 0 {
                continue;
            }
            let public_port = p
                .get("PublicPort")
                .and_then(|n| n.as_u64())
                .and_then(|n| u16::try_from(n).ok());
            let protocol = str_field(p, "Type").unwrap_or_else(|| "tcp".into());
            let ip = str_field(p, "IP");
            let key = (private_port, public_port, protocol.clone());
            if !seen.insert(key) {
                continue;
            }
            let url = public_port
                .filter(|_| protocol.eq_ignore_ascii_case("tcp"))
                .map(|hp| format!("http://localhost:{hp}"));
            ports.push(PortMapping {
                ip,
                private_port,
                public_port,
                protocol,
                url,
            });
        }
    }

    let networks = v
        .get("NetworkSettings")
        .and_then(|ns| ns.get("Networks"))
        .and_then(|m| m.as_object())
        .map(|obj| obj.keys().cloned().collect())
        .unwrap_or_default();
    let mounts_count = v
        .get("Mounts")
        .and_then(|m| m.as_array())
        .map(|a| a.len())
        .unwrap_or(0);

    ContainerSummary {
        short_id: short_id(&id),
        id,
        name,
        image: str_field(v, "Image").unwrap_or_default(),
        image_id: str_field(v, "ImageID").unwrap_or_default(),
        command: str_field(v, "Command").unwrap_or_default(),
        created: v.get("Created").and_then(|c| c.as_i64()).unwrap_or(0),
        state: str_field(v, "State")
            .unwrap_or_else(|| "unknown".into())
            .to_ascii_lowercase(),
        status,
        health,
        compose_project,
        compose_service,
        ports,
        labels,
        networks,
        mounts_count,
    }
}

pub(super) fn build_compose_stacks(containers: &[ContainerSummary]) -> Vec<ComposeStackSummary> {
    let mut map: BTreeMap<String, ComposeStackSummary> = BTreeMap::new();
    for c in containers {
        let Some(project) = c.compose_project.as_ref() else {
            continue;
        };
        let entry = map
            .entry(project.clone())
            .or_insert_with(|| ComposeStackSummary {
                project: project.clone(),
                working_dir: c
                    .labels
                    .get("com.docker.compose.project.working_dir")
                    .cloned(),
                config_files: c
                    .labels
                    .get("com.docker.compose.project.config_files")
                    .cloned(),
                running: 0,
                total: 0,
                container_ids: Vec::new(),
            });
        entry.total += 1;
        if c.state == "running" {
            entry.running += 1;
        }
        entry.container_ids.push(c.id.clone());
    }
    map.into_values().collect()
}

pub(super) fn parse_container_detail(v: &Value, reveal_secrets: bool) -> ContainerDetail {
    let id = str_field(v, "Id").unwrap_or_default();
    let name = str_field(v, "Name")
        .map(|s| s.trim_start_matches('/').to_string())
        .unwrap_or_else(|| short_id(&id));
    let config = v.get("Config").unwrap_or(&Value::Null);
    let state_obj = v.get("State").unwrap_or(&Value::Null);
    let host_cfg = v.get("HostConfig").unwrap_or(&Value::Null);
    let net_settings = v.get("NetworkSettings").unwrap_or(&Value::Null);

    let labels = parse_labels(config.get("Labels"));
    let compose_project = labels.get("com.docker.compose.project").cloned();
    let compose_service = labels.get("com.docker.compose.service").cloned();

    let raw_env = string_array(config, "Env");
    let env = parse_env_entries(&raw_env, reveal_secrets);

    let ports = parse_inspect_ports(net_settings.get("Ports"));
    let mounts = parse_inspect_mounts(v.get("Mounts"));
    let networks = parse_inspect_networks(net_settings.get("Networks"));

    let state = str_field(state_obj, "Status")
        .unwrap_or_else(|| "unknown".into())
        .to_ascii_lowercase();
    let health = state_obj
        .get("Health")
        .and_then(|h| h.get("Status"))
        .and_then(|s| s.as_str())
        .map(str::to_string);
    let restart_policy = host_cfg
        .get("RestartPolicy")
        .and_then(|r| r.get("Name"))
        .and_then(|s| s.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("no")
        .to_string();

    let status = match (&state[..], &health) {
        ("running", Some(h)) => format!("running ({h})"),
        _ => state.clone(),
    };

    ContainerDetail {
        short_id: short_id(&id),
        id,
        name,
        image: str_field(config, "Image").unwrap_or_default(),
        image_id: str_field(v, "Image").unwrap_or_default(),
        created: str_field(v, "Created").unwrap_or_default(),
        state,
        status,
        running: state_obj
            .get("Running")
            .and_then(|b| b.as_bool())
            .unwrap_or(false),
        paused: state_obj
            .get("Paused")
            .and_then(|b| b.as_bool())
            .unwrap_or(false),
        restarting: state_obj
            .get("Restarting")
            .and_then(|b| b.as_bool())
            .unwrap_or(false),
        oom_killed: state_obj
            .get("OOMKilled")
            .and_then(|b| b.as_bool())
            .unwrap_or(false),
        pid: state_obj.get("Pid").and_then(|n| n.as_i64()).unwrap_or(0),
        exit_code: state_obj
            .get("ExitCode")
            .and_then(|n| n.as_i64())
            .unwrap_or(0),
        started_at: non_zero_timestamp(str_field(state_obj, "StartedAt")),
        finished_at: non_zero_timestamp(str_field(state_obj, "FinishedAt")),
        health,
        restart_policy,
        entrypoint: string_array(config, "Entrypoint"),
        cmd: string_array(config, "Cmd"),
        working_dir: str_field(config, "WorkingDir").unwrap_or_default(),
        hostname: str_field(config, "Hostname").unwrap_or_default(),
        compose_project,
        compose_service,
        ports,
        env,
        mounts,
        networks,
        labels,
    }
}

fn parse_inspect_ports(ports_val: Option<&Value>) -> Vec<PortMapping> {
    let mut out = Vec::new();
    let Some(obj) = ports_val.and_then(|v| v.as_object()) else {
        return out;
    };
    let mut seen = BTreeSet::new();
    for (spec, bindings) in obj {
        let (port_str, proto) = spec.split_once('/').unwrap_or((spec.as_str(), "tcp"));
        let Ok(private_port) = port_str.parse::<u16>() else {
            continue;
        };
        if let Some(arr) = bindings.as_array() {
            if !arr.is_empty() {
                for b in arr {
                    let host_port = b
                        .get("HostPort")
                        .and_then(|s| s.as_str())
                        .and_then(|s| s.parse::<u16>().ok());
                    let ip = b
                        .get("HostIp")
                        .and_then(|s| s.as_str())
                        .filter(|s| !s.is_empty())
                        .map(str::to_string);
                    if !seen.insert((private_port, host_port, proto.to_string())) {
                        continue;
                    }
                    let url = host_port
                        .filter(|_| proto.eq_ignore_ascii_case("tcp"))
                        .map(|hp| format!("http://localhost:{hp}"));
                    out.push(PortMapping {
                        ip,
                        private_port,
                        public_port: host_port,
                        protocol: proto.to_string(),
                        url,
                    });
                }
                continue;
            }
        }
        if seen.insert((private_port, None, proto.to_string())) {
            out.push(PortMapping {
                ip: None,
                private_port,
                public_port: None,
                protocol: proto.to_string(),
                url: None,
            });
        }
    }
    out.sort_by_key(|p| (p.public_port.is_none(), p.public_port, p.private_port));
    out
}

fn parse_inspect_mounts(mounts_val: Option<&Value>) -> Vec<MountInfo> {
    let Some(arr) = mounts_val.and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    arr.iter()
        .map(|m| MountInfo {
            mount_type: str_field(m, "Type").unwrap_or_else(|| "bind".into()),
            source: str_field(m, "Source").unwrap_or_default(),
            destination: str_field(m, "Destination").unwrap_or_default(),
            mode: str_field(m, "Mode").unwrap_or_default(),
            rw: m.get("RW").and_then(|b| b.as_bool()).unwrap_or(true),
        })
        .collect()
}

fn parse_inspect_networks(nets_val: Option<&Value>) -> Vec<NetworkAttachment> {
    let Some(obj) = nets_val.and_then(|v| v.as_object()) else {
        return Vec::new();
    };
    obj.iter()
        .map(|(name, n)| NetworkAttachment {
            name: name.clone(),
            network_id: str_field(n, "NetworkID").unwrap_or_default(),
            ip_address: str_field(n, "IPAddress").unwrap_or_default(),
            gateway: str_field(n, "Gateway").unwrap_or_default(),
            mac_address: str_field(n, "MacAddress").unwrap_or_default(),
        })
        .collect()
}

/// Compute Docker CPU%, Memory%, network & block I/O from a `/stats?stream=false` JSON payload.
pub fn parse_container_stats(container_id: &str, v: &Value) -> ContainerStatsResp {
    let cpu_total = v
        .pointer("/cpu_stats/cpu_usage/total_usage")
        .and_then(|n| n.as_u64())
        .unwrap_or(0);
    let precpu_total = v
        .pointer("/precpu_stats/cpu_usage/total_usage")
        .and_then(|n| n.as_u64())
        .unwrap_or(0);
    let sys_cpu = v
        .pointer("/cpu_stats/system_cpu_usage")
        .and_then(|n| n.as_u64())
        .unwrap_or(0);
    let presys_cpu = v
        .pointer("/precpu_stats/system_cpu_usage")
        .and_then(|n| n.as_u64())
        .unwrap_or(0);
    let online_cpus = v
        .pointer("/cpu_stats/online_cpus")
        .and_then(|n| n.as_u64())
        .or_else(|| {
            v.pointer("/cpu_stats/cpu_usage/percpu_usage")
                .and_then(|a| a.as_array())
                .map(|a| a.len() as u64)
        })
        .unwrap_or(1)
        .max(1);

    let cpu_delta = cpu_total.saturating_sub(precpu_total) as f64;
    let sys_delta = sys_cpu.saturating_sub(presys_cpu) as f64;
    let cpu_percent = if sys_delta > 0.0 && cpu_delta > 0.0 {
        ((cpu_delta / sys_delta) * (online_cpus as f64) * 100.0 * 100.0).round() / 100.0
    } else {
        0.0
    };

    let raw_mem = v
        .pointer("/memory_stats/usage")
        .and_then(|n| n.as_u64())
        .unwrap_or(0);
    let cache = v
        .pointer("/memory_stats/stats/inactive_file")
        .or_else(|| v.pointer("/memory_stats/stats/cache"))
        .and_then(|n| n.as_u64())
        .unwrap_or(0);
    let memory_usage_bytes = raw_mem.saturating_sub(cache);
    let memory_limit_bytes = v
        .pointer("/memory_stats/limit")
        .and_then(|n| n.as_u64())
        .unwrap_or(0);
    let memory_percent = if memory_limit_bytes > 0 {
        ((memory_usage_bytes as f64 / memory_limit_bytes as f64) * 100.0 * 100.0).round() / 100.0
    } else {
        0.0
    };

    let (mut net_rx, mut net_tx) = (0u64, 0u64);
    if let Some(nets) = v.get("networks").and_then(|n| n.as_object()) {
        for iface in nets.values() {
            net_rx = net_rx.saturating_add(u64_field(iface, "rx_bytes"));
            net_tx = net_tx.saturating_add(u64_field(iface, "tx_bytes"));
        }
    }

    let (mut blk_r, mut blk_w) = (0u64, 0u64);
    if let Some(entries) = v
        .pointer("/blkio_stats/io_service_bytes_recursive")
        .and_then(|a| a.as_array())
    {
        for entry in entries {
            let op = str_field(entry, "op").unwrap_or_default();
            let val = u64_field(entry, "value");
            if op.eq_ignore_ascii_case("read") {
                blk_r = blk_r.saturating_add(val);
            } else if op.eq_ignore_ascii_case("write") {
                blk_w = blk_w.saturating_add(val);
            }
        }
    }

    ContainerStatsResp {
        container_id: container_id.to_string(),
        read_at: str_field(v, "read").unwrap_or_default(),
        cpu_percent,
        memory_usage_bytes,
        memory_limit_bytes,
        memory_percent,
        net_rx_bytes: net_rx,
        net_tx_bytes: net_tx,
        block_read_bytes: blk_r,
        block_write_bytes: blk_w,
        pids: v
            .pointer("/pids_stats/current")
            .and_then(|n| n.as_u64())
            .unwrap_or(0),
    }
}

pub(super) fn parse_image_summary(v: &Value, used_images: &BTreeSet<String>) -> ImageSummary {
    let id = str_field(v, "Id").unwrap_or_default();
    let repo_tags: Vec<String> = string_array(v, "RepoTags")
        .into_iter()
        .filter(|t| t != "<none>:<none>")
        .collect();
    let repo_digests: Vec<String> = string_array(v, "RepoDigests")
        .into_iter()
        .filter(|d| d != "<none>@<none>")
        .collect();
    let containers = v.get("Containers").and_then(|c| c.as_i64()).unwrap_or(-1);
    let in_use = containers > 0 || used_images.contains(&id);
    let dangling = repo_tags.is_empty();
    ImageSummary {
        short_id: short_id(&id),
        id,
        repo_tags,
        repo_digests,
        created: v.get("Created").and_then(|c| c.as_i64()).unwrap_or(0),
        size_bytes: u64_field(v, "Size"),
        containers,
        dangling,
        in_use,
    }
}

pub(super) fn parse_image_inspect(v: &Value) -> ImageInspectResp {
    let mut exposed_ports: Vec<String> = v
        .pointer("/Config/ExposedPorts")
        .and_then(|p| p.as_object())
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    exposed_ports.sort_by_key(|p| {
        p.split('/')
            .next()
            .and_then(|n| n.parse::<u16>().ok())
            .unwrap_or(u16::MAX)
    });
    ImageInspectResp {
        id: str_field(v, "Id").unwrap_or_default(),
        exposed_ports,
    }
}

pub(super) fn parse_volume_summary(v: &Value) -> VolumeSummary {
    let ref_count = v.pointer("/UsageData/RefCount").and_then(|n| n.as_i64());
    let size_bytes = v.pointer("/UsageData/Size").and_then(|n| n.as_i64());
    VolumeSummary {
        name: str_field(v, "Name").unwrap_or_default(),
        driver: str_field(v, "Driver").unwrap_or_else(|| "local".into()),
        mountpoint: str_field(v, "Mountpoint").unwrap_or_default(),
        created_at: str_field(v, "CreatedAt"),
        scope: str_field(v, "Scope").unwrap_or_else(|| "local".into()),
        labels: parse_labels(v.get("Labels")),
        size_bytes,
        ref_count,
        in_use: ref_count.unwrap_or(0) > 0,
    }
}

pub(super) fn parse_network_summary(v: &Value) -> NetworkSummary {
    let id = str_field(v, "Id").unwrap_or_default();
    let name = str_field(v, "Name").unwrap_or_default();
    let built_in = matches!(name.as_str(), "bridge" | "host" | "none");
    let ipam_cfg = v
        .pointer("/IPAM/Config")
        .and_then(|a| a.as_array())
        .and_then(|a| a.first());
    let subnet = ipam_cfg.and_then(|c| str_field(c, "Subnet"));
    let gateway = ipam_cfg.and_then(|c| str_field(c, "Gateway"));

    let mut containers = Vec::new();
    if let Some(obj) = v.get("Containers").and_then(|c| c.as_object()) {
        for (cid, entry) in obj {
            containers.push(NetworkContainerRef {
                id: short_id(cid),
                name: str_field(entry, "Name").unwrap_or_default(),
                ipv4_address: str_field(entry, "IPv4Address").unwrap_or_default(),
            });
        }
    }
    containers.sort_by(|a, b| a.name.cmp(&b.name));

    NetworkSummary {
        short_id: short_id(&id),
        id,
        name,
        driver: str_field(v, "Driver").unwrap_or_default(),
        scope: str_field(v, "Scope").unwrap_or_else(|| "local".into()),
        internal: v.get("Internal").and_then(|b| b.as_bool()).unwrap_or(false),
        attachable: v
            .get("Attachable")
            .and_then(|b| b.as_bool())
            .unwrap_or(false),
        built_in,
        subnet,
        gateway,
        created: str_field(v, "Created"),
        containers,
        labels: parse_labels(v.get("Labels")),
    }
}

pub(super) fn short_id(id: &str) -> String {
    let hex = id.strip_prefix("sha256:").unwrap_or(id);
    hex.chars().take(12).collect()
}

fn infer_health_from_status(status: &str) -> Option<String> {
    let lower = status.to_ascii_lowercase();
    if lower.contains("(unhealthy)") {
        Some("unhealthy".into())
    } else if lower.contains("(healthy)") {
        Some("healthy".into())
    } else if lower.contains("(health: starting)") {
        Some("starting".into())
    } else {
        None
    }
}

fn non_zero_timestamp(ts: Option<String>) -> Option<String> {
    ts.filter(|s| !s.is_empty() && !s.starts_with("0001-01-01"))
}

fn parse_labels(v: Option<&Value>) -> BTreeMap<String, String> {
    let Some(obj) = v.and_then(|x| x.as_object()) else {
        return BTreeMap::new();
    };
    obj.iter()
        .filter_map(|(k, val)| val.as_str().map(|s| (k.clone(), s.to_string())))
        .collect()
}

pub(super) fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub(super) fn u64_field(v: &Value, key: &str) -> u64 {
    v.get(key).and_then(|x| x.as_u64()).unwrap_or(0)
}

pub(super) fn string_array(v: &Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}
