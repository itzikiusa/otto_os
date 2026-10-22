// Contract mirror: k8s. Part of lib/api/types (see ../types.ts, the barrel).
// Mirrors docs/contracts/api.md + ws.md — change them together.

import type {
  Environment,
} from './connections';
import type {
  Id,
} from './core';

// ---------------------------------------------------------------------------
// Kubernetes console (mirror of crates/otto-k8s — see
// docs/design/aws-k8s-consoles.md §3; keep in lockstep)
// ---------------------------------------------------------------------------

/** One of the CLIs the daemon can install on demand. */
export type K8sTool = 'kubectl' | 'k9s';

/** Background installer state for one tool (shared shape with the AWS
 *  console's `InstallJob`; prefixed here so the two type sections never
 *  collide). */
export interface K8sInstallJob {
  tool: 'aws' | K8sTool;
  state: 'idle' | 'running' | 'done' | 'failed';
  log_tail: string;
  started_at?: string | null;
  finished_at?: string | null;
  error?: string | null;
}

/** `GET /k8s/status` per-binary presence. */
export interface K8sToolStatus {
  installed: boolean;
  version?: string | null;
  path?: string | null;
}

/** `GET /k8s/status`. */
export interface K8sStatus {
  kubectl: K8sToolStatus;
  k9s: K8sToolStatus;
  install: { kubectl: K8sInstallJob; k9s: K8sInstallJob };
}

/** One context found in `~/.kube/config` / `$KUBECONFIG` (`GET /k8s/discover`).
 *  Never carries credentials. */
export interface K8sDiscoveredContext {
  name: string;
  cluster: string;
  user: string;
  namespace?: string | null;
  kubeconfig_path: string;
  server?: string | null;
}

export interface K8sDiscoverResp {
  contexts: K8sDiscoveredContext[];
}

/** Where a cluster row's kubeconfig comes from: the user's own file
 *  (`kubeconfig`), a pasted file Otto owns (`imported`), or an EKS import
 *  (`eks`, linked to an AWS account). */
export type K8sClusterSource = 'kubeconfig' | 'imported' | 'eks';

/** Cached capability probe (`GET /k8s/clusters/{id}/capabilities`). */
export interface K8sCapabilities {
  server_version?: string | null;
  metrics_server: boolean;
  argo_rollouts: boolean;
  argocd: boolean;
  checked_at: string;
}

export interface K8sCluster {
  id: Id;
  name: string;
  source: K8sClusterSource;
  kubeconfig_path?: string | null;
  context_name: string;
  default_namespace?: string | null;
  aws_account_id?: Id | null;
  environment: Environment;
  color?: string | null;
  /** Persisted namespaces offered in the picker even when the cluster forbids listing them. */
  known_namespaces?: string[];
  /** Non-secret extras (`eks_region`, `eks_cluster`, …); always sent (`color` is lifted out
   *  of it into its own field). */
  params: Record<string, unknown>;
  capabilities?: K8sCapabilities | null;
  /** `null` for a row with no recorded creator (Rust `Option<Id>`, serialized as null). */
  created_by: Id | null;
  created_at: string;
  updated_at: string;
  last_used_at?: string | null;
}

/** `POST /k8s/clusters` (create from an existing kubeconfig context) and the
 *  partial body of `PATCH /k8s/clusters/{id}`. */
export interface UpsertK8sClusterReq {
  name: string;
  source: 'kubeconfig';
  kubeconfig_path?: string | null;
  context_name: string;
  default_namespace?: string | null;
  environment?: Environment;
  color?: string | null;
  known_namespaces?: string[];
}

/** `POST /k8s/clusters/import` — a pasted kubeconfig Otto stores itself. */
export interface ImportK8sClusterReq {
  name: string;
  kubeconfig_yaml: string;
  context_name?: string | null;
  default_namespace?: string | null;
  environment?: Environment;
  color?: string | null;
}

/** `POST /k8s/clusters/{id}/test`. */
export interface K8sTestResp {
  ok: boolean;
  latency_ms: number;
  message: string;
  server_version?: string | null;
}

