///<reference types="svelte" />
;
import { plural } from '../../lib/plural';
import { toastError } from '../../lib/toastError';
import { tick, untrack } from 'svelte';
import { onTabKey } from '../../lib/tabKeys';
import { paneResizer, RESIZE_TITLE, LIST_PANE, loadPaneWidth } from '../../lib/paneResizer';
import PaneDivider from '../../lib/components/PaneDivider.svelte';
import PageBody from '../../lib/components/PageBody.svelte';
import PathField from '../../lib/components/PathField.svelte';
// Agent Swarm section: swarm list + the open swarm (org tree, run graph,
// kanban, runs, board) with an inline session panel (reuses SessionView).
import Icon, { type IconName } from '../../lib/components/Icon.svelte';
import Modal from '../../lib/components/Modal.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import AutomateGuide from '../../lib/components/AutomateGuide.svelte';
import AutomateGuideButton from '../../lib/components/AutomateGuideButton.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import StatusBadge from '../../lib/components/StatusBadge.svelte';
import { sentenceCase, type Tone } from '../../lib/status';
import PageHeader from '../../lib/components/PageHeader.svelte';
import { initialSelection, rememberSelection } from '../../lib/lastSelection';
import SessionView from '../agents/SessionView.svelte';
import OrgTree from './OrgTree.svelte';
import AgentGraph from './AgentGraph.svelte';
import KanbanBoard from './KanbanBoard.svelte';
import RunsList from './RunsList.svelte';
import BoardFeed from './BoardFeed.svelte';
import NewSwarm from './NewSwarm.svelte';
import RecruiterWizard from './RecruiterWizard.svelte';
import AgentEditor from './AgentEditor.svelte';
import SwarmSettings from './SwarmSettings.svelte';
import SkillPicker from './SkillPicker.svelte';
import { swarm } from '../../lib/stores/swarm.svelte';
import { swarmPagePort } from '../../lib/uiCommands/swarm';
import { router } from '../../lib/router.svelte';
import { registry } from '../../lib/commands.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { startMouseDrag } from '../../lib/dragCursor';
import { viewport } from '../../lib/stores/viewport.svelte';
import { toasts } from '../../lib/toast.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { ctxMenu } from '../../lib/contextmenu.svelte';
import { loadErrorText } from '../../lib/loadError';
import type { CreateAgentReq, RecruitedAgent, Swarm, SwarmAgent, SwarmProject } from './types';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  type View = 'tree' | 'graph' | 'kanban' | 'runs' | 'board';
  let view = $state<View>('tree');
  /** Swarm lifecycle → the shared badge tone (patterns.md §1). */
  const SWARM_TONE: Record<string, Tone> = { active: 'success', paused: 'neutral', aborted: 'danger' };

  // On a phone the picker collapses once a swarm is open. Tablet and desktop
  // keep the list visible. The view stays mounted when the picker toggles.
  let railOpen = $state(true);
  let railEl = $state<HTMLElement | null>(null);
  let railToggle = $state<HTMLButtonElement | null>(null);
  let lastOpenedId: string | null = null;
  let wasPhone = false;
  async function collapseRail(): Promise<void> {
    const restoreFocus = railEl?.querySelector('.rail-list')?.contains(document.activeElement);
    railOpen = false;
    await tick();
    if (restoreFocus) railToggle?.focus();
  }
  $effect.pre(() => {
    const phone = viewport.isPhone;
    const id = swarm.detail?.id ?? null;
    if (phone && id && (id !== lastOpenedId || !wasPhone)) {
      untrack(() => void collapseRail());
    }
    lastOpenedId = id;
    wasPhone = phone;
  });
  async function pickSwarm(id: string): Promise<void> {
    await openSwarm(id);
    if (viewport.isPhone && swarm.detail?.id === id) await collapseRail();
  }

  let showNew = $state(false);
  let showRecruit = $state(false);
  // Set when hiring from a completed recruit run → opens the wizard pre-filled.
  let recruitProposal = $state<RecruitedAgent | null>(null);
  let recruitProposalRunId = $state<string | null>(null);

  // --- Resizable session panel (drag the divider) --------------------------
  // width % of the board/view; rest goes to the session. Persisted so the
  // chosen split survives reloads (clamped on read in case the stored value
  // predates the current bounds).
  let viewPct = $state(loadViewPct());
  let bodyEl = $state<HTMLDivElement | null>(null);
  function loadViewPct(): number {
    if (typeof localStorage === 'undefined') return 55;
    const v = Number(localStorage.getItem('swarm.viewPct'));
    return Number.isFinite(v) && v >= 25 ? Math.min(75, v) : 55;
  }
  function persistViewPct(): void {
    try {
      localStorage.setItem('swarm.viewPct', String(Math.round(viewPct)));
    } catch {
      /* storage unavailable — non-fatal */
    }
  }
  function startResize(e: MouseEvent) {
    e.preventDefault();
    const el = bodyEl;
    if (!el) return;
    // Overlay cursor + one write per frame (lib/dragCursor.ts).
    startMouseDrag(e, {
      onMove: (ev) => {
        const rect = el.getBoundingClientRect();
        const pct = ((ev.clientX - rect.left) / rect.width) * 100;
        viewPct = Math.min(80, Math.max(20, pct));
      },
      onEnd: () => persistViewPct(),
    });
  }

  // --- Resizable swarms rail (PaneDivider) ------------------------------------
  // Drag to taste, width persists under swarm.railW, double-click resets. On a
  // phone/tablet the rail is a full-width accordion band, so the width binding
  // is skipped and the divider hidden.
  let railW = $state(loadPaneWidth('swarm.railW', LIST_PANE.default, LIST_PANE.min, LIST_PANE.max));
  let editAgent = $state<SwarmAgent | null>(null);
  let editorOpen = $state(false);
  let showSettings = $state(false);
  let projModal = $state(false);
  // null = creating; an id = editing that project.
  let projEditId = $state<string | null>(null);
  let projName = $state('');
  let projRepo = $state('');
  let projGoal = $state('');
  let projSkills = $state<string[]>([]);
  let projSaving = $state(false);

  // Load swarms for the current workspace. A first load shows a skeleton
  // (store `loadingSwarms`), never a flash of the "No swarms in …" empty state.
  const listLoading = $derived(swarm.loadingSwarms);
  function loadList(id: string): Promise<void> {
    return swarm.loadSwarms(id);
  }
  $effect(() => {
    const id = ws.currentId;
    if (id) void loadList(id);
  });

  // Opening a swarm can fail (deleted elsewhere, daemon hiccup). The store
  // rethrows, so catch it here and show an inline error with Retry instead of
  // an unhandled rejection that leaves the pane blank.
  let openError = $state<string | null>(null);
  let openErrorId = $state<string | null>(null);
  let openingId: string | null = null;
  async function openSwarm(id: string): Promise<void> {
    openError = null;
    openErrorId = null;
    openingId = id;
    try {
      await swarm.openSwarm(id);
    } catch (e) {
      openError = swarm.detailError ?? loadErrorText(e);
      openErrorId = id;
    } finally {
      if (openingId === id) openingId = null;
    }
  }

  // Live swarm events apply only while this page is mounted; a remount after
  // missed events resyncs the open swarm (backlog B6 / SE-04).
  $effect(() => swarm.watch());

  // Agent UI control (lib/uiCommands/swarm.ts) opens a swarm / switches the view.
  $effect(() => swarmPagePort.bind({ openSwarm, setView: (v) => (view = v) }));

  // A deep-link (e.g. Product → Swarm) opened a project and asked for the
  // Kanban board — honor it once, then clear the flag.
  $effect(() => {
    if (swarm.pendingKanban) {
      view = 'kanban';
      swarm.pendingKanban = false;
    }
  });

  const detail = $derived(swarm.detail);

  // Never open onto a big empty "Pick a swarm" pane when the workspace has
  // swarms: restore the last-opened one (or the first) once per workspace load.
  // Skipped on a phone, where opening a swarm collapses the rail — the list is
  // the first screen there.
  let autoPickedFor = $state<string | null>(null);
  $effect(() => {
    const wsId = ws.currentId;
    if (!wsId || autoPickedFor === wsId || viewport.isPhone) return;
    // A deep link (`#/swarm/<id>`) opens on its own (below).
    if (swarm.detail || swarm.loading || untrack(() => routeSwarmId())) {
      autoPickedFor = wsId;
      return;
    }
    if (swarm.swarms.length === 0) return;
    autoPickedFor = wsId;
    const id = initialSelection('swarm', swarm.swarms, (s) => s.id);
    if (id) void openSwarm(id);
  });

  // ⌘K: the page's verbs.
  $effect(() =>
    registry.register('swarm', [
      { id: 'swarm.new', title: 'New swarm…', group: 'Swarm', keywords: 'create agents team org', run: () => (showNew = true) },
    ]),
  );

  /** The swarm the URL names (`#/swarm/<id>`), or null. */
  function routeSwarmId(): string | null {
    const [mod, id] = router.parts;
    return mod === 'swarm' && id ? id : null;
  }
  // The URL carries the open swarm so a reload / share / notification lands on
  // it; the last pick is remembered as the fallback. Only a CLOSE clears the
  // URL, and on arrival a link naming a DIFFERENT swarm than the one the store
  // kept wins (the effect below opens it).
  let routedId: string | null = null;
  let arrived = false;
  $effect(() => {
    const id = detail?.id ?? null;
    if (id) rememberSelection('swarm', id);
    const was = routedId;
    routedId = id;
    const first = !arrived;
    arrived = true;
    if (!id && !was) return;
    const linked = untrack(() => routeSwarmId());
    if (first && linked && linked !== id) return;
    untrack(() => {
      if (router.module === 'swarm' && linked !== id) router.replace(id ? `swarm/${id}` : 'swarm');
    });
  });
  // A route change while the page is up (a notification, Back/Forward, a
  // pasted link) opens that swarm.
  $effect(() => {
    const linked = routeSwarmId();
    if (!linked) return;
    untrack(() => {
      if (linked !== swarm.detail?.id && linked !== openingId) void openSwarm(linked);
    });
  });
  // The rail is pointless while there is nothing to list (and no load error to
  // retry): the page-level empty state owns the page then.
  // (A failed first load is shown by the main pane's inline error + Retry.)
  const showRail = $derived(swarm.swarms.length > 0);
  const queued = $derived(swarm.runStats.queued);
  const running = $derived(swarm.runStats.running);
  const cap = $derived(detail?.config.max_parallel_sessions ?? 4);

  const VIEWS: { id: View; label: string; icon: IconName }[] = [
    { id: 'tree', label: 'Org', icon: 'user' },
    { id: 'graph', label: 'Graph', icon: 'split' },
    { id: 'kanban', label: 'Board', icon: 'note' },
    { id: 'runs', label: 'Runs', icon: 'clock' },
    { id: 'board', label: 'Feed', icon: 'comment' },
  ];

  const LIFECYCLE_FAILED: Record<'start' | 'pause' | 'abort' | 'resume', string> = {
    start: "Couldn’t start the swarm",
    pause: "Couldn’t pause the swarm",
    abort: "Couldn’t abort the swarm",
    resume: "Couldn’t resume the swarm",
  };

  async function lifecycle(action: 'start' | 'pause' | 'abort' | 'resume') {
    if (!detail) return;
    // Abort kills every swarm session and cancels queued/running runs — the
    // in-flight work is lost, so it names the blast radius and asks first.
    if (action === 'abort') {
      const agents = running === 1 ? '1 running agent is' : `${running} running agents are`;
      const q = queued === 1 ? '1 queued run is' : `${queued} queued runs are`;
      const ok = await confirmer.ask(
        `Abort “${detail.name}”? ${agents} stopped and their sessions closed, and ${q} canceled. Work in progress is lost.`,
        { title: 'Abort swarm', confirmLabel: 'Abort all', danger: true },
      );
      if (!ok) return;
    }
    try {
      await swarm.lifecycle(action, detail.id);
    } catch (e) {
      toastError(LIFECYCLE_FAILED[action], e);
    }
  }

  async function setCap(input: HTMLInputElement) {
    if (!detail) return;
    const v = Math.floor(Number(input.value));
    // An invalid entry (blank, 0, negative) used to be dropped silently while
    // the field kept showing it — snap back to the saved cap and say why.
    if (!Number.isFinite(v) || v < 1) {
      input.value = String(cap);
      toasts.warn('Parallel sessions must be 1 or more');
      return;
    }
    try {
      await swarm.setParallelCap(detail.id, v);
    } catch (e) {
      input.value = String(cap);
      toastError("Couldn’t change parallel sessions", e);
    }
  }

  // Edit the total-run budget cap (blank/0 = unlimited). The 300 default is a
  // runaway-cost backstop, not a usage limit.
  async function setRunsCap() {
    if (!detail) return;
    const cur = detail.max_total_runs == null ? '' : String(detail.max_total_runs);
    const v = await confirmer.promptText('Max total runs for this swarm (blank = unlimited):', {
      title: 'Run budget',
      confirmLabel: 'Save',
      initial: cur,
      placeholder: 'e.g. 5000 — blank for unlimited',
    });
    if (v === null) return;
    const t = v.trim();
    const next = t === '' || Number(t) <= 0 ? null : Math.floor(Number(t));
    if (next !== null && !Number.isFinite(next)) {
      toasts.warn('Run budget not changed', `“${t}” isn’t a number — enter a whole number, or leave it blank for unlimited.`);
      return;
    }
    try {
      await swarm.updateSwarm(detail.id, { max_total_runs: next } as Partial<Swarm>);
    } catch (e) {
      toastError("Couldn’t change the run budget", e);
    }
  }

  function openProjectCreate() {
    projEditId = null;
    projName = '';
    projRepo = '';
    projGoal = '';
    projSkills = [];
    projModal = true;
  }

  function openProjectEdit(p: SwarmProject) {
    projEditId = p.id;
    projName = p.name;
    projRepo = p.repo_path ?? '';
    projGoal = p.goal_md ?? '';
    projSkills = ((p.skills ?? []) as unknown[]).map((s) => String(s));
    projModal = true;
  }

  async function saveProject() {
    if (!detail || !projName.trim() || projSaving) return;
    projSaving = true;
    try {
      if (projEditId) {
        await swarm.updateProject(projEditId, {
          name: projName.trim(),
          repo_path: projRepo.trim() || null,
          goal_md: projGoal.trim() || null,
          skills: projSkills,
        } as Partial<SwarmProject>);
        toasts.success('Project updated');
      } else {
        await swarm.createProject(detail.id, {
          name: projName.trim(),
          repo_path: projRepo.trim() || undefined,
          goal_md: projGoal.trim() || undefined,
        });
        // Project skills ride PATCH /swarm/projects/{pid}; apply them once the
        // new project exists (createProject pins selectedProjectId to it).
        const pid = swarm.selectedProjectId;
        if (pid && projSkills.length) {
          await swarm.updateProject(pid, { skills: projSkills } as Partial<SwarmProject>);
        }
        view = 'kanban';
        toasts.success('Project created', `“${projName.trim()}” is open on the Board.`);
      }
      projModal = false;
    } catch (e) {
      toastError("Couldn’t save the project", e);
    } finally {
      projSaving = false;
    }
  }

  function projectMenu(e: MouseEvent) {
    ctxMenu.show(e, [
      {
        label: 'Delete project…',
        icon: 'trash',
        danger: true,
        title: 'Delete this project, its tasks and feed',
        action: () => void deleteProject(),
      },
    ]);
  }

  async function deleteProject() {
    if (!projEditId) return;
    const p = detail?.projects.find((x) => x.id === projEditId);
    if (
      await confirmer.ask(
        `Delete project “${p?.name ?? projName}”? All its tasks and feed are removed and in-flight runs are stopped. This cannot be undone.`,
        { title: 'Delete project', confirmLabel: 'Delete', danger: true },
      )
    ) {
      try {
        await swarm.deleteProject(projEditId);
        projModal = false;
        toasts.success('Project deleted');
      } catch (e) {
        toastError("Couldn’t delete the project", e);
      }
    }
  }

  async function deleteSwarm() {
    if (!detail) return;
    const d = detail;
    if (
      await confirmer.ask(
        `Delete swarm “${d.name}” and all its agents, projects and tasks? This cannot be undone.`,
        { title: 'Delete swarm', confirmLabel: 'Delete', danger: true },
      )
    ) {
      try {
        await swarm.deleteSwarm(d.id);
        toasts.success('Swarm deleted', `“${d.name}” and its agents, projects and tasks were removed.`);
        // Don't strand the page on a "pick one" pane: open the next swarm.
        const next = swarm.swarms[0];
        if (next && !viewport.isPhone) void openSwarm(next.id);
      } catch (e) {
        toastError("Couldn’t delete the swarm", e);
      }
    }
  }

  async function runForAgent(a: SwarmAgent) {
    const pid = swarm.selectedProjectId;
    if (!pid) {
      toasts.warn('Create a project first');
      return;
    }
    const title = (await confirmer.promptText('Task for ' + a.name, { title: 'Run a task', confirmLabel: 'Create & run' }))?.trim();
    if (!title) return;
    try {
      const created = await swarm.createTask(pid, { title, assignee_agent_id: a.id });
      await swarm.runTask(created);
      view = 'kanban';
    } catch (e) {
      toastError("Couldn’t run the task", e);
    }
  }

  // `editorPrefill` seeds the editor when creating (not editing) an agent —
  // either a bare reports_to (Add direct report) or a full copy (Duplicate).
  let editorPrefill = $state<Partial<CreateAgentReq> | null>(null);

  function openEditor(a: SwarmAgent | null) {
    editAgent = a;
    editorPrefill = null;
    editorOpen = true;
  }

  // Prefill the editor to add a new agent reporting to `parent` (or top-level
  // when parent is null). OrgTree calls this via its `onadd` prop.
  function openEditorWithParent(parent: SwarmAgent | null) {
    editAgent = null;
    editorPrefill = { reports_to: parent?.id ?? null };
    editorOpen = true;
  }

  // Duplicate an existing agent: open the editor as a NEW agent pre-filled with
  // the source's full config (name suffixed "(copy)"), so the operator can tweak
  // one field — e.g. the model — and hire a near-identical sibling.
  function duplicateAgent(a: SwarmAgent) {
    editAgent = null;
    editorPrefill = {
      name: `${a.name} (copy)`,
      title: a.title,
      provider: a.provider,
      model: a.model ?? null,
      reports_to: a.reports_to ?? null,
      specialization: a.specialization,
      soul_md: a.soul_md ?? null,
      soul_name: a.soul_name ?? null,
      scope_md: a.scope_md,
      avatar: a.avatar,
      skills: a.skills,
      schedule: a.schedule ?? null,
    };
    editorOpen = true;
  }

  let showBudgetModal = $state(false);
  let extraRuns = $state(20);
  let extraCostUsd = $state(5);

  async function raiseBudgetAndResume() {
    if (!detail) return;
    const patch: Partial<Swarm> = {};
    if (detail.max_total_runs != null) patch.max_total_runs = detail.max_total_runs + extraRuns;
    if (detail.max_cost_usd != null) patch.max_cost_usd = detail.max_cost_usd + extraCostUsd;
    try {
      await swarm.updateSwarm(detail.id, patch);
    } catch (e) {
      toastError("Couldn’t raise the budget", e);
      return;
    }
    await lifecycle('resume');
    showBudgetModal = false;
  }

  /** Header ⋯: the swarm's destructive verbs, kept off the toolbar row. */
  function swarmMenu(e: MouseEvent): void {
    if (!detail) return;
    ctxMenu.show(e, [
      ...(detail.status !== 'aborted'
        ? [{ label: 'Abort all…', icon: 'stop', danger: true, title: 'Stop every agent and cancel queued runs', action: () => void lifecycle('abort') }]
        : []),
      { label: 'Delete swarm…', icon: 'trash', danger: true, action: () => void deleteSwarm() },
    ]);
  }

