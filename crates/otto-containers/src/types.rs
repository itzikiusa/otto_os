//! Public REST contract types for `/containers/*` (`otto-containers`).
//!
//! Mirror these structs in `ui/src/lib/api/types/containers.ts` and
//! `docs/contracts/api.md` in lockstep.

use std::collections::BTreeMap;

use otto_core::Id;
use serde::{Deserialize, Serialize};

/// Identified container engine backing a Docker Engine API Unix socket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineKind {
    Kiri,
    Otto,
    DockerDesktop,
    Colima,
    Orbstack,
    RancherDesktop,
    Podman,
    Lima,
    Custom,
    Unknown,
}

/// `GET /containers/runtime` and `POST /containers/runtime/{start,stop,restart}` response.
///
/// Describes the embedded headless `kiri` runtime managed by `ottod` alongside
/// any externally-running Docker daemon (Docker Desktop, Colima, OrbStack, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeStatusResp {
    /// Embedded runtime identifier (`"kiri"`).
    pub runtime: String,
    /// True when a `kiri` binary was discovered.
    pub installed: bool,
    /// Absolute path to the located `kiri` binary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binary_path: Option<String>,
    /// Discovery source (`"env"`, `"sidecar"`, `"data_dir"`, `"path"`, `"homebrew"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binary_source: Option<String>,
    /// Reported runtime version (from `/info` `KiriVersion` or `/version`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// True when the Kiri socket responds to `/_ping` and the VM is not stopped.
    pub running: bool,
    /// True when the Kiri daemon socket is answering (even if VM is stopped/asleep).
    pub daemon_reachable: bool,
    /// Normalized VM state: `"running"`, `"asleep"`, `"stopped"`, or `"offline"`.
    pub vm_state: String,
    /// Unix socket path used by Kiri (default `~/.kiri/run/docker.sock`).
    pub socket_path: String,
    /// True when the running `kiri daemon` process was spawned by `ottod`.
    pub managed_by_otto: bool,
    /// PID of the Otto-managed `kiri daemon` process, if active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// True when `POST /containers/runtime/start` can be invoked.
    pub can_start: bool,
    /// True when `POST /containers/runtime/stop` can be invoked.
    pub can_stop: bool,
    /// True when `POST /containers/runtime/install` is available on this host.
    pub can_install: bool,
    /// True while an in-app embedded Kiri provisioning job is currently running.
    pub installing: bool,
    /// Human-readable progress phase while `installing == true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_progress: Option<String>,
    /// Error message from the most recent failed install attempt, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_error: Option<String>,
    /// Located guest kernel/initramfs asset directory (`KIRI_GUEST_DIR` / `Resources/guest` / `<data_dir>/runtime/guest`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guest_dir: Option<String>,
    /// True when guest VM assets are present (or on Linux where no VM guest is needed).
    pub guest_assets_present: bool,
    /// Path to the Otto-managed daemon log file (`<data_dir>/logs/kiri.log`).
    pub log_path: String,
    /// Actionable install guidance when `installed == false`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_hint: Option<String>,
    /// macOS `com.apple.security.virtualization` entitlement / sidecar note.
    pub entitlement_note: String,
    /// Last startup or probe error, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

/// `POST /containers/runtime/install` request body (all fields optional).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct InstallRuntimeReq {
    /// Local directory containing a built/staged `kiri` binary and optional `guest/` assets.
    #[serde(default)]
    pub source_dir: Option<String>,
    /// Explicit `.tar.gz` archive URL (defaults to `OTTO_KIRI_RELEASE_URL` or upstream feed).
    #[serde(default)]
    pub download_url: Option<String>,
    /// Optional expected 64-hex SHA-256 of the downloaded archive.
    #[serde(default)]
    pub sha256: Option<String>,
    /// Automatically start the embedded Kiri runtime once provisioning completes.
    #[serde(default)]
    pub start_after_install: bool,
}

/// Discovered Docker context or well-known Unix socket endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockerContextInfo {
    pub name: String,
    pub socket_path: String,
    pub host: String,
    pub engine_kind: EngineKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub is_active: bool,
    pub is_docker_default: bool,
    pub reachable: bool,
}

/// `GET /containers/contexts` response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainersContextsResp {
    pub active_context: String,
    pub active_socket: String,
    pub contexts: Vec<DockerContextInfo>,
}

