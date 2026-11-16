<!--
  Stop for one running row of Agents & tasks. A background task stops through
  the daemon's per-task route; a child thread through what it actually runs
  (its turn and/or its background work). `part="button"` sits on the row's
  line, `part="note"` under it: "Stopping…", "Asked the agent to stop it",
  "Couldn’t stop: <why>" (the button then reads Retry). Nothing shows on a
  daemon without the route, nor for rows nothing can stop (a swarm member,
  a turn's own subagent — the composer's Stop ends that turn).
-->
<script lang="ts">
  import Icon from '../../../lib/components/Icon.svelte';
  import { toastError } from '../../../lib/toastError';
  import type { AgentTask } from './agentTasks';
  import { runtimeThreads } from './runtimeThreads.svelte';
  import { rowStop, type RowStop } from './taskStop';
  import { stopThreadWork, taskStops } from './taskStops.svelte';

  interface Props {
    part: 'button' | 'note';
    sessionId: string;
    task: AgentTask;
  }
  let { part, sessionId, task }: Props = $props();

  let childBusy = $state(false);
  const child = $derived(task.childSessionId ? runtimeThreads.get(task.childSessionId) : null);
  const view = $derived.by((): RowStop | null => {
    if (task.status !== 'running') return null;
    if (task.stopId) {
      return rowStop({ stopping: task.stopping, stop_error: task.stopError }, taskStops.get(sessionId, task.stopId), taskStops.unsupported);
    }
    if (task.childSessionId && child && child.status !== 'closed') {
      return childBusy ? { canStop: false, busy: true, note: 'Stopping…', failed: false } : { canStop: true, busy: false, note: null, failed: false };
    }
    return null;
  });

  async function stop(): Promise<void> {
    if (task.stopId) return taskStops.stop(sessionId, task.stopId);
    const c = child;
    if (!c || childBusy) return;
    childBusy = true;
    try {
      const turnRunning = c.status === 'working' || c.status === 'starting' || c.status === 'awaiting_permission';
      await stopThreadWork(c.id, { turnRunning, taskIds: (c.background_tasks ?? []).map((b) => b.id) });
    } catch (e) {
      toastError('Couldn’t stop that thread', e);
    } finally {
      childBusy = false;
    }
  }
</script>

{#if view && part === 'button'}
  {#if view.busy}
    <span class="tst-busy" data-testid="agents-tasks-stopping">
      <span class="spinner" aria-hidden="true"></span>
      <span class="tst-busy-text">Stopping…</span>
    </span>
  {:else if view.canStop}
    <button
      type="button"
      class="tst-btn"
      class:retry={view.failed}
      aria-label="{view.failed ? 'Retry stopping' : 'Stop'} {task.label}"
      title={view.failed ? `Try stopping ${task.label} again` : `Stop ${task.label}`}
      data-testid="agents-tasks-stop"
      onclick={() => void stop()}
    >
      <Icon name={view.failed ? 'refresh' : 'stop'} size={12} />
      <span>{view.failed ? 'Retry' : 'Stop'}</span>
    </button>
  {/if}
{:else if view && part === 'note' && view.note && !view.busy}
  <p class="tst-note" class:failed={view.failed} dir="auto" data-testid="agents-tasks-stop-note" role={view.failed ? 'alert' : 'status'}>
    {view.note}
  </p>
{/if}

<style>
  .tst-btn {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    min-block-size: 24px;
    padding-inline: 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .tst-btn:hover {
    background: var(--hover);
  }
  .tst-btn.retry {
    color: var(--danger);
    border-color: var(--danger);
  }
  .tst-btn:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 1px;
  }
  .tst-busy {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .tst-busy .spinner {
    --spinner-size: 10px;
  }
  .tst-note {
    margin: 0;
    padding-inline-start: 32px;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    overflow-wrap: anywhere;
  }
  .tst-note.failed {
    color: var(--danger);
  }
</style>
