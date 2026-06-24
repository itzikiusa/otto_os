<script lang="ts">
  // Agent-centric swarm graph: TEAM MEMBERS are the nodes (not tasks), laid out
  // radially around the coordinator by `reports_to`. Click a node → open that
  // agent's session; hover → its live sessions. A side rail shows a per-member
  // brief (completed / to-address) and the most-urgent tasks, which you can DRAG
  // onto a node to assign + launch that agent.
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { swarm } from '../../lib/stores/swarm.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { now } from '../../lib/stores/now.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import type { SwarmAgent, SwarmTask, SwarmRun } from './types';

  const agents = $derived(swarm.detail?.agents ?? []);
  const allTasks = $derived(Object.values(swarm.tasksByProject).flat());

  // ── Tidy top-down TREE: root (coordinator) at the top, reports branching down
  //    level by level. x = tidy leaf packing (parents centered over children),
  //    y = depth row. Positions are node CENTERS. ─────────────────────────────
  const NODE_W = 178;
  const NODE_H = 64;
  const GAP_X = 206; // horizontal spacing per leaf
  const LEVEL_H = 142; // vertical spacing per depth

  interface Pos {
    x: number;
    y: number;
    depth: number;
  }
  const positions = $derived.by((): Map<string, Pos> => {
    const ags = agents;
    const pos = new Map<string, Pos>();
    if (ags.length === 0) return pos;
    const childrenOf = (id: string | null) =>
      ags.filter((a) => (a.reports_to ?? null) === id).sort((a, b) => a.order_idx - b.order_idx);
    const seen = new Set<string>();
    let cursor = 0;
    const assign = (id: string, depth: number): number => {
      if (seen.has(id)) return cursor * GAP_X; // reports_to cycle guard
      seen.add(id);
      const kids = childrenOf(id);
      let x: number;
      if (kids.length === 0) {
        x = cursor * GAP_X;
        cursor += 1;
      } else {
        const xs = kids.map((k) => assign(k.id, depth + 1));
        x = (xs[0] + xs[xs.length - 1]) / 2; // centre the parent over its span
      }
      pos.set(id, { x, y: depth * LEVEL_H, depth });
      return x;
    };
    for (const r of childrenOf(null)) assign(r.id, 0);
    // Orphans (reports_to points at a missing agent) → lay out as extra roots.
    for (const a of ags) if (!pos.has(a.id)) assign(a.id, 0);
    return pos;
  });

  // Edges: parent (bottom) → child (top), a vertical S-curve.
  const edges = $derived(
    agents
      .filter((a) => a.reports_to && positions.has(a.reports_to) && positions.has(a.id))
      .map((a) => ({ from: a.reports_to as string, to: a.id })),
  );
  function edgePath(p: Pos, c: Pos): string {
    const y1 = p.y + NODE_H / 2;
    const y2 = c.y - NODE_H / 2;
    const my = (y1 + y2) / 2;
    return `M ${p.x} ${y1} C ${p.x} ${my}, ${c.x} ${my}, ${c.x} ${y2}`;
  }

  // ── Per-agent derivations (sessions / activity / counts) ────────────────────
  function agentSessions(agentId: string) {
    const sid = swarm.detail?.id;
    return ws.sessions.filter((s) => {
      const m = (s.meta ?? {}) as Record<string, unknown>;
      if (s.archived || m.swarm_id !== sid || m.agent_id !== agentId) return false;
      return (ws.statusMap[s.id] ?? s.status) !== 'exited';
    });
  }

  function activeRun(agentId: string): SwarmRun | null {
    const rs = swarm.runs.filter(
      (r) =>
        r.agent_id === agentId &&
        (r.status === 'running' || r.status === 'waiting' || r.status === 'queued'),
    );
    if (rs.length === 0) return null;
    return [...rs].sort(
      (a, b) => new Date(b.enqueued_at).getTime() - new Date(a.enqueued_at).getTime(),
    )[0];
  }

  type Activity = 'working' | 'waiting' | 'open' | 'idle';
  function activity(agentId: string): Activity {
    const run = activeRun(agentId);
    const sess = agentSessions(agentId);
    const liveSess = sess.some((s) => {
      const st = ws.statusMap[s.id] ?? s.status;
      return st === 'running' || st === 'working';
    });
    if (run?.status === 'running' || liveSess) return 'working';
    if (run?.status === 'waiting' || run?.status === 'queued') return 'waiting';
    if (sess.length > 0) return 'open';
    return 'idle';
  }

  function statusLine(a: SwarmAgent): string {
    switch (activity(a.id)) {
      case 'working':
        return 'working…';
      case 'waiting':
        return 'queued…';
      case 'open':
        return 'session open';
      default:
        return a.status === 'paused' ? 'paused' : 'idle';
    }
  }

  function elapsed(agentId: string): string | null {
    const run = activeRun(agentId);
    if (!run?.started_at) return null;
    void now(); // reactive 1s tick
    const ms = Date.now() - new Date(run.started_at).getTime();
    if (ms < 0) return null;
    const s = Math.floor(ms / 1000);
    return `${Math.floor(s / 60)}m ${String(s % 60).padStart(2, '0')}s`;
  }

  const completedCount = (agentId: string) =>
    swarm.runs.filter((r) => r.agent_id === agentId && r.status === 'done').length;

  function toAddress(agentId: string): number {
    const tasks = allTasks.filter(
      (t) => t.assignee_agent_id === agentId && t.status !== 'done' && t.status !== 'cancelled',
    ).length;
    const stuck = swarm.runs.filter(
      (r) => r.agent_id === agentId && (r.status === 'waiting' || r.status === 'error'),
    ).length;
    return tasks + stuck;
  }

  // ── Task rail: most-urgent open tasks + search ──────────────────────────────
  const PRIO: Record<string, number> = { urgent: 3, high: 2, medium: 1, low: 0 };
  const SBIAS: Record<string, number> = {
    in_progress: 5, blocked: 4, todo: 3, in_review: 2, backlog: 1, done: 0, cancelled: 0,
  };
  const openTasks = $derived(
    [...allTasks]
      .filter((t) => t.status !== 'done' && t.status !== 'cancelled')
      .sort(
        (a, b) =>
          (PRIO[b.priority] ?? 0) - (PRIO[a.priority] ?? 0) ||
          (SBIAS[b.status] ?? 0) - (SBIAS[a.status] ?? 0) ||
          a.order_idx - b.order_idx,
      ),
  );
  let taskQuery = $state('');
  // 5 most-urgent by default; searching reveals every match.
  const shownTasks = $derived.by(() => {
    const q = taskQuery.trim().toLowerCase();
    if (!q) return openTasks.slice(0, 5);
    return openTasks.filter((t) => t.title.toLowerCase().includes(q));
  });

  // ── Drag a task onto an agent → assign + launch ─────────────────────────────
  let draggingTaskId = $state<string | null>(null);
  let dropAgentId = $state<string | null>(null);

  function onTaskDragStart(e: DragEvent, t: SwarmTask) {
    draggingTaskId = t.id;
    e.dataTransfer?.setData('text/plain', t.id);
    if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move';
  }
  function onTaskDragEnd() {
    draggingTaskId = null;
    dropAgentId = null;
  }
  function onNodeDragOver(e: DragEvent, a: SwarmAgent) {
    if (!draggingTaskId) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
    dropAgentId = a.id;
  }
  function onNodeDragLeave(a: SwarmAgent) {
    if (dropAgentId === a.id) dropAgentId = null;
  }
  async function onNodeDrop(e: DragEvent, a: SwarmAgent) {
    e.preventDefault();
    const tid = e.dataTransfer?.getData('text/plain') ?? draggingTaskId;
    draggingTaskId = null;
    dropAgentId = null;
    if (!tid) return;
    const task = allTasks.find((t) => t.id === tid);
    if (!task) return;
    try {
      if (task.assignee_agent_id !== a.id) {
        await swarm.updateTask(task, { assignee_agent_id: a.id } as Partial<SwarmTask>);
      }
      await swarm.runTask({ ...task, assignee_agent_id: a.id });
      toasts.success(`Launched ${a.name}`, task.title);
    } catch (err) {
      toasts.error('Launch failed', err instanceof Error ? err.message : String(err));
    }
  }

  // ── Click → open the agent's (most recent) session; hover → its sessions ────
  let hoverAgentId = $state<string | null>(null);
  function openAgent(a: SwarmAgent) {
    const sess = agentSessions(a.id);
    if (sess.length > 0) swarm.selectedSessionId = sess[0].id;
    else toasts.info(`${a.name} has no open session`, 'Drag a task here to launch one.');
  }

  // ── Pan + zoom (drag background, wheel to zoom) ──────────────────────────────
  let tx = $state(0);
  let ty = $state(0);
  let scale = $state(1);
  let drag: { sx: number; sy: number; ox: number; oy: number } | null = null;
  let wrapEl = $state<HTMLDivElement | null>(null);
  // While false, the graph auto-fits (initial layout + container resize). Any
  // manual pan/zoom sets it true so we stop fighting the user's gesture.
  let userTouched = $state(false);

  // Fit ALL nodes into view (with padding); never zoom IN past 1:1.
  function fit(): void {
    if (!wrapEl || positions.size === 0) return;
    let minX = Infinity, maxX = -Infinity, minY = Infinity, maxY = -Infinity;
    for (const p of positions.values()) {
      minX = Math.min(minX, p.x - NODE_W / 2);
      maxX = Math.max(maxX, p.x + NODE_W / 2);
      minY = Math.min(minY, p.y - NODE_H / 2);
      maxY = Math.max(maxY, p.y + NODE_H / 2);
    }
    const bw = Math.max(1, maxX - minX);
    const bh = Math.max(1, maxY - minY);
    const w = wrapEl.clientWidth;
    const h = wrapEl.clientHeight;
    if (w === 0 || h === 0) return;
    const pad = 44;
    const s = Math.min(1, Math.max(0.2, Math.min((w - pad * 2) / bw, (h - pad * 2) / bh)));
    scale = s;
    tx = w / 2 - ((minX + maxX) / 2) * s;
    ty = h / 2 - ((minY + maxY) / 2) * s;
  }

  // Auto-fit on first layout, when the node set changes, and on container resize
  // — until the user manually pans/zooms.
  $effect(() => {
    if (!wrapEl) return;
    void positions.size; // re-run when the graph changes
    if (!userTouched) fit();
    const ro = new ResizeObserver(() => {
      if (!userTouched) fit();
    });
    ro.observe(wrapEl);
    return () => ro.disconnect();
  });

  function onWheel(e: WheelEvent) {
    e.preventDefault();
    userTouched = true;
    scale = Math.min(2, Math.max(0.2, scale * (e.deltaY < 0 ? 1.1 : 0.9)));
  }
  function startPan(e: PointerEvent) {
    const t = e.target as HTMLElement;
    if (t.closest('.node') || t.closest('.tooltip')) return;
    userTouched = true;
    drag = { sx: e.clientX, sy: e.clientY, ox: tx, oy: ty };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }
  function onMove(e: PointerEvent) {
    if (!drag) return;
    tx = drag.ox + (e.clientX - drag.sx);
    ty = drag.oy + (e.clientY - drag.sy);
  }
  function endPan() {
    drag = null;
  }
  function zoomBy(f: number) {
    userTouched = true;
    scale = Math.min(2, Math.max(0.2, scale * f));
  }
  function recenter() {
    userTouched = false;
    fit();
  }
