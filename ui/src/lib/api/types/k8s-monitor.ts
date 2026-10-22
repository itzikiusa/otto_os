// Contract mirror: k8s-monitor. Part of lib/api/types (see ../types.ts, the barrel).
// Mirrors docs/contracts/api.md + ws.md — change them together.

import type {
  ApiRunResult,
} from './api-client';
import type {
  Environment,
} from './connections';
import type {
  Id, User,
} from './core';
import type {
  WorkflowCheckpoint, WorkflowRun,
} from './workflows';

// ---- Kubernetes monitoring (`/k8s/monitor/*`, `/k8s/clusters/{id}/monitor*`) ----

export type K8sProbeFormat = 'prometheus' | 'json' | 'health';
export type K8sUnit = 'number' | 'bytes' | 'bytes_human' | 'duration_human' | 'percent';
export type K8sMonitorTransport = 'auto' | 'proxy' | 'port_forward';

export interface K8sMapping {
  /** Dotted path into the JSON body, e.g. `memory_stats.sys`; numeric segments index arrays. */
  field: string;
  /** Emit a sample under this metric name. */
  metric?: string | null;
  /** Attach the (string) value as a pod-level label on every sample of the cycle. */
  label?: string | null;
  unit?: K8sUnit;
}

export interface K8sProbe {
  name: string;
  /** Omitted ⇒ the container's first declared port. */
  port?: number | null;
  /** Must start with `/`. */
  path: string;
  format: K8sProbeFormat;
  mappings?: K8sMapping[];
  /** Series-name globs (prometheus only); empty include = everything. */
  include?: string[];
  exclude?: string[];
  timeout_ms?: number;
}

export type K8sExclusion =
  | { kind: 'namespace'; match: string }
  | { kind: 'pod'; match: string }
  /** Glob against `<kind>:<workload>`, e.g. `cronjob:*`. */
  | { kind: 'workload'; match: string }
  /** `k=v,k2!=v2,k3` (bare key = exists). */
  | { kind: 'label'; selector: string };

export interface K8sMonitorConfig {
  enabled: boolean;
  /** 15..3600 */
  interval_secs: number;
  /** `[]` ⇒ the cluster's default namespace (required when that is null). */
  namespaces: string[];
  /** ≤ 10 */
  probes: K8sProbe[];
  exclusions: K8sExclusion[];
  transport: K8sMonitorTransport;
  /** 1..32 parallel port-forwards */
  concurrency: number;
  /** 1..90 */
  retention_days: number;
  /** Prometheus series kept per pod per cycle (100..10000, default 1500); `_bucket` series are dropped first. */
  series_cap: number;
  /** Probe metrics-server every cycle; off = the call is skipped and status reports `disabled`. */
  metrics_server: boolean;
  /** Keep per-request `path` + `method` labels on the request / latency counters (never on
   *  histogram buckets) so the fleet dashboard can drill down to a route. Default false. */
  request_labels?: boolean;
}

export interface K8sMonitorStatus {
  cluster_id: Id;
  last_cycle_at?: string | null;
  last_ok_at?: string | null;
  last_error: string;
  transport_used: string;
  /** `ok` | `absent` | `disabled` | `unknown` | `forbidden: <kubectl message>` */
  metrics_server: string;
  pods_seen: number;
  pods_scraped: number;
  pods_failed: number;
  cycle_ms: number;
  /** Set on `monitor/workloads` for a namespace-scoped caller: the row is
   *  cluster-wide, so counts/`cycle_ms` are zeroed, `last_error` is blank and
   *  `metrics_server` keeps only its status word (timestamps survive). */
  restricted?: boolean;
}

export interface K8sMonitorPreset {
  id: string;
  title: string;
  probes: K8sProbe[];
}

export interface K8sMonitorResp {
  config: K8sMonitorConfig;
  status: K8sMonitorStatus | null;
  presets: K8sMonitorPreset[];
}

export interface K8sMonitorTestProbe {
  name: string;
  ok: boolean;
  status?: number;
  ms?: number;
  port?: number;
  samples?: { metric: string; labels: Record<string, string>; value: number }[];
  sample_count?: number;
  labels?: Record<string, string>;
  parse_errors?: number;
  capped?: boolean;
  body_preview?: string;
  error?: string;
}

