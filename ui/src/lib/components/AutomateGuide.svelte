<script lang="ts" module>
  /** The five Automate modules (each renders the guide on its empty state). */
  export type AutomateModule = 'swarm' | 'loops' | 'workflows' | 'scheduled-tasks' | 'personal-agents';
  /** Every entry the chooser lists: the Automate modules plus the two
   *  overlapping automations that live elsewhere (S20-18) — Assistant tasks &
   *  reminders (overlap Scheduled Tasks) and Run with Otto (overlaps Goal
   *  Loops and Workflows' PR runs). */
  export type AutomateGuideId = AutomateModule | 'assistant-tasks' | 'run-with-otto';

  export const AUTOMATE_GUIDE: readonly { id: AutomateGuideId; route: string; label: string; when: string }[] = [
    { id: 'workflows', route: 'workflows', label: 'Workflows', when: 'A fixed sequence of steps you can draw — run on demand, from a trigger, or on a schedule.' },
    { id: 'scheduled-tasks', route: 'scheduled-tasks', label: 'Scheduled Tasks', when: 'One prompt on a cadence that delivers a report (daily digest, weekly check).' },
    { id: 'assistant-tasks', route: 'assistant/tasks', label: 'Assistant tasks & reminders', when: 'A quick “remind me” or one-off errand you ask for in the Assistant chat — no form, no cadence to set up.' },
    { id: 'run-with-otto', route: 'run-with-otto', label: 'Run with Otto', when: 'One ticket, issue or finding: an agent works it on a branch, proves it, and waits for your approval before any PR.' },
    { id: 'loops', route: 'loops', label: 'Goal Loops', when: 'One measurable goal: an agent iterates on a branch until it is met or the budget runs out.' },
    { id: 'swarm', route: 'swarm', label: 'Swarm', when: 'A team of role agents working a project board together — bigger, multi-task work.' },
    { id: 'personal-agents', route: 'personal-agents', label: 'Personal Agents', when: 'A named persona with memory and its own schedules that you talk to over time.' },
  ];
</script>

<script lang="ts">
  // "Which one do I want?" — the Automate modules (and the Assistant's tasks
  // and Run with Otto) overlap: each can run an agent on its own. Every Automate
  // empty state carries this one-line-each chooser, collapsed as a quiet
  // secondary under the single primary CTA; each Automate page's ⋯ also opens
  // it expanded in a sheet (AutomateGuideButton), so a user with one item
  // still finds it.
  import { router } from '../router.svelte';
  import { auth } from '../stores/auth.svelte';
  import { routeAllowed } from '../sidebar';

  let {
    current,
    open = false,
    onnavigate,
  }: {
    current: AutomateModule;
    /** Start expanded (the ⋯ sheet) instead of collapsed (empty states). */
    open?: boolean;
    /** Called before navigating (the sheet closes itself). */
    onnavigate?: () => void;
  } = $props();

  // Only the entries this user can actually open (S20-307): the same RBAC /
  // feature gate the sidebar applies — the current page always stays listed.
  const entries = $derived(
    AUTOMATE_GUIDE.filter((m) => m.id === current || routeAllowed(m.route, (f) => auth.can(f, 'view'))),
  );
</script>

<details class="automate-guide" data-testid="automate-guide" {open}>
  <summary>Which one do I want?</summary>
  <ul>
    {#each entries as m (m.id)}
      <li class:here={m.id === current}>
        {#if m.id === current}
          <strong>{m.label}</strong> <span class="here-tag">(this page)</span>
        {:else}
          <button type="button" class="link" onclick={() => { onnavigate?.(); router.go(m.route); }}>{m.label}</button>
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
  summary:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; border-radius: var(--radius-s); }
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
  .link:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; border-radius: var(--radius-s); }
</style>