</script>

<div class="agent-graph">
  <div class="graph-col">
    <div class="controls">
      <button class="icon-btn" onclick={() => zoomBy(1.15)} aria-label="zoom in"><Icon name="plus" size={14} /></button>
      <button class="icon-btn" onclick={() => zoomBy(0.87)} aria-label="zoom out"><Icon name="minimize" size={14} /></button>
      <button class="icon-btn" onclick={recenter} aria-label="fit to view"><Icon name="maximize" size={14} /></button>
    </div>

    {#if agents.length === 0}
      <EmptyState icon="user" title="No team yet" body="Recruit agents and they'll appear here as a live org graph." />
    {:else}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="canvas"
        bind:this={wrapEl}
        onpointerdown={startPan}
        onpointermove={onMove}
        onpointerup={endPan}
        onwheel={onWheel}
        role="application"
        aria-label="Agent org graph"
      >
        <div class="viewport" style="transform: translate({tx}px,{ty}px) scale({scale});">
          <svg class="edges" aria-hidden="true">
            {#each edges as e (e.from + e.to)}
              {@const p = positions.get(e.from)}
              {@const c = positions.get(e.to)}
              {#if p && c}
                <path d={edgePath(p, c)} class="edge" />
              {/if}
            {/each}
          </svg>

          {#each agents as a (a.id)}
            {@const pos = positions.get(a.id)}
            {#if pos}
              {@const act = activity(a.id)}
              {@const sess = agentSessions(a.id)}
              {@const el = elapsed(a.id)}
              <!-- svelte-ignore a11y_no_static_element_interactions -->
              <div
                class="node act-{act}"
                class:drop={dropAgentId === a.id}
                style="left:{pos.x - NODE_W / 2}px; top:{pos.y - NODE_H / 2}px; width:{NODE_W}px; min-height:{NODE_H}px"
                onmouseenter={() => (hoverAgentId = a.id)}
                onmouseleave={() => (hoverAgentId === a.id ? (hoverAgentId = null) : null)}
                ondragover={(e) => onNodeDragOver(e, a)}
                ondragleave={() => onNodeDragLeave(a)}
                ondrop={(e) => onNodeDrop(e, a)}
              >
                <button class="node-hit" onclick={() => openAgent(a)} title="Open session">
                  <span class="avatar">{a.avatar || a.name.slice(0, 1)}</span>
                  <span class="node-body">
                    <span class="node-top">
                      <span class="node-name">{a.name}</span>
                      <span class="node-state st-{act}" title={statusLine(a)}></span>
                    </span>
                    <span class="node-status dim">{statusLine(a)}</span>
                    <span class="node-foot">
                      <span class="role">{a.specialization || a.title}</span>
                      {#if el}<span class="el"><Icon name="clock" size={9} /> {el}</span>{/if}
                      {#if sess.length > 0}<span class="sess"><Icon name="terminal" size={9} /> {sess.length}</span>{/if}
                    </span>
                  </span>
                </button>

                {#if hoverAgentId === a.id && sess.length > 0}
                  <!-- svelte-ignore a11y_no_static_element_interactions -->
                  <div class="tooltip" onmouseenter={() => (hoverAgentId = a.id)}>
                    <div class="tip-head dim">Sessions ({sess.length})</div>
                    {#each sess as s (s.id)}
                      <button class="tip-row" onclick={() => (swarm.selectedSessionId = s.id)}>
                        <Icon name="terminal" size={11} />
                        <span class="grow ellipsis">{s.title || s.provider}</span>
                        <span class="node-state st-{(ws.statusMap[s.id] ?? s.status) === 'running' ? 'working' : 'idle'}"></span>
                      </button>
                    {/each}
                  </div>
                {/if}
              </div>
            {/if}
          {/each}
        </div>
      </div>
    {/if}
  </div>

  <!-- Side rail: per-member brief + draggable urgent tasks. -->
  <aside class="side">
    <section class="brief">
      <div class="side-head"><Icon name="user" size={12} /> Team brief</div>
      <div class="brief-list">
        {#each agents as a (a.id)}
          {@const addr = toAddress(a.id)}
          <button class="brief-row" onclick={() => openAgent(a)} title="Open session">
            <span class="avatar sm">{a.avatar || a.name.slice(0, 1)}</span>
            <span class="grow ellipsis">{a.name}</span>
            <span class="stat done" title="completed runs"><Icon name="check" size={10} /> {completedCount(a.id)}</span>
            <span class="stat addr" class:has={addr > 0} title="tasks / runs to address"><Icon name="zap" size={10} /> {addr}</span>
          </button>
        {/each}
      </div>
    </section>

    <section class="tasks">
      <div class="side-head"><Icon name="zap" size={12} /> Tasks <span class="dim">— drag onto a member to launch</span></div>
      <div class="search">
        <Icon name="search" size={12} />
        <input class="search-input" placeholder="Search tasks…" bind:value={taskQuery} />
      </div>
      <div class="task-list">
        {#each shownTasks as t (t.id)}
          {@const ag = swarm.agentById(t.assignee_agent_id)}
          <div
            class="task-card prio-{t.priority}"
            class:dragging={draggingTaskId === t.id}
            draggable="true"
            ondragstart={(e) => onTaskDragStart(e, t)}
            ondragend={onTaskDragEnd}
            role="listitem"
          >
            <Icon name="grip" size={12} />
            <span class="task-main">
              <span class="task-title ellipsis2">{t.title}</span>
              <span class="task-meta dim">
                <span class="pchip prio-{t.priority}">{t.priority}</span>
                <span>{t.status.replace('_', ' ')}</span>
                {#if ag}<span>· {ag.name}</span>{/if}
              </span>
            </span>
          </div>
        {/each}
        {#if shownTasks.length === 0}
          <p class="dim empty">{taskQuery ? 'No matching tasks.' : 'No open tasks.'}</p>
        {/if}
      </div>
    </section>
  </aside>
</div>

<style>
  .agent-graph {
    display: flex;
    height: 100%;
    min-height: 0;
  }
  .graph-col {
    position: relative;
    flex: 1;
    min-width: 0;
    overflow: hidden;
  }
  .controls {
    position: absolute;
    inset-inline-end: 10px;
    bottom: 10px;
    z-index: 3;
    display: flex;
    gap: 4px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 3px;
  }
  .canvas {
    position: absolute;
    inset: 0;
    cursor: grab;
    touch-action: none;
    background:
      radial-gradient(circle, color-mix(in srgb, var(--text-dim) 16%, transparent) 1px, transparent 1px);
    background-size: 24px 24px;
  }
  .canvas:active {
    cursor: grabbing;
  }
  .viewport {
    position: absolute;
    transform-origin: 0 0;
  }
  .edges {
    position: absolute;
    overflow: visible;
    width: 1px;
    height: 1px;
    pointer-events: none;
  }
  .edge {
    fill: none;
    stroke: color-mix(in srgb, var(--text-dim) 45%, transparent);
    stroke-width: 1.5;
  }

  .node {
    position: absolute;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    box-shadow: var(--shadow, 0 1px 3px rgba(0, 0, 0, 0.18));
  }
  .node.act-working {
    border-color: color-mix(in srgb, var(--status-working) 55%, var(--border));
  }
  .node.drop {
    border-color: var(--accent);
    outline: 2px dashed color-mix(in srgb, var(--accent) 60%, transparent);
    outline-offset: 1px;
  }
  .node-hit {
    display: flex;
    align-items: flex-start;
    gap: 9px;
    width: 100%;
    padding: 9px 11px;
    border: none;
    background: transparent;
    color: var(--text);
    text-align: start;
    cursor: pointer;
  }
  .avatar {
    width: 26px;
    height: 26px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    font-size: 14px;
    flex: none;
    background: color-mix(in srgb, var(--accent) 22%, transparent);
  }
  .avatar.sm {
    width: 20px;
    height: 20px;
    font-size: 11px;
  }
  .node-body {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
    flex: 1;
  }
  .node-top {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .node-name {
    font-size: 12.5px;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
    min-width: 0;
  }
  .node-state {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex: none;
    background: var(--text-dim);
  }
  .node-state.st-working {
    background: var(--status-working);
  }
  .node-state.st-waiting {
    background: var(--status-idle, #d8a200);
  }
  .node-state.st-open {
    background: var(--accent);
  }
  .node-status {
    font-size: 10.5px;
  }
  .node-foot {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 2px;
    font-size: 9.5px;
    color: var(--text-dim);
    text-transform: uppercase;
    letter-spacing: 0.03em;
  }
  .node-foot .role {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 84px;
  }
  .node-foot .el,
  .node-foot .sess {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    flex: none;
  }

  .tooltip {
    position: absolute;
    top: calc(100% + 6px);
    left: 50%;
    transform: translateX(-50%);
    z-index: 5;
    width: 210px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    box-shadow: var(--shadow, 0 4px 16px rgba(0, 0, 0, 0.28));
    padding: 5px;
  }
  .tip-head {
    font-size: 10px;
    padding: 2px 6px 4px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .tip-row {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    border: none;
    background: transparent;
    color: var(--text);
    border-radius: var(--radius-s);
    padding: 5px 6px;
    cursor: pointer;
    font-size: 11.5px;
    text-align: start;
  }
  .tip-row:hover {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }

  /* ── Side rail ── */
  .side {
    width: 270px;
    flex: none;
    border-inline-start: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .brief {
    display: flex;
    flex-direction: column;
    min-height: 0;
    max-height: 45%;
    border-bottom: 1px solid var(--border);
  }
  .tasks {
    display: flex;
    flex-direction: column;
    min-height: 0;
    flex: 1;
  }
  .side-head {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 10px;
    font-size: 11px;
    font-weight: 600;
    color: var(--text);
    border-bottom: 1px solid var(--border);
  }
  .brief-list {
    overflow-y: auto;
    padding: 4px;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .brief-row {
    display: flex;
    align-items: center;
    gap: 7px;
    width: 100%;
    border: none;
    background: transparent;
    color: var(--text);
    border-radius: var(--radius-s);
    padding: 5px 7px;
    cursor: pointer;
    font-size: 12px;
    text-align: start;
  }
  .brief-row:hover {
    background: color-mix(in srgb, var(--text-dim) 10%, transparent);
  }
  .stat {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    font-size: 10.5px;
    color: var(--text-dim);
    flex: none;
  }
  .stat.done {
    color: var(--status-working);
  }
  .stat.addr.has {
    color: var(--status-exited);
  }
  .search {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 8px;
    padding: 5px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text-dim);
  }
  .search-input {
    flex: 1;
    border: none;
    background: transparent;
    color: var(--text);
    font-size: 12px;
    outline: none;
  }
  .task-list {
    overflow-y: auto;
    padding: 0 8px 8px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .task-card {
    display: flex;
    align-items: flex-start;
    gap: 7px;
    padding: 7px 9px;
    border: 1px solid var(--border);
    border-inline-start-width: 3px;
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text-dim);
    cursor: grab;
  }
  .task-card:hover {
    border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
  }
  .task-card.dragging {
    opacity: 0.45;
  }
  .task-card.prio-urgent {
    border-inline-start-color: var(--status-exited);
  }
  .task-card.prio-high {
    border-inline-start-color: #e6883c;
  }
  .task-card.prio-medium {
    border-inline-start-color: var(--accent);
  }
  .task-card.prio-low {
    border-inline-start-color: var(--text-dim);
  }
  .task-main {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    flex: 1;
  }
  .task-title {
    font-size: 12px;
    color: var(--text);
  }
  .task-meta {
    display: flex;
    align-items: center;
    gap: 5px;
    font-size: 10px;
  }
  .pchip {
    text-transform: uppercase;
    letter-spacing: 0.03em;
    padding: 0 4px;
    border-radius: 999px;
    font-weight: 600;
    background: color-mix(in srgb, var(--text-dim) 16%, transparent);
    color: var(--text-dim);
  }
  .pchip.prio-urgent {
    background: color-mix(in srgb, var(--status-exited) 20%, transparent);
    color: var(--status-exited);
  }
  .pchip.prio-high {
    background: color-mix(in srgb, #e6883c 22%, transparent);
    color: #e6883c;
  }
  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ellipsis2 {
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .empty {
    padding: 8px 4px;
    font-size: 11.5px;
  }

  /* Stack the rail under the graph on narrow viewports. */
  @media (max-width: 720px) {
    .agent-graph {
      flex-direction: column;
    }
    .side {
      width: 100%;
      flex: none;
      border-inline-start: none;
      border-top: 1px solid var(--border);
      max-height: 42%;
    }
    .brief {
      max-height: 38%;
    }
  }
</style>
