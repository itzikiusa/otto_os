<script lang="ts">
  // "Next that needs you" — the one move to the thread waiting on you, at
  // the top of the thread list. Shown only while ANOTHER thread than the one
  // in front of you needs you; ⌥⌘N does the same from anywhere in Threads.
  import Icon from '../../lib/components/Icon.svelte';
  import { plural } from '../../lib/plural';
  import { nextNeedsYouId, openNextNeedsYou, THREAD_KEYS, threadNav, threadNeedsYou } from './threadNav.svelte';
  import { threads } from './threadsStore.svelte';

  let { onselect }: { onselect?: () => void } = $props();

  const target = $derived(nextNeedsYouId());
  const waiting = $derived(threadNav.order.filter((id) => id !== threads.focusedSessionId && threadNeedsYou(id)).length);
</script>

{#if target}
  <button
    type="button"
    class="nn"
    title="Open the next thread that needs you ({THREAD_KEYS.needsYou})"
    data-testid="threads-next-needs-you"
    onclick={() => {
      if (openNextNeedsYou()) onselect?.();
    }}
  >
    <Icon name="bell" size={12} />
    <span class="ellipsis">Next that needs you</span>
    {#if waiting > 1}<span class="nn-count">{plural(waiting, 'thread')}</span>{/if}
    <kbd class="nn-key">{THREAD_KEYS.needsYou}</kbd>
  </button>
{/if}

<style>
  .nn {
    display: flex;
    align-items: center;
    gap: 6px;
    min-inline-size: 0;
    min-block-size: 28px;
    padding-inline: 8px;
    border: 1px solid var(--warning);
    border-radius: var(--radius-m);
    background: var(--warning-soft);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-s);
    text-align: start;
    cursor: pointer;
  }
  .nn :global(svg) {
    flex-shrink: 0;
    color: var(--warning);
  }
  .nn:hover {
    background: var(--hover);
  }
  .nn-count {
    flex-shrink: 0;
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .nn-key {
    margin-inline-start: auto;
    flex-shrink: 0;
    font: inherit;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  @media (hover: none), (pointer: coarse) {
    .nn {
      min-block-size: 44px;
    }
    .nn-key {
      display: none;
    }
  }
</style>
