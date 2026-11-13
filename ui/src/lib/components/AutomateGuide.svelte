<script lang="ts" module>
  import type { Feature } from '../api/types';
  import { routeAllowed } from '../sidebar';

  /** The four Automate places (each renders the guide on its empty state). */
  export type AutomateModule = 'loops' | 'workflows' | 'scheduled-tasks' | 'personal-agents';
  export type AutomateGuideId = AutomateModule;

  /** Intent first: each row leads with the JOB a person has in mind and ends
   *  with the PLACE that does it. The schedule engine is shared; the places
   *  differ in what runs (one prompt, a graph, a goal, a named assistant). */
  export const AUTOMATE_GUIDE: readonly { id: AutomateGuideId; route: string; label: string; job: string }[] = [
    { id: 'scheduled-tasks', route: 'scheduled-tasks', label: 'Scheduled tasks', job: 'Run one prompt or command on a schedule and get the report (Slack, email…)' },
    { id: 'workflows', route: 'workflows', label: 'Workflows', job: 'Several steps in order: on demand, from a trigger, or on a schedule' },
    { id: 'loops', route: 'loops', label: 'Goal Loops', job: 'Keep going until a goal is met or the budget runs out' },
    { id: 'personal-agents', route: 'personal-agents', label: 'Personal Agents', job: 'A named assistant with memory, its own routines and a chat' },
  ];

  export const AUTOMATE_GUIDE_FOOTER =
    'Every morning? One prompt → a Scheduled task. Several steps → a Workflow with a Schedule trigger.';

  /** The rows this person can open (S20-307: the same RBAC / feature gate the
   *  sidebar applies), the current page always kept and marked `here`. */
  export function automateGuideEntries(current: AutomateModule, can: (f: Feature) => boolean) {
    return AUTOMATE_GUIDE.filter((m) => m.id === current || routeAllowed(m.route, can)).map((m) => ({
      ...m,
      here: m.id === current,
    }));
  }
</script>

<script lang="ts">
  // "Which one do I want?" — the Automate places overlap: each can run an agent
  // on its own. On an empty Automate page the chooser is shown OPEN (it is the
  // most useful thing there), under the single primary CTA; each Automate
  // page's ⋯ also opens it in a sheet (AutomateGuideButton), so a person with
  // items still finds it. Collapsed (`open` false) stays for any other mount.
  import Icon from './Icon.svelte';
  import { router } from '../router.svelte';
  import { auth } from '../stores/auth.svelte';

  let {
    current,
    open = false,
    titled = true,
    onnavigate,
  }: {
    current: AutomateModule;
    /** Shown expanded (empty states, the ⋯ sheet) instead of a collapsed disclosure. */
    open?: boolean;
    /** Show the "Which one do I want?" heading (the sheet's own title says it). */
    titled?: boolean;
    /** Called before navigating (the sheet closes itself). */
    onnavigate?: () => void;
  } = $props();

  const entries = $derived(automateGuideEntries(current, (f) => auth.can(f, 'view')));
</script>

{#snippet rows()}
  <ul>
    {#each entries as m (m.id)}
      <li data-testid="automate-guide-row" data-id={m.id}>
        {#if m.here}
          <div class="ag-row here" aria-current="page">
            <span class="job">{m.job}</span>
            <span class="place"><strong>{m.label}</strong> · this page</span>
          </div>
        {:else}
          <button type="button" class="ag-row" onclick={() => { onnavigate?.(); router.go(m.route); }}>
            <span class="job">{m.job}</span>
            <span class="place link">{m.label} <Icon name="chevronRight" size={12} /></span>
          </button>
        {/if}
      </li>
    {/each}
  </ul>
  <p class="foot">{AUTOMATE_GUIDE_FOOTER}</p>
{/snippet}

{#if open}
  <section class="automate-guide" data-testid="automate-guide" aria-label="Which one do I want?">
    {#if titled}<p class="title">Which one do I want?</p>{/if}
    {@render rows()}
  </section>
{:else}
  <details class="automate-guide" data-testid="automate-guide">
    <summary>Which one do I want?</summary>
    {@render rows()}
  </details>
{/if}

<style>
  .automate-guide {
    margin-block-start: 12px;
    max-width: 560px;
    inline-size: 100%;
    text-align: start;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  summary,
  .title {
    color: var(--accent-text);
    text-align: center;
  }
  .title { margin: 0; color: var(--text); font-weight: 600; }
  summary { cursor: pointer; }
  summary:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; border-radius: var(--radius-s); }
  ul {
    margin: 8px 0 0;
    padding: 0;
    list-style: none;
    display: grid;
    gap: 4px;
    line-height: 1.45;
  }
  .ag-row {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    justify-content: space-between;
    gap: 2px 12px;
    inline-size: 100%;
    padding: 6px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
    font: inherit;
    color: var(--text);
    text-align: start;
  }
  button.ag-row { cursor: pointer; }
  button.ag-row:hover { border-color: var(--accent-text); }
  button.ag-row:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; }
  .ag-row.here { background: none; }
  .job { flex: 1 1 16em; min-inline-size: 0; overflow-wrap: anywhere; }
  .place { flex: 0 0 auto; color: var(--text-dim); white-space: nowrap; }
  .place strong { color: var(--text); }
  .place.link {
    color: var(--accent-text);
    display: inline-flex;
    align-items: center;
    gap: 2px;
  }
  .foot {
    margin: 8px 0 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
</style>
