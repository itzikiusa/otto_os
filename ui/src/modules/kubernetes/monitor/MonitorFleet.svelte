<script lang="ts">
  // Fleet dashboard (`#/kubernetes/monitor/fleet[/<overview|table|events|requests>]`):
  // ONE view over every monitored cluster, read from ClickHouse only — the
  // general picture over a window (restarts / OOMs, memory, req/s, 5xx,
  // latency), not the live pod status. Filters (window, clusters, namespace,
  // workload, pod), the table grouping / sort and the events sort are all
  // persisted per device, and every row is a drill-down (cluster → namespace
  // → workload → pod → events).
  import { untrack } from 'svelte';
  import { router } from '../../../lib/router.svelte';
  import { k8s } from '../../../lib/stores/k8s.svelte';
  import { k8sApi } from '../../../lib/api/k8s';
  import type {
    K8sFleetEvent,
    K8sFleetEventSort,
    K8sFleetFilters,
    K8sFleetGroup,
    K8sFleetMetric,
    K8sFleetRequests,
    K8sFleetRow,
    K8sFleetSeries,
    K8sFleetSeriesBy,
    K8sFleetSortKey,
  } from '../../../lib/api/types';
  import type { MetricChartSeries, MetricChartUnit } from '../../../lib/metric-format';
  import Icon from '../../../lib/components/Icon.svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Skeleton from '../../../lib/components/Skeleton.svelte';
  import MetricChart from '../../../lib/components/MetricChart.svelte';
  import { envBadge, formatBytes } from '../k8s-util';
  import { WINDOWS, classColor, classLabel, fmtMs, fmtPct, fmtRate, isWindow } from './monitor-util';

  interface Props {
    tab: string;
  }
  let { tab }: Props = $props();

  const TABS = [
    { id: 'overview', label: 'Overview' },
    { id: 'table', label: 'Table' },
    { id: 'events', label: 'Events' },
    { id: 'requests', label: 'Requests' },
  ] as const;
  const activeTab = $derived(TABS.some((t) => t.id === tab) ? tab : 'overview');
  function goTab(id: string): void {
    router.go(`kubernetes/monitor/fleet/${id}`);
  }

  // ── Persisted selection ───────────────────────────────────────────────────
  // One JSON blob: filters + table/event ordering. Anything malformed falls
  // back field-by-field so an older blob never breaks the page.
  const LS = 'otto_k8s_fleet';
  interface Saved {
    window: (typeof WINDOWS)[number];
    clusters: string[];
    ns: string;
    workload: string;
    pod: string;
    group: K8sFleetGroup;
    sort: K8sFleetSortKey;
    dir: 'asc' | 'desc';
    by: K8sFleetSeriesBy;
    evSort: K8sFleetEventSort;
    evDir: 'asc' | 'desc';
    evClass: string;
  }
  const SORT_KEYS: K8sFleetSortKey[] = ['cluster', 'namespace', 'workload', 'pod', 'pods', 'restarts', 'oom', 'crash', 'probe', 'churn', 'mem_last', 'mem_avg', 'mem_max', 'rps', 'err_pct', 'latency_ms'];
  const EV_SORT_KEYS: K8sFleetEventSort[] = ['ts', 'cluster', 'namespace', 'workload', 'pod', 'kind', 'class', 'reason'];
  const BYS: K8sFleetSeriesBy[] = ['cluster', 'namespace', 'workload', 'pod'];
  function load(): Saved {
    const d: Saved = { window: '24h', clusters: [], ns: '', workload: '', pod: '', group: 'workload', sort: 'restarts', dir: 'desc', by: 'cluster', evSort: 'ts', evDir: 'desc', evClass: '' };
    try {
      const raw = localStorage.getItem(LS);
      if (!raw) return d;
      const o = JSON.parse(raw) as Partial<Saved>;
      const str = (v: unknown): string => (typeof v === 'string' ? v : '');
      return {
        window: isWindow(str(o.window)) ? (o.window as Saved['window']) : d.window,
        clusters: Array.isArray(o.clusters) ? o.clusters.filter((c): c is string => typeof c === 'string') : [],
        ns: str(o.ns),
        workload: str(o.workload),
        pod: str(o.pod),
        group: o.group === 'pod' ? 'pod' : 'workload',
        sort: SORT_KEYS.includes(o.sort as K8sFleetSortKey) ? (o.sort as K8sFleetSortKey) : d.sort,
        dir: o.dir === 'asc' ? 'asc' : 'desc',
        by: BYS.includes(o.by as K8sFleetSeriesBy) ? (o.by as K8sFleetSeriesBy) : d.by,
        evSort: EV_SORT_KEYS.includes(o.evSort as K8sFleetEventSort) ? (o.evSort as K8sFleetEventSort) : d.evSort,
        evDir: o.evDir === 'asc' ? 'asc' : 'desc',
        evClass: str(o.evClass),
      };
    } catch {
      return d;
    }
  }
  const saved = load();
  let window = $state<Saved['window']>(saved.window);
  let clusters = $state<string[]>(saved.clusters);
  let ns = $state(saved.ns);
  let workload = $state(saved.workload);
  let pod = $state(saved.pod);
  let group = $state<K8sFleetGroup>(saved.group);
  let sort = $state<K8sFleetSortKey>(saved.sort);
  let dir = $state<'asc' | 'desc'>(saved.dir);
  let by = $state<K8sFleetSeriesBy>(saved.by);
  let evSort = $state<K8sFleetEventSort>(saved.evSort);
  let evDir = $state<'asc' | 'desc'>(saved.evDir);
  let evClass = $state(saved.evClass);
  $effect(() => {
    const s: Saved = { window, clusters, ns, workload, pod, group, sort, dir, by, evSort, evDir, evClass };
    try {
      localStorage.setItem(LS, JSON.stringify(s));
    } catch {
      /* ignore */
    }
  });

  /** The query fragment every fleet call shares. */
  const sel = $derived({ window, cluster: clusters.join(',') || undefined, ns: ns || undefined, workload: workload || undefined, pod: pod || undefined });
  const hasFilter = $derived(clusters.length > 0 || !!ns || !!workload || !!pod);
  function clearFilters(): void {
    clusters = [];
    ns = '';
    workload = '';
    pod = '';
  }
  function toggleCluster(id: string): void {
    clusters = clusters.includes(id) ? clusters.filter((c) => c !== id) : [...clusters, id];
    // A narrower cluster set can orphan the namespace / workload pick.
    pod = '';
  }

  // ── Filters (what has data) ──────────────────────────────────────────────
  let filters = $state<K8sFleetFilters | null>(null);
  let filtersError = $state('');
  let fAbort: AbortController | null = null;
  async function loadFilters(): Promise<void> {
    fAbort?.abort();
    fAbort = new AbortController();
    try {
      filters = await k8sApi.fleetFilters(sel, fAbort.signal);
      filtersError = '';
    } catch (e) {
      if (e instanceof DOMException && e.name === 'AbortError') return;
      filtersError = e instanceof Error ? e.message : String(e);
    }
  }
  $effect(() => {
    void sel;
    untrack(() => void loadFilters());
  });
  const clusterOpts = $derived(filters?.clusters ?? []);
  const nsOpts = $derived.by(() => {
    const set = new Set<string>();
    for (const n of filters?.namespaces ?? []) if (clusters.length === 0 || clusters.includes(n.cluster_id)) set.add(n.namespace);
    if (ns) set.add(ns);
    return [...set].sort();
  });
  const wlOpts = $derived.by(() => {
    const set = new Set<string>();
    for (const w of filters?.workloads ?? []) {
      if (clusters.length && !clusters.includes(w.cluster_id)) continue;
      if (ns && w.namespace !== ns) continue;
      set.add(w.workload);
    }
    if (workload) set.add(workload);
    return [...set].sort();
  });
  const podOpts = $derived.by(() => {
    const set = new Set<string>((filters?.pods ?? []).map((p) => p.pod));
    if (pod) set.add(pod);
    return [...set].sort();
  });

  // ── Table (also feeds the overview KPIs) ─────────────────────────────────
  let rows = $state<K8sFleetRow[]>([]);
  let total = $state(0);
  let tableLoading = $state(true);
  let tableError = $state('');
  let quick = $state('');
  const PAGE = 200;
  let tAbort: AbortController | null = null;
  async function loadTable(quiet = false, append = false): Promise<void> {
    tAbort?.abort();
    tAbort = new AbortController();
    if (!quiet && !append) tableLoading = true;
    try {
      const r = await k8sApi.fleetTable({ ...sel, group, sort, dir, limit: PAGE, offset: append ? rows.length : 0 }, tAbort.signal);
      rows = append ? [...rows, ...r.rows] : r.rows;
      total = r.total;
      tableError = '';
    } catch (e) {
      if (e instanceof DOMException && e.name === 'AbortError') return;
      tableError = e instanceof Error ? e.message : String(e);
    } finally {
      tableLoading = false;
    }
  }
  function sortBy(k: K8sFleetSortKey): void {
    if (sort === k) dir = dir === 'asc' ? 'desc' : 'asc';
    else {
      sort = k;
      dir = k === 'cluster' || k === 'namespace' || k === 'workload' || k === 'pod' ? 'asc' : 'desc';
    }
  }
  const visibleRows = $derived.by(() => {
    const q = quick.trim().toLowerCase();
    if (!q) return rows;
    return rows.filter((r) => `${r.cluster.name} ${r.namespace} ${r.workload} ${r.pod}`.toLowerCase().includes(q));
  });
  const kpi = $derived.by(() => {
    let restarts = 0;
    let oom = 0;
    let crash = 0;
    let churn = 0;
    let mem = 0;
    let rps = 0;
    let err = 0;
    let pods = 0;
    for (const r of rows) {
      restarts += r.restarts.oom + r.restarts.crash + r.restarts.probe + r.restarts.unknown;
      oom += r.restarts.oom;
      crash += r.restarts.crash;
      churn += r.churn;
      mem += r.mem_last;
      rps += r.rps;
      err += (r.rps * r.err_pct) / 100;
      pods += r.pods;
    }
    return { restarts, oom, crash, churn, mem, rps, errPct: rps > 0 ? (100 * err) / rps : 0, pods, workloads: rows.length };
  });

  // ── Series (overview charts) ─────────────────────────────────────────────
  const METRICS: { id: K8sFleetMetric; label: string; unit: MetricChartUnit }[] = [
    { id: 'restarts', label: 'Restarts by class', unit: 'count' },
    { id: 'mem', label: 'Memory', unit: 'bytes' },
    { id: 'rps', label: 'Requests / s', unit: 'count_per_sec' },
    { id: 'err', label: '5xx %', unit: 'percent' },
    { id: 'latency', label: 'Latency (avg)', unit: 'ms' },
  ];
  let charts = $state<Partial<Record<K8sFleetMetric, K8sFleetSeries>>>({});
  let chartsLoading = $state(true);
  let chartsError = $state('');
  let sAbort: AbortController | null = null;
  async function loadSeries(quiet = false): Promise<void> {
    sAbort?.abort();
    sAbort = new AbortController();
    if (!quiet) chartsLoading = true;
    try {
      const all = await Promise.all(METRICS.map((m) => k8sApi.fleetSeries({ ...sel, metric: m.id, by }, sAbort!.signal)));
      const next: Partial<Record<K8sFleetMetric, K8sFleetSeries>> = {};
      METRICS.forEach((m, i) => (next[m.id] = all[i]));
      charts = next;
      chartsError = '';
    } catch (e) {
      if (e instanceof DOMException && e.name === 'AbortError') return;
      chartsError = e instanceof Error ? e.message : String(e);
    } finally {
      chartsLoading = false;
    }
  }
  function toChart(s: K8sFleetSeries | undefined): MetricChartSeries[] {
    if (!s) return [];
    // Restart classes keep their semantic colours; everything else cycles the palette.
    return s.series.slice(0, 12).map((x) => ({
      label: s.metric === 'restarts' ? classLabel(x.key) : x.label,
      color: s.metric === 'restarts' ? classColor(x.key) : undefined,
      points: x.points.map((p) => ({ t: Date.parse(p.t), v: p.v })),
    }));
  }

  // ── Events ───────────────────────────────────────────────────────────────
  const CLASS_OPTIONS = ['', 'oom', 'crash', 'probe', 'unknown', 'churn', 'version', 'k8s_event'];
  let events = $state<K8sFleetEvent[]>([]);
  let evTotal = $state(0);
  let evLoading = $state(true);
  let evError = $state('');
  let eAbort: AbortController | null = null;
  async function loadEvents(quiet = false, append = false): Promise<void> {
    eAbort?.abort();
    eAbort = new AbortController();
    if (!quiet && !append) evLoading = true;
    try {
      const r = await k8sApi.fleetEvents({ ...sel, class: evClass || undefined, sort: evSort, dir: evDir, limit: PAGE, offset: append ? events.length : 0 }, eAbort.signal);
      events = append ? [...events, ...r.rows] : r.rows;
      evTotal = r.total;
      evError = '';
    } catch (e) {
      if (e instanceof DOMException && e.name === 'AbortError') return;
      evError = e instanceof Error ? e.message : String(e);
    } finally {
      evLoading = false;
    }
  }
  function evSortBy(k: K8sFleetEventSort): void {
    if (evSort === k) evDir = evDir === 'asc' ? 'desc' : 'asc';
    else {
      evSort = k;
      evDir = k === 'ts' ? 'desc' : 'asc';
    }
  }
  function fmtTs(iso: string): string {
    const d = new Date(iso);
    return Number.isNaN(d.getTime()) ? iso : d.toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' });
  }
  function eventMsg(e: K8sFleetEvent): string {
    const d = (e.detail ?? {}) as Record<string, unknown>;
    if (e.kind === 'version') return e.reason;
    if (e.kind === 'churn') return `${e.reason}${d.planned_by ? ` · by ${String(d.planned_by)}` : ''}`;
    const parts = [e.reason];
    if (e.exit_code) parts.push(`exit ${e.exit_code}`);
    if (e.container) parts.push(e.container);
    return parts.filter(Boolean).join(' · ');
  }

  // ── Requests (per route; needs request_labels) ───────────────────────────
  let reqs = $state<K8sFleetRequests | null>(null);
  let reqLoading = $state(true);
  let reqError = $state('');
  let reqSort = $state<'rps' | 'err_pct' | 'avg_ms' | 'path'>('rps');
  let reqDir = $state<'asc' | 'desc'>('desc');
  let rAbort: AbortController | null = null;
  async function loadRequests(quiet = false): Promise<void> {
    rAbort?.abort();
    rAbort = new AbortController();
    if (!quiet) reqLoading = true;
    try {
      reqs = await k8sApi.fleetRequests(sel, rAbort.signal);
      reqError = '';
    } catch (e) {
      if (e instanceof DOMException && e.name === 'AbortError') return;
      reqError = e instanceof Error ? e.message : String(e);
    } finally {
      reqLoading = false;
    }
  }
  const reqRows = $derived.by(() => {
    const list = [...(reqs?.rows ?? [])];
    const k = reqSort;
    const d = reqDir === 'asc' ? 1 : -1;
    list.sort((a, b) => (k === 'path' ? d * a.path.localeCompare(b.path) : d * (a[k] - b[k])));
    return list;
  });
  function reqSortBy(k: typeof reqSort): void {
    if (reqSort === k) reqDir = reqDir === 'asc' ? 'desc' : 'asc';
    else {
      reqSort = k;
      reqDir = k === 'path' ? 'asc' : 'desc';
    }
  }

  // ── Loading orchestration ────────────────────────────────────────────────
  // Each tab loads what it shows; the overview needs the table (KPIs) and the
  // charts. Every dependency is read here so a filter / sort change re-runs
  // exactly the affected fetches.
  $effect(() => {
    void sel;
    void group;
    void sort;
    void dir;
    const t = activeTab;
    if (t === 'overview' || t === 'table') untrack(() => void loadTable());
  });
  $effect(() => {
    void sel;
    void by;
    if (activeTab === 'overview') untrack(() => void loadSeries());
  });
  $effect(() => {
    void sel;
    void evSort;
    void evDir;
    void evClass;
    if (activeTab === 'events') untrack(() => void loadEvents());
  });
  $effect(() => {
    void sel;
    if (activeTab === 'requests') untrack(() => void loadRequests());
  });
  // Live refresh after any cluster's collection cycle.
  $effect(() => {
    const tick = k8s.monitorTick;
    if (tick > 0) {
      untrack(() => {
        if (activeTab === 'overview') {
          void loadTable(true);
          void loadSeries(true);
        }
        if (activeTab === 'table') void loadTable(true);
        if (activeTab === 'events') void loadEvents(true);
        if (activeTab === 'requests') void loadRequests(true);
        void loadFilters();
      });
    }
  });
  function refresh(): void {
    void loadFilters();
    if (activeTab === 'overview') {
      void loadTable();
      void loadSeries();
    }
    if (activeTab === 'table') void loadTable();
    if (activeTab === 'events') void loadEvents();
    if (activeTab === 'requests') void loadRequests();
  }

  // ── Drill-down ───────────────────────────────────────────────────────────
  function drillRow(r: K8sFleetRow): void {
    if (group === 'workload') {
      clusters = [r.cluster_id];
      ns = r.namespace;
      workload = r.workload;
      pod = '';
      group = 'pod';
    } else {
      clusters = [r.cluster_id];
      ns = r.namespace;
      workload = r.workload;
      pod = r.pod;
      goTab('events');
    }
  }
  function restartsTotal(r: K8sFleetRow): number {
    return r.restarts.oom + r.restarts.crash + r.restarts.probe + r.restarts.unknown;
  }
  const arrow = (on: boolean, d: 'asc' | 'desc'): string => (on ? (d === 'asc' ? ' ↑' : ' ↓') : '');
