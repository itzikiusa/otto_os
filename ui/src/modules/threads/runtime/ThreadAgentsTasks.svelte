<script lang="ts">
  // Thread details' "Agents & tasks": every subagent, linked child thread, and
  // background task of the focused runtime thread (agentTasks.ts) — running
  // first with live step/log preview and elapsed time, finished folded below.
  // Clicking any row drills down to inspect its prompt/command, live tool steps,
  // streaming stdout/stderr logs, final output, or open its child thread.
  import { tick, untrack } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import type { IconName } from '../../../lib/components/Icon.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import { scrollBehavior } from '../../../lib/motion';
  import { sessionProviderSlug } from '../../../lib/providers';
  import { now } from '../../../lib/stores/now.svelte';
  import Markdown from '../../agents/conversation/Markdown.svelte'; // ui-guards: allow — shared chat markdown
  import { threads } from '../threadsStore.svelte';
  import {
    agentTasks,
    taskElapsed,
    type AgentTask,
    type LinkedChildThreadSummary,
  } from './agentTasks';
  import RuntimeWorkRow from './RuntimeWorkRow.svelte';
  import { runtimeThreads } from './runtimeThreads.svelte';
  import { runtimeViews } from './runtimeView.svelte';
  import { subagentChildren, workLabel } from './timeline';

  let { sessionId }: { sessionId: string } = $props();

  /** Finished tasks shown when the fold opens (newest first). */
  const FINISHED_SHOWN = 8;
  let showAllFinished = $state(false);
  let maxBlockPx = $state(340);

  $effect(() => {
    if (typeof window === 'undefined') return;
    const syncHeight = (): void => {
      maxBlockPx = Math.max(160, Math.floor((window.innerHeight || 720) * 0.48));
    };
    syncHeight();
    window.addEventListener('resize', syncHeight);
    return () => window.removeEventListener('resize', syncHeight);
  });

  const p = $derived(runtimeViews.projections[sessionId] ?? null);
  const childThreads = $derived.by((): LinkedChildThreadSummary[] => {
    const out: LinkedChildThreadSummary[] = [];
    const all = Object.values(runtimeThreads.byId);
    for (let i = 0; i < all.length; i++) {
      const s = all[i];
      if (s.parent_id !== sessionId) continue;
      const cp = runtimeViews.projections[s.id];
      const lastItem = cp?.items.findLast((it) => it.type !== 'user_message');
      const stepSummary = lastItem
        ? lastItem.type === 'assistant_message'
          ? (lastItem.text.trim().split('\n')[0]?.slice(0, 120) ?? null)
          : workLabel(lastItem)
        : null;
      out.push({
        id: s.id,
        title: s.title,
        provider: sessionProviderSlug(s),
        model: s.model ?? null,
        status: s.status,
        createdAt: s.created_at,
        updatedAt: s.last_active_at,
        stepSummary,
      });
    }
    return out;
  });

  const tasks = $derived(
    agentTasks(
      p?.items ?? [],
      p?.session?.background_tasks ?? runtimeThreads.get(sessionId)?.background_tasks ?? [],
      { output: p?.output ?? {}, childThreads },
    ),
  );
  const childrenMap = $derived(subagentChildren(p?.items ?? []));
  const any = $derived(tasks.running.length + tasks.finished.length > 0);
  const nowMs = $derived(tasks.running.length ? now() : Date.now());

  let expandedIds = $state<Record<string, boolean>>({});
  let attemptedHydration = new Set<string>();
  let lastSessionId = untrack(() => sessionId);
  $effect(() => {
    if (sessionId !== lastSessionId) {
      lastSessionId = sessionId;
      expandedIds = {};
      attemptedHydration = new Set<string>();
    }
  });
  function toggleExpand(id: string): void {
    expandedIds = { ...expandedIds, [id]: !expandedIds[id] };
  }

  // Hydrate omitted TurnItem details and keep expanded child thread projections live.
  $effect(() => {
    const allRows = [...tasks.running, ...tasks.finished];
    const releases: Array<() => void> = [];
    for (let i = 0; i < allRows.length; i++) {
      const r = allRows[i];
      if (!expandedIds[r.id]) continue;
      if (r.itemId) {
        const hKey = `${sessionId}:${r.itemId}`;
        if (r.detailOmitted) {
          void runtimeViews.ensureItemDetail(sessionId, r.itemId);
        } else if (!r.prompt && !r.output && !r.error && r.status !== 'running' && !attemptedHydration.has(hKey)) {
          attemptedHydration.add(hKey);
          void runtimeViews.ensureItemDetail(sessionId, r.itemId, { force: true });
        }
      }
      if (r.childSessionId) {
        releases.push(runtimeViews.open(r.childSessionId));
      }
    }
    return () => {
      for (let i = 0; i < releases.length; i++) releases[i]();
    };
  });

  function scrollToTimelineCard(itemId: string): void {
    if (typeof document === 'undefined') return;
    const esc = typeof CSS !== 'undefined' && CSS.escape ? CSS.escape(itemId) : itemId;
    const target =
      document.querySelector<HTMLElement>(`[data-testid="runtime-subagent-${esc}"]`) ??
      document.querySelector<HTMLElement>(`[data-item-id="${esc}"]`);
    target?.scrollIntoView({ behavior: scrollBehavior(), block: 'center' });
  }

  let el = $state<HTMLElement | null>(null);
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

  function kindIcon(kind: string): IconName {
    if (kind === 'agent') return 'assistant';
    if (kind === 'timer') return 'clock';
    return 'terminal';
  }