export interface K8sMonitorTestResp {
  namespace: string;
  pod: string;
  workload: string;
  transport: string;
  metrics_server: string;
  probes: K8sMonitorTestProbe[];
}

export interface K8sRestartCounts {
  oom: number;
  crash: number;
  probe: number;
  unknown: number;
}

export type K8sMonitorHealth = 'healthy' | 'degraded' | 'incident' | 'off' | 'unknown';

export interface K8sMonitorOverviewRow {
  cluster: { id: Id; name: string; environment: Environment; color?: string | null };
  /** The caller can discover this cluster but lacks cluster-wide `metrics`:
   *  figures are zeroed (namespace-scoped users never see other namespaces). */
  restricted?: boolean;
  enabled: boolean;
  interval_secs: number;
  status: K8sMonitorStatus | null;
  health: K8sMonitorHealth;
  window: string;
  pods: { running: number; pending: number; failed: number; crashloop: number; total: number };
  /** In-place, unplanned restarts in the window. */
  restarts: K8sRestartCounts;
  /** Planned pod replacements (rollouts, scales, drains, Otto actions). */
  churn: number;
  mem: { used: number; limit: number; pct: number };
  rps: number;
  err_pct: number;
  drift: { workload: string; versions: string[] }[];
  workloads: number;
}

/** One replica of a workload (the per-pod view). */
export interface K8sMonitorPodStat {
  pod: string;
  node: string;
  phase: string;
  ready: boolean;
  version: string;
  /** Latest memory sample; 0 = not sampled. */
  mem_bytes: number;
  mem_limit: number;
  /** % of this pod's own limit. */
  mem_pct: number;
  restarts_lifetime: number;
  /** Unplanned restarts inside the window. */
  restarts: K8sRestartCounts;
  crashloop: boolean;
  age_seconds: number;
}

export interface K8sMonitorWorkloadRow {
  namespace: string;
  workload: string;
  kind: string;
  pods: number;
  ready: number;
  /** Sum over pods (capacity view). */
  mem_bytes: number;
  mem_limit: number;
  /** % of limit for the worst pod. */
  mem_pct: number;
  /** Per-pod: average and hungriest pod. */
  mem_avg: number;
  mem_max: number;
  mem_max_pod: string;
  mem_sampled: number;
  pods_detail: K8sMonitorPodStat[];
  mem_trend_pct?: number | null;
  restarts: K8sRestartCounts;
  churn_planned: number;
  churn_unknown: number;
  rps: number;
  err_pct: number;
  err_pct_baseline: number;
  rps_baseline: number;
  latency_kind: 'p95' | 'avg' | '';
  latency_ms: number;
  latency_baseline_ms: number;
  versions: string[];
  crashloop: number;
  spark: { mem: number[]; rps: number[] };
}

export interface K8sMonitorWorkloadsResp {
  window: string;
  step_secs: number;
  enabled: boolean;
  status: K8sMonitorStatus | null;
  /** Every namespace in the last snapshot, regardless of the `ns` filter. */
  namespaces: string[];
  workloads: K8sMonitorWorkloadRow[];
}

export interface K8sMonitorSeries {
  metric: string;
  kind: 'gauge' | 'rate';
  step_secs: number;
  points: { t: string; v: number }[];
}

export type K8sRestartClass = 'oom' | 'crash' | 'probe' | 'planned' | 'completed' | 'unknown';

export interface K8sMonitorEvent {
  ts: string;
  namespace: string;
  workload: string;
  pod: string;
  container: string;
  /** `version` = a workload's dominant build version changed (`reason` is `<from> → <to>`). */
  kind: 'restart' | 'churn' | 'version' | 'k8s_event';
  class: K8sRestartClass | '';
  reason: string;
  exit_code: number;
  detail: Record<string, unknown> | null;
  actor: string;
}

// --- Fleet dashboard (`GET /k8s/monitor/fleet/*`) — ClickHouse-only, cross-cluster ---

/** A cluster as the fleet routes label it: the registry row when it exists,
 *  else the bare id (history for a removed cluster still shows). */
export interface K8sFleetCluster {
  id: string;
  name: string;
  environment: Environment;
  color?: string | null;
}

