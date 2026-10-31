///<reference types="svelte" />
;
import { announce } from '../../lib/announce.svelte';
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
import LoadState from '../../lib/components/LoadState.svelte';
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

;type $$ComponentProps =  { onnewproject?: () => void; oneditproject?: (p: SwarmProject) => void };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  // `onnewproject` / `oneditproject` hand off to SwarmPage, which owns the
  // project create/edit modal (where project skills live). New project lives
  // here — beside the project picker — rather than in the page header.
  let {
    onnewproject,
    oneditproject,
  }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();

  const projects = $derived(swarm.detail?.projects ?? []);
  // Fall back to the first project so the board is never wedged when
  // `selectedProjectId` is null but projects exist (a `<select bind:value>`
  // shows option 0 visually without writing it back to the model).
  const pid = $derived(swarm.selectedProjectId ?? projects[0]?.id ?? null);
  const tasks = $derived(swarm.tasks(pid));
  const taskError = $derived(pid ? (swarm.tasksError[pid] ?? null) : null);
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
    if (swarm.detail) void swarm.stopAgentRun(swarm.detail.id, 'plan');
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

  // a11y-6-01: announce at the state edge — a status element inserted
  // together with its text is not read by VoiceOver/WebKit.
  $effect(() => { if (planning) announce('Planning…'); });
