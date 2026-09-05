<script lang="ts">
  // Task-list snapshot (state AFTER a TodoWrite / TaskCreate / TaskUpdate call).
  import type { TaskItem } from '../../../lib/api/types';

  interface Props {
    tasks: TaskItem[];
  }
  let { tasks }: Props = $props();
  const done = $derived(tasks.filter((t) => t.status === 'completed').length);
</script>

<div class="tasks" data-total={tasks.length}>
  <div class="tasks-head dim">Tasks · {done}/{tasks.length} done</div>
  <ul>
    {#each tasks as t, i (t.ext_id ?? `${i}:${t.title}`)}
      <li class={t.status}>
        <span class="mark" aria-hidden="true">{t.status === 'completed' ? '☑' : t.status === 'in_progress' ? '◐' : '☐'}</span>
        <span class="ttl">{t.status === 'in_progress' && t.active_form ? t.active_form : t.title}</span>
      </li>
    {/each}
  </ul>
</div>

<style>
  .tasks {
    padding: 6px 10px 8px 31px;
    font-size: 12.5px;
  }
  .tasks-head {
    font-size: 11px;
    margin-bottom: 3px;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  li {
    display: flex;
    gap: 7px;
    align-items: baseline;
    min-width: 0;
  }
  .mark {
    flex-shrink: 0;
    color: var(--text-dim);
  }
  li.completed .ttl {
    color: var(--text-dim);
    text-decoration: line-through;
  }
  li.in_progress .mark {
    color: var(--accent);
  }
  .ttl {
    overflow-wrap: anywhere;
  }
</style>
