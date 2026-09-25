<script lang="ts">
  import PathField from '../../lib/components/PathField.svelte';
  // Agent Swarm section: swarm list + the open swarm (org tree, run graph,
  // kanban, runs, board) with an inline session panel (reuses SessionView).
  import Icon, { type IconName } from '../../lib/components/Icon.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
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
  import { ws } from '../../lib/stores/workspace.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import type { CreateAgentReq, RecruitedAgent, Swarm, SwarmAgent, SwarmProject } from './types';

  type View = 'tree' | 'graph' | 'kanban' | 'runs' | 'board';
  let view = $state<View>('tree');
  /** Swarm lifecycle → the shared badge tone (patterns.md §1). */
  const SWARM_TONE: Record<string, Tone> = { active: 'success', paused: 'neutral', aborted: 'danger' };

  // --- Phone chrome (≤640px) ----------------------------------------------
  // On a phone the Swarms rail + the swarm header would eat most of the screen
  // before the chosen view even starts. Both become collapsible sections so the
  // view gets the room; desktop/tablet keep the original always-open chrome.
  // `railOpen` defaults closed once a swarm is open (you've made your pick);
  // (The swarm's secondary actions live in the shared PageHeader, which
  // collapses whatever doesn't fit into its "⋯" menu — no header toggle.)
  let railOpen = $state(true);
  // Auto-collapse the rail when a swarm opens on a phone (one-time per open).
  let lastOpenedId = $state<string | null>(null);
  $effect(() => {
    const id = swarm.detail?.id ?? null;
    if (viewport.isPhone && id && id !== lastOpenedId) {
      railOpen = false;
    }
    lastOpenedId = id;
  });

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
    const onMove = (ev: MouseEvent) => {
      const rect = el.getBoundingClientRect();
      const pct = ((ev.clientX - rect.left) / rect.width) * 100;
      viewPct = Math.min(80, Math.max(20, pct));
    };
    const onUp = () => {
      persistViewPct();
      window.removeEventListener('mousemove', onMove);
      window.removeEventListener('mouseup', onUp);
      document.body.style.userSelect = '';
    };
    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onUp);
    document.body.style.userSelect = 'none';
  }

  // --- Resizable swarms rail (drag the divider) -----------------------------
  // Same idea as the Database page's sidebar resizer: drag to taste, width
  // persists, double-click resets. On a phone the rail is a full-width
  // accordion band, so the width binding is skipped and the divider hidden.
  const RAIL_W_DEFAULT = 220;
  let railW = $state(loadRailW());
  function loadRailW(): number {
    if (typeof localStorage === 'undefined') return RAIL_W_DEFAULT;
    const v = Number(localStorage.getItem('swarm.railW'));
    return Number.isFinite(v) && v >= 180 ? Math.min(400, v) : RAIL_W_DEFAULT;
  }
  function persistRailW(): void {
    try {
      localStorage.setItem('swarm.railW', String(Math.round(railW)));
    } catch {
      /* storage unavailable — non-fatal */
    }
  }
  function startRailResize(e: PointerEvent): void {
    e.preventDefault();
    const startX = e.clientX;
    const startW = railW;
    const onMove = (ev: PointerEvent): void => {
      // The rail is pinned to the LEFT edge, so dragging RIGHT widens it.
      railW = Math.max(180, Math.min(400, startW + (ev.clientX - startX)));
    };
    const onUp = (): void => {
      persistRailW();
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
    };
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
  }
  function resetRailW(): void {
    railW = RAIL_W_DEFAULT;
    persistRailW();
  }
  function onRailKey(e: KeyboardEvent): void {
    const step = e.shiftKey ? 40 : 10;
    if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') {
      // Logical direction: in RTL the rail sits on the right, so → narrows it.
      const rtl = getComputedStyle(e.currentTarget as HTMLElement).direction === 'rtl';
      const grow = (e.key === 'ArrowRight') !== rtl;
      railW = Math.max(180, Math.min(400, railW + (grow ? step : -step)));
      persistRailW();
      e.preventDefault();
    } else if (e.key === 'Enter' || e.key === 'Home') {
      resetRailW();
      e.preventDefault();
    }
  }
  /** Keyboard resize for the view/session divider (same keys as the rail). */
  function onSplitKey(e: KeyboardEvent): void {
    const step = e.shiftKey ? 10 : 3;
    if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') {
      const rtl = getComputedStyle(e.currentTarget as HTMLElement).direction === 'rtl';
      const grow = (e.key === 'ArrowRight') !== rtl;
      viewPct = Math.min(80, Math.max(20, viewPct + (grow ? step : -step)));
      persistViewPct();
      e.preventDefault();
    } else if (e.key === 'Enter' || e.key === 'Home') {
      viewPct = 55;
      persistViewPct();
      e.preventDefault();
    }
  }
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
  async function openSwarm(id: string): Promise<void> {
    openError = null;
    openErrorId = null;
    try {
      await swarm.openSwarm(id);
    } catch (e) {
      openError = swarm.detailError ?? loadErrorText(e);
      openErrorId = id;
    }
  }

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
    if (swarm.detail || swarm.loading) {
      autoPickedFor = wsId;
      return;
    }
    if (swarm.swarms.length === 0) return;
    autoPickedFor = wsId;
    const id = initialSelection('swarm', swarm.swarms, (s) => s.id);
    if (id) void openSwarm(id);
  });
  $effect(() => {
    if (detail?.id) rememberSelection('swarm', detail.id);
  });
  // The rail is pointless while there is nothing to list (and no load error to
  // retry): the page-level empty state owns the page then.
  // (A failed first load is shown by the main pane's inline error + Retry.)
  const showRail = $derived(swarm.swarms.length > 0);
  const queued = $derived(swarm.runs.filter((r) => r.status === 'queued').length);
  const running = $derived(swarm.runs.filter((r) => r.status === 'running' || r.status === 'waiting').length);
  const cap = $derived(detail?.config.max_parallel_sessions ?? 4);

  // View switcher = a real tablist: ←/→ (and Home/End) move between views,
  // with roving tabindex so Tab lands on the active one only.
  function onTabKey(e: KeyboardEvent): void {
    const i = VIEWS.findIndex((v) => v.id === view);
    let next = -1;
    if (e.key === 'ArrowRight' || e.key === 'ArrowDown') next = (i + 1) % VIEWS.length;
    else if (e.key === 'ArrowLeft' || e.key === 'ArrowUp') next = (i - 1 + VIEWS.length) % VIEWS.length;
    else if (e.key === 'Home') next = 0;
    else if (e.key === 'End') next = VIEWS.length - 1;
    if (next < 0) return;
    e.preventDefault();
    view = VIEWS[next].id;
    const list = (e.currentTarget as HTMLElement).querySelectorAll<HTMLButtonElement>('[role="tab"]');
    list[next]?.focus();
  }

  const VIEWS: { id: View; label: string; icon: IconName }[] = [
    { id: 'tree', label: 'Org', icon: 'user' },
    { id: 'graph', label: 'Graph', icon: 'split' },
    { id: 'kanban', label: 'Board', icon: 'note' },
    { id: 'runs', label: 'Runs', icon: 'clock' },
    { id: 'board', label: 'Feed', icon: 'comment' },
  ];

  const LIFECYCLE_FAILED: Record<'start' | 'pause' | 'abort' | 'resume', string> = {
    start: "Couldn't start the swarm",
    pause: "Couldn't pause the swarm",
    abort: "Couldn't abort the swarm",
    resume: "Couldn't resume the swarm",
  };

  async function lifecycle(action: 'start' | 'pause' | 'abort' | 'resume') {
    if (!detail) return;
    // Abort kills every swarm session and cancels queued/running runs — the
    // in-flight work is lost, so it names the blast radius and asks first.
    if (action === 'abort') {
      const agents = running === 1 ? '1 running agent is' : `${running} running agents are`;
      const q = queued === 1 ? '1 queued run is' : `${queued} queued runs are`;
      const ok = await confirmer.ask(
        `Abort “${detail.name}”? ${agents} stopped and their sessions closed, and ${q} cancelled. Work in progress is lost.`,
        { title: 'Abort swarm', confirmLabel: 'Abort all', danger: true },
      );
      if (!ok) return;
    }
    try {
      await swarm.lifecycle(action, detail.id);
    } catch (e) {
      toasts.error(LIFECYCLE_FAILED[action], e instanceof Error ? e.message : String(e));
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
      toasts.error("Couldn't change parallel sessions", e instanceof Error ? e.message : String(e));
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
      toasts.warn('Run budget not changed', `“${t}” isn't a number — enter a whole number, or leave it blank for unlimited.`);
      return;
    }
    try {
      await swarm.updateSwarm(detail.id, { max_total_runs: next } as Partial<Swarm>);
    } catch (e) {
      toasts.error("Couldn't change the run budget", e instanceof Error ? e.message : String(e));
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
      toasts.error("Couldn't save the project", e instanceof Error ? e.message : String(e));
    } finally {
      projSaving = false;
    }
  }

  async function deleteProject() {
    if (!projEditId) return;
    const p = detail?.projects.find((x) => x.id === projEditId);
    if (
      await confirmer.ask(
        `Delete project "${p?.name ?? projName}"? All its tasks and feed are removed and in-flight runs are stopped. This cannot be undone.`,
        { title: 'Delete project', confirmLabel: 'Delete', danger: true },
      )
    ) {
      try {
        await swarm.deleteProject(projEditId);
        projModal = false;
        toasts.success('Project deleted');
      } catch (e) {
        toasts.error("Couldn't delete the project", e instanceof Error ? e.message : String(e));
      }
    }
  }

  async function deleteSwarm() {
    if (!detail) return;
    const d = detail;
    if (
      await confirmer.ask(
        `Delete swarm "${d.name}" and all its agents, projects and tasks? This cannot be undone.`,
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
        toasts.error("Couldn't delete the swarm", e instanceof Error ? e.message : String(e));
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
      toasts.error("Couldn't run the task", e instanceof Error ? e.message : String(e));
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
      toasts.error("Couldn't raise the budget", e instanceof Error ? e.message : String(e));
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
        ? [{ label: 'Abort all…', icon: 'square', danger: true, title: 'Stop every agent and cancel queued runs', action: () => void lifecycle('abort') }]
        : []),
      { label: 'Delete swarm…', icon: 'trash', danger: true, action: () => void deleteSwarm() },
    ]);
  }

  const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? '' : 's'}`;
</script>

<div class="swarm-page" class:phone={viewport.isPhone}>
  <PageHeader
    class="swarm-head"
    title={detail?.name ?? 'Swarm'}
    subtitle={detail ? `${plural(detail.counts.agents, 'agent')} · ${plural(detail.counts.projects, 'project')} · ${running} running · ${queued} queued` : undefined}
  >
    {#snippet badge()}
      {#if detail}
        <span class="status-pill" data-status={detail.status}>
          <StatusBadge tone={SWARM_TONE[detail.status] ?? 'neutral'} label={sentenceCase(detail.status)} />
        </span>
        {#if detail.pause_reason}
          <span class="pause-reason" title={detail.pause_reason}>Paused: {detail.pause_reason}</span>
        {/if}
      {/if}
    {/snippet}
    {#snippet actions()}
      {#if detail}
        <!-- Settings · Recruit · ⋯ · lifecycle. The destructive verbs (Abort all,
             Delete swarm) live in ⋯, one step away from the lifecycle action —
             the same shape as a goal loop's and a workflow's header. "New
             project" lives on the Board, where projects are shown. -->
        <button class="btn small" data-overflow="-1" data-icon="gear" onclick={() => (showSettings = true)} title="Standing goals, team skills & channel triggers" data-label="Settings"><Icon name="gear" size={12} /> Settings</button>
        <button class="btn small" data-icon="plus" title="Let the Recruiter propose an agent for you to review and hire" onclick={() => (showRecruit = true)}><Icon name="plus" size={12} /> Recruit</button>
        <button class="icon-btn" data-keep aria-haspopup="menu" aria-label="More actions" title="More actions" onclick={swarmMenu}><Icon name="more" size={14} /></button>
        {#if detail.status === 'active'}
          <button class="btn small" data-keep title="Stop picking up new runs; running agents finish their current step" onclick={() => lifecycle('pause')}><Icon name="pause" size={12} /> Pause</button>
        {:else if detail.status === 'paused'}
          {#if detail.pause_reason}
            <!-- A budget stop: resuming without headroom would pause again at
                 once, so raising the budget is the primary path (plain Resume
                 is offered inside the sheet). -->
            <button class="btn small primary" onclick={() => (showBudgetModal = true)}><Icon name="play" size={12} /> Raise budget & resume…</button>
          {:else}
            <button class="btn small primary" onclick={() => lifecycle('resume')}><Icon name="play" size={12} /> Resume</button>
          {/if}
        {:else}
          <button class="btn small primary" onclick={() => lifecycle('start')}><Icon name="play" size={12} /> Start</button>
        {/if}
      {/if}
    {/snippet}
  </PageHeader>

  <div class="swarm-split">
  <!-- Swarms rail — a plain sidebar on desktop/tablet; a collapsible accordion
       section on a phone (tap the header to toggle the list). -->
  {#if showRail}
  <aside class="rail" class:collapsed={viewport.isPhone && !railOpen} style={viewport.isPhone ? '' : `width:${railW}px`}>
    <div class="rail-head">
      <button
        class="rail-toggle"
        onclick={() => (railOpen = !railOpen)}
        aria-expanded={railOpen}
        aria-label="Toggle swarms list"
      >
        {#if viewport.isPhone}
          <Icon name={railOpen ? 'chevronDown' : 'chevronRight'} size={13} />
        {/if}
        <span class="section-title">Swarms</span>
        {#if viewport.isPhone && !railOpen && detail}
          <span class="rail-current ellipsis">· {detail.name}</span>
        {/if}
      </button>
      <button class="icon-btn" onclick={() => (showNew = true)} aria-label="New swarm" title="New swarm"><Icon name="plus" size={15} /></button>
    </div>
    <div class="rail-list">
      <LoadState
        what="swarms"
        variant="compact"
        loading={listLoading}
        error={swarm.swarmsError}
        empty={swarm.swarms.length === 0}
        onretry={() => ws.currentId && void loadList(ws.currentId)}
      >
        {#each swarm.swarms as s (s.id)}
          <button
            class="swarm-item"
            class:active={detail?.id === s.id}
            aria-current={detail?.id === s.id ? 'true' : undefined}
            title="{s.name} · {sentenceCase(s.status)}"
            onclick={() => { void openSwarm(s.id); if (viewport.isPhone) railOpen = false; }}
          >
            <span class="grow ellipsis">{s.name}</span>
            <span class="dot {s.status}" role="img" aria-label={sentenceCase(s.status)}></span>
          </button>
        {/each}
      </LoadState>
    </div>
  </aside>

  {/if}

  {#if !viewport.isPhone && showRail}
    <!-- A focusable separator is a widget in ARIA (←/→ resize, Enter resets);
         Svelte's lint doesn't know that — same exemption as agents/SplitNode. -->
    <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
    <div
      class="side-resizer"
      role="separator"
      tabindex="0"
      aria-orientation="vertical"
      aria-valuemin={180}
      aria-valuemax={400}
      aria-valuenow={Math.round(railW)}
      aria-label="Resize the swarms list"
      title="Drag or use ←/→ to resize · double-click or Enter to reset"
      ondblclick={resetRailW}
      onpointerdown={startRailResize}
      onkeydown={onRailKey}
    ></div>
  {/if}

  <!-- Main -->
  <section class="main">
    {#if !detail}
      {#if openError}
        <LoadState what="this swarm" variant="page" error={openError} empty={true} onretry={() => openErrorId && void openSwarm(openErrorId)} />
      {:else if swarm.loading || (listLoading && swarm.swarms.length === 0) || (swarm.swarms.length > 0 && !viewport.isPhone && autoPickedFor !== ws.currentId)}
        <div class="main-loading" aria-label="Loading swarm"><Skeleton rows={6} height={30} /></div>
      {:else if swarm.swarmsError && swarm.swarms.length === 0}
        <LoadState what="swarms" variant="page" error={swarm.swarmsError} empty={true} onretry={() => ws.currentId && void loadList(ws.currentId)} />
      {:else if swarm.swarms.length > 0}
        <!-- Only reachable on a phone (desktop auto-opens one): the list above
             is the next step, so say so. -->
        <EmptyState
          variant="page"
          icon="grid"
          title="{swarm.swarms.length} {swarm.swarms.length === 1 ? 'swarm' : 'swarms'} in {ws.current?.name ?? 'this workspace'}"
          body="Open one from the list to see its org tree, board, runs and feed."
        />
      {:else}
        <!-- Nothing in THIS workspace. Swarms are workspace-scoped, so name the
             workspace and offer to go looking in the others before concluding
             the team was lost. -->
        <EmptyState
          variant="page"
          icon="grid"
          actionIcon="plus"
          title="No swarms in {ws.current?.name ?? 'this workspace'}"
          body="A swarm is a team of role-specialized agents that work projects together — pick a preset or start blank. Swarms belong to the workspace they were created in."
          actionLabel="New swarm"
          onaction={() => (showNew = true)}
        >
          <div class="elsewhere">
            {#if swarm.elsewhere.length > 0}
              <p class="dim">Found swarms in your other workspaces — jump to one:</p>
              <div class="ws-hits">
                {#each swarm.elsewhere as w (w.id)}
                  <button class="btn small" onclick={() => ws.select(w.id)}>
                    {w.name} · {w.count} swarm{w.count === 1 ? '' : 's'}
                  </button>
                {/each}
              </div>
            {:else if swarm.elsewhereChecked}
              <p class="dim">No swarms in any of your other workspaces either.</p>
            {:else}
              <button
                class="btn small ghost"
                disabled={swarm.elsewhereBusy}
                onclick={() => swarm.findSwarmsElsewhere(ws.workspaces)}
              >
                {swarm.elsewhereBusy ? 'Searching…' : 'Look in my other workspaces'}
              </button>
            {/if}
          </div>
        </EmptyState>
      {/if}
    {:else}
      {@const runsCap = detail.max_total_runs}
      {@const runsUsed = detail.counts.total_runs}
      <div class="switcher">
        <div class="segmented seg-tabs" role="tablist" aria-label="Swarm view" tabindex="-1" onkeydown={onTabKey}>
          {#each VIEWS as v (v.id)}
            <button
              class="seg"
              class:active={view === v.id}
              role="tab"
              id="swarm-tab-{v.id}"
              aria-selected={view === v.id}
              aria-controls="swarm-view"
              tabindex={view === v.id ? 0 : -1}
              onclick={() => (view = v.id)}
            >
              <Icon name={v.icon} size={12} /> {v.label}
            </button>
          {/each}
        </div>
        <span class="grow"></span>
        <!-- Budget meters + parallel cap: swarm-level status/settings that sit
             with the views rather than crowding the page header's actions. -->
        <div class="budget-bars">
          <button
            class="budget-label cap-edit"
            onclick={setRunsCap}
            title={runsCap != null
              ? `Run budget: ${runsUsed.toLocaleString()} of ${runsCap.toLocaleString()} runs used. The swarm pauses when it's spent. Click to change.`
              : `${runsUsed.toLocaleString()} runs so far, no run budget. Click to set one.`}
          >
            {#if runsCap != null}
              Runs {runsUsed.toLocaleString()} / {runsCap.toLocaleString()}
            {:else}
              Runs {runsUsed.toLocaleString()} · no limit
            {/if}
            <Icon name="edit" size={12} />
          </button>
          {#if runsCap != null}
            {@const pct = Math.min(100, (runsUsed / runsCap) * 100)}
            <div class="budget-bar" role="img" aria-label="Run budget {Math.round(pct)}% used" title="Run budget {Math.round(pct)}% used">
              <div class="budget-fill" class:budget-warn={pct > 80} style="width:{pct}%"></div>
            </div>
          {/if}
          {#if detail.max_cost_usd != null}
            {@const pct = Math.min(100, (detail.counts.cost_usd / detail.max_cost_usd) * 100)}
            <span class="budget-label" title="Estimated cost so far of the cost budget (USD). The swarm pauses when it's spent.">Cost ${detail.counts.cost_usd.toFixed(2)} / ${detail.max_cost_usd.toFixed(2)}</span>
            <div class="budget-bar" role="img" aria-label="Cost budget {Math.round(pct)}% used" title="Cost budget {Math.round(pct)}% used">
              <div class="budget-fill" class:budget-warn={pct > 80} style="width:{pct}%"></div>
            </div>
          {/if}
        </div>
        <div class="cap" title="The most agent sessions this swarm runs at the same time; extra runs wait in the queue">
          <label for="swarm-cap">Max parallel</label>
          <input id="swarm-cap" class="input small num" type="number" min="1" step="1" value={cap} onchange={(e) => setCap(e.currentTarget)} />
        </div>
      </div>

      <div
        class="body"
        bind:this={bodyEl}
        class:split={swarm.selectedSessionId}
        class:phone-split={viewport.isPhone && swarm.selectedSessionId}
        style="--view-split:{viewPct}%"
      >
        <div class="view" id="swarm-view" role="tabpanel" aria-labelledby="swarm-tab-{view}">
          {#if view === 'tree'}
            <OrgTree onedit={(a) => openEditor(a)} onruntask={runForAgent} onadd={openEditorWithParent} onduplicate={duplicateAgent} />
          {:else if view === 'graph'}
            <AgentGraph />
          {:else if view === 'kanban'}
            <KanbanBoard onnewproject={openProjectCreate} oneditproject={openProjectEdit} />
          {:else if view === 'runs'}
            <RunsList onhire={(p, rid) => { recruitProposal = p; recruitProposalRunId = rid; showRecruit = true; }} />
          {:else if view === 'board'}
            <BoardFeed />
          {/if}
        </div>

        {#if swarm.selectedSessionId}
          {#if !viewport.isPhone}
            <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
            <div
              class="resizer"
              role="separator"
              tabindex="0"
              aria-orientation="vertical"
              aria-valuemin={20}
              aria-valuemax={80}
              aria-valuenow={Math.round(viewPct)}
              aria-label="Resize the session panel"
              title="Drag or use ←/→ to resize · Enter to reset"
              onmousedown={startResize}
              onkeydown={onSplitKey}
            ></div>
          {/if}
          <div class="session-panel">
            {#key swarm.selectedSessionId}
              <SessionView
                sessionId={swarm.selectedSessionId}
                focused={true}
                showClose={true}
                onfocus={() => {}}
                onclosepane={() => (swarm.selectedSessionId = null)}
              />
            {/key}
          </div>
        {/if}
      </div>
    {/if}
  </section>
  </div>
</div>

{#if showNew}
  <NewSwarm onclose={() => (showNew = false)} />
{/if}
{#if showRecruit}
  <RecruiterWizard
    proposal={recruitProposal}
    proposalRunId={recruitProposalRunId}
    onclose={() => { showRecruit = false; recruitProposal = null; recruitProposalRunId = null; }}
  />
{/if}
{#if editorOpen}
  <AgentEditor
    agent={editAgent}
    prefill={editAgent ? null : editorPrefill}
    onclose={() => { editorOpen = false; editorPrefill = null; }}
  />
{/if}
{#if showSettings}
  <SwarmSettings onclose={() => (showSettings = false)} />
{/if}
{#if projModal}
  <Modal title={projEditId ? 'Edit project' : 'New project'} width={480} onclose={() => (projModal = false)}>
    <div class="field"><label for="p-name">Name</label><input id="p-name" class="input" bind:value={projName} /></div>
    <div class="field"><label for="p-repo">Repo path (optional, for code projects)</label><PathField bind:value={projRepo}><input id="p-repo" class="input" bind:value={projRepo} placeholder="/path/to/repo" /></PathField></div>
    <div class="field"><label for="p-goal">Goal (optional, used by Plan from goal)</label><textarea id="p-goal" class="input" rows={3} bind:value={projGoal}></textarea></div>
    <div class="field"><SkillPicker label="Project skills (optional)" selected={projSkills} onchange={(s) => (projSkills = s)} /></div>
    {#snippet footer()}
      {#if projEditId}
        <button class="btn danger" style="margin-inline-end:auto" onclick={deleteProject} disabled={projSaving} title="Delete this project, its tasks and feed">
          Delete project…
        </button>
      {/if}
      <button class="btn ghost" onclick={() => (projModal = false)}>Cancel</button>
      <button class="btn primary" onclick={saveProject} disabled={!projName.trim() || projSaving} title={projName.trim() ? undefined : 'Name the project first'}>
        {projSaving ? 'Saving…' : projEditId ? 'Save' : 'Create project'}
      </button>
    {/snippet}
  </Modal>
{/if}
{#if showBudgetModal}
  {#if detail}
    {@const budgetTitle = 'Raise budget & resume'}
    <Modal title={budgetTitle} width={360} onclose={() => (showBudgetModal = false)}>
      {#if detail.pause_reason}<p class="dim">{detail.pause_reason}</p>{/if}
      {#if detail.max_total_runs != null}
        <div class="field">
          <label for="extra-runs">Add runs (current max: {detail.max_total_runs})</label>
          <input id="extra-runs" class="input" type="number" min="1" bind:value={extraRuns} />
        </div>
      {/if}
      {#if detail.max_cost_usd != null}
        <div class="field">
          <label for="extra-cost">Add budget in USD (current max: ${detail.max_cost_usd.toFixed(2)})</label>
          <input id="extra-cost" class="input" type="number" min="0" step="1" bind:value={extraCostUsd} />
        </div>
      {/if}
      {#snippet footer()}
        <button class="btn ghost" style="margin-inline-end:auto" title="Resume with the current budget — the swarm pauses again as soon as it is spent" onclick={async () => { showBudgetModal = false; await lifecycle('resume'); }}>Resume without raising</button>
        <button class="btn ghost" onclick={() => (showBudgetModal = false)}>Cancel</button>
        <button class="btn primary" onclick={raiseBudgetAndResume} disabled={!(extraRuns > 0) && !(extraCostUsd > 0)}>Raise &amp; resume</button>
      {/snippet}
    </Modal>
  {/if}
{/if}

<style>
  .swarm-page {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .swarm-split {
    flex: 1;
    display: flex;
    min-height: 0;
  }
  .rail {
    /* Default width; on tablet/desktop an inline `width:{railW}px`
       (drag-resizable, persisted) takes over. */
    width: 220px;
    flex: none;
    border-inline-end: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  /* Draggable divider between the swarms rail and the main area. Sits flush
     against the rail's inline-end border; a hit-area wider than its visible
     line makes it easy to grab. */
  .side-resizer {
    flex: none;
    width: 5px;
    margin-inline-start: -3px;
    cursor: col-resize;
    background: transparent;
    position: relative;
    z-index: 2;
    touch-action: none;
  }
  .side-resizer:hover,
  .side-resizer:focus-visible {
    background: color-mix(in srgb, var(--accent) 45%, transparent);
  }
  .side-resizer:focus-visible,
  .resizer:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -1px;
  }
  .rail-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 12px;
    border-block-end: 1px solid var(--border);
  }
  .rail-list {
    overflow-y: auto;
    padding: 6px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .swarm-item {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    border: none;
    background: transparent;
    border-radius: var(--radius-s);
    color: var(--text);
    cursor: pointer;
    text-align: start;
    font-size: var(--fs-m);
    min-height: 30px;
  }
  .swarm-item:hover {
    background: var(--hover);
  }
  /* Selection = the quiet accent tint + normal text (layout.md list rows). */
  .swarm-item.active {
    background: var(--accent-soft);
    font-weight: 500;
  }
  .main-loading {
    padding: 16px;
  }
  /* "Your swarms are in another workspace" shortcuts, rendered inside the main
     empty state. */
  .elsewhere {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    margin-block-start: 10px;
    font-size: var(--fs-s);
  }
  .elsewhere p {
    margin: 0;
  }
  .ws-hits {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 6px;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex: none;
    background: var(--text-dim);
  }
  .dot.active {
    background: var(--status-working);
  }
  .dot.paused {
    background: var(--status-idle, var(--text-dim));
  }
  .dot.aborted {
    background: var(--status-exited);
  }
  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .main {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }
  /* Paused (usually a budget stop) is an attention state, not a failure —
     warning tone, matching the neutral "Paused" badge beside it. */
  .pause-reason {
    font-size: var(--fs-xs);
    color: var(--warning);
    background: var(--warning-soft);
    border-radius: var(--radius-s);
    padding: 2px 8px;
    max-width: 200px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .budget-bars {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }
  .budget-label {
    font-size: var(--fs-s);
    color: var(--text-dim);
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }
  .cap-edit {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    border: none;
    background: transparent;
    cursor: pointer;
    padding: 2px 4px;
    border-radius: var(--radius-s);
  }
  .cap-edit:hover {
    color: var(--text);
    background: var(--hover);
  }
  .budget-bar {
    width: 60px;
    height: 5px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--text-dim) 20%, transparent);
    overflow: hidden;
  }
  .budget-fill {
    height: 100%;
    border-radius: 999px;
    background: var(--accent);
    transition: width 0.3s;
  }
  @media (prefers-reduced-motion: reduce) {
    .budget-fill {
      transition: none;
    }
  }
  /* >80% of the budget: warn (amber), don't alarm — the run isn't failing. */
  .budget-fill.budget-warn {
    background: var(--warning);
  }
  .status-pill {
    display: inline-flex;
    align-items: center;
  }
  .cap {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-s);
    color: var(--text-dim);
    white-space: nowrap;
  }
  .num {
    width: 52px;
  }
  .switcher {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 6px 12px;
    border-block-end: 1px solid var(--border);
    min-height: 40px;
  }
  /* The shared .segmented control (app.css) supplies the surface-lift active
     state; this only lays the icon + label out. */
  .seg-tabs {
    flex: none;
  }
  .seg-tabs > .seg {
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }
  .seg-tabs > .seg:hover:not(.active) {
    color: var(--text);
  }
  .body {
    flex: 1;
    min-height: 0;
    display: flex;
  }
  .view {
    flex: 1;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
  }
  .body.split:not(.phone-split) .view {
    /* Width is operator-draggable via the .resizer (defaults to 55%). */
    flex: 0 0 var(--view-split, 55%);
  }
  .body.phone-split.split .view {
    flex: 0 0 50%;
    border-block-end: 1px solid var(--border);
  }
  .resizer {
    flex: none;
    width: 5px;
    cursor: col-resize;
    background: var(--border);
    transition: background 0.12s;
  }
  .resizer:hover {
    background: color-mix(in srgb, var(--accent) 60%, var(--border));
  }
  .session-panel {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
  }
  /* SessionView's `.pane` has NO intrinsic width to shrink-wrap: its header is a
     `container-type: inline-size` query container (size-contained on the inline
     axis) and the terminal below it is a canvas. As a plain flex item it
     therefore collapses to its padding — a ~20px sliver with just the status dot
     — so the panel must tell it to fill. (Splits/TiledView escape this because
     their parents are CSS grids, where items stretch by default.) */
  .session-panel > :global(.pane) {
    flex: 1 1 auto;
    min-width: 0;
  }

  /* Toggles — invisible chrome on desktop (the rail/header are always open);
     they only carry the chevron + tappable target on a phone. */
  .rail-toggle {
    display: flex;
    align-items: center;
    gap: 6px;
    border: none;
    background: transparent;
    color: inherit;
    padding: 0;
    cursor: pointer;
    min-width: 0;
  }
  .rail-current {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    max-width: 160px;
  }

  @media (max-width: 640px) {
    .swarm-page.phone .swarm-split {
      flex-direction: column;
    }
    /* Rail = collapsible accordion. Header is a tappable toggle; the list
       scrolls within a capped height when open, and is hidden when collapsed. */
    .swarm-page.phone .rail {
      width: 100%;
      flex: none;
      border-inline-end: none;
      border-block-end: 1px solid var(--border);
      min-height: 0;
    }
    .swarm-page.phone .rail-head {
      padding: 10px 14px;
    }
    .swarm-page.phone .rail-toggle {
      flex: 1;
      padding: 4px 0;
    }
    .swarm-page.phone .rail-toggle .section-title {
      font-size: var(--fs-s);
    }
    .swarm-page.phone .rail-list {
      max-height: 38vh;
      overflow-y: auto;
    }
    .swarm-page.phone .rail.collapsed .rail-list {
      display: none;
    }
    .swarm-page.phone .swarm-item {
      font-size: var(--fs-l);
      padding: 11px 12px;
    }

    .swarm-page.phone .cap {
      flex: none;
    }
    .swarm-page.phone .budget-bars {
      flex: none;
      flex-wrap: nowrap;
    }

    /* View switcher: horizontally scrollable so all 5 tabs are reachable on a
       320px screen, instead of the last one ("Feed") clipping off the edge. */
    .swarm-page.phone .switcher {
      overflow-x: auto;
      flex-wrap: nowrap;
      scrollbar-width: none;
      -webkit-overflow-scrolling: touch;
    }
    .swarm-page.phone .switcher::-webkit-scrollbar {
      display: none;
    }
    .swarm-page.phone .seg {
      flex: none;
      font-size: var(--fs-m);
      height: 32px;
      padding: 0 12px;
    }

    /* When a session panel opens, stack it under the view (vertical split) rather
       than the desktop side-by-side, and let each half scroll on its own. */
    .swarm-page.phone .body.phone-split {
      flex-direction: column;
    }
    .swarm-page.phone .body.phone-split .view {
      flex: 1 1 45%;
      min-height: 0;
      border-inline-end: none;
      border-block-end: 1px solid var(--border);
    }
    .swarm-page.phone .body.phone-split .session-panel {
      flex: 1 1 55%;
      min-height: 0;
    }
  }
</style>
