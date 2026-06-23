<script lang="ts">
  // Recursive org tree (CEO → … → devs) by `reports_to`. Each node shows the
  // agent, a status dot, task/run counts, and its open sessions (click → open).
  // Supports drag-and-drop to reparent agents within the hierarchy.
  import Icon from '../../lib/components/Icon.svelte';
  import { swarm } from '../../lib/stores/swarm.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import type { SwarmAgent } from './types';

  interface Props {
    onedit: (a: SwarmAgent) => void;
    onruntask: (a: SwarmAgent) => void;
    /** Called when + button is clicked to add a new direct report to `parent`. */
    onadd?: (parent: SwarmAgent | null) => void;
    /** Duplicate `a` as a new agent (e.g. same role on a different model). */
    onduplicate?: (a: SwarmAgent) => void;
  }
  let { onedit, onruntask, onadd, onduplicate }: Props = $props();

  const agents = $derived(swarm.detail?.agents ?? []);
  const roots = $derived(agents.filter((a) => !a.reports_to));
  const childrenOf = (id: string) => agents.filter((a) => a.reports_to === id);

  // Hide finished (exited) task sessions by default — show only active work.
  let showCompleted = $state(false);

  // Sessions this agent holds (tagged at spawn with meta.swarm_id + meta.agent_id).
  function agentSessions(agentId: string) {
    const sid = swarm.detail?.id;
    return ws.sessions.filter((s) => {
      const m = (s.meta ?? {}) as Record<string, unknown>;
      if (s.archived || m.swarm_id !== sid || m.agent_id !== agentId) return false;
      const st = ws.statusMap[s.id] ?? s.status;
      return showCompleted || st !== 'exited';
    });
  }

  function runCount(agentId: string): number {
    return swarm.runs.filter((r) => r.agent_id === agentId && (r.status === 'running' || r.status === 'waiting')).length;
  }

  let open = $state<Record<string, boolean>>({});
  function toggle(id: string) {
    open[id] = !(open[id] ?? true);
    open = { ...open };
  }

  function menu(e: MouseEvent, a: SwarmAgent) {
    ctxMenu.show(e, [
      { label: 'Edit agent', icon: 'edit', action: () => onedit(a) },
      { label: 'Duplicate agent', icon: 'split', action: () => onduplicate?.(a) },
      { label: 'Run a task…', icon: 'play', action: () => onruntask(a) },
      { label: 'Add direct report', icon: 'plus', action: () => onadd?.(a) },
      { separator: true },
      { label: 'Move to top level', icon: 'user', action: () => swarm.updateAgent(a.id, { reports_to: null }) },
      { separator: true },
      { label: 'Delete agent', icon: 'trash', danger: true, action: () => swarm.deleteAgent(a.id) },
    ]);
  }

  // --- Drag-and-drop reparenting -------------------------------------------
  let draggingAgentId = $state<string | null>(null);
  let dropTargetId = $state<string | null>(null); // null = root drop zone

  function onAgentDragStart(e: DragEvent, a: SwarmAgent) {
    draggingAgentId = a.id;
    e.dataTransfer?.setData('text/plain', a.id);
    if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move';
  }

  function onAgentDragEnd() {
    draggingAgentId = null;
    dropTargetId = null;
  }

  function onNodeDragOver(e: DragEvent, targetId: string | null) {
    // Prevent dropping on self or a descendant.
    if (draggingAgentId === targetId) return;
    if (targetId !== null && isDescendant(draggingAgentId, targetId)) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
    dropTargetId = targetId;
  }

  function onNodeDragLeave(targetId: string | null) {
    if (dropTargetId === targetId) dropTargetId = null;
  }

  async function onNodeDrop(e: DragEvent, newParentId: string | null) {
    e.preventDefault();
    const aid = e.dataTransfer?.getData('text/plain') ?? draggingAgentId;
    draggingAgentId = null;
    dropTargetId = null;
    if (!aid) return;
    const a = agents.find((x) => x.id === aid);
    if (!a) return;
    if (a.reports_to === newParentId) return; // no change
    if (newParentId !== null && isDescendant(aid, newParentId)) return; // cycle guard
    await swarm.updateAgent(aid, { reports_to: newParentId });
  }

  /** Returns true if `potentialChild` is a descendant of `ancestorId`. */
  function isDescendant(potentialChildId: string | null, ancestorId: string): boolean {
    if (!potentialChildId) return false;
    let cur: string | null = ancestorId;
    let safety = 0;
    while (cur && safety++ < 50) {
      const a = agents.find((x) => x.id === cur);
      if (!a) break;
      if (a.reports_to === potentialChildId) return true;
      cur = a.reports_to ?? null;
    }
    return false;
  }
</script>

<div
  class="tree"
  role="tree"
  tabindex="-1"
  ondragover={(e) => onNodeDragOver(e, null)}
  ondragleave={() => onNodeDragLeave(null)}
  ondrop={(e) => onNodeDrop(e, null)}
