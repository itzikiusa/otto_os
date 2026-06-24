<script lang="ts">
  // The SvelteFlow integration. The store owns the canonical Scene; this editor
  // owns the *live* flow arrays (for smooth dragging) and reconciles in both
  // directions:
  //
  //   store → editor   a `$effect` that READS `canvas.rev` re-derives `nodes`/
  //                    `edges` from `sceneToFlow(canvas.scene)`. `rev` bumps on
  //                    open/undo/redo/assist/template/rename/tool-insert.
  //   editor → store   on drag-stop / connect / a structural change we debounce
  //                    (~250ms) `canvas.commitFromEditor(flowToScene(...))` —
  //                    which records history + autosaves WITHOUT bumping rev (so
  //                    we're not yanked mid-interaction).
  //
  // Selection is lifted to CanvasPage via the `onselect` callback so the Toolbar
  // and Inspector can act on the current node(s). Tool insertion: when an
  // `activeTool` is set, a pane click drops a `makeNode(kind, …)` at the click
  // point (scene coords via `screenToFlowPosition`) and resets the tool.

  import { untrack } from 'svelte';
  import {
    SvelteFlow,
    Background,
    Controls,
    MiniMap,
    useSvelteFlow,
    type Node as FlowNodeT,
    type Edge as FlowEdgeT,
  } from '@xyflow/svelte';
  import { canvas } from '../../lib/stores/canvas.svelte';
  import { flowToScene, sceneToFlow, makeNode, genId } from './scene';
  import type { CanvasNode, NodeKind, ShapeVariant } from './types';
  import type { Tool } from './tools';

  import ShapeNode from './nodes/ShapeNode.svelte';
  import TextNode from './nodes/TextNode.svelte';
  import StickyNode from './nodes/StickyNode.svelte';
  import CodeNode from './nodes/CodeNode.svelte';
  import JsonNode from './nodes/JsonNode.svelte';
  import MermaidNode from './nodes/MermaidNode.svelte';
  import ImageNode from './nodes/ImageNode.svelte';
  import FreehandNode from './nodes/FreehandNode.svelte';
  import FrameNode from './nodes/FrameNode.svelte';

  interface Props {
    /** Currently selected tool from the Toolbar (or 'select'). */
    activeTool: Tool;
    /** Reset the tool back to 'select' after a one-shot insert. */
    onToolDone: () => void;
    /** Lift selected node ids up to the page (Inspector/Toolbar). */
    onselect: (ids: string[]) => void;
    /** Pick a tool via a single-key shortcut (V/S/T/R/C/M/J/I/F). */
    ontool?: (t: Tool) => void;
    /** Read-only mode (phone) — disables editing affordances. */
    readonly?: boolean;
  }
  let { activeTool, onToolDone, onselect, ontool, readonly = false }: Props = $props();

  // Single-key → tool map (fires only when the editor has focus and the user is
  // not typing into a node field). Mirrors the ToolRail tooltips.
  const TOOL_KEYS: Record<string, Tool> = {
    v: 'select',
    s: 'sticky',
    t: 'text',
    r: 'shape:rect',
    c: 'connector',
    m: 'mermaid',
    j: 'json',
    i: 'image',
    f: 'frame',
  };

  // Map our NodeKind → the registered node component. `group` reuses FrameNode.
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const nodeTypes: Record<string, any> = {
    shape: ShapeNode,
    text: TextNode,
    sticky: StickyNode,
    code: CodeNode,
    json: JsonNode,
    mermaid: MermaidNode,
    image: ImageNode,
    frame: FrameNode,
    freehand: FreehandNode,
    group: FrameNode,
  };

  // Live flow arrays — SvelteFlow mutates these in place (drag updates position),
  // so they're bound and re-seeded from the store on every `rev` bump.
  let nodes = $state.raw<FlowNodeT[]>([]);
  let edges = $state.raw<FlowEdgeT[]>([]);

  const { screenToFlowPosition, fitView } = useSvelteFlow();

  // store → editor: reload ONLY when `rev` bumps (open/undo/redo/assist/template/
  // rename/tool-insert). The scene is read UNTRACKED so a plain `commitFromEditor`
  // write (drag/connect) does NOT re-run this effect and remount every node.
  $effect(() => {
    void canvas.rev; // the sole dependency
    const scene = untrack(() => canvas.scene);
    if (!scene) {
      nodes = [];
      edges = [];
      return;
    }
    const flow = sceneToFlow(scene);
    nodes = flow.nodes as unknown as FlowNodeT[];
    edges = flow.edges as unknown as FlowEdgeT[];
  });

  // editor → store: debounce structural/position changes back into the scene.
  let commitTimer: ReturnType<typeof setTimeout> | null = null;
  function scheduleCommit(): void {
    if (readonly || !canvas.scene) return;
    if (commitTimer) clearTimeout(commitTimer);
    commitTimer = setTimeout(() => {
      if (!canvas.scene) return;
      // Cast through our FlowNode/FlowEdge shape — sceneToFlow produced these,
      // and SvelteFlow only mutated position/measured fields on them.
      const next = flowToScene(
        canvas.scene,
        nodes as unknown as ReturnType<typeof sceneToFlow>['nodes'],
        edges as unknown as ReturnType<typeof sceneToFlow>['edges'],
      );
      canvas.commitFromEditor(next);
    }, 250);
  }

  // --- selection → page -----------------------------------------------------
  function emitSelection(): void {
    const ids = nodes.filter((n) => n.selected).map((n) => n.id);
    onselect(ids);
  }

  // --- connect: append an edge ----------------------------------------------
  function onconnect(conn: { source: string; target: string }): void {
    if (readonly) return;
    const id = genId('e');
    const newEdge = {
      id,
      source: conn.source,
      target: conn.target,
      type: 'default',
      data: {
        edge: { id, source: conn.source, target: conn.target, kind: 'arrow' as const },
      },
    } as unknown as FlowEdgeT;
    edges = [...edges, newEdge];
    scheduleCommit();
  }

  // --- tool insert on pane click --------------------------------------------
  function onpaneclick({ event }: { event: MouseEvent }): void {
    if (readonly) {
      onselect([]);
      return;
    }
    onselect([]); // clicking empty pane clears selection
    if (activeTool === 'select' || activeTool === 'connector') return;
    const pos = screenToFlowPosition({ x: event.clientX, y: event.clientY });
    insertTool(activeTool, pos.x, pos.y);
    onToolDone();
  }

  // Map a tool to a node kind (+ optional shape variant) and drop it at (x,y).
  function insertTool(tool: Tool, x: number, y: number): void {
    if (!canvas.scene) return;
    let node: CanvasNode;
    const shapeVariants: ShapeVariant[] = [
      'rect',
      'roundrect',
      'ellipse',
      'diamond',
      'triangle',
      'cylinder',
      'parallelogram',
    ];
    if (tool.startsWith('shape:')) {
      const variant = tool.slice('shape:'.length) as ShapeVariant;
      node = makeNode('shape', Math.round(x), Math.round(y), {
        variant: shapeVariants.includes(variant) ? variant : 'rect',
      });
    } else {
      node = makeNode(tool as NodeKind, Math.round(x), Math.round(y));
    }
    canvas.setScene({ ...canvas.scene, nodes: [...canvas.scene.nodes, node] });
  }

  // --- keyboard: delete / undo / redo / select-all --------------------------
  // Scoped to the editor surface so it doesn't fight global shortcuts; the host
  // div is focusable and the handler runs only when focus is inside it.
  function onkeydown(e: KeyboardEvent): void {
    if (readonly) return;
    const target = e.target as HTMLElement | null;
    // Don't hijack keys while typing into a node's textarea/input.
    const tag = target?.tagName;
    const typing = tag === 'INPUT' || tag === 'TEXTAREA' || target?.isContentEditable;
    const mod = e.metaKey || e.ctrlKey;

    if (!typing && (e.key === 'Delete' || e.key === 'Backspace')) {
      const selIds = new Set(nodes.filter((n) => n.selected).map((n) => n.id));
      if (selIds.size) {
        e.preventDefault();
        deleteSelected(selIds);
      }
      return;
    }
    if (mod && (e.key === 'z' || e.key === 'Z')) {
      e.preventDefault();
      if (e.shiftKey) canvas.redo();
      else canvas.undo();
      return;
    }
    if (mod && (e.key === 'y' || e.key === 'Y')) {
      e.preventDefault();
      canvas.redo();
      return;
    }
    if (!typing && mod && (e.key === 'a' || e.key === 'A')) {
      e.preventDefault();
      nodes = nodes.map((n) => ({ ...n, selected: true }));
      emitSelection();
      return;
    }
    // Single-key tool shortcuts (no modifier, not typing).
    if (!typing && !mod && ontool) {
      const tool = TOOL_KEYS[e.key.toLowerCase()];
      if (tool) {
        e.preventDefault();
        ontool(tool);
      }
    }
  }

  // Remove the selected nodes (and any edges touching them) and commit.
  function deleteSelected(ids: Set<string>): void {
    if (!canvas.scene) return;
    const nextNodes = canvas.scene.nodes.filter((n) => !ids.has(n.id));
    const nextEdges = canvas.scene.edges.filter(
      (e) => !ids.has(e.source) && !ids.has(e.target),
    );
    onselect([]);
    canvas.setScene({ ...canvas.scene, nodes: nextNodes, edges: nextEdges });
  }

  // Expose a zoom-to-fit for the Toolbar via a module-level binding on the page.
  export function fit(): void {
    void fitView({ padding: 0.2, duration: 200 });
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
  class="editor"
  class:tool-active={activeTool !== 'select'}
  tabindex="0"
  role="application"
  {onkeydown}
>
  <SvelteFlow
    bind:nodes
    bind:edges
    {nodeTypes}
    fitView
    minZoom={0.1}
    maxZoom={4}
    deleteKey={null}
    multiSelectionKey={['Meta', 'Control', 'Shift']}
    nodesDraggable={!readonly}
    nodesConnectable={!readonly}
    elementsSelectable
    onnodedragstop={scheduleCommit}
    onconnect={onconnect}
    onpaneclick={onpaneclick}
    onnodeclick={emitSelection}
    onselectionclick={emitSelection}
  >
    <Background gap={20} />
    <Controls />
    <MiniMap pannable zoomable />
  </SvelteFlow>
</div>

<style>
  .editor {
    width: 100%;
    height: 100%;
    position: relative;
    outline: none;
  }
  /* While a tool is armed, hint the insert action with a crosshair cursor. */
  .editor.tool-active :global(.svelte-flow__pane) {
    cursor: crosshair;
  }
  /* Theme the flow chrome with our tokens (it ships neutral defaults). */
  .editor :global(.svelte-flow) {
    background: var(--bg);
  }
  .editor :global(.svelte-flow__minimap) {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
  }
  .editor :global(.svelte-flow__controls) {
    box-shadow: var(--shadow);
    border-radius: var(--radius-m);
    overflow: hidden;
  }
  .editor :global(.svelte-flow__controls-button) {
    background: var(--surface);
    border-bottom: 1px solid var(--border);
    color: var(--text);
  }
  .editor :global(.svelte-flow__controls-button:hover) {
    background: var(--surface-2);
  }
  .editor :global(.svelte-flow__controls-button svg) {
    fill: var(--text);
  }
</style>
