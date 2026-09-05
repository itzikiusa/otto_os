// Kubernetes console store: tool status + installer polling, the cluster list
// (+ per-cluster capabilities), the workspace selection (cluster / kind /
// namespace / filter / selected row / drawer tab) and the resource-table cache
// with its auto-refresh timer. Like the other event-driven stores it does NOT
// import events.svelte.ts — the dispatcher calls `k8s.applyEvent(...)`.
//
// Gotchas mirrored from sftp.svelte.ts: nothing here mutates `$state` from a
// getter/`$derived` (the table reads `filteredRows`, a pure derived); every
// mutation happens in a method the page calls from an effect or a handler.

import { ApiError } from '../api/client';
import { formatBytes, formatMillicores } from '../../modules/kubernetes/k8s-util';
import { k8sApi } from '../api/k8s';
import type {
  ImportK8sClusterReq,
  K8sCapabilities,
  K8sCluster,
  K8sInstallJob,
  K8sNamespace,
  K8sNode,
  K8sResourceKind,
  K8sRow,
  K8sStatus,
  K8sTool,
  OttoEvent,
  UpsertK8sClusterReq,
} from '../api/types';

export type K8sDrawerTab =
  | 'overview'
  | 'manifest'
  | 'describe'
  | 'events'
  | 'logs'
  | 'terminal'
  | 'metrics'
  | 'pods';

/** A row identity inside the current cluster+kind (the route's `<ns>/<name>`). */
export interface K8sSelection {
  ns: string;
  name: string;
}

const NS_KEY = (clusterId: string): string => `otto_k8s_ns:${clusterId}`;
/** Namespaces this cluster is KNOWN to have — the default namespace plus every
 *  one the user selected and could read. Rancher project-scoped users can't
 *  `get namespaces` (cluster-scope list is forbidden), so without this the
 *  picker would offer nothing but "All namespaces" — which is forbidden too. */
const KNOWN_NS_KEY = (clusterId: string): string => `otto_k8s_known_ns:${clusterId}`;
const CLUSTER_SCOPE_HINT =
  'This kubeconfig user can\'t list across all namespaces (cluster scope). Pick a namespace (press n) — e.g. the cluster\'s default one.';
const AUTO_KEY = 'otto_k8s_autorefresh';
const AUTO_REFRESH_MS = 10_000;

function lsGet(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}
function lsSet(key: string, value: string): void {
  try {
    localStorage.setItem(key, value);
  } catch {
    /* private mode / quota — the preference just doesn't stick */
  }
}
function knownNamespaces(clusterId: string): string[] {
  try {
    const v = JSON.parse(lsGet(KNOWN_NS_KEY(clusterId)) ?? '[]');
    return Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string' && x !== '') : [];
  } catch {
    return [];
  }
}

/** Nodes come from their own endpoint; fold them into the shared row shape so
 *  the table renders every kind the same way. */
export function nodeToRow(n: K8sNode): K8sRow {
  const ready = n.status === 'Ready';
  return {
    name: n.name,
    namespace: '',
    kind: 'Node',
    status: n.status,
    age_seconds: n.age_seconds,
    cpu: n.cpu_usage ?? null,
    mem: n.mem_usage ?? null,
    labels: {},
    extra: {
      roles: n.roles,
      version: n.version,
      cpu_capacity: formatMillicores(n.cpu_capacity),
      mem_capacity: formatBytes(n.mem_capacity),
    },
    health: ready ? 'ok' : n.status === 'Unknown' ? 'warn' : 'bad',
  };
}

/** Case-insensitive substring match over the columns a user would scan
 *  (name, namespace, status, node, ip, extra values, images, labels). */
export function rowMatches(r: K8sRow, q: string): boolean {
  if (!q) return true;
  const hay = [
    r.name,
    r.namespace,
    r.status,
    r.node ?? '',
    r.ip ?? '',
    ...Object.values(r.extra ?? {}),
    ...(r.images ?? []),
    ...Object.entries(r.labels ?? {}).map(([k, v]) => `${k}=${v}`),
  ]
    .join('\n')
    .toLowerCase();
  return hay.includes(q);
}

