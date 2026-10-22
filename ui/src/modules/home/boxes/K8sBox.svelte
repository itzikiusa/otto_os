<script lang="ts">
  import { plural } from '../../../lib/plural';
  // Kubernetes box: one row per registered cluster from the Monitor overview
  // (health, pods, restarts, memory, rps, error %). Falls back to the plain
  // cluster list (name + env + version) when the monitor is not collecting
  // (ClickHouse off / monitoring disabled) so the box is never blank for a
  // user who has clusters but no metrics.
  import { untrack } from 'svelte';
  import { radioKey } from '../../../lib/radioKey';
  import Icon from '../../../lib/components/Icon.svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import Skeleton from '../../../lib/components/Skeleton.svelte';
  import { k8s } from '../../../lib/stores/k8s.svelte';
  import { k8sApi } from '../../../lib/api/k8s';
  import { router } from '../../../lib/router.svelte';
  import type { K8sMonitorOverviewRow } from '../../../lib/api/types';
  import { formatBytes } from '../../kubernetes/k8s-util';
  import EnvBadge from '../../../lib/components/EnvBadge.svelte';
  import { WINDOWS, fmtPct, fmtRate, healthLabel, isWindow } from '../../kubernetes/monitor/monitor-util';
  import { loadErrorText } from '../../../lib/loadError';
  import { home, type HomeBox } from '../home.svelte';
  import { freshness, poll, type Poller } from './poll';

  interface Props {
    box: HomeBox;
    viewId: string;
    zoomed: boolean;
    tick: number;
    /** False while this box's Home space is not on screen (or another box
     *  is zoomed): the poller stops, the data stays (review 06 F3). */
    active?: boolean;
  }
  let { box, viewId, zoomed: _zoomed, tick, active = true }: Props = $props();

  const win = $derived(isWindow(String(box.config.window ?? '')) ? (box.config.window as (typeof WINDOWS)[number]) : '24h');

  let rows = $state<K8sMonitorOverviewRow[]>([]);
  let loading = $state(true);
  let error = $state('');
  let booted = false;
  let seq = 0;

  async function load(signal?: AbortSignal): Promise<boolean> {
    // perf K8s R5: status, the cluster list and the overview start together
    // (they were three serial round-trips on mount); the overview is aborted
    // with the poller when the box unmounts or leaves the screen.
    const mine = ++seq;
    try {
      const boot = booted ? null : Promise.all([k8s.loadStatus(), k8s.loadClusters()]);
      booted = true;
      const overview = k8sApi.monitorOverview(win, signal);
      if (boot) await boot;
      if (k8s.unavailable) {
        overview.catch(() => undefined);
        return true;
      }
      const next = await overview;
      if (mine !== seq) return true;
      rows = next;
      error = '';
      return true;
    } catch (e) {
      // An aborted read (the space went off screen mid-request) loaded
      // nothing: `false` keeps `freshness` from stamping it fresh, so coming
      // back fetches at once instead of showing empty rows for a minute.
      if (signal?.aborted) return false;
      if (mine !== seq) return true;
      error = loadErrorText(e);
      return false;
    } finally {
      if (mine === seq) loading = false;
    }
  }

  let poller: Poller | null = null;
  const fresh = freshness(load);
  $effect(() => {
    const key = win;
    if (!active) return;
    poller?.stop();
    poller = poll(fresh.run, 60_000, fresh.start(key, 60_000));
    return () => poller?.stop();
  });
  // Manual refresh (frame button) runs interactive; a collection cycle
  // (coalesced by the store, never while hidden) on the background lane.
  // Only CHANGES count — the poller's own first run already covers mount.
  let seenTick = untrack(() => tick);
  let seenCycle = untrack(() => k8s.monitorTick);
  $effect(() => {
    const t = tick;
    if (t === seenTick) return;
    seenTick = t;
    untrack(() => poller?.now());
  });
  $effect(() => {
    const c = k8s.monitorTick;
    if (c === seenCycle) return;
    seenCycle = c;
    untrack(() => poller?.now({ background: true }));
  });

  // Overview rows keyed by cluster id; clusters with no row fall back to the
  // registry entry so every saved cluster shows.
  const byId = $derived(new Map(rows.map((r) => [r.cluster.id, r])));
  const monitored = $derived(rows.some((r) => r.enabled && r.status));
