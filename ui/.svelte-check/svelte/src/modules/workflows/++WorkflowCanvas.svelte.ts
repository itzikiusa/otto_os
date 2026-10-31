///<reference types="svelte" />
;// A lightweight n8n-style node-graph canvas: pan (drag background), zoom
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
import { tick } from 'svelte';
import { neighbourAfterDelete } from './canvasFocus';
import { NODE_W, nodeHeight, loopStepLabels, loopUntil, loopMax, edgePath, edgeMid, condLabel } from './canvasGeometry';
import type { WorkflowGraph, WorkflowNode, NodeTypeSpec, NodeRunState } from '../../lib/api/types';
;

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
  };;

  // --- drag state -----------------------------------------------------------
  type Drag =
    | { mode: 'pan'; sx: number; sy: number; ox: number; oy: number }
    | { mode: 'node'; id: string; sx: number; sy: number; nx: number; ny: number }
    | { mode: 'connect'; from: string; mx: number; my: number };;function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

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
  }: Props = $props()/*Ωignore_startΩ*/;graph;/*Ωignore_endΩ*/;

  let scale = $state(1);
  let tx = $state(40);
  let ty = $state(20);
  let surface = $state<HTMLDivElement | null>(null);

  const typeMap = $derived(new Map(types.map((t) => [t.kind, t])));

  let drag = $state<Drag | null>(null);

  function spec(kind: string): NodeTypeSpec | undefined {
    return typeMap.get(kind);
  }
  /** Kind mark for the stripe only. Selection, ports, and focus stay on the theme accent. */
  function kindMark(kind: string): string {
    if (kind === 'human_approval' || kind === 'approval') return 'var(--warning)';
    if (kind === 'condition' || kind === 'loop' || kind === 'foreach') return 'var(--info)';
    if (kind === 'delay' || kind === 'transform') return 'var(--text-dim)';
    return 'var(--accent-text)';
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

  // After a keyboard Delete: focus a neighbouring step (or the canvas) once
  // the keyed {#each} has dropped the removed one, and say what went.
  let announcement = $state('');
  async function afterDelete(focusNodeId: string | null, label: string): Promise<void> {
    announcement = '';
    await tick();
    const btn = focusNodeId
      ? surface?.querySelector<HTMLElement>(`[data-node-id="${CSS.escape(focusNodeId)}"] .node-main`)
      : null;
    (btn ?? surface)?.focus();
    announcement = `Deleted ${label}`;
  }

  const NUDGE = 8;
  /** Keys on a focused node: arrows nudge it (⇧ = 10×), Delete removes it. */
  function onNodeKey(e: KeyboardEvent, n: WorkflowNode): void {
    if (!editable || e.metaKey || e.ctrlKey || e.altKey) return;
    if (e.key === 'Delete' || e.key === 'Backspace') {
      e.preventDefault();
      const focusNext = neighbourAfterDelete(graph.nodes, n.id);
      const label = nodeLabel(n);
      graph.nodes = graph.nodes.filter((x) => x.id !== n.id);
      graph.edges = graph.edges.filter((x) => x.source !== n.id && x.target !== n.id);
      onselect?.(null);
      onchange?.(graph);
      void afterDelete(focusNext, label);
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
      const edge = graph.edges.find((x) => x.id === id);
      const s = edge ? nodeOf(edge.source) : undefined;
      const t = edge ? nodeOf(edge.target) : undefined;
      graph.edges = graph.edges.filter((x) => x.id !== id);
      onedgeselect?.(null);
      onchange?.(graph);
      // A connection's natural neighbour is its source step.
      void afterDelete(s?.id ?? null, s && t ? `connection from ${nodeLabel(s)} to ${nodeLabel(t)}` : 'connection');
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
;
async () => {


  { const $$_div0 = svelteHTML.createElement("div", {                 "class":`canvas`,"role":`application`,"aria-label":`Workflow canvas`,"tabindex":-1,"onpointerdown":startPan,"onpointermove":onMove,"onpointerup":endDrag,"onwheel":onWheel,});surface = $$_div0;
   { svelteHTML.createElement("div", { "class":`dots`,}); }
  
   { svelteHTML.createElement("div", {     "class":`viewport`,"dir":`ltr`,"style":`transform: translate(${tx}px,${ty}px) scale(${scale});`,});
     { svelteHTML.createElement("svg", {     "class":`edges`,"width":`6000`,"height":`4000`,});
         for(let e of __sveltets_2_ensureArray(graph.edges)){e.id;
        const s = nodeOf(e.source);
        const t = nodeOf(e.target);
        if(s && t){
           { svelteHTML.createElement("path", {       "class":`edge`,"d":edgePath(s, t),});invalidEdges.includes(e.id);selectedEdgeId === e.id;!!e.condition;}
           { svelteHTML.createElement("path", {                   "class":`edge-hit`,"d":edgePath(s, t),"onpointerdown":(ev) => {
              ev.stopPropagation();
              selectEdge(e.id);
            },"onkeydown":(ev) => onEdgeKey(ev, e.id),"role":`button`,"tabindex":0,"aria-pressed":selectedEdgeId === e.id,"aria-label":`Connection from ${nodeLabel(s)} to ${nodeLabel(t)}${e.condition ? `, when ${e.condition}` : ''}`,"aria-describedby":editable ? `${hintId}-edge` : undefined,});}
          if(e.condition){
            const m = edgeMid(s, t);
             { svelteHTML.createElement("g", {   "class":`edge-badge`,"pointer-events":`none`,});
               { svelteHTML.createElement("title", {});e.condition; }
               { svelteHTML.createElement("rect", {          "x":m.x - 30,"y":m.y - 9,"width":`60`,"height":`18`,"rx":`9`,});}
               { svelteHTML.createElement("text", {     "x":m.x,"y":m.y + 3,"text-anchor":`middle`,}); condLabel(e.condition); }
             }
          }
        }
      }
      if(drag?.mode === 'connect'){
         { svelteHTML.createElement("path", {    "class":`edge temp`,"d":tempPath(),});}
      }
     }

       for(let n of __sveltets_2_ensureArray(graph.nodes)){n.id;
      const st = statusOf(n.id);
      
       { svelteHTML.createElement("div", {            "class":`node`,...__sveltets_2_empty({"data-status":st}),...__sveltets_2_empty({"data-node-id":n.id}),"style":`left:${n.x}px; top:${n.y}px; width:${NODE_W}px; height:${nodeHeight(n)}px; --kind-mark:${kindMark(n.kind)};`,});invalidNodes.includes(n.id);selectedId === n.id;n.kind === 'loop';
        
         { svelteHTML.createElement("button", {                 "type":`button`,"class":`node-main`,"onpointerdown":(e) => startNode(e, n),"aria-label":`Edit ${nodeLabel(n)}`,"aria-pressed":selectedId === n.id,"aria-describedby":editable ? `${hintId}-node` : undefined,"onclick":() => { onselect?.(n.id); onedgeselect?.(null); },"onkeydown":(e) => onNodeKey(e, n),});
           { svelteHTML.createElement("span", { "class":`stripe`,}); }
           { svelteHTML.createElement("span", { "class":`head`,});
             { svelteHTML.createElement("span", { "class":`ic`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":asIcon(spec(n.kind)?.icon, 'box'),"size":14,}});} }
             { svelteHTML.createElement("span", {   "class":`body`,"dir":`auto`,});
               { svelteHTML.createElement("span", {   "class":`title`,"title":nodeLabel(n),});nodeLabel(n); }
               { svelteHTML.createElement("span", { "class":`kind`,});spec(n.kind)?.label ?? n.kind; }
             }
            if(st){ { svelteHTML.createElement("span", {       "class":`dot ${st}`,"role":`img`,"title":runStatus(st).label,"aria-label":runStatus(st).label,}); }}
           }

          if(n.kind === 'loop'){
            
             { svelteHTML.createElement("span", { "class":`steps`,});
                 for(let s of __sveltets_2_ensureArray(loopStepLabels(n))){let i = 1;
                 { svelteHTML.createElement("span", { "class":`step`,});
                   { svelteHTML.createElement("span", { "class":`si`,});i + 1; }
                   { svelteHTML.createElement("span", {   "class":`sn`,"title":s.name,});s.name; }
                   { svelteHTML.createElement("span", { "class":`sk`,});spec(s.kind)?.label ?? s.kind; }
                 }
              }
              if(loopUntil(n)){
                 { svelteHTML.createElement("span", {   "class":`until`,"title":loopUntil(n),});
                   { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":12,}});}   { svelteHTML.createElement("code", {});condLabel(loopUntil(n)); }if(loopMax(n)){' · max ';loopMax(n);}
                 }
              }
             }
          }
         }

        if((spec(n.kind)?.inputs ?? 1) > 0){
           { svelteHTML.createElement("span", {   "class":`port in`,"aria-hidden":`true`,}); }
        }
        if(editable && (spec(n.kind)?.outputs ?? 1) > 0){
           { svelteHTML.createElement("button", {               "type":`button`,"class":`port out`,"aria-label":`Connect from ${nodeLabel(n)}`,"title":`Connect from ${nodeLabel(n)} — drag to a step’s input, or click to choose…`,"aria-haspopup":`menu`,"onpointerdown":(e) => startConnect(e, n),"onclick":(e) => connectMenu(e, n),}); }
        } else if ((spec(n.kind)?.outputs ?? 1) > 0){
           { svelteHTML.createElement("span", {   "class":`port out`,"aria-hidden":`true`,}); }
        }
       }
    }
   }

   { svelteHTML.createElement("div", { "class":`hud`,});
     { svelteHTML.createElement("button", {       "class":`zbtn`,"onclick":() => (scale = Math.min(2, scale * 1.15)),"title":`Zoom in`,"aria-label":`Zoom in`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});} }
     { svelteHTML.createElement("button", {       "class":`zbtn`,"onclick":() => (scale = Math.max(0.3, scale * 0.87)),"title":`Zoom out`,"aria-label":`Zoom out`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`minus`,"size":12,}});} }
     { svelteHTML.createElement("button", {       "class":`zbtn`,"onclick":fit,"title":`Reset view`,"aria-label":`Reset view`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`maximize`,"size":12,}});} }
     { svelteHTML.createElement("span", { "class":`zpct`,});Math.round(scale * 100);  }
   }
   { svelteHTML.createElement("span", {   "class":`sr-only`,"id":`${hintId}-node`,});            }
   { svelteHTML.createElement("span", {   "class":`sr-only`,"id":`${hintId}-edge`,});       }
   { svelteHTML.createElement("span", {     "class":`sr-only`,"role":`status`,"aria-live":`polite`,});announcement; }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings('graph'), slots: {}, events: {} }}
const WorkflowCanvas__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type WorkflowCanvas__SvelteComponent_ = ReturnType<typeof WorkflowCanvas__SvelteComponent_>;
/*Ωignore_endΩ*/export default WorkflowCanvas__SvelteComponent_;