>
  <div class="tree-bar">
    <label class="show-done">
      <input type="checkbox" bind:checked={showCompleted} /> Show completed
    </label>
  </div>
  {#if roots.length === 0}
    <p class="dim pad">No agents yet. Use <strong>Recruit</strong> to add one.</p>
  {:else if draggingAgentId}
    <div class="drop-zone" class:drop-active={dropTargetId === null}>
      <Icon name="user" size={13} /> Drop here to make top-level
    </div>
  {/if}
  {#each roots as r (r.id)}
    {@render node(r, 0)}
  {/each}
  {#if !draggingAgentId}
    <button class="add-top-btn dim" onclick={() => onadd?.(null)} title="Add agent at top level">
      <Icon name="plus" size={13} /> Add agent
    </button>
  {/if}
</div>

{#snippet node(a: SwarmAgent, depth: number)}
  {@const kids = childrenOf(a.id)}
  {@const sessions = agentSessions(a.id)}
  {@const isOpen = open[a.id] ?? true}
  {@const running = runCount(a.id)}
  <div
    class="row"
    class:drag-over={dropTargetId === a.id}
    class:dragging-self={draggingAgentId === a.id}
    style="padding-left:{depth * 14 + 6}px"
    draggable="true"
    ondragstart={(e) => onAgentDragStart(e, a)}
    ondragend={onAgentDragEnd}
    ondragover={(e) => onNodeDragOver(e, a.id)}
    ondragleave={() => onNodeDragLeave(a.id)}
    ondrop={(e) => onNodeDrop(e, a.id)}
    oncontextmenu={(e) => menu(e, a)}
    role="treeitem"
    aria-selected="false"
    tabindex="-1"
  >
    {#if kids.length > 0 || sessions.length > 0}
      <button class="twist" onclick={() => toggle(a.id)} aria-label="toggle">
        <Icon name={isOpen ? 'chevronDown' : 'chevronRight'} size={12} />
      </button>
    {:else}
      <span class="twist-spacer"></span>
    {/if}
    <span class="avatar">{a.avatar || a.name.slice(0, 1)}</span>
    <span class="who grow">
      <span class="name">{a.name}</span>
      <span class="title dim">{a.title}</span>
    </span>
    {#if a.schedule?.enabled}
      <span class="badge" title="scheduled"><Icon name="clock" size={11} /></span>
    {/if}
    {#if running > 0}
      <span class="chip accent" title="active runs">{running}●</span>
    {/if}
    <span class="state {a.status}" title={a.status}></span>
    <button class="icon-btn small" onclick={() => onadd?.(a)} aria-label="add direct report" title="Add direct report">
      <Icon name="plus" size={11} />
    </button>
    <button class="icon-btn small" onclick={(e) => menu(e, a)} aria-label="agent menu">
      <Icon name="dot" size={14} />
    </button>
  </div>
  {#if isOpen}
    {#each sessions as s (s.id)}
      <button
        class="session-row"
        class:selected={swarm.selectedSessionId === s.id}
        style="padding-left:{depth * 14 + 30}px"
        onclick={() => (swarm.selectedSessionId = s.id)}
      >
        <Icon name="terminal" size={12} />
        <span class="grow mono">{s.title || s.provider}</span>
        <span class="state {ws.statusMap[s.id] ?? s.status}"></span>
      </button>
    {/each}
    {#each kids as k (k.id)}
      {@render node(k, depth + 1)}
    {/each}
  {/if}
{/snippet}

<style>
  .tree {
    overflow: auto;
    height: 100%;
    padding: 4px 0;
  }
  .pad {
    padding: 12px;
  }
  .tree-bar {
    display: flex;
    justify-content: flex-end;
    padding: 4px 10px;
    border-bottom: 1px solid var(--border);
  }
  .show-done {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 11px;
    color: var(--text-dim);
    cursor: pointer;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 30px;
    cursor: grab;
  }
  .row:hover {
    background: color-mix(in srgb, var(--text-dim) 10%, transparent);
  }
  .row.drag-over {
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    outline: 1px dashed color-mix(in srgb, var(--accent) 60%, transparent);
  }
  .row.dragging-self {
    opacity: 0.4;
  }
  .drop-zone {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    font-size: 11.5px;
    color: var(--text-dim);
    border: 1px dashed var(--border);
    border-radius: var(--radius-s);
    margin: 4px 6px;
  }
  .drop-zone.drop-active {
    background: color-mix(in srgb, var(--accent) 10%, transparent);
    border-color: color-mix(in srgb, var(--accent) 50%, transparent);
    color: var(--accent);
  }
  .add-top-btn {
    display: flex;
    align-items: center;
    gap: 5px;
    width: 100%;
    border: none;
    background: transparent;
    color: var(--text-dim);
    font-size: 11.5px;
    padding: 6px 10px;
    cursor: pointer;
    text-align: start;
  }
  .add-top-btn:hover {
    color: var(--text);
    background: color-mix(in srgb, var(--text-dim) 8%, transparent);
  }
  .twist,
  .twist-spacer {
    width: 16px;
    height: 16px;
    display: grid;
    place-items: center;
    border: none;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    flex: none;
  }
  .avatar {
    width: 20px;
    height: 20px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    font-size: 12px;
    background: color-mix(in srgb, var(--accent) 22%, transparent);
    flex: none;
  }
  .who {
    display: flex;
    flex-direction: column;
    line-height: 1.1;
    overflow: hidden;
  }
  .name {
    font-size: 12.5px;
    font-weight: 600;
  }
  .title {
    font-size: 10.5px;
  }
  .state {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex: none;
    background: var(--text-dim);
  }
  .state.active,
  .state.running,
  .state.working {
    background: var(--status-working);
  }
  .state.idle,
  .state.reconnectable {
    background: var(--status-idle, var(--text-dim));
  }
  .state.exited,
  .state.paused {
    background: var(--status-exited);
  }
  .badge {
    color: var(--text-dim);
    display: grid;
    place-items: center;
  }
  .session-row {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 26px;
    width: 100%;
    border: none;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    text-align: start;
    font-size: 11.5px;
  }
  .session-row:hover {
    background: color-mix(in srgb, var(--text-dim) 10%, transparent);
    color: var(--text);
  }
  .session-row.selected {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    color: var(--accent);
  }
</style>
