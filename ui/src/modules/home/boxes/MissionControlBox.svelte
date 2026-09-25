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
  import { loadErrorText } from '../../../lib/loadError';
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

  // Request token: a workspace/limit change starts a new poller while the old
  // one's fetch may still be in flight — only the newest load may land.
  let seq = 0;

  async function load(): Promise<boolean> {
    const id = ws.currentId;
    if (!id) return true;
    const mine = ++seq;
    try {
      const [s, its] = await Promise.all([
        missionControlApi.summary(id),
        missionControlApi.items(id, { limit }),
      ]);
      if (mine !== seq) return true;
      summary = s;
      items = its;
      error = '';
      return true;
    } catch (e) {
      if (mine !== seq) return true;
      error = loadErrorText(e);
      return false;
    } finally {
      if (mine === seq) loading = false;
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
    <EmptyState icon="warning" title="Couldn't load Mission Control" body={error}>
      <button class="btn small" onclick={() => poller?.now()}><Icon name="refresh" size={12} />Retry</button>
    </EmptyState>
  {:else if summary}
    <div class="stats">
      <div class="stat"><span class="n" class:working={summary.active > 0}>{summary.active}</span><span class="l">active</span></div>
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
              <span class="ago" title={new Date(it.updated_at).toLocaleString()}>{now() && relTime(it.updated_at)}</span>
            </button>
          </li>
        {/each}
      </ul>
    {/if}
    <button class="foot" onclick={() => router.go('mission-control')}>
      Open Mission Control<Icon name="chevronRight" size={12} />
    </button>
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
  /* Widget figures: number over label, split by hairlines — no tile fills
     (the calm desktop-widget look shared by every Home box). */
  .stats {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 0;
  }
  .stat {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    min-width: 0;
    padding: 2px 12px;
    border-inline-start: 1px solid var(--separator);
  }
  .stat:first-child {
    padding-inline-start: 2px;
    border-inline-start: none;
  }
  .n {
    font-size: var(--fs-xl);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    line-height: 1.1;
  }
  /* Tone only when there is something to see: a green "0 working" was noise. */
  .n.working {
    color: var(--success);
  }
  .n.needs {
    color: var(--warning);
  }
  .l {
    font-size: var(--fs-xs);
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
    font-size: var(--fs-xs);
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
    font-size: var(--fs-s);
  }
  .row:hover {
    background: var(--hover);
  }
  .foot {
    flex: none;
    align-self: flex-start;
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding: 2px 0;
    border: none;
    background: none;
    color: var(--accent-text);
    font: inherit;
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .foot:hover {
    text-decoration: underline;
  }
  :global([dir='rtl']) .foot :global(svg) {
    transform: scaleX(-1);
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
    flex: none;
    font-size: var(--fs-xs);
  }
  .ago {
    color: var(--text-dim);
    font-size: var(--fs-xs);
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
