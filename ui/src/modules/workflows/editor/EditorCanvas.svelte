<script lang="ts">
  // The editor's canvas (ux.md §9.1–9.2): the graph drawn top to bottom by
  // autoLayout on every render — no dragging steps around, no "Tidy" — fitted
  // on open, calm 208 × 56 cards (icon tile, name, one-line summary, an
  // outward mark), vertical edges with yes / no pills, a bottom "Not
  // connected" row, and a zoom pill. Connect by dragging from a card's bottom
  // port onto another card, or with the port's "Connect to…" menu. No run state.
  import { onMount, tick } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
  import type { WorkflowGraph, WorkflowNode, WorkflowTrigger } from '../../../lib/api/types';
  import { nodeSideEffect } from '../sideEffects';
  import { autoLayout, openZoom, CARD_H, CARD_W } from './autoLayout';
  import { nodeIcon, nodeSummary } from './nodeSummary';

  interface Props {
    graph: WorkflowGraph;
    selectedId: string | null;
    selectedEdgeId: string | null;
    /** Validation messages by node id (the card's "!" says it). */
    invalid?: Record<string, string>;
    triggers?: readonly WorkflowTrigger[];
    defaultProvider: string;
    onselect: (id: string | null) => void;
    onedgeselect: (id: string | null) => void;
    onconnect: (source: string, target: string) => void;
    ondeletenode: (id: string) => void;
    ondeleteedge: (id: string) => void;
  }
  let { graph, selectedId, selectedEdgeId, invalid = {}, triggers = [], defaultProvider, onselect, onedgeselect, onconnect, ondeletenode, ondeleteedge }: Props = $props();

  const PAD = 32;
  const layout = $derived(autoLayout(graph));
  const byId = $derived(new Map(graph.nodes.map((n) => [n.id, n])));
  const ctx = $derived({ nodes: graph.nodes, defaultProvider, triggerKinds: triggers.map((t) => t.kind) });

  let surface = $state<HTMLDivElement | null>(null);
  let zoom = $state(1);
  let tx = $state(PAD);
  let ty = $state(PAD);

  /** Fit: the whole graph when it fits (≤ 100 %), else 80 % from the top. */
  function fit(): void {
    if (!surface) return;
    const w = surface.clientWidth;
    const h = surface.clientHeight;
    zoom = openZoom(layout, w, h, PAD);
    tx = Math.round((w - layout.width * zoom) / 2);
    ty = Math.round(Math.max(PAD, (h - layout.height * zoom) / 2));
  }
  onMount(() => void tick().then(fit));
  function zoomBy(f: number): void {
    if (!surface) return;
    const next = Math.min(2, Math.max(0.3, zoom * f));
    const cx = surface.clientWidth / 2;
    const cy = surface.clientHeight / 2;
    tx = cx - ((cx - tx) * next) / zoom;
    ty = cy - ((cy - ty) * next) / zoom;
    zoom = next;
  }

  function onWheel(e: WheelEvent): void {
    e.preventDefault();
    // A pinch (ctrl + wheel) zooms around the pointer; a two-finger scroll pans.
    if (!e.ctrlKey) {
      tx -= e.deltaX;
      ty -= e.deltaY;
      return;
    }
    const r = surface!.getBoundingClientRect();
    const px = e.clientX - r.left;
    const py = e.clientY - r.top;
    const next = Math.min(2, Math.max(0.3, zoom * (e.deltaY < 0 ? 1.1 : 0.9)));
    tx = px - ((px - tx) * next) / zoom;
    ty = py - ((py - ty) * next) / zoom;
    zoom = next;
  }

  type Drag = { mode: 'pan'; sx: number; sy: number; ox: number; oy: number } | { mode: 'connect'; from: string; mx: number; my: number; moved: boolean };
  let drag = $state<Drag | null>(null);
  const toWorld = (cx: number, cy: number): { x: number; y: number } => {
    const r = surface!.getBoundingClientRect();
    return { x: (cx - r.left - tx) / zoom, y: (cy - r.top - ty) / zoom };
  };
  function startPan(e: PointerEvent): void {
    if (e.button !== 0 || e.target !== e.currentTarget) return;
    onselect(null);
    onedgeselect(null);
    drag = { mode: 'pan', sx: e.clientX, sy: e.clientY, ox: tx, oy: ty };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }
  function startConnect(e: PointerEvent, id: string): void {
    if (e.button !== 0) return;
    e.stopPropagation();
    const w = toWorld(e.clientX, e.clientY);
    drag = { mode: 'connect', from: id, mx: w.x, my: w.y, moved: false };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }
  function onMove(e: PointerEvent): void {
    if (drag?.mode === 'pan') {
      tx = drag.ox + e.clientX - drag.sx;
      ty = drag.oy + e.clientY - drag.sy;
    } else if (drag?.mode === 'connect') {
      const w = toWorld(e.clientX, e.clientY);
      drag.moved ||= Math.hypot(w.x - drag.mx, w.y - drag.my) > 4 / zoom;
      drag.mx = w.x;
      drag.my = w.y;
    }
  }
  function endDrag(): void {
    const d = drag;
    drag = null;
    if (d?.mode !== 'connect' || !d.moved) return;
    connectDragged = true;
    const hit = graph.nodes.find((n) => {
      const p = layout.pos[n.id];
      return n.id !== d.from && p && d.mx >= p.x && d.mx <= p.x + CARD_W && d.my >= p.y - 12 && d.my <= p.y + CARD_H;
    });
    if (hit) onconnect(d.from, hit.id);
  }

  const label = (n: WorkflowNode): string => n.name || n.kind;
  let connectDragged = false;
  function connectMenu(e: MouseEvent, n: WorkflowNode): void {
    if (connectDragged) return void (connectDragged = false);
    const targets = graph.nodes.filter((t) => t.id !== n.id && t.kind !== 'manual_trigger');
    const items: MenuItem[] = targets.length
      ? targets.map((t) => {
          const linked = graph.edges.some((x) => x.source === n.id && x.target === t.id);
          return { label: label(t), checked: linked, disabled: linked, title: linked ? 'Already connected' : `Connect “${label(n)}” to “${label(t)}”`, action: () => onconnect(n.id, t.id) };
        })
      : [{ label: 'No other step to connect to', disabled: true }];
    ctxMenu.showAt(e.currentTarget as HTMLElement, items, { filter: items.length > 8, filterPlaceholder: 'Find a step…' });
  }

  function onNodeKey(e: KeyboardEvent, id: string): void {
    if (e.key === 'Delete' || e.key === 'Backspace') {
      e.preventDefault();
      ondeletenode(id);
    }
  }
  function onEdgeKey(e: KeyboardEvent, id: string): void {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      onselect(null);
      onedgeselect(id);
    } else if (e.key === 'Delete' || e.key === 'Backspace') {
      e.preventDefault();
      ondeleteedge(id);
    }
  }

  function edgePath(s: string, t: string): string {
    const a = layout.pos[s];
    const b = layout.pos[t];
    const x1 = a.x + CARD_W / 2;
    const y1 = a.y + CARD_H;
    const x2 = b.x + CARD_W / 2;
    const y2 = b.y;
    const dy = Math.max(30, Math.abs(y2 - y1) / 2);
    return `M ${x1} ${y1} C ${x1} ${y1 + dy}, ${x2} ${y2 - dy}, ${x2} ${y2}`;
  }
  /** "yes" / "no" for a condition's `== true` / `== false` edge, else the expression's start. */
  function pill(source: WorkflowNode | undefined, c: string): string {
    const t = c.replace(/\s+/g, ' ').trim();
    if (source?.kind === 'condition' && /^(output\.)?result == true$/.test(t)) return 'yes';
    if (source?.kind === 'condition' && /^(output\.)?result == false$/.test(t)) return 'no';
    return t.length > 24 ? `${t.slice(0, 23)}…` : t;
  }
  function mid(s: string, t: string): { x: number; y: number } {
    const a = layout.pos[s];
    const b = layout.pos[t];
    return { x: (a.x + b.x + CARD_W) / 2, y: (a.y + CARD_H + b.y) / 2 };
  }
  const tempPath = $derived.by(() => {
    if (drag?.mode !== 'connect' || !drag.moved) return '';
    const a = layout.pos[drag.from];
    if (!a) return '';
    const x1 = a.x + CARD_W / 2;
    const y1 = a.y + CARD_H;
    const dy = Math.max(30, Math.abs(drag.my - y1) / 2);
    return `M ${x1} ${y1} C ${x1} ${y1 + dy}, ${drag.mx} ${drag.my - dy}, ${drag.mx} ${drag.my}`;
  });
  const hintId = $props.id();
