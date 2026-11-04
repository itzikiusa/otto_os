<script lang="ts">
  // Tool calls that ran at the same time, under one summary line ("Fetched 3
  // web pages", with how many are still running) that opens to the calls.
  // Ported from T3 Code (MIT, Copyright (c) 2026 T3 Tools Inc.):
  // apps/web/src/components/chat/TimelineWorkGroupRows.tsx (the work-toggle row).
  import type { Snippet } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { groupSummary, type WorkEntry } from './timeline';

  interface Props {
    id: string;
    entries: WorkEntry[];
    /** Held open (a call in it waits for your approval). */
    open: boolean;
    children: Snippet;
  }
  let { id, entries, open, children }: Props = $props();

  const live = $derived(entries.filter((e) => e.item.status === 'running' || e.item.status === 'pending').length);
  const failed = $derived(entries.some((e) => e.item.status === 'failed'));
</script>

<details class="rg" open={open || undefined} data-testid="runtime-group-{id}">
  <summary>
    <Icon name="layers" size={13} />
    <span class="ellipsis">{groupSummary(entries)}</span>
    {#if live}<span class="rg-live"><span class="spinner" aria-hidden="true"></span>{live} running</span>{/if}
    {#if failed}<Icon name="x" size={12} />{/if}
    <Icon name="chevronRight" size={12} />
  </summary>
  {@render children()}
</details>

<style>
  .rg > summary {
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
  .rg > summary::-webkit-details-marker {
    display: none;
  }
  .rg > summary:hover {
    background: var(--hover);
    color: var(--text);
  }
  .rg > summary .ellipsis {
    flex: 1;
    min-width: 0;
  }
  .rg[open] > summary > :global(svg:last-child) {
    transform: rotate(90deg);
  }
  .rg[open] {
    padding-block-end: 4px;
  }
  .rg > :global(:not(summary)) {
    margin-inline-start: 14px;
  }
  .rg-live {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: var(--fs-xs);
  }
  .rg-live .spinner {
    --spinner-size: 10px;
  }
</style>
