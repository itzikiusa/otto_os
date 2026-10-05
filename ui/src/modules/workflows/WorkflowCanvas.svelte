<script lang="ts">
  // A lightweight n8n-style node-graph canvas: pan (drag background), zoom
  // (wheel), drag nodes, drag output→input ports to connect, live run-status
  // coloring. Pure SVG + absolutely-positioned cards inside one transformed
  // viewport, so everything works in graph coordinates.
  //
  // Keyboard: every node is a button (Tab to it; Enter selects; ←↑→↓ nudge it,
  // ⇧ for 10×; Delete removes it), its output port is a sibling button
  // ("Connect from <node>…" opens a menu of targets), and every connection is
  // focusable (Enter selects it, Delete removes it).
  import Icon, { asIcon } from '../../lib/components/Icon.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { runStatus } from '../../lib/status';
  import type { WorkflowGraph, WorkflowNode, NodeTypeSpec, NodeRunState } from '../../lib/api/types';

  interface Props {
    graph: WorkflowGraph;
    types: NodeTypeSpec[];
    runStates?: Record<string, NodeRunState>;
    editable?: boolean;
    selectedId?: string | null;
    selectedEdgeId?: string | null;
    invalidNodes?: string[];
    invalidEdges?: string[];
    onchange?: (graph: WorkflowGraph) => void;
    onselect?: (id: string | null) => void;
    onedgeselect?: (id: string | null) => void;
  }
  let {
    graph = $bindable(),
    types,
    runStates = {},
    editable = true,
    selectedId = null,
    selectedEdgeId = null,
    invalidNodes = [],
    invalidEdges = [],
    onchange,
    onselect,
    onedgeselect,
  }: Props = $props();

  const NODE_W = 210;
  const NODE_H = 62;
  const HEAD_H = 60; // header row height (icon + title) inside any node
  const STEP_H = 22; // one inner loop step row
  const LOOP_FOOT = 24; // the "until … · max …" footer row

  interface LoopStep {
    name: string;
    kind: string;
  }
  function loopSteps(n: WorkflowNode): LoopStep[] {
    const p = n.params as { steps?: { name?: string; kind?: string }[] } | null;
    const steps = Array.isArray(p?.steps) ? p!.steps : [];
    return steps.map((s, i) => ({ name: s?.name || `step ${i + 1}`, kind: s?.kind || '?' }));
  }
  function loopUntil(n: WorkflowNode): string {
    return (n.params as { until?: string } | null)?.until ?? '';
  }
  function loopMax(n: WorkflowNode): number | undefined {
    return (n.params as { max_iterations?: number } | null)?.max_iterations;
  }
  /** The actual card height — loop nodes grow to show their inner steps. */
  function nodeHeight(n: WorkflowNode): number {
    if (n.kind === 'loop') {
      const c = loopSteps(n).length;
      return HEAD_H + c * STEP_H + (loopUntil(n) ? LOOP_FOOT : 0) + 8;
    }
    return NODE_H;
  }

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
    return spec(kind)?.color ?? 'var(--text-dim)';
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
    // Only a trackpad PINCH (or ctrl+wheel) zooms — macOS delivers pinch as a
    // wheel event with ctrlKey set. A plain two-finger scroll PANS instead, so
    // scrolling no longer zooms the graph.
    if (!e.ctrlKey) {
      tx -= e.deltaX;
      ty -= e.deltaY;
      return;
    }
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
    onedgeselect?.(null);
    drag = { mode: 'pan', sx: e.clientX, sy: e.clientY, ox: tx, oy: ty };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }

  function startNode(e: PointerEvent, n: WorkflowNode): void {
    e.stopPropagation();
    if (e.button !== 0) return;
    onselect?.(n.id);
    onedgeselect?.(null);
    if (!editable) return;
    drag = { mode: 'node', id: n.id, sx: e.clientX, sy: e.clientY, nx: n.x, ny: n.y };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }

  // A port press that turned into a drag must not also open the "Connect to…"
  // menu from the click that follows the pointerup.
  let connectFrom: { x: number; y: number } | null = null;
  let connectDragged = false;
  function startConnect(e: PointerEvent, n: WorkflowNode): void {
    e.stopPropagation();
    if (!editable || e.button !== 0) return;
    const g = toGraph(e.clientX, e.clientY);
    drag = { mode: 'connect', from: n.id, mx: g.x, my: g.y };
    connectFrom = { x: e.clientX, y: e.clientY };
    connectDragged = false;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }

  function nodeLabel(n: WorkflowNode): string {
    return n.name || spec(n.kind)?.label || n.kind;
  }

  /** The output port's menu: every other node that takes an input, with the
   *  ones already connected shown checked (and not offered twice). */
  function connectMenu(e: MouseEvent, n: WorkflowNode): void {
    e.stopPropagation();
    if (connectDragged) {
      connectDragged = false;
      return;
    }
    const targets = graph.nodes.filter((t) => t.id !== n.id && (spec(t.kind)?.inputs ?? 1) > 0);
    const items: MenuItem[] = targets.length
      ? targets.map((t) => {
          const linked = graph.edges.some((x) => x.source === n.id && x.target === t.id);
          return {
            label: nodeLabel(t),
            checked: linked,
            disabled: linked,
            title: linked ? 'Already connected' : `Connect “${nodeLabel(n)}” to “${nodeLabel(t)}”`,
            action: () => connect(n.id, t.id),
          };
        })
      : [{ label: 'No other step takes an input', disabled: true }];
    ctxMenu.show(e, items);
  }

  const NUDGE = 8;
  /** Keys on a focused node: arrows nudge it (⇧ = 10×), Delete removes it. */
  function onNodeKey(e: KeyboardEvent, n: WorkflowNode): void {
    if (!editable || e.metaKey || e.ctrlKey || e.altKey) return;
    if (e.key === 'Delete' || e.key === 'Backspace') {
      e.preventDefault();
      graph.nodes = graph.nodes.filter((x) => x.id !== n.id);
      graph.edges = graph.edges.filter((x) => x.source !== n.id && x.target !== n.id);
      onselect?.(null);
      onchange?.(graph);
      return;
    }
    const step = e.shiftKey ? NUDGE * 10 : NUDGE;
    const d: Record<string, [number, number]> = {
      ArrowLeft: [-step, 0],
      ArrowRight: [step, 0],
      ArrowUp: [0, -step],
      ArrowDown: [0, step],
    };
    const v = d[e.key];
    if (!v) return;
    e.preventDefault();
    n.x = Math.max(0, n.x + v[0]);
    n.y = Math.max(0, n.y + v[1]);
    graph = graph;
    if (selectedId !== n.id) {
      onselect?.(n.id);
      onedgeselect?.(null);
    }
    onchange?.(graph);
  }

  /** Keys on a focused connection: Enter / Space selects it, Delete removes it. */
  function onEdgeKey(e: KeyboardEvent, id: string): void {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      selectEdge(id);
    } else if (editable && (e.key === 'Delete' || e.key === 'Backspace')) {
      e.preventDefault();
      graph.edges = graph.edges.filter((x) => x.id !== id);
      onedgeselect?.(null);
      onchange?.(graph);
    }
  }
  const hintId = $props.id();

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
      if (connectFrom && Math.hypot(e.clientX - connectFrom.x, e.clientY - connectFrom.y) > 4) connectDragged = true;
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
          g.y >= n.y + nodeHeight(n) / 2 - 18 &&
          g.y <= n.y + nodeHeight(n) / 2 + 18,
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

  function selectEdge(id: string): void {
    onselect?.(null);
    onedgeselect?.(id);
  }

  // Bezier path between an output port and an input port (graph coords).
  function edgePath(s: WorkflowNode, t: WorkflowNode): string {
    const x1 = s.x + NODE_W;
    const y1 = s.y + nodeHeight(s) / 2;
    const x2 = t.x;
    const y2 = t.y + nodeHeight(t) / 2;
    const dx = Math.max(40, Math.abs(x2 - x1) * 0.5);
    return `M ${x1} ${y1} C ${x1 + dx} ${y1}, ${x2 - dx} ${y2}, ${x2} ${y2}`;
  }

  /** Midpoint of an edge (for the condition badge). */
  function edgeMid(s: WorkflowNode, t: WorkflowNode): { x: number; y: number } {
    return {
      x: (s.x + NODE_W + t.x) / 2,
      y: (s.y + nodeHeight(s) / 2 + t.y + nodeHeight(t) / 2) / 2,
    };
  }
  function condLabel(c: string): string {
    return c.length > 18 ? `${c.slice(0, 17)}…` : c;
  }

  function tempPath(): string {
    if (drag?.mode !== 'connect') return '';
    const d = drag;
    const s = graph.nodes.find((n) => n.id === d.from);
    if (!s) return '';
    const x1 = s.x + NODE_W;
    const y1 = s.y + nodeHeight(s) / 2;
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
          <path class="edge" class:invalid={invalidEdges.includes(e.id)} class:selected={selectedEdgeId === e.id} class:conditional={!!e.condition} d={edgePath(s, t)} />
          <path
            class="edge-hit"
            d={edgePath(s, t)}
            onpointerdown={(ev) => {
              ev.stopPropagation();
              selectEdge(e.id);
            }}
            onkeydown={(ev) => onEdgeKey(ev, e.id)}
            role="button"
            tabindex="0"
            aria-pressed={selectedEdgeId === e.id}
            aria-label={`Connection from ${nodeLabel(s)} to ${nodeLabel(t)}${e.condition ? `, when ${e.condition}` : ''}`}
            aria-describedby={editable ? `${hintId}-edge` : undefined}
          />
          {#if e.condition}
            {@const m = edgeMid(s, t)}
            <g class="edge-badge" pointer-events="none">
              <title>{e.condition}</title>
              <rect x={m.x - 30} y={m.y - 9} width="60" height="18" rx="9" />
              <text x={m.x} y={m.y + 3} text-anchor="middle">ƒ {condLabel(e.condition)}</text>
            </g>
          {/if}
        {/if}
      {/each}
      {#if drag?.mode === 'connect'}
        <path class="edge temp" d={tempPath()} />
      {/if}
    </svg>

    {#each graph.nodes as n (n.id)}
      {@const st = statusOf(n.id)}
      <!-- The card is a plain box: the node itself is the button inside it
           (select / move / delete), the output port a sibling button — never
           one control nested in another. -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="node"
        class:invalid={invalidNodes.includes(n.id)}
        class:selected={selectedId === n.id}
        class:loop={n.kind === 'loop'}
        data-status={st}
        style="left:{n.x}px; top:{n.y}px; width:{NODE_W}px; height:{nodeHeight(n)}px; --accent:{color(n.kind)};"
        onpointerdown={(e) => startNode(e, n)}
      >
        <button
          type="button"
          class="node-main"
          aria-label={`Edit ${nodeLabel(n)}`}
          aria-pressed={selectedId === n.id}
          aria-describedby={editable ? `${hintId}-node` : undefined}
          onclick={() => { onselect?.(n.id); onedgeselect?.(null); }}
          onkeydown={(e) => onNodeKey(e, n)}
        >
          <span class="stripe"></span>
          <span class="head">
            <span class="ic"><Icon name={asIcon(spec(n.kind)?.icon, 'box')} size={14} /></span>
            <span class="body">
              <span class="title" title={nodeLabel(n)}>{nodeLabel(n)}</span>
              <span class="kind">{spec(n.kind)?.label ?? n.kind}</span>
            </span>
            {#if st}<span class="dot {st}" role="img" title={runStatus(st).label} aria-label={runStatus(st).label}></span>{/if}
          </span>

          {#if n.kind === 'loop'}
            <!-- The exact loop, expanded: its inner steps + stop condition. -->
            <span class="steps">
              {#each loopSteps(n) as s, i}
                <span class="step">
                  <span class="si">{i + 1}</span>
                  <span class="sn" title={s.name}>{s.name}</span>
                  <span class="sk">{spec(s.kind)?.label ?? s.kind}</span>
                </span>
              {/each}
              {#if loopUntil(n)}
                <span class="until" title={loopUntil(n)}>
                  <Icon name="refresh" size={12} /> until <code>{condLabel(loopUntil(n))}</code>{#if loopMax(n)}{' · max '}{loopMax(n)}{/if}
                </span>
              {/if}
            </span>
          {/if}
        </button>

        {#if (spec(n.kind)?.inputs ?? 1) > 0}
          <span class="port in" aria-hidden="true"></span>
        {/if}
        {#if editable && (spec(n.kind)?.outputs ?? 1) > 0}
          <button
            type="button"
            class="port out"
            aria-label={`Connect from ${nodeLabel(n)}`}
            title={`Connect from ${nodeLabel(n)} — drag to a step’s input, or click to choose…`}
            aria-haspopup="menu"
            onpointerdown={(e) => startConnect(e, n)}
            onclick={(e) => connectMenu(e, n)}
          ></button>
        {:else if (spec(n.kind)?.outputs ?? 1) > 0}
          <span class="port out" aria-hidden="true"></span>
        {/if}
      </div>
    {/each}
  </div>

  <div class="hud">
    <button class="zbtn" onclick={() => (scale = Math.min(2, scale * 1.15))} title="Zoom in" aria-label="Zoom in">+</button>
    <button class="zbtn" onclick={() => (scale = Math.max(0.3, scale * 0.87))} title="Zoom out" aria-label="Zoom out">−</button>
    <button class="zbtn" onclick={fit} title="Reset view" aria-label="Reset view"><Icon name="maximize" size={12} /></button>
    <span class="zpct">{Math.round(scale * 100)}%</span>
  </div>
  <span class="sr-only" id="{hintId}-node">Arrow keys move the step, Shift moves it further, Delete removes it.</span>
  <span class="sr-only" id="{hintId}-edge">Enter selects the connection, Delete removes it.</span>
</div>

<style>
  .node.invalid { outline: 2px solid var(--status-exited); }
  .edge.invalid { stroke: var(--status-exited); stroke-width: 3; }
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
  .edge.conditional {
    stroke: color-mix(in srgb, var(--accent) 70%, var(--text-dim));
    stroke-dasharray: 6 4;
  }
  .edge.selected {
    stroke: var(--accent);
    stroke-width: 3;
  }
  .edge-badge rect {
    fill: var(--surface);
    stroke: color-mix(in srgb, var(--accent) 55%, var(--border));
    stroke-width: 1;
  }
  .edge-badge text {
    fill: var(--text-dim);
    font-size: var(--fs-xs);
    font-family: var(--font-mono);
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
  /* A focused connection: a soft accent band along the curve (an SVG path
     has no outline box). */
  .edge-hit:focus {
    outline: none;
  }
  .edge-hit:focus-visible {
    stroke: color-mix(in srgb, var(--accent-solid) 45%, transparent);
    stroke-width: 8;
  }
  .node {
    position: absolute;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    box-shadow: var(--shadow);
    cursor: grab;
    user-select: none;
    transition: border-color var(--dur-fast) ease-out;
  }
  /* The node's own button fills the card (the ports sit outside it, so the
     card itself doesn't clip — this does, for the stripe + loop rows). */
  .node-main {
    all: unset;
    box-sizing: border-box;
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: stretch;
    width: 100%;
    height: 100%;
    border-radius: inherit;
    overflow: hidden;
    color: var(--text);
    font: inherit;
    text-align: start;
    cursor: inherit;
  }
  .node-main:focus-visible {
    outline: 2px solid var(--accent-solid);
    outline-offset: 2px;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 60px;
    padding-block: 0; padding-inline: 14px 12px;
    flex-shrink: 0;
  }
  .node:not(.loop) .head {
    height: 100%;
  }
  .node.loop .head {
    border-bottom: 1px solid var(--border);
  }
  .steps {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 2px;
    padding-block: 5px 6px; padding-inline: 14px 10px;
    min-height: 0;
  }
  .step {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-xs);
    line-height: 1.4;
    white-space: nowrap;
  }
  .si {
    display: grid;
    place-items: center;
    width: 15px;
    height: 15px;
    border-radius: var(--radius-s);
    font-size: var(--fs-xs);
    font-weight: 600;
    background: color-mix(in srgb, var(--accent) 18%, transparent);
    color: var(--accent-text);
    flex-shrink: 0;
  }
  .sn {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--text);
  }
  .sk {
    font-size: var(--fs-xs);
    letter-spacing: .06em;
    text-transform: uppercase;
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .until {
    display: block;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    margin-top: 2px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .until :global(svg) {
    vertical-align: -2px;
  }
  .until code {
    color: var(--accent-text);
    font-size: var(--fs-xs);
  }
  .node:hover {
    border-color: color-mix(in srgb, var(--accent) 50%, var(--border));
  }
  .node.selected {
    border-color: var(--accent);
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent) 35%, transparent), var(--shadow);
  }
  /* Run status uses the shared run vocabulary (lib/status.ts): running is
     info-blue and pulses, succeeded is green — they used to share one green,
     so a finished step looked like a running one. */
  .node[data-status='running'] {
    border-color: var(--info);
  }
  .node[data-status='error'] {
    border-color: var(--status-exited);
  }
  .stripe {
    position: absolute;
    left: 0;
    top: 8px;
    bottom: 8px;
    width: 4px;
    border-radius: var(--radius-s);
    background: var(--accent);
  }
  .ic {
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    border-radius: var(--radius-s);
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    color: var(--accent-text);
    flex-shrink: 0;
  }
  .body {
    display: flex;
    flex-direction: column;
    min-width: 0;
    gap: 1px;
  }
  .title {
    font-size: var(--fs-m);
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .kind {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    text-transform: uppercase;
    letter-spacing: .06em;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    margin-inline-start: auto;
    flex-shrink: 0;
  }
  .dot.running {
    background: var(--info);
    animation: otto-pulse 1.6s ease-in-out infinite;
  }
  .dot.success {
    background: var(--status-working);
  }
  .dot.error {
    background: var(--status-exited);
  }
  .dot.skipped {
    background: var(--text-dim);
  }
  .dot.pending {
    background: color-mix(in srgb, var(--text-dim) 50%, transparent);
  }
  
  @media (prefers-reduced-motion: reduce) {
    .dot.running {
      animation: none;
    }
  }
  .port {
    position: absolute;
    box-sizing: content-box;
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
    padding: 0;
    cursor: crosshair;
  }
  /* The visible port is 12 px; the press target around it is 28 px. */
  button.port.out::after {
    content: '';
    position: absolute;
    inset: -8px;
    border-radius: 50%;
  }
  .port.out:hover {
    background: var(--accent);
  }
  button.port.out:focus-visible {
    outline: 2px solid var(--accent-solid);
    outline-offset: 2px;
    background: var(--accent);
  }
  .hud {
    position: absolute;
    inset-inline-end: 12px;
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
    font-size: var(--fs-l);
    border-radius: var(--radius-s);
    cursor: pointer;
  }
  .zbtn:hover {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }
  .zpct {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    padding: 0 4px;
    min-width: 34px;
    text-align: center;
  }
</style>
