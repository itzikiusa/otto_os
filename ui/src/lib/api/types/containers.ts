// Contract mirror: containers. Part of lib/api/types (see ../types.ts, the barrel).
// Mirrors crates/otto-containers/src/types.rs — change them together.

import type { Id } from './core';

// ---------------------------------------------------------------------------
// Containers console (mirror of crates/otto-containers)
// ---------------------------------------------------------------------------

/** Identified container engine backing a Docker Engine API Unix socket. */
export type EngineKind =
  | 'kiri'
  | 'otto'
  | 'docker_desktop'
  | 'colima'
  | 'orbstack'
  | 'rancher_desktop'
  | 'podman'
  | 'lima'
  | 'custom'
  | 'unknown';

/** `GET /containers/runtime` and `POST /containers/runtime/{start,stop,restart}` response. */
export interface RuntimeStatusResp {
  /** Embedded runtime identifier (`"kiri"`). */
  runtime: string;
  installed: boolean;
  binary_path?: string | null;
  /** Discovery source (`"env"`, `"sidecar"`, `"data_dir"`, `"path"`, `"homebrew"`). */
  binary_source?: string | null;
  version?: string | null;
  /** True when the runtime socket responds to `/_ping` and the VM is not stopped. */
  running: boolean;
  /** True when the daemon socket answers even if the VM is stopped or asleep. */
  daemon_reachable: boolean;
  /** Normalized VM state: `"running"`, `"asleep"`, `"stopped"`, or `"offline"`. */
  vm_state: string;
  socket_path: string;
  managed_by_otto: boolean;
  pid?: number | null;
  can_start: boolean;
  can_stop: boolean;
  /** True when `POST /containers/runtime/install` is available on this host. */
  can_install?: boolean;
  /** True while an in-app embedded Kiri provisioning job is currently running. */
  installing?: boolean;
  /** Human-readable progress phase while `installing == true`. */
  install_progress?: string | null;
  /** Error message from the most recent failed install attempt, if any. */
  install_error?: string | null;
  guest_dir?: string | null;
  guest_assets_present: boolean;
  log_path: string;
  install_hint?: string | null;
  entitlement_note: string;
  last_error?: string | null;
}

/** `POST /containers/runtime/install` request body (all fields optional). */
export interface InstallRuntimeReq {
  source_dir?: string | null;
  download_url?: string | null;
  sha256?: string | null;
  start_after_install?: boolean;
}

/** Discovered Docker context or well-known Unix socket endpoint. */
export interface DockerContextInfo {
  name: string;
  socket_path: string;
  host: string;
  engine_kind: EngineKind;
  description?: string | null;
  is_active: boolean;
  is_docker_default: boolean;
  reachable: boolean;
}

/** `GET /containers/contexts` response. */
export interface ContainersContextsResp {
  active_context: string;
  active_socket: string;
  contexts: DockerContextInfo[];
}

/** `POST /containers/contexts/select` request body. */
export interface SelectContextReq {
  name?: string | null;
  socket_path?: string | null;
}

/** Optional VM telemetry populated when the daemon exposes `Otto*` `/info` fields. */
export interface VmResources {
  state: string;
  cpus?: number | null;
  memory_used_bytes?: number | null;
  memory_target_bytes?: number | null;
  memory_ceiling_bytes?: number | null;
  cpu_percent?: number | null;
}

/** `GET /containers/status` response. */
export interface ContainersStatusResp {
  available: boolean;
  error?: string | null;
  active_context: string;
  socket_path: string;
  engine_kind: EngineKind;
  engine_version?: string | null;
  api_version?: string | null;
  os?: string | null;
  arch?: string | null;
  containers_running: number;
  containers_paused: number;
  containers_stopped: number;
  images_count: number;
  mem_total_bytes?: number | null;
  ncpu?: number | null;
  docker_cli_available: boolean;
  docker_cli_path?: string | null;
  vm?: VmResources | null;
}

/** Published or exposed port mapping on a container. */
export interface PortMapping {
  ip?: string | null;
  private_port: number;
  public_port?: number | null;
  protocol: string;
  /** Clickable `http://localhost:<public_port>` hint for TCP published ports. */
  url?: string | null;
}

/** One row in `GET /containers`. */
export interface ContainerSummary {
  id: string;
  short_id: string;
  name: string;
  image: string;
  image_id: string;
  command: string;
  created: number;
  /** Normalized lowercase Docker state (`running`, `exited`, `paused`, `created`, `restarting`, `dead`). */
  state: string;
  /** Human status string from Docker (e.g. `"Up 2 hours (healthy)"`). */
  status: string;
  health?: string | null;
  compose_project?: string | null;
  compose_service?: string | null;
  ports: PortMapping[];
  labels: Record<string, string>;
  networks: string[];
  mounts_count: number;
}

/** Compose project rollup derived from container labels. */
export interface ComposeStackSummary {
  project: string;
  working_dir?: string | null;
  config_files?: string | null;
  running: number;
  total: number;
  container_ids: string[];
}

/** `GET /containers` response. */
export interface ContainersListResp {
  containers: ContainerSummary[];
  stacks: ComposeStackSummary[];
}

/** One environment variable on a container detail view. */
export interface EnvEntry {
  key: string;
  value: string;
  secret: boolean;
  masked: boolean;
}

/** Bind mount, volume, or tmpfs attached to a container. */
export interface MountInfo {
  mount_type: string;
  source: string;
  destination: string;
  mode: string;
  rw: boolean;
}

/** Network attachment on a container detail view. */
export interface NetworkAttachment {
  name: string;
  network_id: string;
  ip_address: string;
  gateway: string;
  mac_address: string;
}

