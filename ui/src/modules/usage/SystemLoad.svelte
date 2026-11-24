<script lang="ts">
  // System load (root): host CPU and memory, the daemon's own memory and the
  // active sessions. Mounted only while the details card's "System load" is
  // open: it fetches the last 180 minutes once and follows the
  // `usage_metrics_tick` event (throttled) — never on page open.
  import { onMount } from 'svelte';
  import { formatCount } from '../../lib/metric-format';
  import { pluralNoun } from '../../lib/plural';
  import { usage } from '../../lib/api/usage.svelte';

  let { intervalSecs }: { intervalSecs: number } = $props();

  onMount(() => {
    const unwatch = usage.watchMetrics();
    void usage.loadMetrics();
    return unwatch;
  });

  const latest = $derived(usage.metrics.at(-1) ?? null);
  const cpuSeries = $derived(usage.metrics.map((p) => p.cpu_pct));
  const memSeries = $derived(usage.metrics.map((p) => p.mem_pct));
  function sparkPath(values: number[], max: number, w: number, h: number): string {
    if (values.length === 0) return '';
    const m = Math.max(max, 1);
    const step = values.length > 1 ? w / (values.length - 1) : 0;
    return values
      .map((v, i) => `${i === 0 ? 'M' : 'L'}${(i * step).toFixed(1)},${(h - (v / m) * h).toFixed(1)}`)
      .join(' ');
  }
</script>

<div class="sys" data-testid="usage-system-load">
  {#if latest}
    <p class="dc-note now">
      CPU {latest.cpu_pct.toFixed(0)} % · memory {latest.mem_pct.toFixed(0)} % ({formatCount(Math.round(latest.mem_used_mb))} of {formatCount(Math.round(latest.mem_total_mb))} MB) ·
      Otto daemon {latest.process_rss_mb.toFixed(0)} MB · {formatCount(latest.active_sessions)} active {pluralNoun(latest.active_sessions, 'session', 'sessions')}
    </p>
  {/if}
  {#if usage.metrics.length > 1}
    <div class="metrics">
      <div class="metric">
        <span class="metric-label">CPU %</span>
        <svg viewBox="0 0 300 48" preserveAspectRatio="none" class="spark" role="img" aria-label="CPU over the last {usage.metrics.length} samples"><path d={sparkPath(cpuSeries, 100, 300, 48)} /></svg>
      </div>
      <div class="metric">
        <span class="metric-label">Memory %</span>
        <svg viewBox="0 0 300 48" preserveAspectRatio="none" class="spark" role="img" aria-label="Memory over the last {usage.metrics.length} samples"><path d={sparkPath(memSeries, 100, 300, 48)} /></svg>
      </div>
    </div>
    <p class="dc-note">Last {usage.metrics.length} samples, one every {intervalSecs} s</p>
  {:else}
    <p class="dc-note">Collecting samples… one every {intervalSecs} s.</p>
  {/if}
</div>

<style>
  .sys {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
  }
  .now {
    font-variant-numeric: tabular-nums;
  }
  .metrics {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: var(--sp-4);
    padding-inline: var(--sp-6);
  }
  .metric {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
  }
  .metric-label {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .spark {
    inline-size: 100%;
    block-size: 40px;
  }
  /* Both series are labelled; colour isn't carrying meaning here. */
  .spark path {
    fill: none;
    stroke: var(--info);
    stroke-width: 1.5;
    vector-effect: non-scaling-stroke;
  }
</style>