/// `POST /containers/contexts/select` body.
#[derive(Debug, Clone, Deserialize)]
pub struct SelectContextReq {
    /// Named context from `GET /containers/contexts` (e.g. `"desktop-linux"`,
    /// `"colima"`, `"orbstack"`, `"default"`).
    #[serde(default)]
    pub name: Option<String>,
    /// Explicit Unix socket path (with or without `unix://` prefix).
    #[serde(default)]
    pub socket_path: Option<String>,
}

/// Optional VM telemetry populated when the daemon exposes `Otto*` `/info`
/// fields (reserved for an embedded VM backend; `None` for standard daemons).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VmResources {
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpus: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_used_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_target_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_ceiling_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_percent: Option<f64>,
}

/// `GET /containers/status` response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainersStatusResp {
    pub available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub active_context: String,
    pub socket_path: String,
    pub engine_kind: EngineKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arch: Option<String>,
    pub containers_running: u64,
    pub containers_paused: u64,
    pub containers_stopped: u64,
    pub images_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mem_total_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ncpu: Option<u32>,
    pub docker_cli_available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docker_cli_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vm: Option<VmResources>,
}

/// Published or exposed port mapping on a container.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortMapping {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ip: Option<String>,
    pub private_port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_port: Option<u16>,
    pub protocol: String,
    /// Clickable `http://localhost:<public_port>` hint for TCP published ports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// One row in `GET /containers`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerSummary {
    pub id: String,
    pub short_id: String,
    pub name: String,
    pub image: String,
    pub image_id: String,
    pub command: String,
    pub created: i64,
    /// Normalized lowercase Docker state (`running`, `exited`, `paused`, `created`, `restarting`, `dead`).
    pub state: String,
    /// Human status string from Docker (e.g. `"Up 2 hours (healthy)"`).
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compose_project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compose_service: Option<String>,
    pub ports: Vec<PortMapping>,
    pub labels: BTreeMap<String, String>,
    pub networks: Vec<String>,
    pub mounts_count: usize,
}

/// Compose project rollup derived from container labels.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposeStackSummary {
    pub project: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_files: Option<String>,
    pub running: usize,
    pub total: usize,
    pub container_ids: Vec<String>,
}

/// `GET /containers` response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainersListResp {
    pub containers: Vec<ContainerSummary>,
    pub stacks: Vec<ComposeStackSummary>,
}

/// One environment variable on a container detail view.
///
/// Variables classified as secrets (`secret == true`) have `value` replaced
/// with `"••••••••"` and `masked == true` unless `GET /containers/{id}` is
/// called with `?reveal_secrets=true`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvEntry {
    pub key: String,
    pub value: String,
    pub secret: bool,
    pub masked: bool,
}

/// Bind mount, volume, or tmpfs attached to a container.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MountInfo {
    pub mount_type: String,
    pub source: String,
    pub destination: String,
    pub mode: String,
    pub rw: bool,
}

/// Network attachment on a container detail view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkAttachment {
    pub name: String,
    pub network_id: String,
    pub ip_address: String,
    pub gateway: String,
    pub mac_address: String,
}

/// `GET /containers/{id}` response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerDetail {
    pub id: String,
    pub short_id: String,
    pub name: String,
    pub image: String,
    pub image_id: String,
    pub created: String,
    pub state: String,
    pub status: String,
    pub running: bool,
    pub paused: bool,
    pub restarting: bool,
    pub oom_killed: bool,
    pub pid: i64,
    pub exit_code: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health: Option<String>,
    pub restart_policy: String,
    pub entrypoint: Vec<String>,
    pub cmd: Vec<String>,
    pub working_dir: String,
    pub hostname: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compose_project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compose_service: Option<String>,
    pub ports: Vec<PortMapping>,
    pub env: Vec<EnvEntry>,
    pub mounts: Vec<MountInfo>,
    pub networks: Vec<NetworkAttachment>,
    pub labels: BTreeMap<String, String>,
}

/// Port binding entry inside [`RunContainerReq`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortBindingReq {
    #[serde(default)]
    pub host_ip: Option<String>,
    #[serde(default)]
    pub host_port: Option<u16>,
    pub container_port: u16,
    #[serde(default)]
    pub protocol: Option<String>,
}

/// `POST /containers/run` body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunContainerReq {
    pub image: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub cmd: Vec<String>,
    #[serde(default)]
    pub env: Vec<String>,
    #[serde(default)]
    pub ports: Vec<PortBindingReq>,
    #[serde(default)]
    pub volumes: Vec<String>,
    #[serde(default)]
    pub working_dir: Option<String>,
    #[serde(default)]
    pub restart_policy: Option<String>,
    #[serde(default)]
    pub auto_remove: bool,
    #[serde(default)]
    pub pull_if_missing: bool,
}