</script>

<div class="ec" role="application" aria-label="Workflow canvas" tabindex="-1" data-testid="wf-editor-canvas" bind:this={surface}
  onpointerdown={startPan} onpointermove={onMove} onpointerup={endDrag} onwheel={onWheel}>
  <!-- Graph coordinates are physical: the world layer stays LTR (card text reads in its own direction). -->
  <div class="ec-world" dir="ltr" style="transform: translate({tx}px, {ty}px) scale({zoom});">
    <svg class="ec-edges" width={layout.width + 400} height={layout.height + 200} aria-hidden="false">
      {#each graph.edges as e (e.id)}
        {#if layout.pos[e.source] && layout.pos[e.target]}
          {@const s = byId.get(e.source)}
          {@const t = byId.get(e.target)}
          <path class="ec-edge" class:selected={selectedEdgeId === e.id} class:cond={!!e.condition} d={edgePath(e.source, e.target)} />
          <path class="ec-hit" d={edgePath(e.source, e.target)} role="button" tabindex="0" aria-pressed={selectedEdgeId === e.id}
            aria-label="Connection from {s ? label(s) : e.source} to {t ? label(t) : e.target}{e.condition ? `, when ${e.condition}` : ''}"
            aria-describedby="{hintId}-edge"
            onpointerdown={(ev) => { ev.stopPropagation(); onselect(null); onedgeselect(e.id); }} onkeydown={(ev) => onEdgeKey(ev, e.id)} />
          {#if e.condition}
            {@const m = mid(e.source, e.target)}
            {@const text = pill(s, e.condition)}
            <g class="ec-pill" pointer-events="none">
              <title>{e.condition}</title>
              <rect x={m.x - (text.length * 3.4 + 10)} y={m.y - 9} width={text.length * 6.8 + 20} height="18" rx="9" />
              <text x={m.x} y={m.y + 4} text-anchor="middle">{text}</text>
            </g>
          {/if}
        {/if}
      {/each}
      {#if tempPath}<path class="ec-edge temp" d={tempPath} />{/if}
    </svg>

    {#if layout.looseY !== null}
      <span class="ec-loose" style="inset-inline-start: 0; inset-block-start: {layout.looseY - 26}px;">Not connected</span>
    {/if}

    {#each graph.nodes as n (n.id)}
      {@const p = layout.pos[n.id]}
      {@const se = nodeSideEffect(n)}
      {@const bad = invalid[n.id]}
      {#if p}
        <div class="ec-node" class:selected={selectedId === n.id} class:invalid={!!bad} class:outward={!!se}
          style="inset-inline-start: {p.x}px; inset-block-start: {p.y}px;" data-testid="wf-node" data-node-id={n.id} data-kind={n.kind}>
          <button type="button" class="ec-main" aria-pressed={selectedId === n.id} aria-describedby="{hintId}-node"
            aria-label="{label(n)}: {nodeSummary(n, ctx)}{bad ? `. Problem: ${bad}` : ''}"
            onclick={() => { onselect(n.id); onedgeselect(null); }} onkeydown={(e) => onNodeKey(e, n.id)}>
            <span class="ec-tile"><Icon name={nodeIcon(n.kind)} size={14} /></span>
            <span class="ec-text" dir="auto">
              <span class="ec-name">{label(n)}</span>
              <span class="ec-sum">{nodeSummary(n, ctx)}</span>
            </span>
          </button>
          {#if bad}<span class="ec-bad" title={bad} aria-hidden="true">!</span>
          {:else if se}<span class="ec-out" title="Posts outside Otto: {se.summary}" aria-hidden="true"><Icon name="external" size={12} /></span>{/if}
          <button type="button" class="ec-port" aria-label="Connect {label(n)} to…" title="Drag to a step, or click to pick one"
            onpointerdown={(e) => startConnect(e, n.id)} onclick={(e) => connectMenu(e, n)}></button>
        </div>
      {/if}
    {/each}
  </div>

  <div class="ec-hud" role="group" aria-label="Zoom">
    <button type="button" aria-label="Zoom out" title="Zoom out" onclick={() => zoomBy(1 / 1.2)}><Icon name="minus" size={12} /></button>
    <span class="ec-pct">{Math.round(zoom * 100)}%</span>
    <button type="button" aria-label="Zoom in" title="Zoom in" onclick={() => zoomBy(1.2)}><Icon name="plus" size={12} /></button>
    <button type="button" aria-label="Fit to view" title="Fit to view" onclick={fit}><Icon name="maximize" size={12} /></button>
  </div>
  <p class="ec-sr" id="{hintId}-node">Delete removes the step. Its port below connects it to another step.</p>
  <p class="ec-sr" id="{hintId}-edge">Enter selects the connection, Delete removes it.</p>
</div>

<style>
  .ec {
    position: relative;
    flex: 1;
    min-block-size: 0;
    min-inline-size: 0;
    overflow: hidden;
    background-color: var(--bg);
    background-image: radial-gradient(var(--border) 1px, transparent 1px);
    background-size: 18px 18px;
    touch-action: none;
  }
  .ec-world {
    position: absolute;
    inset-block-start: 0;
    inset-inline-start: 0;
    transform-origin: 0 0;
  }
  .ec-edges {
    position: absolute;
    inset-inline-start: 0;
    inset-block-start: 0;
    overflow: visible;
  }
  .ec-edge {
    fill: none;
    stroke: var(--border-strong);
    stroke-width: 1.5;
  }
  .ec-edge.cond {
    stroke-dasharray: 5 4;
  }
  .ec-edge.selected {
    stroke: var(--accent-text);
    stroke-width: 2;
  }
  .ec-edge.temp {
    stroke: var(--accent-text);
    stroke-dasharray: 4 4;
  }
  .ec-hit {
    fill: none;
    stroke: transparent;
    stroke-width: 12;
    cursor: pointer;
    pointer-events: stroke;
  }
  .ec-hit:focus-visible {
    stroke: var(--accent-soft-strong);
    outline: none;
  }
  .ec-pill rect {
    fill: var(--surface);
    stroke: var(--border);
  }
  .ec-pill text {
    font-size: var(--fs-xs);
    fill: var(--text-dim);
  }
  .ec-loose {
    position: absolute;
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-dim);
  }
  .ec-node {
    position: absolute;
    inline-size: 208px;
    block-size: 56px;
  }
  .ec-main {
    display: flex;
    align-items: center;
    gap: 10px;
    inline-size: 100%;
    block-size: 100%;
    padding: 0 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    box-shadow: var(--shadow-xs);
    color: var(--text);
    font: inherit;
    text-align: start;
    cursor: pointer;
  }
  .ec-node.outward .ec-main,
  .ec-node.invalid .ec-main {
    padding-inline-end: 24px;
  }
  .ec-main:hover {
    border-color: var(--border-strong);
  }
  .ec-node.selected .ec-main {
    border-color: var(--accent);
    box-shadow: 0 0 0 2px var(--accent-line);
  }
  .ec-node.invalid .ec-main {
    border-color: var(--danger);
  }
  .ec-main:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 2px;
  }
  .ec-tile {
    display: grid;
    place-items: center;
    flex: none;
    inline-size: 28px;
    block-size: 28px;
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .ec-text {
    display: flex;
    flex-direction: column;
    min-inline-size: 0;
  }
  .ec-name {
    font-size: var(--fs-m);
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .ec-sum {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .ec-out,
  .ec-bad {
    position: absolute;
    inset-block-start: 6px;
    inset-inline-end: 8px;
    color: var(--text-dim);
    pointer-events: auto;
  }
  .ec-bad {
    font-weight: 600;
    font-size: var(--fs-s);
    color: var(--danger);
  }
  .ec-port {
    position: absolute;
    inset-block-end: -6px;
    inset-inline-start: calc(50% - 6px);
    inline-size: 12px;
    block-size: 12px;
    padding: 0;
    border: 1px solid var(--border-strong);
    border-radius: 50%;
    background: var(--surface);
    cursor: crosshair;
    opacity: 0;
  }
  .ec-node:hover .ec-port,
  .ec-node.selected .ec-port,
  .ec-port:focus-visible {
    opacity: 1;
  }
  .ec-hud {
    position: absolute;
    inset-block-end: 12px;
    inset-inline-start: 12px;
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 2px 4px;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: var(--surface);
    box-shadow: var(--shadow-xs);
  }
  .ec-hud button {
    display: grid;
    place-items: center;
    inline-size: 28px;
    block-size: 28px;
    border: none;
    border-radius: 50%;
    background: none;
    color: var(--text-dim);
    cursor: pointer;
  }
  .ec-hud button:hover {
    color: var(--text);
  }
  .ec-pct {
    min-inline-size: 36px;
    text-align: center;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .ec-sr {
    position: absolute;
    inline-size: 1px;
    block-size: 1px;
    overflow: hidden;
    clip-path: inset(50%);
  }
  @media (pointer: coarse) {
    .ec-port {
      opacity: 1;
      inline-size: 20px;
      block-size: 20px;
      inset-block-end: -10px;
      inset-inline-start: calc(50% - 10px);
    }
  }
</style>