export interface K8sFleetFilters {
  window: string;
  /** Every registered cluster plus any id with rows; `rows` = sample + event rows in the window. */
  clusters: (K8sFleetCluster & { rows: number })[];
  namespaces: { cluster_id: string; namespace: string }[];
  workloads: { cluster_id: string; namespace: string; workload: string }[];
  /** Only for a narrowed selection (a workload / pod, or one cluster + namespace); ≤ 2000. */
  pods: { cluster_id: string; namespace: string; workload: string; pod: string }[];
}

export type K8sFleetGroup = 'workload' | 'pod';
export type K8sFleetSortKey =
  | 'cluster' | 'namespace' | 'workload' | 'pod' | 'pods' | 'restarts' | 'oom' | 'crash' | 'probe' | 'churn'
  | 'mem_last' | 'mem_avg' | 'mem_max' | 'rps' | 'err_pct' | 'latency_ms';

export interface K8sFleetRow {
  cluster: K8sFleetCluster;
  cluster_id: string;
  namespace: string;
  workload: string;
  /** Empty in workload grouping. */
  pod: string;
  /** Distinct pods seen in the window (1 in pod grouping). */
  pods: number;
  restarts: K8sRestartCounts;
  /** Planned pod replacements. */
  churn: number;
  /** Bytes: latest sample summed over pods / sum of per-pod averages / hungriest pod. */
  mem_last: number;
  mem_avg: number;
  mem_max: number;
  rps: number;
  err_pct: number;
  latency_kind: 'p95' | 'avg' | '';
  latency_ms: number;
}

export interface K8sFleetTable {
  window: string;
  group: K8sFleetGroup;
  sort: K8sFleetSortKey;
  dir: 'asc' | 'desc';
  total: number;
  offset: number;
  rows: K8sFleetRow[];
}

export type K8sFleetMetric = 'restarts' | 'mem' | 'rps' | 'err' | 'latency';
export type K8sFleetSeriesBy = 'cluster' | 'namespace' | 'workload' | 'pod';

export interface K8sFleetSeries {
  window: string;
  metric: K8sFleetMetric;
  unit: 'count' | 'bytes' | 'rate' | 'percent' | 'ms';
  /** `class` for restarts (always per class), else the requested `by`. */
  by: K8sFleetSeriesBy | 'class';
  step_secs: number;
  series: { key: string; label: string; points: { t: string; v: number }[] }[];
}

export type K8sFleetEventSort = 'ts' | 'cluster' | 'namespace' | 'workload' | 'pod' | 'kind' | 'class' | 'reason';

export interface K8sFleetEvent extends K8sMonitorEvent {
  cluster_id: string;
  cluster: K8sFleetCluster;
}

export interface K8sFleetEvents {
  window: string;
  sort: K8sFleetEventSort;
  dir: 'asc' | 'desc';
  total: number;
  offset: number;
  rows: K8sFleetEvent[];
}

export interface K8sFleetRequestRow {
  path: string;
  method: string;
  rps: number;
  err_pct: number;
  avg_ms: number;
}

export interface K8sFleetRequests {
  window: string;
  /** Clusters whose monitor config keeps request labels (the only ones that can have rows). */
  enabled_on: { id: string; name: string }[];
  disabled_on: { id: string; name: string }[];
  rows: K8sFleetRequestRow[];
}

/** The `k8s_health` digest (`GET /k8s/clusters/{id}/monitor/health`). */
export interface K8sHealthDigest {
  cluster: string;
  cluster_id: Id;
  environment?: Environment;
  window: string;
  collected_at?: string | null;
  collector: {
    enabled: boolean;
    ok: boolean;
    last_ok_at?: string | null;
    error: string;
    transport?: string;
    metrics_server?: string;
    pods_seen?: number;
    pods_scraped?: number;
    pods_failed?: number;
    cycle_ms?: number;
  };
  pods?: { running: number; pending: number; failed: number; succeeded: number; crashloop: number; total: number };
  unplanned_restarts?: number;
  restarts?: Record<'oom' | 'crash' | 'probe' | 'unknown', Record<string, unknown>[]>;
  churn?: { workload: string; class: string; pods: number; by: string }[];
  /** Workloads whose dominant version changed in the window ("a new version came up"). */
  deployments?: { namespace: string; workload: string; from: string; to: string; at: string }[];
  memory_outliers?: Record<string, unknown>[];
  error_rate?: Record<string, unknown>[];
  latency?: Record<string, unknown>[];
  drift?: { namespace: string; workload: string; versions: string[] }[];
  thresholds?: Record<string, number>;
}