/** `GET /containers/{id}` response. */
export interface ContainerDetail {
  id: string;
  short_id: string;
  name: string;
  image: string;
  image_id: string;
  created: string;
  state: string;
  status: string;
  running: boolean;
  paused: boolean;
  restarting: boolean;
  oom_killed: boolean;
  pid: number;
  exit_code: number;
  started_at?: string | null;
  finished_at?: string | null;
  health?: string | null;
  restart_policy: string;
  entrypoint: string[];
  cmd: string[];
  working_dir: string;
  hostname: string;
  compose_project?: string | null;
  compose_service?: string | null;
  ports: PortMapping[];
  env: EnvEntry[];
  mounts: MountInfo[];
  networks: NetworkAttachment[];
  labels: Record<string, string>;
}

/** Port binding entry inside `RunContainerReq`. */
export interface PortBindingReq {
  host_ip?: string | null;
  host_port?: number | null;
  container_port: number;
  protocol?: string | null;
}

/** `POST /containers/run` request body. */
export interface RunContainerReq {
  image: string;
  name?: string | null;
  cmd?: string[];
  env?: string[];
  ports?: PortBindingReq[];
  volumes?: string[];
  working_dir?: string | null;
  restart_policy?: string | null;
  auto_remove?: boolean;
  pull_if_missing?: boolean;
}

/** `POST /containers/run` response. */
export interface RunContainerResp {
  id: string;
  short_id: string;
  warnings: string[];
}

/** Result of a single-container lifecycle action. */
export interface ContainerActionResp {
  ok: boolean;
  id: string;
  action: string;
  message?: string | null;
}

/** Per-container failure inside a compose stack batch action. */
export interface StackActionFailure {
  id: string;
  name: string;
  error: string;
}

/** `POST /containers/stacks/{project}/{start,stop,restart}` response. */
export interface StackActionResp {
  project: string;
  action: string;
  succeeded: string[];
  failed: StackActionFailure[];
}

/** One decoded line from `GET /containers/{id}/logs`. */
export interface ContainerLogLine {
  stream: 'stdout' | 'stderr' | 'tty' | string;
  timestamp?: string | null;
  text: string;
}

/** `GET /containers/{id}/logs` response. */
export interface ContainerLogsResp {
  container_id: string;
  lines: ContainerLogLine[];
  truncated: boolean;
}

/** `GET /containers/{id}/stats` one-shot snapshot response. */
export interface ContainerStatsResp {
  container_id: string;
  read_at: string;
  cpu_percent: number;
  memory_usage_bytes: number;
  memory_limit_bytes: number;
  memory_percent: number;
  net_rx_bytes: number;
  net_tx_bytes: number;
  block_read_bytes: number;
  block_write_bytes: number;
  pids: number;
}

/** `POST /containers/{id}/exec` request body (spawns an Otto PTY session). */
export interface ContainerExecReq {
  workspace_id: Id;
  command?: string[] | null;
  user?: string | null;
  working_dir?: string | null;
}

/** One row in `GET /containers/images`. */
export interface ImageSummary {
  id: string;
  short_id: string;
  repo_tags: string[];
  repo_digests: string[];
  created: number;
  size_bytes: number;
  containers: number;
  dangling: boolean;
  in_use: boolean;
}

/** `GET /containers/images` response. */
export interface ImagesListResp {
  images: ImageSummary[];
}

/** `GET /containers/images/{id}` response: what a local image declares (never its env). */
export interface ImageInspectResp {
  id: string;
  /** `Config.ExposedPorts` keys such as `27017/tcp`, sorted by port. */
  exposed_ports: string[];
}

/** `POST /containers/images/pull` request body. */
export interface PullImageReq {
  reference: string;
  platform?: string | null;
}

/** `POST /containers/images/pull` response. */
export interface PullImageResp {
  reference: string;
  status: string;
  digest?: string | null;
  events: string[];
}

/** One row in `GET /containers/volumes`. */
export interface VolumeSummary {
  name: string;
  driver: string;
  mountpoint: string;
  created_at?: string | null;
  scope: string;
  labels: Record<string, string>;
  size_bytes?: number | null;
  ref_count?: number | null;
  in_use: boolean;
}

/** `GET /containers/volumes` response. */
export interface VolumesListResp {
  volumes: VolumeSummary[];
}

/** Container endpoint attached to a Docker network. */
export interface NetworkContainerRef {
  id: string;
  name: string;
  ipv4_address: string;
}

/** One row in `GET /containers/networks`. */
export interface NetworkSummary {
  id: string;
  short_id: string;
  name: string;
  driver: string;
  scope: string;
  internal: boolean;
  attachable: boolean;
  built_in: boolean;
  subnet?: string | null;
  gateway?: string | null;
  created?: string | null;
  containers: NetworkContainerRef[];
  labels: Record<string, string>;
}

/** `GET /containers/networks` response. */
export interface NetworksListResp {
  networks: NetworkSummary[];
}

/** Result of `POST /containers/{images,volumes,networks}/prune`. */
export interface PruneResp {
  kind: string;
  deleted: string[];
  space_reclaimed_bytes: number;
}

/** `POST /containers/system/prune` request body. */
export interface SystemPruneReq {
  include_volumes?: boolean;
  all_unused_images?: boolean;
}

/** `POST /containers/system/prune` response. */
export interface SystemPruneResp {
  containers_deleted: string[];
  images_deleted: string[];
  networks_deleted: string[];
  volumes_deleted: string[];
  space_reclaimed_bytes: number;
}
