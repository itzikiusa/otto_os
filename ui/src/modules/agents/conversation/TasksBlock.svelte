<script lang="ts">
  // The agent's plan (task-list state AFTER a TodoWrite / TaskCreate /
  // TaskUpdate call) as a checklist in the flow: "Plan · 1 of 4 done" + items
  // with a status mark. Earlier snapshots of the same response start folded to
  // their header line (the latest one is the plan that matters).
  import { untrack } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import type { TaskItem } from '../../../lib/api/types';

  interface Props {
    tasks: TaskItem[];
    /** Start folded to the header line. */
    folded?: boolean;
  }
  let { tasks, folded = false }: Props = $props();
  const done = $derived(tasks.filter((t) => t.status === 'completed').length);
  const current = $derived(tasks.find((t) => t.status === 'in_progress') ?? null);
  let open = $state(untrack(() => !folded));
</script>

<div class="tasks" class:open data-total={tasks.length}>
  <button class="tasks-head" onclick={() => (open = !open)} aria-expanded={open}>
    <Icon name="check" size={12} />
    <span class="tasks-title">Plan</span>
    <span class="tasks-count">{done} of {tasks.length} done</span>
    {#if !open && current}<span class="tasks-now">· {current.active_form || current.title}</span>{/if}
    <span class="tasks-caret" aria-hidden="true"><Icon name={open ? 'chevronDown' : 'chevronRight'} size={12} /></span>
  </button>
  {#if open}
    <ul>
      {#each tasks as t, i (t.ext_id ?? `${i}:${t.title}`)}
        <li class={t.status}>
          <span class="mark" aria-hidden="true">
            {#if t.status === 'completed'}<Icon name="check" size={11} />{:else}<span class="ring"></span>{/if}
          </span>
          <span class="ttl">{t.status === 'in_progress' && t.active_form ? t.active_form : t.title}</span>
          <span class="sr-only">— {t.status === 'completed' ? 'done' : t.status === 'in_progress' ? 'in progress' : 'to do'}</span>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .tasks {
    margin: 2px 0;
    font-size: var(--fs-s);
    min-width: 0;
  }
  .tasks-head {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 4px 8px;
    background: none;
    border: 0;
    border-radius: var(--radius-s);
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-s);
    cursor: pointer;
    text-align: start;
    min-width: 0;
  }
  .tasks-head:hover {
    background: var(--hover);
  }
  .tasks-head:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
  }
  .tasks-title {
    color: var(--text);
    font-weight: 500;
  }
  .tasks-count {
    font-size: var(--fs-xs);
    white-space: nowrap;
  }
  .tasks-now {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--fs-xs);
  }
  .tasks-caret {
    display: inline-flex;
    margin-inline-start: auto;
  }
  ul {
    list-style: none;
    margin: 0 0 4px;
    padding-block: 2px; padding-inline: 6px 0;
    margin-inline-start: 14px;
    border-inline-start: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  li {
    display: flex;
    gap: 8px;
    align-items: baseline;
    min-width: 0;
    padding-inline-start: 2px;
  }
  .mark {
    flex-shrink: 0;
    width: 12px;
    display: inline-flex;
    justify-content: center;
    align-self: center;
    color: var(--success);
  }
  .ring {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    border: 1.5px solid var(--text-dim);
  }
  li.in_progress .ring {
    border-color: var(--accent);
    background: var(--accent-soft);
  }
  li.in_progress .ttl {
    font-weight: 500;
  }
  li.completed .ttl {
    color: var(--text-dim);
    text-decoration: line-through;
  }
  .ttl {
    overflow-wrap: anywhere;
  }
</style>
