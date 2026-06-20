<script lang="ts">
  // Agent Swarm section: swarm list + the open swarm (org tree, run graph,
  // kanban, runs, board) with an inline session panel (reuses SessionView).
  import Icon from '../../lib/components/Icon.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import SessionView from '../agents/SessionView.svelte';
  import OrgTree from './OrgTree.svelte';
  import RunGraph from './RunGraph.svelte';
  import KanbanBoard from './KanbanBoard.svelte';
  import RunsList from './RunsList.svelte';
  import BoardFeed from './BoardFeed.svelte';
  import NewSwarm from './NewSwarm.svelte';
  import RecruiterWizard from './RecruiterWizard.svelte';
  import AgentEditor from './AgentEditor.svelte';
  import { swarm } from '../../lib/stores/swarm.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import type { SwarmAgent } from './types';

  type View = 'tree' | 'graph' | 'kanban' | 'runs' | 'board';
  let view = $state<View>('tree');

  let showNew = $state(false);
  let showRecruit = $state(false);
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
    if (await confirmer.ask(`Delete swarm “${detail.name}” and all its agents/projects?`, { title: 'Delete swarm?' })) {
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
    await swarm.createTask(pid, { title, assignee_agent_id: a.id });
    const t = swarm.tasks(pid).find((x) => x.title === title && x.assignee_agent_id === a.id);
    if (t) await swarm.runTask(t);
    view = 'kanban';
  }

  function openEditor(a: SwarmAgent | null) {
    editAgent = a;
    editorOpen = true;
  }
</script>

<div class="swarm-page">
  <!-- Swarms rail -->
  <aside class="rail">
    <div class="rail-head">
      <span class="section-title">Swarms</span>
      <button class="icon-btn" onclick={() => (showNew = true)} aria-label="New swarm"><Icon name="plus" size={15} /></button>
    </div>
    <div class="rail-list">
      {#each swarm.swarms as s (s.id)}
        <button class="swarm-item" class:active={detail?.id === s.id} onclick={() => swarm.openSwarm(s.id)}>
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
      <header class="page-header swarm-head">
        <div class="title-wrap">
          <h2>{detail.name}</h2>
          <span class="status-pill {detail.status}">{detail.status}</span>
          <span class="dim counts">{detail.counts.agents} agents · {detail.counts.projects} projects · {running} running · {queued} queued</span>
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
          <button class="btn small danger" onclick={() => lifecycle('abort')}><Icon name="x" size={12} /> Abort all</button>
        {:else}
          <button class="btn small primary" onclick={() => lifecycle('start')}><Icon name="play" size={12} /> Start</button>
        {/if}
        <button class="btn small" onclick={() => (showRecruit = true)}><Icon name="plus" size={12} /> Recruit</button>
        <button class="btn small" onclick={() => (projModal = true)}><Icon name="note" size={12} /> Project</button>
        <button class="icon-btn" onclick={deleteSwarm} aria-label="delete swarm"><Icon name="trash" size={14} /></button>
      </header>

      <div class="switcher">
        {#each VIEWS as v (v.id)}
          <button class="seg" class:active={view === v.id} onclick={() => (view = v.id)}>
            <Icon name={v.icon} size={13} /> {v.label}
          </button>
        {/each}
      </div>

      <div class="body" class:split={swarm.selectedSessionId}>
        <div class="view">
          {#if view === 'tree'}
            <OrgTree onedit={(a) => openEditor(a)} onruntask={runForAgent} />
          {:else if view === 'graph'}
            <RunGraph />
          {:else if view === 'kanban'}
            <KanbanBoard />
          {:else if view === 'runs'}
            <RunsList />
          {:else if view === 'board'}
            <BoardFeed />
          {/if}
        </div>

        {#if swarm.selectedSessionId}
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
  <RecruiterWizard onclose={() => (showRecruit = false)} />
{/if}
{#if editorOpen}
  <AgentEditor agent={editAgent} onclose={() => (editorOpen = false)} />
{/if}
{#if projModal}
  <Modal title="New project" width={480} onclose={() => (projModal = false)}>
    <div class="field"><label for="p-name">Name</label><input id="p-name" class="input" bind:value={projName} /></div>
    <div class="field"><label for="p-repo">Repo path (optional, for code projects)</label><input id="p-repo" class="input" bind:value={projRepo} placeholder="/path/to/repo" /></div>
    <div class="field"><label for="p-goal">Goal (optional — used by “Plan from goal”)</label><textarea id="p-goal" class="input" rows="3" bind:value={projGoal}></textarea></div>
    {#snippet footer()}
      <button class="btn ghost" onclick={() => (projModal = false)}>Cancel</button>
      <button class="btn primary" onclick={createProject} disabled={!projName.trim()}>Create</button>
    {/snippet}
  </Modal>
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
    border-right: 1px solid var(--border);
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
    text-align: left;
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
  .body.split .view {
    flex: 0 0 55%;
    border-right: 1px solid var(--border);
  }
  .session-panel {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
  }

  @media (max-width: 640px) {
    .swarm-page {
      flex-direction: column;
    }
    .rail {
      width: 100%;
      border-right: none;
      border-bottom: 1px solid var(--border);
      max-height: 30vh;
      overflow-y: auto;
    }
  }
</style>
