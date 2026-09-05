<script lang="ts">
  // Monitor overview: one card per registered cluster (health badge, pods,
  // restarts by class, memory vs limits, rps / error %, version drift, and the
  // collector status line with the exact metrics-server RBAC message when it
  // is denied). Refreshes on WS `k8s_monitor_cycle` and on a window change.
  import { untrack } from 'svelte';
  import { router } from '../../../lib/router.svelte';
  import { k8s } from '../../../lib/stores/k8s.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import { k8sApi } from '../../../lib/api/k8s';
  import type { K8sMonitorOverviewRow } from '../../../lib/api/types';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Skeleton from '../../../lib/components/Skeleton.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { envBadge, formatBytes } from '../k8s-util';
  import { WINDOWS, classColor, classLabel, collectorLine, fmtPct, fmtRate, healthLabel, isWindow, rbacMessage } from './monitor-util';

  let window = $state<(typeof WINDOWS)[number]>('24h');
  let rows = $state<K8sMonitorOverviewRow[]>([]);
  let loading = $state(true);
  let error = $state('');
  let abort: AbortController | null = null;

  try {
    const saved = localStorage.getItem('otto_k8s_monitor_window');
    if (isWindow(saved ?? undefined)) window = saved as (typeof WINDOWS)[number];
  } catch {
    /* storage unavailable */
  }

  async function load(quiet = false): Promise<void> {
    abort?.abort();
    abort = new AbortController();
    if (!quiet) loading = true;
    try {
      rows = await k8sApi.monitorOverview(window, abort.signal);
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
    try {
      localStorage.setItem('otto_k8s_monitor_window', w);
    } catch {
      /* ignore */
    }
    untrack(() => void load());
  });

  // Live refresh after any cluster's cycle.
  $effect(() => {
    const t = k8s.monitorTick;
    if (t > 0) untrack(() => void load(true));
  });

  function open(row: K8sMonitorOverviewRow, tab = 'workloads'): void {
    router.go(`kubernetes/monitor/${encodeURIComponent(row.cluster.id)}/${tab}`);
  }

  async function copy(text: string): Promise<void> {
    try {
      await navigator.clipboard.writeText(text);
      toasts.success('Copied', 'RBAC message copied to the clipboard.');
    } catch {
      toasts.error('Copy failed', text);
    }
  }

  function restartTotal(r: K8sMonitorOverviewRow): number {
    return r.restarts.oom + r.restarts.crash + r.restarts.probe + r.restarts.unknown;
  }
  const CLASSES = ['oom', 'crash', 'probe', 'unknown'] as const;
</script>

