<script lang="ts">
  // A finished stretch of the work log under one quiet line — "Worked for
  // 2m 14s · 6 steps" — that opens to every call (T3 Code's collapsed work
  // log). A stretch that is still running, or holds a failure or an approval
  // waiting on you, is never folded: it renders as is (workLog.ts shouldFold).
  import type { Snippet } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import type { WorkEntry } from './timeline';
  import { workedFor } from './workLog';

  interface Props {
    id: string;
    entries: WorkEntry[];
    fold: boolean;
    children: Snippet;
  }
  let { id, entries, fold, children }: Props = $props();
</script>

{#if fold}
  <details class="rf" data-testid="runtime-fold-{id}">
    <summary>
      <Icon name="check" size={12} />
      <span class="ellipsis">{workedFor(entries)} · {entries.length} steps</span>
      <Icon name="chevronRight" size={12} />
    </summary>
    <div class="rf-body">{@render children()}</div>
  </details>
{:else}
  {@render children()}
{/if}

<style>
  .rf > summary {
    display: flex;
    align-items: center;
    gap: 8px;
    min-block-size: 28px;
    padding: 2px 6px;
    border-radius: var(--radius-s);
    color: var(--text-dim);
    font-size: var(--fs-s);
    list-style: none;
    cursor: pointer;
  }
  .rf > summary::-webkit-details-marker {
    display: none;
  }
  .rf > summary:hover {
    background: var(--hover);
    color: var(--text);
  }
  .rf > summary:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
  }
  .rf > summary .ellipsis {
    flex: 1;
    min-width: 0;
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
