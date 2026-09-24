<script lang="ts">
  // The Tasks tab: everything the assistant is doing or waiting on — needs
  // you → running → queued → done. What needs you comes first, in the full
  // approval / question / limit shape, so it can be decided right here.
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import { assistant } from '../../lib/stores/assistant.svelte';
  import { rel } from '../../lib/stores/now.svelte';
  import NeedsYouCard from './cards/NeedsYouCard.svelte';
  import StatePill from './cards/StatePill.svelte';
  import { TASK_KIND, groupTasks, taskStateLabel, taskTone } from './model';
  import { whenLabel } from './format';
  import type { AssistantTask } from '../../lib/api/types';

  interface Props {
    onopenthread: (id: string) => void;
  }
  let { onopenthread }: Props = $props();

  $effect(() => {
    void assistant.loadTasks();
    void assistant.loadNeedsYou();
  });

  const tasks = $derived(assistant.tasks);
  // The board lists everything except the queue; the queue comes from its own (live) source.
  const needs = $derived(assistant.needs.items);
  const needIds = $derived(new Set(needs.map((n) => n.id)));
  const groups = $derived(groupTasks(tasks.data.filter((t) => !needIds.has(t.id) && t.state !== 'needs_you')));
  const total = $derived(tasks.data.length + needs.length);
  const threadTitle = (id: string | null): string | null => (id ? (assistant.thread(id)?.title ?? null) : null);
  const COLUMNS = $derived([
    { key: 'running', label: 'Running', list: groups.running },
    { key: 'queued', label: 'Queued', list: groups.queued },
    { key: 'finished', label: 'Done', list: groups.finished },
  ]);
</script>

{#snippet row(t: AssistantTask)}
  <li class="task">
    <div class="line1">
      <span class="title">{t.title}</span>
      <StatePill tone={taskTone(t)} label={taskStateLabel(t)} live={t.state === 'running'} />
    </div>
    {#if t.detail}<div class="detail">{t.detail}</div>{/if}
    <div class="meta">
      <span>{TASK_KIND[t.kind] ?? t.kind}</span>
      {#if t.run_at && t.state === 'queued'}<span>· {whenLabel(t.run_at)}</span>{/if}
      <span>· <time datetime={t.updated_at} title={new Date(t.updated_at).toLocaleString()}>{rel(t.updated_at)}</time></span>
      {#if t.thread_id && threadTitle(t.thread_id)}
        <span>·</span>
        <button class="link" onclick={() => onopenthread(t.thread_id!)}>{threadTitle(t.thread_id)}</button>
      {/if}
    </div>
  </li>
{/snippet}

<div class="tasks" data-testid="assistant-tasks">
  {#if tasks.state === 'loading' && !tasks.data.length && !needs.length}
    <div aria-busy="true" aria-label="Loading tasks"><Skeleton rows={4} height={52} /></div>
  {:else if tasks.state === 'unsupported'}
    <EmptyState icon="check" title="Tasks aren’t available yet" body="This daemon doesn’t have the assistant’s task board. Update Otto to track reminders and long jobs here." />
  {:else if tasks.state === 'error' && !tasks.data.length}
    <div class="error" role="alert">
      <Icon name="warning" size={14} />
      <div class="error-t"><strong>Couldn’t load tasks.</strong><span class="dim">{tasks.error}</span></div>
      <button class="btn small" onclick={() => void assistant.loadTasks()}>Retry</button>
    </div>
  {:else if total === 0}
    <EmptyState icon="check" title="No tasks yet" body="Ask Otto for something that takes a while — “find me a cheaper flight”, “remind me at 5” — and it shows up here while it runs." />
  {:else}
    {#if needs.length}
      <section>
        <h2 class="section-title">Needs you <span class="count warn">{needs.length}</span></h2>
        <div class="stack">
          {#each needs as n (n.id)}
            <div>
              <NeedsYouCard task={n} compact />
              {#if n.thread_id && threadTitle(n.thread_id)}
                <button class="link from" onclick={() => onopenthread(n.thread_id!)}>Open in {threadTitle(n.thread_id)}</button>
              {/if}
            </div>
          {/each}
        </div>
      </section>
    {/if}
    {#each COLUMNS as col (col.key)}
      {#if col.list.length}
        <section>
          <h2 class="section-title">{col.label} <span class="count">{col.list.length}</span></h2>
          <ul class="list">
            {#each col.list as t (t.id)}{@render row(t)}{/each}
          </ul>
        </section>
      {/if}
    {/each}
  {/if}
</div>

<style>
  .tasks {
    max-width: 820px;
    padding: 18px 20px 32px;
    display: flex;
    flex-direction: column;
    gap: 20px;
  }
  .section-title {
    margin: 0 0 8px;
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .count {
    font-weight: 500;
    letter-spacing: 0;
  }
  .count.warn {
    color: var(--warning);
  }
  .stack {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .from {
    margin-top: 4px;
    font-size: var(--fs-s);
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
  }
  .task {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 10px 12px;
  }
  .task + .task {
    border-top: 1px solid var(--border);
  }
  .line1 {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .title {
    flex: 1;
    min-width: 0;
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .detail {
    font-size: var(--fs-s);
  }
  .meta {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .link {
    border: 0;
    background: none;
    padding: 0;
    font: inherit;
    color: var(--accent-text);
    cursor: pointer;
  }
  .link:hover {
    text-decoration: underline;
  }
  .dim {
    color: var(--text-dim);
  }
  .error {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
  }
  .error > :global(svg) {
    color: var(--danger);
    margin-top: 2px;
  }
  .error-t {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 2px;
    font-size: var(--fs-s);
  }
  @media (max-width: 640px) {
    .tasks {
      padding: 12px 12px 24px;
    }
  }
</style>
