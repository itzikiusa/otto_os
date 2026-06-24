<script lang="ts">
  // Agent Swarm section: swarm list + the open swarm (org tree, run graph,
  // kanban, runs, board) with an inline session panel (reuses SessionView).
  import Icon from '../../lib/components/Icon.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import SessionView from '../agents/SessionView.svelte';
  import OrgTree from './OrgTree.svelte';
  import AgentGraph from './AgentGraph.svelte';
  import KanbanBoard from './KanbanBoard.svelte';
  import RunsList from './RunsList.svelte';
  import BoardFeed from './BoardFeed.svelte';
  import NewSwarm from './NewSwarm.svelte';
  import RecruiterWizard from './RecruiterWizard.svelte';
  import AgentEditor from './AgentEditor.svelte';
  import { swarm } from '../../lib/stores/swarm.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import type { CreateAgentReq, RecruitedAgent, Swarm, SwarmAgent } from './types';

  type View = 'tree' | 'graph' | 'kanban' | 'runs' | 'board';
  let view = $state<View>('tree');

  // --- Phone chrome (≤640px) ----------------------------------------------
  // On a phone the Swarms rail + the swarm header would eat most of the screen
  // before the chosen view even starts. Both become collapsible sections so the
  // view gets the room; desktop/tablet keep the original always-open chrome.
  // `railOpen` defaults closed once a swarm is open (you've made your pick);
  // `headOpen` keeps the title + primary lifecycle action visible while tucking
  // the budget/parallel/secondary controls away until tapped.
  let railOpen = $state(true);
  let headOpen = $state(false);
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
  let viewPct = $state(55); // width % of the board/view; rest goes to the session
  let bodyEl = $state<HTMLDivElement | null>(null);
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
      window.removeEventListener('mousemove', onMove);
      window.removeEventListener('mouseup', onUp);
      document.body.style.userSelect = '';
    };
    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onUp);
    document.body.style.userSelect = 'none';
  }
  let editAgent = $state<SwarmAgent | null>(null);
  let editorOpen = $state(false);
  let projModal = $state(false);
  let projName = $state('');
  let projRepo = $state('');
  let projGoal = $state('');

  // Load swarms for the current workspace.
  $effect(() => {
    const id = ws.currentId;
    if (id) swarm.loadSwarms(id);
  });

  // A deep-link (e.g. Product → Swarm) opened a project and asked for the
  // Kanban board — honor it once, then clear the flag.
  $effect(() => {
    if (swarm.pendingKanban) {
      view = 'kanban';
      swarm.pendingKanban = false;
    }
  });

  const detail = $derived(swarm.detail);
  const queued = $derived(swarm.runs.filter((r) => r.status === 'queued').length);
  const running = $derived(swarm.runs.filter((r) => r.status === 'running' || r.status === 'waiting').length);
  const cap = $derived(detail?.config.max_parallel_sessions ?? 4);

  const VIEWS: { id: View; label: string; icon: string }[] = [
    { id: 'tree', label: 'Org', icon: 'user' },
    { id: 'graph', label: 'Graph', icon: 'split' },
    { id: 'kanban', label: 'Board', icon: 'note' },
    { id: 'runs', label: 'Runs', icon: 'clock' },
    { id: 'board', label: 'Feed', icon: 'comment' },
  ];

  async function lifecycle(action: 'start' | 'pause' | 'abort' | 'resume') {
    if (!detail) return;
    try {
      await swarm.lifecycle(action, detail.id);
    } catch (e) {
      toasts.error(`${action} failed`, e instanceof Error ? e.message : String(e));
    }
  }

  async function setCap(v: number) {
    if (!detail || v < 1) return;
    await swarm.setParallelCap(detail.id, v);
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
    if (next !== null && !Number.isFinite(next)) return;
    await swarm.updateSwarm(detail.id, { max_total_runs: next } as Partial<Swarm>);
  }

  async function createProject() {
    if (!detail || !projName.trim()) return;
    await swarm.createProject(detail.id, {
      name: projName.trim(),
      repo_path: projRepo.trim() || undefined,
      goal_md: projGoal.trim() || undefined,
    });
    projName = '';
    projRepo = '';
    projGoal = '';
    projModal = false;
    view = 'kanban';
  }

  async function deleteSwarm() {
    if (!detail) return;
    if (await confirmer.ask(`Delete swarm "${detail.name}" and all its agents/projects?`, { title: 'Delete swarm?' })) {
      await swarm.deleteSwarm(detail.id);
    }
  }

  async function runForAgent(a: SwarmAgent) {
    const pid = swarm.selectedProjectId;
    if (!pid) {
      toasts.warn('Create a project first');
      return;
    }
    const title = await confirmer.promptText('Task for ' + a.name, { title: 'Run a task', confirmLabel: 'Create & run' });
    if (!title) return;
    const created = await swarm.createTask(pid, { title, assignee_agent_id: a.id });
    await swarm.runTask(created);
    view = 'kanban';
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
    await swarm.updateSwarm(detail.id, patch);
    await lifecycle('resume');
    showBudgetModal = false;
  }