export interface K8sNamespace {
  name: string;
  status: string;
  age_seconds: number;
}

export interface K8sNode {
  name: string;
  status: string;
  roles: string;
  version: string;
  /** Millicores (allocatable). */
  cpu_capacity: number;
  /** Bytes (allocatable). */
  mem_capacity: number;
  /** Millicores from metrics-server; null when unavailable. */
  cpu_usage?: number | null;
  /** Bytes from metrics-server; null when unavailable. */
  mem_usage?: number | null;
  age_seconds: number;
}

/** Resource kinds the `/resources` endpoint understands. `nodes` is served by
 *  its own endpoint but shares the UI kinds list. */
export type K8sResourceKind =
  | 'pods'
  | 'deployments'
  | 'statefulsets'
  | 'daemonsets'
  | 'replicasets'
  | 'jobs'
  | 'cronjobs'
  | 'services'
  | 'ingresses'
  | 'configmaps'
  | 'secrets'
  | 'pvcs'
  | 'hpas'
  | 'rollouts'
  | 'applications'
  | 'events'
  | 'nodes';

export type K8sHealth = 'ok' | 'warn' | 'bad' | 'progressing';

/** One normalized table row. `extra` carries kind-specific columns
 *  (deployments: desired/updated/available; rollouts: strategy/phase/step/
 *  weight; applications: sync/health/revision; secrets: type/keys — never
 *  values). */
export interface K8sRow {
  name: string;
  namespace: string;
  kind: string;
  status: string;
  ready?: string | null;
  restarts?: number | null;
  age_seconds: number;
  /** perf R1: unix seconds the age counts from (creationTimestamp; an
   *  event's last-seen time). Render Age as `now - created_at` (`rowAge`) —
   *  `age_seconds` is frozen at list time and a 304 keeps the old rows. */
  created_at?: number | null;
  node?: string | null;
  ip?: string | null;
  /** Millicores (metrics-server); null when unavailable. Format client-side. */
  cpu?: number | null;
  /** Bytes (metrics-server); null when unavailable. Format client-side. */
  mem?: number | null;
  images?: string[] | null;
  labels: Record<string, string>;
  extra: Record<string, string>;
  health?: K8sHealth | null;
}

export interface K8sResourcesResp {
  kind: string;
  items: K8sRow[];
  has_metrics: boolean;
}

export interface K8sEvent {
  type: string;
  reason: string;
  message: string;
  count: number;
  last_seen: string;
}

/** `GET /k8s/clusters/{id}/resource` — manifest has `managedFields` stripped
 *  and Secret data replaced by `<redacted>`. */
export interface K8sResourceDetail {
  manifest: unknown;
  describe: string;
  events: K8sEvent[];
}

export interface K8sContainer {
  name: string;
  image: string;
  ready: boolean;
  state: string;
  restarts: number;
  init: boolean;
}

export interface K8sContainersResp {
  containers: K8sContainer[];
}

/** Query options for `GET …/pods/{ns}/{name}/logs`. */
export interface K8sLogsOpts {
  container?: string;
  tail?: number;
  since?: string;
  previous?: boolean;
  follow?: boolean;
  timestamps?: boolean;
}

/** What a logs stream reads from: one pod, or every pod matching a
 *  workload's label selector (`row.extra.selector`) — the latter arrives with
 *  a `[pod/<pod>/<container>] ` prefix on every line. */
export type K8sLogTarget = { pod: string } | { selector: string };

export interface K8sContainerMetrics {
  name: string;
  cpu_millicores: number;
  mem_bytes: number;
}

export interface K8sPodMetrics {
  name: string;
  namespace: string;
  cpu_millicores: number;
  mem_bytes: number;
  containers: K8sContainerMetrics[];
}

export interface K8sMetricsResp {
  pods: K8sPodMetrics[];
  available: boolean;
}

/** `POST /k8s/clusters/{id}/exec` → `Session` (PTY running `kubectl exec -it`). */
export interface K8sExecReq {
  workspace_id: Id;
  ns: string;
  pod: string;
  container?: string | null;
  command?: string[] | null;
}

