<script lang="ts">
  // A finished stretch of the work log under one quiet line that says how
  // long and what was done — "Worked for 2m 14s · ran 2 commands and edited
  // 3 files" — and opens to every call (T3 Code's collapsed work log). A stretch that is still running, or holds a failure or an approval
  // waiting on you, is never folded: it renders as is (workLog.ts shouldFold).
  import { tick, type Snippet } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { foldScrollCompensation } from './follow';
  import type { WorkEntry } from './timeline';
  import { foldLine } from './workLog';

  interface Props {
    id: string;
    entries: WorkEntry[];
    fold: boolean;
    /** The turn's token use ("12.4k tokens") on its last stretch, or ''. */
    tokens?: string;
    children: Snippet;
  }
  let { id, entries, fold, tokens = '', children }: Props = $props();

  let anchorEl = $state<HTMLDivElement | null>(null);
  let userOpen = $state(false);
  let prevFold: boolean | null = null;

  // When a running work block auto-collapses (`false -> true`) while the reader
  // is scrolled up reading, compensate `scrollTop` so text below shifts 0 px.
  $effect.pre(() => {
    const nextFold = fold;
    const wasFolded = prevFold;
    prevFold = nextFold;
    if (wasFolded !== false || !nextFold || !anchorEl) return;
    const scroller = anchorEl.closest<HTMLElement>('.rt-scroll');
    if (!scroller || scroller.dataset.atBottom === '1') return;
    const scrollerTop = scroller.getBoundingClientRect().top;
    const rect = anchorEl.getBoundingClientRect();
    const beforeTop = rect.top - scrollerTop;
    const beforeBottom = rect.bottom - scrollerTop;
    const viewportHeight = scroller.clientHeight;
    void tick().then(() => {
      if (!anchorEl || !scroller.isConnected) return;
      const nowScrollerTop = scroller.getBoundingClientRect().top;
      const afterBottom = anchorEl.getBoundingClientRect().bottom - nowScrollerTop;
      const delta = foldScrollCompensation(
        scroller.dataset.atBottom === '1',
        beforeTop,
        beforeBottom,
        afterBottom,
        viewportHeight,
      );
      if (delta !== 0) scroller.scrollTop += delta;
    });
  });
</script>

<div class="rf-wrap" bind:this={anchorEl}>
  {#if fold}
    <details class="rf" bind:open={userOpen} data-testid="runtime-fold-{id}">
      <summary>
        <span class="ellipsis">{foldLine(entries)}{tokens ? ` · ${tokens}` : ''}</span>
        <Icon name="chevronRight" size={12} />
      </summary>
      {#if userOpen}
        <div class="rf-body">{@render children()}</div>
      {/if}
    </details>
  {:else}
    {@render children()}
  {/if}
</div>

<style>
  .rf-wrap {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  /* One dim line, a hairline under it, then the answer (the target's fold). */
  .rf > summary {
    display: flex;
    align-items: center;
    gap: var(--sp-1);
    min-block-size: 32px;
    padding-block: var(--sp-2);
    border-block-end: 1px solid var(--border);
    color: var(--text-dim);
    font-size: var(--fs-m);
    list-style: none;
    cursor: pointer;
  }
  .rf > summary::-webkit-details-marker {
    display: none;
  }
  .rf > summary:hover {
    color: var(--text);
  }
  .rf > summary:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
  }
  .rf > summary .ellipsis {
    min-width: 0;
  }
  .rf-wrap:has(> .rf) {
    margin-block-end: var(--sp-3);
  }
  .rf[open] > summary > :global(svg:last-child) {
    transform: rotate(90deg);
  }
  .rf-body {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin-inline-start: 14px;
  }
</style>
