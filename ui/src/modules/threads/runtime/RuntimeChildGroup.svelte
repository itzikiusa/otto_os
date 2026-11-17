<!--
  Several threads started together from this one (Ask several, or the
  agent's parallel `otto.thread_open` calls) under one header: how many,
  how many still work, and Stop all — T3 Code's workflow row
  (ThreadLineageWorkflowRow.tsx, MIT, Copyright (c) 2026 T3 Tools Inc.:
  "N/M agents", "Stop entire workflow"). One child is just its card.
-->
<script lang="ts">
  import Icon from '../../../lib/components/Icon.svelte';
  import { toastError } from '../../../lib/toastError';
  import RuntimeChildCard from './RuntimeChildCard.svelte';
  import { childState } from './childThreads';
  import { runtimeThreads } from './runtimeThreads.svelte';
  import { runtimeViews } from './runtimeView.svelte';
  import { stopThreadWork } from './taskStops.svelte';
  import { childGroupSummary } from '../threadAttention';

  type Hint = { provider?: string | null; title?: string | null; model?: string | null; effort?: string | null };
  interface Props {
    kids: { id: string; hint?: Hint }[];
  }
  let { kids }: Props = $props();

  let stopping = $state(false);
  const turnRuns = (id: string): boolean => {
    const st = runtimeThreads.get(id)?.status;
    return st === 'working' || st === 'starting' || st === 'awaiting_permission';
  };
  // A child busy only on background work counts as working too (its Stop stops that work).
  const working = $derived(kids.filter((k) => turnRuns(k.id) || (runtimeThreads.get(k.id)?.background_tasks?.length ?? 0) > 0));
  // The header tallies the children's real states (the same words as their cards).
  const summary = $derived(childGroupSummary(kids.map((k) => childState(runtimeThreads.get(k.id), runtimeViews.projections[k.id] ?? null).tone)));

  async function stopAll(): Promise<void> {
    if (stopping) return;
    stopping = true;
    const results = await Promise.allSettled(
      working.map((k) => stopThreadWork(k.id, { turnRunning: turnRuns(k.id), taskIds: (runtimeThreads.get(k.id)?.background_tasks ?? []).map((b) => b.id) })),
    );
    stopping = false;
    const failed = results.find((r): r is PromiseRejectedResult => r.status === 'rejected');
    if (failed) toastError('Couldn’t stop every thread', failed.reason);
  }
</script>

{#if kids.length === 1}
  <RuntimeChildCard id={kids[0].id} hint={kids[0].hint} />
{:else if kids.length > 1}
  <section class="rg" aria-label="{kids.length} agents asked" data-testid="runtime-child-group">
    <header class="rg-head">
      <Icon name="people" size={13} />
      <span class="rg-title">{kids.length} agents</span>
      <span class="rg-sub" class:rg-warn={summary.tone === 'warning'} class:rg-bad={summary.tone === 'danger'} data-testid="runtime-child-group-summary">{summary.text}</span>
      {#if working.length > 0}
        <button type="button" class="btn small ghost" disabled={stopping} data-testid="runtime-child-stop-all" onclick={() => void stopAll()}>
          <Icon name="stop" size={12} />{stopping ? 'Stopping…' : 'Stop all'}
        </button>
      {/if}
    </header>
    <div class="rg-list">
      {#each kids as k (k.id)}
        <RuntimeChildCard id={k.id} hint={k.hint} />
      {/each}
    </div>
  </section>
{/if}

<style>
  .rg {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-inline-size: 0;
  }
  .rg-head {
    display: flex;
    align-items: center;
    gap: 6px;
    min-block-size: 28px;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .rg-title {
    color: var(--text);
    font-weight: 500;
  }
  .rg-sub {
    flex: 1;
    min-inline-size: 0;
    font-size: var(--fs-xs);
  }
  .rg-warn {
    color: var(--warning);
  }
  .rg-bad {
    color: var(--danger);
  }
  .rg-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding-inline-start: 12px;
    border-inline-start: 2px solid var(--border);
  }
</style>
