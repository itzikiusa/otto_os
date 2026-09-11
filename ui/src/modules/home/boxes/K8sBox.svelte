<script lang="ts">
  // Kubernetes box: one row per registered cluster from the Monitor overview
  // (health, pods, restarts, memory, rps, error %). Falls back to the plain
  // cluster list (name + env + version) when the monitor is not collecting
  // (ClickHouse off / monitoring disabled) so the box is never blank for a
  // user who has clusters but no metrics.
  import { untrack } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Skeleton from '../../../lib/components/Skeleton.svelte';
  import { k8s } from '../../../lib/stores/k8s.svelte';
  import { k8sApi } from '../../../lib/api/k8s';
  import { router } from '../../../lib/router.svelte';
  import type { K8sMonitorOverviewRow } from '../../../lib/api/types';
  import { envBadge, formatBytes } from '../../kubernetes/k8s-util';
  import { WINDOWS, fmtPct, fmtRate, healthLabel, isWindow } from '../../kubernetes/monitor/monitor-util';
  import { home, type HomeBox } from '../home.svelte';
  import { poll, type Poller } from './poll';

  interface Props {
    box: HomeBox;
    viewId: string;
    zoomed: boolean;
    tick: number;
  }
  let { box, viewId, zoomed: _zoomed, tick }: Props = $props();

  const win = $derived(isWindow(String(box.config.window ?? '')) ? (box.config.window as (typeof WINDOWS)[number]) : '24h');

  let rows = $state<K8sMonitorOverviewRow[]>([]);
  let loading = $state(true);
  let error = $state('');
  let booted = false;

  async function load(): Promise<boolean> {
    if (!booted) {
      booted = true;
      await k8s.loadStatus();
      await k8s.loadClusters();
    }
    if (k8s.unavailable) {
      loading = false;
      return true;
    }
    try {
      rows = await k8sApi.monitorOverview(win);
      error = '';
      return true;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
      return false;
    } finally {
      loading = false;
    }
  }

  let poller: Poller | null = null;
  $effect(() => {
    void win;
    poller?.stop();
    poller = poll(load, 60_000);
    return () => poller?.stop();
  });
  $effect(() => {
    void tick;
    const t = k8s.monitorTick;
    void t;
    untrack(() => poller?.now());
  });

  // Overview rows keyed by cluster id; clusters with no row fall back to the
  // registry entry so every saved cluster shows.
  const byId = $derived(new Map(rows.map((r) => [r.cluster.id, r])));
  const monitored = $derived(rows.some((r) => r.enabled && r.status));
</script>

