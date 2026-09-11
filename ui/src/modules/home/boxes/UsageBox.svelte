<script lang="ts">
  // Usage box: spend / tokens / events for the last N days (box config, default
  // 7), a per-day cost sparkline and the per-provider split. Reads the same
  // /usage endpoints the Usage page does, scoped to this box so it never
  // disturbs the Usage page's own window selection.
  import { untrack } from 'svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Skeleton from '../../../lib/components/Skeleton.svelte';
  import ProviderIcon from '../../../lib/components/ProviderIcon.svelte';
  import { api } from '../../../lib/api/client';
  import { router } from '../../../lib/router.svelte';
  import type { UsageStatus, UsageSummary } from '../../../lib/api/usage.svelte';
  import { home, type HomeBox } from '../home.svelte';
  import { poll, type Poller } from './poll';

  interface Props {
    box: HomeBox;
    viewId: string;
    zoomed: boolean;
    tick: number;
  }
  let { box, viewId, zoomed: _zoomed, tick }: Props = $props();

  const DAYS = [1, 7, 30] as const;
  const days = $derived(DAYS.includes(Number(box.config.days) as (typeof DAYS)[number]) ? Number(box.config.days) : 7);

  let status = $state<UsageStatus | null>(null);
  let summary = $state<UsageSummary | null>(null);
  let loading = $state(true);
  let error = $state('');

  async function load(): Promise<boolean> {
    try {
      status = await api.get<UsageStatus>('/usage/status');
      if (status.available) {
        summary = await api.get<UsageSummary>(`/usage/summary?days=${days}&otto_only=false`);
      } else {
        summary = null;
      }
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
    void days;
    poller?.stop();
    poller = poll(load, 60_000);
    return () => poller?.stop();
  });
  $effect(() => {
    void tick;
    untrack(() => poller?.now());
  });

  const fmtUsd = (n: number): string => (n >= 100 ? `$${Math.round(n)}` : `$${n.toFixed(2)}`);
  const fmtK = (n: number): string => (n >= 1e9 ? `${(n / 1e9).toFixed(1)}B` : n >= 1e6 ? `${(n / 1e6).toFixed(1)}M` : n >= 1e3 ? `${(n / 1e3).toFixed(1)}K` : String(n));

  // Sparkline path over daily cost (oldest → newest), normalized to the box.
  const spark = $derived.by(() => {
    const d = summary?.daily ?? [];
    if (d.length < 2) return '';
    const sorted = [...d].sort((a, b) => a.day.localeCompare(b.day));
    const max = Math.max(...sorted.map((x) => x.cost_usd), 0.0001);
    const W = 100;
    const H = 28;
    return sorted
      .map((x, i) => `${i === 0 ? 'M' : 'L'}${((i / (sorted.length - 1)) * W).toFixed(1)},${(H - (x.cost_usd / max) * (H - 2) - 1).toFixed(1)}`)
      .join(' ');
  });
  const providers = $derived([...(summary?.providers ?? [])].sort((a, b) => b.cost_usd - a.cost_usd));
  const maxProv = $derived(Math.max(...providers.map((p) => p.cost_usd), 0.0001));
</script>

<div class="usage">
  <div class="bar">
    <span class="dim">last {days === 1 ? 'day' : `${days} days`}</span>
    <span class="spacer"></span>
    <div class="seg" role="radiogroup" aria-label="Window">
      {#each DAYS as d (d)}
        <button class:on={d === days} role="radio" aria-checked={d === days} onclick={() => home.updateBoxConfig(viewId, box.id, { days: d })}>{d}d</button>
      {/each}
    </div>
  </div>
  {#if loading && !summary}
    <Skeleton rows={3} />
  {:else if error && !summary}
    <EmptyState icon="chart" title="Usage unavailable" body={error} />
  {:else if status && !status.available}
    <EmptyState icon="chart" title="Usage engine is off" body="Enable the embedded ClickHouse engine on the Usage page." actionLabel="Open Usage" onaction={() => router.go('usage')} />
  {:else if summary}
    <div class="stats">
      <div class="stat"><span class="n">{fmtUsd(summary.total_cost_usd)}</span><span class="l">spend</span></div>
      <div class="stat"><span class="n">{fmtK(summary.total_tokens)}</span><span class="l">tokens</span></div>
      <div class="stat"><span class="n">{fmtK(summary.total_output_tokens)}</span><span class="l">output</span></div>
      <div class="stat"><span class="n">{fmtK(summary.total_events)}</span><span class="l">events</span></div>
    </div>
    {#if spark}
      <svg class="spark" viewBox="0 0 100 28" preserveAspectRatio="none" aria-label="Daily spend">
        <path d={spark} fill="none" stroke="var(--accent)" stroke-width="1.5" vector-effect="non-scaling-stroke" />
      </svg>
    {/if}
    <ul class="provs">
      {#each providers as p (p.provider)}
        <li>
          <ProviderIcon provider={p.provider} size={12} />
          <span class="pn ellipsis">{p.provider}</span>
          <span class="meter"><i style:width="{(p.cost_usd / maxProv) * 100}%"></i></span>
          <span class="pc">{fmtUsd(p.cost_usd)}</span>
        </li>
      {:else}
        <li class="dim">No usage recorded in this window.</li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .usage {
    display: flex;
    flex-direction: column;
    gap: 8px;
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
  .stats {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 6px;
  }
  .stat {
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 6px 4px;
    background: var(--surface-2);
    border-radius: var(--radius-s);
  }
  .n {
    font-size: 18px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    line-height: 1.1;
  }
  .l {
    font-size: 10px;
    color: var(--text-dim);
  }
  .spark {
    width: 100%;
    height: 34px;
    flex: none;
  }
  .provs {
    list-style: none;
    margin: 0;
    padding: 0;
    overflow-y: auto;
    min-height: 0;
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 12px;
  }
  .provs li {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 2px 4px;
  }
  .pn {
    min-width: 0;
    width: 90px;
  }
  .meter {
    flex: 1;
    height: 6px;
    border-radius: 999px;
    background: var(--surface-2);
    overflow: hidden;
  }
  .meter i {
    display: block;
    height: 100%;
    background: var(--accent);
  }
  .pc {
    min-width: 54px;
    text-align: end;
    font-variant-numeric: tabular-nums;
  }
  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