;
async () => {

 { svelteHTML.createElement("div", { "class":`kanban`,});
   { svelteHTML.createElement("div", { "class":`kb-toolbar`,});
    if(projects.length > 1){
       { svelteHTML.createElement("select", {       "class":`input small kb-project`,"aria-label":`Project`,"title":`Project shown on the board`,"bind:value":swarm.selectedProjectId,});/*Ωignore_startΩ*/() => swarm.selectedProjectId = __sveltets_2_any(null);/*Ωignore_endΩ*/
           for(let p of __sveltets_2_ensureArray(projects)){p.id;
           { svelteHTML.createElement("option", { "value":p.id,});p.name; }
        }
       }
    } else if (projects[0]){
       { svelteHTML.createElement("span", {   "class":`kb-project-name`,"title":projects[0].name,});projects[0].name; }
    }
    if(onnewproject && projects.length > 0){
       { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":onnewproject,"aria-label":`New project`,"title":`New project`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":14,}});} }
    }
     { svelteHTML.createElement("span", { "class":`grow`,}); }
    if(pid){
      if(planning){
         { svelteHTML.createElement("span", { "class":`planning`,}); { svelteHTML.createElement("span", {     "class":`spinner`,"style":`--spinner-size: 10px`,"aria-hidden":`true`,}); }   { svelteHTML.createElement("span", { "class":`dim`,});    } }
         { svelteHTML.createElement("button", {     "class":`btn small danger`,...__sveltets_2_empty({"data-keep":true}),"onclick":() => void stopPlan(),"title":`Stop the planner agents`,});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`stop`,"size":12,}});} 
         }
      }else{
         { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":planFromGoal,"title":`Break the project goal into tasks with several planner agents and a summarizer`,});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`zap`,"size":12,}});}   
         }
      }
       { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":boardMenu,"aria-label":`Board actions`,"title":`Board actions`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":14,}});} }
      
       { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":() => (adding = !adding),"aria-expanded":adding,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}  
       }
    }
   }

  if(selectedCount > 0){
     { svelteHTML.createElement("div", { "class":`bulk-bar`,});
       { svelteHTML.createElement("span", { "class":`bulk-count`,});selectedCount;  }
       { svelteHTML.createElement("button", {   "class":`btn small`,...__sveltets_2_empty({"data-keep":true}),"onclick":bulkMoveMenu,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`split`,"size":13,}});}   }
       { svelteHTML.createElement("button", {   "class":`btn small`,...__sveltets_2_empty({"data-keep":true}),"onclick":bulkAssignMenu,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`user`,"size":13,}});}  }
       { svelteHTML.createElement("button", {   "class":`btn small danger`,...__sveltets_2_empty({"data-keep":true}),"onclick":bulkDelete,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":13,}});}  }
       { svelteHTML.createElement("span", { "class":`grow`,}); }
       { svelteHTML.createElement("button", {   "class":`btn small ghost`,...__sveltets_2_empty({"data-keep":true}),"onclick":clearSelection,});  }
     }
  }

  if(pid){
     { svelteHTML.createElement("div", {  "class":`goal-bar`,});!goal;
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`zap`,"size":12,}});}
      if(goal){
         { svelteHTML.createElement("span", {   "class":`goal-text`,"title":goal,});goal; }
         { svelteHTML.createElement("button", {   "class":`link`,"onclick":openGoalEditor,});  }
      }else{
         { svelteHTML.createElement("span", { "class":`goal-text dim`,});     { svelteHTML.createElement("button", {   "class":`link`,"onclick":openGoalEditor,});   }      }
      }
     }
  }

  if(adding){
     { svelteHTML.createElement("div", { "class":`add-row`,});
       {const $$action_0 = __sveltets_2_ensureAction(focusOnMount(svelteHTML.mapElementTag('input')));{ svelteHTML.createElement("input", __sveltets_2_union($$action_0), {             "dir":`auto`,"class":`input grow`,"aria-label":`New task title`,"placeholder":`Task title…`,"bind:value":newTitle,"onkeydown":(e) => {
          if (e.key === 'Enter') addTask();
          else if (e.key === 'Escape') adding = false;
        },});/*Ωignore_startΩ*/() => newTitle = __sveltets_2_any(null);/*Ωignore_endΩ*/}}
       { svelteHTML.createElement("button", {       "class":`btn small primary`,"onclick":addTask,"disabled":addingTask || !newTitle.trim(),"title":newTitle.trim() ? undefined : 'Type a task title first',});  }
       { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => (adding = false),});  }
     }
  }

  if(!pid){
     { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {             "icon":`note`,"title":`No projects yet`,"body":`A project holds the board’s tasks and its goal. Create one, then add tasks or plan them from the goal.`,"actionLabel":onnewproject ? 'New project' : undefined,"actionIcon":`plus`,"onaction":onnewproject,}});}
  }else{
    
    if(selectedProject?.story_id){
       { const $$_draCkniLyrotS1C = __sveltets_2_ensureComponent(StoryLinkCard); new $$_draCkniLyrotS1C({ target: __sveltets_2_any(), props: {  "project":selectedProject,}});}
    }
     { svelteHTML.createElement("div", { "class":`board-body`,});
     { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {           children:() => { return __sveltets_2_any(0); },"what":`tasks`,"loading":swarm.tasksLoading,"error":taskError,"empty":tasks.length === 0 && (!!taskError || swarm.tasksLoading),"onretry":() => pid && void swarm.loadTasks(pid),}});
     { svelteHTML.createElement("div", { "class":`columns`,});
         for(let col of __sveltets_2_ensureArray(TASK_COLUMNS)){col;
        const colTasks = byStatus(col);
         { svelteHTML.createElement("div", {            "class":`column`,"role":`group`,"ondragover":(e) => onDragOver(e, col),"ondragleave":() => onDragLeave(col),"ondrop":(e) => onDrop(e, col),});dropCol === col;
           { svelteHTML.createElement("div", { "class":`col-head`,});
             { svelteHTML.createElement("span", {});COLUMN_LABEL[col]; }
             { svelteHTML.createElement("span", {   "class":`count`,"aria-label":plural(colTasks.length, 'task'),});colTasks.length; }
           }
           { svelteHTML.createElement("div", {   "class":`col-body`,"role":`list`,});
               for(let t of __sveltets_2_ensureArray(visibleIn(col))){t.id;
              const agent = swarm.agentById(t.assignee_agent_id);
              const gs = goalSummary(t.id);
              
               {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('div')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {                "class":`card kb-card`,"role":`listitem`,"draggable":`true`,"ondragstart":(e) => onDragStart(e, t),"ondragend":onDragEnd,"oncontextmenu":(e) => cardMenu(e, t),});draggingId === t.id;selected.has(t.id);
                 { svelteHTML.createElement("div", { "class":`card-title`,});
                   { svelteHTML.createElement("input", {           "type":`checkbox`,"class":`card-sel`,"checked":selected.has(t.id),"onchange":() => toggleSelect(t.id),"aria-label":`Select “${t.title}”`,});}
                   { svelteHTML.createElement("button", {             "type":`button`,"class":`card-main`,"aria-haspopup":`menu`,"aria-label":`${t.title} — ${agent ? agent.name : 'unassigned'}, ${t.priority} priority. Actions…`,"onclick":(e) => cardMenu(e, t),"onkeydown":(e) => onCardKey(e, t),});t.title; }
                 }
                 { svelteHTML.createElement("div", { "class":`card-meta`,});
                  if(agent){
                     { svelteHTML.createElement("span", {   "class":`assignee`,"title":agent.title,});agent.avatar || agent.name.slice(0, 1); agent.name; }
                  }else{
                     { svelteHTML.createElement("span", { "class":`assignee dim`,});  }
                  }
                   { svelteHTML.createElement("span", { "class":`grow`,}); }
                  if(t.status === 'verifying'){
                     { svelteHTML.createElement("span", {   "class":`vchip`,"title":`The Coordinator is checking this task’s goals`,});  }
                  }
                  if(gs){
                     { svelteHTML.createElement("button", {          "class":`gchip`,"onclick":(e) => { e.stopPropagation(); goalsTask = t; },"title":`${gs.passed} of ${gs.total} goals passed — view goals`,"aria-label":`Goals: ${gs.passed} of ${gs.total} passed`,});gs.passed === gs.total;
                       { const $$_nocI9C = __sveltets_2_ensureComponent(Icon); new $$_nocI9C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});} gs.passed; gs.total;
                     }
                  }else{
                     { svelteHTML.createElement("button", {       "class":`icon-btn small`,"onclick":(e) => { e.stopPropagation(); goalsTask = t; },"aria-label":`Goals for “${t.title}”`,"title":`Goals for “${t.title}”`,});
                       { const $$_nocI9C = __sveltets_2_ensureComponent(Icon); new $$_nocI9C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":13,}});}
                     }
                  }
                   { svelteHTML.createElement("span", {   "class":`chip ${PRIORITY_CLASS[t.priority]}`,"title":`Priority`,});sentenceCase(t.priority); }
                   { svelteHTML.createElement("button", {       "class":`icon-btn small`,"onclick":(e) => cardMenu(e, t),"aria-label":`Task actions`,"title":`Task actions`,});
                     { const $$_nocI9C = __sveltets_2_ensureComponent(Icon); new $$_nocI9C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":14,}});}
                   }
                 }
                if(t.delegated){ { svelteHTML.createElement("span", { "class":`deleg`,}); { const $$_egdaB8C = __sveltets_2_ensureComponent(Badge); new $$_egdaB8C({ target: __sveltets_2_any(), props: {    "label":`Delegated`,"title":`Handed to this agent by another agent`,}});} }}
                if(t.status === 'todo' && waiting[t.id]){
                  const w = waiting[t.id];
                   { svelteHTML.createElement("span", {   "class":`waiting`,"title":`Ready, but not started: ${w.detail} (since ${rel(w.since)})`,});
                     { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":`clock`,"size":12,}});}  w.detail;
                   }
                }
               }}
            }
            if(!showAllFinished && (col === 'done' || col === 'cancelled') && colTasks.length > DONE_CAP){
               { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => (showAllFinished = true),});
                  colTasks.length;
               }
            }
           }
         }
      }
     }
     LoadState}
     }
  }
 }

