<script lang="ts">
  // Workflows & Goal Loops box: combines live `ws.activeWorkflowRuns` with
  // workspace Goal Loops (`/workspaces/{id}/goal-loops`) and Workflows
  // (`/workspaces/{id}/workflows`) via `livePoll` (respecting `active` & `tick`).
  import { untrack } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import Badge from '../../../lib/components/Badge.svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import { api } from '../../../lib/api/client';
  import { loadErrorText } from '../../../lib/loadError';
  import { router } from '../../../lib/router.svelte';
  import { rel } from '../../../lib/stores/now.svelte';
  import { ws } from '../../../lib/stores/workspace.svelte';
  import type { GoalLoop, Workflow } from '../../../lib/api/types';
  import type { HomeBox } from '../home.svelte';
  import { freshness, livePoll, type Poller } from './poll';
  import { DEFAULT_BOX_ROW_LIMIT, summarizeAutomations } from './homeBoxesData';

  interface Props {
    box: HomeBox;
    zoomed: boolean;
    tick: number;
    active?: boolean;
  }
  let { box: _box, zoomed, tick, active = true }: Props = $props();

  const POLL_MS = 60_000;

  let loops = $state<GoalLoop[]>([]);
  let workflows = $state<Workflow[]>([]);
  let loading = $state(true);
  let loaded = $state(false);
  let error = $state('');

  let seq = 0;

  async function load(signal?: AbortSignal): Promise<boolean> {
    const mine = ++seq;
    const wsId = ws.currentId;
    if (!wsId) {
      if (ws.listSettled) {
        loops = [];
        workflows = [];
        loaded = true;
        loading = false;
        error = '';
      }
      return true;
    }
    try {
      const encoded = encodeURIComponent(wsId);
      const [loopsRes, workflowsRes] = await Promise.allSettled([
        api.bg.get<GoalLoop[]>(`/workspaces/${encoded}/goal-loops`, signal),
        api.bg.get<Workflow[]>(`/workspaces/${encoded}/workflows`, signal),
      ]);
      if (mine !== seq || signal?.aborted) return true;

      let failed = 0;
      let firstErr: unknown = null;
      if (loopsRes.status === 'fulfilled') loops = loopsRes.value;
      else {
        failed += 1;
        firstErr = loopsRes.reason;
      }
      if (workflowsRes.status === 'fulfilled') workflows = workflowsRes.value;
      else {
        failed += 1;
        if (!firstErr) firstErr = workflowsRes.reason;
      }

      if (failed < 2) loaded = true;
      if (failed === 2) {
        error = loadErrorText(firstErr);
        return false;
      }
      if (failed === 1) {
        const which = loopsRes.status === 'rejected' ? 'goal loops' : 'workflows';
        error = `Couldn’t load ${which}: ${loadErrorText(firstErr)}`;
        return false;
      }
      error = '';
      return true;
    } catch (e) {
      if (mine !== seq || signal?.aborted) return true;
      error = loadErrorText(e);
      return false;
    } finally {
      if (mine === seq) loading = false;
    }
  }

  let poller: Poller | null = null;
  const fresh = freshness(load);
  let lastWs: string | null | undefined;
  $effect(() => {
    const wsKey = ws.listSettled ? `ws:${ws.currentId ?? ''}` : 'pending';
    if (lastWs !== undefined && lastWs !== wsKey) {
      untrack(() => {
        seq += 1;
        loops = [];
        workflows = [];
        loaded = false;
        loading = true;
        error = '';
      });
    }
    lastWs = wsKey;
    if (!active) return;
    poller?.stop();
    poller = livePoll(fresh.run, POLL_MS, ['workflow_run_updated', 'goal_loop_updated'], {
      immediate: fresh.start(wsKey, POLL_MS),
    });
    return () => poller?.stop();
  });

  // Header refresh button only — avoid queuing a duplicate load on mount.
  let seenTick = untrack(() => tick);
  $effect(() => {
    const t = tick;
    if (t === seenTick) return;
    seenTick = t;
    untrack(() => poller?.now());
  });

  const summary = $derived(
    summarizeAutomations({
      activeRuns: ws.activeWorkflowRuns,
      loops,
      workflows,
      limit: zoomed ? 30 : DEFAULT_BOX_ROW_LIMIT,
    }),
  );

  const hasData = $derived(loaded || summary.rows.length > 0);
  const unknown = $derived(!loaded && summary.rows.length === 0 && (loading || !!error));
  const statText = (n: number): string => (unknown ? '—' : String(n));
