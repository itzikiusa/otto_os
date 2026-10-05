<script lang="ts">
  import { rowMenu } from '../../lib/rowMenu';
  // Per-project Kanban: columns by task status, cards = tasks. Move status,
  // reassign, run now, delete via a card menu. Add task + Plan-from-goal.
  // Cards support HTML5 drag-and-drop to change status columns.
  import Icon from '../../lib/components/Icon.svelte';
  import { focusOnMount } from '../../lib/focusOnMount';
  import Badge from '../../lib/components/Badge.svelte';
  import { toastError } from '../../lib/toastError';
  import { plural } from '../../lib/plural';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import StoryLinkCard from './StoryLinkCard.svelte';
  import GoalsPanel from './GoalsPanel.svelte';
  import { swarm, type BulkTaskResult } from '../../lib/stores/swarm.svelte';
  import { isAbortError } from '../../lib/api/client';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { sentenceCase } from '../../lib/status';
  import { TASK_COLUMNS, type SwarmProject, type SwarmTask, type TaskStatus, type SwarmWaiting, type TaskWaiting } from './types';
  import { api } from '../../lib/api/client';
  import { rel } from '../../lib/stores/now.svelte';
  import { pollWhileVisible, type Poller } from '../../lib/poll';

  // `onnewproject` / `oneditproject` hand off to SwarmPage, which owns the
  // project create/edit modal (where project skills live). New project lives
  // here — beside the project picker — rather than in the page header.
  let {
    onnewproject,
    oneditproject,
  }: { onnewproject?: () => void; oneditproject?: (p: SwarmProject) => void } = $props();

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

  // Why each ready "To do" card isn't starting (12-mcp W1): the coordinator
  // records a reason per ready task every tick; read it from the in-memory
  // `/waiting` endpoint (not the heavier utilization snapshot) while the swarm is active — on task changes (debounced) and every
  // 15 s while the page is visible. Best effort: a failed read keeps the last
  // reasons and never surfaces an error (the board itself is unaffected).
  let waiting: Record<string, TaskWaiting> = $state({});
  const activeSid = $derived(swarm.detail?.status === 'active' ? swarm.detail.id : null);
  async function loadWaiting(sid: string, signal?: AbortSignal) {
    try {
      const u = await api.get<SwarmWaiting>(`/swarm/swarms/${sid}/waiting`, signal);
      if (sid === activeSid) waiting = u.waiting ?? {};
    } catch {
      /* keep the last reasons */
    }
  }
  let waitingPoller: Poller | null = null;
  $effect(() => {
    const sid = activeSid;
    if (!sid) {
      waiting = {};
      return;
    }
    const p = pollWhileVisible((signal) => loadWaiting(sid, signal), { ms: 15_000 });
    waitingPoller = p;
    return () => {
      p.stop();
      if (waitingPoller === p) waitingPoller = null;
    };
  });
  // Re-read soon after the board's tasks change (a claim, a finished run).
  $effect(() => {
    void tasks;
    const t = setTimeout(() => waitingPoller?.now({ background: true }), 800);
    return () => clearTimeout(t);
  });

  const COLUMN_LABEL: Record<TaskStatus, string> = {
    backlog: 'Backlog',
    todo: 'To do',
    in_progress: 'In progress',
    in_review: 'In review',
    blocked: 'Blocked',
    done: 'Done',
    cancelled: 'Canceled',
    verifying: 'Verifying',
  };

  // One pass per task change into a column map (was three full filters per
  // column per render — backlog B6 / SE-10).
  const columns = $derived.by(() => {
    const m = new Map<TaskStatus, SwarmTask[]>();
    for (const t of tasks) {
      // `verifying` is a transient status with no column of its own — fold
      // those cards into "In review" (with a verifying badge) so they never
      // disappear.
      const col: TaskStatus = t.status === 'verifying' ? 'in_review' : (t.status as TaskStatus);
      const list = m.get(col);
      if (list) list.push(t);
      else m.set(col, [t]);
    }
    return m;
  });
  const NO_TASKS: SwarmTask[] = [];
  function byStatus(s: TaskStatus): SwarmTask[] {
    return columns.get(s) ?? NO_TASKS;
  }
  // Finished columns only grow: render the first DONE_CAP cards until asked.
  const DONE_CAP = 50;
  let showAllFinished = $state(false);
  function visibleIn(s: TaskStatus): SwarmTask[] {
    const list = byStatus(s);
    if (showAllFinished || (s !== 'done' && s !== 'cancelled') || list.length <= DONE_CAP) return list;
    return list.slice(0, DONE_CAP);
  }

  // -- Per-task goals --------------------------------------------------------
  let goalsTask = $state<SwarmTask | null>(null);

  // Live goal summary for a card (only when goal data is in the store — from
  // verification events or after the panel was opened once). Avoids eager loads.
  function goalSummary(tid: string): { passed: number; total: number } | null {
    const gs = swarm.goalsByTask[tid];
    if (!gs || gs.length === 0) return null;
    return { passed: gs.filter((g) => g.status === 'passed').length, total: gs.length };
  }

  // --- Bulk selection -------------------------------------------------------
  let selected = $state<Set<string>>(new Set());
  const selectedCount = $derived(selected.size);
  const selectedTasks = $derived(tasks.filter((t) => selected.has(t.id)));
  // Bulk actions belong to the visible project, never to a previous board.
  $effect(() => {
    void pid;
    selected = new Set();
  });

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
  // Every board mutation reports a failure (a rejected promise from a menu
  // action or button used to vanish, leaving the card where it was).
  async function attempt(failed: string, fn: () => Promise<unknown>): Promise<boolean> {
    try {
      await fn();
      return true;
    } catch (e) {
      toastError(failed, e);
      return false;
    }
  }
  /** A bulk action tries every selected task: on a partial failure say how
   *  many didn't go through ("Couldn’t move 2 of 5 tasks") and keep just those
   *  selected so the action can be retried on them. */
  async function bulk(verb: string, fn: () => Promise<BulkTaskResult>): Promise<void> {
    const n = selectedTasks.length;
    let res: BulkTaskResult;
    try {
      res = await fn();
    } catch (e) {
      toastError(`Couldn’t ${verb} the ${n === 1 ? 'task' : 'tasks'}`, e);
      return;
    }
    if (res.failed.length === 0) {
      clearSelection();
      return;
    }
    toasts.error(
      res.failed.length === n ? `Couldn’t ${verb} the ${n === 1 ? 'task' : 'tasks'}` : `Couldn’t ${verb} ${res.failed.length} of ${n} tasks`,
      res.error ?? '',
    );
    selected = new Set(res.failed.map((t) => t.id));
  }
  async function bulkMove(status: TaskStatus) {
    await bulk('move', () => swarm.bulkUpdateTasks(selectedTasks, { status }));
  }
  async function bulkAssign(agentId: string | null) {
    await bulk('reassign', () => swarm.bulkUpdateTasks(selectedTasks, { assignee_agent_id: agentId }));
  }
  async function bulkDelete() {
    const n = selectedTasks.length;
    if (!n) return;
    if (
      await confirmer.ask(`Delete ${plural(n, 'selected task')}? This cannot be undone.`, {
        title: 'Delete tasks',
        confirmLabel: 'Delete',
        danger: true,
      })
    ) {
      await bulk('delete', () => swarm.bulkDeleteTasks(selectedTasks));
    }
  }
  async function clearBoard() {
    if (!pid || !tasks.length) return;
    const board = pid;
    if (
      await confirmer.ask(
        `Delete ALL ${tasks.length} tasks on this board? In-flight agent runs are stopped and the project’s feed is cleared too. This cannot be undone.`,
        { title: 'Clear board', confirmLabel: 'Clear board', danger: true },
      )
    ) {
      if (await attempt("Couldn’t clear the board", () => swarm.clearProject(board))) clearSelection();
    }
  }

  let adding = $state(false);
  let newTitle = $state('');
  let addingTask = $state(false);
  async function addTask() {
    if (!pid || !newTitle.trim() || addingTask) return;
    const project = pid;
    const submitted = newTitle;
    addingTask = true;
    try {
      if (await attempt("Couldn’t add the task", () => swarm.createTask(project, { title: submitted.trim(), priority: 'medium' }))) {
        if (pid === project && newTitle === submitted) {
          newTitle = '';
          adding = false;
        }
      }
    } finally {
      addingTask = false;
    }
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
      toastError('Couldn’t save the goal', e);
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
        toastError('Couldn’t plan tasks from the goal', e);
      }
    } finally {
      planning = false;
      planCtl = null;
    }
  }

  // Stop ends live agent sessions, so it asks first (patterns: a Stop that
  // ends agent work is red, ends with "…", and confirms).
  async function stopPlan() {
    const ok = await confirmer.ask('Stop planning? The planner agents are stopped; tasks they already created stay on the board.', {
      title: 'Stop planning',
      confirmLabel: 'Stop planning',
      cancelLabel: 'Keep planning',
      danger: true,
    });
    if (!ok || !planning) return;
    // Kill the live planner session(s) server-side, then abandon the UI wait.
    if (swarm.detail) void swarm.stopAgentRun(swarm.detail.id);
    planCtl?.abort();
  }

  function cardMenu(e: MouseEvent, t: SwarmTask) {
    const moves = TASK_COLUMNS.filter((s) => s !== t.status).map((s) => ({
      label: `Move to ${COLUMN_LABEL[s]}`,
      action: () => void attempt("Couldn’t move the task", () => swarm.updateTask(t, { status: s })),
    }));
    const assigns = agents.map((a) => ({
      label: `Assign to ${a.name}`,
      action: () => void attempt("Couldn’t reassign the task", () => swarm.updateTask(t, { assignee_agent_id: a.id })),
    }));
    ctxMenu.show(e, [
      { label: 'Run now', icon: 'play', action: () => runNow(t) },
      { label: 'Goals…', icon: 'check', action: () => (goalsTask = t) },
      { separator: true },
      ...moves,
      { separator: true },
      ...assigns,
      { separator: true },
      {
        label: 'Delete…',
        icon: 'trash',
        danger: true,
        action: async () => {
          if (
            await confirmer.ask(`Delete task “${t.title}”? This cannot be undone.`, {
              title: 'Delete task',
              confirmLabel: 'Delete',
              danger: true,
            })
          )
            await attempt("Couldn’t delete the task", () => swarm.deleteTask(t));
        },
      },
    ]);
  }

  async function runNow(t: SwarmTask) {
    try {
      await swarm.runTask(t);
      toasts.success('Task queued');
    } catch (e) {
      toastError('Couldn’t queue the task', e);
    }
  }

  // Board-level actions that aren't used every minute (and the destructive
  // Clear board) live in one ⋯ menu, keeping the toolbar to picker · Plan · Add.
  function boardMenu(e: MouseEvent) {
    const items: MenuItem[] = [
      { label: goal ? 'Edit goal…' : 'Set goal…', icon: 'note', action: openGoalEditor },
    ];
    if (oneditproject && selectedProject) {
      const p = selectedProject;
      items.push({ label: 'Project settings…', icon: 'gear', action: () => oneditproject?.(p) });
    }
    if (tasks.length) {
      items.push({ separator: true }, { label: 'Clear board…', icon: 'trash', danger: true, action: clearBoard });
    }
    ctxMenu.show(e, items);
  }

  // Keyboard: Enter/Space on a focused card opens its menu (the same actions a
  // right-click or the ⋯ button gives); x toggles its selection.
  /** The title button: Enter / Space open the actions (its click does);
   *  x toggles the card's selection. */
  function onCardKey(e: KeyboardEvent, t: SwarmTask) {
    if (e.key === 'x' && !e.metaKey && !e.ctrlKey && !e.altKey) {
      e.preventDefault();
      toggleSelect(t.id);
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
      toastError('Couldn’t move the task', err);
    }
  }
