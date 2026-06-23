<script lang="ts">
  // Per-project Kanban: columns by task status, cards = tasks. Move status,
  // reassign, run now, delete via a card menu. Add task + Plan-from-goal.
  // Cards support HTML5 drag-and-drop to change status columns.
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import StoryLinkCard from './StoryLinkCard.svelte';
  import { swarm } from '../../lib/stores/swarm.svelte';
  import { isAbortError } from '../../lib/api/client';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { TASK_COLUMNS, type SwarmTask, type TaskStatus } from './types';

  // `onrecruit` lets the board surface the existing Recruiter (the agent
  // configurator) without owning its modal — SwarmPage flips `showRecruit`.
  let { onrecruit }: { onrecruit?: () => void } = $props();

  const projects = $derived(swarm.detail?.projects ?? []);
  // Fall back to the first project so the board is never wedged when
  // `selectedProjectId` is null but projects exist (a `<select bind:value>`
  // shows option 0 visually without writing it back to the model).
  const pid = $derived(swarm.selectedProjectId ?? projects[0]?.id ?? null);
  const tasks = $derived(swarm.tasks(pid));
  const agents = $derived(swarm.detail?.agents ?? []);
  // Selected project object (needed for the story back-link card + goal view).
  const selectedProject = $derived(projects.find((p) => p.id === pid) ?? null);
  const goal = $derived((selectedProject?.goal_md ?? '').trim());

  // Reconcile the bound model with what's rendered: when projects exist but
  // `selectedProjectId` is unset or stale (not in the list), pin it to the first
  // project. This persists the choice and keeps the toolbar `<select>` in sync,
  // so Plan-from-goal / Add-task are enabled whenever a project is present.
  $effect(() => {
    if (!projects.length) return;
    const cur = swarm.selectedProjectId;
    if (!cur || !projects.some((p) => p.id === cur)) {
      swarm.selectedProjectId = projects[0].id;
    }
  });

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

  // --- Bulk selection -------------------------------------------------------
  let selected = $state<Set<string>>(new Set());
  const selectedCount = $derived(selected.size);
  const selectedTasks = $derived(tasks.filter((t) => selected.has(t.id)));

  function toggleSelect(id: string) {
    const n = new Set(selected);
    if (n.has(id)) n.delete(id);
    else n.add(id);
    selected = n;
  }
  function clearSelection() {
    selected = new Set();
  }
  function bulkMoveMenu(e: MouseEvent) {
    ctxMenu.show(
      e,
      TASK_COLUMNS.map((s) => ({ label: `Move to ${COLUMN_LABEL[s]}`, action: () => bulkMove(s) })),
    );
  }
  function bulkAssignMenu(e: MouseEvent) {
    ctxMenu.show(e, [
      { label: 'Unassign', action: () => bulkAssign(null) },
      { separator: true },
      ...agents.map((a) => ({ label: a.name, action: () => bulkAssign(a.id) })),
    ]);
  }
  async function bulkMove(status: TaskStatus) {
    await swarm.bulkUpdateTasks(selectedTasks, { status });
    clearSelection();
  }
  async function bulkAssign(agentId: string | null) {
    await swarm.bulkUpdateTasks(selectedTasks, { assignee_agent_id: agentId });
    clearSelection();
  }
  async function bulkDelete() {
    const n = selectedTasks.length;
    if (!n) return;
    if (await confirmer.ask(`Delete ${n} selected task${n === 1 ? '' : 's'}?`, { title: 'Delete tasks' })) {
      await swarm.bulkDeleteTasks(selectedTasks);
      clearSelection();
    }
  }
  async function clearBoard() {
    if (!pid || !tasks.length) return;
    if (await confirmer.ask(`Delete ALL ${tasks.length} tasks on this board? This cannot be undone.`, { title: 'Clear board' })) {
      await swarm.bulkDeleteTasks(tasks);
      clearSelection();
    }
  }

  let adding = $state(false);
  let newTitle = $state('');
  async function addTask() {
    if (!pid || !newTitle.trim()) return;
    await swarm.createTask(pid, { title: newTitle.trim(), priority: 'medium' });
    newTitle = '';
    adding = false;
  }

  // -- Goal view + edit ------------------------------------------------------
  // The goal was previously write-once (only the New-project modal). Here it can
  // be viewed at the top of the board and edited via swarm.updateProject.
  let editingGoal = $state(false);
  let goalDraft = $state('');
  let savingGoal = $state(false);

  function openGoalEditor() {
    if (!pid) return;
    goalDraft = selectedProject?.goal_md ?? '';
    editingGoal = true;
  }

  async function saveGoal() {
    if (!pid) return;
    savingGoal = true;
    try {
      await swarm.updateProject(pid, { goal_md: goalDraft.trim() });
      editingGoal = false;
      toasts.success('Goal saved');
    } catch (e) {
      toasts.error('Save failed', e instanceof Error ? e.message : String(e));
    } finally {
      savingGoal = false;
    }
  }

  // Planning runs multiple planner agents + a summarizer and can take a few
  // minutes, so surface an in-progress state with a Stop. Stop abandons the
  // wait (the run may still finish server-side and tasks appear on next refresh).
  let planning = $state(false);
  let planCtl: AbortController | null = null;

  async function planFromGoal() {
    if (!pid || planning) return;
    // No goal yet → guide the user to set one instead of firing a request the
    // backend rejects with "project has no goal to plan".
    if (!goal) {
      toasts.info('Set a goal first', 'Plan from goal needs a project goal.');
      openGoalEditor();
      return;
    }
    planCtl = new AbortController();
    planning = true;
    try {
      await swarm.plan(pid, planCtl.signal);
      toasts.success('Planner created tasks');
    } catch (e) {
      if (isAbortError(e)) {
        toasts.info('Plan stopped', 'The planner may still finish in the background.');
      } else {
        toasts.error('Plan failed', e instanceof Error ? e.message : String(e));
      }
    } finally {
      planning = false;
      planCtl = null;
    }
  }

  function stopPlan() {
    // Kill the live planner session(s) server-side, then abandon the UI wait.
    if (swarm.detail) void swarm.stopAgentRun(swarm.detail.id);
    planCtl?.abort();
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

  // --- Drag-and-drop state --------------------------------------------------
  // draggingId: the task id being dragged; dropCol: the column being hovered over.
  let draggingId = $state<string | null>(null);
  let dropCol = $state<TaskStatus | null>(null);

  function onDragStart(e: DragEvent, t: SwarmTask) {
    draggingId = t.id;
    e.dataTransfer?.setData('text/plain', t.id);
    if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move';
  }

  function onDragEnd() {
    draggingId = null;
    dropCol = null;
  }

  function onDragOver(e: DragEvent, col: TaskStatus) {
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
    dropCol = col;
  }

  function onDragLeave(col: TaskStatus) {
    if (dropCol === col) dropCol = null;
  }

  async function onDrop(e: DragEvent, col: TaskStatus) {
    e.preventDefault();
    dropCol = null;
    const tid = e.dataTransfer?.getData('text/plain') ?? draggingId;
    draggingId = null;
    if (!tid) return;
    const t = tasks.find((x) => x.id === tid);
    if (!t || t.status === col) return;
    try {
      await swarm.updateTask(t, { status: col });
    } catch (err) {
      toasts.error('Move failed', err instanceof Error ? err.message : String(err));
    }
  }
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
    {#if onrecruit}
      <button class="btn small" onclick={onrecruit} title="Let the Recruiter propose an agent — role, soul & skills — for you to edit and hire">
        <Icon name="plus" size={13} /> Recruit agent
      </button>
    {/if}
    <button class="btn small" onclick={openGoalEditor} disabled={!pid} title="View or edit the project goal">
      <Icon name="note" size={13} /> {goal ? 'Edit goal' : 'Set goal'}
    </button>
    {#if planning}
      <span class="planning"><span class="spinner-xs"></span> Planning… <span class="dim">watch live in Runs</span></span>
      <button class="btn small" onclick={stopPlan} title="Stop waiting for the planner">
        <Icon name="x" size={13} /> Stop
      </button>
    {:else}
      <button class="btn small" onclick={planFromGoal} disabled={!pid} title="Break the project goal into tasks with multiple planner agents + a summarizer">
        <Icon name="zap" size={13} /> Plan from goal
      </button>
    {/if}
    {#if pid && tasks.length}
      <button class="btn small ghost" onclick={clearBoard} title="Delete every task on this board">
        <Icon name="trash" size={13} /> Clear board
      </button>
    {/if}
    <button class="btn small primary" onclick={() => (adding = !adding)} disabled={!pid}>
      <Icon name="plus" size={13} /> Add task
    </button>
  </div>

  {#if selectedCount > 0}
    <div class="bulk-bar">
      <span class="bulk-count">{selectedCount} selected</span>
      <button class="btn small" onclick={bulkMoveMenu}><Icon name="split" size={13} /> Move to…</button>
      <button class="btn small" onclick={bulkAssignMenu}><Icon name="user" size={13} /> Assign…</button>
      <button class="btn small danger" onclick={bulkDelete}><Icon name="trash" size={13} /> Delete</button>
      <span class="grow"></span>
      <button class="btn small ghost" onclick={clearSelection}>Clear selection</button>
    </div>
  {/if}

  {#if pid}
    <div class="goal-bar" class:empty={!goal}>
      <Icon name="zap" size={12} />
      {#if goal}
        <span class="goal-text" title={goal}>{goal}</span>
        <button class="link" onclick={openGoalEditor}>Edit</button>
      {:else}
        <span class="goal-text dim">No goal set — <button class="link" onclick={openGoalEditor}>set a goal</button> to use Plan from goal.</span>
      {/if}
    </div>
  {/if}

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
    <!-- Story back-link: shown when this project was seeded from a Product story. -->
    {#if selectedProject?.story_id}
      <StoryLinkCard project={selectedProject} />
    {/if}
    <div class="columns">
      {#each TASK_COLUMNS as col (col)}
        <div
          class="column"
          role="group"
          class:drop-target={dropCol === col}
          ondragover={(e) => onDragOver(e, col)}
          ondragleave={() => onDragLeave(col)}
          ondrop={(e) => onDrop(e, col)}
        >
          <div class="col-head">
            <span>{COLUMN_LABEL[col]}</span>
            <span class="count">{byStatus(col).length}</span>
          </div>
          <div class="col-body">
            {#each byStatus(col) as t (t.id)}
              {@const agent = swarm.agentById(t.assignee_agent_id)}
              <div
                class="card"
                class:dragging={draggingId === t.id}
                class:selected={selected.has(t.id)}
                draggable="true"
                ondragstart={(e) => onDragStart(e, t)}
                ondragend={onDragEnd}
                oncontextmenu={(e) => cardMenu(e, t)}
                role="button"
                tabindex="0"
              >
                <div class="card-title">
                  <input
                    type="checkbox"
                    class="card-sel"
                    checked={selected.has(t.id)}
                    onclick={(e) => e.stopPropagation()}
                    onchange={() => toggleSelect(t.id)}
                    aria-label="Select task"
                  />
                  <span>{t.title}</span>
                </div>
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

{#if editingGoal}
  <Modal title={goal ? 'Edit project goal' : 'Set project goal'} width={560} onclose={() => (editingGoal = false)}>
    <div class="field">
      <label for="goal-md">Goal</label>
      <textarea
        id="goal-md"
        class="input"
        rows={8}
        bind:value={goalDraft}
        placeholder="Describe what this project should achieve. Plan from goal turns this into tasks."
      ></textarea>
    </div>
    {#snippet footer()}
      <button class="btn ghost" onclick={() => (editingGoal = false)}>Cancel</button>
      <button class="btn primary" onclick={saveGoal} disabled={savingGoal}>{savingGoal ? 'Saving…' : 'Save'}</button>
    {/snippet}
  </Modal>
{/if}

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
  .goal-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    font-size: 12px;
    color: var(--text-dim);
    background: color-mix(in srgb, var(--accent) 6%, transparent);
    border-bottom: 1px solid var(--border);
  }
  .goal-bar.empty {
    background: transparent;
  }
  .goal-text {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .link {
    background: none;
    border: none;
    color: var(--accent);
    cursor: pointer;
    padding: 0;
    font: inherit;
  }
  .link:hover {
    text-decoration: underline;
  }
  .planning {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--text-dim);
  }
  .spinner-xs {
    width: 11px;
    height: 11px;
    border: 2px solid color-mix(in srgb, var(--accent) 35%, transparent);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: kb-spin 0.7s linear infinite;
  }
  @keyframes kb-spin {
    to {
      transform: rotate(360deg);
    }
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .field label {
    font-size: 12px;
    color: var(--text-dim);
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
    /* Allow the card list to scroll within the column instead of growing the
       column past the viewport (the flex-child height-collapse rule). */
    min-height: 0;
  }
  .card {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 8px;
    cursor: grab;
  }
  .card:hover {
    border-color: color-mix(in srgb, var(--accent) 40%, var(--border));
  }
  .card.dragging {
    opacity: 0.4;
    cursor: grabbing;
  }
  .card.selected {
    border-color: var(--accent);
    box-shadow: inset 0 0 0 1px var(--accent);
  }
  .card-sel {
    margin-inline-end: 6px;
    vertical-align: middle;
    cursor: pointer;
  }
  .bulk-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    border-bottom: 1px solid var(--border);
    background: color-mix(in srgb, var(--accent) 10%, transparent);
    font-size: 12px;
  }
  .bulk-count {
    font-weight: 600;
    color: var(--accent);
  }
  .column.drop-target {
    background: color-mix(in srgb, var(--accent) 8%, var(--surface-2));
    border-color: color-mix(in srgb, var(--accent) 50%, var(--border));
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