<div class="k8s">
  <div class="bar">
    <span class="dim">{k8s.clusters.length} cluster{k8s.clusters.length === 1 ? '' : 's'}</span>
    <span class="spacer"></span>
    <div class="seg" role="radiogroup" aria-label="Window">
      {#each WINDOWS as w (w)}
        <button class:on={w === win} role="radio" aria-checked={w === win} onclick={() => home.updateBoxConfig(viewId, box.id, { window: w })}>{w}</button>
      {/each}
    </div>
  </div>
  {#if loading && k8s.clusters.length === 0}
    <Skeleton rows={3} />
  {:else if k8s.unavailable}
    <EmptyState icon="helm" title="Kubernetes console is off" body="Enable it in the daemon to see clusters here." />
  {:else if k8s.clusters.length === 0}
    <EmptyState icon="helm" title="No clusters yet" body="Add a kubeconfig context on the Kubernetes page." actionLabel="Open Kubernetes" onaction={() => router.go('kubernetes')} />
  {:else}
    {#if error && !monitored}
      <div class="note" title={error}><Icon name="zap" size={10} />monitor data unavailable — showing the registry</div>
    {/if}
    <ul class="rows">
      {#each k8s.clusters as c (c.id)}
        {@const r = byId.get(c.id)}
        {@const h = healthLabel(r?.health ?? 'unknown')}
        <li>
          <button class="row" onclick={() => router.go(`kubernetes/${encodeURIComponent(c.id)}`)} title="Open {c.name}">
            <span class="cdot" style:background={c.color ?? 'var(--text-dim)'}></span>
            <span class="name ellipsis">{c.name}</span>
            <span class="env">{envBadge(c.environment)}</span>
            <span class="health {h.cls}">{h.label}</span>
            {#if r && r.enabled && r.status}
              <span class="m" title="Pods running / total">
                <b>{r.pods.running}</b>/{r.pods.total}
                {#if r.pods.crashloop > 0}<em class="bad">{r.pods.crashloop} crash</em>{/if}
                {#if r.pods.pending > 0}<em class="warn">{r.pods.pending} pend</em>{/if}
              </span>
              <span class="m" title="Memory used vs limits">
                <span class="meter"><i style:width="{Math.min(100, r.mem.pct)}%" class:warn={r.mem.pct >= 80} class:bad={r.mem.pct >= 95}></i></span>
                {fmtPct(r.mem.pct, 0)} · {formatBytes(r.mem.used)}
              </span>
              <span class="m" title="Requests / errors">{fmtRate(r.rps)} rps · <span class:bad={r.err_pct >= 5} class:warn={r.err_pct >= 1}>{fmtPct(r.err_pct)} err</span></span>
              {#if r.drift.length > 0}<span class="m warn" title="Version drift">{r.drift.length} drift</span>{/if}
            {:else}
              <span class="m dim">{c.capabilities?.server_version ?? c.context_name}</span>
            {/if}
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .k8s {
    display: flex;
    flex-direction: column;
    gap: 6px;
    height: 100%;
    min-height: 0;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 11px;
  }
  .spacer {
    flex: 1;
  }
  .dim {
    color: var(--text-dim);
  }
  .seg {
    display: inline-flex;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    overflow: hidden;
  }
  .seg button {
    border: none;
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    font-size: 10.5px;
    padding: 2px 7px;
    cursor: pointer;
  }
  .seg button.on {
    background: color-mix(in srgb, var(--accent) 18%, transparent);
    color: var(--accent);
  }
  .note {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    align-self: flex-start;
    padding: 1px 7px;
    font-size: 10px;
    color: var(--status-warn);
    background: var(--status-warn-soft);
    border-radius: 999px;
  }
  .rows {
    list-style: none;
    margin: 0;
    padding: 0;
    overflow-y: auto;
    min-height: 0;
    flex: 1;
  }
  .row {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
    width: 100%;
    padding: 6px 8px;
    border: none;
    background: transparent;
    color: var(--text);
    border-radius: var(--radius-s);
    cursor: pointer;
    text-align: start;
    font: inherit;
    font-size: 12px;
  }
  .row:hover {
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
  }
  .cdot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex: none;
  }
  .name {
    font-weight: 600;
    min-width: 0;
    max-width: 40%;
  }
  .env {
    font-size: 9.5px;
    font-weight: 700;
    letter-spacing: 0.04em;
    padding: 0 5px;
    border: 1px solid var(--border);
    border-radius: 4px;
    color: var(--text-dim);
  }
  .health {
    font-size: 10.5px;
    padding: 0 7px;
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .health.ok {
    color: var(--status-working);
    background: color-mix(in srgb, var(--status-working) 16%, transparent);
  }
  .health.warn {
    color: var(--status-warn);
    background: var(--status-warn-soft);
  }
  .health.bad {
    color: var(--status-exited);
    background: color-mix(in srgb, var(--status-exited) 16%, transparent);
  }
  .m {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 11px;
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .m b {
    color: var(--text);
  }
  .m em {
    font-style: normal;
    font-size: 10px;
    padding: 0 5px;
    border-radius: 999px;
  }
  .warn {
    color: var(--status-warn);
  }
  em.warn {
    background: var(--status-warn-soft);
  }
  .bad {
    color: var(--status-exited);
  }
  em.bad {
    background: color-mix(in srgb, var(--status-exited) 16%, transparent);
  }
  .meter {
    width: 46px;
    height: 5px;
    border-radius: 999px;
    background: var(--surface-2);
    overflow: hidden;
  }
  .meter i {
    display: block;
    height: 100%;
    background: var(--accent);
  }
  .meter i.warn {
    background: var(--status-warn);
  }
  .meter i.bad {
    background: var(--status-exited);
  }
  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
