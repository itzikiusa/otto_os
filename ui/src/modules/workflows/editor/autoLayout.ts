// The editor's layout (pure): every graph is drawn top to bottom, computed on
// each render — stored node x/y stay in the data untouched (old builds and
// imports keep them) but are no longer used to draw (ux.md §9.1, DR-026).
//
// Longest-path layering from the trigger, then one barycentre pass inside each
// layer; ties keep the stored order — (y, x), then graph index — because
// legacy graphs were drawn left to right, so y carries their branch order.
// A card sits under the middle of its parents (a lone child under its parent,
// a join under the span it joins, a fan centred under its source), pushed
// apart only as far as the cards need.
// A step no trigger reaches (its incoming edge was deleted, or a cycle) goes
// to a bottom "Not connected" row instead of jumping beside the trigger.
import type { WorkflowGraph, WorkflowNode } from '../../../lib/api/types';

export const CARD_W = 208;
export const CARD_H = 56;
/** Top of one layer to the top of the next. */
export const LAYER_PITCH = 94;
const COL_GAP = 24;
/** Extra room above the "Not connected" row for its label. */
const LOOSE_GAP = 40;

export interface Layout {
  pos: Record<string, { x: number; y: number }>;
  /** Node ids per layer, in drawing order (the phone outline reads this). */
  layers: string[][];
  /** Steps no trigger reaches, in stored order. */
  loose: string[];
  /** Top of the "Not connected" row (null when every step is connected). */
  looseY: number | null;
  width: number;
  height: number;
}

const byStored = (a: { n: WorkflowNode; i: number }, b: { n: WorkflowNode; i: number }): number =>
  a.n.y - b.n.y || a.n.x - b.n.x || a.i - b.i;

export function autoLayout(graph: WorkflowGraph, isTrigger: (kind: string) => boolean = (k) => k === 'manual_trigger'): Layout {
  const items = graph.nodes.map((n, i) => ({ n, i }));
  const ids = new Set(graph.nodes.map((n) => n.id));
  const edges = graph.edges.filter((e) => ids.has(e.source) && ids.has(e.target) && e.source !== e.target);
  const out = new Map<string, string[]>();
  const parents = new Map<string, string[]>();
  for (const e of edges) {
    out.set(e.source, [...(out.get(e.source) ?? []), e.target]);
    parents.set(e.target, [...(parents.get(e.target) ?? []), e.source]);
  }
  // Roots: the triggers; a graph without one starts from its sources.
  let roots = items.filter((x) => isTrigger(x.n.kind));
  if (roots.length === 0) roots = items.filter((x) => !parents.has(x.n.id));
  // What the roots reach.
  const reached = new Set(roots.map((x) => x.n.id));
  const queue = [...reached];
  while (queue.length) for (const t of out.get(queue.shift()!) ?? []) if (!reached.has(t)) (reached.add(t), queue.push(t));
  // Longest path (Kahn over the reached part; a cycle never drains and is loose).
  const indeg = new Map<string, number>();
  for (const e of edges) if (reached.has(e.source) && reached.has(e.target)) indeg.set(e.target, (indeg.get(e.target) ?? 0) + 1);
  const layer = new Map<string, number>();
  const ready = items.filter((x) => reached.has(x.n.id) && !indeg.get(x.n.id)).map((x) => x.n.id);
  for (const id of ready) layer.set(id, 0);
  while (ready.length) {
    const id = ready.shift()!;
    for (const t of out.get(id) ?? []) {
      if (!reached.has(t)) continue;
      layer.set(t, Math.max(layer.get(t) ?? 0, layer.get(id)! + 1));
      const left = indeg.get(t)! - 1;
      indeg.set(t, left);
      if (left === 0) ready.push(t);
    }
  }
  const layers: string[][] = [];
  const slot = new Map<string, number>();
  const placed = items.filter((x) => layer.has(x.n.id) && !indeg.get(x.n.id));
  const depth = placed.reduce((m, x) => Math.max(m, layer.get(x.n.id)! + 1), 0);
  for (let k = 0; k < depth; k++) {
    const here = placed.filter((x) => layer.get(x.n.id) === k);
    const bary = (x: { n: WorkflowNode }): number => {
      const ps = (parents.get(x.n.id) ?? []).filter((p) => slot.has(p));
      return ps.length ? ps.reduce((s, p) => s + slot.get(p)!, 0) / ps.length : 0;
    };
    here.sort((a, b) => bary(a) - bary(b) || byStored(a, b));
    here.forEach((x, i) => slot.set(x.n.id, i));
    layers.push(here.map((x) => x.n.id));
  }
  const loose = items.filter((x) => !slot.has(x.n.id)).sort(byStored).map((x) => x.n.id);
  const PITCH = CARD_W + COL_GAP;
  const pos: Layout['pos'] = {};
  layers.forEach((l, k) => {
    // Where each card wants to be: the middle of its parents' span.
    const want = l.map((id, i) => {
      const xs = (parents.get(id) ?? []).filter((p) => pos[p]).map((p) => pos[p].x);
      return xs.length ? (Math.min(...xs) + Math.max(...xs)) / 2 : i * PITCH;
    });
    // In order, at least a pitch apart; then the layer moves back by its mean push.
    const x: number[] = [];
    want.forEach((w, i) => x.push(i ? Math.max(w, x[i - 1] + PITCH) : w));
    const push = x.reduce((s, v, i) => s + v - want[i], 0) / x.length;
    l.forEach((id, i) => (pos[id] = { x: x[i] - push, y: k * LAYER_PITCH }));
  });
  const xs = Object.values(pos).map((p) => p.x);
  const minX = xs.length ? Math.min(...xs) : 0;
  for (const p of Object.values(pos)) p.x -= minX;
  const placedW = xs.length ? Math.max(...xs) - minX + CARD_W : 0;
  const looseW = loose.length * PITCH - COL_GAP;
  const width = Math.max(CARD_W, placedW, looseW);
  const bottom = layers.length ? (layers.length - 1) * LAYER_PITCH + CARD_H : 0;
  const looseY = loose.length ? bottom + LOOSE_GAP : null;
  loose.forEach((id, i) => (pos[id] = { x: (width - looseW) / 2 + i * PITCH, y: looseY! }));
  const height = looseY !== null ? looseY + CARD_H : bottom;
  return { pos, layers, loose, looseY, width, height };
}

/** The zoom an editor opens at: the whole graph when it fits (never above
 *  100 %); a graph that would shrink below 60 % opens at 80 % instead, its
 *  trigger at the top, and scrolls. */
export function openZoom(layout: Pick<Layout, 'width' | 'height'>, viewW: number, viewH: number, pad = 32): number {
  const fit = Math.min(1, (viewW - 2 * pad) / Math.max(1, layout.width), (viewH - 2 * pad) / Math.max(1, layout.height));
  return fit < 0.6 ? 0.8 : fit;
}