</script>

<div class="auto">
  <div class="stats">
    <div class="stat">
      <span class="n" class:info={!unknown && summary.stats.running > 0}>{statText(summary.stats.running)}</span>
      <span class="l">running</span>
    </div>
    <div class="stat">
      <span class="n">{statText(summary.stats.loops)}</span>
      <span class="l">loops</span>
    </div>
    <div class="stat">
      <span class="n">{statText(summary.stats.workflows)}</span>
      <span class="l">workflows</span>
    </div>
  </div>

  <LoadState
    what="workflows and loops"
    variant="compact"
    loading={loading && !hasData}
    error={error || null}
    empty={summary.rows.length === 0}
    onretry={() => poller?.now()}
  >
    {#snippet emptyView()}
      <EmptyState
        icon="split"
        title="No workflows or goal loops"
        body="Create a workflow or start a goal loop to automate repeatable work."
      >
        <button class="btn small" onclick={() => router.go('workflows')}>Open Workflows</button>
      </EmptyState>
    {/snippet}

    <ul class="rows">
      {#each summary.rows as row (row.id)}
        <li>
          <button class="arow" onclick={() => router.go(row.route)} title={row.title}>
            <span class="ricon">
              <Icon name={row.kind === 'goal-loop' ? 'target' : row.kind === 'workflow-run' ? 'play' : 'split'} size={12} />
            </span>
            <span class="meta">
              <span class="title ellipsis">{row.title}</span>
              <span class="sub ellipsis">{row.detail}</span>
            </span>
            <Badge tone={row.badgeTone} label={row.badgeLabel} live={row.live} />
            {#if row.at}
              <span class="ago" title={new Date(row.at).toLocaleString()}>{rel(row.at, { compact: true })}</span>
            {/if}
            <span class="go" aria-hidden="true"><Icon name="chevronRight" size={12} /></span>
          </button>
        </li>
      {/each}
    </ul>

    <div class="foot-bar">
      <div class="foot-links">
        <button class="foot" onclick={() => router.go('workflows')}>
          Workflows<Icon name="chevronRight" size={12} />
        </button>
        <button class="foot" onclick={() => router.go('loops')}>
          Goal Loops<Icon name="chevronRight" size={12} />
        </button>
      </div>
      {#if summary.truncatedCount > 0}
        <span class="more">+{summary.truncatedCount} more</span>
      {/if}
    </div>
  </LoadState>
</div>

<style>
  .auto {
    display: flex;
    flex-direction: column;
    gap: 8px;
    height: 100%;
    min-height: 0;
  }
  .stats {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
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
  .n.info {
    color: var(--info);
  }
  .l {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .rows {
    list-style: none;
    margin: 0;
    padding: 0;
    overflow-y: auto;
    min-height: 0;
    flex: 1;
  }
  .arow {
    display: flex;
    align-items: center;
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
  .arow:hover {
    background: var(--hover);
  }
  .ricon {
    display: inline-flex;
    color: var(--text-dim);
    flex: none;
  }
  .meta {
    display: flex;
    flex-direction: column;
    min-width: 0;
    flex: 1;
  }
  .title {
    font-size: var(--fs-s);
    font-weight: 500;
  }
  .sub {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .ago {
    color: var(--text-dim);
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
    flex: none;
  }
  .go {
    display: inline-flex;
    color: var(--text-dim);
    opacity: 0;
    transition: opacity var(--dur-fast) ease-out;
    flex: none;
  }
  .arow:hover .go,
  .arow:focus-visible .go {
    opacity: 1;
  }
  .foot-bar {
    flex: none;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .foot-links {
    display: inline-flex;
    align-items: center;
    gap: 12px;
  }
  .foot {
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
  .more {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