<div class="page" data-testid="k8s-monitor-overview">
  <div class="page-header">
    <div>
      <h1>
        <button class="crumb" onclick={() => router.go('kubernetes')}>Kubernetes</button>
        <span class="sep">/</span> Monitor
      </h1>
      <div class="sub">Pod-level metrics from your services' own endpoints, restart classification and a health digest per cluster.</div>
    </div>
    <div class="actions">
      <div class="seg" role="radiogroup" aria-label="Window">
        {#each WINDOWS as w (w)}
          <button class="seg-btn" class:on={window === w} role="radio" aria-checked={window === w} onclick={() => (window = w)}>{w}</button>
        {/each}
      </div>
      <button class="btn ghost" onclick={() => void load()} title="Refresh" aria-label="Refresh overview"><Icon name="refresh" size={14} /></button>
    </div>
  </div>

  {#if loading && !rows.length}
    <div class="grid"><Skeleton rows={3} height={140} /></div>
  {:else if error && !rows.length}
    <EmptyState icon="helm" title="Couldn't load the overview" body={error} actionLabel="Retry" onaction={() => void load()} />
  {:else if !rows.length}
    <EmptyState icon="helm" title="No clusters yet" body="Add a cluster in the Kubernetes console first, then enable monitoring on it here." actionLabel="Open clusters" onaction={() => router.go('kubernetes')} />
  {:else}
    <div class="grid" data-testid="k8s-monitor-grid">
      {#each rows as r (r.cluster.id)}
        {@const h = healthLabel(r.health)}
        {@const total = restartTotal(r)}
        {@const rbac = rbacMessage(r.status?.metrics_server)}
        <div class="card cluster" class:off={!r.enabled} role="button" tabindex="0" onclick={() => open(r)} onkeydown={(e) => { if (e.key === 'Enter') open(r); }} data-testid="k8s-monitor-card" data-health={r.health}>
          <div class="row1">
            <span class="dot" style="background: {r.cluster.color ?? 'var(--accent)'}"></span>
            <span class="name">{r.cluster.name}</span>
            <span class="env-badge" class:prod={r.cluster.environment === 'prod'}>{envBadge(r.cluster.environment)}</span>
            <span class="health {h.cls}" data-testid="k8s-monitor-health">{h.label}</span>
          </div>

          {#if !r.enabled}
            <div class="off-body">
              <span class="dim">Nothing is collected for this cluster.</span>
              <button class="btn small primary" onclick={(e) => { e.stopPropagation(); open(r, 'settings'); }}>Enable monitoring</button>
            </div>
          {:else}
            <div class="stats">
              <div class="stat">
                <span class="k">Pods</span>
                <span class="v mono">{r.pods.running}<span class="dim">/{r.pods.total}</span></span>
                {#if r.pods.pending || r.pods.failed || r.pods.crashloop}
                  <span class="note">
                    {#if r.pods.pending}{r.pods.pending} pending{/if}
                    {#if r.pods.failed}{r.pods.pending ? ' · ' : ''}{r.pods.failed} failed{/if}
                    {#if r.pods.crashloop}{r.pods.pending || r.pods.failed ? ' · ' : ''}{r.pods.crashloop} crashloop{/if}
                  </span>
                {/if}
              </div>
              <div class="stat">
                <span class="k">Restarts <span class="dim">({r.window})</span></span>
                <span class="v mono">{total}<span class="dim"> unplanned</span></span>
                <span class="note">{r.churn} planned replacement{r.churn === 1 ? '' : 's'}</span>
              </div>
              <div class="stat">
                <span class="k">Memory</span>
                <span class="v mono">{formatBytes(r.mem.used)}{#if r.mem.limit > 0}<span class="dim"> / {formatBytes(r.mem.limit)}</span>{/if}</span>
                {#if r.mem.limit > 0}
                  <div class="bar" title="{fmtPct(r.mem.pct)} of limits"><div class="fill" class:warn={r.mem.pct >= 85} style="width: {Math.min(100, r.mem.pct)}%"></div></div>
                {/if}
              </div>
              <div class="stat">
                <span class="k">Traffic</span>
                <span class="v mono">{fmtRate(r.rps)}</span>
                <span class="note" class:bad={r.err_pct >= 1}>{fmtPct(r.err_pct, 2)} 5xx</span>
              </div>
            </div>

            {#if total > 0}
              <div class="stack" aria-label="Restarts by class">
                {#each CLASSES as c (c)}
                  {#if r.restarts[c] > 0}
                    <div class="seg-bar" style="width: {(100 * r.restarts[c]) / total}%; background: {classColor(c)}" title="{classLabel(c)}: {r.restarts[c]}"></div>
                  {/if}
                {/each}
              </div>
              <div class="legend">
                {#each CLASSES as c (c)}
                  {#if r.restarts[c] > 0}
                    <span class="lg"><i style="background: {classColor(c)}"></i>{classLabel(c)} {r.restarts[c]}</span>
                  {/if}
                {/each}
              </div>
            {/if}

            {#if r.drift.length}
              <div class="drift" title={r.drift.map((d) => `${d.workload}: ${d.versions.join(', ')}`).join('\n')}>
                <Icon name="branch" size={11} /> {r.drift.length} workload{r.drift.length === 1 ? '' : 's'} running mixed versions
              </div>
            {/if}

            <div class="status dim" title={r.status?.last_error || ''}>{collectorLine(r.status, r.enabled)}</div>
            {#if rbac}
              <div class="rbac">
                <span>metrics-server blocked by RBAC — ask your cluster admin to grant it:</span>
                <code>{rbac}</code>
                <button class="btn small" onclick={(e) => { e.stopPropagation(); void copy(rbac); }}>Copy</button>
              </div>
            {:else if r.status?.metrics_server === 'ok'}
              <div class="dim tiny">metrics-server: CPU + memory per container available</div>
            {:else if r.status?.metrics_server === 'disabled'}
              <div class="dim tiny">metrics-server probing is off (Settings)</div>
            {/if}
          {/if}
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
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
    margin: 0 4px;
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
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(320px, 1fr));
    gap: 12px;
  }
  .cluster {
    padding: 14px 16px;
    display: flex;
    flex-direction: column;
    gap: 10px;
    cursor: pointer;
    transition: border-color 130ms ease-out, background 130ms ease-out;
  }
  .cluster:hover,
  .cluster:focus-visible {
    border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
    background: color-mix(in srgb, var(--accent) 4%, var(--surface));
  }
  .cluster.off {
    opacity: 0.85;
  }
  .row1 {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .dot {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  .name {
    font-weight: 600;
    font-size: 13.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
    min-width: 0;
  }
  .env-badge {
    flex-shrink: 0;
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
  .health {
    flex-shrink: 0;
    font-size: 10.5px;
    font-weight: 600;
    padding: 2px 8px;
    border-radius: 999px;
    border: 1px solid var(--border);
    color: var(--text-dim);
  }
  .health.ok {
    color: var(--status-working);
    border-color: color-mix(in srgb, var(--status-working) 40%, transparent);
  }
  .health.warn {
    color: orange;
    border-color: color-mix(in srgb, orange 40%, transparent);
  }
  .health.bad {
    color: var(--status-exited);
    border-color: color-mix(in srgb, var(--status-exited) 40%, transparent);
    background: color-mix(in srgb, var(--status-exited) 10%, transparent);
  }
  .off-body {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    font-size: 12px;
  }
  .stats {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 8px 14px;
  }
  .stat {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .k {
    font-size: 10.5px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
  }
  .v {
    font-size: 15px;
    font-weight: 600;
  }
  .note {
    font-size: 11px;
    color: var(--text-dim);
  }
  .note.bad {
    color: var(--status-exited);
  }
  .bar {
    height: 4px;
    border-radius: 2px;
    background: var(--surface-2);
    overflow: hidden;
  }
  .fill {
    height: 100%;
    background: var(--accent);
  }
  .fill.warn {
    background: var(--status-exited);
  }
  .stack {
    display: flex;
    height: 6px;
    border-radius: 3px;
    overflow: hidden;
    background: var(--surface-2);
  }
  .legend {
    display: flex;
    gap: 10px;
    flex-wrap: wrap;
    font-size: 11px;
    color: var(--text-dim);
  }
  .lg i {
    display: inline-block;
    width: 8px;
    height: 8px;
    border-radius: 2px;
    margin-right: 4px;
    vertical-align: middle;
  }
  .drift {
    display: flex;
    align-items: center;
    gap: 5px;
    font-size: 11.5px;
    color: orange;
  }
  .status {
    font-size: 11px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .rbac {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 11px;
    padding: 8px;
    border-radius: var(--radius-s, 6px);
    background: color-mix(in srgb, orange 8%, var(--surface));
    border: 1px solid color-mix(in srgb, orange 30%, var(--border));
  }
  .rbac code {
    font-family: var(--font-mono);
    font-size: 10.5px;
    white-space: pre-wrap;
    word-break: break-word;
    user-select: all;
  }
  .rbac .btn {
    align-self: flex-start;
  }
  .dim {
    color: var(--text-dim);
  }
  .tiny {
    font-size: 11px;
  }
  .mono {
    font-family: var(--font-mono);
  }
  @media (max-width: 640px) {
    .stats {
      grid-template-columns: 1fr;
    }
  }
</style>
