<script lang="ts">
  // Fleet dashboard (`#/kubernetes/monitor/fleet[/<overview|table|events|requests>]`):
  // ONE view over every monitored cluster, read from ClickHouse only — the
  // general picture over a window (restarts / OOMs, memory, req/s, 5xx,
  // latency), not the live pod status. Filters (window, clusters, namespace,
  // workload, pod), the table grouping / sort and the events sort are all
  // persisted per device, and every row is a drill-down (cluster → namespace
  // → workload → pod → events).
  import { loadErrorText } from '../../../lib/loadError';
  import { tick, untrack } from 'svelte';
  import { radioKey } from '../../../lib/radioKey';
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
  import PageHeader from '../../../lib/components/PageHeader.svelte';
  import PageBody from '../../../lib/components/PageBody.svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Skeleton from '../../../lib/components/Skeleton.svelte';
  import MetricChart from '../../../lib/components/MetricChart.svelte';
  import { formatBytes } from '../k8s-util';
  import EnvBadge from '../../../lib/components/EnvBadge.svelte';
  import VirtualList from '../../../lib/components/VirtualList.svelte';
  import { ApiError } from '../../../lib/api/client';
  import { WINDOWS, classColor, classLabel, fmtMs, fmtPct, fmtRate, isWindow, liveRefreshLimit, FLEET_TABLE_MAX, FLEET_EVENTS_MAX, fleetEventKey, fleetRowKey, reuseByKey, uniqueKeys } from './monitor-util';

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
      filtersError = loadErrorText(e);
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
  // Raw: replaced wholesale, never mutated in place (up to 2k rows).
  let rows = $state.raw<K8sFleetRow[]>([]);
  let total = $state(0);
  let tableLoading = $state(true);
  let tableError = $state('');
  let quick = $state('');
  const PAGE = 200;
  /** Fleet table / events row height (px): fixed, the lists are windowed. */
  const ROW_H = 32;
  /** Keyboard focus inside the table (roving tabindex) + VirtualList scroll. */
  let tableFocus = $state(0);
  let tableScrollIndex = $state(-1);
  let tableScrollVersion = $state(0);
  let tAbort: AbortController | null = null;
  let tAppendCtrl: AbortController | null = null;
  async function loadTable(quiet = false, append = false): Promise<void> {
    // A live (quiet) refresh must not throw away the user's "Load more"
    // pages or abort one in flight (r3-02-04): it re-reads as many rows as
    // are loaded, or skips when that is past the server's page cap.
    const refreshLimit = liveRefreshLimit(quiet && !append, rows.length, PAGE, FLEET_TABLE_MAX, tAppendCtrl !== null);
    if (refreshLimit === null) return;
    tAbort?.abort();
    const ctrl = (tAbort = new AbortController());
    if (!quiet && !append) tableLoading = true;
    if (append) tAppendCtrl = ctrl;
    try {
      const r = await k8sApi.fleetTable({ ...sel, group, sort, dir, limit: refreshLimit, offset: append ? rows.length : 0 }, ctrl.signal);
      const before = rows.length;
      // perf K8s: unchanged rows keep their object (keyed each skips them).
      rows = append ? [...rows, ...r.rows] : reuseByKey(rows, r.rows, fleetRowKey);
      total = r.total;
      tableError = '';
      // "Load more": bring the first new row into view (the list is windowed).
      if (append && rows.length > before) void scrollTableTo(before);
    } catch (e) {
      if (e instanceof DOMException && e.name === 'AbortError') return;
      tableError = loadErrorText(e);
    } finally {
      tableLoading = false;
      if (tAppendCtrl === ctrl) tAppendCtrl = null;
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
  // Raw: replaced per load; an unchanged metric keeps its object so the
  // memoised chart data below (and the chart) stays put.
  let charts = $state.raw<Partial<Record<K8sFleetMetric, K8sFleetSeries>>>({});
  let chartsLoading = $state(true);
  let chartsError = $state('');
  let sAbort: AbortController | null = null;
  async function loadSeries(quiet = false): Promise<void> {
    sAbort?.abort();
    sAbort = new AbortController();
    if (!quiet) chartsLoading = true;
    try {
      const fresh = await fetchSeries(sAbort.signal);
      const next: Partial<Record<K8sFleetMetric, K8sFleetSeries>> = {};
      for (const m of METRICS) {
        const f = fresh[m.id];
        const p = charts[m.id];
        next[m.id] = p && f && JSON.stringify(p) === JSON.stringify(f) ? p : f;
      }
      charts = next;
      chartsError = '';
    } catch (e) {
      if (e instanceof DOMException && e.name === 'AbortError') return;
      chartsError = loadErrorText(e);
    } finally {
      chartsLoading = false;
    }
  }
  /** perf K8s: every chart in ONE request (`/fleet/series/batch`). A daemon
   *  without the batch route (404) gets the per-metric reads instead. */
  let batchMissing = false;
  async function fetchSeries(signal: AbortSignal): Promise<Partial<Record<K8sFleetMetric, K8sFleetSeries>>> {
    if (!batchMissing) {
      try {
        return (await k8sApi.fleetSeriesBatch({ ...sel, metrics: METRICS.map((m) => m.id), by }, signal)).series;
      } catch (e) {
        if (!(e instanceof ApiError && e.status === 404)) throw e;
        batchMissing = true;
      }
    }
    const all = await Promise.all(METRICS.map((m) => k8sApi.fleetSeries({ ...sel, metric: m.id, by }, signal)));
    const out: Partial<Record<K8sFleetMetric, K8sFleetSeries>> = {};
    METRICS.forEach((m, i) => (out[m.id] = all[i]));
    return out;
  }
  /** toChart per metric, memoised on the series object (unchanged metric ⇒
   *  same array ⇒ the chart doesn't redraw). */
  const chartMemo = new WeakMap<K8sFleetSeries, MetricChartSeries[]>();
  const chartData = $derived.by(() => {
    const out: Partial<Record<K8sFleetMetric, MetricChartSeries[]>> = {};
    for (const m of METRICS) {
      const s = charts[m.id];
      if (!s) {
        out[m.id] = [];
        continue;
      }
      let c = chartMemo.get(s);
      if (!c) {
        c = toChart(s);
        chartMemo.set(s, c);
      }
      out[m.id] = c;
    }
    return out;
  });
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
  let events = $state.raw<K8sFleetEvent[]>([]);
  let evTotal = $state(0);
  let evLoading = $state(true);
  let evError = $state('');
  let eAbort: AbortController | null = null;
  let eAppendCtrl: AbortController | null = null;
  async function loadEvents(quiet = false, append = false): Promise<void> {
    // Same rule as the table: a live refresh keeps paged-in events.
    const refreshLimit = liveRefreshLimit(quiet && !append, events.length, PAGE, FLEET_EVENTS_MAX, eAppendCtrl !== null);
    if (refreshLimit === null) return;
    eAbort?.abort();
    const ctrl = (eAbort = new AbortController());
    if (!quiet && !append) evLoading = true;
    if (append) eAppendCtrl = ctrl;
    try {
      const r = await k8sApi.fleetEvents({ ...sel, class: evClass || undefined, sort: evSort, dir: evDir, limit: refreshLimit, offset: append ? events.length : 0 }, ctrl.signal);
      events = append ? [...events, ...r.rows] : reuseByKey(events, r.rows, fleetEventKey);
      evTotal = r.total;
      evError = '';
    } catch (e) {
      if (e instanceof DOMException && e.name === 'AbortError') return;
      evError = loadErrorText(e);
    } finally {
      evLoading = false;
      if (eAppendCtrl === ctrl) eAppendCtrl = null;
    }
  }
  /** Stable, unique per-event keys for the windowed list (never the index). */
  const eventKeys = $derived(uniqueKeys(events.map(fleetEventKey)));
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
      reqError = loadErrorText(e);
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
  // Live refresh after collection cycles — coalesced by the store (at most
  // one per MONITOR_TICK_MIN_MS, none while hidden). Only a NEW tick: the
  // effects above already load on mount.
  let seenTick = untrack(() => k8s.monitorTick);
  $effect(() => {
    const tick = k8s.monitorTick;
    if (tick !== seenTick) {
      seenTick = tick;
      untrack(() => {
        if (activeTab === 'overview') {
          void loadTable(true);
          void loadSeries(true);
        }
        if (activeTab === 'table') void loadTable(true);
        if (activeTab === 'events') void loadEvents(true);
        if (activeTab === 'requests') void loadRequests(true);
        // perf K8s: no loadFilters here — the filter options only change with
        // the window / selection (its own effect) or a manual Refresh.
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
  // Table keyboard: one tab stop (roving tabindex); ↑/↓/Home/End move, Enter
  // drills (the same as a click). The focused row stays mounted (VirtualList
  // `pinnedIndex`) even when scrolled out of the window.
  let tableEl = $state<HTMLDivElement | null>(null);
  const focusIdx = $derived(Math.max(0, Math.min(tableFocus, visibleRows.length - 1)));
  function rowKeydown(e: KeyboardEvent, r: K8sFleetRow, i: number): void {
    if (e.target !== e.currentTarget) return; // a drill button handles its own Enter
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      drillRow(r);
      return;
    }
    const n = visibleRows.length;
    let j = -1;
    if (e.key === 'ArrowDown') j = Math.min(n - 1, i + 1);
    else if (e.key === 'ArrowUp') j = Math.max(0, i - 1);
    else if (e.key === 'Home') j = 0;
    else if (e.key === 'End') j = n - 1;
    if (j < 0) return;
    e.preventDefault();
    tableFocus = j;
    void scrollTableTo(j).then(() => tableEl?.querySelector<HTMLElement>(`[data-i="${j}"]`)?.focus());
  }
  /** One-shot scroll of the windowed table to row `i` (cleared after the
   *  flush, so a later refresh never yanks the list back there). */
  async function scrollTableTo(i: number): Promise<void> {
    tableScrollIndex = i;
    tableScrollVersion += 1;
    await tick();
    tableScrollIndex = -1;
  }
  function restartBreakdown(r: K8sFleetRow): string {
    return [r.restarts.oom && `oom ${r.restarts.oom}`, r.restarts.crash && `crash ${r.restarts.crash}`, r.restarts.probe && `probe ${r.restarts.probe}`, r.restarts.unknown && `? ${r.restarts.unknown}`].filter(Boolean).join(' · ');
  }
  /** The table's shared grid template (header + every windowed row). */
  const tableCols = $derived(
    `minmax(150px, 1.2fr) minmax(110px, 1fr) minmax(150px, 1.4fr) ${group === 'pod' ? 'minmax(170px, 1.4fr)' : '64px'} minmax(150px, 1fr) 56px 64px 88px 88px 80px 70px 120px`,
  );
  const EV_COLS = '130px minmax(130px, 1fr) minmax(110px, 1fr) minmax(140px, 1.2fr) minmax(170px, 1.4fr) 130px minmax(200px, 2fr)';
  const ariaSort = (on: boolean, d: 'asc' | 'desc'): 'ascending' | 'descending' | 'none' => (on ? (d === 'asc' ? 'ascending' : 'descending') : 'none');
  function restartsTotal(r: K8sFleetRow): number {
    return r.restarts.oom + r.restarts.crash + r.restarts.probe + r.restarts.unknown;
  }
  const arrow = (on: boolean, d: 'asc' | 'desc'): string => (on ? (d === 'asc' ? ' ↑' : ' ↓') : '');
</script>

<div class="fleet-page" data-testid="k8s-fleet">
<PageHeader
  title="Fleet"
  crumbs={[
    { label: 'Kubernetes', onclick: () => router.go('kubernetes') },
    { label: 'Monitor', onclick: () => router.go('kubernetes/monitor') },
  ]}
  subtitle="Every monitored cluster in one dashboard — restarts & OOMs, memory, requests, latency — straight from ClickHouse. Filters, grouping and ordering stick."
>
  {#snippet actions()}
    <div class="seg" role="radiogroup" aria-label="Window" data-keep>
      {#each WINDOWS as w (w)}
        <button class="seg-btn" class:on={window === w} role="radio" onkeydown={radioKey} aria-checked={window === w} tabindex={window === w ? 0 : -1} onclick={() => (window = w)}>{w}</button>
      {/each}
    </div>
    <button class="icon-btn" onclick={refresh} title="Refresh" aria-label="Refresh fleet"><Icon name="refresh" size={14} /></button>
  {/snippet}
</PageHeader>
<PageBody>
<div class="fleet">

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
          <EnvBadge env={c.environment} />
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
        <button class="btn small ghost" onclick={clearFilters} data-testid="k8s-fleet-clear"><Icon name="x" size={12} /> Clear</button>
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
      <EmptyState actionKind="secondary" actionIcon="refresh" icon="warning" title="Couldn't load the fleet" body={tableError} actionLabel="Retry" onaction={refresh} />
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
          <button class="seg-btn" class:on={by === b} role="radio" onkeydown={radioKey} aria-checked={by === b} tabindex={by === b ? 0 : -1} onclick={() => (by = b)}>{b}</button>
        {/each}
      </div>
      <span class="dim small">(restarts are always per class)</span>
    </div>
    {#if chartsLoading && !Object.keys(charts).length}
      <Skeleton rows={3} height={160} />
    {:else if chartsError}
      <EmptyState actionKind="secondary" icon="warning" title="Couldn't load the charts" body={chartsError} actionLabel="Retry" onaction={() => void loadSeries()} />
    {:else}
      <div class="charts" data-testid="k8s-fleet-charts">
        {#each METRICS as m (m.id)}
          <div class="card chart">
            <div class="chart-head"><b>{m.label}</b><span class="dim small">{charts[m.id]?.step_secs ? `${charts[m.id]?.step_secs}s buckets` : ''}</span></div>
            <MetricChart series={chartData[m.id] ?? []} unit={m.unit} height={150} area={m.id !== 'restarts'} emptyText="No samples in this window" />
          </div>
        {/each}
      </div>
    {/if}
  {:else if activeTab === 'table'}
    <div class="toolbar">
      <input class="input" placeholder="Quick filter…" bind:value={quick} aria-label="Quick filter" />
      <div class="seg" role="radiogroup" aria-label="Group by">
        <button class="seg-btn" class:on={group === 'workload'} role="radio" onkeydown={radioKey} aria-checked={group === 'workload'} tabindex={group === 'workload' ? 0 : -1} onclick={() => (group = 'workload')}>Workloads</button>
        <button class="seg-btn" class:on={group === 'pod'} role="radio" onkeydown={radioKey} aria-checked={group === 'pod'} tabindex={group === 'pod' ? 0 : -1} onclick={() => (group = 'pod')}>Pods</button>
      </div>
      <span class="spacer"></span>
      <span class="dim small" data-testid="k8s-fleet-table-count">{total.toLocaleString()} {group === 'pod' ? 'pods' : 'workloads'}</span>
    </div>
    {#if tableLoading && !rows.length}
      <Skeleton rows={8} height={30} />
    {:else if tableError}
      <EmptyState actionKind="secondary" icon="warning" title="Couldn't load the table" body={tableError} actionLabel="Retry" onaction={() => void loadTable()} />
    {:else if !rows.length}
      <EmptyState icon="clock" title="No data in this window" body="Nothing was collected for this selection. Widen the window, clear a filter, or enable monitoring on a cluster." />
    {:else}
      <div class="tablewrap card">
        <div class="vt" role="table" aria-label={group === 'pod' ? 'Fleet pods' : 'Fleet workloads'} aria-rowcount={visibleRows.length + 1} data-testid="k8s-fleet-table" bind:this={tableEl}>
          <div role="rowgroup">
            <div class="vt-head" role="row" aria-rowindex={1} style="grid-template-columns:{tableCols}">
              <div class="vt-th" role="columnheader" aria-sort={ariaSort(sort === 'cluster', dir)}><button class="th-btn" class:on={sort === 'cluster'} onclick={() => sortBy('cluster')}>Cluster{arrow(sort === 'cluster', dir)}</button></div>
              <div class="vt-th" role="columnheader" aria-sort={ariaSort(sort === 'namespace', dir)}><button class="th-btn" class:on={sort === 'namespace'} onclick={() => sortBy('namespace')}>Namespace{arrow(sort === 'namespace', dir)}</button></div>
              <div class="vt-th" role="columnheader" aria-sort={ariaSort(sort === 'workload', dir)}><button class="th-btn" class:on={sort === 'workload'} onclick={() => sortBy('workload')}>Workload{arrow(sort === 'workload', dir)}</button></div>
              {#if group === 'pod'}
                <div class="vt-th" role="columnheader" aria-sort={ariaSort(sort === 'pod', dir)}><button class="th-btn" class:on={sort === 'pod'} onclick={() => sortBy('pod')}>Pod{arrow(sort === 'pod', dir)}</button></div>
              {:else}
                <div class="vt-th num" role="columnheader" aria-sort={ariaSort(sort === 'pods', dir)}><button class="th-btn" class:on={sort === 'pods'} onclick={() => sortBy('pods')}>Pods{arrow(sort === 'pods', dir)}</button></div>
              {/if}
              <div class="vt-th num" role="columnheader" aria-sort={ariaSort(sort === 'restarts', dir)} title="Unplanned restarts in the window; OOM / crash / probe breakdown"><button class="th-btn" class:on={sort === 'restarts'} onclick={() => sortBy('restarts')}>Restarts{arrow(sort === 'restarts', dir)}</button></div>
              <div class="vt-th num" role="columnheader" aria-sort={ariaSort(sort === 'oom', dir)}><button class="th-btn" class:on={sort === 'oom'} onclick={() => sortBy('oom')}>OOM{arrow(sort === 'oom', dir)}</button></div>
              <div class="vt-th num" role="columnheader" aria-sort={ariaSort(sort === 'churn', dir)} title="Planned pod replacements"><button class="th-btn" class:on={sort === 'churn'} onclick={() => sortBy('churn')}>Churn{arrow(sort === 'churn', dir)}</button></div>
              <div class="vt-th num" role="columnheader" aria-sort={ariaSort(sort === 'mem_last', dir)} title="Latest sample summed over pods"><button class="th-btn" class:on={sort === 'mem_last'} onclick={() => sortBy('mem_last')}>Memory{arrow(sort === 'mem_last', dir)}</button></div>
              <div class="vt-th num" role="columnheader" aria-sort={ariaSort(sort === 'mem_max', dir)} title="Hungriest pod sample in the window"><button class="th-btn" class:on={sort === 'mem_max'} onclick={() => sortBy('mem_max')}>Peak{arrow(sort === 'mem_max', dir)}</button></div>
              <div class="vt-th num" role="columnheader" aria-sort={ariaSort(sort === 'rps', dir)}><button class="th-btn" class:on={sort === 'rps'} onclick={() => sortBy('rps')}>Req/s{arrow(sort === 'rps', dir)}</button></div>
              <div class="vt-th num" role="columnheader" aria-sort={ariaSort(sort === 'err_pct', dir)}><button class="th-btn" class:on={sort === 'err_pct'} onclick={() => sortBy('err_pct')}>5xx{arrow(sort === 'err_pct', dir)}</button></div>
              <div class="vt-th num" role="columnheader" aria-sort={ariaSort(sort === 'latency_ms', dir)}><button class="th-btn" class:on={sort === 'latency_ms'} onclick={() => sortBy('latency_ms')}>Latency{arrow(sort === 'latency_ms', dir)}</button></div>
            </div>
          </div>
          <div role="rowgroup" class="vt-body">
            <VirtualList items={visibleRows} estimateHeight={ROW_H} class="vt-list" key={fleetRowKey} pinnedIndex={focusIdx} scrollIndex={tableScrollIndex} scrollVersion={tableScrollVersion}>
              {#snippet row(r, i)}
                {@const rt = restartsTotal(r)}
                <div
                  class="vt-row wl-row"
                  role="row"
                  aria-rowindex={i + 2}
                  tabindex={i === focusIdx ? 0 : -1}
                  data-i={i}
                  style="grid-template-columns:{tableCols};height:{ROW_H}px"
                  onclick={() => drillRow(r)}
                  onkeydown={(e) => rowKeydown(e, r, i)}
                  onfocus={() => (tableFocus = i)}
                  title={group === 'workload' ? "Show this workload's pods" : "Show this pod's events"}
                  data-testid="k8s-fleet-row"
                >
                  <div class="vt-td" role="cell"><span class="dot" style="background: {r.cluster.color ?? 'var(--accent)'}"></span> {r.cluster.name} <EnvBadge env={r.cluster.environment} /></div>
                  <div class="vt-td dim" role="cell" title={r.namespace}>{r.namespace}</div>
                  <div class="vt-td" role="cell" title={r.workload}>{#if group === 'workload'}<button class="drill-btn" tabindex="-1" aria-label={`Show pods for ${r.workload}`} onclick={(e) => { e.stopPropagation(); drillRow(r); }}>{r.workload}</button>{:else}<b>{r.workload}</b>{/if}</div>
                  {#if group === 'pod'}<div class="vt-td mono small" role="cell" title={r.pod}><button class="drill-btn" tabindex="-1" aria-label={`Show events for ${r.pod}`} onclick={(e) => { e.stopPropagation(); drillRow(r); }}>{r.pod}</button></div>{:else}<div dir="ltr" class="vt-td num mono" role="cell">{r.pods}</div>{/if}
                  <div dir="ltr" class="vt-td num mono" role="cell" class:bad={rt > 0} title={rt > 0 ? restartBreakdown(r) : undefined}>
                    {rt}{#if rt > 0}<span class="dim small"> ({restartBreakdown(r)})</span>{/if}
                  </div>
                  <div dir="ltr" class="vt-td num mono" role="cell" class:bad={r.restarts.oom > 0}>{r.restarts.oom}</div>
                  <div dir="ltr" class="vt-td num mono" role="cell">{r.churn}</div>
                  <div dir="ltr" class="vt-td num mono" role="cell">{r.mem_last ? formatBytes(r.mem_last) : '—'}</div>
                  <div dir="ltr" class="vt-td num mono" role="cell">{r.mem_max ? formatBytes(r.mem_max) : '—'}</div>
                  <div dir="ltr" class="vt-td num mono" role="cell">{r.rps ? fmtRate(r.rps) : '—'}</div>
                  <div dir="ltr" class="vt-td num mono" role="cell" class:bad={r.err_pct >= 5} class:warn={r.err_pct >= 1 && r.err_pct < 5}>{r.rps ? fmtPct(r.err_pct) : '—'}</div>
                  <div dir="ltr" class="vt-td num mono" role="cell">{r.latency_kind ? `${fmtMs(r.latency_ms)} ${r.latency_kind}` : '—'}</div>
                </div>
              {/snippet}
            </VirtualList>
          </div>
        </div>
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
      <EmptyState actionKind="secondary" icon="warning" title="Couldn't load events" body={evError} actionLabel="Retry" onaction={() => void loadEvents()} />
    {:else if !events.length}
      <EmptyState icon="check" title="Nothing in this window" body="No restarts or pod replacements were recorded for this selection." />
    {:else}
      <div class="tablewrap card">
        <div class="vt ev" role="table" aria-label="Fleet events" aria-rowcount={events.length + 1} data-testid="k8s-fleet-events">
          <div role="rowgroup">
            <div class="vt-head" role="row" aria-rowindex={1} style="grid-template-columns:{EV_COLS}">
              <div class="vt-th" role="columnheader" aria-sort={ariaSort(evSort === 'ts', evDir)}><button class="th-btn" class:on={evSort === 'ts'} onclick={() => evSortBy('ts')}>When{arrow(evSort === 'ts', evDir)}</button></div>
              <div class="vt-th" role="columnheader" aria-sort={ariaSort(evSort === 'cluster', evDir)}><button class="th-btn" class:on={evSort === 'cluster'} onclick={() => evSortBy('cluster')}>Cluster{arrow(evSort === 'cluster', evDir)}</button></div>
              <div class="vt-th" role="columnheader" aria-sort={ariaSort(evSort === 'namespace', evDir)}><button class="th-btn" class:on={evSort === 'namespace'} onclick={() => evSortBy('namespace')}>Namespace{arrow(evSort === 'namespace', evDir)}</button></div>
              <div class="vt-th" role="columnheader" aria-sort={ariaSort(evSort === 'workload', evDir)}><button class="th-btn" class:on={evSort === 'workload'} onclick={() => evSortBy('workload')}>Workload{arrow(evSort === 'workload', evDir)}</button></div>
              <div class="vt-th" role="columnheader" aria-sort={ariaSort(evSort === 'pod', evDir)}><button class="th-btn" class:on={evSort === 'pod'} onclick={() => evSortBy('pod')}>Pod{arrow(evSort === 'pod', evDir)}</button></div>
              <div class="vt-th" role="columnheader" aria-sort={ariaSort(evSort === 'class', evDir)}><button class="th-btn" class:on={evSort === 'class'} onclick={() => evSortBy('class')}>Class{arrow(evSort === 'class', evDir)}</button></div>
              <div class="vt-th" role="columnheader" aria-sort={ariaSort(evSort === 'reason', evDir)}><button class="th-btn" class:on={evSort === 'reason'} onclick={() => evSortBy('reason')}>Detail{arrow(evSort === 'reason', evDir)}</button></div>
            </div>
          </div>
          <div role="rowgroup" class="vt-body">
            <VirtualList items={events} estimateHeight={ROW_H} class="vt-list" key={(_e, i) => eventKeys[i]}>
              {#snippet row(e, i)}
                {@const msg = eventMsg(e)}
                <div class="vt-row" role="row" aria-rowindex={i + 2} style="grid-template-columns:{EV_COLS};height:{ROW_H}px" data-testid="k8s-fleet-event-row">
                  <div class="vt-td mono small" role="cell">{fmtTs(e.ts)}</div>
                  <div class="vt-td" role="cell" title={e.cluster.name}><span class="dot" style="background: {e.cluster.color ?? 'var(--accent)'}"></span> {e.cluster.name}</div>
                  <div class="vt-td dim" role="cell" title={e.namespace}>{e.namespace}</div>
                  <div class="vt-td" role="cell" title={e.workload}><b>{e.workload}</b></div>
                  <div class="vt-td mono small" role="cell" title={e.pod}>{e.pod}</div>
                  <div class="vt-td" role="cell"><span class="tclass" style="color: {e.kind === 'version' ? 'var(--status-working)' : classColor(e.class)}">{e.kind === 'k8s_event' ? e.reason : e.kind === 'version' ? 'New version' : e.kind === 'churn' ? `churn · ${classLabel(e.class)}` : classLabel(e.class)}</span></div>
                  <div class="vt-td dim" role="cell" title={msg}>{msg}</div>
                </div>
              {/snippet}
            </VirtualList>
          </div>
        </div>
        {#if events.length < evTotal}
          <div class="more"><button class="btn small ghost" onclick={() => void loadEvents(true, true)}>Load {Math.min(PAGE, evTotal - events.length)} more</button></div>
        {/if}
      </div>
    {/if}
  {:else}
    {#if reqLoading && !reqs}
      <Skeleton rows={6} height={28} />
    {:else if reqError}
      <EmptyState actionKind="secondary" icon="warning" title="Couldn't load requests" body={reqError} actionLabel="Retry" onaction={() => void loadRequests()} />
    {:else if reqs}
      {#if reqs.enabled_on.length === 0}
        <div class="note card" data-testid="k8s-fleet-requests-off">
          <b>No cluster keeps request path labels yet.</b>
          <span class="dim">Per-route drill-down needs <em>Keep request path labels</em> in a cluster's Monitor settings (it multiplies request rows per pod by the number of routes — enable it where you need it).</span>
          <span class="links">
            {#each reqs.disabled_on as c (c.id)}
              <button class="btn small ghost" onclick={() => router.go(`kubernetes/${encodeURIComponent(c.id)}/monitor/settings`)}>{c.name} settings</button>
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
                  <td class="mono" dir="ltr">{r.path}</td>
                  <td class="mono small dim" dir="ltr">{r.method || '—'}</td>
                  <td dir="ltr" class="num mono">{fmtRate(r.rps)}</td>
                  <td dir="ltr" class="num mono" class:bad={r.err_pct >= 5} class:warn={r.err_pct >= 1 && r.err_pct < 5}>{fmtPct(r.err_pct)}</td>
                  <td dir="ltr" class="num mono">{r.avg_ms ? fmtMs(r.avg_ms) : '—'}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
    {/if}
  {/if}
</div>
</PageBody>
</div>

<style>
  .fleet-page {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .fleet {
    display: flex;
    flex-direction: column;
    gap: 12px;
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
    font-size: var(--fs-s);
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
    padding-block: 3px; padding-inline: 8px 10px;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-s);
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
    font-size: var(--fs-m);
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
    font-size: var(--fs-xs);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
  }
  .kpi .v {
    font-size: var(--fs-xl);
    font-weight: 600;
    line-height: 1.1;
  }
  .kpi .d {
    font-size: var(--fs-xs);
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
    font-size: var(--fs-s);
    margin-bottom: 4px;
  }
  .tablewrap {
    overflow-x: auto;
  }
  .wl {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--fs-s);
  }
  .wl th {
    text-align: start;
    font-size: var(--fs-xs);
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
  .drill-btn {
    padding: 0;
    border: 0;
    background: transparent;
    color: inherit;
    font: inherit;
    font-weight: 600;
    cursor: pointer;
  }
  /* Windowed Fleet table / events (perf K8s): a sticky header row + a
     VirtualList body sharing one grid template; fixed-height rows. */
  .vt {
    display: flex;
    flex-direction: column;
    min-width: 1180px;
    font-size: var(--fs-s);
  }
  .vt.ev {
    min-width: 1040px;
  }
  .vt-head,
  .vt-row {
    display: grid;
    align-items: center;
    width: 100%;
    box-sizing: border-box;
  }
  .vt-head {
    position: sticky;
    top: 0;
    z-index: 1;
    background: var(--surface);
    border-bottom: 1px solid var(--border);
  }
  .vt-th {
    padding: 8px 10px;
    font-size: var(--fs-xs);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
    text-align: start;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .vt-body :global(.vt-list) {
    max-height: min(70vh, 720px);
  }
  .vt-row {
    border-bottom: 1px solid var(--border);
  }
  .vt-row:focus-visible {
    outline: none;
    box-shadow: inset 0 0 0 2px color-mix(in srgb, var(--accent) 55%, transparent);
  }
  .vt-td {
    padding: 0 10px;
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .wl-row {
    cursor: pointer;
  }
  .wl-row:hover {
    background: color-mix(in srgb, var(--accent) 4%, transparent);
  }
  .num {
    text-align: end;
    white-space: nowrap;
  }
  .mono {
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
  }
  .small {
    font-size: var(--fs-xs);
  }
  .dim {
    color: var(--text-dim);
  }
  .bad {
    color: var(--danger);
  }
  .warn {
    color: var(--warning);
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
    font-size: var(--fs-m);
  }
  .note .links {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  @media (max-width: 640px) {
    .charts {
      grid-template-columns: 1fr;
    }
  }
</style>
