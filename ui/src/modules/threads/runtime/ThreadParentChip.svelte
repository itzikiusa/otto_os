<!--
  In a subagent shown in the main view: the fixed "Subagent of <parent> ·
  Open parent" line with its task (RuntimeSubagentOf). A council's seats
  2–4 and its synthesis keep a plain "‹ <seat 1>" back chip.
-->
<script lang="ts">
  import Icon from '../../../lib/components/Icon.svelte';
  import { threads } from '../threadsStore.svelte';
  import { childOf } from './childThreads';
  import { councilOf } from './council';
  import { runtimeThreads } from './runtimeThreads.svelte';
  import RuntimeSubagentOf from './RuntimeSubagentOf.svelte';

  let { threadId }: { threadId: string } = $props();

  const me = $derived(runtimeThreads.get(threadId));
  const seatParent = $derived.by(() => {
    if (childOf(me) || !councilOf(me) || !me?.parent_id) return null;
    const p = runtimeThreads.get(me.parent_id);
    return p && !threads.paneIds.includes(p.id) ? p : null;
  });
</script>

{#if childOf(me)}
  <RuntimeSubagentOf {threadId} />
{:else if seatParent}
  {@const id = seatParent.id}
  <button type="button" class="tp-back" title="Back to {seatParent.title}" data-testid="threads-back-to-parent" onclick={() => threads.selectThread(id)}>
    <Icon name="chevronLeft" size={12} /><span class="ellipsis" dir="auto">{seatParent.title}</span>
  </button>
{/if}

<style>
  .tp-back {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    flex: 0 1 auto;
    min-inline-size: 0;
    max-inline-size: 40%;
    block-size: 24px;
    padding-inline: 6px 8px;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .tp-back:hover {
    background: var(--hover);
    color: var(--text);
  }
  .tp-back:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 1px;
  }
  @media (pointer: coarse) {
    .tp-back {
      min-block-size: 44px;
    }
  }
</style>
