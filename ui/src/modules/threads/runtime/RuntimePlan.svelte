<script lang="ts">
  // The running turn's plan above the composer (T3 Code's plan steps):
  // done steps tick green, the running one reads full strength.
  import Icon from '../../../lib/components/Icon.svelte';
  import type { PlanArtifact } from '../../../lib/api/types';

  let { plan }: { plan: PlanArtifact } = $props();
</script>

<ol class="rt-plan" aria-label="Plan">
  {#each plan.steps as s (s.id)}
    <li class="step-{s.status}" dir="auto"><Icon name={s.status === 'completed' ? 'check' : 'dot'} size={12} />{s.text}</li>
  {/each}
</ol>

<style>
  .rt-plan {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0;
    padding: 6px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    color: var(--text-dim);
    font-size: var(--fs-s);
    list-style: none;
  }
  .rt-plan li {
    display: flex;
    align-items: center;
    gap: 6px;
    overflow-wrap: anywhere;
  }
  .rt-plan .step-running {
    color: var(--text);
  }
  .rt-plan .step-completed {
    color: var(--success);
  }
</style>