/** `POST /k8s/clusters/{id}/k9s` → `Session`. */
export interface K8sK9sReq {
  workspace_id: Id;
  ns?: string | null;
}

/** Mutating actions (`POST /k8s/clusters/{id}/actions`, §4.6). */
export type K8sAction =
  | 'restart'
  | 'scale'
  | 'delete_pod'
  | 'rollout_status'
  | 'rollout_undo'
  | 'rollout_pause'
  | 'rollout_resume'
  | 'rollout_promote'
  | 'rollout_abort'
  | 'rollout_retry'
  | 'argocd_sync'
  | 'argocd_refresh'
  | 'argocd_terminate_op'
  | 'argocd_app_restart'
  | 'cronjob_trigger'
  | 'cronjob_suspend'
  | 'cronjob_resume';

/** Destructive actions (`delete_pod`, `scale` to 0, `rollout_undo`,
 *  `argocd_sync` with prune) must carry `params.confirm_name === name`. */
export interface K8sActionReq {
  action: K8sAction;
  kind: K8sResourceKind;
  ns: string;
  name: string;
  params?: Record<string, unknown>;
}

export interface K8sActionResp {
  ok: boolean;
  message: string;
  output?: string | null;
}

// --- Kubernetes pod HTTP actions (K-3; docs/contracts/api.md "Pod HTTP actions") ---

export type K8sPodHttpMethod = 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE';

/** `POST /k8s/clusters/{id}/pod-http` — exactly one of `pod` / `workload`. */
export interface K8sPodHttpReq {
  namespace: string;
  pod?: string;
  /** kind: deployment | statefulset | daemonset | replicaset | job. */
  workload?: { kind: string; name: string };
  port: number;
  method: K8sPodHttpMethod;
  /** Starts with `/`; no `..`, no `://`, no whitespace. */
  path: string;
  headers?: Record<string, string>;
  body?: string | null;
  /** Default 10000, max 30000. */
  timeout_ms?: number;
  /** Default 4, max 8. */
  max_concurrency?: number;
  /** Prod / read-only clusters: a mutating call must echo the target name. */
  confirm_name?: string;
}

export interface K8sPodHttpResult {
  pod: string;
  /** null when the request never got an HTTP response (`error` says why). */
  status: number | null;
  duration_ms: number;
  /** Authorization / Cookie / Set-Cookie are redacted. */
  headers: Record<string, string>;
  /** Capped at 256 KB; base64 when the body isn't UTF-8. */
  body: string;
  body_base64: boolean;
  truncated: boolean;
  error: string | null;
  via: 'proxy' | 'port_forward';
}

export interface K8sPodHttpResp {
  results: K8sPodHttpResult[];
  target_name: string;
  mutating: boolean;
}

/** A saved per-workload pod HTTP action (`/k8s/clusters/{id}/pod-actions`).
 *  `{{var}}` placeholders in path / body are filled in by the client. */
export interface K8sPodAction {
  id: string;
  cluster_id: string;
  namespace: string;
  /** Plural wire kind, e.g. "deployments" (inputs accept singular aliases). */
  workload_kind: string;
  workload: string;
  name: string;
  method: K8sPodHttpMethod;
  port: number;
  path: string;
  headers: Record<string, string>;
  body_template: string | null;
  created_by: string | null;
  updated_at: string;
}

export interface K8sPodActionInput {
  id?: string;
  namespace: string;
  workload_kind: string;
  workload: string;
  name: string;
  method: K8sPodHttpMethod;
  port: number;
  path: string;
  headers?: Record<string, string>;
  body_template?: string | null;
}

// --- perf K8s (UI half of review K2/K6) ---------------------------------------
// Declaration-merged into the interfaces above so the additions stay in one block.

export interface K8sResourcesResp {
  /** Opaque version of this list (contract K2): also the quoted `ETag`; send it
   *  back as `If-None-Match` and an unchanged list answers `304` (no body).
   *  Optional so an older daemon's body still type-checks. */
  version?: string;
}