if(editingGoal){
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":goal ? 'Edit project goal' : 'Set project goal',"width":560,"onclose":() => (editingGoal = false),footer:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("button", {   "class":`btn ghost`,"onclick":() => (editingGoal = false),});  }
       { svelteHTML.createElement("button", {     "class":`btn primary`,"onclick":saveGoal,"disabled":savingGoal,});savingGoal ? 'Saving…' : 'Save'; }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`goal-md`,});  }
       { svelteHTML.createElement("textarea", {            "dir":`auto`,"id":`goal-md`,"class":`input`,"rows":8,"bind:value":goalDraft,"placeholder":`Describe what this project should achieve. Plan from goal turns this into tasks.`,});/*Ωignore_startΩ*/() => goalDraft = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
     }
    
   Modal}
}

if(goalsTask){
   { const $$_lenaPslaoG0C = __sveltets_2_ensureComponent(GoalsPanel); new $$_lenaPslaoG0C({ target: __sveltets_2_any(), props: {    "task":goalsTask,"onclose":() => (goalsTask = null),}});}
}


};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const KanbanBoard__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type KanbanBoard__SvelteComponent_ = ReturnType<typeof KanbanBoard__SvelteComponent_>;
/*Ωignore_endΩ*/export default KanbanBoard__SvelteComponent_;