class K8sStore {
  // --- tool status -------------------------------------------------------------
  status: K8sStatus | null = $state(null);
  statusLoading = $state(false);
  /** Non-empty when `/k8s/status` failed for a reason other than "backend
   *  route missing" (which sets `unavailable`). */
  statusError = $state('');
  /** True when `GET /k8s/status` 404s — the daemon predates the console. */
  unavailable = $state(false);

  // --- clusters ------------------------------------------------------------------
  clusters: K8sCluster[] = $state([]);
  clustersLoading = $state(false);
  clustersLoaded = $state(false);
  clustersError = $state('');
  /** cluster id → last probe (seeded from the row's cached `capabilities`). */
  capabilities: Record<string, K8sCapabilities> = $state({});

  // --- workspace selection ------------------------------------------------------
  clusterId: string | null = $state(null);
  kind: K8sResourceKind = $state('pods');
  /** '' = all namespaces. */
  namespace = $state('');
  namespaces: K8sNamespace[] = $state([]);
  namespacesError = $state('');
  filter = $state('');
  selected: K8sSelection | null = $state(null);
  drawerTab: K8sDrawerTab = $state('overview');
  autoRefresh = $state(lsGet(AUTO_KEY) !== '0');

  // --- resource cache -----------------------------------------------------------
  rows: K8sRow[] = $state([]);
  /** The (cluster, kind, ns) the cached rows belong to — the table shows a
   *  skeleton, not stale rows, when the selection moved on. */
  rowsKey = $state('');
  hasMetrics = $state(false);
  rowsLoading = $state(false);
  rowsError = $state('');
  rowsLoadedAt: number | null = $state(null);

  /** The k9s PTY session open in the workspace (full-pane terminal), if any. */
  k9sSessionId: string | null = $state(null);

  private rowsAbort: AbortController | null = null;
  private refreshTimer: ReturnType<typeof setInterval> | null = null;

  readonly cluster = $derived(this.clusters.find((c) => c.id === this.clusterId) ?? null);
  readonly caps = $derived(
    (this.clusterId ? this.capabilities[this.clusterId] : undefined) ?? null,
  );
  readonly currentKey = $derived(`${this.clusterId ?? ''}|${this.kind}|${this.namespace}`);
  /** Rows for the CURRENT selection only, narrowed by the free-text filter. */
  readonly filteredRows = $derived.by(() => {
    if (this.rowsKey !== this.currentKey) return [];
    const q = this.filter.trim().toLowerCase();
    return q ? this.rows.filter((r) => rowMatches(r, q)) : this.rows;
  });
  readonly selectedRow = $derived(
    this.selected
      ? (this.rows.find(
          (r) => r.name === this.selected!.name && r.namespace === this.selected!.ns,
        ) ?? null)
      : null,
  );
  readonly installRunning = $derived(
    this.status?.install.kubectl.state === 'running' || this.status?.install.k9s.state === 'running',
  );

  // --- status / install ------------------------------------------------------------

  async loadStatus(): Promise<void> {
    this.statusLoading = true;
    try {
      this.status = await k8sApi.status();
      this.statusError = '';
      this.unavailable = false;
    } catch (e) {
      if (e instanceof ApiError && e.status === 404) this.unavailable = true;
      else this.statusError = e instanceof Error ? e.message : String(e);
    } finally {
      this.statusLoading = false;
    }
  }

  async install(tool: K8sTool): Promise<K8sInstallJob> {
    const job = await k8sApi.install(tool);
    if (this.status) this.status = { ...this.status, install: { ...this.status.install, [tool]: job } };
    return job;
  }

  // --- clusters ---------------------------------------------------------------------

  async loadClusters(): Promise<void> {
    this.clustersLoading = true;
    try {
      const list = await k8sApi.listClusters();
      this.clusters = list;
      this.clustersError = '';
      // Seed capability chips from the cached probe on each row.
      const seeded = { ...this.capabilities };
      for (const c of list) if (c.capabilities && !seeded[c.id]) seeded[c.id] = c.capabilities;
      this.capabilities = seeded;
    } catch (e) {
      this.clustersError = e instanceof Error ? e.message : String(e);
    } finally {
      this.clustersLoading = false;
      this.clustersLoaded = true;
    }
  }

  async loadCapabilities(id: string, refresh = false): Promise<K8sCapabilities | null> {
    try {
      const caps = await k8sApi.capabilities(id, refresh);
      this.capabilities = { ...this.capabilities, [id]: caps };
      return caps;
    } catch {
      return this.capabilities[id] ?? null;
    }
  }

