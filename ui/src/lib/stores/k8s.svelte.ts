// Kubernetes console store: tool status + installer polling, the cluster list
// (+ per-cluster capabilities), the workspace selection (cluster / kind /
// namespace / filter / selected row / drawer tab) and the resource-table cache
// with its auto-refresh timer. Like the other event-driven stores it does NOT
// import events.svelte.ts — the dispatcher calls `k8s.applyEvent(...)`.
//
// Gotchas mirrored from sftp.svelte.ts: nothing here mutates `$state` from a
// getter/`$derived` (the table reads `filteredRows`, a pure derived); every
// mutation happens in a method the page calls from an effect or a handler.

import { auth } from './auth.svelte';
import { pollWhileVisible, type Poller } from '../poll';
import { resourceAccess, type ResourceAccessChange } from './resource-access.svelte';
import { ApiError } from '../api/client';
import { formatBytes, formatMillicores } from '../../modules/kubernetes/k8s-util';
import { k8sApi } from '../api/k8s';
import { resourcesPollMs, resourcesValidator } from '../../modules/kubernetes/resourcePoll';
import { TickCoalescer, browserTickEnv } from '../../modules/kubernetes/monitor/tickCoalescer';
import { parseClusterUi, type K8sClusterUi, type K8sDrawerTab, type K8sMonitorUi } from '../../modules/kubernetes/viewState';
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
import { announceModule } from '../lazyModule';

export type { K8sDrawerTab, K8sMonitorUi, K8sClusterUi } from '../../modules/kubernetes/viewState';
/** How many (cluster, kind, namespace) row sets stay cached so switching back
 *  (another kind, the Monitor, another module) paints at once (K-2). */
const ROWS_CACHE_MAX = 4;

/** A row identity inside the current cluster+kind (the route's `<ns>/<name>`). */
export interface K8sSelection {
  ns: string;
  name: string;
}

const NS_KEY = (clusterId: string): string => `otto_k8s_ns:${auth.me?.id ?? 'anonymous'}:${clusterId}`;
/** Namespaces this cluster is KNOWN to have — the default namespace plus every
 *  one the user selected and could read. Rancher project-scoped users can't
 *  `get namespaces` (cluster-scope list is forbidden), so without this the
 *  picker would offer nothing but "All namespaces" — which is forbidden too. */
const KNOWN_NS_KEY = (clusterId: string): string => `otto_k8s_known_ns:${auth.me?.id ?? 'anonymous'}:${clusterId}`;
const CLUSTER_SCOPE_HINT =
  'This kubeconfig user can\'t list across all namespaces (cluster scope). Pick a namespace (press n) — e.g. the cluster\'s default one.';
const UI_KEY = (clusterId: string): string => `otto_k8s_ui:${auth.me?.id ?? 'anonymous'}:${clusterId}`;
const AUTO_KEY = 'otto_k8s_autorefresh';
/** Monitor views re-read at most this often, however many clusters cycle:
 *  every read is a ClickHouse aggregation and the dashboards' windows (≥ 1 h,
 *  read from minute-or-coarser rollups) do not move faster than this. */
export const MONITOR_TICK_MIN_MS = 30_000;

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

/** The lowercased text a filter matches against: the columns a user would
 *  scan (name, namespace, status, node, ip, extra values, images, labels). */
