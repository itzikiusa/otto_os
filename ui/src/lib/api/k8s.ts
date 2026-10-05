// Kubernetes console API client — thin typed wrappers over the generic `api`
// helper for every `/k8s/*` route in docs/design/aws-k8s-consoles.md §3, plus
// a streaming helper for `kubectl logs -f` (a chunked `text/plain` body the
// JSON-parsing `request()` cannot read).

import { api, ApiError, getToken, laneFetch, type Conditional } from './client';
import type {
  ImportK8sClusterReq,
  K8sActionReq,
  K8sActionResp,
  K8sCapabilities,
  K8sCluster,
  K8sContainersResp,
  K8sDiscoverResp,
  K8sExecReq,
  K8sFleetEvents,
  K8sFleetFilters,
  K8sFleetMetric,
  K8sFleetRequests,
  K8sFleetSeries,
  K8sFleetSeriesBatch,
  K8sFleetTable,
  K8sHealthDigest,
  K8sInstallJob,
  K8sK9sReq,
  K8sLogsOpts,
  K8sLogTarget,
  K8sMetricsResp,
  K8sMonitorConfig,
  K8sMonitorEvent,
  K8sMonitorOverviewRow,
  K8sMonitorResp,
  K8sMonitorSeries,
  K8sMonitorStatus,
  K8sMonitorTestResp,
  K8sMonitorWorkloadsResp,
  K8sNamespace,
  K8sNode,
  K8sPodAction,
  K8sPodActionInput,
  K8sPodHttpReq,
  K8sPodHttpResp,
  K8sResourceDetail,
  K8sResourceKind,
  K8sResourcesResp,
  K8sStatus,
  K8sTestResp,
  K8sTool,
  Problem,
  Session,
  UpsertK8sClusterReq,
} from './types';

const enc = encodeURIComponent;

/** Build `?k=v&…` from the defined entries only (empty strings are kept —
 *  `ns=` empty means "all namespaces" on the resources endpoint). */
function qs(params: Record<string, string | number | boolean | undefined | null>): string {
  const u = new URLSearchParams();
  for (const [k, v] of Object.entries(params)) {
    if (v === undefined || v === null) continue;
    u.set(k, String(v));
  }
  const s = u.toString();
  return s ? `?${s}` : '';
}

