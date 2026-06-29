<script lang="ts">
  // Work-in-progress DAG (GitHub-Actions-style): task/run nodes with status
  // badges + dependency edges, laid out left→right by depth. Pan (drag bg),
  // zoom (wheel), click a node with a session → open it. Live via events.
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import RunInspector from './RunInspector.svelte';
  import { swarm } from '../../lib/stores/swarm.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import type { GraphNode, SwarmRun } from './types';

  let inspecting = $state<SwarmRun | null>(null);
  const live = $derived(inspecting ? (swarm.runs.find((r) => r.id === inspecting!.id) ?? inspecting) : null);

  // Hide finished work by default so the graph foregrounds what's active /
  // pending; toggle to reveal the full history.
  let showDone = $state(false);
  const isCompleted = (s: string) => s === 'done' || s === 'cancelled';

  // Latest run for a task node (graph nodes are tasks; ids are `task:<id>`).
  function latestRun(nodeId: string): SwarmRun | null {
    const tid = nodeId.startsWith('task:') ? nodeId.slice(5) : null;
    if (!tid) return null;
    const runs = swarm.runs.filter((r) => r.task_id === tid);
    if (runs.length === 0) return null;
    return [...runs].sort(
      (a, b) => new Date(b.enqueued_at).getTime() - new Date(a.enqueued_at).getTime(),
    )[0];
  }

  const COL_W = 210;
  const ROW_H = 86;
  const NODE_W = 180;
  const NODE_H = 60;

  let tx = $state(40);
  let ty = $state(20);
  let scale = $state(1);
  let drag: { sx: number; sy: number; ox: number; oy: number } | null = null;

  // Phone auto-fit: scale the whole graph to fit the container width and center
  // it, recomputed on resize and when the node set changes — but never after the
  // user has manually panned/zoomed (so we don't fight their gesture).
  const FIT_MIN_SCALE = 0.35;
  let wrapEl = $state<HTMLDivElement | null>(null);
  let userTouchedZoom = $state(false);

  interface Placed extends GraphNode {
    x: number;
    y: number;
  }

  // Layered layout: depth = longest dependency chain to a node.
  const placed = $derived.by((): Placed[] => {
    const g = swarm.graph;
    if (!g) return [];
    const incoming: Record<string, string[]> = {};
    for (const n of g.nodes) incoming[n.id] = [];
    for (const e of g.edges) if (incoming[e.to]) incoming[e.to].push(e.from);
    const depthCache: Record<string, number> = {};
    const depthOf = (id: string, seen = new Set<string>()): number => {
      if (depthCache[id] != null) return depthCache[id];
      if (seen.has(id)) return 0;
      seen.add(id);
      const ins = incoming[id] ?? [];
      const d = ins.length === 0 ? 0 : 1 + Math.max(...ins.map((p) => depthOf(p, seen)));
      depthCache[id] = d;
      return d;
    };
    const byDepth: Record<number, GraphNode[]> = {};
    for (const n of g.nodes) {
      if (!showDone && isCompleted(n.status)) continue; // hide finished by default
      const d = depthOf(n.id);
      (byDepth[d] ??= []).push(n);
    }
    const out: Placed[] = [];
    for (const [dStr, nodes] of Object.entries(byDepth)) {
      const d = Number(dStr);
      nodes.forEach((n, i) => out.push({ ...n, x: d * COL_W, y: i * ROW_H }));
    }
    return out;
  });

  const posById = $derived.by(() => {
    const m: Record<string, Placed> = {};
    for (const p of placed) m[p.id] = p;
    return m;
  });

  const edges = $derived(swarm.graph?.edges ?? []);

  // Bounding box of the laid-out nodes (content coords, pre-transform).
  const bounds = $derived.by(() => {
    if (placed.length === 0) return { w: 0, h: 0 };
    let w = 0;
    let h = 0;
    for (const p of placed) {
      w = Math.max(w, p.x + NODE_W);
      h = Math.max(h, p.y + NODE_H);
    }
    return { w, h };
  });

  function statusIcon(s: string): { icon: string; cls: string } {
    if (s === 'done') return { icon: 'check', cls: 'done' };
    if (s === 'in_progress' || s === 'running' || s === 'waiting') return { icon: 'play', cls: 'run' };
    if (s === 'error' || s === 'blocked') return { icon: 'x', cls: 'err' };
    if (s === 'in_review') return { icon: 'eye', cls: 'run' };
    return { icon: 'dot', cls: 'idle' };
  }

  function onWheel(e: WheelEvent) {
    e.preventDefault();
    userTouchedZoom = true;
    // Pinch (ctrl+wheel on macOS) zooms; a plain two-finger scroll pans.
    if (!e.ctrlKey) {
      tx -= e.deltaX;
      ty -= e.deltaY;
      return;
    }
    const next = Math.min(2, Math.max(0.4, scale * (e.deltaY < 0 ? 1.1 : 0.9)));
    scale = next;
  }
  function startPan(e: PointerEvent) {
    if ((e.target as HTMLElement).closest('.node')) return;
    userTouchedZoom = true;
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
  // Scale the full graph to fit the container width and center it horizontally.
  // Phone-oriented: clamps to a readable minimum and pins to the top so the
  // first column is visible. Clears the "touched" flag — fit is a fresh baseline.
  function autoFit() {
    const cw = wrapEl?.clientWidth ?? 0;
    if (cw === 0 || bounds.w === 0) return;
    const pad = 24;
    const s = Math.min(2, Math.max(FIT_MIN_SCALE, (cw - pad * 2) / bounds.w));
    scale = s;
    tx = Math.max(pad, (cw - bounds.w * s) / 2);
    ty = 20;
    userTouchedZoom = false;
  }
  function fit() {
    if (viewport.isPhone) {
      autoFit();
      return;
    }
    tx = 40;
    ty = 20;
    scale = 1;
    userTouchedZoom = false;
  }

  function edgePath(from: Placed, to: Placed): string {
    const x1 = from.x + NODE_W;
    const y1 = from.y + NODE_H / 2;
    const x2 = to.x;
    const y2 = to.y + NODE_H / 2;
    const mx = (x1 + x2) / 2;
    return `M ${x1} ${y1} C ${mx} ${y1}, ${mx} ${y2}, ${x2} ${y2}`;
  }

  function click(n: Placed) {
    // Prefer the run inspector (richer); fall back to opening the live session.
    const run = latestRun(n.id);
    if (run) inspecting = run;
    else if (n.session_id) swarm.selectedSessionId = n.session_id;
  }

  // Phone only: auto-fit on first layout, when the node set changes, and on
  // container resize — but only while the user hasn't manually panned/zoomed.
  $effect(() => {
    if (!viewport.isPhone || !wrapEl) return;
    // Track the graph signature so this re-runs when nodes appear/change.
    const sig = bounds.w + 'x' + bounds.h;
    void sig;
    if (!userTouchedZoom) autoFit();
    const ro = new ResizeObserver(() => {
      if (!userTouchedZoom) autoFit();
    });
    ro.observe(wrapEl);
    return () => ro.disconnect();
  });
</script>

<div class="graph-wrap" bind:this={wrapEl}>
  <div class="controls">
    <button class="icon-btn" class:active={showDone} onclick={() => (showDone = !showDone)} aria-label="toggle completed" title={showDone ? 'Hide completed tasks' : 'Show completed tasks'}><Icon name={showDone ? 'eye' : 'eyeOff'} size={14} /></button>
    <button class="icon-btn" onclick={() => { userTouchedZoom = true; scale = Math.min(2, scale * 1.1); }} aria-label="zoom in"><Icon name="plus" size={14} /></button>
    <button class="icon-btn" onclick={() => { userTouchedZoom = true; scale = Math.max(0.4, scale * 0.9); }} aria-label="zoom out"><Icon name="minimize" size={14} /></button>
    <button class="icon-btn" onclick={fit} aria-label="fit"><Icon name="maximize" size={14} /></button>
    <button class="icon-btn" onclick={() => swarm.detail && swarm.loadGraph(swarm.detail.id)} aria-label="refresh"><Icon name="refresh" size={14} /></button>
  </div>

  {#if placed.length === 0}
    <EmptyState icon="split" title="No work yet" body="Tasks and their dependencies show up here as a live graph." />
  {:else}
    <div
      class="canvas"
      onpointerdown={startPan}
      onpointermove={onMove}
      onpointerup={endPan}
      onwheel={onWheel}
      role="application"
      aria-label="Run graph"
    >
      <div class="viewport" style="transform: translate({tx}px,{ty}px) scale({scale});">
        <svg class="edges" aria-hidden="true">
          {#each edges as e (e.from + e.to)}
            {#if posById[e.from] && posById[e.to]}
              <path d={edgePath(posById[e.from], posById[e.to])} class="edge {e.kind}" />
            {/if}
          {/each}
        </svg>
        {#each placed as n (n.id)}
          {@const si = statusIcon(n.status)}
          {@const agent = swarm.agentById(n.agent_id)}
          {@const run = latestRun(n.id)}
          <button
            class="node {si.cls}"
            class:clickable={!!n.session_id || !!run}
            style="left:{n.x}px; top:{n.y}px; width:{NODE_W}px; height:{NODE_H}px"
            onclick={() => click(n)}
          >
            <span class="node-badge {si.cls}"><Icon name={si.icon} size={12} /></span>
            <span class="node-main">
              <span class="node-label">{n.label}</span>
              <span class="node-sub dim">{agent ? agent.name : n.status}{run ? ' · inspect' : n.session_id ? ' · open' : ''}</span>
            </span>
          </button>
        {/each}
      </div>
    </div>
  {/if}
</div>

{#if live}
  <RunInspector run={live} onclose={() => (inspecting = null)} />
{/if}

<style>
  .graph-wrap {
    position: relative;
    height: 100%;
    overflow: hidden;
  }
  .controls {
    position: absolute;
    inset-inline-end: 10px;
    bottom: 10px;
    z-index: 2;
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
    /* Pan via pointer events on all input types; stop the browser from
       hijacking the touch gesture (scroll/zoom) so drag-to-pan works on phones. */
    touch-action: none;
    background:
      radial-gradient(circle, color-mix(in srgb, var(--text-dim) 18%, transparent) 1px, transparent 1px);
    background-size: 22px 22px;
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
    stroke: color-mix(in srgb, var(--text-dim) 50%, transparent);
    stroke-width: 1.5;
  }
  .edge.review {
    stroke: color-mix(in srgb, var(--accent) 60%, transparent);
    stroke-dasharray: 4 3;
  }
  .node {
    position: absolute;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    color: var(--text);
    text-align: start;
    cursor: default;
  }
  .node.clickable {
    cursor: pointer;
  }
  .node.clickable:hover {
    border-color: color-mix(in srgb, var(--accent) 55%, var(--border));
  }
  .node-badge {
    width: 20px;
    height: 20px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    flex: none;
    background: color-mix(in srgb, var(--text-dim) 18%, transparent);
    color: var(--text-dim);
  }
  .node-badge.done {
    background: color-mix(in srgb, var(--accent) 22%, transparent);
    color: var(--accent);
  }
  .node-badge.run {
    background: color-mix(in srgb, var(--status-working) 24%, transparent);
    color: var(--status-working);
  }
  .node-badge.err {
    background: color-mix(in srgb, var(--status-exited) 24%, transparent);
    color: var(--status-exited);
  }
  .node-main {
    display: flex;
    flex-direction: column;
    overflow: hidden;
    line-height: 1.2;
  }
  .node-label {
    font-size: 12px;
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .node-sub {
    font-size: 10.5px;
  }
</style>