  async createCluster(body: UpsertK8sClusterReq): Promise<K8sCluster> {
    const c = await k8sApi.createCluster(body);
    this.upsertRow(c);
    return c;
  }

  async importCluster(body: ImportK8sClusterReq): Promise<K8sCluster> {
    const c = await k8sApi.importCluster(body);
    this.upsertRow(c);
    return c;
  }

  async updateCluster(id: string, body: Partial<UpsertK8sClusterReq>): Promise<K8sCluster> {
    const c = await k8sApi.updateCluster(id, body);
    this.upsertRow(c);
    return c;
  }

  async deleteCluster(id: string): Promise<void> {
    await k8sApi.deleteCluster(id);
    this.clusters = this.clusters.filter((c) => c.id !== id);
    if (this.clusterId === id) this.clusterId = null;
  }

  private upsertRow(c: K8sCluster): void {
    const i = this.clusters.findIndex((x) => x.id === c.id);
    this.clusters = i < 0 ? [...this.clusters, c] : this.clusters.map((x) => (x.id === c.id ? c : x));
    if (c.capabilities) this.capabilities = { ...this.capabilities, [c.id]: c.capabilities };
  }

  // --- workspace selection ------------------------------------------------------------

  /** Enter a cluster workspace: restore its remembered namespace (falls back to
   *  the row's default namespace, then "all"), drop the previous cluster's rows
   *  and kick off the namespace list + capability probe. */
  selectCluster(id: string | null): void {
    if (id === this.clusterId) return;
    this.clusterId = id;
    this.selected = null;
    this.filter = '';
    this.rows = [];
    this.rowsKey = '';
    this.rowsError = '';
    this.namespaces = [];
    this.namespacesError = '';
    this.k9sSessionId = null;
    if (!id) return;
    const remembered = lsGet(NS_KEY(id));
    const row = this.clusters.find((c) => c.id === id);
    // Namespaces are lowercase DNS labels; normalize whatever was remembered
    // or typed so an auto-capitalized "Mscasino" can't 403 forever.
    this.namespace = (remembered ?? row?.default_namespace ?? '').trim().toLowerCase();
    if (row?.default_namespace) this.rememberKnownNamespace(row.default_namespace.trim().toLowerCase());
    void this.loadNamespaces();
    void this.loadCapabilities(id);
  }

  /** Switching kind also clears the text filter (k9s semantics: a filter
   *  belongs to the view it was typed in — a stale "worker" silently hiding
   *  every Service is worse than retyping). */
  setKind(kind: K8sResourceKind): void {
    if (kind === this.kind) return;
    this.kind = kind;
    this.selected = null;
    this.filter = '';
  }

  setNamespace(ns: string): void {
    ns = ns.trim().toLowerCase();
    this.namespace = ns;
    this.selected = null;
    if (this.clusterId) lsSet(NS_KEY(this.clusterId), ns);
  }

  setAutoRefresh(on: boolean): void {
    this.autoRefresh = on;
    lsSet(AUTO_KEY, on ? '1' : '0');
    if (on) this.startAutoRefresh();
    else this.stopAutoRefresh();
  }

  select(sel: K8sSelection | null, tab?: K8sDrawerTab): void {
    this.selected = sel;
    if (tab) this.drawerTab = tab;
  }

  async loadNamespaces(): Promise<void> {
    const id = this.clusterId;
    if (!id) return;
    try {
      const r = await k8sApi.namespaces(id);
      if (this.clusterId !== id) return;
      this.namespaces = this.mergeKnown(id, r.namespaces);
      this.namespacesError = '';
    } catch (e) {
      if (this.clusterId !== id) return;
      // RBAC-limited user: fall back to the namespaces we know work here so
      // the picker still has real entries to switch between.
      this.namespaces = this.mergeKnown(id, []);
      this.namespacesError = e instanceof Error ? e.message : String(e);
    }
  }

  /** Listed namespaces ∪ known-good ones (known ones the API didn't return
   *  are appended, e.g. when the list is RBAC-partial). */
  private mergeKnown(clusterId: string, listed: K8sNamespace[]): K8sNamespace[] {
    const have = new Set(listed.map((n) => n.name));
    const extra = knownNamespaces(clusterId)
      .filter((n) => !have.has(n))
      .sort()
      .map((name) => ({ name, status: '', age_seconds: 0 }));
    return [...listed, ...extra];
  }

