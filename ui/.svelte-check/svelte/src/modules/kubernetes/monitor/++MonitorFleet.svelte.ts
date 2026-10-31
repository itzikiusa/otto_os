///<reference types="svelte" />
;// Fleet dashboard (`#/kubernetes/monitor/fleet[/<overview|table|events|requests>]`):
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
import LoadState from '../../../lib/components/LoadState.svelte';
import Skeleton from '../../../lib/components/Skeleton.svelte';
import MetricChart from '../../../lib/components/MetricChart.svelte';
import { formatBytes } from '../k8s-util';
import EnvBadge from '../../../lib/components/EnvBadge.svelte';
import VirtualList from '../../../lib/components/VirtualList.svelte';
import { ApiError } from '../../../lib/api/client';
import { WINDOWS, classColor, classLabel, fmtMs, fmtPct, fmtRate, isWindow, liveRefreshLimit, FLEET_TABLE_MAX, FLEET_EVENTS_MAX, fleetEventKey, fleetRowKey, reuseByKey, uniqueKeys } from './monitor-util';
;

  interface Props {
    tab: string;
  };;
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
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

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
;
async () => {

 { svelteHTML.createElement("div", {   "class":`fleet-page`,...__sveltets_2_empty({"data-testid":`k8s-fleet`}),});
  { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {      "title":`Fleet`,"crumbs":[
    { label: 'Kubernetes', onclick: () => router.go('kubernetes') },
    { label: 'Monitor', onclick: () => router.go('kubernetes/monitor') },
  ],"subtitle":`Restarts, memory, requests and latency across every cluster`,actions:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("div", {      "class":`segmented win-seg`,"role":`radiogroup`,"aria-label":`Window`,...__sveltets_2_empty({"data-keep":true}),});
         for(let w of __sveltets_2_ensureArray(WINDOWS)){w;
         { svelteHTML.createElement("button", {         "role":`radio`,"onkeydown":radioKey,"aria-checked":window === w,"tabindex":window === w ? 0 : -1,"onclick":() => (window = w),});w; }
      }
     }
     { svelteHTML.createElement("button", {           "class":`icon-btn`,...__sveltets_2_empty({"data-icon":`refresh`}),...__sveltets_2_empty({"data-label":`Refresh fleet`}),"onclick":refresh,"title":`Refresh`,"aria-label":`Refresh fleet`,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":14,}});} }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {actions} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
  
 PageHeader}
 { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },}});
 { svelteHTML.createElement("div", { "class":`fleet`,});

  
   { svelteHTML.createElement("div", {   "class":`filters card`,...__sveltets_2_empty({"data-testid":`k8s-fleet-filters`}),});
     { svelteHTML.createElement("div", {     "class":`pills`,"role":`group`,"aria-label":`Clusters`,});
      if(clusterOpts.length === 0){
         { svelteHTML.createElement("span", { "class":`dim small`,});filtersError ? filtersError : 'No clusters have monitoring data in this window.'; }
      }
         for(let c of __sveltets_2_ensureArray(clusterOpts)){c.id;
         { svelteHTML.createElement("button", {             "class":`pill-toggle`,"onclick":() => toggleCluster(c.id),"title":c.rows === 0 ? `${c.name}: nothing collected in this window` : `${c.name}: ${c.rows.toLocaleString()} rows in ${window}`,"aria-pressed":clusters.includes(c.id),...__sveltets_2_empty({"data-testid":`k8s-fleet-cluster`}),});clusters.includes(c.id);c.rows === 0;
           { svelteHTML.createElement("span", {   "class":`dot`,"style":`background: ${c.color ?? 'var(--accent)'}`,}); }
          c.name;
           { const $$_egdaBvnE6C = __sveltets_2_ensureComponent(EnvBadge); new $$_egdaBvnE6C({ target: __sveltets_2_any(), props: {  "env":c.environment,}});}
         }
      }
     }
     { svelteHTML.createElement("div", { "class":`selects`,});
       { svelteHTML.createElement("select", {       "class":`input`,"bind:value":ns,"aria-label":`Namespace`,"onchange":() => { workload = ''; pod = ''; },});/*Ωignore_startΩ*/() => ns = __sveltets_2_any(null);/*Ωignore_endΩ*/
         { svelteHTML.createElement("option", {"value":"",});  }
           for(let n of __sveltets_2_ensureArray(nsOpts)){n; { svelteHTML.createElement("option", { "value":n,});n; }}
       }
       { svelteHTML.createElement("select", {       "class":`input`,"bind:value":workload,"aria-label":`Workload`,"onchange":() => (pod = ''),});/*Ωignore_startΩ*/() => workload = __sveltets_2_any(null);/*Ωignore_endΩ*/
         { svelteHTML.createElement("option", {"value":"",});  }
           for(let w of __sveltets_2_ensureArray(wlOpts)){w; { svelteHTML.createElement("option", { "value":w,});w; }}
       }
       { svelteHTML.createElement("select", {         "class":`input`,"bind:value":pod,"aria-label":`Pod`,"disabled":podOpts.length === 0 && !pod,"title":podOpts.length === 0 ? 'Pick a workload (or one cluster + namespace) to list pods' : '',});/*Ωignore_startΩ*/() => pod = __sveltets_2_any(null);/*Ωignore_endΩ*/
         { svelteHTML.createElement("option", {"value":"",});  }
           for(let p of __sveltets_2_ensureArray(podOpts)){p; { svelteHTML.createElement("option", { "value":p,});p; }}
       }
      if(hasFilter){
         { svelteHTML.createElement("button", {     "class":`btn small ghost`,"onclick":clearFilters,...__sveltets_2_empty({"data-testid":`k8s-fleet-clear`}),}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}  }
      }
     }
   }

   { svelteHTML.createElement("nav", {   "class":`tabs`,"aria-label":`Fleet sections`,});
       for(let t of __sveltets_2_ensureArray(TABS)){t.id;
       { svelteHTML.createElement("button", {        "class":`tab`,"onclick":() => goTab(t.id),"aria-current":activeTab === t.id ? 'page' : undefined,...__sveltets_2_empty({"data-testid":`k8s-fleet-tab-${t.id}`}),});activeTab === t.id;t.label; }
    }
   }

  if(activeTab === 'overview'){
    if(tableLoading && !rows.length){
       { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {    "rows":2,"height":60,}});}
    } else if (tableError){
       { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {      "what":`the fleet`,"error":tableError,"empty":true,"onretry":refresh,}});}
    }else{
       { svelteHTML.createElement("div", {   "class":`kpis`,...__sveltets_2_empty({"data-testid":`k8s-fleet-kpis`}),});
         { svelteHTML.createElement("div", { "class":`kpi`,}); { svelteHTML.createElement("span", { "class":`k`,});  } { svelteHTML.createElement("span", {  "class":`v mono`,});kpi.restarts > 0;kpi.restarts; } { svelteHTML.createElement("span", { "class":`d`,}); kpi.oom;   kpi.crash; } }
         { svelteHTML.createElement("div", { "class":`kpi`,}); { svelteHTML.createElement("span", { "class":`k`,});  } { svelteHTML.createElement("span", { "class":`v mono`,});kpi.churn; } { svelteHTML.createElement("span", { "class":`d`,});   } }
         { svelteHTML.createElement("div", { "class":`kpi`,}); { svelteHTML.createElement("span", { "class":`k`,});  } { svelteHTML.createElement("span", { "class":`v mono`,});formatBytes(kpi.mem); } { svelteHTML.createElement("span", { "class":`d`,});kpi.pods;   kpi.workloads;  } }
         { svelteHTML.createElement("div", { "class":`kpi`,}); { svelteHTML.createElement("span", { "class":`k`,});  } { svelteHTML.createElement("span", { "class":`v mono`,});fmtRate(kpi.rps); } { svelteHTML.createElement("span", {  "class":`d`,});kpi.errPct >= 1; fmtPct(kpi.errPct); } }
       }
    }
     { svelteHTML.createElement("div", { "class":`toolbar`,});
       { svelteHTML.createElement("span", { "class":`dim small`,});  }
       { svelteHTML.createElement("div", {     "class":`segmented`,"role":`radiogroup`,"aria-label":`Series by`,});
           for(let b of __sveltets_2_ensureArray(BYS)){b;
           { svelteHTML.createElement("button", {         "role":`radio`,"onkeydown":radioKey,"aria-checked":by === b,"tabindex":by === b ? 0 : -1,"onclick":() => (by = b),});b; }
        }
       }
       { svelteHTML.createElement("span", { "class":`dim small`,});     }
     }
    if(chartsLoading && !Object.keys(charts).length){
       { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {    "rows":3,"height":160,}});}
    } else if (chartsError){
       { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {      "what":`the charts`,"error":chartsError,"empty":true,"onretry":() => void loadSeries(),}});}
    }else{
       { svelteHTML.createElement("div", {   "class":`charts`,...__sveltets_2_empty({"data-testid":`k8s-fleet-charts`}),});
           for(let m of __sveltets_2_ensureArray(METRICS)){m.id;
           { svelteHTML.createElement("div", { "class":`card chart`,});
             { svelteHTML.createElement("div", { "class":`chart-head`,}); { svelteHTML.createElement("b", {});m.label; } { svelteHTML.createElement("span", { "class":`dim small`,});charts[m.id]?.step_secs ? `${charts[m.id]?.step_secs}s buckets` : ''; } }
             { const $$_trahCcirteM5C = __sveltets_2_ensureComponent(MetricChart); new $$_trahCcirteM5C({ target: __sveltets_2_any(), props: {          "series":chartData[m.id] ?? [],"unit":m.unit,"height":150,"area":m.id !== 'restarts',"emptyText":`No samples in this window`,}});}
           }
        }
       }
    }
  } else if (activeTab === 'table'){
     { svelteHTML.createElement("div", { "class":`toolbar`,});
       { svelteHTML.createElement("input", {          "dir":`ltr`,"class":`input`,"placeholder":`Quick filter…`,"bind:value":quick,"aria-label":`Quick filter`,});/*Ωignore_startΩ*/() => quick = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       { svelteHTML.createElement("div", {     "class":`segmented`,"role":`radiogroup`,"aria-label":`Group by`,});
         { svelteHTML.createElement("button", {         "role":`radio`,"onkeydown":radioKey,"aria-checked":group === 'workload',"tabindex":group === 'workload' ? 0 : -1,"onclick":() => (group = 'workload'),});  }
         { svelteHTML.createElement("button", {         "role":`radio`,"onkeydown":radioKey,"aria-checked":group === 'pod',"tabindex":group === 'pod' ? 0 : -1,"onclick":() => (group = 'pod'),});  }
       }
       { svelteHTML.createElement("span", { "class":`spacer`,}); }
       { svelteHTML.createElement("span", {   "class":`dim small`,...__sveltets_2_empty({"data-testid":`k8s-fleet-table-count`}),});total.toLocaleString(); group === 'pod' ? 'pods' : 'workloads'; }
     }
    if(tableLoading && !rows.length){
       { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {    "rows":8,"height":30,}});}
    } else if (tableError){
       { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {      "what":`the table`,"error":tableError,"empty":true,"onretry":() => void loadTable(),}});}
    } else if (!rows.length){
       { const $$_etatSytpmE3C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE3C({ target: __sveltets_2_any(), props: {      "icon":`clock`,"title":`No data in this window`,"body":`Nothing was collected for this selection. Widen the window, clear a filter, or enable monitoring on a cluster. Data comes straight from ClickHouse; filters, grouping and ordering are remembered.`,}});}
    }else{
       { svelteHTML.createElement("div", { "class":`tablewrap card`,});
         { const $$_div4 = svelteHTML.createElement("div", {          "class":`vt`,"role":`table`,"aria-label":group === 'pod' ? 'Fleet pods' : 'Fleet workloads',"aria-rowcount":visibleRows.length + 1,...__sveltets_2_empty({"data-testid":`k8s-fleet-table`}),});tableEl = $$_div4;
           { svelteHTML.createElement("div", { "role":`rowgroup`,});
             { svelteHTML.createElement("div", {       "class":`vt-head`,"role":`row`,"aria-rowindex":1,"style":`grid-template-columns:${tableCols}`,});
               { svelteHTML.createElement("div", {     "class":`vt-th`,"role":`columnheader`,"aria-sort":ariaSort(sort === 'cluster', dir),}); { svelteHTML.createElement("button", {    "class":`th-btn`,"onclick":() => sortBy('cluster'),});sort === 'cluster'; arrow(sort === 'cluster', dir); } }
               { svelteHTML.createElement("div", {     "class":`vt-th`,"role":`columnheader`,"aria-sort":ariaSort(sort === 'namespace', dir),}); { svelteHTML.createElement("button", {    "class":`th-btn`,"onclick":() => sortBy('namespace'),});sort === 'namespace'; arrow(sort === 'namespace', dir); } }
               { svelteHTML.createElement("div", {     "class":`vt-th`,"role":`columnheader`,"aria-sort":ariaSort(sort === 'workload', dir),}); { svelteHTML.createElement("button", {    "class":`th-btn`,"onclick":() => sortBy('workload'),});sort === 'workload'; arrow(sort === 'workload', dir); } }
              if(group === 'pod'){
                 { svelteHTML.createElement("div", {     "class":`vt-th`,"role":`columnheader`,"aria-sort":ariaSort(sort === 'pod', dir),}); { svelteHTML.createElement("button", {    "class":`th-btn`,"onclick":() => sortBy('pod'),});sort === 'pod'; arrow(sort === 'pod', dir); } }
              }else{
                 { svelteHTML.createElement("div", {     "class":`vt-th num`,"role":`columnheader`,"aria-sort":ariaSort(sort === 'pods', dir),}); { svelteHTML.createElement("button", {    "class":`th-btn`,"onclick":() => sortBy('pods'),});sort === 'pods'; arrow(sort === 'pods', dir); } }
              }
               { svelteHTML.createElement("div", {       "class":`vt-th num`,"role":`columnheader`,"aria-sort":ariaSort(sort === 'restarts', dir),"title":`Unplanned restarts in the window; OOM / crash / probe breakdown`,}); { svelteHTML.createElement("button", {    "class":`th-btn`,"onclick":() => sortBy('restarts'),});sort === 'restarts'; arrow(sort === 'restarts', dir); } }
               { svelteHTML.createElement("div", {     "class":`vt-th num`,"role":`columnheader`,"aria-sort":ariaSort(sort === 'oom', dir),}); { svelteHTML.createElement("button", {    "class":`th-btn`,"onclick":() => sortBy('oom'),});sort === 'oom'; arrow(sort === 'oom', dir); } }
               { svelteHTML.createElement("div", {       "class":`vt-th num`,"role":`columnheader`,"aria-sort":ariaSort(sort === 'churn', dir),"title":`Planned pod replacements`,}); { svelteHTML.createElement("button", {    "class":`th-btn`,"onclick":() => sortBy('churn'),});sort === 'churn'; arrow(sort === 'churn', dir); } }
               { svelteHTML.createElement("div", {       "class":`vt-th num`,"role":`columnheader`,"aria-sort":ariaSort(sort === 'mem_last', dir),"title":`Latest sample summed over pods`,}); { svelteHTML.createElement("button", {    "class":`th-btn`,"onclick":() => sortBy('mem_last'),});sort === 'mem_last'; arrow(sort === 'mem_last', dir); } }
               { svelteHTML.createElement("div", {       "class":`vt-th num`,"role":`columnheader`,"aria-sort":ariaSort(sort === 'mem_max', dir),"title":`Hungriest pod sample in the window`,}); { svelteHTML.createElement("button", {    "class":`th-btn`,"onclick":() => sortBy('mem_max'),});sort === 'mem_max'; arrow(sort === 'mem_max', dir); } }
               { svelteHTML.createElement("div", {     "class":`vt-th num`,"role":`columnheader`,"aria-sort":ariaSort(sort === 'rps', dir),}); { svelteHTML.createElement("button", {    "class":`th-btn`,"onclick":() => sortBy('rps'),});sort === 'rps'; arrow(sort === 'rps', dir); } }
               { svelteHTML.createElement("div", {     "class":`vt-th num`,"role":`columnheader`,"aria-sort":ariaSort(sort === 'err_pct', dir),}); { svelteHTML.createElement("button", {    "class":`th-btn`,"onclick":() => sortBy('err_pct'),});sort === 'err_pct'; arrow(sort === 'err_pct', dir); } }
               { svelteHTML.createElement("div", {     "class":`vt-th num`,"role":`columnheader`,"aria-sort":ariaSort(sort === 'latency_ms', dir),}); { svelteHTML.createElement("button", {    "class":`th-btn`,"onclick":() => sortBy('latency_ms'),});sort === 'latency_ms'; arrow(sort === 'latency_ms', dir); } }
             }
           }
           { svelteHTML.createElement("div", {   "role":`rowgroup`,"class":`vt-body`,});
             { const $$_tsiLlautriV6C = __sveltets_2_ensureComponent(VirtualList); const $$_tsiLlautriV6 = new $$_tsiLlautriV6C({ target: __sveltets_2_any(), props: {             "items":visibleRows,"estimateHeight":ROW_H,"class":`vt-list`,"key":fleetRowKey,"pinnedIndex":focusIdx,"scrollIndex":tableScrollIndex,"scrollVersion":tableScrollVersion,row:(r, i) => { async ()/*Ωignore_positionΩ*/ => {
                const rt = restartsTotal(r);
                 { svelteHTML.createElement("div", {                       "class":`vt-row wl-row`,"role":`row`,"aria-rowindex":i + 2,"tabindex":i === focusIdx ? 0 : -1,...__sveltets_2_empty({"data-i":i}),"style":`grid-template-columns:${tableCols};height:${ROW_H}px`,"onclick":() => drillRow(r),"onkeydown":(e) => rowKeydown(e, r, i),"onfocus":() => (tableFocus = i),"title":group === 'workload' ? "Show this workload's pods" : "Show this pod's events",...__sveltets_2_empty({"data-testid":`k8s-fleet-row`}),});
                   { svelteHTML.createElement("div", {   "class":`vt-td`,"role":`cell`,}); { svelteHTML.createElement("span", {   "class":`dot`,"style":`background: ${r.cluster.color ?? 'var(--accent)'}`,}); } r.cluster.name;  { const $$_egdaBvnE2C = __sveltets_2_ensureComponent(EnvBadge); new $$_egdaBvnE2C({ target: __sveltets_2_any(), props: {  "env":r.cluster.environment,}});} }
                   { svelteHTML.createElement("div", {     "class":`vt-td dim`,"role":`cell`,"title":r.namespace,});r.namespace; }
                   { svelteHTML.createElement("div", {     "class":`vt-td`,"role":`cell`,"title":r.workload,});if(group === 'workload'){ { svelteHTML.createElement("button", {       "class":`drill-btn`,"tabindex":-1,"aria-label":`Show pods for ${r.workload}`,"onclick":(e) => { e.stopPropagation(); drillRow(r); },});r.workload; }}else{ { svelteHTML.createElement("b", {});r.workload; }} }
                  if(group === 'pod'){ { svelteHTML.createElement("div", {     "class":`vt-td mono small`,"role":`cell`,"title":r.pod,}); { svelteHTML.createElement("button", {       "class":`drill-btn`,"tabindex":-1,"aria-label":`Show events for ${r.pod}`,"onclick":(e) => { e.stopPropagation(); drillRow(r); },});r.pod; } }}else{ { svelteHTML.createElement("div", {     "dir":`ltr`,"class":`vt-td num mono`,"role":`cell`,});r.pods; }}
                   { svelteHTML.createElement("div", {        "dir":`ltr`,"class":`vt-td num mono`,"role":`cell`,"title":rt > 0 ? restartBreakdown(r) : undefined,});rt > 0;
                    rt;if(rt > 0){ { svelteHTML.createElement("span", { "class":`dim small`,}); restartBreakdown(r);  }}
                   }
                   { svelteHTML.createElement("div", {      "dir":`ltr`,"class":`vt-td num mono`,"role":`cell`,});r.restarts.oom > 0;r.restarts.oom; }
                   { svelteHTML.createElement("div", {     "dir":`ltr`,"class":`vt-td num mono`,"role":`cell`,});r.churn; }
                   { svelteHTML.createElement("div", {     "dir":`ltr`,"class":`vt-td num mono`,"role":`cell`,});r.mem_last ? formatBytes(r.mem_last) : '—'; }
                   { svelteHTML.createElement("div", {     "dir":`ltr`,"class":`vt-td num mono`,"role":`cell`,});r.mem_max ? formatBytes(r.mem_max) : '—'; }
                   { svelteHTML.createElement("div", {     "dir":`ltr`,"class":`vt-td num mono`,"role":`cell`,});r.rps ? fmtRate(r.rps) : '—'; }
                   { svelteHTML.createElement("div", {       "dir":`ltr`,"class":`vt-td num mono`,"role":`cell`,});r.err_pct >= 5;r.err_pct >= 1 && r.err_pct < 5;r.rps ? fmtPct(r.err_pct) : '—'; }
                   { svelteHTML.createElement("div", {     "dir":`ltr`,"class":`vt-td num mono`,"role":`cell`,});r.latency_kind ? `${fmtMs(r.latency_ms)} ${r.latency_kind}` : '—'; }
                 }
              };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {row} = $$_tsiLlautriV6.$$prop_def;/*Ωignore_endΩ*/
              
             VirtualList}
           }
         }
        if(rows.length < total){
           { svelteHTML.createElement("div", { "class":`more`,}); { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => void loadTable(true, true),}); Math.min(PAGE, total - rows.length);  } }
        }
       }
    }
  } else if (activeTab === 'events'){
     { svelteHTML.createElement("div", { "class":`toolbar`,});
       { svelteHTML.createElement("select", {     "class":`input`,"bind:value":evClass,"aria-label":`Event class`,});/*Ωignore_startΩ*/() => evClass = __sveltets_2_any(null);/*Ωignore_endΩ*/
           for(let c of __sveltets_2_ensureArray(CLASS_OPTIONS)){c;
           { svelteHTML.createElement("option", { "value":c,});c === '' ? 'Restarts + churn' : c === 'churn' ? 'Churn only' : c === 'version' ? 'New versions' : c === 'k8s_event' ? 'Raw cluster events' : classLabel(c); }
        }
       }
       { svelteHTML.createElement("span", { "class":`spacer`,}); }
       { svelteHTML.createElement("span", {   "class":`dim small`,...__sveltets_2_empty({"data-testid":`k8s-fleet-events-count`}),});evTotal.toLocaleString();  }
     }
    if(evLoading && !events.length){
       { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {    "rows":8,"height":28,}});}
    } else if (evError){
       { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {      "what":`events`,"error":evError,"empty":true,"onretry":() => void loadEvents(),}});}
    } else if (!events.length){
       { const $$_etatSytpmE3C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE3C({ target: __sveltets_2_any(), props: {      "icon":`check`,"title":`Nothing in this window`,"body":`No restarts or pod replacements were recorded for this selection.`,}});}
    }else{
       { svelteHTML.createElement("div", { "class":`tablewrap card`,});
         { svelteHTML.createElement("div", {         "class":`vt ev`,"role":`table`,"aria-label":`Fleet events`,"aria-rowcount":events.length + 1,...__sveltets_2_empty({"data-testid":`k8s-fleet-events`}),});
           { svelteHTML.createElement("div", { "role":`rowgroup`,});
             { svelteHTML.createElement("div", {       "class":`vt-head`,"role":`row`,"aria-rowindex":1,"style":`grid-template-columns:${EV_COLS}`,});
               { svelteHTML.createElement("div", {     "class":`vt-th`,"role":`columnheader`,"aria-sort":ariaSort(evSort === 'ts', evDir),}); { svelteHTML.createElement("button", {    "class":`th-btn`,"onclick":() => evSortBy('ts'),});evSort === 'ts'; arrow(evSort === 'ts', evDir); } }
               { svelteHTML.createElement("div", {     "class":`vt-th`,"role":`columnheader`,"aria-sort":ariaSort(evSort === 'cluster', evDir),}); { svelteHTML.createElement("button", {    "class":`th-btn`,"onclick":() => evSortBy('cluster'),});evSort === 'cluster'; arrow(evSort === 'cluster', evDir); } }
               { svelteHTML.createElement("div", {     "class":`vt-th`,"role":`columnheader`,"aria-sort":ariaSort(evSort === 'namespace', evDir),}); { svelteHTML.createElement("button", {    "class":`th-btn`,"onclick":() => evSortBy('namespace'),});evSort === 'namespace'; arrow(evSort === 'namespace', evDir); } }
               { svelteHTML.createElement("div", {     "class":`vt-th`,"role":`columnheader`,"aria-sort":ariaSort(evSort === 'workload', evDir),}); { svelteHTML.createElement("button", {    "class":`th-btn`,"onclick":() => evSortBy('workload'),});evSort === 'workload'; arrow(evSort === 'workload', evDir); } }
               { svelteHTML.createElement("div", {     "class":`vt-th`,"role":`columnheader`,"aria-sort":ariaSort(evSort === 'pod', evDir),}); { svelteHTML.createElement("button", {    "class":`th-btn`,"onclick":() => evSortBy('pod'),});evSort === 'pod'; arrow(evSort === 'pod', evDir); } }
               { svelteHTML.createElement("div", {     "class":`vt-th`,"role":`columnheader`,"aria-sort":ariaSort(evSort === 'class', evDir),}); { svelteHTML.createElement("button", {    "class":`th-btn`,"onclick":() => evSortBy('class'),});evSort === 'class'; arrow(evSort === 'class', evDir); } }
               { svelteHTML.createElement("div", {     "class":`vt-th`,"role":`columnheader`,"aria-sort":ariaSort(evSort === 'reason', evDir),}); { svelteHTML.createElement("button", {    "class":`th-btn`,"onclick":() => evSortBy('reason'),});evSort === 'reason'; arrow(evSort === 'reason', evDir); } }
             }
           }
           { svelteHTML.createElement("div", {   "role":`rowgroup`,"class":`vt-body`,});
             { const $$_tsiLlautriV6C = __sveltets_2_ensureComponent(VirtualList); const $$_tsiLlautriV6 = new $$_tsiLlautriV6C({ target: __sveltets_2_any(), props: {       "items":events,"estimateHeight":ROW_H,"class":`vt-list`,"key":(_e, i) => eventKeys[i],row:(e, i) => { async ()/*Ωignore_positionΩ*/ => {
                const msg = eventMsg(e);
                 { svelteHTML.createElement("div", {         "class":`vt-row`,"role":`row`,"aria-rowindex":i + 2,"style":`grid-template-columns:${EV_COLS};height:${ROW_H}px`,...__sveltets_2_empty({"data-testid":`k8s-fleet-event-row`}),});
                   { svelteHTML.createElement("div", {   "class":`vt-td mono small`,"role":`cell`,});fmtTs(e.ts); }
                   { svelteHTML.createElement("div", {     "class":`vt-td`,"role":`cell`,"title":e.cluster.name,}); { svelteHTML.createElement("span", {   "class":`dot`,"style":`background: ${e.cluster.color ?? 'var(--accent)'}`,}); } e.cluster.name; }
                   { svelteHTML.createElement("div", {     "class":`vt-td dim`,"role":`cell`,"title":e.namespace,});e.namespace; }
                   { svelteHTML.createElement("div", {     "class":`vt-td`,"role":`cell`,"title":e.workload,}); { svelteHTML.createElement("b", {});e.workload; } }
                   { svelteHTML.createElement("div", {     "class":`vt-td mono small`,"role":`cell`,"title":e.pod,});e.pod; }
                   { svelteHTML.createElement("div", {   "class":`vt-td`,"role":`cell`,}); { svelteHTML.createElement("span", {   "class":`tclass`,"style":`color: ${e.kind === 'version' ? 'var(--status-working)' : classColor(e.class)}`,});e.kind === 'k8s_event' ? e.reason : e.kind === 'version' ? 'New version' : e.kind === 'churn' ? `churn · ${classLabel(e.class)}` : classLabel(e.class); } }
                   { svelteHTML.createElement("div", {     "class":`vt-td dim`,"role":`cell`,"title":msg,});msg; }
                 }
              };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {row} = $$_tsiLlautriV6.$$prop_def;/*Ωignore_endΩ*/
              
             VirtualList}
           }
         }
        if(events.length < evTotal){
           { svelteHTML.createElement("div", { "class":`more`,}); { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => void loadEvents(true, true),}); Math.min(PAGE, evTotal - events.length);  } }
        }
       }
    }
  }else{
    if(reqLoading && !reqs){
       { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {    "rows":6,"height":28,}});}
    } else if (reqError){
       { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {      "what":`requests`,"error":reqError,"empty":true,"onretry":() => void loadRequests(),}});}
    } else if (reqs){
      if(reqs.enabled_on.length === 0){
         { svelteHTML.createElement("div", {   "class":`note card`,...__sveltets_2_empty({"data-testid":`k8s-fleet-requests-off`}),});
           { svelteHTML.createElement("b", {});       }
           { svelteHTML.createElement("span", { "class":`dim`,});    { svelteHTML.createElement("em", {});    }                        }
           { svelteHTML.createElement("span", { "class":`links`,});
               for(let c of __sveltets_2_ensureArray(reqs.disabled_on)){c.id;
               { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => router.go(`kubernetes/${encodeURIComponent(c.id)}/monitor/settings`),});c.name;  }
            }
           }
         }
      } else if (reqs.disabled_on.length > 0){
         { svelteHTML.createElement("div", { "class":`dim small`,});     reqs.enabled_on.map((c) => c.name).join(', ');   reqs.disabled_on.map((c) => c.name).join(', ');  }
      }
      if(!reqRows.length){
        if(reqs.enabled_on.length > 0){
           { const $$_etatSytpmE3C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE3C({ target: __sveltets_2_any(), props: {      "icon":`clock`,"title":`No per-route samples yet`,"body":`Rows appear after the next collection cycle on a cluster with request labels enabled.`,}});}
        }
      }else{
         { svelteHTML.createElement("div", { "class":`tablewrap card`,});
           { svelteHTML.createElement("table", {   "class":`wl`,...__sveltets_2_empty({"data-testid":`k8s-fleet-requests`}),});
             { svelteHTML.createElement("thead", {});
               { svelteHTML.createElement("tr", {});
                 { svelteHTML.createElement("th", {}); { svelteHTML.createElement("button", {    "class":`th-btn`,"onclick":() => reqSortBy('path'),});reqSort === 'path'; arrow(reqSort === 'path', reqDir); } }
                 { svelteHTML.createElement("th", {});  }
                 { svelteHTML.createElement("th", { "class":`num`,}); { svelteHTML.createElement("button", {    "class":`th-btn`,"onclick":() => reqSortBy('rps'),});reqSort === 'rps'; arrow(reqSort === 'rps', reqDir); } }
                 { svelteHTML.createElement("th", { "class":`num`,}); { svelteHTML.createElement("button", {    "class":`th-btn`,"onclick":() => reqSortBy('err_pct'),});reqSort === 'err_pct'; arrow(reqSort === 'err_pct', reqDir); } }
                 { svelteHTML.createElement("th", { "class":`num`,}); { svelteHTML.createElement("button", {    "class":`th-btn`,"onclick":() => reqSortBy('avg_ms'),});reqSort === 'avg_ms'; arrow(reqSort === 'avg_ms', reqDir); } }
               }
             }
             { svelteHTML.createElement("tbody", {});
                 for(let r of __sveltets_2_ensureArray(reqRows)){`${r.method} ${r.path}`;
                 { svelteHTML.createElement("tr", {});
                   { svelteHTML.createElement("td", {   "class":`mono`,"dir":`ltr`,});r.path; }
                   { svelteHTML.createElement("td", {   "class":`mono small dim`,"dir":`ltr`,});r.method || '—'; }
                   { svelteHTML.createElement("td", {   "dir":`ltr`,"class":`num mono`,});fmtRate(r.rps); }
                   { svelteHTML.createElement("td", {     "dir":`ltr`,"class":`num mono`,});r.err_pct >= 5;r.err_pct >= 1 && r.err_pct < 5;fmtPct(r.err_pct); }
                   { svelteHTML.createElement("td", {   "dir":`ltr`,"class":`num mono`,});r.avg_ms ? fmtMs(r.avg_ms) : '—'; }
                 }
              }
             }
           }
         }
      }
    }
  }
 }
 PageBody}
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const MonitorFleet__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type MonitorFleet__SvelteComponent_ = ReturnType<typeof MonitorFleet__SvelteComponent_>;
/*Ωignore_endΩ*/export default MonitorFleet__SvelteComponent_;