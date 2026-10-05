<script lang="ts" module>
  /** The five Automate modules, in the order the guide lists them. */
  export type AutomateModule = 'swarm' | 'loops' | 'workflows' | 'scheduled-tasks' | 'personal-agents';

  export const AUTOMATE_GUIDE: readonly { id: AutomateModule; label: string; when: string }[] = [
    { id: 'workflows', label: 'Workflows', when: 'A fixed sequence of steps you can draw — run on demand, from a trigger, or on a schedule.' },
    { id: 'scheduled-tasks', label: 'Scheduled Tasks', when: 'One prompt on a cadence that delivers a report (daily digest, weekly check).' },
    { id: 'loops', label: 'Goal Loops', when: 'One measurable goal: an agent iterates on a branch until it is met or the budget runs out.' },
    { id: 'swarm', label: 'Swarm', when: 'A team of role agents working a project board together — bigger, multi-task work.' },
    { id: 'personal-agents', label: 'Personal Agents', when: 'A named persona with memory and its own schedules that you talk to over time.' },
  ];
</script>

<script lang="ts">
  // "Which one do I want?" — the five Automate modules overlap (each can run
  // an agent on its own), so every one of their empty states carries the same
  // one-line-each chooser with links to the others. Collapsed by default: a
  // quiet secondary under the empty state's single primary CTA.
  import { router } from '../router.svelte';

  let { current }: { current: AutomateModule } = $props();
</script>

<details class="automate-guide" data-testid="automate-guide">
  <summary>Which one do I want?</summary>
  <ul>
    {#each AUTOMATE_GUIDE as m (m.id)}
      <li class:here={m.id === current}>
        {#if m.id === current}
          <strong>{m.label}</strong> <span class="here-tag">(this page)</span>
        {:else}
          <button type="button" class="link" onclick={() => router.go(m.id)}>{m.label}</button>
        {/if}
        <span class="when">— {m.when}</span>
      </li>
    {/each}
  </ul>
</details>

<style>
  .automate-guide {
    margin-block-start: 12px;
    max-width: 560px;
    text-align: start;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  summary {
    cursor: pointer;
    color: var(--accent-text);
    text-align: center;
  }
  summary:focus-visible { outline: 2px solid var(--accent-solid); outline-offset: 2px; border-radius: 4px; }
  ul {
    margin: 8px 0 0;
    padding: 0;
    list-style: none;
    display: grid;
    gap: 6px;
    line-height: 1.45;
  }
  .here strong { color: var(--text); }
  .here-tag { font-size: var(--fs-xs); }
  .link {
    background: none;
    border: 0;
    padding: 0;
    font: inherit;
    color: var(--accent-text);
    cursor: pointer;
    text-decoration: underline;
    text-underline-offset: 2px;
  }
  .link:focus-visible { outline: 2px solid var(--accent-solid); outline-offset: 2px; border-radius: 3px; }
</style>
