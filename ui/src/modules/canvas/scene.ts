// Pure Scene helpers + Svelte Flow adapters. No Svelte state here — these are
// the seam every canvas component shares (node factory, scene⇄flow mapping).

import type { CanvasEdge, CanvasNode, NodeKind, Scene, ShapeVariant } from './types';

let _seq = 0;
/** Monotonic-ish id (avoids Math.random, which is fine in the browser but we
 *  keep it deterministic-friendly for tests). */
export function genId(prefix = 'n'): string {
  _seq += 1;
  return `${prefix}_${Date.now().toString(36)}_${_seq.toString(36)}`;
}

export function emptyScene(title = 'Untitled scene'): Scene {
  return { schema: 1, title, nodes: [], edges: [], slides: [], appState: { grid: true } };
}

/** Default size per node kind. */
export function defaultSize(kind: NodeKind): { w: number; h: number } {
  switch (kind) {
    case 'sticky':
      return { w: 180, h: 140 };
    case 'text':
      return { w: 220, h: 48 };
    case 'code':
      return { w: 360, h: 220 };
    case 'json':
      return { w: 320, h: 220 };
    case 'mermaid':
      return { w: 520, h: 360 };
    case 'image':
      return { w: 280, h: 200 };
    case 'frame':
      return { w: 640, h: 420 };
    case 'shape':
    default:
      return { w: 160, h: 90 };
  }
}

/** Build a fresh node of `kind` at scene coords. */
export function makeNode(
  kind: NodeKind,
  x: number,
  y: number,
  opts: { variant?: ShapeVariant; label?: string } = {},
): CanvasNode {
  const { w, h } = defaultSize(kind);
  const node: CanvasNode = { id: genId(kind), kind, x, y, w, h };
  switch (kind) {
    case 'shape':
      node.shape = { variant: opts.variant ?? 'rect' };
      node.label = opts.label ?? '';
      break;
    case 'text':
      node.text = { value: opts.label ?? 'Text', align: 'left', size: 16 };
      break;
    case 'sticky':
      node.sticky = { value: opts.label ?? '', color: '#ffe9a8' };
      break;
    case 'code':
      node.code = { value: '// code', lang: 'ts' };
      break;
    case 'json':
      node.json = { value: '{\n  "key": "value"\n}' };
      break;
    case 'mermaid':
      node.mermaid = { src: 'sequenceDiagram\n  A->>B: hello', kind: 'sequence' };
      break;
    case 'image':
      node.image = {};
      break;
    case 'frame':
      node.label = opts.label ?? 'Frame';
      break;
    case 'freehand':
      node.freehand = { points: [], color: 'var(--text)', size: 4 };
      break;
    case 'group':
      break;
  }
  return node;
}

// ---------------------------------------------------------------------------
// Svelte Flow adapters
// ---------------------------------------------------------------------------

/** A Svelte Flow node carrying our CanvasNode in `data.node`. */
export interface FlowNode {
  id: string;
  type: string;
  position: { x: number; y: number };
  data: { node: CanvasNode };
  width?: number;
  height?: number;
  zIndex?: number;
  selected?: boolean;
}

export interface FlowEdge {
  id: string;
  source: string;
  target: string;
  label?: string;
  type?: string;
  data?: { edge: CanvasEdge };
  animated?: boolean;
}

export function sceneToFlow(scene: Scene): { nodes: FlowNode[]; edges: FlowEdge[] } {
  const nodes: FlowNode[] = scene.nodes.map((n) => ({
    id: n.id,
    type: n.kind,
    position: { x: n.x, y: n.y },
    data: { node: n },
    width: n.w,
    height: n.h,
    zIndex: n.kind === 'frame' ? 0 : (n.z ?? 1),
  }));
  const edges: FlowEdge[] = scene.edges.map((e) => ({
    id: e.id,
    source: e.source,
    target: e.target,
    label: e.label,
    type: e.kind === 'line' ? 'straight' : 'default',
    data: { edge: e },
    animated: false,
  }));
  return { nodes, edges };
}

