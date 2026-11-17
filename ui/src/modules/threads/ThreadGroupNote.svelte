<script lang="ts">
  // A folded group's roll-up beside its header (a collapsed project, "Agent
  // runs", "Pull requests"…): a dot and "1 needs you" (or "2 failed") when a
  // member is flagged, so nothing that needs you hides behind a fold.
  // Nothing when every member is calm.
  import StatusDot from '../../lib/components/StatusDot.svelte';
  import type { Session } from '../../lib/api/types';
  import { mostUrgent, rollup, rollupNote } from './threadAttention';
  import { threadRowState } from './threadNav.svelte';

  let { items, testid }: { items: readonly Session[]; testid?: string } = $props();

  const states = $derived(items.filter((s) => !s.archived).map(threadRowState));
  const note = $derived(rollupNote(rollup(states)));
  const top = $derived(mostUrgent(states));
</script>

{#if note && top}
  <span class="gn" class:gn-bad={note.tone === 'danger'} data-testid={testid}>
    <StatusDot state={top} />
    {note.text}
  </span>
{/if}

<style>
  .gn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    flex-shrink: 0;
    font-size: var(--fs-xs);
    font-weight: 400;
    color: var(--warning);
    white-space: nowrap;
  }
  .gn-bad {
    color: var(--danger);
  }
</style>
