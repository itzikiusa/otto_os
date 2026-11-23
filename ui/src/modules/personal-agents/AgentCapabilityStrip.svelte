<script lang="ts">
  // At-a-glance capability strip under a personal agent's header (E-15).
  import { router } from '../../lib/router.svelte';
  import type { PersonalAgent, PersonalAgentAutonomy, PersonalAgentSchedule } from '../../lib/api/types';
  import { agentCapabilityChips } from './agentCapabilities';

  interface Props {
    agent: PersonalAgent;
    autonomy?: PersonalAgentAutonomy;
    schedules: readonly PersonalAgentSchedule[];
  }
  let { agent, autonomy, schedules }: Props = $props();

  const chips = $derived(agentCapabilityChips(agent, autonomy, schedules));
</script>

{#if chips.length > 0}
  <div class="cap-strip" aria-label="Agent capabilities" data-testid="pa-capability-strip">
    {#each chips as c (c.id)}
      <button
        type="button"
        class="cap-chip tone-{c.tone}"
        title={c.title}
        onclick={() => router.go(`personal-agents/${agent.id}/${c.tab}`)}
      >
        {c.label}
      </button>
    {/each}
  </div>
{/if}

<style>
  .cap-strip {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    padding: 6px 14px;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
  }
  .cap-chip {
    padding: 2px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--bg);
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .cap-chip:hover {
    background: var(--hover);
    color: var(--text);
  }
  .cap-chip.tone-accent {
    border-color: var(--accent-line-strong);
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .cap-chip.tone-warning {
    border-color: color-mix(in srgb, var(--warning) 40%, transparent);
    background: var(--warning-soft);
    color: var(--warning);
    font-weight: 600;
  }
  .cap-chip:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 1px;
  }
</style>