export const k8sApi = {
  // --- plumbing ---------------------------------------------------------------
  status: () => api.get<K8sStatus>('/k8s/status'),
  install: (tool: K8sTool) => api.post<K8sInstallJob>('/k8s/install', { tool }),
  discover: () => api.get<K8sDiscoverResp>('/k8s/discover'),

  // --- clusters ---------------------------------------------------------------
  listClusters: () => api.get<K8sCluster[]>('/k8s/clusters'),
  createCluster: (body: UpsertK8sClusterReq) => api.post<K8sCluster>('/k8s/clusters', body),
  importCluster: (body: ImportK8sClusterReq) =>
    api.post<K8sCluster>('/k8s/clusters/import', body),
  getCluster: (id: string) => api.get<K8sCluster>(`/k8s/clusters/${enc(id)}`),
  updateCluster: (id: string, body: Partial<UpsertK8sClusterReq>) =>
    api.patch<K8sCluster>(`/k8s/clusters/${enc(id)}`, body),
  deleteCluster: (id: string) => api.del<void>(`/k8s/clusters/${enc(id)}`),
  testCluster: (id: string) => api.post<K8sTestResp>(`/k8s/clusters/${enc(id)}/test`, {}),
  capabilities: (id: string, refresh = false) =>
    api.get<K8sCapabilities>(`/k8s/clusters/${enc(id)}/capabilities${qs({ refresh: refresh || undefined })}`),

  // --- reads (View) -----------------------------------------------------------
  namespaces: (id: string, signal?: AbortSignal) =>
    api.get<{ namespaces: K8sNamespace[] }>(`/k8s/clusters/${enc(id)}/namespaces`, signal),
  nodes: (id: string, signal?: AbortSignal) =>
    api.get<{ nodes: K8sNode[] }>(`/k8s/clusters/${enc(id)}/nodes`, signal),
  /** `ns` empty ⇒ all namespaces (`-A`). */
  resources: (
    id: string,
    kind: K8sResourceKind,
    opts: { ns?: string; label?: string; q?: string } = {},
    signal?: AbortSignal,
  ) =>
    api.get<K8sResourcesResp>(
      `/k8s/clusters/${enc(id)}/resources${qs({ kind, ns: opts.ns ?? '', label: opts.label || undefined, q: opts.q || undefined })}`,
      signal,
    ),
  /** perf K8s: {@link resources} as a conditional GET. `ifNoneMatch` is the
   *  validator of the rows the caller holds (`resourcesValidator`,
   *  modules/kubernetes/resourcePoll.ts);
   *  an unchanged list resolves `{ notModified: true }` (304, nothing parsed).
   *  Same path ⇒ same lane / connection pool as `resources`. */
  resourcesIfChanged: (
    id: string,
    kind: K8sResourceKind,
    opts: { ns?: string; label?: string; q?: string } = {},
    ifNoneMatch: string | null,
    signal?: AbortSignal,
  ): Promise<Conditional<K8sResourcesResp>> =>
    api.getConditional<K8sResourcesResp>(
      `/k8s/clusters/${enc(id)}/resources${qs({ kind, ns: opts.ns ?? '', label: opts.label || undefined, q: opts.q || undefined })}`,
      ifNoneMatch,
      signal,
    ),
  resource: (id: string, kind: K8sResourceKind, ns: string, name: string, signal?: AbortSignal) =>
    api.get<K8sResourceDetail>(
      `/k8s/clusters/${enc(id)}/resource${qs({ kind, ns, name })}`,
      signal,
    ),
  containers: (id: string, ns: string, pod: string, signal?: AbortSignal) =>
    api.get<K8sContainersResp>(
      `/k8s/clusters/${enc(id)}/pods/${enc(ns)}/${enc(pod)}/containers`,
      signal,
    ),
  /** `pod` (with `ns`) narrows the answer to that one pod — the drawer's
   *  Metrics tab must not pull a whole namespace's metrics every 10 s. */
  metrics: (id: string, ns?: string, signal?: AbortSignal, pod?: string) =>
    api.get<K8sMetricsResp>(`/k8s/clusters/${enc(id)}/metrics${qs({ ns: ns ?? '', pod: pod || undefined })}`, signal),

  // --- monitoring (contract "Kubernetes monitoring") ---
  monitor: (id: string) => api.get<K8sMonitorResp>(`/k8s/clusters/${enc(id)}/monitor`),
  monitorSave: (id: string, body: K8sMonitorConfig) =>
    api.put<K8sMonitorResp>(`/k8s/clusters/${enc(id)}/monitor`, body),
  monitorTest: (id: string, body: { ns?: string; pod?: string }) =>
    api.post<K8sMonitorTestResp>(`/k8s/clusters/${enc(id)}/monitor/test`, body),
  monitorRun: (id: string) => api.post<K8sMonitorStatus>(`/k8s/clusters/${enc(id)}/monitor/run`, {}),
  monitorOverview: (window = '24h', signal?: AbortSignal) =>
    api.get<K8sMonitorOverviewRow[]>(`/k8s/monitor/overview${qs({ window })}`, signal),
  monitorWorkloads: (id: string, window: string, ns?: string, signal?: AbortSignal) =>
    api.get<K8sMonitorWorkloadsResp>(`/k8s/clusters/${enc(id)}/monitor/workloads${qs({ window, ns: ns || undefined })}`, signal),
  monitorSeries: (id: string, p: { metric: string; workload?: string; pod?: string; ns?: string; window: string; step?: number }, signal?: AbortSignal) =>
    api.get<K8sMonitorSeries>(`/k8s/clusters/${enc(id)}/monitor/series${qs(p)}`, signal),
  monitorEvents: (id: string, p: { window: string; class?: string; workload?: string; ns?: string; limit?: number }, signal?: AbortSignal) =>
    api.get<K8sMonitorEvent[]>(`/k8s/clusters/${enc(id)}/monitor/events${qs(p)}`, signal),
  monitorHealth: (id: string, window = '1h') =>
    api.get<K8sHealthDigest>(`/k8s/clusters/${enc(id)}/monitor/health${qs({ window })}`),

  // --- fleet dashboard (ClickHouse-only, cross-cluster; contract "Fleet dashboard") ---
  // `cluster` is a comma-separated id list (empty = all); ns / workload / pod narrow it.
  fleetFilters: (p: { window: string; cluster?: string; ns?: string; workload?: string; pod?: string }, signal?: AbortSignal) =>
    api.get<K8sFleetFilters>(`/k8s/monitor/fleet/filters${qs(p)}`, signal),
  fleetTable: (
    p: { window: string; cluster?: string; ns?: string; workload?: string; pod?: string; group?: string; sort?: string; dir?: string; limit?: number; offset?: number },
    signal?: AbortSignal,
  ) => api.get<K8sFleetTable>(`/k8s/monitor/fleet/table${qs(p)}`, signal),
  fleetSeries: (
    p: { window: string; metric: string; by?: string; step?: number; cluster?: string; ns?: string; workload?: string; pod?: string },
    signal?: AbortSignal,
  ) => api.get<K8sFleetSeries>(`/k8s/monitor/fleet/series${qs(p)}`, signal),
  /** perf K8s: every overview metric in ONE request (`metrics` ≤ 8) — one
   *  socket and one server cache key instead of a `fleetSeries` per chart. */
  fleetSeriesBatch: (
    p: { window: string; metrics: K8sFleetMetric[]; by?: string; step?: number; cluster?: string; ns?: string; workload?: string; pod?: string },
    signal?: AbortSignal,
  ) => api.get<K8sFleetSeriesBatch>(`/k8s/monitor/fleet/series/batch${qs({ ...p, metrics: p.metrics.join(',') })}`, signal),
  fleetEvents: (
    p: { window: string; cluster?: string; ns?: string; workload?: string; pod?: string; class?: string; sort?: string; dir?: string; limit?: number; offset?: number },
    signal?: AbortSignal,
  ) => api.get<K8sFleetEvents>(`/k8s/monitor/fleet/events${qs(p)}`, signal),
  fleetRequests: (p: { window: string; cluster?: string; ns?: string; workload?: string; pod?: string }, signal?: AbortSignal) =>
    api.get<K8sFleetRequests>(`/k8s/monitor/fleet/requests${qs(p)}`, signal),

  // --- writes (Edit) ----------------------------------------------------------
  exec: (id: string, body: K8sExecReq) => api.post<Session>(`/k8s/clusters/${enc(id)}/exec`, body),
  k9s: (id: string, body: K8sK9sReq) => api.post<Session>(`/k8s/clusters/${enc(id)}/k9s`, body),
  action: (id: string, body: K8sActionReq) =>
    api.post<K8sActionResp>(`/k8s/clusters/${enc(id)}/actions`, body),

  /** The logs URL (relative to `/api/v1`) for a non-follow fetch/download.
   *  A `{ selector }` target hits the workload-level route (every matching
   *  pod, `[pod/<pod>/<container>] `-prefixed lines). */
  logsPath: (id: string, ns: string, target: string | K8sLogTarget, opts: K8sLogsOpts = {}): string => {
    const t: K8sLogTarget = typeof target === 'string' ? { pod: target } : target;
    const common = {
      container: opts.container || undefined,
      tail: opts.tail,
      since: opts.since || undefined,
      previous: opts.previous || undefined,
      follow: opts.follow || undefined,
      timestamps: opts.timestamps || undefined,
    };
    return 'pod' in t
      ? `/k8s/clusters/${enc(id)}/pods/${enc(ns)}/${enc(t.pod)}/logs${qs(common)}`
      : `/k8s/clusters/${enc(id)}/logs${qs({ ns, selector: t.selector, ...common })}`;
  },

  // --- pod HTTP actions (K-3; contract "Pod HTTP actions") ---
  // One pod or every pod of a workload, through the kubectl-proxy gateway
  // (port-forward fallback). Mutating methods need Edit; on prod they also
  // need `confirm_name` = the target name (409 `confirm_required` otherwise).
  podHttp: (id: string, body: K8sPodHttpReq) =>
    api.post<K8sPodHttpResp>(`/k8s/clusters/${enc(id)}/pod-http`, body),
  podActions: (id: string, p: { namespace?: string; workload_kind?: string; workload?: string } = {}) =>
    api.get<{ actions: K8sPodAction[] }>(`/k8s/clusters/${enc(id)}/pod-actions${qs(p)}`),
  savePodAction: (id: string, body: K8sPodActionInput) =>
    api.put<K8sPodAction>(`/k8s/clusters/${enc(id)}/pod-actions`, body),
  deletePodAction: (id: string, actionId: string) =>
    api.del<void>(`/k8s/clusters/${enc(id)}/pod-actions/${enc(actionId)}`),
};