</script>

<div class="page" data-testid="k8s-fleet">
  <div class="page-header">
    <div>
      <h1>
        <button class="crumb" onclick={() => router.go('kubernetes')}>Kubernetes</button>
        <span class="sep">/</span>
        <button class="crumb" onclick={() => router.go('kubernetes/monitor')}>Monitor</button>
        <span class="sep">/</span> Fleet
      </h1>
      <div class="sub">Every monitored cluster in one dashboard — restarts &amp; OOMs, memory, requests, latency — straight from ClickHouse. Filters, grouping and ordering stick.</div>
    </div>
    <div class="actions">
      <div class="seg" role="radiogroup" aria-label="Window">
        {#each WINDOWS as w (w)}
          <button class="seg-btn" class:on={window === w} role="radio" aria-checked={window === w} onclick={() => (window = w)}>{w}</button>
        {/each}
      </div>
      <button class="btn ghost" onclick={refresh} title="Refresh" aria-label="Refresh fleet"><Icon name="refresh" size={14} /></button>
    </div>
  </div>

  <!-- Filters: cluster pills (none = all) + namespace / workload / pod selects. -->
  <div class="filters card" data-testid="k8s-fleet-filters">
    <div class="pills" role="group" aria-label="Clusters">
      {#if clusterOpts.length === 0}
        <span class="dim small">{filtersError ? filtersError : 'No clusters have monitoring data in this window.'}</span>
      {/if}
      {#each clusterOpts as c (c.id)}
        <button
          class="pill"
          class:on={clusters.includes(c.id)}
          class:empty={c.rows === 0}
          onclick={() => toggleCluster(c.id)}
          title={c.rows === 0 ? `${c.name}: nothing collected in this window` : `${c.name}: ${c.rows.toLocaleString()} rows in ${window}`}
          aria-pressed={clusters.includes(c.id)}
          data-testid="k8s-fleet-cluster"
        >
          <span class="dot" style="background: {c.color ?? 'var(--accent)'}"></span>
          {c.name}
          <span class="env-badge" class:prod={c.environment === 'prod'}>{envBadge(c.environment)}</span>
        </button>
      {/each}
    </div>
    <div class="selects">
      <select class="input" bind:value={ns} aria-label="Namespace" onchange={() => { workload = ''; pod = ''; }}>
        <option value="">All namespaces</option>
        {#each nsOpts as n (n)}<option value={n}>{n}</option>{/each}
      </select>
      <select class="input" bind:value={workload} aria-label="Workload" onchange={() => (pod = '')}>
        <option value="">All workloads</option>
        {#each wlOpts as w (w)}<option value={w}>{w}</option>{/each}
      </select>
      <select class="input" bind:value={pod} aria-label="Pod" disabled={podOpts.length === 0 && !pod} title={podOpts.length === 0 ? 'Pick a workload (or one cluster + namespace) to list pods' : ''}>
        <option value="">All pods</option>
        {#each podOpts as p (p)}<option value={p}>{p}</option>{/each}
      </select>
      {#if hasFilter}
        <button class="btn small ghost" onclick={clearFilters} data-testid="k8s-fleet-clear"><Icon name="x" size={11} /> Clear</button>
      {/if}
    </div>
  </div>

  <nav class="tabs" aria-label="Fleet sections">
    {#each TABS as t (t.id)}
      <button class="tab" class:active={activeTab === t.id} onclick={() => goTab(t.id)} aria-current={activeTab === t.id ? 'page' : undefined} data-testid="k8s-fleet-tab-{t.id}">{t.label}</button>
    {/each}
  </nav>

  {#if activeTab === 'overview'}
    {#if tableLoading && !rows.length}
      <Skeleton rows={2} height={60} />
    {:else if tableError}
      <EmptyState icon="helm" title="Couldn't load the fleet" body={tableError} actionLabel="Retry" onaction={refresh} />
    {:else}
      <div class="kpis" data-testid="k8s-fleet-kpis">
        <div class="kpi"><span class="k">Unplanned restarts</span><span class="v mono" class:bad={kpi.restarts > 0}>{kpi.restarts}</span><span class="d">OOM {kpi.oom} · crash {kpi.crash}</span></div>
        <div class="kpi"><span class="k">Planned churn</span><span class="v mono">{kpi.churn}</span><span class="d">rollouts, scales, drains</span></div>
        <div class="kpi"><span class="k">Memory (latest)</span><span class="v mono">{formatBytes(kpi.mem)}</span><span class="d">{kpi.pods} pods · {kpi.workloads} workloads</span></div>
        <div class="kpi"><span class="k">Requests</span><span class="v mono">{fmtRate(kpi.rps)}</span><span class="d" class:bad={kpi.errPct >= 1}>5xx {fmtPct(kpi.errPct)}</span></div>
      </div>
    {/if}
    <div class="toolbar">
      <span class="dim small">Series per</span>
      <div class="seg" role="radiogroup" aria-label="Series by">
        {#each BYS as b (b)}
          <button class="seg-btn" class:on={by === b} role="radio" aria-checked={by === b} onclick={() => (by = b)}>{b}</button>
        {/each}
      </div>
      <span class="dim small">(restarts are always per class)</span>
    </div>
    {#if chartsLoading && !Object.keys(charts).length}
      <Skeleton rows={3} height={160} />
    {:else if chartsError}
      <EmptyState icon="helm" title="Couldn't load the charts" body={chartsError} actionLabel="Retry" onaction={() => void loadSeries()} />
    {:else}
      <div class="charts" data-testid="k8s-fleet-charts">
        {#each METRICS as m (m.id)}
          <div class="card chart">
            <div class="chart-head"><b>{m.label}</b><span class="dim small">{charts[m.id]?.step_secs ? `${charts[m.id]?.step_secs}s buckets` : ''}</span></div>
            <MetricChart series={toChart(charts[m.id])} unit={m.unit} height={150} area={m.id !== 'restarts'} emptyText="No samples in this window" />
          </div>
        {/each}
      </div>
    {/if}
  {:else if activeTab === 'table'}
    <div class="toolbar">
      <input class="input" placeholder="Quick filter…" bind:value={quick} aria-label="Quick filter" />
      <div class="seg" role="radiogroup" aria-label="Group by">
        <button class="seg-btn" class:on={group === 'workload'} role="radio" aria-checked={group === 'workload'} onclick={() => (group = 'workload')}>Workloads</button>
        <button class="seg-btn" class:on={group === 'pod'} role="radio" aria-checked={group === 'pod'} onclick={() => (group = 'pod')}>Pods</button>
      </div>
      <span class="spacer"></span>
      <span class="dim small" data-testid="k8s-fleet-table-count">{total.toLocaleString()} {group === 'pod' ? 'pods' : 'workloads'}</span>
    </div>
    {#if tableLoading && !rows.length}
      <Skeleton rows={8} height={30} />
    {:else if tableError}
      <EmptyState icon="helm" title="Couldn't load the table" body={tableError} actionLabel="Retry" onaction={() => void loadTable()} />
    {:else if !rows.length}
      <EmptyState icon="clock" title="No data in this window" body="Nothing was collected for this selection. Widen the window, clear a filter, or enable monitoring on a cluster." />
    {:else}
      <div class="tablewrap card">
        <table class="wl" data-testid="k8s-fleet-table">
          <thead>
            <tr>
              <th><button class="th-btn" class:on={sort === 'cluster'} onclick={() => sortBy('cluster')}>Cluster{arrow(sort === 'cluster', dir)}</button></th>
              <th><button class="th-btn" class:on={sort === 'namespace'} onclick={() => sortBy('namespace')}>Namespace{arrow(sort === 'namespace', dir)}</button></th>
              <th><button class="th-btn" class:on={sort === 'workload'} onclick={() => sortBy('workload')}>Workload{arrow(sort === 'workload', dir)}</button></th>
              {#if group === 'pod'}
                <th><button class="th-btn" class:on={sort === 'pod'} onclick={() => sortBy('pod')}>Pod{arrow(sort === 'pod', dir)}</button></th>
              {:else}
                <th class="num"><button class="th-btn" class:on={sort === 'pods'} onclick={() => sortBy('pods')}>Pods{arrow(sort === 'pods', dir)}</button></th>
              {/if}
              <th class="num" title="Unplanned restarts in the window; OOM / crash / probe breakdown"><button class="th-btn" class:on={sort === 'restarts'} onclick={() => sortBy('restarts')}>Restarts{arrow(sort === 'restarts', dir)}</button></th>
              <th class="num"><button class="th-btn" class:on={sort === 'oom'} onclick={() => sortBy('oom')}>OOM{arrow(sort === 'oom', dir)}</button></th>
              <th class="num" title="Planned pod replacements"><button class="th-btn" class:on={sort === 'churn'} onclick={() => sortBy('churn')}>Churn{arrow(sort === 'churn', dir)}</button></th>
              <th class="num" title="Latest sample summed over pods"><button class="th-btn" class:on={sort === 'mem_last'} onclick={() => sortBy('mem_last')}>Memory{arrow(sort === 'mem_last', dir)}</button></th>
              <th class="num" title="Hungriest pod sample in the window"><button class="th-btn" class:on={sort === 'mem_max'} onclick={() => sortBy('mem_max')}>Peak{arrow(sort === 'mem_max', dir)}</button></th>
              <th class="num"><button class="th-btn" class:on={sort === 'rps'} onclick={() => sortBy('rps')}>Req/s{arrow(sort === 'rps', dir)}</button></th>
              <th class="num"><button class="th-btn" class:on={sort === 'err_pct'} onclick={() => sortBy('err_pct')}>5xx{arrow(sort === 'err_pct', dir)}</button></th>
              <th class="num"><button class="th-btn" class:on={sort === 'latency_ms'} onclick={() => sortBy('latency_ms')}>Latency{arrow(sort === 'latency_ms', dir)}</button></th>
            </tr>
          </thead>
          <tbody>
            {#each visibleRows as r (`${r.cluster_id}/${r.namespace}/${r.workload}/${r.pod}`)}
              <tr class="wl-row" onclick={() => drillRow(r)} title={group === 'workload' ? "Show this workload's pods" : "Show this pod's events"} data-testid="k8s-fleet-row">
                <td><span class="dot" style="background: {r.cluster.color ?? 'var(--accent)'}"></span> {r.cluster.name} <span class="env-badge" class:prod={r.cluster.environment === 'prod'}>{envBadge(r.cluster.environment)}</span></td>
                <td class="dim">{r.namespace}</td>
                <td><b>{r.workload}</b></td>
                {#if group === 'pod'}<td class="mono small">{r.pod}</td>{:else}<td class="num mono">{r.pods}</td>{/if}
                <td class="num mono" class:bad={restartsTotal(r) > 0}>
                  {restartsTotal(r)}
                  {#if restartsTotal(r) > 0}<span class="dim small"> ({[r.restarts.oom && `oom ${r.restarts.oom}`, r.restarts.crash && `crash ${r.restarts.crash}`, r.restarts.probe && `probe ${r.restarts.probe}`, r.restarts.unknown && `? ${r.restarts.unknown}`].filter(Boolean).join(' · ')})</span>{/if}
                </td>
                <td class="num mono" class:bad={r.restarts.oom > 0}>{r.restarts.oom}</td>
                <td class="num mono">{r.churn}</td>
                <td class="num mono">{r.mem_last ? formatBytes(r.mem_last) : '—'}</td>
                <td class="num mono">{r.mem_max ? formatBytes(r.mem_max) : '—'}</td>
                <td class="num mono">{r.rps ? fmtRate(r.rps) : '—'}</td>
                <td class="num mono" class:bad={r.err_pct >= 5} class:warn={r.err_pct >= 1 && r.err_pct < 5}>{r.rps ? fmtPct(r.err_pct) : '—'}</td>
                <td class="num mono">{r.latency_kind ? `${fmtMs(r.latency_ms)} ${r.latency_kind}` : '—'}</td>
              </tr>
            {/each}
          </tbody>
        </table>
        {#if rows.length < total}
          <div class="more"><button class="btn small ghost" onclick={() => void loadTable(true, true)}>Load {Math.min(PAGE, total - rows.length)} more</button></div>
        {/if}
      </div>
    {/if}
  {:else if activeTab === 'events'}
    <div class="toolbar">
      <select class="input" bind:value={evClass} aria-label="Event class">
        {#each CLASS_OPTIONS as c (c)}
          <option value={c}>{c === '' ? 'Restarts + churn' : c === 'churn' ? 'Churn only' : c === 'version' ? 'New versions' : c === 'k8s_event' ? 'Raw cluster events' : classLabel(c)}</option>
        {/each}
      </select>
      <span class="spacer"></span>
      <span class="dim small" data-testid="k8s-fleet-events-count">{evTotal.toLocaleString()} events</span>
    </div>
    {#if evLoading && !events.length}
      <Skeleton rows={8} height={28} />
    {:else if evError}
      <EmptyState icon="helm" title="Couldn't load events" body={evError} actionLabel="Retry" onaction={() => void loadEvents()} />
    {:else if !events.length}
      <EmptyState icon="check" title="Nothing in this window" body="No restarts or pod replacements were recorded for this selection." />
    {:else}
      <div class="tablewrap card">
        <table class="wl" data-testid="k8s-fleet-events">
          <thead>
            <tr>
              <th><button class="th-btn" class:on={evSort === 'ts'} onclick={() => evSortBy('ts')}>When{arrow(evSort === 'ts', evDir)}</button></th>
              <th><button class="th-btn" class:on={evSort === 'cluster'} onclick={() => evSortBy('cluster')}>Cluster{arrow(evSort === 'cluster', evDir)}</button></th>
              <th><button class="th-btn" class:on={evSort === 'namespace'} onclick={() => evSortBy('namespace')}>Namespace{arrow(evSort === 'namespace', evDir)}</button></th>
              <th><button class="th-btn" class:on={evSort === 'workload'} onclick={() => evSortBy('workload')}>Workload{arrow(evSort === 'workload', evDir)}</button></th>
              <th><button class="th-btn" class:on={evSort === 'pod'} onclick={() => evSortBy('pod')}>Pod{arrow(evSort === 'pod', evDir)}</button></th>
              <th><button class="th-btn" class:on={evSort === 'class'} onclick={() => evSortBy('class')}>Class{arrow(evSort === 'class', evDir)}</button></th>
              <th><button class="th-btn" class:on={evSort === 'reason'} onclick={() => evSortBy('reason')}>Detail{arrow(evSort === 'reason', evDir)}</button></th>
            </tr>
          </thead>
          <tbody>
            {#each events as e, i (i)}
              <tr>
                <td class="mono small nowrap">{fmtTs(e.ts)}</td>
                <td><span class="dot" style="background: {e.cluster.color ?? 'var(--accent)'}"></span> {e.cluster.name}</td>
                <td class="dim">{e.namespace}</td>
                <td><b>{e.workload}</b></td>
                <td class="mono small">{e.pod}</td>
                <td><span class="tclass" style="color: {e.kind === 'version' ? 'var(--status-working)' : classColor(e.class)}">{e.kind === 'k8s_event' ? e.reason : e.kind === 'version' ? 'New version' : e.kind === 'churn' ? `churn · ${classLabel(e.class)}` : classLabel(e.class)}</span></td>
                <td class="dim">{eventMsg(e)}</td>
              </tr>
            {/each}
          </tbody>
        </table>
        {#if events.length < evTotal}
          <div class="more"><button class="btn small ghost" onclick={() => void loadEvents(true, true)}>Load {Math.min(PAGE, evTotal - events.length)} more</button></div>
        {/if}
      </div>
    {/if}
  {:else}
    {#if reqLoading && !reqs}
      <Skeleton rows={6} height={28} />
    {:else if reqError}
      <EmptyState icon="helm" title="Couldn't load requests" body={reqError} actionLabel="Retry" onaction={() => void loadRequests()} />
    {:else if reqs}
      {#if reqs.enabled_on.length === 0}
        <div class="note card" data-testid="k8s-fleet-requests-off">
          <b>No cluster keeps request path labels yet.</b>
          <span class="dim">Per-route drill-down needs <em>Keep request path labels</em> in a cluster's Monitor settings (it multiplies request rows per pod by the number of routes — enable it where you need it).</span>
          <span class="links">
            {#each reqs.disabled_on as c (c.id)}
              <button class="btn small ghost" onclick={() => router.go(`kubernetes/monitor/${encodeURIComponent(c.id)}/settings`)}>{c.name} settings</button>
            {/each}
          </span>
        </div>
      {:else if reqs.disabled_on.length > 0}
        <div class="dim small">Request labels are on for {reqs.enabled_on.map((c) => c.name).join(', ')}; off for {reqs.disabled_on.map((c) => c.name).join(', ')}.</div>
      {/if}
      {#if !reqRows.length}
        {#if reqs.enabled_on.length > 0}
          <EmptyState icon="clock" title="No per-route samples yet" body="Rows appear after the next collection cycle on a cluster with request labels enabled." />
        {/if}
      {:else}
        <div class="tablewrap card">
          <table class="wl" data-testid="k8s-fleet-requests">
            <thead>
              <tr>
                <th><button class="th-btn" class:on={reqSort === 'path'} onclick={() => reqSortBy('path')}>Route{arrow(reqSort === 'path', reqDir)}</button></th>
                <th>Method</th>
                <th class="num"><button class="th-btn" class:on={reqSort === 'rps'} onclick={() => reqSortBy('rps')}>Req/s{arrow(reqSort === 'rps', reqDir)}</button></th>
                <th class="num"><button class="th-btn" class:on={reqSort === 'err_pct'} onclick={() => reqSortBy('err_pct')}>5xx{arrow(reqSort === 'err_pct', reqDir)}</button></th>
                <th class="num"><button class="th-btn" class:on={reqSort === 'avg_ms'} onclick={() => reqSortBy('avg_ms')}>Avg latency{arrow(reqSort === 'avg_ms', reqDir)}</button></th>
              </tr>
            </thead>
            <tbody>
              {#each reqRows as r (`${r.method} ${r.path}`)}
                <tr>
                  <td class="mono">{r.path}</td>
                  <td class="mono small dim">{r.method || '—'}</td>
                  <td class="num mono">{fmtRate(r.rps)}</td>
                  <td class="num mono" class:bad={r.err_pct >= 5} class:warn={r.err_pct >= 1 && r.err_pct < 5}>{fmtPct(r.err_pct)}</td>
                  <td class="num mono">{r.avg_ms ? fmtMs(r.avg_ms) : '—'}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
    {/if}
  {/if}
</div>

<style>
  .page {
    padding: 16px 20px 24px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    height: 100%;
    overflow-y: auto;
  }
  .page-header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 12px;
    flex-wrap: wrap;
  }
  h1 {
    margin: 0;
    font-size: 17px;
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .crumb {
    background: none;
    border: none;
    padding: 0;
    font: inherit;
    color: var(--text-dim);
    cursor: pointer;
  }
  .crumb:hover {
    color: var(--text);
  }
  .sep {
    color: var(--text-dim);
  }
  .sub {
    font-size: 12px;
    color: var(--text-dim);
    margin-top: 2px;
    max-width: 720px;
  }
  .actions {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .seg {
    display: inline-flex;
    border: 1px solid var(--border);
    border-radius: 999px;
    overflow: hidden;
  }
  .seg-btn {
    background: none;
    border: none;
    padding: 3px 10px;
    font-size: 11.5px;
    color: var(--text-dim);
    cursor: pointer;
  }
  .seg-btn.on {
    background: var(--surface-2);
    color: var(--text);
    font-weight: 600;
  }
  .card {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
  }
  .filters {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px 12px;
  }
  .pills {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-items: center;
  }
  .pill {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 3px 10px 3px 8px;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: 12px;
    cursor: pointer;
  }
  .pill:hover {
    border-color: var(--accent);
  }
  .pill.on {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    border-color: var(--accent);
  }
  .pill.empty {
    opacity: 0.6;
  }
  .dot {
    display: inline-block;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    vertical-align: middle;
  }
  .env-badge {
    font-size: 8.5px;
    font-weight: 700;
    letter-spacing: 0.04em;
    padding: 1px 5px;
    border-radius: 999px;
    color: var(--status-working);
    background: color-mix(in srgb, var(--status-working) 16%, transparent);
  }
  .env-badge.prod {
    color: var(--status-exited);
    background: color-mix(in srgb, var(--status-exited) 16%, transparent);
  }
  .selects {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    align-items: center;
  }
  .selects .input {
    max-width: 240px;
  }
  .tabs {
    display: flex;
    gap: 2px;
    border-bottom: 1px solid var(--border);
  }
  .tab {
    background: none;
    border: none;
    border-bottom: 2px solid transparent;
    padding: 6px 12px;
    font-size: 12.5px;
    color: var(--text-dim);
    cursor: pointer;
  }
  .tab.active {
    color: var(--text);
    border-bottom-color: var(--accent);
    font-weight: 600;
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .toolbar .input {
    max-width: 260px;
  }
  .spacer {
    flex: 1;
  }
  .kpis {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
    gap: 10px;
  }
  .kpi {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 10px 12px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
  }
  .kpi .k {
    font-size: 10.5px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
  }
  .kpi .v {
    font-size: 20px;
    font-weight: 700;
    line-height: 1.1;
  }
  .kpi .d {
    font-size: 11px;
    color: var(--text-dim);
  }
  .charts {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(340px, 1fr));
    gap: 10px;
  }
  .chart {
    padding: 10px 12px;
    min-width: 0;
  }
  .chart-head {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    font-size: 12px;
    margin-bottom: 4px;
  }
  .tablewrap {
    overflow-x: auto;
  }
  .wl {
    width: 100%;
    border-collapse: collapse;
    font-size: 12px;
  }
  .wl th {
    text-align: left;
    font-size: 10.5px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
    padding: 8px 10px;
    border-bottom: 1px solid var(--border);
    white-space: nowrap;
  }
  .th-btn {
    background: none;
    border: none;
    padding: 0;
    font: inherit;
    color: inherit;
    text-transform: inherit;
    letter-spacing: inherit;
    cursor: pointer;
  }
  .th-btn:hover,
  .th-btn.on {
    color: var(--text);
  }
  .wl td {
    padding: 7px 10px;
    border-bottom: 1px solid var(--border);
    vertical-align: top;
  }
  .wl-row {
    cursor: pointer;
  }
  .wl-row:hover {
    background: color-mix(in srgb, var(--accent) 4%, transparent);
  }
  .num {
    text-align: right;
    white-space: nowrap;
  }
  .nowrap {
    white-space: nowrap;
  }
  .mono {
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
  }
  .small {
    font-size: 11px;
  }
  .dim {
    color: var(--text-dim);
  }
  .bad {
    color: var(--status-exited);
  }
  .warn {
    color: var(--status-warn);
  }
  .tclass {
    font-weight: 600;
  }
  .more {
    display: flex;
    justify-content: center;
    padding: 8px;
  }
  .note {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 12px 14px;
    font-size: 12.5px;
  }
  .note .links {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  @media (max-width: 720px) {
    .page {
      padding: 12px;
    }
    .charts {
      grid-template-columns: 1fr;
    }
  }
</style>