/** Re-derive the canonical Scene from the live flow arrays (positions/sizes
 *  may have changed via drag/resize). `prev` supplies title/slides/appState. */
export function flowToScene(prev: Scene, flowNodes: FlowNode[], flowEdges: FlowEdge[]): Scene {
  const nodes: CanvasNode[] = flowNodes.map((fn) => {
    const base = fn.data.node;
    return {
      ...base,
      x: Math.round(fn.position.x),
      y: Math.round(fn.position.y),
      w: Math.round(fn.width ?? base.w),
      h: Math.round(fn.height ?? base.h),
    };
  });
  const edges: CanvasEdge[] = flowEdges.map((fe) => fe.data?.edge ?? {
    id: fe.id,
    source: fe.source,
    target: fe.target,
    label: typeof fe.label === 'string' ? fe.label : undefined,
    kind: 'arrow' as const,
  });
  // Drop slide reveal refs / frame refs pointing at deleted nodes.
  const ids = new Set(nodes.map((n) => n.id));
  const slides = prev.slides.map((s) => ({
    ...s,
    frameNodeId: s.frameNodeId && ids.has(s.frameNodeId) ? s.frameNodeId : undefined,
    mermaidNodeId: s.mermaidNodeId && ids.has(s.mermaidNodeId) ? s.mermaidNodeId : undefined,
    reveal: s.reveal.map((r) => ({
      ...r,
      nodeIds: r.nodeIds?.filter((id) => ids.has(id)),
    })),
  }));
  return { schema: 1, title: prev.title, nodes, edges, slides, appState: prev.appState };
}

/** Parse a stored `doc_json`, tolerating bad data by returning an empty scene. */
export function parseScene(docJson: string, fallbackTitle = 'Untitled scene'): Scene {
  try {
    const s = JSON.parse(docJson) as Partial<Scene>;
    return {
      schema: 1,
      title: s.title ?? fallbackTitle,
      nodes: Array.isArray(s.nodes) ? (s.nodes as CanvasNode[]) : [],
      edges: Array.isArray(s.edges) ? (s.edges as CanvasEdge[]) : [],
      slides: Array.isArray(s.slides) ? s.slides : [],
      appState: s.appState ?? { grid: true },
    };
  } catch {
    return emptyScene(fallbackTitle);
  }
}

/** Convert an AssistResult mermaid/nodes payload into ready-to-insert nodes,
 *  laid out near `(ox, oy)`. Mermaid → one mermaid node; tier-2 → its nodes. */
export function assistToNodes(
  res: { mermaid: string | null; nodes: Partial<CanvasNode>[]; edges: Partial<CanvasEdge>[] },
  ox: number,
  oy: number,
): { nodes: CanvasNode[]; edges: CanvasEdge[] } {
  if (res.mermaid) {
    const n = makeNode('mermaid', ox, oy);
    n.mermaid = { src: res.mermaid };
    return { nodes: [n], edges: [] };
  }
  const idMap = new Map<string, string>();
  const nodes: CanvasNode[] = res.nodes.map((raw, i) => {
    const kind: NodeKind = (raw.kind as NodeKind) ?? 'shape';
    const base = makeNode(kind, ox + (raw.x ?? (i % 4) * 200), oy + (raw.y ?? Math.floor(i / 4) * 130));
    if (raw.id) idMap.set(raw.id, base.id);
    base.label = raw.label ?? base.label;
    if (raw.w) base.w = raw.w;
    if (raw.h) base.h = raw.h;
    return base;
  });
  const edges: CanvasEdge[] = res.edges
    .filter((e) => e.source && e.target)
    .map((e) => ({
      id: genId('e'),
      source: idMap.get(e.source as string) ?? (e.source as string),
      target: idMap.get(e.target as string) ?? (e.target as string),
      label: e.label,
      kind: 'arrow',
    }));
  return { nodes, edges };
}