// Per-resource authorization: feature access, resource visibility, then operations.
export type ResourceKind = 'connection' | 'mcp_server' | 'aws_account' | 'k8s_cluster';
export type AccessMode = 'legacy' | 'enforced';
export interface AccessRule {
  id: Id; subject_kind: 'user' | 'group'; subject_id: Id; effect: 'allow' | 'deny';
  operations: string[]; children: string[] | null; grantable_operations: string[];
  credential_connection_id?: Id | null;
}
export interface AccessPolicy { kind: ResourceKind; resource_id: Id; mode: AccessMode; revision: number; rules: AccessRule[] }
export interface AccessDecision { allowed: boolean; reason: string; matched_rule_ids: Id[]; mode: AccessMode }
export interface EffectiveAccessBatchRequest { children: string[] }
export interface EffectiveAccess { kind: ResourceKind; resource_id: Id; user_id: Id; child: string | null; mode: AccessMode; operations: Record<string, AccessDecision> }
export interface AccessGroup { id: Id; name: string; description: string | null; created_at: string; updated_at: string }
export interface AccessRole { id: Id; name: string; description: string | null; kind: ResourceKind; operations: string[]; grantable_operations: string[]; created_at: string; updated_at: string }
export interface AccessSubjects { users: Pick<User, 'id' | 'username' | 'display_name'>[]; groups: AccessGroup[]; roles: AccessRole[] }
export interface AccessPreview { token: string; revision: number; issues: string[]; changes: {user_id: Id; display_name: string; before: Record<string, AccessDecision>; after: Record<string, AccessDecision>; children: {child: string; before: Record<string, AccessDecision>; after: Record<string, AccessDecision>}[]}[] }

// Independently reviewed database-change artifacts and durable target attempts.
export type ChangeTarget = {connection_id:string;node:string};
export type ChangeInput = {title:string;description:string;script:string;targets:ChangeTarget[]};
export type DatabaseChange = ChangeInput & {id:string;author_id:string;real_author_id:string;revision:number;status:string;content_hash:string;executor_id:string|null;validation:unknown;approved_by:string|null;approved_real_by:string|null;approval_hash:string|null;cancellation_requested:boolean;created_at:string;updated_at:string};
export type ChangeAttempt = {id:string;change_id:string;connection_id:string;node:string|null;state:string;executor_id:string;ordinal:number;summary:string|null;started_at:string|null;finished_at:string|null};
export type ChangeEvent = {id:string;revision:number;action:string;actor_id:string;real_actor_id:string;data:unknown;created_at:string};
export type ChangeDetail = {change:DatabaseChange;attempts:ChangeAttempt[];history:ChangeEvent[]};

// Git recovery tools (otto-git/recovery.rs).
export interface GitRecoveryEntry { sha: string; selector: string; subject: string }
export interface GitPlanCommit { sha: string; subject: string; action: 'pick' | 'squash' | 'edit' }
export interface GitInteractivePlan { head_sha: string; onto_sha: string; base_sha: string; commits: GitPlanCommit[] }
export interface GitBisectState {
  active: boolean; current_sha: string; current_subject: string;
  finished: boolean; first_bad: string | null; remaining: number | null;
  log: string; output: string;
}

export interface StartApiAutomationRunReq {
  environment_id?: Id | null;
  stop_on_failure?: boolean;
  dataset?: Record<string, unknown>[];
}
export interface ApiAutomationRun {
  id: Id; workspace_id: Id; automation_id: Id; environment_id: Id | null;
  created_by: Id; status: 'running' | 'passed' | 'failed' | 'cancelled' | 'interrupted';
  created_at: string; finished_at: string | null; stop_on_failure: boolean;
  dataset_rows: number; snapshot: unknown; report: ApiRunResult; result_rows: number[]; result_ids: Id[]; error: string | null;
  /** Run-LIST rows only: completed / passed step counts. List rows carry no
   *  `report.steps` and no `snapshot` — fetch the run by id for those. */
  steps_total?: number; steps_passed?: number;
}

