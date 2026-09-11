<script lang="ts">
  // Insights box: the newest report of each kind (daily / weekly / monthly /
  // ad-hoc) with its plain-text summary. Reports are generated on a schedule
  // (or on demand from the Insights page), so a 5-minute cadence is plenty.
  import { untrack } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Skeleton from '../../../lib/components/Skeleton.svelte';
  import { insightsApi } from '../../../lib/api/insights';
  import { router } from '../../../lib/router.svelte';
  import type { InsightReport } from '../../../lib/api/types';
  import type { HomeBox } from '../home.svelte';
  import { poll, type Poller } from './poll';

  interface Props {
    box: HomeBox;
    zoomed: boolean;
    tick: number;
  }
  let { box: _box, zoomed, tick }: Props = $props();

  let reports = $state<InsightReport[]>([]);
  let loading = $state(true);
  let error = $state('');

  async function load(): Promise<boolean> {
    try {
      reports = await insightsApi.listReports();
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
    poller = poll(load, 300_000);
    return () => poller?.stop();
  });
  $effect(() => {
    void tick;
    untrack(() => poller?.now());
  });

  const KIND_LABEL: Record<string, string> = { daily: 'Daily', weekly: 'Weekly', monthly: 'Monthly', adhoc: 'Ad-hoc' };

  // Newest per kind (the list arrives newest-first); zoomed shows everything.
  const shown = $derived.by(() => {
    if (zoomed) return reports;
    const seen = new Set<string>();
    const out: InsightReport[] = [];
    for (const r of reports) {
      if (seen.has(r.kind)) continue;
      seen.add(r.kind);
      out.push(r);
    }
    return out;
  });

  function period(r: InsightReport): string {
    const s = new Date(r.period_start);
    const e = new Date(r.period_end);
    const f = (d: Date): string => d.toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
    return r.kind === 'daily' ? f(s) : `${f(s)} – ${f(e)}`;
  }
</script>

<div class="ins">
  {#if loading && reports.length === 0}
    <Skeleton rows={3} />
  {:else if error && reports.length === 0}
    <EmptyState icon="gauge" title="Insights unavailable" body={error} />
  {:else if reports.length === 0}
    <EmptyState icon="gauge" title="No reports yet" body="Turn on daily / weekly reports or run one now." actionLabel="Open Insights" onaction={() => router.go('insights')} />
  {:else}
    <ul class="rows">
      {#each shown as r (r.html_path)}
        <li>
          <button class="card" onclick={() => router.go('insights')} title="Open in Insights">
            <div class="head">
              <span class="kind">{KIND_LABEL[r.kind] ?? r.kind}</span>
              <span class="per">{period(r)}</span>
              <Icon name="external" size={11} />
            </div>
            <p class="sum" class:clamp={!zoomed}>{r.summary || 'No summary.'}</p>
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .ins {
    height: 100%;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .rows {
    list-style: none;
    margin: 0;
    padding: 0;
    overflow-y: auto;
    min-height: 0;
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .card {
    display: block;
    width: 100%;
    padding: 8px 10px;
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text);
    border-radius: var(--radius-s);
    cursor: pointer;
    text-align: start;
    font: inherit;
  }
  .card:hover {
    border-color: color-mix(in srgb, var(--accent) 50%, var(--border));
  }
  .head {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 11px;
    color: var(--text-dim);
  }
  .kind {
    font-weight: 700;
    color: var(--accent);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    font-size: 10px;
  }
  .per {
    flex: 1;
  }
  .sum {
    margin: 4px 0 0;
    font-size: 12px;
    line-height: 1.4;
    white-space: pre-line;
  }
  .sum.clamp {
    display: -webkit-box;
    -webkit-line-clamp: 4;
    line-clamp: 4;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
</style>
