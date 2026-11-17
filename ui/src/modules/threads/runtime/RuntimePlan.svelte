<script lang="ts">
  // The running turn's plan above the composer (T3 Code's plan steps):
  // done steps tick green, the running one reads full strength. On a phone
  // (or a short window) it is one line — "Plan 2/4 · the current step" — and
  // a tap opens every step, so a running turn's chrome stays small.
  import Icon from '../../../lib/components/Icon.svelte';
  import type { PlanArtifact } from '../../../lib/api/types';

  let { plan }: { plan: PlanArtifact } = $props();
  const done = $derived(plan.steps.filter((s) => s.status === 'completed').length);
  const current = $derived(plan.steps.find((s) => s.status === 'running') ?? plan.steps.find((s) => s.status === 'pending'));
  let small = $state(false);
  let open = $state(false);
  $effect(() => {
    if (typeof matchMedia !== 'function') return;
    const mq = matchMedia('(max-width: 640px), (max-height: 500px)');
    small = mq.matches;
    const on = (): void => void (small = mq.matches);
    mq.addEventListener('change', on);
    return () => mq.removeEventListener('change', on);
  });
</script>

{#if small && !open}
  <button type="button" class="rt-plan rt-plan-line" aria-expanded="false" data-testid="runtime-plan-line" onclick={() => (open = true)}>
    <Icon name="dot" size={12} />
    <span class="ellipsis" dir="auto">Plan {done}/{plan.steps.length}{current ? ` · ${current.text}` : ''}</span>
    <Icon name="chevronDown" size={12} />
  </button>
{:else}
  <ol class="rt-plan" aria-label="Plan" data-testid="runtime-plan">
    {#each plan.steps as s (s.id)}
      <li class="step-{s.status}" dir="auto"><Icon name={s.status === 'completed' ? 'check' : 'dot'} size={12} />{s.text}</li>
    {/each}
    {#if small}
      <li><button type="button" class="rt-plan-less" aria-expanded="true" onclick={() => (open = false)}>Show less</button></li>
    {/if}
  </ol>
{/if}

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
  .rt-plan-line {
    flex-direction: row;
    align-items: center;
    gap: 6px;
    inline-size: 100%;
    min-inline-size: 0;
    min-block-size: var(--hit-min);
    font: inherit;
    font-size: var(--fs-s);
    text-align: start;
    cursor: pointer;
  }
  .rt-plan-line .ellipsis {
    flex: 1;
    min-inline-size: 0;
    color: var(--text);
  }
  .rt-plan-less {
    padding: 4px 0;
    border: 0;
    background: transparent;
    color: var(--accent-text);
    font: inherit;
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  :is(.rt-plan-line, .rt-plan-less):focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
  }
</style>
