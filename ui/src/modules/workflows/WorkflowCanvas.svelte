<script lang="ts">
  // A lightweight n8n-style node-graph canvas: pan (drag background), zoom
  // (wheel), drag nodes, drag output→input ports to connect, live run-status
  // coloring. Pure SVG + absolutely-positioned cards inside one transformed
  // viewport, so everything works in graph coordinates.
  import Icon from '../../lib/components/Icon.svelte';
  import type { WorkflowGraph, WorkflowNode, NodeTypeSpec, NodeRunState } from '../../lib/api/types';

  interface Props {
    graph: WorkflowGraph;
    types: NodeTypeSpec[];
    runStates?: Record<string, NodeRunState>;
    editable?: boolean;
    selectedId?: string | null;
    onchange?: (graph: WorkflowGraph) => void;
    onselect?: (id: string | null) => void;
  }
  let {
    graph = $bindable(),
    types,
    runStates = {},
    editable = true,
    selectedId = null,
    onchange,
    onselect,
  }: Props = $props();

  const NODE_W = 190;
  const NODE_H = 62;

  let scale = $state(1);
  let tx = $state(40);
  let ty = $state(20);
  let surface = $state<HTMLDivElement | null>(null);

  const typeMap = $derived(new Map(types.map((t) => [t.kind, t])));

  // --- drag state -----------------------------------------------------------
  type Drag =
    | { mode: 'pan'; sx: number; sy: number; ox: number; oy: number }
    | { mode: 'node'; id: string; sx: number; sy: number; nx: number; ny: number }
    | { mode: 'connect'; from: string; mx: number; my: number };
  let drag = $state<Drag | null>(null);

  function spec(kind: string): NodeTypeSpec | undefined {
    return typeMap.get(kind);
  }
  function color(kind: string): string {
    return spec(kind)?.color ?? '#7a8190';
  }

  // Graph coords from a client (screen) point.
  function toGraph(clientX: number, clientY: number): { x: number; y: number } {
    const r = surface?.getBoundingClientRect();
    const px = clientX - (r?.left ?? 0);
    const py = clientY - (r?.top ?? 0);
    return { x: (px - tx) / scale, y: (py - ty) / scale };
  }

  function onWheel(e: WheelEvent): void {
    e.preventDefault();
    const r = surface?.getBoundingClientRect();
    const px = e.clientX - (r?.left ?? 0);
    const py = e.clientY - (r?.top ?? 0);
    const next = Math.min(2, Math.max(0.3, scale * (e.deltaY < 0 ? 1.1 : 0.9)));
    // Zoom around the cursor.
    tx = px - ((px - tx) * next) / scale;
    ty = py - ((py - ty) * next) / scale;
    scale = next;
  }

  function startPan(e: PointerEvent): void {
    if (e.button !== 0) return;
    onselect?.(null);
    drag = { mode: 'pan', sx: e.clientX, sy: e.clientY, ox: tx, oy: ty };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }

  function startNode(e: PointerEvent, n: WorkflowNode): void {
    e.stopPropagation();
    if (e.button !== 0) return;
    onselect?.(n.id);
    if (!editable) return;
    drag = { mode: 'node', id: n.id, sx: e.clientX, sy: e.clientY, nx: n.x, ny: n.y };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }

  function startConnect(e: PointerEvent, n: WorkflowNode): void {
    e.stopPropagation();
    if (!editable || e.button !== 0) return;
    const g = toGraph(e.clientX, e.clientY);
    drag = { mode: 'connect', from: n.id, mx: g.x, my: g.y };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }

  function onMove(e: PointerEvent): void {
    if (!drag) return;
    if (drag.mode === 'pan') {
      tx = drag.ox + (e.clientX - drag.sx);
      ty = drag.oy + (e.clientY - drag.sy);
    } else if (drag.mode === 'node') {
      const d = drag;
      const n = graph.nodes.find((x) => x.id === d.id);
      if (n) {
        n.x = d.nx + (e.clientX - d.sx) / scale;
        n.y = d.ny + (e.clientY - d.sy) / scale;
        graph = graph;
      }
    } else if (drag.mode === 'connect') {
      const g = toGraph(e.clientX, e.clientY);
      drag.mx = g.x;
      drag.my = g.y;
    }
  }

  function endDrag(e: PointerEvent): void {
    if (drag?.mode === 'node') onchange?.(graph);
    if (drag?.mode === 'connect') {
      // Hit-test for an input port under the pointer.
      const d = drag;
      const g = toGraph(e.clientX, e.clientY);
      const target = graph.nodes.find(
        (n) =>
          n.id !== d.from &&
          g.x >= n.x - 14 &&
          g.x <= n.x + 30 &&
          g.y >= n.y + NODE_H / 2 - 18 &&
          g.y <= n.y + NODE_H / 2 + 18,
      );
      if (target) connect(d.from, target.id);
    }
    drag = null;
  }

  function connect(source: string, target: string): void {
    if (graph.edges.some((e) => e.source === source && e.target === target)) return;
    graph.edges = [
      ...graph.edges,
      { id: `e-${source}-${target}-${graph.edges.length}`, source, target },
    ];
    onchange?.(graph);
  }

  function removeEdge(id: string): void {
    if (!editable) return;
    graph.edges = graph.edges.filter((e) => e.id !== id);
    onchange?.(graph);
  }

  // Bezier path between an output port and an input port (graph coords).
  function edgePath(s: WorkflowNode, t: WorkflowNode): string {
    const x1 = s.x + NODE_W;
    const y1 = s.y + NODE_H / 2;
    const x2 = t.x;
    const y2 = t.y + NODE_H / 2;
    const dx = Math.max(40, Math.abs(x2 - x1) * 0.5);
    return `M ${x1} ${y1} C ${x1 + dx} ${y1}, ${x2 - dx} ${y2}, ${x2} ${y2}`;
  }

  function tempPath(): string {
    if (drag?.mode !== 'connect') return '';
    const d = drag;
    const s = graph.nodes.find((n) => n.id === d.from);
    if (!s) return '';
    const x1 = s.x + NODE_W;
    const y1 = s.y + NODE_H / 2;
    const dx = Math.max(40, Math.abs(d.mx - x1) * 0.5);
    return `M ${x1} ${y1} C ${x1 + dx} ${y1}, ${d.mx - dx} ${d.my}, ${d.mx} ${d.my}`;
  }

  function nodeOf(id: string): WorkflowNode | undefined {
    return graph.nodes.find((n) => n.id === id);
  }

  function statusOf(id: string): string {
    return runStates[id]?.status ?? '';
  }

  function fit(): void {
    scale = 1;
    tx = 40;
    ty = 20;
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="canvas"
  bind:this={surface}
  onpointerdown={startPan}
  onpointermove={onMove}
  onpointerup={endDrag}
  onwheel={onWheel}
>
  <div class="dots"></div>
  <div class="viewport" style="transform: translate({tx}px,{ty}px) scale({scale});">
    <svg class="edges" width="6000" height="4000">
      {#each graph.edges as e (e.id)}
        {@const s = nodeOf(e.source)}
        {@const t = nodeOf(e.target)}
        {#if s && t}
          <path class="edge" d={edgePath(s, t)} />
          <path
            class="edge-hit"
            d={edgePath(s, t)}
            onpointerdown={(ev) => {
              ev.stopPropagation();
              removeEdge(e.id);
            }}
            role="button"
            tabindex="-1"
          />
        {/if}
      {/each}
      {#if drag?.mode === 'connect'}
        <path class="edge temp" d={tempPath()} />
      {/if}
    </svg>

    {#each graph.nodes as n (n.id)}
      {@const st = statusOf(n.id)}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="node"
        class:selected={selectedId === n.id}
        data-status={st}
        style="left:{n.x}px; top:{n.y}px; width:{NODE_W}px; --accent:{color(n.kind)};"
        onpointerdown={(e) => startNode(e, n)}
      >
        <span class="stripe"></span>
        <span class="ic"><Icon name={spec(n.kind)?.icon ?? 'box'} size={14} /></span>
        <span class="body">
          <span class="title">{n.name || spec(n.kind)?.label || n.kind}</span>
          <span class="kind">{spec(n.kind)?.label ?? n.kind}</span>
        </span>
        {#if st}<span class="dot {st}" title={st}></span>{/if}

        {#if (spec(n.kind)?.inputs ?? 1) > 0}
          <span class="port in" title="input"></span>
        {/if}
        {#if (spec(n.kind)?.outputs ?? 1) > 0}
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <span
            class="port out"
            title="drag to connect"
            onpointerdown={(e) => startConnect(e, n)}
          ></span>
        {/if}
      </div>
    {/each}
  </div>

  <div class="hud">
    <button class="zbtn" onclick={() => (scale = Math.min(2, scale * 1.15))} title="Zoom in">+</button>
    <button class="zbtn" onclick={() => (scale = Math.max(0.3, scale * 0.87))} title="Zoom out">−</button>
    <button class="zbtn" onclick={fit} title="Reset view"><Icon name="maximize" size={12} /></button>
    <span class="zpct">{Math.round(scale * 100)}%</span>
  </div>
</div>

<style>
  .canvas {
    position: relative;
    width: 100%;
    height: 100%;
    overflow: hidden;
    background: var(--surface-2);
    cursor: grab;
    touch-action: none;
  }
  .canvas:active {
    cursor: grabbing;
  }
  .dots {
    position: absolute;
    inset: 0;
    background-image: radial-gradient(
      color-mix(in srgb, var(--text-dim) 30%, transparent) 1px,
      transparent 1px
    );
    background-size: 22px 22px;
    pointer-events: none;
    opacity: 0.5;
  }
  .viewport {
    position: absolute;
    top: 0;
    left: 0;
    transform-origin: 0 0;
  }
  .edges {
    position: absolute;
    top: 0;
    left: 0;
    overflow: visible;
    pointer-events: none;
  }
  .edge {
    fill: none;
    stroke: color-mix(in srgb, var(--text-dim) 60%, transparent);
    stroke-width: 2;
  }
  .edge.temp {
    stroke: var(--accent);
    stroke-dasharray: 5 4;
  }
  .edge-hit {
    fill: none;
    stroke: transparent;
    stroke-width: 14;
    pointer-events: stroke;
    cursor: pointer;
  }
  .edge-hit:hover + .edge,
  .edge-hit:hover {
    stroke: var(--status-exited);
  }
  .node {
    position: absolute;
    display: flex;
    align-items: center;
    gap: 8px;
    height: 62px;
    padding: 0 12px 0 14px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    box-shadow: var(--shadow);
    cursor: grab;
    user-select: none;
    transition: border-color 120ms ease-out;
  }
  .node:hover {
    border-color: color-mix(in srgb, var(--accent) 50%, var(--border));
  }
  .node.selected {
    border-color: var(--accent);
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent) 35%, transparent), var(--shadow);
  }
  .node[data-status='running'] {
    border-color: var(--status-working, #28c840);
  }
  .node[data-status='error'] {
    border-color: var(--status-exited, #ff5f57);
  }
  .stripe {
    position: absolute;
    left: 0;
    top: 8px;
    bottom: 8px;
    width: 4px;
    border-radius: 4px;
    background: var(--accent);
  }
  .ic {
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    border-radius: 7px;
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    color: var(--accent);
    flex-shrink: 0;
  }
  .body {
    display: flex;
    flex-direction: column;
    min-width: 0;
    gap: 1px;
  }
  .title {
    font-size: 12.5px;
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .kind {
    font-size: 10px;
    color: var(--text-dim);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    margin-left: auto;
    flex-shrink: 0;
  }
  .dot.running {
    background: var(--status-working, #28c840);
    animation: pulse 1s infinite;
  }
  .dot.success {
    background: var(--status-working, #28c840);
  }
  .dot.error {
    background: var(--status-exited, #ff5f57);
  }
  .dot.skipped {
    background: var(--text-dim);
  }
  .dot.pending {
    background: color-mix(in srgb, var(--text-dim) 50%, transparent);
  }
  @keyframes pulse {
    50% {
      opacity: 0.35;
    }
  }
  .port {
    position: absolute;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: var(--surface);
    border: 2px solid var(--accent);
    top: calc(50% - 6px);
  }
  .port.in {
    left: -7px;
  }
  .port.out {
    right: -7px;
    cursor: crosshair;
  }
  .port.out:hover {
    background: var(--accent);
  }
  .hud {
    position: absolute;
    right: 12px;
    bottom: 12px;
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 4px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    box-shadow: var(--shadow);
  }
  .zbtn {
    display: grid;
    place-items: center;
    width: 26px;
    height: 24px;
    border: none;
    background: transparent;
    color: var(--text);
    font-size: 15px;
    border-radius: var(--radius-s);
    cursor: pointer;
  }
  .zbtn:hover {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }
  .zpct {
    font-size: 10.5px;
    color: var(--text-dim);
    padding: 0 4px;
    min-width: 34px;
    text-align: center;
  }
</style>