  private rememberKnownNamespace(ns: string): void {
    const id = this.clusterId;
    if (!id || !ns) return;
    const known = knownNamespaces(id);
    if (known.includes(ns)) return;
    known.push(ns);
    lsSet(KNOWN_NS_KEY(id), JSON.stringify(known));
    if (!this.namespaces.some((n) => n.name === ns)) this.namespaces = this.mergeKnown(id, this.namespaces);
  }

  // --- resources ------------------------------------------------------------------------

  /** (Re)load the table for the current cluster/kind/namespace. Cancels an
   *  in-flight load; `quiet` keeps the current rows visible (auto-refresh)
   *  instead of showing the loading state. */
  async loadResources(quiet = false): Promise<void> {
    const id = this.clusterId;
    if (!id) return;
    const key = this.currentKey;
    const kind = this.kind;
    const ns = this.namespace;
    this.rowsAbort?.abort();
    const ac = new AbortController();
    this.rowsAbort = ac;
    if (!quiet || this.rowsKey !== key) this.rowsLoading = true;
    try {
      let items: K8sRow[];
      let hasMetrics = false;
      if (kind === 'nodes') {
        const r = await k8sApi.nodes(id, ac.signal);
        items = r.nodes.map(nodeToRow);
        hasMetrics = r.nodes.some((n) => n.cpu_usage != null);
      } else {
        const r = await k8sApi.resources(id, kind, { ns }, ac.signal);
        items = r.items;
        hasMetrics = r.has_metrics;
      }
      if (ac.signal.aborted || this.currentKey !== key) return;
      this.rows = items;
      this.hasMetrics = hasMetrics;
      this.rowsKey = key;
      this.rowsError = '';
      this.rowsLoadedAt = Date.now();
      if (ns) this.rememberKnownNamespace(ns);
    } catch (e) {
      if (ac.signal.aborted || this.currentKey !== key) return;
      const msg = e instanceof Error ? e.message : String(e);
      this.rowsError = !ns && /at the cluster scope/i.test(msg) ? `${CLUSTER_SCOPE_HINT}\n\n${msg}` : msg;
      if (this.rowsKey !== key) {
        this.rows = [];
        this.rowsKey = key;
      }
    } finally {
      if (this.rowsAbort === ac) {
        this.rowsAbort = null;
        this.rowsLoading = false;
      }
    }
  }

  startAutoRefresh(): void {
    this.stopAutoRefresh();
    if (!this.autoRefresh) return;
    this.refreshTimer = setInterval(() => {
      if (document.hidden || !this.clusterId || this.rowsLoading) return;
      void this.loadResources(true);
    }, AUTO_REFRESH_MS);
  }

  stopAutoRefresh(): void {
    if (this.refreshTimer) clearInterval(this.refreshTimer);
    this.refreshTimer = null;
  }

  /** Leaving the module: stop timers + cancel loads (state is kept so coming
   *  back is instant). */
  suspend(): void {
    this.stopAutoRefresh();
    this.rowsAbort?.abort();
    this.rowsAbort = null;
  }

  // --- live events ----------------------------------------------------------------------

  /** Bumped on every `k8s_monitor_cycle`; the Monitor views `$effect` on it
   *  (plus the cluster id of the cycle) to re-fetch without polling. */
  monitorTick = $state(0);
  monitorTickCluster: string | null = $state(null);

  applyEvent(
    ev: Extract<OttoEvent, { type: 'k8s_cluster_updated' | 'k8s_install_updated' | 'k8s_monitor_cycle' }>,
  ): void {
    if (ev.type === 'k8s_monitor_cycle') {
      this.monitorTickCluster = ev.cluster_id;
      this.monitorTick += 1;
      return;
    }
    if (ev.type === 'k8s_cluster_updated') {
      if (ev.deleted) {
        this.clusters = this.clusters.filter((c) => c.id !== ev.cluster_id);
        if (this.clusterId === ev.cluster_id) this.clusterId = null;
      } else {
        void this.loadClusters();
      }
    } else {
      // Installer state moved — refetch the status (carries the log tail).
      void this.loadStatus();
    }
  }
}

export const k8s = new K8sStore();
