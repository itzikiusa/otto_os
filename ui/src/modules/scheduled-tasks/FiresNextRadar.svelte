<script lang="ts">
  // Unified "Fires next" radar + one-click "Pause all automations…" kill switch
  // across workspace scheduled tasks and personal agent routines (E-15).
  import Badge from '../../lib/components/Badge.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import RelTime from '../../lib/components/RelTime.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { router } from '../../lib/router.svelte';
  import { personalAgents } from '../../lib/stores/personalAgents.svelte';
  import { scheduledTasks } from '../../lib/stores/scheduledTasks.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { toastError } from '../../lib/toastError';
  import type { ScheduledTask } from '../../lib/api/types';
  import { buildFiresNext, countActiveAutomations } from './firesNext';

  interface Props {
    wsId: string;
    tasks: readonly ScheduledTask[];
    onopentask: (id: string) => void;
  }
  let { wsId, tasks, onopentask }: Props = $props();

  let pausingAll = $state(false);
  let showAll = $state(false);

  $effect(() => {
    const id = wsId;
    if (!id) return;
    if (personalAgents.agents.length === 0 && !personalAgents.loadingAgents) {
      void personalAgents.loadAgents(id);
    }
  });

  const wsAgents = $derived(personalAgents.agents.filter((a) => a.workspace_id === wsId));
  const upcoming = $derived(buildFiresNext(tasks, wsAgents, personalAgents.schedulesByAgent, 6));
  const active = $derived(countActiveAutomations(tasks, wsAgents, personalAgents.schedulesByAgent));

  async function pauseAll(): Promise<void> {
    if (pausingAll || active.total === 0) return;
    const parts: string[] = [];
    if (active.tasks > 0) parts.push(`${active.tasks} scheduled ${active.tasks === 1 ? 'task' : 'tasks'}`);
    if (active.agents > 0) parts.push(`${active.agents} personal ${active.agents === 1 ? 'agent' : 'agents'}`);
    const ok = await confirmer.ask(
      `Pause ${parts.join(' and ')} in this workspace? Scheduled runs and agent routines will stop firing until you resume them individually.`,
      { title: 'Pause all automations?', confirmLabel: 'Pause all', danger: true },
    );
    if (!ok) return;
    pausingAll = true;
    try {
      const taskTargets = tasks.filter((t) => t.enabled);
      const agentTargets = wsAgents.filter((a) => a.enabled);
      await Promise.all([
        ...taskTargets.map((t) => scheduledTasks.setEnabled(t.id, false)),
        ...agentTargets.map((a) => personalAgents.update(a.id, { enabled: false })),
      ]);
      toasts.success('Paused all automations', `${taskTargets.length + agentTargets.length} paused.`);
    } catch (e) {
      toastError('Couldn’t pause all automations', e);
    } finally {
      pausingAll = false;
    }
  }
</script>

{#if upcoming.length > 0 || active.total > 0}
  <section class="radar" aria-label="Fires next" data-testid="fires-next-radar">
    <div class="radar-head">
      <Icon name="clock" size={13} />
      <h3>Fires next</h3>
      <!-- The tasks are listed right below: say what fires first, fold the rest. -->
      <span class="radar-count" dir="auto">{#if upcoming.length}{upcoming[0].title} · <RelTime iso={upcoming[0].nextAt} />{:else}No upcoming runs scheduled{/if}</span>
      <span class="grow"></span>
      {#if active.total > 0}
        <button
          type="button"
          class="btn small danger"
          disabled={pausingAll}
          onclick={() => void pauseAll()}
          title="Pause every enabled scheduled task and personal agent in this workspace"
          data-testid="pause-all-automations"
        >
          <Icon name="pause" size={12} /> {pausingAll ? 'Pausing…' : 'Pause all automations…'}
        </button>
      {/if}
    </div>
    {#if upcoming.length > 1}
      <button type="button" class="radar-more" aria-expanded={showAll} onclick={() => (showAll = !showAll)}>{showAll ? 'Hide upcoming' : `All ${upcoming.length} upcoming`}</button>
    {/if}
    {#if showAll && upcoming.length > 1}
      <ul class="radar-list">
        {#each upcoming as item (item.id)}
          <li>
            <button
              type="button"
              class="radar-item"
              onclick={() => {
                if (item.kind === 'task') onopentask(item.id.slice('task:'.length));
                else router.go(item.route);
              }}
            >
              <Badge label={item.kind === 'task' ? 'Task' : 'Agent'} tone={item.kind === 'task' ? 'neutral' : 'accent'} />
              <span class="radar-title" dir="auto">{item.title}</span>
              <span class="radar-sub" dir="auto">{item.subtitle}</span>
              <span class="grow"></span>
              <span class="radar-when" title={new Date(item.nextAt).toLocaleString()}><RelTime iso={item.nextAt} /></span>
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </section>
{/if}

<style>
  .radar {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-block-end: 14px;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
  }
  .radar-head {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    color: var(--text-dim);
  }
  .radar-more {
    align-self: flex-start;
    padding: 0;
    border: 0;
    background: none;
    cursor: pointer;
    font-size: var(--fs-s);
    color: var(--accent-text);
  }
  .radar-head h3 {
    margin: 0;
    font-size: var(--fs-s);
    font-weight: 600;
    color: var(--text);
  }
  .radar-count {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .grow { flex: 1; }
  .radar-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .radar-item {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px;
    border: 0;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-s);
    text-align: start;
    cursor: pointer;
  }
  .radar-item:hover {
    background: var(--hover);
  }
  .radar-item:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 1px;
  }
  .radar-title {
    font-weight: 500;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .radar-sub {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    white-space: nowrap;
  }
  .radar-when {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    white-space: nowrap;
  }
</style>