</script>

<div class="k8s">
  <!-- The count + time window only mean something once there are clusters;
       the empty / off states below stand alone. -->
  {#if k8s.clusters.length > 0 && !k8s.unavailable}
    <div class="bar">
      <span class="dim">{plural(k8s.clusters.length, 'cluster')}</span>
      <span class="spacer"></span>
      <div class="seg" role="radiogroup" aria-label="Window">
        {#each WINDOWS as w (w)}
          <button class:on={w === win} role="radio" aria-checked={w === win} tabindex={w === win ? 0 : -1} onkeydown={radioKey} onclick={() => home.updateBoxConfig(viewId, box.id, { window: w })}>{w}</button>
        {/each}
      </div>
    </div>
  {/if}
  {#if loading && k8s.clusters.length === 0}
    <Skeleton rows={3} />
  {:else if k8s.unavailable}
    <EmptyState icon="helm" title="Kubernetes console is off" body="Enable it in the daemon to see clusters here." />
  {:else if k8s.clusters.length === 0 && k8s.clustersError}
    <!-- A failed list must not read as "No clusters yet". -->
    <LoadState what="clusters" variant="compact" error={k8s.clustersError} empty={true} onretry={() => void k8s.loadClusters()} />
  {:else if k8s.clusters.length === 0}
    <EmptyState icon="helm" title="No clusters yet" body="Add a kubeconfig context on the Kubernetes page.">
      <button class="btn small" onclick={() => router.go('kubernetes')}>Open Kubernetes</button>
    </EmptyState>
  {:else}
    {#if error && !monitored}
      <LoadState what="Kubernetes monitor" variant="compact" {error} empty={false} onretry={() => poller?.now()}>
        <span class="dim">Showing the cluster registry.</span>
      </LoadState>
    {/if}
    <ul class="rows">
      {#each k8s.clusters as c (c.id)}
        {@const r = byId.get(c.id)}
        {@const h = healthLabel(r?.health ?? 'unknown')}
        <li>
          <button class="k8s-row" onclick={() => router.go(`kubernetes/${encodeURIComponent(c.id)}`)} title="Open {c.name}">
            <span class="cdot" style:background={c.color ?? 'var(--text-dim)'}></span>
            <span class="name ellipsis">{c.name}</span>
            <EnvBadge env={c.environment} />
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
    font-size: var(--fs-xs);
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
    font-size: var(--fs-xs);
    padding: 2px 6px;
    cursor: pointer;
  }
  .seg button.on {
    background: var(--accent-soft-strong);
    color: var(--accent-text);
  }
  .rows {
    list-style: none;
    margin: 0;
    padding: 0;
    overflow-y: auto;
    min-height: 0;
    flex: 1;
  }
  .k8s-row {
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
    font-size: var(--fs-s);
  }
  .k8s-row:hover {
    background: var(--hover);
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
  .health {
    font-size: var(--fs-xs);
    padding: 0 6px;
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .health.ok {
    color: var(--success);
    background: color-mix(in srgb, var(--status-working) 16%, transparent);
  }
  .health.warn {
    color: var(--warning);
    background: var(--status-warn-soft);
  }
  .health.bad {
    color: var(--danger);
    background: color-mix(in srgb, var(--status-exited) 16%, transparent);
  }
  .m {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .m b {
    color: var(--text);
  }
  .m em {
    font-style: normal;
    font-size: var(--fs-xs);
    padding: 0 4px;
    border-radius: 999px;
  }
  .warn {
    color: var(--warning);
  }
  em.warn {
    background: var(--status-warn-soft);
  }
  .bad {
    color: var(--danger);
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