;
async () => {

 { svelteHTML.createElement("div", {   "class":`swarm-page`,});viewport.isPhone;viewport.isMobile;
   { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {       "class":`swarm-head`,"title":detail?.name ?? 'Swarm',"subtitle":detail ? `${plural(detail.counts.agents, 'agent')} · ${plural(detail.counts.projects, 'project')} · ${running} running · ${queued} queued` : undefined,badge:() => { async ()/*Ωignore_positionΩ*/ => {
      if(detail){
         { svelteHTML.createElement("span", {   "class":`swarm-status`,...__sveltets_2_empty({"data-status":detail.status}),});
           { const $$_egdaBsutatS1C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS1C({ target: __sveltets_2_any(), props: {    "tone":SWARM_TONE[detail.status] ?? 'neutral',"label":sentenceCase(detail.status),}});}
         }
        if(detail.pause_reason){
           { svelteHTML.createElement("span", {   "class":`pause-reason`,"title":detail.pause_reason,}); detail.pause_reason; }
        }
      }
    };return __sveltets_2_any(0)},actions:() => { async ()/*Ωignore_positionΩ*/ => {
       { const $$_nottuBediuGetamotuA0C = __sveltets_2_ensureComponent(AutomateGuideButton); new $$_nottuBediuGetamotuA0C({ target: __sveltets_2_any(), props: {  "current":`swarm`,}});}
      if(detail){
        
         { svelteHTML.createElement("button", {           "class":`btn small`,...__sveltets_2_empty({"data-overflow":`-1`}),...__sveltets_2_empty({"data-icon":`gear`}),"onclick":() => (showSettings = true),"title":`Standing goals, team skills & channel triggers`,...__sveltets_2_empty({"data-label":`Settings`}),}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`gear`,"size":12,}});}  }
         { svelteHTML.createElement("button", {       "class":`btn small`,...__sveltets_2_empty({"data-icon":`plus`}),"title":`Let the Recruiter propose an agent for you to review and hire`,"onclick":() => (showRecruit = true),}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}  }
        if(detail.status === 'active'){
           { svelteHTML.createElement("button", {     "class":`btn small`,...__sveltets_2_empty({"data-keep":true}),"title":`Stop picking up new runs; running agents finish their current step`,"onclick":() => lifecycle('pause'),}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`pause`,"size":12,}});}  }
        } else if (detail.status === 'paused'){
          if(detail.pause_reason){
            
             { svelteHTML.createElement("button", {   "class":`btn small primary`,"onclick":() => (showBudgetModal = true),}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`play`,"size":12,}});}     }
          }else{
             { svelteHTML.createElement("button", {   "class":`btn small primary`,"onclick":() => lifecycle('resume'),}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`play`,"size":12,}});}  }
          }
        }else{
           { svelteHTML.createElement("button", {   "class":`btn small primary`,"onclick":() => lifecycle('start'),}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`play`,"size":12,}});}  }
        }
        
         { svelteHTML.createElement("button", {               "class":`icon-btn`,...__sveltets_2_empty({"data-overflow":`-10`}),...__sveltets_2_empty({"data-icon":`more`}),...__sveltets_2_empty({"data-label":`More actions…`}),"aria-haspopup":`menu`,"aria-label":`More actions`,"title":`More actions`,"onclick":swarmMenu,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":14,}});} }
      }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {badge, actions} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
    
    
   PageHeader}

   { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"fill":true,"padded":false,}});
   { svelteHTML.createElement("div", { "class":`swarm-split`,});
  
  if(showRail){
   { const $$_aside3 = svelteHTML.createElement("aside", {     "class":`rail`,"style":viewport.isMobile ? '' : `width:${railW}px`,});railEl = $$_aside3;viewport.isMobile && !railOpen;
     { svelteHTML.createElement("div", { "class":`rail-head`,});
       { const $$_button5 = svelteHTML.createElement("button", {          "class":`rail-toggle`,"onclick":() => (railOpen = !railOpen),"aria-expanded":railOpen,"aria-label":`Toggle swarms list`,});railToggle = $$_button5;
        if(viewport.isMobile){
           { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":railOpen ? 'chevronDown' : 'chevronRight',"size":13,}});}
        }
         { svelteHTML.createElement("span", { "class":`section-title`,});  }
        if(viewport.isMobile && !railOpen && detail){
           { svelteHTML.createElement("span", { "class":`rail-current ellipsis`,}); detail.name; }
        }
       }
       { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => (showNew = true),"aria-label":`New swarm`,"title":`New swarm`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":14,}});} }
     }
     { svelteHTML.createElement("div", { "class":`rail-list`,});
       { const $$_etatSdaoL5C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL5C({ target: __sveltets_2_any(), props: {             children:() => { return __sveltets_2_any(0); },"what":`swarms`,"variant":`compact`,"loading":listLoading,"error":swarm.swarmsError,"empty":swarm.swarms.length === 0,"onretry":() => ws.currentId && void loadList(ws.currentId),}});
           for(let s of __sveltets_2_ensureArray(swarm.swarms)){s.id;
           { svelteHTML.createElement("button", {          "class":`swarm-item`,"aria-current":detail?.id === s.id ? 'true' : undefined,"title":`${s.name} · ${sentenceCase(s.status)}`,"onclick":() => void pickSwarm(s.id),});detail?.id === s.id;
             { svelteHTML.createElement("span", { "class":`grow ellipsis`,});s.name; }
             { svelteHTML.createElement("span", {   "class":`dot ${s.status}`,"aria-hidden":`true`,}); }
             { svelteHTML.createElement("span", { "class":`rail-status`,});sentenceCase(s.status); }
           }
        }
       LoadState}
     }
   }

  }

  if(!viewport.isMobile && showRail){
     { const $$_rediviDenaP3C = __sveltets_2_ensureComponent(PaneDivider); const $$_rediviDenaP3 = new $$_rediviDenaP3C({ target: __sveltets_2_any(), props: {       width:railW,"storageKey":`swarm.railW`,"label":`Resize the swarms list`,}});/*Ωignore_startΩ*/() => railW = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_rediviDenaP3.$$bindings = 'width';}
  }

  
   { svelteHTML.createElement("section", { "class":`main`,});
    if(!detail){
      if(openError){
         { const $$_etatSdaoL4C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL4C({ target: __sveltets_2_any(), props: {          "what":`this swarm`,"variant":`page`,"error":openError,"empty":true,"onretry":() => openErrorId && void openSwarm(openErrorId),}});}
      } else if (swarm.loading || (listLoading && swarm.swarms.length === 0) || (swarm.swarms.length > 0 && !viewport.isPhone && autoPickedFor !== ws.currentId)){
         { svelteHTML.createElement("div", { "class":`main-loading`,}); { const $$_notelekS5C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS5C({ target: __sveltets_2_any(), props: {      "rows":6,"height":30,"label":`the swarm`,}});} }
      } else if (swarm.swarmsError && swarm.swarms.length === 0){
         { const $$_etatSdaoL4C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL4C({ target: __sveltets_2_any(), props: {          "what":`swarms`,"variant":`page`,"error":swarm.swarmsError,"empty":true,"onretry":() => ws.currentId && void loadList(ws.currentId),}});}
      } else if (swarm.swarms.length > 0){
        
         { const $$_etatSytpmE4C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE4C({ target: __sveltets_2_any(), props: {         "variant":`page`,"icon":`grid`,"title":`${swarm.swarms.length} ${swarm.swarms.length === 1 ? 'swarm' : 'swarms'} in ${ws.current?.name ?? 'this workspace'}`,"body":`Open one from the list to see its org tree, board, runs and feed.`,}});}
      }else{
        
         { const $$_etatSytpmE4C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE4C({ target: __sveltets_2_any(), props: {               children:() => { return __sveltets_2_any(0); },"variant":`page`,"icon":`grid`,"actionIcon":`plus`,"title":`No swarms in ${ws.current?.name ?? 'this workspace'}`,"body":`A swarm is a team of role-specialized agents that work projects together — pick a preset or start blank. Swarms belong to the workspace they were created in.`,"actionLabel":`New swarm`,"onaction":() => (showNew = true),}});
           { svelteHTML.createElement("div", { "class":`elsewhere`,});
            if(swarm.elsewhere.length > 0){
               { svelteHTML.createElement("p", { "class":`dim`,});          }
               { svelteHTML.createElement("div", { "class":`ws-hits`,});
                   for(let w of __sveltets_2_ensureArray(swarm.elsewhere)){w.id;
                   { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => ws.select(w.id),});
                    w.name;  plural(w.count, 'swarm');
                   }
                }
               }
            } else if (swarm.elsewhereChecked){
               { svelteHTML.createElement("p", { "class":`dim`,});         }
            }else{
               { svelteHTML.createElement("button", {       "class":`btn small ghost`,"disabled":swarm.elsewhereBusy,"onclick":() => swarm.findSwarmsElsewhere(ws.workspaces),});
                swarm.elsewhereBusy ? 'Searching…' : 'Look in my other workspaces';
               }
            }
           }
           { const $$_ediuGetamotuA5C = __sveltets_2_ensureComponent(AutomateGuide); new $$_ediuGetamotuA5C({ target: __sveltets_2_any(), props: {  "current":`swarm`,}});}
         EmptyState}
      }
    }else{
      const runsCap = detail.max_total_runs;
      const runsUsed = detail.counts.total_runs;
       { svelteHTML.createElement("div", { "class":`switcher`,});
         { svelteHTML.createElement("div", {         "class":`segmented seg-tabs`,"role":`tablist`,"aria-label":`Swarm view`,"tabindex":-1,"onkeydown":onTabKey,});
             for(let v of __sveltets_2_ensureArray(VIEWS)){v.id;
             { svelteHTML.createElement("button", {                "class":`seg`,"role":`tab`,"id":`swarm-tab-${v.id}`,"aria-selected":view === v.id,"aria-controls":`swarm-view`,"tabindex":view === v.id ? 0 : -1,"onclick":() => (view = v.id),});view === v.id;
               { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":v.icon,"size":12,}});} v.label;
             }
          }
         }
         { svelteHTML.createElement("span", { "class":`grow switcher-spacer`,}); }
        
         { svelteHTML.createElement("div", { "class":`budget-bars`,});
           { svelteHTML.createElement("button", {       "class":`budget-label cap-edit`,"onclick":setRunsCap,"title":runsCap != null
              ? `Run budget: ${runsUsed.toLocaleString()} of ${runsCap.toLocaleString()} runs used. The swarm pauses when it’s spent. Click to change.`
              : `${runsUsed.toLocaleString()} runs so far, no run budget. Click to set one.`,});
            if(runsCap != null){ runsUsed.toLocaleString();  runsCap.toLocaleString();
            }else{ runsUsed.toLocaleString();   }
             { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`edit`,"size":12,}});}
           }
          if(runsCap != null){
            const pct = Math.min(100, (runsUsed / runsCap) * 100);
             { svelteHTML.createElement("div", {       "class":`budget-bar`,"role":`img`,"aria-label":`Run budget ${Math.round(pct)}% used`,"title":`Run budget ${Math.round(pct)}% used`,});
               { svelteHTML.createElement("div", {    "class":`budget-fill`,"style":`width:${pct}%`,});pct > 80; }
             }
          }
          if(detail.max_cost_usd != null){
            const pct = Math.min(100, (detail.counts.cost_usd / detail.max_cost_usd) * 100);
             { svelteHTML.createElement("span", {   "class":`budget-label`,"title":`Estimated cost so far of the cost budget (USD). The swarm pauses when it’s spent.`,}); detail.counts.cost_usd.toFixed(2);  detail.max_cost_usd.toFixed(2); }
             { svelteHTML.createElement("div", {       "class":`budget-bar`,"role":`img`,"aria-label":`Cost budget ${Math.round(pct)}% used`,"title":`Cost budget ${Math.round(pct)}% used`,});
               { svelteHTML.createElement("div", {    "class":`budget-fill`,"style":`width:${pct}%`,});pct > 80; }
             }
          }
         }
         { svelteHTML.createElement("div", {   "class":`cap`,"title":`The most agent sessions this swarm runs at the same time; extra runs wait in the queue`,});
           { svelteHTML.createElement("label", { "for":`swarm-cap`,});  }
           { svelteHTML.createElement("input", {              "id":`swarm-cap`,"class":`input small num`,"type":`number`,"min":`1`,"step":`1`,"value":cap,"onchange":(e) => setCap(e.currentTarget),});}
         }
       }

       { const $$_div4 = svelteHTML.createElement("div", {        "class":`body`,"style":`--view-split:${viewPct}%`,});bodyEl = $$_div4;swarm.selectedSessionId;viewport.isPhone && swarm.selectedSessionId;
         { svelteHTML.createElement("div", {       "class":`view`,"id":`swarm-view`,"role":`tabpanel`,"aria-labelledby":`swarm-tab-${view}`,});
          if(view === 'tree'){
             { const $$_eerTgrO6C = __sveltets_2_ensureComponent(OrgTree); new $$_eerTgrO6C({ target: __sveltets_2_any(), props: {        "onedit":(a) => openEditor(a),"onruntask":runForAgent,"onadd":openEditorWithParent,"onduplicate":duplicateAgent,}});}
          } else if (view === 'graph'){
             { const $$_hparGtnegA6C = __sveltets_2_ensureComponent(AgentGraph); new $$_hparGtnegA6C({ target: __sveltets_2_any(), props: {    "onedit":(a) => openEditor(a),"onadd":openEditorWithParent,}});}
          } else if (view === 'kanban'){
             { const $$_draoBnabnaK6C = __sveltets_2_ensureComponent(KanbanBoard); new $$_draoBnabnaK6C({ target: __sveltets_2_any(), props: {    "onnewproject":openProjectCreate,"oneditproject":openProjectEdit,}});}
          } else if (view === 'runs'){
             { const $$_tsiLsnuR6C = __sveltets_2_ensureComponent(RunsList); new $$_tsiLsnuR6C({ target: __sveltets_2_any(), props: {  "onhire":(p, rid) => { recruitProposal = p; recruitProposalRunId = rid; showRecruit = true; },}});}
          } else if (view === 'board'){
             { const $$_deeFdraoB6C = __sveltets_2_ensureComponent(BoardFeed); new $$_deeFdraoB6C({ target: __sveltets_2_any(), props: {}});}
          }
         }

        if(swarm.selectedSessionId){
          if(!viewport.isPhone){
             {const $$action_0 = __sveltets_2_ensureAction(paneResizer(svelteHTML.mapElementTag('div'),({ value: viewPct, min: 20, max: 80, step: 3, bigStep: 10, onChange: (v) => { viewPct = v; persistViewPct(); }, onReset: () => { viewPct = 55; persistViewPct(); }, onDragStart: startResize, text: (v) => `${Math.round(v)} percent` })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {           "class":`resizer`,"role":`separator`,"aria-label":`Resize the session panel`,"title":RESIZE_TITLE,}); }}
          }
           { svelteHTML.createElement("div", { "class":`session-panel`,});
            swarm.selectedSessionId; {
               { const $$_weiVnoisseS6C = __sveltets_2_ensureComponent(SessionView); new $$_weiVnoisseS6C({ target: __sveltets_2_any(), props: {           "sessionId":swarm.selectedSessionId,"focused":true,"showClose":true,"onfocus":() => {},"onclosepane":() => (swarm.selectedSessionId = null),}});}
            }
           }
        }
       }
    }
   }
   }
   PageBody}
 }

