<script lang="ts">
  // Thread details' "Agents & tasks": every subagent and background task of
  // the focused runtime thread (agentTasks.ts) — running first with its
  // elapsed time, the last few finished folded under one line. Hidden when
  // the thread has none. The strip above the composer brings it into view
  // (threads.showAgentsTasks). A row opens its work in place (Back returns;
  // a Modal on a narrow window): a subagent is the very card the timeline
  // shows (RuntimeSubagentCard — its prompt, activity, result; an
  // Antigravity swarm member's board and steps), a background command its
  // description, command, launch call and output (T3 Code's
  // BackgroundWorkDetails, MIT, Copyright (c) 2026 T3 Tools Inc.).
  import { tick, untrack } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import type { IconName } from '../../../lib/components/Icon.svelte';
  import Modal from '../../../lib/components/Modal.svelte';
  import type { TurnItem } from '../../../lib/api/types';
  import { now } from '../../../lib/stores/now.svelte';
  import { threads } from '../threadsStore.svelte';
  import { agentTasks, taskElapsed, type AgentTask } from './agentTasks';
  import { runtimeThreads } from './runtimeThreads.svelte';
  import { runtimeViews } from './runtimeView.svelte';
  import { subagentChildren } from './timeline';
  import RuntimeSubagentCard from './RuntimeSubagentCard.svelte';
  import RuntimeTaskOutput from './RuntimeTaskOutput.svelte';
  import RuntimeWorkRow from './RuntimeWorkRow.svelte';

  let { sessionId }: { sessionId: string } = $props();

  /** Finished tasks shown when the fold opens (newest first). */
  const FINISHED_SHOWN = 5;

  const p = $derived(runtimeViews.projections[sessionId] ?? null);
  const tasks = $derived(agentTasks(p?.items ?? [], p?.session?.background_tasks ?? runtimeThreads.get(sessionId)?.background_tasks ?? []));
  const any = $derived(tasks.running.length + tasks.finished.length > 0);
  const byId = $derived(new Map((p?.items ?? []).map((i) => [i.id, i] as const)));
  const children = $derived(p ? subagentChildren(p.items) : new Map<string, TurnItem[]>());

  // The shared 1 s clock, read only while something runs.
  const nowMs = $derived(tasks.running.length ? now() : Date.now());

  /** The task opened, by id (it may move from running to finished meanwhile). */
  let openId = $state<string | null>(null);
  const opened = $derived(openId ? ([...tasks.running, ...tasks.finished].find((t) => t.id === openId) ?? null) : null);
  // Another thread: back to its list.
  $effect(() => {
    void sessionId;
    untrack(() => (openId = null));
  });

  /** The subagent card a row stands for. */
  function cardOf(t: AgentTask): Extract<TurnItem, { type: 'subagent' }> | null {
    const i = t.itemId ? byId.get(t.itemId) : undefined;
    return i?.type === 'subagent' ? i : null;
  }

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
  {@const card = cardOf(t)}
  {#if card}
    <li class="tat-card" data-testid="agents-tasks-row" data-status={t.status}>
      <RuntimeSubagentCard item={card} activity={children.get(card.id) ?? []} output={p?.output} {sessionId} onopen={() => (openId = t.id)} />
    </li>
  {:else}
    {@const e = taskElapsed(t, nowMs)}
    <li data-testid="agents-tasks-row" data-status={t.status}>
      <button type="button" class="tat-row" class:failed={t.status === 'failed'} onclick={() => (openId = t.id)}>
        {#if t.status === 'running'}
          <span class="spinner" aria-hidden="true"></span>
        {:else}
          <Icon name={STATUS[t.status].icon} size={12} />
        {/if}
        <Icon name={t.kind === 'agent' ? 'assistant' : 'terminal'} size={12} />
        <span class="tat-text">
          <span class="ellipsis" dir="auto" title={t.label}>{t.label}</span>
          {#if t.command && t.command !== t.label}<span class="tat-cmd ellipsis mono" dir="ltr" title={t.command}>{t.command}</span>{/if}
        </span>
        <span class="tat-meta">{STATUS[t.status].label}{t.background && t.status === 'running' ? ' · background' : ''}{e ? ` · ${e}` : ''}</span>
        <Icon name="chevronRight" size={12} />
      </button>
    </li>
  {/if}
{/snippet}

{#snippet detail(t: AgentTask)}
  {@const card = cardOf(t)}
  {#if card}
    <RuntimeSubagentCard item={card} activity={children.get(card.id) ?? []} output={p?.output} {sessionId} startOpen />
  {:else}
    {@const launch = t.itemId ? byId.get(t.itemId) : undefined}
    {@const e = taskElapsed(t, nowMs)}
    <div class="tat-cmd-detail">
      <p class="tat-meta">{STATUS[t.status].label}{t.background ? ' · background' : ''}{e ? ` · ${e}` : ''}</p>
      {#if t.command}<pre class="tat-command mono" dir="ltr">{t.command}</pre>{/if}
      {#if launch}<RuntimeWorkRow item={launch} output={p?.output?.[launch.id]} cwd={runtimeThreads.get(sessionId)?.cwd ?? null} startOpen={!t.taskId} />{/if}
      {#if t.taskId}
        <RuntimeTaskOutput {sessionId} taskId={t.taskId} live={t.status === 'running'} />
      {:else if !launch}
        <p class="tat-meta">The agent reported no output for it.</p>
      {/if}
    </div>
  {/if}
{/snippet}

{#if any}
  <section class="tat" class:drilled={opened && !threads.inspectorCompact} aria-label="Agents and tasks" data-testid="thread-agents-tasks" tabindex="-1" bind:this={el}>
    {#if opened && !threads.inspectorCompact}
      <div class="tat-detail" data-testid="agents-tasks-detail">
        <header class="tat-detail-head">
          <button type="button" class="icon-btn" aria-label="Back to agents and tasks" title="Back" data-testid="agents-tasks-back" onclick={() => (openId = null)}>
            <Icon name="chevronLeft" size={13} />
          </button>
          <h3 class="tat-head ellipsis" dir="auto" title={opened.label}>{opened.label}</h3>
        </header>
        {@render detail(opened)}
      </div>
    {:else}
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
    {/if}
  </section>
{/if}

{#if opened && threads.inspectorCompact}
  <Modal title={opened.label} width={640} onclose={() => (openId = null)}>
    <div class="tat-detail" data-testid="agents-tasks-detail">{@render detail(opened)}</div>
  </Modal>
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
  .tat.drilled {
    max-block-size: 70vh;
  }
  .tat-row {
    display: flex;
    align-items: center;
    gap: 6px;
    inline-size: 100%;
    min-width: 0;
    min-block-size: 30px;
    padding: 2px 6px;
    border: 1px solid transparent;
    border-radius: var(--radius-m);
    background: none;
    font: inherit;
    font-size: var(--fs-s);
    color: var(--text);
    text-align: start;
    cursor: pointer;
  }
  .tat-row:hover {
    background: var(--hover);
  }
  .tat-row:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 1px;
  }
  .tat-row.failed {
    color: var(--danger);
  }
  .tat-text {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .tat-cmd {
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .tat-card {
    min-width: 0;
  }
  .tat-detail {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
  }
  .tat-detail-head {
    display: flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
  }
  .tat-detail-head .tat-head {
    margin: 0;
    min-width: 0;
  }
  .tat-cmd-detail {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
  }
  .tat-command {
    margin: 0;
    padding: 6px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
    color: var(--text);
    font-size: var(--fs-xs);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
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
