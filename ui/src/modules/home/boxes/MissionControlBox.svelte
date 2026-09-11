<script lang="ts">
  // Mission Control box: the work-graph header summary (active / needs approval
  // / total / spend) plus the most recently updated items. Reloads on every
  // live `work_graph_updated` tick and on a 30s cadence as a safety net.
  import { untrack } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Skeleton from '../../../lib/components/Skeleton.svelte';
  import { ws } from '../../../lib/stores/workspace.svelte';
  import { router } from '../../../lib/router.svelte';
  import { missionControlBus } from '../../../lib/events.svelte';
  import { missionControlApi } from '../../../lib/api/missionControl';
  import { now } from '../../../lib/stores/now.svelte';
  import type { MissionSummary, WorkItem } from '../../../lib/api/types';
  import { KIND_ICON, STATUS_LABEL, fmtCost, relTime, statusColor } from '../../mission-control/lib';
  import type { HomeBox } from '../home.svelte';
  import { poll, type Poller } from './poll';

  interface Props {
    box: HomeBox;
    zoomed: boolean;
    tick: number;
  }
  let { box, zoomed, tick }: Props = $props();

  let summary = $state<MissionSummary | null>(null);
  let items = $state<WorkItem[]>([]);
  let loading = $state(true);
  let error = $state('');

  const limit = $derived(zoomed ? 60 : Math.max(5, Number(box.config.limit) || 12));

  async function load(): Promise<boolean> {
    const id = ws.currentId;
    if (!id) return true;
    try {
      const [s, its] = await Promise.all([
        missionControlApi.summary(id),
        missionControlApi.items(id, { limit }),
      ]);
      summary = s;
      items = its;
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
    void ws.currentId;
    void limit;
    loading = true;
    poller?.stop();
    poller = poll(load, 30_000);
    return () => poller?.stop();
  });
  // Manual refresh (frame button) + live work-graph ticks: rerun immediately.
  $effect(() => {
    void tick;
    void missionControlBus.tick;
    untrack(() => poller?.now());
  });

  const active = $derived(items.filter((i) => i.status === 'running' || i.status === 'waiting' || i.status === 'blocked'));
  const recent = $derived(items.filter((i) => !active.includes(i)));
</script>

<div class="mc">
  {#if loading && !summary}
    <Skeleton rows={4} />
  {:else if error && !summary}
    <EmptyState icon="radar" title="Mission Control unavailable" body={error} />
  {:else if summary}
    <div class="stats">
      <div class="stat"><span class="n working">{summary.active}</span><span class="l">active</span></div>
      <div class="stat"><span class="n" class:needs={summary.needs_approval > 0}>{summary.needs_approval}</span><span class="l">need approval</span></div>
      <div class="stat"><span class="n">{summary.total}</span><span class="l">items</span></div>
      <div class="stat"><span class="n">{fmtCost(summary.total_cost)}</span><span class="l">spend</span></div>
    </div>
    <div class="chips">
      {#each summary.by_status as b (b.key)}
        {#if b.count > 0}
          <span class="chip" style:--c={statusColor(b.key as WorkItem['status'])}>
            <i></i>{STATUS_LABEL[b.key as WorkItem['status']] ?? b.key} {b.count}
          </span>
        {/if}
      {/each}
    </div>
    {#if items.length === 0}
      <EmptyState icon="radar" title="Nothing in flight" body="Sessions, reviews, loops and PRs show up here as they run." />
    {:else}
      <ul class="rows">
        {#each [...active, ...recent] as it (it.id)}
          <li>
            <button class="row" onclick={() => router.go(`mission-control/${it.id}`)} title={it.title}>
              <span class="dot" style:background={statusColor(it.status)}></span>
              <Icon name={KIND_ICON[it.kind]} size={12} />
              <span class="title ellipsis">{it.title}</span>
              <span class="st" style:color={statusColor(it.status)}>{STATUS_LABEL[it.status]}</span>
              <span class="ago">{now() && relTime(it.updated_at)}</span>
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  {/if}
</div>

<style>
  .mc {
    display: flex;
    flex-direction: column;
    gap: 8px;
    height: 100%;
    min-height: 0;
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
  .n.working {
    color: var(--status-working);
  }
  .n.needs {
    color: var(--status-warn);
  }
  .l {
    font-size: 10px;
    color: var(--text-dim);
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 1px 7px;
    font-size: 10.5px;
    border: 1px solid var(--border);
    border-radius: 999px;
    color: var(--text-dim);
  }
  .chip i {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--c);
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
    gap: 8px;
    width: 100%;
    padding: 5px 8px;
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
  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex: none;
  }
  .title {
    flex: 1;
    min-width: 0;
  }
  .st {
    font-size: 10.5px;
  }
  .ago {
    color: var(--text-dim);
    font-size: 11px;
    font-variant-numeric: tabular-nums;
    min-width: 28px;
    text-align: end;
  }
  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