/// `POST /containers/run` response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunContainerResp {
    pub id: String,
    pub short_id: String,
    pub warnings: Vec<String>,
}

/// Result of a single-container lifecycle action (`start`, `stop`, `restart`,
/// `pause`, `unpause`, `kill`, `remove`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerActionResp {
    pub ok: bool,
    pub id: String,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Per-container failure inside a compose stack batch action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StackActionFailure {
    pub id: String,
    pub name: String,
    pub error: String,
}

/// `POST /containers/stacks/{project}/{start,stop,restart}` response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StackActionResp {
    pub project: String,
    pub action: String,
    pub succeeded: Vec<String>,
    pub failed: Vec<StackActionFailure>,
}

/// One decoded line from `GET /containers/{id}/logs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerLogLine {
    /// `"stdout"`, `"stderr"`, or `"tty"`.
    pub stream: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    pub text: String,
}

/// `GET /containers/{id}/logs` response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerLogsResp {
    pub container_id: String,
    pub lines: Vec<ContainerLogLine>,
    pub truncated: bool,
}

/// `GET /containers/{id}/stats` one-shot snapshot response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerStatsResp {
    pub container_id: String,
    pub read_at: String,
    pub cpu_percent: f64,
    pub memory_usage_bytes: u64,
    pub memory_limit_bytes: u64,
    pub memory_percent: f64,
    pub net_rx_bytes: u64,
    pub net_tx_bytes: u64,
    pub block_read_bytes: u64,
    pub block_write_bytes: u64,
    pub pids: u64,
}

/// `POST /containers/{id}/exec` request body (spawns an Otto PTY session).
#[derive(Debug, Clone, Deserialize)]
pub struct ContainerExecReq {
    pub workspace_id: Id,
    /// Optional explicit command argv; defaults to an interactive bash-or-sh shell.
    #[serde(default)]
    pub command: Option<Vec<String>>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub working_dir: Option<String>,
}

/// One row in `GET /containers/images`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageSummary {
    pub id: String,
    pub short_id: String,
    pub repo_tags: Vec<String>,
    pub repo_digests: Vec<String>,
    pub created: i64,
    pub size_bytes: u64,
    pub containers: i64,
    pub dangling: bool,
    pub in_use: bool,
}

/// `GET /containers/images` response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImagesListResp {
    pub images: Vec<ImageSummary>,
}

/// `GET /containers/images/{id}` response: what a local image declares.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageInspectResp {
    pub id: String,
    /// `Config.ExposedPorts` keys (`5432/tcp`, `53/udp`), sorted by port.
    pub exposed_ports: Vec<String>,
}

/// `POST /containers/images/pull` request body.
#[derive(Debug, Clone, Deserialize)]
pub struct PullImageReq {
    pub reference: String,
    #[serde(default)]
    pub platform: Option<String>,
}

/// `POST /containers/images/pull` response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullImageResp {
    pub reference: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
    pub events: Vec<String>,
}

/// One row in `GET /containers/volumes`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeSummary {
    pub name: String,
    pub driver: String,
    pub mountpoint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    pub scope: String,
    pub labels: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ref_count: Option<i64>,
    pub in_use: bool,
}

/// `GET /containers/volumes` response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumesListResp {
    pub volumes: Vec<VolumeSummary>,
}

/// Container endpoint attached to a Docker network.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkContainerRef {
    pub id: String,
    pub name: String,
    pub ipv4_address: String,
}

/// One row in `GET /containers/networks`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkSummary {
    pub id: String,
    pub short_id: String,
    pub name: String,
    pub driver: String,
    pub scope: String,
    pub internal: bool,
    pub attachable: bool,
    pub built_in: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subnet: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    pub containers: Vec<NetworkContainerRef>,
    pub labels: BTreeMap<String, String>,
}

/// `GET /containers/networks` response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworksListResp {
    pub networks: Vec<NetworkSummary>,
}

/// Result of `POST /containers/{images,volumes,networks}/prune`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PruneResp {
    pub kind: String,
    pub deleted: Vec<String>,
    pub space_reclaimed_bytes: u64,
}

/// `POST /containers/system/prune` request body.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SystemPruneReq {
    #[serde(default)]
    pub include_volumes: bool,
    #[serde(default)]
    pub all_unused_images: bool,
}

/// `POST /containers/system/prune` response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemPruneResp {
    pub containers_deleted: Vec<String>,
    pub images_deleted: Vec<String>,
    pub networks_deleted: Vec<String>,
    pub volumes_deleted: Vec<String>,
    pub space_reclaimed_bytes: u64,
}
