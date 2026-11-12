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
  import { runtimeThreads } from './runtimeThreads.svelte';

  type Hint = { provider?: string | null; title?: string | null; model?: string | null; effort?: string | null };
  interface Props {
    kids: { id: string; hint?: Hint }[];
  }
  let { kids }: Props = $props();

  let stopping = $state(false);
  const working = $derived(
    kids.filter((k) => {
      const st = runtimeThreads.get(k.id)?.status;
      return st === 'working' || st === 'starting' || st === 'awaiting_permission';
    }),
  );

  async function stopAll(): Promise<void> {
    if (stopping) return;
    stopping = true;
    const results = await Promise.allSettled(working.map((k) => runtimeThreads.interrupt(k.id)));
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
      <span class="rg-sub">{working.length > 0 ? `${working.length} working` : 'all done'}</span>
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
  .rg-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding-inline-start: 12px;
    border-inline-start: 2px solid var(--border);
  }
</style>
