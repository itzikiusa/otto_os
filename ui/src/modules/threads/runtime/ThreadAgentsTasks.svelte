<script lang="ts">
  // Thread details' "Agents & tasks": every subagent and background task of
  // the focused runtime thread (agentTasks.ts) — running first with its
  // elapsed time, the last few finished folded under one line. Hidden when
  // the thread has none. The strip above the composer brings it into view
  // (threads.showAgentsTasks).
  import { tick, untrack } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import type { IconName } from '../../../lib/components/Icon.svelte';
  import { now } from '../../../lib/stores/now.svelte';
  import { threads } from '../threadsStore.svelte';
  import { agentTasks, taskElapsed, type AgentTask } from './agentTasks';
  import { runtimeThreads } from './runtimeThreads.svelte';
  import { runtimeViews } from './runtimeView.svelte';

  let { sessionId }: { sessionId: string } = $props();

  /** Finished tasks shown when the fold opens (newest first). */
  const FINISHED_SHOWN = 5;

  const p = $derived(runtimeViews.projections[sessionId] ?? null);
  const tasks = $derived(agentTasks(p?.items ?? [], p?.session?.background_tasks ?? runtimeThreads.get(sessionId)?.background_tasks ?? []));
  const any = $derived(tasks.running.length + tasks.finished.length > 0);

  // The shared 1 s clock, read only while something runs.
  const nowMs = $derived(tasks.running.length ? now() : Date.now());

  let el = $state<HTMLElement | null>(null);
  /** Reveals asked for before this mount are not ours to replay. */
  const revealed = untrack(() => threads.agentsTasksReveal);
  $effect(() => {
    if (threads.agentsTasksReveal <= revealed) return;
    void tick().then(() => {
      el?.scrollIntoView({ block: 'nearest' });
      el?.focus({ preventScroll: true });
    });
  });

  const STATUS: Record<AgentTask['status'], { icon: IconName; label: string }> = {
    running: { icon: 'dot', label: 'Running' },
    done: { icon: 'check', label: 'Done' },
    failed: { icon: 'warning', label: 'Failed' },
    stopped: { icon: 'stop', label: 'Stopped' },
  };
</script>

{#snippet row(t: AgentTask)}
  {@const e = taskElapsed(t, nowMs)}
  <li class="tat-row" class:failed={t.status === 'failed'} data-testid="agents-tasks-row" data-status={t.status}>
    {#if t.status === 'running'}
      <span class="spinner" aria-hidden="true"></span>
    {:else}
      <Icon name={STATUS[t.status].icon} size={12} />
    {/if}
    <Icon name={t.kind === 'agent' ? 'assistant' : 'terminal'} size={12} />
    <span class="ellipsis" dir="auto" title={t.label}>{t.label}</span>
    <span class="tat-meta">{STATUS[t.status].label}{t.background && t.status === 'running' ? ' · background' : ''}{e ? ` · ${e}` : ''}</span>
  </li>
{/snippet}

{#if any}
  <section class="tat" aria-label="Agents and tasks" data-testid="thread-agents-tasks" tabindex="-1" bind:this={el}>
    <h3 class="tat-head">
      Agents &amp; tasks
      {#if tasks.running.length}<span class="tat-count">{tasks.running.length} running</span>{/if}
    </h3>
    {#if tasks.running.length}
      <ul class="tat-list">{#each tasks.running as t (t.id)}{@render row(t)}{/each}</ul>
    {/if}
    {#if tasks.finished.length}
      <details class="tat-done">
        <summary>{tasks.finished.length} finished</summary>
        <ul class="tat-list">{#each tasks.finished.slice(0, FINISHED_SHOWN) as t (t.id)}{@render row(t)}{/each}</ul>
      </details>
    {/if}
  </section>
{/if}

<style>
  .tat {
    flex-shrink: 0;
    max-block-size: 40vh;
    overflow-y: auto;
    padding-block: 8px;
    padding-inline: 10px;
    border-block-end: 1px solid var(--border);
  }
  .tat:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
  }
  .tat-head {
    display: flex;
    align-items: baseline;
    gap: 8px;
    margin: 0 0 4px;
    font-size: var(--fs-s);
    font-weight: 600;
    color: var(--text);
  }
  .tat-count {
    font-weight: 400;
    color: var(--text-dim);
  }
  .tat-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .tat-row {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    min-block-size: 24px;
    font-size: var(--fs-s);
    color: var(--text);
  }
  .tat-row.failed {
    color: var(--danger);
  }
  .tat-row .ellipsis {
    flex: 1;
    min-width: 0;
  }
  .tat-row .spinner {
    --spinner-size: 11px;
  }
  .tat-meta {
    flex-shrink: 0;
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .tat-done > summary {
    padding-block: 4px;
    color: var(--text-dim);
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .tat-done > summary:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 2px;
  }
</style>
