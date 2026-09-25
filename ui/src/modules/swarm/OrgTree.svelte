<script lang="ts">
  // Recursive org tree (CEO → … → devs) by `reports_to`. Each node shows the
  // agent, a status dot, task/run counts, and its open sessions (click → open).
  // Supports drag-and-drop to reparent agents within the hierarchy.
  import Icon from '../../lib/components/Icon.svelte';
  import { sentenceCase } from '../../lib/status';
  import { swarm } from '../../lib/stores/swarm.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
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
  // Orphans (reports_to names an agent that no longer exists — the daemon does
  // not cascade a manager's delete) are roots too; otherwise they vanished from
  // the tree entirely while still showing in the Graph view.
  const roots = $derived.by(() => {
    const ids = new Set(agents.map((a) => a.id));
    return agents.filter((a) => !a.reports_to || !ids.has(a.reports_to));
  });
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

  // Reparent from the menu or a drop; a failed PATCH must not vanish silently
  // (the row would just snap back with no explanation).
  async function reparent(aid: string, reportsTo: string | null) {
    try {
      await swarm.updateAgent(aid, { reports_to: reportsTo });
    } catch (e) {
      toasts.error("Couldn't move the agent", e instanceof Error ? e.message : String(e));
    }
  }

  // Deleting an agent is irreversible (its config, soul and schedule go with
  // it), so it confirms like every other swarm delete does.
  async function deleteAgent(a: SwarmAgent) {
    const reports = childrenOf(a.id).length;
    const extra =
      reports === 0
        ? ''
        : reports === 1
          ? ' Its direct report moves to the top level.'
          : ` Its ${reports} direct reports move to the top level.`;
    const ok = await confirmer.ask(`Delete agent “${a.name}”?${extra} This cannot be undone.`, {
      title: 'Delete agent',
      confirmLabel: 'Delete',
      danger: true,
    });
    if (!ok) return;
    try {
      await swarm.deleteAgent(a.id);
    } catch (e) {
      toasts.error("Couldn't delete the agent", e instanceof Error ? e.message : String(e));
    }
  }

  function menu(e: MouseEvent, a: SwarmAgent) {
    ctxMenu.show(e, [
      { label: 'Edit agent', icon: 'edit', action: () => onedit(a) },
      { label: 'Duplicate agent', icon: 'split', action: () => onduplicate?.(a) },
      { label: 'Run a task…', icon: 'play', action: () => onruntask(a) },
      { label: 'Add direct report', icon: 'plus', action: () => onadd?.(a) },
      { separator: true },
      { label: 'Move to top level', icon: 'user', action: () => reparent(a.id, null) },
      { separator: true },
      { label: 'Delete agent', icon: 'trash', danger: true, action: () => deleteAgent(a) },
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
    await reparent(aid, newParentId);
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
      <input type="checkbox" bind:checked={showCompleted} /> Show finished sessions
    </label>
  </div>
  {#if roots.length === 0}
    <EmptyState
      icon="user"
      title="No agents yet"
      body="Recruit agents in the header, or add one by hand. They report to each other in an org tree; drag a row onto another to change who it reports to."
      actionLabel={onadd ? 'Add agent' : undefined}
      actionIcon="plus"
      onaction={() => onadd?.(null)}
    />
  {:else if draggingAgentId}
    <div class="drop-zone" class:drop-active={dropTargetId === null}>
      <Icon name="user" size={13} /> Drop here to make top-level
    </div>
  {/if}
  {#each roots as r (r.id)}
    {@render node(r, 0)}
  {/each}
  {#if !draggingAgentId && roots.length > 0}
    <button class="add-top-btn dim" onclick={() => onadd?.(null)} title="Add an agent at the top level">
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
    class="row org-row"
    class:drag-over={dropTargetId === a.id}
    class:dragging-self={draggingAgentId === a.id}
    style="padding-inline-start:{depth * 14 + 6}px"
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
      <button class="twist" onclick={() => toggle(a.id)} aria-label={isOpen ? `Collapse ${a.name}` : `Expand ${a.name}`} aria-expanded={isOpen}>
        <Icon name={isOpen ? 'chevronDown' : 'chevronRight'} size={12} />
      </button>
    {:else}
      <span class="twist-spacer"></span>
    {/if}
    <!-- Status sits on the avatar (presence-style) so it reads with the name
         instead of floating at the far edge of a wide pane. Green only while
         the agent is actually working; an idle, enabled agent is grey. -->
    <span class="avatar-wrap">
      <span class="avatar" aria-hidden="true">{a.avatar || a.name.slice(0, 1)}</span>
      {#if a.status === 'paused'}
        <span class="presence paused" role="img" aria-label="Paused" title="Paused — picks up no new work"><Icon name="pause" size={8} /></span>
      {:else}
        <span class="presence state {running > 0 ? 'working' : 'idle'}" role="img" aria-label={running > 0 ? 'Working' : 'Idle'} title={running > 0 ? 'Working' : 'Idle — waiting for work'}></span>
      {/if}
    </span>
    <!-- The name opens the agent's editor (the most common action); the rest
         are in ⋯ / right-click. -->
    <button class="who" onclick={() => onedit(a)} title={a.title ? `${a.name} — ${a.title} · Edit agent` : `${a.name} · Edit agent`}>
      <span class="name">{a.name}</span>
      {#if a.title}<span class="title dim">{a.title}</span>{/if}
    </button>
    {#if a.schedule?.enabled}
      <span class="badge" role="img" aria-label="Runs on a schedule" title="Runs on a schedule"><Icon name="clock" size={12} /></span>
    {/if}
    {#if running > 0}
      <span class="chip runs-chip" title="{running} active run{running === 1 ? '' : 's'}"><span class="run-dot" aria-hidden="true"></span>{running}</span>
    {/if}
    <span class="grow"></span>
    <span class="row-tools">
      <button class="icon-btn small" onclick={() => onadd?.(a)} aria-label="Add a direct report to {a.name}" title="Add direct report">
        <Icon name="plus" size={12} />
      </button>
      <button class="icon-btn small" onclick={(e) => menu(e, a)} aria-label="Actions for {a.name}" title="Agent actions">
        <Icon name="more" size={14} />
      </button>
    </span>
  </div>
  {#if isOpen}
    {#each sessions as s (s.id)}
      <button
        class="session-row"
        class:selected={swarm.selectedSessionId === s.id}
        style="padding-inline-start:{depth * 14 + 30}px"
        onclick={() => (swarm.selectedSessionId = s.id)}
        title="Open session: {s.title || s.provider}"
        aria-current={swarm.selectedSessionId === s.id ? 'true' : undefined}
      >
        <Icon name="terminal" size={12} />
        <span class="grow mono ellipsis">{s.title || s.provider}</span>
        <span class="state {ws.statusMap[s.id] ?? s.status}" role="img" aria-label={sentenceCase(ws.statusMap[s.id] ?? s.status)} title={sentenceCase(ws.statusMap[s.id] ?? s.status)}></span>
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
  .tree-bar {
    display: flex;
    justify-content: flex-end;
    padding: 4px 12px;
    border-block-end: 1px solid var(--border);
  }
  .show-done {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: var(--fs-s);
    color: var(--text-dim);
    cursor: pointer;
  }
  .org-row {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 36px;
    padding-block: 3px;
    box-sizing: border-box;
    cursor: grab;
  }
  .org-row:hover {
    background: var(--hover);
  }
  /* Row tools surface on hover / keyboard focus (always shown on touch). */
  .row-tools {
    display: inline-flex;
    align-items: center;
    padding-inline-end: 6px;
    opacity: 0;
  }
  .org-row:hover .row-tools,
  .org-row:focus-within .row-tools {
    opacity: 1;
  }
  @media (hover: none) {
    .row-tools {
      opacity: 1;
    }
  }
  .runs-chip {
    gap: 4px;
    font-variant-numeric: tabular-nums;
  }
  .run-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--status-working);
  }
  .org-row.drag-over {
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    outline: 1px dashed color-mix(in srgb, var(--accent) 60%, transparent);
  }
  .org-row.dragging-self {
    opacity: 0.4;
  }
  .drop-zone {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    font-size: var(--fs-s);
    color: var(--text-dim);
    border: 1px dashed var(--border);
    border-radius: var(--radius-s);
    margin: 4px 6px;
  }
  .drop-zone.drop-active {
    background: color-mix(in srgb, var(--accent) 10%, transparent);
    border-color: color-mix(in srgb, var(--accent) 50%, transparent);
    color: var(--accent-text);
  }
  .add-top-btn {
    display: flex;
    align-items: center;
    gap: 5px;
    width: 100%;
    border: none;
    background: transparent;
    color: var(--text-dim);
    font-size: var(--fs-s);
    padding: 6px 12px;
    cursor: pointer;
    text-align: start;
  }
  .add-top-btn:hover {
    color: var(--text);
    background: var(--hover);
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
    width: 24px;
    height: 24px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    font-size: var(--fs-s);
    background: var(--surface-2);
    border: 1px solid var(--border);
    flex: none;
  }
  .who {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 1px;
    line-height: 1.3;
    overflow: hidden;
    min-width: 0;
    border: none;
    background: transparent;
    color: var(--text);
    padding: 0;
    text-align: start;
    cursor: pointer;
    font: inherit;
  }
  .who:hover .name {
    text-decoration: underline;
  }
  .who > span {
    max-width: 100%;
  }
  /* One line each — a long name/title used to wrap inside the fixed 30px row
     and get clipped vertically with no ellipsis (full text is in the title). */
  .name,
  .title,
  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .name {
    font-size: var(--fs-m);
    font-weight: 600;
  }
  .title {
    font-size: var(--fs-xs);
  }
  .avatar-wrap {
    position: relative;
    flex: none;
  }
  .presence {
    position: absolute;
    inset-inline-end: -2px;
    inset-block-end: -2px;
    box-shadow: 0 0 0 2px var(--bg);
  }
  .presence.paused {
    width: 12px;
    height: 12px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .state {
    width: 8px;
    height: 8px;
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
  /* A paused agent is parked, not failed — same idle tone the swarm rail uses
     for a paused swarm (red stays for an exited session). */
  .state.paused {
    background: var(--status-idle, var(--text-dim));
  }
  .state.exited {
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
    font-size: var(--fs-s);
    padding-inline-end: 12px;
  }
  .session-row:hover {
    background: var(--hover);
    color: var(--text);
  }
  .session-row.selected {
    background: var(--accent-soft);
    color: var(--text);
  }
</style>