</script>

<div class="kanban">
  <div class="kb-toolbar">
    {#if projects.length > 1}
      <select class="input small kb-project" aria-label="Project" title="Project shown on the board" bind:value={swarm.selectedProjectId}>
        {#each projects as p (p.id)}
          <option value={p.id}>{p.name}</option>
        {/each}
      </select>
    {:else if projects[0]}
      <span class="kb-project-name" title={projects[0].name}>{projects[0].name}</span>
    {/if}
    {#if onnewproject && projects.length > 0}
      <button class="icon-btn" onclick={onnewproject} aria-label="New project" title="New project"><Icon name="plus" size={14} /></button>
    {/if}
    <span class="grow"></span>
    {#if pid}
      {#if planning}
        <span class="planning" role="status"><span class="spinner" style="--spinner-size: 10px" aria-hidden="true"></span> Planning… <span class="dim">watch live in Runs</span></span>
        <button class="btn small danger" onclick={() => void stopPlan()} title="Stop the planner agents">
          <Icon name="stop" size={12} /> Stop…
        </button>
      {:else}
        <button class="btn small" onclick={planFromGoal} title="Break the project goal into tasks with several planner agents and a summarizer">
          <Icon name="zap" size={12} /> Plan from goal
        </button>
      {/if}
      <button class="icon-btn" onclick={boardMenu} aria-label="Board actions" title="Board actions"><Icon name="more" size={14} /></button>
      <!-- Secondary: the page header's lifecycle button is the view's one primary. -->
      <button class="btn small" onclick={() => (adding = !adding)} aria-expanded={adding}>
        <Icon name="plus" size={12} /> Add task
      </button>
    {/if}
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
      <input dir="auto"
        class="input grow"
        aria-label="New task title"
        placeholder="Task title…"
        use:focusOnMount
        bind:value={newTitle}
        onkeydown={(e) => {
          if (e.key === 'Enter') addTask();
          else if (e.key === 'Escape') adding = false;
        }}
      />
      <button class="btn small primary" onclick={addTask} disabled={addingTask || !newTitle.trim()} title={newTitle.trim() ? undefined : 'Type a task title first'}>Add task</button>
      <button class="btn small ghost" onclick={() => (adding = false)}>Cancel</button>
    </div>
  {/if}

  {#if !pid}
    <EmptyState
      icon="note"
      title="No projects yet"
      body="A project holds the board’s tasks and its goal. Create one, then add tasks or plan them from the goal."
      actionLabel={onnewproject ? 'New project' : undefined}
      actionIcon="plus"
      onaction={onnewproject}
    />
  {:else}
    <!-- Story back-link: shown when this project was seeded from a Product story. -->
    {#if selectedProject?.story_id}
      <StoryLinkCard project={selectedProject} />
    {/if}
    <div class="columns">
      {#each TASK_COLUMNS as col (col)}
        {@const colTasks = byStatus(col)}
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
            <span class="count" aria-label={plural(colTasks.length, 'task')}>{colTasks.length}</span>
          </div>
          <div class="col-body" role="list">
            {#each visibleIn(col) as t (t.id)}
              {@const agent = swarm.agentById(t.assignee_agent_id)}
              {@const gs = goalSummary(t.id)}
              <!-- The card is a plain (draggable) box; its controls are siblings:
                   the select checkbox, the title button (Enter / click → the
                   task's actions; x toggles selection) and the meta buttons.
                   Never a control nested inside a role="button" card. -->
              <div use:rowMenu
                class="card kb-card"
                role="listitem"
                class:dragging={draggingId === t.id}
                class:selected={selected.has(t.id)}
                draggable="true"
                ondragstart={(e) => onDragStart(e, t)}
                ondragend={onDragEnd}
                oncontextmenu={(e) => cardMenu(e, t)}
              >
                <div class="card-title">
                  <input
                    type="checkbox"
                    class="card-sel"
                    checked={selected.has(t.id)}
                    onchange={() => toggleSelect(t.id)}
                    aria-label="Select “{t.title}”"
                  />
                  <button
                    type="button"
                    class="card-main"
                    aria-haspopup="menu"
                    aria-label="{t.title} — {agent ? agent.name : 'unassigned'}, {t.priority} priority. Actions…"
                    onclick={(e) => cardMenu(e, t)}
                    onkeydown={(e) => onCardKey(e, t)}
                  >{t.title}</button>
                </div>
                <div class="card-meta">
                  {#if agent}
                    <span class="assignee" title={agent.title}>{agent.avatar || agent.name.slice(0, 1)} {agent.name}</span>
                  {:else}
                    <span class="assignee dim">Unassigned</span>
                  {/if}
                  <span class="grow"></span>
                  {#if t.status === 'verifying'}
                    <span class="vchip" title="The Coordinator is checking this task’s goals">Verifying</span>
                  {/if}
                  {#if gs}
                    <button
                      class="gchip"
                      class:all-passed={gs.passed === gs.total}
                      onclick={(e) => { e.stopPropagation(); goalsTask = t; }}
                      title="{gs.passed} of {gs.total} goals passed — view goals"
                      aria-label="Goals: {gs.passed} of {gs.total} passed"
                    >
                      <Icon name="check" size={12} /> {gs.passed}/{gs.total}
                    </button>
                  {:else}
                    <button class="icon-btn small" onclick={(e) => { e.stopPropagation(); goalsTask = t; }} aria-label="Goals" title="Goals">
                      <Icon name="check" size={13} />
                    </button>
                  {/if}
                  <span class="chip {PRIORITY_CLASS[t.priority]}" title="Priority">{sentenceCase(t.priority)}</span>
                  <button class="icon-btn small" onclick={(e) => cardMenu(e, t)} aria-label="Task actions" title="Task actions">
                    <Icon name="more" size={14} />
                  </button>
                </div>
                {#if t.delegated}<span class="deleg"><Badge label="Delegated" title="Handed to this agent by another agent" /></span>{/if}
                {#if t.status === 'todo' && waiting[t.id]}
                  {@const w = waiting[t.id]}
                  <span class="waiting" title="Ready, but not started: {w.detail} (since {rel(w.since)})">
                    <Icon name="clock" size={11} /> Waiting: {w.detail}
                  </span>
                {/if}
              </div>
            {/each}
            {#if !showAllFinished && (col === 'done' || col === 'cancelled') && colTasks.length > DONE_CAP}
              <button class="btn small ghost" onclick={() => (showAllFinished = true)}>
                Show all {colTasks.length}
              </button>
            {/if}
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
      <textarea dir="auto"
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

{#if goalsTask}
  <GoalsPanel task={goalsTask} onclose={() => (goalsTask = null)} />
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
    padding: 6px 12px;
    border-block-end: 1px solid var(--border);
    min-height: 40px;
  }
  .kb-project {
    max-width: 240px;
  }
  .kb-project-name {
    font-size: var(--fs-m);
    font-weight: 600;
    min-width: 0;
    max-width: 280px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .goal-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 12px;
    font-size: var(--fs-s);
    color: var(--text-dim);
    background: var(--surface-2);
    border-block-end: 1px solid var(--border);
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
    color: var(--accent-text);
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
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  
  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .field label {
    font-size: var(--fs-s);
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
    font-size: var(--fs-s);
    font-weight: 600;
    color: var(--text-dim);
    border-block-end: 1px solid var(--border);
  }
  .count {
    background: color-mix(in srgb, var(--text-dim) 18%, transparent);
    border-radius: 999px;
    padding: 0 6px;
    font-variant-numeric: tabular-nums;
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
  .kb-card {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 8px;
    cursor: grab;
  }
  .kb-card:hover {
    border-color: var(--border-strong);
  }
  /* The title button's focus rings the whole card. */
  .kb-card:has(.card-main:focus-visible) {
    outline: 2px solid var(--accent-text);
    outline-offset: 1px;
  }
  .card-main {
    all: unset;
    cursor: pointer;
    overflow-wrap: anywhere;
  }
  .card-main:hover {
    text-decoration: underline;
  }
  .card-main:focus-visible {
    outline: none;
  }
  .kb-card.dragging {
    opacity: 0.4;
    cursor: grabbing;
  }
  .kb-card.selected {
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
    padding: 6px 12px;
    border-block-end: 1px solid var(--border);
    background: var(--accent-soft);
    font-size: var(--fs-s);
  }
  .bulk-count {
    font-weight: 600;
    color: var(--accent-text);
  }
  .column.drop-target {
    background: color-mix(in srgb, var(--accent) 8%, var(--surface-2));
    border-color: color-mix(in srgb, var(--accent) 50%, var(--border));
  }
  .card-title {
    font-size: var(--fs-m);
    margin-bottom: 6px;
  }
  .waiting {
    display: flex;
    align-items: center;
    gap: 4px;
    margin-block-start: 6px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    overflow-wrap: anywhere;
  }
  .card-meta {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-xs);
  }
  .assignee {
    color: var(--text-dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 120px;
  }
  .deleg {
    display: inline-flex;
    margin-block-start: 6px;
  }
  .vchip {
    font-size: var(--fs-xs);
    color: var(--accent-text);
    border: 1px solid var(--accent-line);
    border-radius: 999px;
    padding: 0 6px;
    animation: otto-pulse 1.4s ease-in-out infinite;
  }
  
  @media (prefers-reduced-motion: reduce) {
    .vchip {
      animation: none;
    }
  }
  .gchip {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    background: transparent;
    border: 1px solid var(--border);
    border-radius: 999px;
    padding: 0 6px;
    cursor: pointer;
  }
  .gchip:hover {
    border-color: color-mix(in srgb, var(--accent) 50%, var(--border));
    color: var(--text);
  }
  .gchip.all-passed {
    background: var(--success-soft);
    color: var(--success);
    border-color: var(--success);
    font-weight: 600;
  }
</style>
