<!--
  "‹ <parent title>" beside a child thread's title while the thread it was
  opened from is not on screen (a phone, or a child opened in place): one
  click back. The "Back to council" chip's pattern, for every child thread.
-->
<script lang="ts">
  import Icon from '../../../lib/components/Icon.svelte';
  import { threads } from '../threadsStore.svelte';
  import { childOf } from './childThreads';
  import { councilOf } from './council';
  import { runtimeThreads } from './runtimeThreads.svelte';

  let { threadId }: { threadId: string } = $props();

  const parent = $derived.by(() => {
    const me = runtimeThreads.get(threadId);
    const of = childOf(me);
    if (!of || councilOf(me)) return null;
    const p = runtimeThreads.get(of.id);
    return p && !threads.paneIds.includes(p.id) ? p : null;
  });
</script>

{#if parent}
  {@const id = parent.id}
  <button type="button" class="tp-back" title="Back to {parent.title}" data-testid="threads-back-to-parent" onclick={() => threads.selectThread(id)}>
    <Icon name="chevronLeft" size={12} /><span class="ellipsis" dir="auto">{parent.title}</span>
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
      min-block-size: var(--hit-min);
    }
  }
</style>