</script>

<div class="swarm-page" class:phone={viewport.isPhone}>
  <!-- Swarms rail — a plain sidebar on desktop/tablet; a collapsible accordion
       section on a phone (tap the header to toggle the list). -->
  <aside class="rail" class:collapsed={viewport.isPhone && !railOpen}>
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
      <button class="icon-btn" onclick={() => (showNew = true)} aria-label="New swarm"><Icon name="plus" size={15} /></button>
    </div>
    <div class="rail-list">
      {#each swarm.swarms as s (s.id)}
        <button class="swarm-item" class:active={detail?.id === s.id} onclick={() => { swarm.openSwarm(s.id); if (viewport.isPhone) railOpen = false; }}>
          <span class="grow ellipsis">{s.name}</span>
          <span class="dot {s.status}" title={s.status}></span>
        </button>
      {/each}
      {#if swarm.swarms.length === 0}
        <p class="dim empty">No swarms yet.</p>
      {/if}
    </div>
  </aside>

  <!-- Main -->
  <section class="main">
    {#if !detail}
      <EmptyState
        icon="user"
        title="Build an agent swarm"
        body="A team of role-specialized agents that work projects together — pick a preset or start blank."
        actionLabel="New swarm"
        onaction={() => (showNew = true)}
      />
    {:else}
      <header class="page-header swarm-head" class:head-collapsed={viewport.isPhone && !headOpen}>
        <div class="title-wrap">
          {#if viewport.isPhone}
            <button class="head-toggle" onclick={() => (headOpen = !headOpen)} aria-expanded={headOpen} aria-label="Toggle swarm controls">
              <Icon name={headOpen ? 'chevronDown' : 'chevronRight'} size={14} />
            </button>
          {/if}
          <h2 class="ellipsis">{detail.name}</h2>
          <span class="status-pill {detail.status}">{detail.status}</span>
          <span class="dim counts">{detail.counts.agents} agents · {detail.counts.projects} projects · {running} running · {queued} queued</span>
        </div>
        {#if detail.pause_reason}
          <span class="pause-reason" title={detail.pause_reason}>Paused: {detail.pause_reason}</span>
        {/if}
        <!-- Collapsible-on-phone controls: budget meters, parallel cap, lifecycle
             + recruit/project/delete. Always shown on desktop/tablet. -->
        <div class="head-controls">
          <div class="budget-bars">
            <button class="budget-label dim cap-edit" onclick={setRunsCap} title="Click to change the run budget (blank = unlimited)">
              {#if detail.max_total_runs != null}
                runs {detail.counts.total_runs}/{detail.max_total_runs}
              {:else}
                runs {detail.counts.total_runs} · ∞
              {/if}
            </button>
            {#if detail.max_total_runs != null}
              {@const pct = Math.min(100, (detail.counts.total_runs / detail.max_total_runs) * 100)}
              <div class="budget-bar" title="Run budget: {detail.counts.total_runs}/{detail.max_total_runs}">
                <div class="budget-fill" class:budget-warn={pct > 80} style="width:{pct}%"></div>
              </div>
            {/if}
            {#if detail.max_cost_usd != null}
              {@const pct = Math.min(100, (detail.counts.cost_usd / detail.max_cost_usd) * 100)}
              <span class="budget-label dim">cost ${detail.counts.cost_usd.toFixed(2)}/${detail.max_cost_usd.toFixed(2)}</span>
              <div class="budget-bar" title="Cost budget: ${detail.counts.cost_usd.toFixed(2)}/${detail.max_cost_usd.toFixed(2)}">
                <div class="budget-fill" class:budget-warn={pct > 80} style="width:{pct}%"></div>
              </div>
            {/if}
          </div>
          <div class="grow"></div>
          <div class="cap">
            <label for="cap">parallel</label>
            <input id="cap" class="input small num" type="number" min="1" value={cap} onchange={(e) => setCap(Number((e.target as HTMLInputElement).value))} />
          </div>
          {#if detail.status === 'active'}
            <button class="btn small" onclick={() => lifecycle('pause')}><Icon name="square" size={12} /> Pause</button>
            <button class="btn small danger" onclick={() => lifecycle('abort')}><Icon name="x" size={12} /> Abort all</button>
          {:else if detail.status === 'paused'}
            <button class="btn small primary" onclick={() => lifecycle('resume')}><Icon name="play" size={12} /> Resume</button>
            {#if detail.pause_reason}
              <button class="btn small" onclick={() => (showBudgetModal = true)}><Icon name="play" size={12} /> Raise budget & resume</button>
            {/if}
            <button class="btn small danger" onclick={() => lifecycle('abort')}><Icon name="x" size={12} /> Abort all</button>
          {:else}
            <button class="btn small primary" onclick={() => lifecycle('start')}><Icon name="play" size={12} /> Start</button>
          {/if}
          <button class="btn small" onclick={() => (showRecruit = true)}><Icon name="plus" size={12} /> Recruit</button>
          <button class="btn small" onclick={() => (projModal = true)}><Icon name="note" size={12} /> Project</button>
          <button class="icon-btn" onclick={deleteSwarm} aria-label="delete swarm"><Icon name="trash" size={14} /></button>
        </div>
      </header>

      <div class="switcher">
        {#each VIEWS as v (v.id)}
          <button class="seg" class:active={view === v.id} onclick={() => (view = v.id)}>
            <Icon name={v.icon} size={13} /> {v.label}
          </button>
        {/each}
      </div>

      <div
        class="body"
        bind:this={bodyEl}
        class:split={swarm.selectedSessionId}
        class:phone-split={viewport.isPhone && swarm.selectedSessionId}
        style="--view-split:{viewPct}%"
      >
        <div class="view">
          {#if view === 'tree'}
            <OrgTree onedit={(a) => openEditor(a)} onruntask={runForAgent} onadd={openEditorWithParent} onduplicate={duplicateAgent} />
          {:else if view === 'graph'}
            <AgentGraph />
          {:else if view === 'kanban'}
            <KanbanBoard onrecruit={() => (showRecruit = true)} />
          {:else if view === 'runs'}
            <RunsList onhire={(p, rid) => { recruitProposal = p; recruitProposalRunId = rid; showRecruit = true; }} />
          {:else if view === 'board'}
            <BoardFeed />
          {/if}
        </div>

        {#if swarm.selectedSessionId}
          {#if !viewport.isPhone}
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <div class="resizer" title="Drag to resize" onmousedown={startResize}></div>
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
{#if projModal}
  <Modal title="New project" width={480} onclose={() => (projModal = false)}>
    <div class="field"><label for="p-name">Name</label><input id="p-name" class="input" bind:value={projName} /></div>
    <div class="field"><label for="p-repo">Repo path (optional, for code projects)</label><input id="p-repo" class="input" bind:value={projRepo} placeholder="/path/to/repo" /></div>
    <div class="field"><label for="p-goal">Goal (optional, used by Plan from goal)</label><textarea id="p-goal" class="input" rows={3} bind:value={projGoal}></textarea></div>
    {#snippet footer()}
      <button class="btn" class:ghost={true} onclick={() => (projModal = false)}>Cancel</button>
      <button class="btn" class:primary={true} onclick={createProject} disabled={!projName.trim()}>Create</button>
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
          <label for="extra-cost">Add budget $USD (current max: ${detail.max_cost_usd})</label>
          <input id="extra-cost" class="input" type="number" min="0" step="1" bind:value={extraCostUsd} />
        </div>
      {/if}
      {#snippet footer()}
        <button class="btn" class:ghost={true} onclick={() => (showBudgetModal = false)}>Cancel</button>
        <button class="btn" class:primary={true} onclick={raiseBudgetAndResume}>Raise &amp; resume</button>
      {/snippet}
    </Modal>
  {/if}
{/if}

<style>
  .swarm-page {
    display: flex;
    height: 100%;
    min-height: 0;
  }
  .rail {
    width: 220px;
    flex: none;
    border-inline-end: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .rail-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 12px;
    border-bottom: 1px solid var(--border);
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
    font-size: 13px;
  }
  .swarm-item:hover {
    background: color-mix(in srgb, var(--text-dim) 10%, transparent);
  }
  .swarm-item.active {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    color: var(--accent);
  }
  .empty {
    padding: 12px;
    font-size: 12px;
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
  .swarm-head {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .title-wrap {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .title-wrap h2 {
    margin: 0;
    font-size: 16px;
  }
  .counts {
    font-size: 11.5px;
  }
  .pause-reason {
    font-size: 11px;
    color: var(--status-exited);
    background: color-mix(in srgb, var(--status-exited) 10%, transparent);
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
    gap: 5px;
    flex-wrap: wrap;
  }
  .budget-label {
    font-size: 10.5px;
    white-space: nowrap;
  }
  .cap-edit {
    border: none;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    padding: 0;
  }
  .cap-edit:hover {
    color: var(--text);
    text-decoration: underline;
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
  .budget-fill.budget-warn {
    background: var(--status-exited);
  }
  .status-pill {
    font-size: 10.5px;
    padding: 1px 8px;
    border-radius: 999px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    background: color-mix(in srgb, var(--text-dim) 18%, transparent);
    color: var(--text-dim);
  }
  .status-pill.active {
    background: color-mix(in srgb, var(--status-working) 22%, transparent);
    color: var(--status-working);
  }
  .status-pill.aborted {
    background: color-mix(in srgb, var(--status-exited) 22%, transparent);
    color: var(--status-exited);
  }
  .cap {
    display: flex;
    align-items: center;
    gap: 5px;
    font-size: 11px;
    color: var(--text-dim);
  }
  .num {
    width: 52px;
  }
  .switcher {
    display: flex;
    gap: 4px;
    padding: 6px 10px;
    border-bottom: 1px solid var(--border);
  }
  .seg {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    border: 1px solid transparent;
    background: transparent;
    color: var(--text-dim);
    border-radius: var(--radius-s);
    padding: 4px 10px;
    font-size: 12px;
    cursor: pointer;
  }
  .seg:hover {
    background: color-mix(in srgb, var(--text-dim) 10%, transparent);
    color: var(--text);
  }
  .seg.active {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    color: var(--accent);
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

  /* Toggles — invisible chrome on desktop (the rail/header are always open);
     they only carry the chevron + tappable target on a phone. */
  .rail-toggle,
  .head-toggle {
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
  .head-toggle {
    flex: none;
    color: var(--text-dim);
  }
  .rail-current {
    font-size: 11.5px;
    color: var(--text-dim);
    max-width: 160px;
  }
  .head-controls {
    display: contents;
  }

  @media (max-width: 640px) {
    .swarm-page.phone {
      flex-direction: column;
    }
    /* Rail = collapsible accordion. Header is a tappable toggle; the list
       scrolls within a capped height when open, and is hidden when collapsed. */
    .swarm-page.phone .rail {
      width: 100%;
      flex: none;
      border-inline-end: none;
      border-bottom: 1px solid var(--border);
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
      font-size: 14px;
    }
    .swarm-page.phone .rail-list {
      max-height: 38vh;
      overflow-y: auto;
    }
    .swarm-page.phone .rail.collapsed .rail-list {
      display: none;
    }
    .swarm-page.phone .swarm-item {
      font-size: 14px;
      padding: 11px 12px;
    }

    /* Header: title row always visible; the controls block collapses behind the
       title's toggle so the chosen view gets the vertical room back. */
    .swarm-page.phone .swarm-head {
      padding: 10px 14px;
      gap: 8px 10px;
      align-items: flex-start;
    }
    .swarm-page.phone .title-wrap {
      flex: 1 1 100%;
      min-width: 0;
      flex-wrap: wrap;
      row-gap: 4px;
    }
    .swarm-page.phone .title-wrap h2 {
      font-size: 16px;
      flex: 1 1 auto;
      min-width: 0;
      max-width: 100%;
    }
    /* Counts move to their own full-width line so the swarm name keeps its room. */
    .swarm-page.phone .title-wrap .counts {
      flex: 1 1 100%;
    }
    .swarm-page.phone .head-controls {
      display: flex;
      flex: 1 1 100%;
      flex-wrap: wrap;
      align-items: center;
      gap: 8px 8px;
    }
    .swarm-page.phone .head-collapsed .head-controls {
      display: none;
    }
    .swarm-page.phone .head-controls .btn.small {
      font-size: 13px;
      padding: 7px 10px;
    }
    .swarm-page.phone .cap {
      font-size: 12px;
    }
    .swarm-page.phone .budget-bars {
      flex: 1 1 100%;
    }
    .swarm-page.phone .budget-bar {
      flex: 1;
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
      font-size: 13px;
      padding: 7px 12px;
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
      border-bottom: 1px solid var(--border);
    }
    .swarm-page.phone .body.phone-split .session-panel {
      flex: 1 1 55%;
      min-height: 0;
    }
  }
</style>