/** Portable saved-data archive. Authentication secrets and live processes are excluded. */
export interface StateArchive {
  archive_format: 2;
  schema_version: number;
  daemon_version: string;
  snapshot_at: string;
  records: Record<string, Record<string, unknown>[]>;
  roots: { id: string; kind: string; owner_id?: string }[];
  files: { root: string; path: string; sha256: string; content_base64: string }[];
  excluded: string[];
  reconnect: string[];
}
export type RestoreConflictPolicy = 'abort' | 'skip_existing';
export interface RestorePreview {
  preview_token: string;
  can_restore: boolean;
  record_count: number;
  file_count: number;
  table_counts: Record<string, number>;
  conflicts: { kind: 'record' | 'file'; location: string; reason: string }[];
  excluded: string[];
  reconnect: string[];
  warnings: string[];
}
export interface RestoreResult {
  records_inserted: number;
  records_skipped: number;
  files_restored: number;
  files_skipped: number;
  restore_root: string;
  reconnect: string[];
}

export interface GitBackupStatus {
  repo_path: string;
  head: string | null;
  branch: string | null;
  upstream: string | null;
  ahead: number;
  behind: number;
  dirty: boolean;
  remotes: string[];
}
export interface GitBackupPreview {
  token: string;
  snapshot_digest: string;
  status: GitBackupStatus;
  changes: { path: string; action: 'added' | 'modified' | 'removed' | 'unchanged'; bytes: number }[];
  excluded: string[];
  reconnect: string[];
}

export interface ConnectionExportFormat {
  id: string;
  label: string;
  kinds: string[];
  password_support: 'native' | 'sidecar';
  description: string;
  import_instructions: string;
}
export interface ConnectionExportResult {
  format: string;
  total_connections: number;
  exported_connections: number;
  contains_passwords: boolean;
  files: { name: string; mime: string; content: string }[];
  skipped: { id: string; name: string; kind: string; reason: string }[];
  warnings: string[];
  import_instructions: string;
}

/** Conditional workflow summaries use 200 for both changed and unchanged. */
export type WorkflowProgressResponse = {changed: false; rev: number} | {changed: true; rev: number; run: WorkflowRun};
export interface WorkflowCheckpointPage {
  checkpoint_rev: number;
  generation: number;
  items: WorkflowCheckpoint[];
  next_cursor: string | null;
}
export interface WorkflowDetail<T> {rev: number; detail_version: string; body: T}


/** Common project identity/context, including existing Swarm projects. */
export interface Project {
  id: Id;
  workspace_id: Id;
  swarm_id: Id | null;
  name: string;
  description: string;
  repo_path: string | null;
  goal_md: string;
  instructions_md: string;
  references: string[];
  memory_md: string;
  decisions_md: string;
  artifacts: string[];
  status: 'active' | 'archived';
  context_version: number;
  created_by: Id;
  created_at: string;
  updated_at: string;
}
export type ProjectInput = Pick<Project, 'name' | 'description' | 'repo_path' | 'goal_md' | 'instructions_md' | 'references' | 'memory_md' | 'decisions_md' | 'artifacts' | 'status'>;

/** Versioned user-editable personal-agent document. Memory has a resolved path;
 * context is stored separately in SQLite. Send version unchanged on PUT. */
export interface PersonalAgentDocument {
  content: string;
  version: string;
  exists: boolean;
  path: string | null;
}

/** Explicit, session-scoped SSH TCP forwards. No transparent VPN/proxy routing. */
export interface NetworkEndpoint {
  name: string;
  remote_host: string;
  remote_port: number;
  host_env?: string | null;
  port_env?: string | null;
}
export interface NetworkProfileInput {
  name: string;
  ssh_connection_id: Id;
  endpoints: NetworkEndpoint[];
  archived?: boolean;
}
export interface NetworkProfile extends NetworkProfileInput {
  id: Id;
  workspace_id: Id;
  archived: boolean;
  version: number;
  created_by: Id;
  created_at: string;
  updated_at: string;
}
export interface SessionNetwork {
  profile_id: Id | null;
  profile_name: string | null;
  profile_version: number | null;
  selected_profile_id: Id | null;
  restart_required: boolean;
  status: 'connected' | 'error' | 'stopped' | 'disabled';
  error: string | null;
  endpoints: (NetworkEndpoint & {host: string; port: number})[];
}

/** `GET /k8s/monitor/fleet/series/batch?metrics=a,b,…` (≤ 8 metrics): one
 *  `K8sFleetSeries` per requested metric, keyed by metric id. */
export interface K8sFleetSeriesBatch {
  series: Partial<Record<K8sFleetMetric, K8sFleetSeries>>;
}