export function rowHaystack(r: K8sRow): string {
  return [
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
}

/** Case-insensitive substring match over [`rowHaystack`]. */
export function rowMatches(r: K8sRow, q: string): boolean {
  if (!q) return true;
  return rowHaystack(r).includes(q);
}

class K8sStore {
  accessRevision = $state(0);
  onAccessChange(change: ResourceAccessChange): void {
    if (
      change.type === 'decision' &&
      (change.kind !== 'k8s_cluster' ||
        !change.before ||
        !Object.keys(change.before.operations).some(
          (op) => change.before?.operations[op]?.allowed && !change.after?.operations[op]?.allowed,
        ))
    )
      return;
    this.accessRevision++;
    this.rowsCache.clear();
    this.uiCache.clear();
    this.rowsAbort?.abort();
    this.rows = [];
    this.rowsKey = '';
    this.rowsError = '';
    this.rowsLoading = false;
    this.rowsLoadedAt = null;
    this.hasMetrics = false;
    this.namespaces = [];
    this.namespacesError = '';
    this.selected = null;
    this.k9sSessionId = null;
    this.namespace = '';
    this.filter = '';
    if (change.type === 'decision') {
      this.clusters = this.clusters.filter((c) => c.id !== change.id);
      const caps = { ...this.capabilities };
      delete caps[change.id];
      this.capabilities = caps;
      try {
        localStorage.removeItem(NS_KEY(change.id));
        localStorage.removeItem(KNOWN_NS_KEY(change.id));
        localStorage.removeItem(UI_KEY(change.id));
      } catch {
        /* optional preference */
      }
    } else {
      this.clusters = [];
      this.capabilities = {};
      if (change.identity) this.clusterId = null;
    }
    this.clusterId = null;
    this.clustersLoaded = false;
    this.clustersLoading = false;
  }

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
  /** Raw (not deep-proxied), replaced wholesale on every load: 5k pods behind
   *  a deep proxy made each filter keystroke ~40 ms. */
  rows: K8sRow[] = $state.raw([]);
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
  private refreshTimer: Poller | null = null;
  /** LRU of recent row sets by `currentKey` (insertion order = recency).
   *  Plain field — the table reads `rows`, never this. */
  private rowsCache = new Map<string, { rows: K8sRow[]; hasMetrics: boolean; loadedAt: number; validator: string | null }>();
  /** perf K8s: how long the last resources load took (ms) — with the row
   *  count it sets the auto-refresh cadence ({@link resourcesPollMs}). */
  private lastLoadMs = 0;
  /** Per-cluster view state (K-1), mirrored to localStorage. Plain field. */
  private uiCache = new Map<string, K8sClusterUi>();
  private uiSaveTimer: ReturnType<typeof setTimeout> | null = null;
  private uiDirty = new Set<string>();

  readonly cluster = $derived(this.clusters.find((c) => c.id === this.clusterId) ?? null);
  readonly caps = $derived(
    (this.clusterId ? this.capabilities[this.clusterId] : undefined) ?? null,
  );
  readonly currentKey = $derived(`${this.clusterId ?? ''}|${this.kind}|${this.namespace}`);
  /** Rows for the CURRENT selection only, narrowed by the free-text filter. */
  /** Lowercased filter haystack per row — built once per load, not per row
   *  per keystroke (and per 10 s refresh while a filter is active). */
  private readonly hay = $derived(this.rows.map(rowHaystack));
  readonly filteredRows = $derived.by(() => {
    if (this.rowsKey !== this.currentKey) return [];
    const q = this.filter.trim().toLowerCase();
    if (!q) return this.rows;
    const hay = this.hay;
    return this.rows.filter((_, i) => hay[i].includes(q));
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
    const revision=this.accessRevision;
    this.clustersLoading = true;
    try {
      const list = await k8sApi.listClusters();
      if(revision!==this.accessRevision)return;
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
    const revision=this.accessRevision;
    try {
      const caps = await k8sApi.capabilities(id, refresh);
      if(revision!==this.accessRevision)return null;
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
    this.rememberConsoleUi();
    this.clusterId = id;
    this.selected = null;
    this.rows = [];
    this.rowsKey = '';
    this.rowsError = '';
    this.namespaces = [];
    this.namespacesError = '';
    this.k9sSessionId = null;
    if (!id) {
      this.filter = '';
      return;
    }
    // The cluster's own remembered view: per-kind filter + drawer tab.
    const ui = this.clusterUi(id);
    this.filter = ui.filters[this.kind] ?? '';
    this.drawerTab = ui.drawerTab;
    const remembered = lsGet(NS_KEY(id));
    const row = this.clusters.find((c) => c.id === id);
    // Namespaces are lowercase DNS labels; normalize whatever was remembered
    // or typed so an auto-capitalized "Mscasino" can't 403 forever.
    this.namespace = (remembered ?? row?.default_namespace ?? '').trim().toLowerCase();
    if (row?.default_namespace) this.rememberKnownNamespace(row.default_namespace.trim().toLowerCase());
    void this.loadNamespaces();
    void this.loadCapabilities(id);
    this.paintCached();
  }

  /** Switching kind also clears the text filter (k9s semantics: a filter
   *  belongs to the view it was typed in — a stale "worker" silently hiding
   *  every Service is worse than retyping). */
  setKind(kind: K8sResourceKind): void {
    if (kind === this.kind) return;
    this.rememberConsoleUi();
    this.kind = kind;
    this.selected = null;
    // Per-view memory: each kind gets back the filter last typed in it.
    this.filter = this.clusterId ? (this.clusterUi(this.clusterId).filters[kind] ?? '') : '';
    this.paintCached();
  }

  setNamespace(ns: string): void {
    ns = ns.trim().toLowerCase();
    if (ns === this.namespace) return;
    this.namespace = ns;
    this.selected = null;
    if (this.clusterId) lsSet(NS_KEY(this.clusterId), ns);
    this.paintCached();
  }

  /** Pre-set the filter a kind opens with (a cross-link such as Monitor
   *  workload → its pods), before routing to it. */
  presetFilter(clusterId: string, kind: K8sResourceKind, filter: string): void {
    const ui = this.clusterUi(clusterId);
    ui.filters[kind] = filter;
    this.markUi(clusterId);
    if (this.clusterId === clusterId && this.kind === kind) this.filter = filter;
  }

  // --- per-cluster view state (K-1) -----------------------------------------------------

  private clusterUi(id: string): K8sClusterUi {
    let ui = this.uiCache.get(id);
    if (!ui) {
      ui = parseClusterUi(lsGet(UI_KEY(id)));
      this.uiCache.set(id, ui);
    }
    return ui;
  }

  /** Debounced localStorage write of the clusters whose view state moved. */
  private markUi(id: string): void {
    this.uiDirty.add(id);
    if (this.uiSaveTimer) return;
    this.uiSaveTimer = setTimeout(() => this.flushUi(), 300);
  }

  /** Write every pending per-cluster view state now. */
  flushUi(): void {
    if (this.uiSaveTimer) clearTimeout(this.uiSaveTimer);
    this.uiSaveTimer = null;
    for (const id of this.uiDirty) {
      const ui = this.uiCache.get(id);
      if (ui) lsSet(UI_KEY(id), JSON.stringify(ui));
    }
    this.uiDirty.clear();
  }

  /** Fold the live console filter + drawer tab into the current cluster's
   *  remembered view (the workspace calls this as they change). */
  rememberConsoleUi(): void {
    const id = this.clusterId;
    if (!id) return;
    const ui = this.clusterUi(id);
    const f = this.filter;
    if ((ui.filters[this.kind] ?? '') === f && ui.drawerTab === this.drawerTab) return;
    if (f) ui.filters[this.kind] = f;
    else delete ui.filters[this.kind];
    ui.drawerTab = this.drawerTab;
    this.markUi(id);
  }

  /** The table's remembered scroll offset for a `currentKey`. */
  scrollFor(key: string): number {
    const id = key.split('|')[0];
    return id ? (this.clusterUi(id).scroll[key] ?? 0) : 0;
  }

  saveScroll(key: string, top: number): void {
    const id = key.split('|')[0];
    if (!id) return;
    const ui = this.clusterUi(id);
    const t = Math.max(0, Math.round(top));
    if ((ui.scroll[key] ?? 0) === t) return;
    if (t) ui.scroll[key] = t;
    else delete ui.scroll[key];
    // Bounded: only the most recent dozen views keep an offset.
    const keys = Object.keys(ui.scroll);
    if (keys.length > 12) delete ui.scroll[keys[0]];
    this.markUi(id);
  }

  /** A copy of a cluster's remembered Monitor view. */
  monitorUi(clusterId: string): K8sMonitorUi {
    return { ...this.clusterUi(clusterId).monitor };
  }

  saveMonitorUi(clusterId: string, patch: Partial<K8sMonitorUi>): void {
    const ui = this.clusterUi(clusterId);
    const next = { ...ui.monitor, ...patch };
    if (JSON.stringify(next) === JSON.stringify(ui.monitor)) return;
    ui.monitor = next;
    this.markUi(clusterId);
  }

  /** Show the cached rows for the current selection at once (no skeleton);
   *  the workspace's load effect then refreshes them quietly. */
  private paintCached(): void {
    const key = this.currentKey;
    if (this.rowsKey === key) return;
    const hit = this.rowsCache.get(key);
    if (!hit) return;
    this.rows = hit.rows;
    this.hasMetrics = hit.hasMetrics;
    this.rowsKey = key;
    this.rowsError = '';
    this.rowsLoadedAt = hit.loadedAt;
  }

  private cacheRows(key: string, rows: K8sRow[], hasMetrics: boolean, loadedAt: number, validator: string | null = null): void {
    this.rowsCache.delete(key);
    this.rowsCache.set(key, { rows, hasMetrics, loadedAt, validator });
    while (this.rowsCache.size > ROWS_CACHE_MAX) {
      const oldest = this.rowsCache.keys().next().value;
      if (oldest === undefined) break;
      this.rowsCache.delete(oldest);
    }
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
    const revision=this.accessRevision;
    const id = this.clusterId;
    if (!id) return;
    try {
      const r = await k8sApi.namespaces(id);
      if (this.clusterId !== id || revision!==this.accessRevision) return;
      this.namespaces = this.mergeKnown(id, r.namespaces);
      this.namespacesError = '';
    } catch (e) {
      if (this.clusterId !== id || revision!==this.accessRevision) return;
      // RBAC-limited user: fall back to the namespaces we know work here so
      // the picker still has real entries to switch between.
      this.namespaces = this.mergeKnown(id, []);
      this.namespacesError = e instanceof Error ? e.message : String(e);
    }
  }

  /** Listed namespaces ∪ known-good ones (known ones the API didn't return
   *  are appended, e.g. when the list is RBAC-partial). */
  private mergeKnown(clusterId: string, listed: K8sNamespace[]): K8sNamespace[] {
    if(!auth.isRoot && resourceAccess.get('k8s_cluster',clusterId)?.mode!=='legacy')return listed;
    const have = new Set(listed.map((n) => n.name));
    const persisted = this.clusters.find((c) => c.id === clusterId)?.known_namespaces ?? [];
    const extra = [...new Set([...persisted, ...knownNamespaces(clusterId)])]
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
    // Persist on the cluster row too (the registry is Admin-only; everyone
    // else keeps the localStorage copy).
    if (auth.isRoot || auth.can('kubernetes', 'admin')) {
      const c = this.clusters.find((x) => x.id === id);
      const persisted = c?.known_namespaces ?? [];
      if (!persisted.includes(ns)) {
        void this.updateCluster(id, { known_namespaces: [...persisted, ns] }).catch(() => {});
      }
    }
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
    const had = this.rowsKey === key;
    this.paintCached();
    // Just painted from the cache: refresh behind those rows, no skeleton.
    if (!had && this.rowsKey === key) quiet = true;
    if (!quiet || this.rowsKey !== key) this.rowsLoading = true;
    const started = Date.now();
    try {
      let items: K8sRow[];
      let hasMetrics = false;
      let validator: string | null = null;
      if (kind === 'nodes') {
        const r = await k8sApi.nodes(id, ac.signal);
        items = r.nodes.map(nodeToRow);
        hasMetrics = r.nodes.some((n) => n.cpu_usage != null);
      } else {
        // perf K8s: conditional read — only when the rows on screen ARE this
        // key's cached set, so a 304 can never leave another view's rows up.
        const held = this.rowsKey === key ? this.rowsCache.get(key) : undefined;
        const r = await k8sApi.resourcesIfChanged(id, kind, { ns }, held?.validator ?? null, ac.signal);
        if (ac.signal.aborted || this.currentKey !== key) return;
        this.lastLoadMs = Date.now() - started;
        if (r.notModified) {
          // Unchanged list: the rows stay the very same array (no reassign,
          // no parse, no table re-render) — only the freshness moves.
          this.rowsError = '';
          this.rowsLoadedAt = Date.now();
          if (held) this.cacheRows(key, held.rows, held.hasMetrics, this.rowsLoadedAt, held.validator);
          return;
        }
        items = r.data.items;
        hasMetrics = r.data.has_metrics;
        validator = resourcesValidator(r.etag, r.data.version);
      }
      if (ac.signal.aborted || this.currentKey !== key) return;
      this.lastLoadMs = Date.now() - started;
      this.rows = items;
      this.hasMetrics = hasMetrics;
      this.rowsKey = key;
      this.rowsError = '';
      this.rowsLoadedAt = Date.now();
      this.cacheRows(key, items, hasMetrics, this.rowsLoadedAt, validator);
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
    // Shared chain (lib/poll): never overlaps a slow kubectl list, paused
    // while hidden, backs off while the cluster fails; `/k8s/*` rides the
    // long lane (api/client.ts), off the interactive socket pool.
    // perf K8s: the cadence adapts to the list (`ms` is re-read per schedule):
    // 10 s under 1000 rows, else max(30 s, 3 × the last load). No poll while
    // the k9s terminal covers the table — nobody can see the rows.
    const rowsNow = (): number => this.rows.length;
    const loadMs = (): number => this.lastLoadMs;
    this.refreshTimer = pollWhileVisible(
      async () => {
        if (!this.clusterId || this.rowsLoading || this.k9sSessionId) return;
        await this.loadResources(true);
      },
      {
        get ms() {
          return resourcesPollMs(rowsNow(), loadMs());
        },
        immediate: false,
      },
    );
  }

  stopAutoRefresh(): void {
    this.refreshTimer?.stop();
    this.refreshTimer = null;
  }

  /** Leaving the module: stop timers + cancel loads (state is kept so coming
   *  back is instant). */
  suspend(): void {
    this.stopAutoRefresh();
    this.rowsAbort?.abort();
    this.rowsAbort = null;
    this.rememberConsoleUi();
    this.flushUi();
  }

  // --- live events ----------------------------------------------------------------------

  /** Bumped after `k8s_monitor_cycle` events; the Monitor views `$effect` on
   *  it (plus {@link monitorTicked} for their cluster) to re-fetch without
   *  polling. Coalesced: at most one bump per {@link MONITOR_TICK_MIN_MS}
   *  however many clusters cycle, and none while the document is hidden —
   *  one fires when it is visible again. */
  monitorTick = $state(0);
  /** The last cluster that cycled before the latest tick. */
  monitorTickCluster: string | null = $state(null);
  /** Every cluster that cycled since the previous tick. */
  monitorTickClusters = $state.raw<ReadonlySet<string>>(new Set());
  private readonly cycles = new TickCoalescer(
    MONITOR_TICK_MIN_MS,
    (clusters) => {
      this.monitorTickClusters = new Set(clusters);
      this.monitorTickCluster = clusters[clusters.length - 1] ?? null;
      this.monitorTick += 1;
    },
    browserTickEnv,
  );

  /** Did `clusterId` cycle since the previous tick? */
  monitorTicked(clusterId: string): boolean {
    return this.monitorTickClusters.has(clusterId);
  }

  applyEvent(
    ev: Extract<OttoEvent, { type: 'k8s_cluster_updated' | 'k8s_install_updated' | 'k8s_monitor_cycle' }>,
  ): void {
    if (ev.type === 'k8s_monitor_cycle') {
      this.cycles.cycle(ev.cluster_id);
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
// Routed by `peek()` in lib/events.svelte.ts (perf G2): let it see this store
// however it was first imported.
announceModule('k8s', k8s);
resourceAccess.subscribe(change=>k8s.onAccessChange(change));
