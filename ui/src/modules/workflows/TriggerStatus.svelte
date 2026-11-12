<script lang="ts">
  // One line under a trigger: does it work? See triggerStatus.ts.
  import { rel } from '../../lib/stores/now.svelte';
  import type { WorkflowTrigger } from '../../lib/api/types';
  import { triggerStatusLine } from './triggerStatus';

  let { trigger }: { trigger: WorkflowTrigger } = $props();
  const line = $derived(triggerStatusLine(trigger, (iso) => rel(iso)));
  const exact = $derived(trigger.last_fired_at ? new Date(trigger.last_fired_at).toLocaleString() : 'This trigger has not fired yet');
</script>

<span class="trig-status {line.tone}" data-testid="trigger-status" title={`${line.text} — ${exact}`}>{line.text}</span>

<style>
  .trig-status {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .trig-status.warn {
    color: var(--warning);
  }
</style>
