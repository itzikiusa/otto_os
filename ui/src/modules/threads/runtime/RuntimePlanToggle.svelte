<script lang="ts">
  // A structured thread's Plan mode: on, every prompt asks for a plan until
  // the person turns it off here (or ⇧⇥ in the composer).
  import Icon from '../../../lib/components/Icon.svelte';
  import { runtimeThreads } from './runtimeThreads.svelte';

  let { id, on, disabled }: { id: string; on: boolean; disabled: boolean } = $props();
</script>

<button
  type="button"
  class="rt-plan"
  class:on
  aria-pressed={on}
  title="Plan first: the agent researches and proposes, and changes nothing (⇧⇥)"
  data-testid="runtime-plan"
  {disabled}
  onclick={() => (runtimeThreads.planIds[id] = !on)}
>
  <Icon name="compass" size={13} /> Plan
</button>

<style>
  .rt-plan {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    block-size: 28px;
    padding-inline: 6px;
    border: 0;
    border-radius: var(--radius-m);
    background: transparent;
    font: inherit;
    font-size: var(--fs-s);
    color: var(--text-dim);
    cursor: pointer;
  }
  .rt-plan:hover:not(:disabled) {
    background: var(--hover);
    color: var(--text);
  }
  .rt-plan.on {
    color: var(--warning);
  }
</style>
