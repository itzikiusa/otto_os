///<reference types="svelte" />
;
import { announceShown } from '../../../lib/announce.svelte';
import { rel } from '../../../lib/stores/now.svelte';
import { rowMenu as rowMenuKeys } from '../../../lib/rowMenu';
import { plural } from '../../../lib/plural';
import { loadErrorText } from '../../../lib/loadError';
import { radioKey } from '../../../lib/radioKey';
// Per-cluster Monitor view — the Monitor half of the cluster workspace
// (Resources | Monitor switch). Tabs live in the URL
// (`#/kubernetes/<id>/monitor/<workloads|events|insights|settings>`):
// Workloads = sortable table with sparklines + an expandable Trends row;
// Events = classified restart / churn timeline; Insights = the watchdog
// agent's latest report; Settings = the probe configuration. Re-fetches on
// WS `k8s_monitor_cycle` for this cluster. Namespace, filter, sort, the
// expanded row, the events filter and the scroll offset live in the k8s
// store per cluster (persisted), so the view survives module switches,
// Resources ↔ Monitor and reloads; picking a namespace here also sets it in Resources.
// Cross-links: workload → its pods, a pod row → its drawer (Metrics), an
// event → the pod drawer's Events tab.
import { untrack, onDestroy, tick } from 'svelte';
import { router } from '../../../lib/router.svelte';
import { k8s } from '../../../lib/stores/k8s.svelte';
import { viewport } from '../../../lib/stores/viewport.svelte';
import { auth } from '../../../lib/stores/auth.svelte';
import { ctxMenu } from '../../../lib/contextmenu.svelte';
import { k8sApi } from '../../../lib/api/k8s';
import type { K8sCluster, K8sMonitorEvent, K8sMonitorSeries, K8sMonitorStatus, K8sMonitorWorkloadRow } from '../../../lib/api/types';
import Icon from '../../../lib/components/Icon.svelte';
import PageHeader from '../../../lib/components/PageHeader.svelte';
import PageBody from '../../../lib/components/PageBody.svelte';
import EmptyState from '../../../lib/components/EmptyState.svelte';
import LoadState from '../../../lib/components/LoadState.svelte';
import Skeleton from '../../../lib/components/Skeleton.svelte';
import { formatAge, formatBytes } from '../k8s-util';
import EnvBadge from '../../../lib/components/EnvBadge.svelte';
import { envTone } from '../../../lib/status';
import Sparkline from './Sparkline.svelte';
import MonitorSettings from './MonitorSettings.svelte';
import LazyMount from '../../../lib/components/LazyMount.svelte';
import { lazyComponent } from '../../../lib/lazy-component.svelte';
import { tableWindow, WINDOW_MIN_ROWS } from './tableWindow';
import K8sViewSwitch from '../K8sViewSwitch.svelte';
import { monitorPath, resourcesPath } from '../../../lib/k8s/viewState';
import type { K8sDrawerTab } from '../../../lib/stores/k8s.svelte';
import { WINDOWS, classColor, classLabel, collectorLine, fmtMs, fmtPct, fmtRate, isWindow, rbacMessage, credentialsRejected } from './monitor-util';
;

  interface Props { cluster: K8sCluster; tab: string };function $$render() {
  const windowPicker/*Ωignore_positionΩ*/ = ()/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("div", {     "class":`segmented`,"role":`radiogroup`,"aria-label":`Window`,});
           for(let w of __sveltets_2_ensureArray(WINDOWS)){w;
           { svelteHTML.createElement("button", {         "role":`radio`,"onkeydown":radioKey,"aria-checked":window === w,"tabindex":window === w ? 0 : -1,"onclick":() => (window = w),});w; }
        }
       }
};return __sveltets_2_any(0)};
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  // The Insights tab renders markdown (`marked`): load it on first open.
  const MonitorInsightsLazy = lazyComponent(() => import('./MonitorInsights.svelte'));

  let { cluster, tab }: Props = $props();

  const TABS = [
    { id: 'workloads', label: 'Workloads' },
    { id: 'events', label: 'Events' },
    { id: 'insights', label: 'Insights' },
    { id: 'settings', label: 'Settings' },
  ];
  const activeTab = $derived(TABS.some((t) => t.id === tab) ? tab : 'workloads');
  const canEdit = $derived(auth.can('kubernetes', 'edit'));

  let window = $state<(typeof WINDOWS)[number]>('1h');
  try {
    const saved = localStorage.getItem('otto_k8s_monitor_cluster_window');
    if (isWindow(saved ?? undefined)) window = saved as (typeof WINDOWS)[number];
  } catch {
    /* ignore */
  }

  // --- workloads ---------------------------------------------------------------
  let rows = $state.raw<K8sMonitorWorkloadRow[]>([]); // replaced per poll, never mutated
  let status = $state<K8sMonitorStatus | null>(null);
  let enabled = $state(true);
  let allNamespaces = $state<string[]>([]);
  let loading = $state(true);
  let error = $state('');
  // Remembered view (K-1): seeded from the store, written back as it moves.
  const saved = untrack(() => k8s.monitorUi(cluster.id));
  const SORT_KEYS = ['workload', 'pods', 'mem_max', 'mem_pct', 'restarts_total', 'churn_planned', 'rps', 'err_pct', 'latency_ms'];
  // Not seeded from the console namespace: the Monitor lists only its
  // configured namespaces, and the console's (often `default`) may not be one.
  let ns = $state(saved.ns);
  let filter = $state(saved.filter);
  let sortKey = $state<keyof K8sMonitorWorkloadRow | 'restarts_total'>(
    (SORT_KEYS.includes(saved.sortKey) ? saved.sortKey : 'mem_max') as keyof K8sMonitorWorkloadRow | 'restarts_total',
  );
  let sortDir = $state<1 | -1>(saved.sortDir);
  let expanded = $state<string | null>(saved.expanded);
  /** The restored expanded row still needs its trends once rows arrive. */
  let trendsPending = saved.expanded !== null;
  let scrollRestored = false;
  let firstReset = true;
  let monEl = $state<HTMLDivElement | null>(null);
  let series = $state<{ mem: K8sMonitorSeries | null; rps: K8sMonitorSeries | null; err: K8sMonitorSeries | null }>({ mem: null, rps: null, err: null });
  let seriesLoading = $state(false);
  let seriesError = $state('');
  let seriesRequest = 0;
  let eventsRequest = 0;
  let abort: AbortController | null = null;

  function goTab(id: string): void {
    router.go(monitorPath(cluster.id, id));
  }
  /** Monitor → Resources: a workload's pods (namespace + filter set). */
  function openPods(r: K8sMonitorWorkloadRow): void {
    if (r.namespace && k8s.clusterId === cluster.id && k8s.namespace !== '') k8s.setNamespace(r.namespace);
    k8s.presetFilter(cluster.id, 'pods', r.workload);
    router.go(resourcesPath(cluster.id, 'pods'));
  }
  /** Monitor → a pod's drawer on the given tab. */
  function openPod(podNs: string, pod: string, tab: K8sDrawerTab): void {
    k8s.drawerTab = tab;
    if (podNs && k8s.clusterId === cluster.id && k8s.namespace !== '' && k8s.namespace !== podNs) k8s.setNamespace(podNs);
    router.go(resourcesPath(cluster.id, 'pods', podNs, pod));
  }
  function rowMenu(e: MouseEvent, r: K8sMonitorWorkloadRow): void {
    e.preventDefault();
    ctxMenu.show(e, [
      { label: 'Open pods', icon: 'box', action: () => openPods(r) },
      { label: 'Show trends', icon: 'chart', action: () => void toggle(r) },
    ]);
  }

  // Persist the view + share the namespace with Resources.
  $effect(() => {
    const snap = { ns, filter, sortKey: String(sortKey), sortDir, expanded, classFilter };
    untrack(() => {
      k8s.saveMonitorUi(cluster.id, snap);
      if (snap.ns && k8s.clusterId === cluster.id && k8s.namespace !== snap.ns) k8s.setNamespace(snap.ns);
    });
  });
  function scrollHost(): HTMLElement | null {
    return monEl?.closest<HTMLElement>('.page-body') ?? null;
  }
  function restoreScroll(): void {
    if (scrollRestored) return;
    scrollRestored = true;
    const top = untrack(() => k8s.monitorUi(cluster.id).scrollTop);
    const host = scrollHost();
    if (host && top > 0) requestAnimationFrame(() => (host.scrollTop = top));
  }
  $effect(() => {
    const host = scrollHost();
    if (!host) return;
    let t: ReturnType<typeof setTimeout> | null = null;
    const onScroll = (): void => {
      if (t) clearTimeout(t);
      t = setTimeout(() => k8s.saveMonitorUi(cluster.id, { scrollTop: Math.round(host.scrollTop) }), 200);
    };
    host.addEventListener('scroll', onScroll, { passive: true });
    return () => {
      host.removeEventListener('scroll', onScroll);
      if (t) clearTimeout(t);
    };
  });
  function clusterMenu(e: MouseEvent): void {
    ctxMenu.show(
      e,
      k8s.clusters.map((c) => ({
        label: `${c.name} · ${envTone(c.environment).label}`,
        icon: 'helm',
        disabled: c.id === cluster.id,
        action: () => router.go(monitorPath(c.id, activeTab)),
      })),
    );
  }

  async function loadWorkloads(quiet = false): Promise<void> {
    abort?.abort();
    abort = new AbortController();
    if (!quiet) loading = true;
    try {
      const r = await k8sApi.monitorWorkloads(cluster.id, window, ns, abort.signal);
      rows = r.workloads;
      status = r.status;
      enabled = r.enabled;
      allNamespaces = r.namespaces ?? [];
      error = '';
      if (trendsPending) {
        trendsPending = false;
        const row = expanded ? r.workloads.find((w) => `${w.namespace}/${w.workload}` === expanded) : undefined;
        if (row) void loadTrends(row);
        else expanded = null;
      }
      restoreScroll();
    } catch (e) {
      if (e instanceof DOMException && e.name === 'AbortError') return;
      error = loadErrorText(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    const w = window;
    const n = ns;
    const t = activeTab;
    const id = cluster.id;
    void n;
    void id;
    try {
      localStorage.setItem('otto_k8s_monitor_cluster_window', w);
    } catch {
      /* ignore */
    }
    // Events load from their own effect below (class filter + window) — one
    // fetch per change, not two.
    if (t === 'workloads') untrack(() => void loadWorkloads());
    untrack(() => {
      // The first run is the restored view — keep its expanded row.
      if (firstReset) {
        firstReset = false;
        return;
      }
      // A different cluster, namespace, window or section owns fresh detail.
      seriesRequest++;
      expanded = null;
      seriesLoading = false;
      seriesError = '';
      if (t !== 'events') eventsRequest++;
    });
  });

  // Coalesced by the store (≤ 1 per MONITOR_TICK_MIN_MS, none while
  // hidden); only a NEW tick that included this cluster re-fetches.
  let seenTick = untrack(() => k8s.monitorTick);
  $effect(() => {
    const tick = k8s.monitorTick;
    if (tick !== seenTick && k8s.monitorTicked(cluster.id)) {
      seenTick = tick;
      untrack(() => {
        if (activeTab === 'workloads') void loadWorkloads(true);
        if (activeTab === 'events') void loadEvents(true);
      });
    }
  });

  // Picker options come from the daemon's unfiltered snapshot, never from the
  // (possibly namespace-filtered) rows — otherwise picking one namespace made
  // every other option vanish.
  const namespaces = $derived(allNamespaces.length ? allNamespaces : [...new Set(rows.map((r) => r.namespace))].sort());
  function restartsTotal(r: K8sMonitorWorkloadRow): number {
    return r.restarts.oom + r.restarts.crash + r.restarts.probe + r.restarts.unknown;
  }
  const visible = $derived.by(() => {
    const q = filter.trim().toLowerCase();
    const list = rows.filter((r) => !q || r.workload.toLowerCase().includes(q) || r.namespace.toLowerCase().includes(q));
    const key = sortKey;
    const dir = sortDir;
    return [...list].sort((a, b) => {
      const av = key === 'restarts_total' ? restartsTotal(a) : (a[key] as number | string);
      const bv = key === 'restarts_total' ? restartsTotal(b) : (b[key] as number | string);
      if (typeof av === 'string' && typeof bv === 'string') return dir * av.localeCompare(bv);
      return dir * ((av as number) - (bv as number));
    });
  });
  function sortBy(key: typeof sortKey): void {
    if (sortKey === key) sortDir = sortDir === 1 ? -1 : 1;
    else {
      sortKey = key;
      sortDir = key === 'workload' ? 1 : -1;
    }
  }

  async function toggle(r: K8sMonitorWorkloadRow): Promise<void> {
    const key = `${r.namespace}/${r.workload}`;
    if (expanded === key) {
      expanded = null;
      seriesRequest++;
      return;
    }
    expanded = key;
    await loadTrends(r);
  }

  let seriesAbort: AbortController | null = null;
  async function loadTrends(r: K8sMonitorWorkloadRow): Promise<void> {
    const request = ++seriesRequest;
    seriesAbort?.abort();
    const ctrl = (seriesAbort = new AbortController());
    seriesLoading = true;
    seriesError = '';
    series = { mem: null, rps: null, err: null };
    try {
      const memMetric = status?.metrics_server === 'ok' ? 'mem_working_set_bytes' : 'mem_sys_bytes';
      const [mem, rps] = await Promise.all([
        k8sApi.monitorSeries(cluster.id, { metric: memMetric, workload: r.workload, ns: r.namespace || undefined, window }, ctrl.signal),
        k8sApi.monitorSeries(cluster.id, { metric: 'http_requests_total', workload: r.workload, ns: r.namespace || undefined, window }, ctrl.signal),
      ]);
      if (request === seriesRequest) series = { mem, rps, err: null };
    } catch (e) {
      if (request === seriesRequest && !ctrl.signal.aborted) seriesError = loadErrorText(e);
    } finally {
      if (request === seriesRequest) seriesLoading = false;
    }
  }

  // --- events --------------------------------------------------------------------
  let events = $state.raw<K8sMonitorEvent[]>([]); // replaced per load, never mutated
  let eventsLoading = $state(false);
  let eventsError = $state('');
  let classFilter = $state(saved.classFilter);
  const CLASS_OPTIONS = ['', 'oom', 'crash', 'probe', 'unknown', 'planned', 'completed', 'version', 'k8s_event'];

  let eventsAbort: AbortController | null = null;
  async function loadEvents(quiet = false): Promise<void> {
    const request = ++eventsRequest;
    eventsAbort?.abort();
    const ctrl = (eventsAbort = new AbortController());
    if (!quiet) eventsLoading = true;
    try {
      const next = await k8sApi.monitorEvents(cluster.id, { window, class: classFilter || undefined, ns: ns || undefined, limit: 300 }, ctrl.signal);
      if (request !== eventsRequest) return;
      events = next;
      eventsError = '';
      restoreScroll();
    } catch (e) {
      if (request !== eventsRequest || ctrl.signal.aborted) return;
      eventsError = loadErrorText(e);
    } finally {
      if (request === eventsRequest) eventsLoading = false;
    }
  }
  $effect(() => {
    const c = classFilter;
    void c;
    void window;
    void ns;
    void cluster.id;
    if (activeTab === 'events') untrack(() => void loadEvents());
  });

  onDestroy(() => {
    abort?.abort();
    eventsAbort?.abort();
    seriesAbort?.abort();
    eventsRequest++;
    seriesRequest++;
  });

  // --- workloads table windowing (perf K8s R5) ------------------------------------
  // Only the rows near the viewport render (two SVG sparklines each); the
  // page body is the scroll host. Small tables render whole.
  let tbodyEl = $state<HTMLTableSectionElement | null>(null);
  let viewTop = $state(0);
  let viewH = $state(800);
  let rowH = $state(52);
  let detailH = $state(0);
  const expandedIdx = $derived(expanded ? visible.findIndex((r) => `${r.namespace}/${r.workload}` === expanded) : -1);
  const win = $derived(visible.length > WINDOW_MIN_ROWS ? tableWindow({ count: visible.length, rowH, viewTop, viewH, expanded: expandedIdx, detailH }) : null);
  const shown = $derived(win ? visible.slice(win.start, win.end) : visible);
  function measure(): void {
    const host = scrollHost();
    if (!host || !tbodyEl) return;
    viewTop = host.getBoundingClientRect().top - tbodyEl.getBoundingClientRect().top;
    viewH = host.clientHeight || 800;
    const rowEls = tbodyEl.querySelectorAll<HTMLElement>(':scope > tr.wl-row');
    if (rowEls.length) {
      let sum = 0;
      rowEls.forEach((el) => (sum += el.offsetHeight));
      const avg = Math.round(sum / rowEls.length);
      if (avg > 0 && Math.abs(avg - rowH) > 2) rowH = avg;
    }
    const d = tbodyEl.querySelector<HTMLElement>(':scope > tr.detail');
    if (d) detailH = d.offsetHeight;
  }
  $effect(() => {
    const host = scrollHost();
    if (!host || !tbodyEl) return;
    let raf = 0;
    // `win` is read untracked: it changes every scrolled frame, and tracking it
    // here tore down and re-added both listeners (plus queued an extra measure)
    // per frame. The effect depends only on the host/tbody elements.
    const onMove = (): void => {
      if (raf || !untrack(() => win)) return;
      raf = requestAnimationFrame(() => {
        raf = 0;
        measure();
      });
    };
    host.addEventListener('scroll', onMove, { passive: true });
    globalThis.addEventListener('resize', onMove);
    onMove();
    return () => {
      host.removeEventListener('scroll', onMove);
      globalThis.removeEventListener('resize', onMove);
      if (raf) cancelAnimationFrame(raf);
    };
  });
  // Re-measure once the rendered slice / expanded detail changes. Keyed on the
  // slice BOUNDS (a string, so equal bounds don't re-fire): `shown` is a fresh
  // array on every `win` change, which re-measured on every scrolled frame.
  const sliceKey = $derived(win ? `${win.start}:${win.end}` : 'all');
  $effect(() => {
    void sliceKey;
    void expanded;
    void seriesLoading;
    if (untrack(() => win)) void tick().then(() => untrack(measure));
  });

  function fmtTs(ts: string): string {
    const d = new Date(ts.replace(' ', 'T') + (ts.endsWith('Z') ? '' : 'Z'));
    return Number.isNaN(d.getTime()) ? ts : d.toLocaleString();
  }
  function plannedBy(e: K8sMonitorEvent): string {
    const by = (e.detail?.planned_by as string | undefined) ?? '';
    return by ? ` · ${by}` : '';
  }
  function eventMsg(e: K8sMonitorEvent): string {
    if (e.kind === 'k8s_event') return (e.detail?.message as string | undefined) ?? '';
    if (e.kind === 'version') {
      const pods = e.detail?.next_restarts as number | undefined;
      return `new version ${e.reason}${pods ? ` · ${plural(pods, 'pod')} on it` : ''}`;
    }
    if (e.kind === 'restart') {
      const p = e.detail?.prev_restarts as number | undefined;
      const n = e.detail?.next_restarts as number | undefined;
      return `${e.container ? e.container + ' ' : ''}restart ${p ?? '?'}→${n ?? '?'}${e.reason ? ` · ${e.reason}` : ''}${e.exit_code ? ` · exit ${e.exit_code}` : ''}`;
    }
    return `pod replaced${plannedBy(e)}${e.reason ? ` · ${e.reason}` : ''}`;
  }
;
async () => {

 { svelteHTML.createElement("div", {   "class":`mon-page`,...__sveltets_2_empty({"data-testid":`k8s-monitor-cluster`}),});
  { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {        "title":cluster.name,"crumbs":[
    { label: 'Kubernetes', onclick: () => router.go('kubernetes') },
    { label: 'All clusters (Monitor)', onclick: () => router.go('kubernetes/monitor') },
  ],"subtitle":collectorLine(status, enabled, rel),"tabsPlacement":`below`,titleContent:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("button", {       "class":`cluster-pick`,"onclick":clusterMenu,"title":`Switch cluster`,...__sveltets_2_empty({"data-testid":`k8s-monitor-cluster-pick`}),});
       { svelteHTML.createElement("span", {   "class":`dot`,"style":`background: ${cluster.color ?? 'var(--accent)'}`,}); }
      cluster.name;
       { const $$_egdaBvnE1C = __sveltets_2_ensureComponent(EnvBadge); new $$_egdaBvnE1C({ target: __sveltets_2_any(), props: {  "env":cluster.environment,}});}
       { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`chevronDown`,"size":12,}});}
     }
  };return __sveltets_2_any(0)},badge:() => { async ()/*Ωignore_positionΩ*/ => {
     { const $$_hctiwSweiVs8K0C = __sveltets_2_ensureComponent(K8sViewSwitch); new $$_hctiwSweiVs8K0C({ target: __sveltets_2_any(), props: {    "clusterId":cluster.id,"view":`monitor`,}});}
  };return __sveltets_2_any(0)},actions:() => { async ()/*Ωignore_positionΩ*/ => {
    if(!viewport.isPhone && (activeTab === 'workloads' || activeTab === 'events')){
      ;__sveltets_2_ensureSnippet(windowPicker());
    }
  };return __sveltets_2_any(0)},tabs:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("nav", {   "class":`tabs`,"aria-label":`Monitor sections`,});
         for(let t of __sveltets_2_ensureArray(TABS)){t.id;
         { svelteHTML.createElement("button", {        "class":`tab`,"onclick":() => goTab(t.id),"aria-current":activeTab === t.id ? 'page' : undefined,...__sveltets_2_empty({"data-testid":`k8s-monitor-tab-${t.id}`}),});activeTab === t.id;t.label; }
      }
     }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {titleContent, badge, actions, tabs} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
  
  
  
  
 PageHeader}
 { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },}});
 { const $$_div2 = svelteHTML.createElement("div", {  "class":`mon`,});monEl = $$_div2;
  if(viewport.isPhone && (activeTab === 'workloads' || activeTab === 'events')){ { svelteHTML.createElement("div", { "class":`window-bar`,});;__sveltets_2_ensureSnippet(windowPicker()); }}

  if(activeTab === 'settings'){
     { const $$_sgnitteSrotinoM3C = __sveltets_2_ensureComponent(MonitorSettings); new $$_sgnitteSrotinoM3C({ target: __sveltets_2_any(), props: {    cluster,canEdit,"onsaved":(c, s) => { enabled = c.enabled; status = s; },}});}
  } else if (activeTab === 'insights'){
     { const $$_tnuoMyzaL3C = __sveltets_2_ensureComponent(LazyMount); new $$_tnuoMyzaL3C({ target: __sveltets_2_any(), props: {      "lazy":MonitorInsightsLazy,"what":`the insights report`,"variant":`panel`,}});}
  } else if (activeTab === 'events'){
     { svelteHTML.createElement("div", { "class":`toolbar`,});
       { svelteHTML.createElement("select", {     "class":`input`,"bind:value":classFilter,"aria-label":`Event class`,});/*Ωignore_startΩ*/() => classFilter = __sveltets_2_any(null);/*Ωignore_endΩ*/
           for(let c of __sveltets_2_ensureArray(CLASS_OPTIONS)){c;
           { svelteHTML.createElement("option", { "value":c,});c === '' ? 'Restarts + churn' : c === 'k8s_event' ? 'Raw cluster events' : classLabel(c); }
        }
       }
       { svelteHTML.createElement("button", {       "class":`btn ghost small`,"onclick":() => void loadEvents(),"aria-label":`Refresh events`,"title":`Refresh events`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":13,}});} }
     }
    if(eventsLoading && !events.length){
       { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {    "rows":6,"height":28,}});}
    } else if (eventsError){
       { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {      "what":`events`,"error":eventsError,"empty":true,"onretry":() => void loadEvents(),}});}
    } else if (!events.length){
       { const $$_etatSytpmE3C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE3C({ target: __sveltets_2_any(), props: {      "icon":`check`,"title":`Nothing in this window`,"body":enabled ? 'No restarts or pod replacements were recorded. Widen the window or check the raw cluster events.' : 'Monitoring is off for this cluster.',}});}
    }else{
       { svelteHTML.createElement("ol", {   "class":`timeline`,...__sveltets_2_empty({"data-testid":`k8s-monitor-events`}),});
            for(let e of __sveltets_2_ensureArray(events)){let i = 1;i;
           { svelteHTML.createElement("li", {});
             { svelteHTML.createElement("span", {   "class":`tdot`,"style":`background: ${classColor(e.class)}`,}); }
             { svelteHTML.createElement("span", { "class":`tts mono`,});fmtTs(e.ts); }
             { svelteHTML.createElement("span", {   "class":`tclass`,"style":`color: ${e.kind === 'version' ? 'var(--status-working)' : classColor(e.class)}`,});e.kind === 'k8s_event' ? e.reason : e.kind === 'version' ? 'New version' : classLabel(e.class); }
             { svelteHTML.createElement("span", { "class":`twl`,}); { svelteHTML.createElement("span", { "class":`dim`,});e.namespace;  } { svelteHTML.createElement("b", {});e.workload || e.pod; }if(e.pod && e.pod !== e.workload){ { svelteHTML.createElement("span", { "class":`dim`,});   } { svelteHTML.createElement("button", {     "class":`linkish mono`,"onclick":() => openPod(e.namespace, e.pod, 'events'),"title":`Open this pod’s Events in Resources`,});e.pod; }} }
             { svelteHTML.createElement("span", { "class":`tmsg dim`,});eventMsg(e); }
           }
        }
       }
    }
  }else{
     { svelteHTML.createElement("div", { "class":`toolbar`,});
       { svelteHTML.createElement("input", {          "dir":`ltr`,"class":`input`,"placeholder":`Filter workloads…`,"bind:value":filter,"aria-label":`Filter workloads`,});/*Ωignore_startΩ*/() => filter = __sveltets_2_any(null);/*Ωignore_endΩ*/}
      if(namespaces.length > 1 || ns){
         { svelteHTML.createElement("select", {     "class":`input`,"bind:value":ns,"aria-label":`Namespace`,});/*Ωignore_startΩ*/() => ns = __sveltets_2_any(null);/*Ωignore_endΩ*/
           { svelteHTML.createElement("option", {"value":"",});   }
             for(let n of __sveltets_2_ensureArray(namespaces)){n; { svelteHTML.createElement("option", { "value":n,});n; }}
         }
      }
      if(rbacMessage(status?.metrics_server)){
         { svelteHTML.createElement("span", {   "class":`chip`,"title":rbacMessage(status?.metrics_server) ?? '',});      }
      } else if (credentialsRejected(status?.metrics_server)){ { svelteHTML.createElement("span", {   "class":`chip`,"title":status?.metrics_server,});     }
      } else if (status?.metrics_server === 'ok'){ { svelteHTML.createElement("span", { "class":`chip ok`,});  }
      } else if (status?.metrics_server === 'disabled'){
         { svelteHTML.createElement("span", {   "class":`chip`,"title":`Metrics-server probing is off in Settings`,});  }
      }
       { svelteHTML.createElement("span", { "class":`spacer`,}); }
       { svelteHTML.createElement("button", {       "class":`btn ghost small`,"onclick":() => void loadWorkloads(),"aria-label":`Refresh workloads`,"title":`Refresh workloads`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":13,}});} }
     }
    if(!enabled && !loading){
       { const $$_etatSytpmE3C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE3C({ target: __sveltets_2_any(), props: {          "icon":`helm`,"title":`Monitoring is off`,"body":`Enable it in Settings: pick a preset, adjust ports, test the probes, save.`,"actionLabel":canEdit ? 'Open settings' : undefined,"onaction":canEdit ? () => goTab('settings') : undefined,}});}
    } else if (loading && !rows.length){
       { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {    "rows":8,"height":30,}});}
    } else if (error){
       { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {     "what":`workloads`,error,"empty":true,"onretry":() => void loadWorkloads(),}});}
    } else if (!rows.length){
       { const $$_etatSytpmE3C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE3C({ target: __sveltets_2_any(), props: {      "icon":`clock`,"title":`No data yet`,"body":`The first cycle runs within the configured interval. Use “Run once” in Settings to collect immediately.`,}});}
    }else{
       { svelteHTML.createElement("div", { "class":`tablewrap card`,});
         { svelteHTML.createElement("table", {     "class":`wl`,...__sveltets_2_empty({"data-testid":`k8s-monitor-workloads`}),"aria-rowcount":win ? visible.length + 1 : undefined,});
           { svelteHTML.createElement("thead", {});
             { svelteHTML.createElement("tr", {});
               { svelteHTML.createElement("th", { "aria-sort":sortKey === 'workload' ? (sortDir === 1 ? 'ascending' : 'descending') : 'none',}); { svelteHTML.createElement("button", {   "class":`th-btn`,"onclick":() => sortBy('workload'),});  } }
               { svelteHTML.createElement("th", {   "class":`num`,"aria-sort":sortKey === 'pods' ? (sortDir === 1 ? 'ascending' : 'descending') : 'none',}); { svelteHTML.createElement("button", {   "class":`th-btn`,"onclick":() => sortBy('pods'),});  } }
               { svelteHTML.createElement("th", {   "class":`num`,"title":`Per pod: average · hungriest pod; % = worst pod vs its own limit. Click a row for every pod.`,});
                 { svelteHTML.createElement("button", {    "class":`th-btn`,"onclick":() => sortBy('mem_max'),});sortKey === 'mem_max';   }
                 { svelteHTML.createElement("span", { "class":`dim`,});   }
                 { svelteHTML.createElement("button", {      "class":`th-btn`,"onclick":() => sortBy('mem_pct'),"title":`Sort by % of limit (worst pod)`,});sortKey === 'mem_pct';  }
               }
               { svelteHTML.createElement("th", { "class":`spark`,});  }
               { svelteHTML.createElement("th", {   "class":`num`,"aria-sort":sortKey === 'restarts_total' ? (sortDir === 1 ? 'ascending' : 'descending') : 'none',}); { svelteHTML.createElement("button", {   "class":`th-btn`,"onclick":() => sortBy('restarts_total'),});  } }
               { svelteHTML.createElement("th", {   "class":`num`,"aria-sort":sortKey === 'churn_planned' ? (sortDir === 1 ? 'ascending' : 'descending') : 'none',}); { svelteHTML.createElement("button", {   "class":`th-btn`,"onclick":() => sortBy('churn_planned'),});  } }
               { svelteHTML.createElement("th", {   "class":`num`,"aria-sort":sortKey === 'rps' ? (sortDir === 1 ? 'ascending' : 'descending') : 'none',}); { svelteHTML.createElement("button", {   "class":`th-btn`,"onclick":() => sortBy('rps'),});  } }
               { svelteHTML.createElement("th", { "class":`spark`,});  }
               { svelteHTML.createElement("th", {   "class":`num`,"aria-sort":sortKey === 'err_pct' ? (sortDir === 1 ? 'ascending' : 'descending') : 'none',}); { svelteHTML.createElement("button", {   "class":`th-btn`,"onclick":() => sortBy('err_pct'),});  } }
               { svelteHTML.createElement("th", {   "class":`num`,"aria-sort":sortKey === 'latency_ms' ? (sortDir === 1 ? 'ascending' : 'descending') : 'none',}); { svelteHTML.createElement("button", {   "class":`th-btn`,"onclick":() => sortBy('latency_ms'),});  } }
               { svelteHTML.createElement("th", {});  }
             }
           }
           { const $$_tbody5 = svelteHTML.createElement("tbody", { });tbodyEl = $$_tbody5;
            if(win && win.padTop > 0){ { svelteHTML.createElement("tr", {     "class":`spacer`,"aria-hidden":`true`,"style":`height:${win.padTop}px`,}); { svelteHTML.createElement("td", { "colspan":11,}); } }}
                for(let r of __sveltets_2_ensureArray(shown)){let j = 1;`${r.namespace}/${r.workload}`;
              const key = `${r.namespace}/${r.workload}`;
              const total = restartsTotal(r);
               {const $$action_0 = __sveltets_2_ensureAction(rowMenuKeys(svelteHTML.mapElementTag('tr')));{ svelteHTML.createElement("tr", __sveltets_2_union($$action_0), {         "class":`wl-row`,"aria-rowindex":win ? win.start + j + 2 : undefined,"onclick":() => void toggle(r),"oncontextmenu":(e) => rowMenu(e, r),});expanded === key;
                 { svelteHTML.createElement("td", {});
                   { svelteHTML.createElement("div", { "class":`wlname`,}); { svelteHTML.createElement("button", {     "class":`workload-toggle`,"aria-expanded":expanded === key,"onclick":(e) => { e.stopPropagation(); void toggle(r); },});r.workload; } { svelteHTML.createElement("span", { "class":`dim small`,}); r.kind;namespaces.length > 1 ? ` · ${r.namespace}` : ''; } }
                  if(r.crashloop){ { svelteHTML.createElement("span", { "class":`chip bad`,}); r.crashloop; }}
                 }
                 { svelteHTML.createElement("td", { "class":`num mono`,});r.ready; { svelteHTML.createElement("span", { "class":`dim`,}); r.pods; } }
                 { svelteHTML.createElement("td", { "class":`num mono`,});
                  if(r.mem_sampled > 0){
                     { svelteHTML.createElement("span", { "title":`average per pod`,});formatBytes(r.mem_avg); }
                    if(r.pods > 1){ { svelteHTML.createElement("span", { "class":`dim`,});    } { svelteHTML.createElement("span", { "title":`hungriest pod: ${r.mem_max_pod}`,});formatBytes(r.mem_max); }}
                     { svelteHTML.createElement("div", {  "class":`pct`,});r.mem_pct >= 85;fmtPct(r.mem_pct, 0);  if(r.pods > 1){' '; }if(r.mem_sampled < r.pods){' '; r.mem_sampled; r.pods; } }
                  }else{
                     { svelteHTML.createElement("span", {   "class":`dim`,"title":`No memory sample yet — configure a memory probe (e.g. the Go actuator preset) or grant metrics-server`,});  }
                    if(r.mem_limit > 0){ { svelteHTML.createElement("div", { "class":`pct`,}); formatBytes(r.mem_limit / Math.max(1, r.pods));  }}
                  }
                  if(r.mem_trend_pct !== null && r.mem_trend_pct !== undefined && Math.abs(r.mem_trend_pct) >= 5){ { svelteHTML.createElement("div", {  "class":`pct`,});r.mem_trend_pct >= 25;r.mem_trend_pct > 0 ? '+' : '';r.mem_trend_pct.toFixed(0);  window; }}
                 }
                 { svelteHTML.createElement("td", { "class":`spark`,}); { const $$_enilkrapS8C = __sveltets_2_ensureComponent(Sparkline); new $$_enilkrapS8C({ target: __sveltets_2_any(), props: {    "points":r.spark.mem,"label":`memory trend`,}});} }
                 { svelteHTML.createElement("td", { "class":`num mono`,});
                  if(total){
                     { svelteHTML.createElement("span", {   "class":`rc`,"style":`color: ${r.restarts.oom ? classColor('oom') : r.restarts.crash ? classColor('crash') : 'inherit'}`,});total; }
                     { svelteHTML.createElement("div", { "class":`pct`,});if(r.restarts.oom){ r.restarts.oom; }if(r.restarts.crash){ r.restarts.crash; }if(r.restarts.probe){ r.restarts.probe; }if(r.restarts.unknown){ r.restarts.unknown;} }
                  }else{ { svelteHTML.createElement("span", { "class":`dim`,});  }}
                 }
                 { svelteHTML.createElement("td", { "class":`num mono`,});r.churn_planned || r.churn_unknown ? `${r.churn_planned}` : '—';if(r.churn_unknown){ { svelteHTML.createElement("span", { "class":`dim`,}); r.churn_unknown;  }} }
                 { svelteHTML.createElement("td", { "class":`num mono`,});r.rps > 0 ? fmtRate(r.rps) : '—'; }
                 { svelteHTML.createElement("td", { "class":`spark`,}); { const $$_enilkrapS8C = __sveltets_2_ensureComponent(Sparkline); new $$_enilkrapS8C({ target: __sveltets_2_any(), props: {      "points":r.spark.rps,"label":`request-rate trend`,"stroke":`var(--status-working)`,}});} }
                 { svelteHTML.createElement("td", {  "class":`num mono`,});r.err_pct >= 1 && r.err_pct >= 3 * Math.max(r.err_pct_baseline, 0.1);r.rps > 0 ? fmtPct(r.err_pct, 2) : '—'; }
                 { svelteHTML.createElement("td", {  "class":`num mono`,});r.latency_ms > 0 && r.latency_baseline_ms > 0 && r.latency_ms >= 3 * r.latency_baseline_ms;r.latency_ms > 0 ? `${fmtMs(r.latency_ms)} ${r.latency_kind}` : '—'; }
                 { svelteHTML.createElement("td", {});
                     for(let v of __sveltets_2_ensureArray(r.versions)){v; { svelteHTML.createElement("span", {  "class":`chip`,});r.versions.length > 1;v; }}
                 }
               }}
              if(expanded === key){
                 { svelteHTML.createElement("tr", { "class":`detail`,});
                   { svelteHTML.createElement("td", { "colspan":11,});
                     { svelteHTML.createElement("div", { "class":`detail-tools`,});
                       { svelteHTML.createElement("span", { "class":`dim small`,});  r.workload; }
                       { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":() => openPods(r),...__sveltets_2_empty({"data-testid":`k8s-monitor-open-pods`}),}); { const $$_nocI10C = __sveltets_2_ensureComponent(Icon); new $$_nocI10C({ target: __sveltets_2_any(), props: {    "name":`box`,"size":12,}});}     }
                     }
                     { svelteHTML.createElement("table", {   "class":`pods`,...__sveltets_2_empty({"data-testid":`k8s-monitor-pods`}),});
                       { svelteHTML.createElement("thead", {});
                         { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", { "class":`num`,});  } { svelteHTML.createElement("th", { "class":`num`,}); window;  } { svelteHTML.createElement("th", { "class":`num`,});  } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", { "class":`num`,});  } }
                       }
                       { svelteHTML.createElement("tbody", {});
                           for(let p of __sveltets_2_ensureArray(r.pods_detail)){p.pod;
                          const pr = p.restarts.oom + p.restarts.crash + p.restarts.probe + p.restarts.unknown;
                           { svelteHTML.createElement("tr", {  "class":`pod-row`,});p.crashloop || pr > 0 || (p.mem_pct >= 85);
                             { svelteHTML.createElement("td", { "class":`mono`,}); { svelteHTML.createElement("button", {     "class":`linkish mono`,"onclick":() => openPod(r.namespace, p.pod, 'metrics'),"title":`Open this pod (Metrics) in Resources`,});p.pod; } }
                             { svelteHTML.createElement("td", { "class":`mono dim`,});p.node || '—'; }
                             { svelteHTML.createElement("td", {});if(p.crashloop){ { svelteHTML.createElement("span", { "class":`chip bad`,});  }}else{ { svelteHTML.createElement("span", {  "class":`chip`,});p.ready;p.ready ? 'Ready' : p.phase; }} }
                             { svelteHTML.createElement("td", { "class":`num mono`,});
                              if(p.mem_bytes > 0){formatBytes(p.mem_bytes);if(p.mem_limit > 0){ { svelteHTML.createElement("div", {  "class":`pct`,});p.mem_pct >= 85;fmtPct(p.mem_pct, 0);  formatBytes(p.mem_limit); }}}else{ { svelteHTML.createElement("span", { "class":`dim`,});  }}
                             }
                             { svelteHTML.createElement("td", {  "class":`num mono`,});pr > 0;pr;if(pr){ { svelteHTML.createElement("div", { "class":`pct`,});if(p.restarts.oom){ p.restarts.oom; }if(p.restarts.crash){ p.restarts.crash; }if(p.restarts.probe){ p.restarts.probe; }if(p.restarts.unknown){ p.restarts.unknown;} }} }
                             { svelteHTML.createElement("td", { "class":`num mono dim`,});p.restarts_lifetime; }
                             { svelteHTML.createElement("td", {});if(p.version){ { svelteHTML.createElement("span", {  "class":`chip`,});r.versions.length > 1 && p.version !== r.versions[0];p.version; }}else{ { svelteHTML.createElement("span", { "class":`dim`,});  }} }
                             { svelteHTML.createElement("td", { "class":`num mono dim`,});formatAge(p.age_seconds); }
                           }
                        }
                       }
                     }
                    if(seriesLoading){
                       { const $$_notelekS8C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS8C({ target: __sveltets_2_any(), props: {    "rows":2,"height":60,}});}
                    } else if (seriesError){
                       {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),({ text: `Couldn’t load trends. ${seriesError}`, assertive: true })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {   "class":`trend-error`,}); { svelteHTML.createElement("strong", {});   } { svelteHTML.createElement("p", {});seriesError; } { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void loadTrends(r),});  } }}
                    }else{
                       { svelteHTML.createElement("div", { "class":`charts`,});
                         { svelteHTML.createElement("div", { "class":`chart`,});
                           { svelteHTML.createElement("div", { "class":`ct`,});  { svelteHTML.createElement("span", { "class":`dim`,}); series.mem?.metric ?? '…';  } }
                           { const $$_enilkrapS10C = __sveltets_2_ensureComponent(Sparkline); new $$_enilkrapS10C({ target: __sveltets_2_any(), props: {        "points":series.mem?.points.map((p) => p.v) ?? [],"width":420,"height":80,"label":`memory`,}});}
                           { svelteHTML.createElement("div", { "class":`dim small`,});series.mem?.points.length ?? 0;    series.mem?.step_secs ?? '—';  }
                         }
                         { svelteHTML.createElement("div", { "class":`chart`,});
                           { svelteHTML.createElement("div", { "class":`ct`,});  }
                           { const $$_enilkrapS10C = __sveltets_2_ensureComponent(Sparkline); new $$_enilkrapS10C({ target: __sveltets_2_any(), props: {          "points":series.rps?.points.map((p) => p.v) ?? [],"width":420,"height":80,"stroke":`var(--status-working)`,"label":`requests`,}});}
                           { svelteHTML.createElement("div", { "class":`dim small`,}); fmtRate(r.rps_baseline);    fmtPct(r.err_pct_baseline, 2);if(r.latency_baseline_ms){' ';   fmtMs(r.latency_baseline_ms);} }
                         }
                       }
                    }
                   }
                 }
              }
            }
            if(win && win.padBottom > 0){ { svelteHTML.createElement("tr", {     "class":`spacer`,"aria-hidden":`true`,"style":`height:${win.padBottom}px`,}); { svelteHTML.createElement("td", { "colspan":11,}); } }}
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
const MonitorCluster__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type MonitorCluster__SvelteComponent_ = ReturnType<typeof MonitorCluster__SvelteComponent_>;
/*Ωignore_endΩ*/export default MonitorCluster__SvelteComponent_;