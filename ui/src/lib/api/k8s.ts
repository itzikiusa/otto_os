// Kubernetes console API client — thin typed wrappers over the generic `api`
// helper for every `/k8s/*` route in docs/design/aws-k8s-consoles.md §3, plus
// a streaming helper for `kubectl logs -f` (a chunked `text/plain` body the
// JSON-parsing `request()` cannot read).

import { api, ApiError, baseUrl, getToken } from './client';
import type {
  ImportK8sClusterReq,
  K8sActionReq,
  K8sActionResp,
  K8sCapabilities,
  K8sCluster,
  K8sContainersResp,
  K8sDiscoverResp,
  K8sExecReq,
  K8sInstallJob,
  K8sK9sReq,
  K8sLogsOpts,
  K8sMetricsResp,
  K8sNamespace,
  K8sNode,
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
  metrics: (id: string, ns?: string, signal?: AbortSignal) =>
    api.get<K8sMetricsResp>(`/k8s/clusters/${enc(id)}/metrics${qs({ ns: ns ?? '' })}`, signal),

  // --- writes (Edit) ----------------------------------------------------------
  exec: (id: string, body: K8sExecReq) => api.post<Session>(`/k8s/clusters/${enc(id)}/exec`, body),
  k9s: (id: string, body: K8sK9sReq) => api.post<Session>(`/k8s/clusters/${enc(id)}/k9s`, body),
  action: (id: string, body: K8sActionReq) =>
    api.post<K8sActionResp>(`/k8s/clusters/${enc(id)}/actions`, body),

  /** The logs URL (relative to `/api/v1`) for a non-follow fetch/download. */
  logsPath: (id: string, ns: string, pod: string, opts: K8sLogsOpts = {}): string =>
    `/k8s/clusters/${enc(id)}/pods/${enc(ns)}/${enc(pod)}/logs${qs({
      container: opts.container || undefined,
      tail: opts.tail,
      since: opts.since || undefined,
      previous: opts.previous || undefined,
      follow: opts.follow || undefined,
      timestamps: opts.timestamps || undefined,
    })}`,
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
  pod: string,
  opts: K8sLogsOpts,
  onChunk: (text: string) => void,
  signal?: AbortSignal,
): Promise<void> {
  const headers: Record<string, string> = { Accept: 'text/plain' };
  const token = getToken();
  if (token) headers['Authorization'] = `Bearer ${token}`;
  let resp: Response;
  try {
    resp = await fetch(`${baseUrl()}/api/v1${k8sApi.logsPath(clusterId, ns, pod, opts)}`, {
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
