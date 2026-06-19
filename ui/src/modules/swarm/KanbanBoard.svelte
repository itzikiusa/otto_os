<script lang="ts">
  // Per-project Kanban: columns by task status, cards = tasks. Move status,
  // reassign, run now, delete via a card menu. Add task + Plan-from-goal.
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { swarm } from '../../lib/stores/swarm.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { TASK_COLUMNS, type SwarmTask, type TaskStatus } from './types';

  const projects = $derived(swarm.detail?.projects ?? []);
  const pid = $derived(swarm.selectedProjectId);
  const tasks = $derived(swarm.tasks(pid));
  const agents = $derived(swarm.detail?.agents ?? []);

  const COLUMN_LABEL: Record<TaskStatus, string> = {
    backlog: 'Backlog',
    todo: 'To do',
    in_progress: 'In progress',
    in_review: 'In review',
    blocked: 'Blocked',
    done: 'Done',
    cancelled: 'Cancelled',
  };

  function byStatus(s: TaskStatus): SwarmTask[] {
    return tasks.filter((t) => t.status === s);
  }

  let adding = $state(false);
  let newTitle = $state('');
  async function addTask() {
    if (!pid || !newTitle.trim()) return;
    await swarm.createTask(pid, { title: newTitle.trim(), priority: 'medium' });
    newTitle = '';
    adding = false;
  }

  async function planFromGoal() {
    if (!pid) return;
    try {
      await swarm.plan(pid);
      toasts.success('Planner created tasks');
    } catch (e) {
      toasts.error('Plan failed', e instanceof Error ? e.message : String(e));
    }
  }

  function cardMenu(e: MouseEvent, t: SwarmTask) {
    const moves = TASK_COLUMNS.filter((s) => s !== t.status).map((s) => ({
      label: `Move to ${COLUMN_LABEL[s]}`,
      action: () => swarm.updateTask(t, { status: s }),
    }));
    const assigns = agents.map((a) => ({
      label: `Assign to ${a.name}`,
      action: () => swarm.updateTask(t, { assignee_agent_id: a.id }),
    }));
    ctxMenu.show(e, [
      { label: 'Run now', icon: 'play', action: () => runNow(t) },
      { separator: true },
      ...moves,
      { separator: true },
      ...assigns,
      { separator: true },
      {
        label: 'Delete',
        icon: 'trash',
        danger: true,
        action: async () => {
          if (await confirmer.ask(t.title, { title: 'Delete task?' })) swarm.deleteTask(t);
        },
      },
    ]);
  }

  async function runNow(t: SwarmTask) {
    try {
      await swarm.runTask(t);
      toasts.success('Task queued');
    } catch (e) {
      toasts.error('Run failed', e instanceof Error ? e.message : String(e));
    }
  }

  const PRIORITY_CLASS: Record<string, string> = {
    urgent: 'bad',
    high: 'accent',
    medium: '',
    low: 'dim',
  };
</script>

<div class="kanban">
  <div class="kb-toolbar">
    {#if projects.length > 1}
      <select class="input" bind:value={swarm.selectedProjectId}>
        {#each projects as p (p.id)}
          <option value={p.id}>{p.name}</option>
        {/each}
      </select>
    {:else if projects[0]}
      <span class="section-title">{projects[0].name}</span>
    {/if}
    <span class="grow"></span>
    <button class="btn small" onclick={planFromGoal} disabled={!pid} title="Break the project goal into tasks">
      <Icon name="zap" size={13} /> Plan from goal
    </button>
    <button class="btn small primary" onclick={() => (adding = !adding)} disabled={!pid}>
      <Icon name="plus" size={13} /> Add task
    </button>
  </div>

  {#if adding}
    <div class="add-row">
      <input
        class="input grow"
        placeholder="Task title…"
        bind:value={newTitle}
        onkeydown={(e) => e.key === 'Enter' && addTask()}
      />
      <button class="btn small primary" onclick={addTask}>Add</button>
      <button class="btn small ghost" onclick={() => (adding = false)}>Cancel</button>
    </div>
  {/if}

  {#if !pid}
    <EmptyState icon="note" title="No project selected" body="Create a project to start a board." />
  {:else}
    <div class="columns">
      {#each TASK_COLUMNS as col (col)}
        <div class="column">
          <div class="col-head">
            <span>{COLUMN_LABEL[col]}</span>
            <span class="count">{byStatus(col).length}</span>
          </div>
          <div class="col-body">
            {#each byStatus(col) as t (t.id)}
              {@const agent = swarm.agentById(t.assignee_agent_id)}
              <div class="card" oncontextmenu={(e) => cardMenu(e, t)} role="button" tabindex="0">
                <div class="card-title">{t.title}</div>
                <div class="card-meta">
                  {#if agent}
                    <span class="assignee" title={agent.title}>{agent.avatar || agent.name.slice(0, 1)} {agent.name}</span>
                  {:else}
                    <span class="assignee dim">unassigned</span>
                  {/if}
                  <span class="grow"></span>
                  <span class="chip {PRIORITY_CLASS[t.priority]}">{t.priority}</span>
                  <button class="icon-btn small" onclick={(e) => cardMenu(e, t)} aria-label="task menu">
                    <Icon name="dot" size={14} />
                  </button>
                </div>
                {#if t.delegated}<span class="tag">delegated</span>{/if}
              </div>
            {/each}
          </div>
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .kanban {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .kb-toolbar,
  .add-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    border-bottom: 1px solid var(--border);
  }
  .columns {
    display: flex;
    gap: 10px;
    padding: 10px;
    overflow-x: auto;
    flex: 1;
    min-height: 0;
  }
  .column {
    flex: 0 0 240px;
    display: flex;
    flex-direction: column;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    min-height: 0;
  }
  .col-head {
    display: flex;
    justify-content: space-between;
    padding: 8px 10px;
    font-size: 11.5px;
    font-weight: 600;
    color: var(--text-dim);
    border-bottom: 1px solid var(--border);
  }
  .count {
    background: color-mix(in srgb, var(--text-dim) 18%, transparent);
    border-radius: 999px;
    padding: 0 6px;
  }
  .col-body {
    padding: 8px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    overflow-y: auto;
  }
  .card {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 8px;
    cursor: default;
  }
  .card:hover {
    border-color: color-mix(in srgb, var(--accent) 40%, var(--border));
  }
  .card-title {
    font-size: 12.5px;
    margin-bottom: 6px;
  }
  .card-meta {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 11px;
  }
  .assignee {
    color: var(--text-dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 120px;
  }
  .tag {
    display: inline-block;
    margin-top: 6px;
    font-size: 10px;
    color: var(--text-dim);
    border: 1px solid var(--border);
    border-radius: 999px;
    padding: 0 6px;
  }
</style>