/**
 * Stream pod logs. Opens `GET …/logs` with the bearer token and reads the
 * `text/plain` body as a `ReadableStream`, invoking `onChunk` with each decoded
 * piece as it arrives (line-splitting is the caller's job — chunks can end
 * mid-line). With `opts.follow` the response stays open until `signal` aborts
 * (the daemon kills the `kubectl logs -f` child on disconnect); without it the
 * promise resolves once the body is drained. Throws `ApiError` on a non-2xx
 * status; an abort resolves silently.
 */
export async function followLogs(
  clusterId: string,
  ns: string,
  target: string | K8sLogTarget,
  opts: K8sLogsOpts,
  onChunk: (text: string) => void,
  signal?: AbortSignal,
): Promise<void> {
  const headers: Record<string, string> = { Accept: 'text/plain' };
  const token = getToken();
  if (token) headers['Authorization'] = `Bearer ${token}`;
  let resp: Response;
  try {
    // `kubectl logs -f` holds its socket for as long as the view is open: the
    // long lane (alias host), so it never pins an interactive socket.
    resp = await laneFetch('long', k8sApi.logsPath(clusterId, ns, target, opts), {
      headers,
      signal,
    });
  } catch (e) {
    if (signal?.aborted) return;
    throw e;
  }
  if (!resp.ok) {
    let problem: Problem = { code: 'internal', message: resp.statusText };
    try {
      problem = await resp.json();
    } catch {
      // non-JSON error body — keep statusText
    }
    throw new ApiError(resp.status, problem);
  }
  if (!resp.body) {
    onChunk(await resp.text());
    return;
  }
  const reader = resp.body.getReader();
  const decoder = new TextDecoder();
  try {
    for (;;) {
      const { value, done } = await reader.read();
      if (done) break;
      const text = decoder.decode(value, { stream: true });
      if (text) onChunk(text);
    }
    const tail = decoder.decode();
    if (tail) onChunk(tail);
  } catch (e) {
    if (signal?.aborted) return;
    throw e;
  }
}
