<script lang="ts">
  // Per-cluster Monitor view. Tabs live in the URL
  // (`#/kubernetes/monitor/<id>/<workloads|events|insights|settings>`):
  // Workloads = sortable table with sparklines + an expandable Trends row;
  // Events = classified restart / churn timeline; Insights = the watchdog
  // agent's latest report; Settings = the probe configuration. Re-fetches on
  // WS `k8s_monitor_cycle` for this cluster.
  import { untrack } from 'svelte';
  import { router } from '../../../lib/router.svelte';
  import { k8s } from '../../../lib/stores/k8s.svelte';
  import { auth } from '../../../lib/stores/auth.svelte';
  import { k8sApi } from '../../../lib/api/k8s';
  import type { K8sCluster, K8sMonitorEvent, K8sMonitorSeries, K8sMonitorStatus, K8sMonitorWorkloadRow } from '../../../lib/api/types';
  import Icon from '../../../lib/components/Icon.svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Skeleton from '../../../lib/components/Skeleton.svelte';
  import { envBadge, formatBytes } from '../k8s-util';
  import Sparkline from './Sparkline.svelte';
  import MonitorSettings from './MonitorSettings.svelte';
  import MonitorInsights from './MonitorInsights.svelte';
  import { WINDOWS, classColor, classLabel, collectorLine, fmtMs, fmtPct, fmtRate, isWindow, rbacMessage } from './monitor-util';

  interface Props {
    cluster: K8sCluster;
    tab: string;
  }
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
  let rows = $state<K8sMonitorWorkloadRow[]>([]);
  let status = $state<K8sMonitorStatus | null>(null);
  let enabled = $state(true);
  let loading = $state(true);
  let error = $state('');
  let ns = $state('');
  let filter = $state('');
  let sortKey = $state<keyof K8sMonitorWorkloadRow | 'restarts_total'>('mem_bytes');
  let sortDir = $state<1 | -1>(-1);
  let expanded = $state<string | null>(null);
  let series = $state<{ mem: K8sMonitorSeries | null; rps: K8sMonitorSeries | null; err: K8sMonitorSeries | null }>({ mem: null, rps: null, err: null });
  let seriesLoading = $state(false);
  let abort: AbortController | null = null;

  function goTab(id: string): void {
    router.go(`kubernetes/monitor/${encodeURIComponent(cluster.id)}/${id}`);
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
      error = '';
    } catch (e) {
      if (e instanceof DOMException && e.name === 'AbortError') return;
      error = e instanceof Error ? e.message : String(e);
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
    if (t === 'workloads') untrack(() => void loadWorkloads());
    if (t === 'events') untrack(() => void loadEvents());
  });

  $effect(() => {
    const tick = k8s.monitorTick;
    const who = k8s.monitorTickCluster;
    if (tick > 0 && who === cluster.id) {
      untrack(() => {
        if (activeTab === 'workloads') void loadWorkloads(true);
        if (activeTab === 'events') void loadEvents(true);
      });
    }
  });

  const namespaces = $derived([...new Set(rows.map((r) => r.namespace))].sort());
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
      return;
    }
    expanded = key;
    seriesLoading = true;
    series = { mem: null, rps: null, err: null };
    try {
      const memMetric = status?.metrics_server === 'ok' ? 'mem_working_set_bytes' : 'mem_sys_bytes';
      const [mem, rps] = await Promise.all([
        k8sApi.monitorSeries(cluster.id, { metric: memMetric, workload: r.workload, window }),
        k8sApi.monitorSeries(cluster.id, { metric: 'http_requests_total', workload: r.workload, window }),
      ]);
      series = { mem, rps, err: null };
    } catch {
      /* charts are best-effort */
    } finally {
      seriesLoading = false;
    }
  }

  // --- events --------------------------------------------------------------------
  let events = $state<K8sMonitorEvent[]>([]);
  let eventsLoading = $state(false);
  let eventsError = $state('');
  let classFilter = $state('');
  const CLASS_OPTIONS = ['', 'oom', 'crash', 'probe', 'unknown', 'planned', 'completed', 'k8s_event'];

  async function loadEvents(quiet = false): Promise<void> {
    if (!quiet) eventsLoading = true;
    try {
      events = await k8sApi.monitorEvents(cluster.id, { window, class: classFilter || undefined, limit: 300 });
      eventsError = '';
    } catch (e) {
      eventsError = e instanceof Error ? e.message : String(e);
    } finally {
      eventsLoading = false;
    }
  }
  $effect(() => {
    const c = classFilter;
    void c;
    if (activeTab === 'events') untrack(() => void loadEvents());
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
    if (e.kind === 'restart') {
      const p = e.detail?.prev_restarts as number | undefined;
      const n = e.detail?.next_restarts as number | undefined;
      return `${e.container ? e.container + ' ' : ''}restart ${p ?? '?'}→${n ?? '?'}${e.reason ? ` · ${e.reason}` : ''}${e.exit_code ? ` · exit ${e.exit_code}` : ''}`;
    }
    return `pod replaced${plannedBy(e)}${e.reason ? ` · ${e.reason}` : ''}`;
  }
</script>

<div class="page mon" data-testid="k8s-monitor-cluster">
  <div class="page-header">
    <div class="titles">
      <h1>
        <button class="crumb" onclick={() => router.go('kubernetes')}>Kubernetes</button>
        <span class="sep">/</span>
        <button class="crumb" onclick={() => router.go('kubernetes/monitor')}>Monitor</button>
        <span class="sep">/</span>
        <span class="dot" style="background: {cluster.color ?? 'var(--accent)'}"></span>
        {cluster.name}
        <span class="env-badge" class:prod={cluster.environment === 'prod'}>{envBadge(cluster.environment)}</span>
      </h1>
      <div class="sub" title={status?.last_error || ''}>{collectorLine(status, enabled)}</div>
    </div>
    <div class="actions">
      {#if activeTab === 'workloads' || activeTab === 'events'}
        <div class="seg" role="radiogroup" aria-label="Window">
          {#each WINDOWS as w (w)}
            <button class="seg-btn" class:on={window === w} role="radio" aria-checked={window === w} onclick={() => (window = w)}>{w}</button>
          {/each}
        </div>
      {/if}
      <button class="btn small ghost" onclick={() => router.go(`kubernetes/${encodeURIComponent(cluster.id)}`)} title="Open the console for this cluster"><Icon name="helm" size={12} /> Console</button>
    </div>
  </div>

  <nav class="tabs" aria-label="Monitor sections">
    {#each TABS as t (t.id)}
      <button class="tab" class:active={activeTab === t.id} onclick={() => goTab(t.id)} aria-current={activeTab === t.id ? 'page' : undefined} data-testid="k8s-monitor-tab-{t.id}">{t.label}</button>
    {/each}
  </nav>

  {#if activeTab === 'settings'}
    <MonitorSettings {cluster} {canEdit} onsaved={(c, s) => { enabled = c.enabled; status = s; }} />
  {:else if activeTab === 'insights'}
    <MonitorInsights />
  {:else if activeTab === 'events'}
    <div class="toolbar">
      <select class="input" bind:value={classFilter} aria-label="Event class">
        {#each CLASS_OPTIONS as c (c)}
          <option value={c}>{c === '' ? 'Restarts + churn' : c === 'k8s_event' ? 'Raw cluster events' : classLabel(c)}</option>
        {/each}
      </select>
      <button class="btn ghost small" onclick={() => void loadEvents()} aria-label="Refresh events"><Icon name="refresh" size={13} /></button>
    </div>
    {#if eventsLoading && !events.length}
      <Skeleton rows={6} height={28} />
    {:else if eventsError}
      <EmptyState icon="helm" title="Couldn't load events" body={eventsError} actionLabel="Retry" onaction={() => void loadEvents()} />
    {:else if !events.length}
      <EmptyState icon="check" title="Nothing in this window" body={enabled ? 'No restarts or pod replacements were recorded. Widen the window or check the raw cluster events.' : 'Monitoring is off for this cluster.'} />
    {:else}
      <ol class="timeline" data-testid="k8s-monitor-events">
        {#each events as e, i (i)}
          <li>
            <span class="tdot" style="background: {classColor(e.class)}"></span>
            <span class="tts mono">{fmtTs(e.ts)}</span>
            <span class="tclass" style="color: {classColor(e.class)}">{e.kind === 'k8s_event' ? e.reason : classLabel(e.class)}</span>
            <span class="twl"><b>{e.workload || e.pod}</b>{#if e.pod && e.pod !== e.workload}<span class="dim"> · {e.pod}</span>{/if}</span>
            <span class="tmsg dim">{eventMsg(e)}</span>
          </li>
        {/each}
      </ol>
    {/if}
  {:else}
    <div class="toolbar">
      <input class="input" placeholder="Filter workloads…" bind:value={filter} aria-label="Filter workloads" />
      {#if namespaces.length > 1 || ns}
        <select class="input" bind:value={ns} aria-label="Namespace">
          <option value="">All configured namespaces</option>
          {#each namespaces as n (n)}<option value={n}>{n}</option>{/each}
        </select>
      {/if}
      {#if rbacMessage(status?.metrics_server)}
        <span class="chip" title={rbacMessage(status?.metrics_server) ?? ''}>metrics-server: RBAC denied — CPU unavailable</span>
      {:else if status?.metrics_server === 'ok'}
        <span class="chip ok">metrics-server ok</span>
      {/if}
      <span class="spacer"></span>
      <button class="btn ghost small" onclick={() => void loadWorkloads()} aria-label="Refresh workloads"><Icon name="refresh" size={13} /></button>
    </div>
    {#if !enabled && !loading}
      <EmptyState icon="helm" title="Monitoring is off" body="Enable it in Settings: pick a preset, adjust ports, test the probes, save." actionLabel={canEdit ? 'Open settings' : undefined} onaction={canEdit ? () => goTab('settings') : undefined} />
    {:else if loading && !rows.length}
      <Skeleton rows={8} height={30} />
    {:else if error}
      <EmptyState icon="helm" title="Couldn't load workloads" body={error} actionLabel="Retry" onaction={() => void loadWorkloads()} />
    {:else if !rows.length}
      <EmptyState icon="clock" title="No data yet" body="The first cycle runs within the configured interval. Use “Run once” in Settings to collect immediately." />
    {:else}
      <div class="tablewrap card">
        <table class="wl" data-testid="k8s-monitor-workloads">
          <thead>
            <tr>
              <th class="sortable" onclick={() => sortBy('workload')}>Workload</th>
              <th class="num sortable" onclick={() => sortBy('pods')}>Pods</th>
              <th class="num sortable" onclick={() => sortBy('mem_bytes')}>Memory</th>
              <th class="spark">Trend</th>
              <th class="num sortable" onclick={() => sortBy('restarts_total')}>Restarts</th>
              <th class="num sortable" onclick={() => sortBy('churn_planned')}>Churn</th>
              <th class="num sortable" onclick={() => sortBy('rps')}>Req/s</th>
              <th class="spark">Trend</th>
              <th class="num sortable" onclick={() => sortBy('err_pct')}>5xx</th>
              <th class="num sortable" onclick={() => sortBy('latency_ms')}>Latency</th>
              <th>Versions</th>
            </tr>
          </thead>
          <tbody>
            {#each visible as r (`${r.namespace}/${r.workload}`)}
              {@const key = `${r.namespace}/${r.workload}`}
              {@const total = restartsTotal(r)}
              <tr class="row" class:open={expanded === key} onclick={() => void toggle(r)}>
                <td>
                  <div class="wlname"><b>{r.workload}</b><span class="dim small"> {r.kind}{namespaces.length > 1 ? ` · ${r.namespace}` : ''}</span></div>
                  {#if r.crashloop}<span class="chip bad">CrashLoopBackOff ×{r.crashloop}</span>{/if}
                </td>
                <td class="num mono">{r.ready}<span class="dim">/{r.pods}</span></td>
                <td class="num mono">
                  {formatBytes(r.mem_bytes)}
                  {#if r.mem_limit > 0}<div class="pct" class:warn={r.mem_pct >= 85}>{fmtPct(r.mem_pct, 0)} of {formatBytes(r.mem_limit)}</div>{/if}
                  {#if r.mem_trend_pct !== null && r.mem_trend_pct !== undefined && Math.abs(r.mem_trend_pct) >= 5}<div class="pct" class:warn={r.mem_trend_pct >= 25}>{r.mem_trend_pct > 0 ? '+' : ''}{r.mem_trend_pct.toFixed(0)}% / {window}</div>{/if}
                </td>
                <td class="spark"><Sparkline points={r.spark.mem} label="memory trend" /></td>
                <td class="num mono">
                  {#if total}
                    <span class="rc" style="color: {r.restarts.oom ? classColor('oom') : r.restarts.crash ? classColor('crash') : 'inherit'}">{total}</span>
                    <div class="pct">{#if r.restarts.oom}oom {r.restarts.oom} {/if}{#if r.restarts.crash}crash {r.restarts.crash} {/if}{#if r.restarts.probe}probe {r.restarts.probe} {/if}{#if r.restarts.unknown}? {r.restarts.unknown}{/if}</div>
                  {:else}<span class="dim">0</span>{/if}
                </td>
                <td class="num mono">{r.churn_planned || r.churn_unknown ? `${r.churn_planned}` : '—'}{#if r.churn_unknown}<span class="dim"> +{r.churn_unknown}?</span>{/if}</td>
                <td class="num mono">{r.rps > 0 ? fmtRate(r.rps) : '—'}</td>
                <td class="spark"><Sparkline points={r.spark.rps} label="request-rate trend" stroke="var(--status-working)" /></td>
                <td class="num mono" class:bad={r.err_pct >= 1 && r.err_pct >= 3 * Math.max(r.err_pct_baseline, 0.1)}>{r.rps > 0 ? fmtPct(r.err_pct, 2) : '—'}</td>
                <td class="num mono" class:bad={r.latency_ms > 0 && r.latency_baseline_ms > 0 && r.latency_ms >= 3 * r.latency_baseline_ms}>{r.latency_ms > 0 ? `${fmtMs(r.latency_ms)} ${r.latency_kind}` : '—'}</td>
                <td>
                  {#each r.versions as v (v)}<span class="chip" class:accent={r.versions.length > 1}>{v}</span>{/each}
                </td>
              </tr>
              {#if expanded === key}
                <tr class="detail">
                  <td colspan="11">
                    {#if seriesLoading}
                      <Skeleton rows={2} height={60} />
                    {:else}
                      <div class="charts">
                        <div class="chart">
                          <div class="ct">Memory <span class="dim">({series.mem?.metric ?? '…'})</span></div>
                          <Sparkline points={series.mem?.points.map((p) => p.v) ?? []} width={420} height={80} label="memory" />
                          <div class="dim small">{series.mem?.points.length ?? 0} points · step {series.mem?.step_secs ?? '—'}s</div>
                        </div>
                        <div class="chart">
                          <div class="ct">Requests/s</div>
                          <Sparkline points={series.rps?.points.map((p) => p.v) ?? []} width={420} height={80} stroke="var(--status-working)" label="requests" />
                          <div class="dim small">baseline {fmtRate(r.rps_baseline)} · 5xx baseline {fmtPct(r.err_pct_baseline, 2)}{#if r.latency_baseline_ms} · latency baseline {fmtMs(r.latency_baseline_ms)}{/if}</div>
                        </div>
                      </div>
                    {/if}
                  </td>
                </tr>
              {/if}
            {/each}
          </tbody>
        </table>
      </div>
    {/if}
  {/if}
</div>

<style>
  .mon {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .titles h1 {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }
  .crumb {
    background: none;
    border: none;
    padding: 0;
    color: var(--text-dim);
    font: inherit;
    cursor: pointer;
  }
  .crumb:hover {
    color: var(--accent);
  }
  .sep {
    color: var(--text-dim);
  }
  .dot {
    width: 9px;
    height: 9px;
    border-radius: 50%;
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
  .wl th.sortable {
    cursor: pointer;
  }
  .wl th.sortable:hover {
    color: var(--text);
  }
  .wl td {
    padding: 7px 10px;
    border-bottom: 1px solid var(--border);
    vertical-align: top;
  }
  .row {
    cursor: pointer;
  }
  .row:hover {
    background: color-mix(in srgb, var(--accent) 4%, transparent);
  }
  .row.open {
    background: var(--surface-2);
  }
  .num {
    text-align: right;
    white-space: nowrap;
  }
  .spark {
    width: 120px;
  }
  .pct {
    font-size: 10.5px;
    color: var(--text-dim);
  }
  .pct.warn {
    color: var(--status-exited);
  }
  .bad {
    color: var(--status-exited);
  }
  .rc {
    font-weight: 600;
  }
  .wlname {
    white-space: nowrap;
  }
  .detail td {
    background: var(--surface-2);
  }
  .charts {
    display: flex;
    gap: 24px;
    flex-wrap: wrap;
  }
  .chart {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .ct {
    font-size: 11px;
    font-weight: 600;
  }
  .timeline {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .timeline li {
    display: grid;
    grid-template-columns: 10px 150px 80px minmax(120px, 1fr) 2fr;
    gap: 8px;
    align-items: baseline;
    padding: 5px 8px;
    border-radius: 6px;
    font-size: 12px;
  }
  .timeline li:hover {
    background: var(--surface-2);
  }
  .tdot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    align-self: center;
  }
  .tts {
    font-size: 11px;
    color: var(--text-dim);
    white-space: nowrap;
  }
  .tclass {
    font-weight: 600;
    font-size: 11px;
  }
  .twl,
  .tmsg {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dim {
    color: var(--text-dim);
  }
  .small {
    font-size: 11px;
  }
  .mono {
    font-family: var(--font-mono);
  }
  @media (max-width: 760px) {
    .timeline li {
      grid-template-columns: 10px 1fr;
    }
    .tts,
    .tmsg {
      grid-column: 2;
    }
  }
</style>