if(showNew){
   { const $$_mrawSweN0C = __sveltets_2_ensureComponent(NewSwarm); new $$_mrawSweN0C({ target: __sveltets_2_any(), props: {  "onclose":() => (showNew = false),}});}
}
if(showRecruit){
   { const $$_draziWretiurceR0C = __sveltets_2_ensureComponent(RecruiterWizard); new $$_draziWretiurceR0C({ target: __sveltets_2_any(), props: {       "proposal":recruitProposal,"proposalRunId":recruitProposalRunId,"onclose":() => { showRecruit = false; recruitProposal = null; recruitProposalRunId = null; },}});}
}
if(editorOpen){
   { const $$_rotidEtnegA0C = __sveltets_2_ensureComponent(AgentEditor); new $$_rotidEtnegA0C({ target: __sveltets_2_any(), props: {       "agent":editAgent,"prefill":editAgent ? null : editorPrefill,"onclose":() => { editorOpen = false; editorPrefill = null; },}});}
}
if(showSettings){
   { const $$_sgnitteSmrawS0C = __sveltets_2_ensureComponent(SwarmSettings); new $$_sgnitteSmrawS0C({ target: __sveltets_2_any(), props: {  "onclose":() => (showSettings = false),}});}
}
if(projModal){
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":projEditId ? 'Edit project' : 'New project',"width":480,"onclose":() => (projModal = false),footer:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("button", {   "class":`btn ghost`,"onclick":() => (projModal = false),});  }
       { svelteHTML.createElement("button", {       "class":`btn primary`,"onclick":saveProject,"disabled":!projName.trim() || projSaving,"title":projName.trim() ? undefined : 'Name the project first',});
        projSaving ? 'Saving…' : projEditId ? 'Save' : 'Create project';
       }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
    if(projEditId){
       { svelteHTML.createElement("div", { "class":`proj-actions`,});
         { svelteHTML.createElement("button", {             "class":`icon-btn`,"aria-label":`More actions for ${projName.trim() || 'this project'}`,"title":`More actions`,"aria-haspopup":`menu`,"onclick":projectMenu,"disabled":projSaving,});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":14,}});}
         }
       }
    }
     { svelteHTML.createElement("div", { "class":`field`,}); { svelteHTML.createElement("label", { "for":`p-name`,});  } { svelteHTML.createElement("input", {        "dir":`auto`,"id":`p-name`,"class":`input`,"bind:value":projName,});/*Ωignore_startΩ*/() => projName = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
     { svelteHTML.createElement("div", { "class":`field`,}); { svelteHTML.createElement("label", { "for":`p-repo`,});      } { const $$_dleiFhtaP2C = __sveltets_2_ensureComponent(PathField); const $$_dleiFhtaP2 = new $$_dleiFhtaP2C({ target: __sveltets_2_any(), props: {  children:() => { return __sveltets_2_any(0); },value:projRepo,}});/*Ωignore_startΩ*/() => projRepo = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_dleiFhtaP2.$$bindings = 'value'; { svelteHTML.createElement("input", {          "dir":`ltr`,"id":`p-repo`,"class":`input`,"bind:value":projRepo,"placeholder":`/path/to/repo`,});/*Ωignore_startΩ*/() => projRepo = __sveltets_2_any(null);/*Ωignore_endΩ*/} PathField} }
     { svelteHTML.createElement("div", { "class":`field`,}); { svelteHTML.createElement("label", { "for":`p-goal`,});       } { svelteHTML.createElement("textarea", {         "dir":`auto`,"id":`p-goal`,"class":`input`,"rows":3,"bind:value":projGoal,});/*Ωignore_startΩ*/() => projGoal = __sveltets_2_any(null);/*Ωignore_endΩ*/ } }
     { svelteHTML.createElement("div", { "class":`field`,}); { const $$_rekciPllikS2C = __sveltets_2_ensureComponent(SkillPicker); new $$_rekciPllikS2C({ target: __sveltets_2_any(), props: {      "label":`Project skills (optional)`,"selected":projSkills,"onchange":(s) => (projSkills = s),}});} }
    
   Modal}
}
if(showBudgetModal){
  if(detail){
    const budgetTitle = 'Raise budget & resume';
     { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":budgetTitle,"width":360,"onclose":() => (showBudgetModal = false),footer:() => { async ()/*Ωignore_positionΩ*/ => {
         { svelteHTML.createElement("button", {       "class":`btn ghost`,"style":`margin-inline-end:auto`,"title":`Resume with the current budget — the swarm pauses again as soon as it is spent`,"onclick":async () => { showBudgetModal = false; await lifecycle('resume'); },});   }
         { svelteHTML.createElement("button", {   "class":`btn ghost`,"onclick":() => (showBudgetModal = false),});  }
         { svelteHTML.createElement("button", {     "class":`btn primary`,"onclick":raiseBudgetAndResume,"disabled":!(extraRuns > 0) && !(extraCostUsd > 0),});   }
      };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
      if(detail.pause_reason){ { svelteHTML.createElement("p", { "class":`dim`,});detail.pause_reason; }}
      if(detail.max_total_runs != null){
         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("label", { "for":`extra-runs`,});    detail.max_total_runs;  }
           { svelteHTML.createElement("input", {          "id":`extra-runs`,"class":`input`,"type":`number`,"min":`1`,"bind:value":extraRuns,});/*Ωignore_startΩ*/() => extraRuns = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
      }
      if(detail.max_cost_usd != null){
         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("label", { "for":`extra-cost`,});      detail.max_cost_usd.toFixed(2);  }
           { svelteHTML.createElement("input", {            "id":`extra-cost`,"class":`input`,"type":`number`,"min":`0`,"step":`1`,"bind:value":extraCostUsd,});/*Ωignore_startΩ*/() => extraCostUsd = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
      }
      
     Modal}
  }
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const SwarmPage__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SwarmPage__SvelteComponent_ = ReturnType<typeof SwarmPage__SvelteComponent_>;
/*Ωignore_endΩ*/export default SwarmPage__SvelteComponent_;