</script>

{#snippet row(t: AgentTask)}
  {@const e = taskElapsed(t, nowMs)}
  {@const open = Boolean(expandedIds[t.id])}
  {@const subLine = [t.sublabel, t.stepSummary].filter(Boolean).join(' · ')}
  {@const activity = t.itemId
    ? (childrenMap.get(t.itemId) ?? [])
    : t.childSessionId
      ? (runtimeViews.projections[t.childSessionId]?.items.filter((it) => it.type !== 'user_message').slice(-10) ?? [])
      : []}
  {@const liveOut = (t.itemId ? p?.output?.[t.itemId] : null) || t.output}
  {@const detailKey = t.itemId ? `${sessionId}:${t.itemId}` : ''}
  {@const detailLoading = Boolean(detailKey && runtimeViews.loadingItem[detailKey])}
  {@const detailError = detailKey ? (runtimeViews.itemError[detailKey] ?? null) : null}
  <li
    class="tat-item"
    class:failed={t.status === 'failed'}
    class:expanded={open}
    data-testid="agents-tasks-row"
    data-status={t.status}
    data-kind={t.kind}
  >
    <button
      type="button"
      class="tat-row-btn"
      aria-expanded={open}
      onclick={() => toggleExpand(t.id)}
      data-testid="agents-tasks-toggle"
    >
      <span class="tat-main">
        {#if t.status === 'running'}
          <span class="spinner" aria-hidden="true"></span>
        {:else}
          <Icon name={STATUS[t.status].icon} size={12} />
        {/if}
        <Icon name={kindIcon(t.kind)} size={12} />
        <span class="ellipsis" dir="auto" title={t.label}>{t.label}</span>
        <span class="tat-meta">
          {STATUS[t.status].label}{t.background && t.status === 'running' ? ' · bg' : ''}{e ? ` · ${e}` : ''}
        </span>
        <span class="tat-chev" aria-hidden="true">
          <Icon name={open ? 'chevronDown' : 'chevronRight'} size={12} />
        </span>
      </span>
      {#if subLine}
        <span class="tat-sub ellipsis" dir="auto" title={subLine}>{subLine}</span>
      {/if}
    </button>

    {#if open}
      <div class="tat-detail" data-testid="agents-tasks-detail">
        {#if t.childSessionId || t.itemId}
          <div class="tat-actions">
            {#if t.childSessionId}
              {@const cid = t.childSessionId}
              <button
                type="button"
                class="tat-act-btn"
                onclick={() => threads.openBeside(cid)}
              >
                <Icon name="split" size={12} />
                Open beside
              </button>
              <button
                type="button"
                class="tat-act-btn"
                onclick={() => threads.selectThread(cid)}
              >
                <Icon name="arrow" size={12} />
                Open thread
              </button>
            {:else if t.itemId && t.kind === 'agent'}
              {@const iid = t.itemId}
              <button
                type="button"
                class="tat-act-btn"
                onclick={() => scrollToTimelineCard(iid)}
              >
                <Icon name="arrow" size={12} />
                Show in chat
              </button>
            {/if}
          </div>
        {/if}

        {#if detailLoading || detailError}
          <LoadState
            variant="compact"
            what="task details"
            loading={detailLoading}
            error={detailError}
            empty
            rows={1}
            onretry={t.itemId ? () => void runtimeViews.ensureItemDetail(sessionId, t.itemId!, { force: true }) : undefined}
          />
        {/if}

        {#if t.prompt}
          <p class="tat-sec-label">{t.kind === 'command' ? 'Command' : t.kind === 'timer' ? 'Schedule' : 'Task'}</p>
          <pre class="tat-prompt" dir="auto">{t.prompt}</pre>
        {/if}

        {#if activity.length > 0}
          <p class="tat-sec-label">Activity ({activity.length})</p>
          <div class="tat-activity">
            {#each activity as a (a.id)}
              {#if a.type === 'assistant_message'}
                <div class="tat-md" dir="auto"><Markdown md={a.text} /></div>
              {:else}
                <RuntimeWorkRow
                  sessionId={t.childSessionId ?? sessionId}
                  item={a}
                  output={(t.childSessionId ? runtimeViews.projections[t.childSessionId]?.output : p?.output)?.[a.id]}
                />
              {/if}
            {/each}
          </div>
        {/if}

        {#if t.error}
          <p class="tat-sec-label">Error</p>
          <pre class="tat-log is-error" dir="auto">{t.error}</pre>
        {:else if liveOut}
          <p class="tat-sec-label">{t.kind === 'agent' ? 'Result' : 'Output'}</p>
          {#if t.kind === 'agent'}
            <div class="tat-md" dir="auto"><Markdown md={liveOut} /></div>
          {:else}
            <pre class="tat-log" dir="ltr">{liveOut}</pre>
          {/if}
        {:else if activity.length === 0 && !t.prompt}
          <p class="tat-empty-note">
            {t.kind === 'timer'
              ? 'Scheduled timer — resumes thread automatically when due.'
              : t.status === 'running'
                ? 'Running in background — waiting for live steps or output…'
                : 'Completed with no captured output.'}
          </p>
        {/if}
      </div>
    {/if}
  </li>
{/snippet}

{#if any}
  <section
    class="tat"
    style:--tat-max="{maxBlockPx}px"
    aria-label="Agents and tasks"
    data-testid="thread-agents-tasks"
    tabindex="-1"
    bind:this={el}
  >
    <h3 class="tat-head">
      Agents &amp; tasks
      {#if tasks.running.length}<span class="tat-count">{tasks.running.length} running</span>{/if}
    </h3>
    {#if tasks.running.length}
      <ul class="tat-list">{#each tasks.running as t (t.id)}{@render row(t)}{/each}</ul>
    {/if}
    {#if tasks.finished.length}
      {@const visibleFinished = showAllFinished ? tasks.finished : tasks.finished.slice(0, FINISHED_SHOWN)}
      <details class="tat-done">
        <summary>{tasks.finished.length} finished</summary>
        <ul class="tat-list">{#each visibleFinished as t (t.id)}{@render row(t)}{/each}</ul>
        {#if tasks.finished.length > FINISHED_SHOWN}
          <button
            type="button"
            class="btn small ghost tat-more"
            onclick={() => (showAllFinished = !showAllFinished)}
            data-testid="thread-agents-tasks-more"
          >
            {showAllFinished ? 'Show fewer' : `Show all (${tasks.finished.length})`}
          </button>
        {/if}
      </details>
    {/if}
  </section>
{/if}

<style>
  .tat {
    flex-shrink: 0;
    max-block-size: var(--tat-max, 340px);
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
    font-variant-numeric: tabular-nums;
  }
  .tat-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .tat-item {
    display: flex;
    flex-direction: column;
    min-inline-size: 0;
    border-radius: var(--radius-s);
  }
  .tat-item.expanded {
    background: var(--surface-2);
  }
  .tat-row-btn {
    appearance: none;
    border: none;
    background: transparent;
    color: var(--text);
    font: inherit;
    text-align: start;
    cursor: pointer;
    padding-block: 4px;
    padding-inline: 6px;
    border-radius: var(--radius-s);
    display: flex;
    flex-direction: column;
    gap: 2px;
    inline-size: 100%;
    min-inline-size: 0;
  }
  .tat-row-btn:hover {
    background: var(--hover);
  }
  .tat-row-btn:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
  }
  .tat-item.failed .tat-row-btn {
    color: var(--danger);
  }
  .tat-main {
    display: flex;
    align-items: center;
    gap: 6px;
    min-inline-size: 0;
    min-block-size: 20px;
    font-size: var(--fs-s);
  }
  .tat-main .ellipsis {
    flex: 1;
    min-inline-size: 0;
  }
  .tat-main .spinner {
    --spinner-size: 11px;
  }
  .tat-meta {
    flex-shrink: 0;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
  }
  .tat-more {
    margin-block-start: 4px;
  }
  .tat-chev {
    flex-shrink: 0;
    display: inline-flex;
    color: var(--text-dim);
  }
  .tat-sub {
    padding-inline-start: 28px;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    min-inline-size: 0;
  }
  .tat-detail {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding-block: 4px 8px;
    padding-inline: 12px 8px;
    margin-inline-start: 14px;
    border-inline-start: 2px solid var(--border);
    min-inline-size: 0;
  }
  .tat-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .tat-act-btn {
    appearance: none;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text);
    font-size: var(--fs-xs);
    padding-block: 2px;
    padding-inline: 8px;
    border-radius: var(--radius-s);
    display: inline-flex;
    align-items: center;
    gap: 4px;
    cursor: pointer;
  }
  .tat-act-btn:hover {
    background: var(--hover);
  }
  .tat-act-btn:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 1px;
  }
  .tat-sec-label {
    margin: 4px 0 0;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }
  .tat-prompt,
  .tat-log {
    margin: 0;
    padding: 6px 8px;
    border-radius: var(--radius-s);
    background: var(--surface);
    border: 1px solid var(--border);
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    color: var(--text);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    max-block-size: 160px;
    overflow-y: auto;
  }
  .tat-log.is-error {
    color: var(--danger);
    border-color: var(--danger);
  }
  .tat-activity {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-inline-size: 0;
    max-block-size: 200px;
    overflow-y: auto;
  }
  .tat-md {
    min-inline-size: 0;
    font-size: var(--fs-s);
    color: var(--text);
    max-block-size: 180px;
    overflow-y: auto;
  }
  .tat-empty-note {
    margin: 0;
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
