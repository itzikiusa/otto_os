// Split-layout tree — the PURE half (no runes, no DOM, no runtime imports) so
// `node --test` can exercise it directly and the store file can re-export it.
// A layout is a binary tree of `col`/`row` splits with a fraction per node and
// session ids at the leaves. Leaves carry their own `key` because the same
// session may legally sit in two panes (split() clones the focused id), so
// every op addresses leaves by key, never by session id.
//
// Erasable syntax only (no enums / namespaces / parameter properties): Node's
// built-in type stripping runs the unit tests without a build step.
import type { Id } from '../api/types';

export type Axis = 'col' | 'row';
export type Side = 'left' | 'right' | 'up' | 'down';
export type Leaf = { kind: 'leaf'; key: string; session: Id };
export type Split = { kind: 'split'; key: string; axis: Axis; frac: number; a: LayoutNode; b: LayoutNode };
export type LayoutNode = Leaf | Split;
export type Preset = 'cols' | 'rows' | 'one-two-below' | 'one-two-beside' | 'grid';
export type LayoutV2 = { v: 2; tree: LayoutNode | null; focused: string | null };
export type LayoutV1 = { panes?: Id[]; axis?: Axis };
/** Structural rect (DOMRect-compatible) so the pure module + unit tests need no DOM lib. */
export type Rect = { left: number; top: number; width: number; height: number };

/** Pane cap. SINGLE SOURCE OF TRUTH shared with the tiled view: `TiledView.svelte` imports this and sets
 *  `const MAX_LIVE_TILES = MAX_PANES;` (the old local literal 15 is gone). User decision 2026-09-11: 15, not 6. */
export const MAX_PANES = 15;
export const FRAC_MIN = 0.1;
export const FRAC_MAX = 0.9;
/** Gutter drag refuses to make either side narrower than this (drag-time only; window resize is not guarded). */
export const MIN_PANE_PX = 160;
export const LS_PANES = 'otto_panes_'; // + ws key — SAME key as before; payload v2
export const LS_TILE_ORDER = 'otto_tile_order_'; // + ws key (C3a tiled order)

export const clampFrac = (f: number): number =>
  Math.min(FRAC_MAX, Math.max(FRAC_MIN, Number.isFinite(f) ? f : 0.5));
export const newKey = (): string => `p${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
export const leaf = (session: Id): Leaf => ({ kind: 'leaf', key: newKey(), session });
export const split = (axis: Axis, a: LayoutNode, b: LayoutNode, frac = 0.5): Split => ({
  kind: 'split',
  key: newKey(),
  axis,
  frac: clampFrac(frac),
  a,
  b,
});

/** Leaves in reading order (a before b, depth-first) — `ws.panes` is this list's sessions. */
export function leaves(t: LayoutNode | null): Leaf[] {
  return !t ? [] : t.kind === 'leaf' ? [t] : [...leaves(t.a), ...leaves(t.b)];
}
export function findLeaf(t: LayoutNode | null, key: string): Leaf | null {
  return leaves(t).find((l) => l.key === key) ?? null;
}
/** The split node whose direct child is leaf/split `key` (null for the root or unknown keys). */
export function parentOf(t: LayoutNode | null, key: string): Split | null {
  if (!t || t.kind === 'leaf') return null;
  if (t.a.key === key || t.b.key === key) return t;
  return parentOf(t.a, key) ?? parentOf(t.b, key);
}
/** Insert `node` beside leaf `targetKey`: left/up ⇒ node becomes `a`, right/down ⇒ `b`; col for left/right,
 *  row for up/down. Unknown target ⇒ unchanged. */
export function insertBeside(t: LayoutNode, targetKey: string, node: LayoutNode, side: Side): LayoutNode {
  if (t.kind === 'leaf') {
    if (t.key !== targetKey) return t;
    const axis: Axis = side === 'left' || side === 'right' ? 'col' : 'row';
    return side === 'left' || side === 'up' ? split(axis, node, t) : split(axis, t, node);
  }
  return { ...t, a: insertBeside(t.a, targetKey, node, side), b: insertBeside(t.b, targetKey, node, side) };
}
/** Remove a leaf; its parent split collapses into the sibling subtree (the sibling keeps its own key and fracs). */
export function removeLeaf(t: LayoutNode | null, key: string): LayoutNode | null {
  if (!t) return null;
  if (t.kind === 'leaf') return t.key === key ? null : t;
  const a = removeLeaf(t.a, key);
  const b = removeLeaf(t.b, key);
  if (a === t.a && b === t.b) return t;
  return !a ? b : !b ? a : { ...t, a, b };
}
/** Swap the SESSIONS of two leaves; both keys stay where they are (the focused SLOT is unchanged). */
export function swapLeaves(t: LayoutNode, k1: string, k2: string): LayoutNode {
  const l1 = findLeaf(t, k1);
  const l2 = findLeaf(t, k2);
  if (!l1 || !l2 || k1 === k2) return t;
  const go = (n: LayoutNode): LayoutNode =>
    n.kind === 'leaf'
      ? n.key === k1
        ? { ...n, session: l2.session }
        : n.key === k2
          ? { ...n, session: l1.session }
          : n
      : { ...n, a: go(n.a), b: go(n.b) };
  return go(t);
}
/** Move leaf `key` beside leaf `targetKey` (remove + insert; the moved leaf keeps its key). No-op when
 *  key === targetKey, unknown, or the tree has one leaf. */
export function moveLeaf(t: LayoutNode, key: string, targetKey: string, side: Side): LayoutNode {
  const l = findLeaf(t, key);
  if (!l || key === targetKey || !findLeaf(t, targetKey)) return t;
  const rest = removeLeaf(t, key);
  return rest ? insertBeside(rest, targetKey, l, side) : t;
}
export function setFrac(t: LayoutNode, splitKey: string, frac: number): LayoutNode {
  if (t.kind === 'leaf') return t;
  if (t.key === splitKey) return { ...t, frac: clampFrac(frac) };
  return { ...t, a: setFrac(t.a, splitKey, frac), b: setFrac(t.b, splitKey, frac) };
}
/** Every leaf whose session is `from` becomes `to`; `to === null` removes them. Dedupe: a REPLACED leaf whose new
 *  session already exists in another leaf is removed instead (mirrors closeTab's `arr.indexOf(p) === i` filter for
 *  the replaced slots; pre-existing duplicates from split() are left alone). */
export function replaceSession(t: LayoutNode | null, from: Id, to: Id | null): LayoutNode | null {
  let out = t;
  const present = new Set(leaves(t).filter((l) => l.session !== from).map((l) => l.session));
  for (const l of leaves(t).filter((l) => l.session === from)) {
    if (to === null || present.has(to)) out = removeLeaf(out, l.key);
    else {
      present.add(to);
      out = mapLeaf(out, l.key, (x) => ({ ...x, session: to }));
    }
  }
  return out;
}
function mapLeaf(t: LayoutNode | null, key: string, f: (l: Leaf) => Leaf): LayoutNode | null {
  if (!t) return null;
  if (t.kind === 'leaf') return t.key === key ? f(t) : t;
  return { ...t, a: mapLeaf(t.a, key, f)!, b: mapLeaf(t.b, key, f)! };
}
/** removeLeaf for every leaf failing `keep`. Returns the SAME object when nothing changed (callers compare by identity). */
export function retain(t: LayoutNode | null, keep: (id: Id) => boolean): LayoutNode | null {
  let out = t;
  for (const l of leaves(t)) if (!keep(l.session)) out = removeLeaf(out, l.key);
  return out;
}
/** Rebuild a tree of the same sessions in a canonical shape (`ids` = leaves in reading order). BALANCED halving —
 *  never a head/tail chain: `1/n` would fall under FRAC_MIN at n ≥ 11 (cap 15) and the chain would be n−1 deep.
 *  Halving keeps every fraction in [1/3, 2/3] (never clamped), depth ⌈log₂ n⌉, exact 1/n shares at every n. */
export function applyPreset(ids: Id[], preset: Preset): LayoutNode | null {
  const L = ids.map(leaf);
  const n = L.length;
  if (n === 0) return null;
  if (n === 1) return L[0];
  const chain = (axis: Axis, nodes: LayoutNode[]): LayoutNode => {
    if (nodes.length === 1) return nodes[0];
    const h = Math.floor(nodes.length / 2);
    return split(axis, chain(axis, nodes.slice(0, h)), chain(axis, nodes.slice(h)), h / nodes.length);
  };
  switch (preset) {
    case 'cols':
      return chain('col', L);
    case 'rows':
      return chain('row', L);
    case 'one-two-below':
      return split('row', L[0], chain('col', L.slice(1)));
    case 'one-two-beside':
      return split('col', L[0], chain('row', L.slice(1)));
    case 'grid': {
      // Same column rule as TiledView.svelte (2 for n ≤ 4, 3 for n ≤ 9, else 4).
      const cols = n <= 4 ? 2 : n <= 9 ? 3 : 4;
      const rs = Array.from({ length: Math.ceil(n / cols) }, (_, i) =>
        chain('col', L.slice(i * cols, i * cols + cols)),
      );
      return chain('row', rs);
    }
  }
}
/** v1 → v2. n=1 leaf; n=2 one split on `axis` at the legacy fraction of that axis; n=3 top row (p0|p1) at colFrac
 *  OVER p2 full-width at rowFrac (the user's example — the old 2×2 left an empty cell); n=4 two rows of two. */
export function fromLegacy(v1: LayoutV1, colFrac: number, rowFrac: number): LayoutNode | null {
  const p = (v1.panes ?? []).slice(0, 4);
  const L = p.map(leaf);
  const axis: Axis = v1.axis === 'row' ? 'row' : 'col';
  if (L.length === 0) return null;
  if (L.length === 1) return L[0];
  if (L.length === 2) return split(axis, L[0], L[1], axis === 'col' ? colFrac : rowFrac);
  const top = split('col', L[0], L[1], colFrac);
  return L.length === 3
    ? split('row', top, L[2], rowFrac)
    : split('row', top, split('col', L[2], L[3], colFrac), rowFrac);
}
export function toLegacy(t: LayoutNode | null): { panes: Id[]; axis: Axis } {
  return { panes: leaves(t).map((l) => l.session), axis: t?.kind === 'split' ? t.axis : 'col' };
}

/** Walk a persisted v2 node: leaves whose session is a string passing `valid` survive (keys regenerated when not
 *  a string); splits with a known axis recurse on a/b, clamp the fraction, and collapse into the surviving child
 *  when the other came back null; anything else is dropped. */
function parseNode(raw: unknown, valid: (id: Id) => boolean): LayoutNode | null {
  if (!raw || typeof raw !== 'object') return null;
  const o = raw as Record<string, unknown>;
  const key = typeof o.key === 'string' && o.key !== '' ? o.key : newKey();
  if (o.kind === 'leaf') {
    return typeof o.session === 'string' && valid(o.session) ? { kind: 'leaf', key, session: o.session } : null;
  }
  if (o.kind === 'split' && (o.axis === 'col' || o.axis === 'row')) {
    const a = parseNode(o.a, valid);
    const b = parseNode(o.b, valid);
    if (!a) return b;
    if (!b) return a;
    return { kind: 'split', key, axis: o.axis, frac: clampFrac(Number(o.frac)), a, b };
  }
  return null;
}
/** Parse whatever is in localStorage. (1) null → {tree:null}; (2) JSON.parse throws → {tree:null}; (3) `obj.v === 2`
 *  → walk `obj.tree` (see parseNode); `focused` kept only if it names a surviving leaf; (4) otherwise v1:
 *  `{panes: (obj.panes ?? []).filter(valid), axis: obj.axis}` → fromLegacy(v1, colFrac, rowFrac). Finally trailing
 *  leaves beyond MAX_PANES are removed. */
export function parseLayout(
  raw: string | null,
  valid: (id: Id) => boolean,
  colFrac: number,
  rowFrac: number,
): { tree: LayoutNode | null; focused: string | null } {
  if (raw == null) return { tree: null, focused: null };
  let obj: unknown;
  try {
    obj = JSON.parse(raw);
  } catch {
    return { tree: null, focused: null };
  }
  if (!obj || typeof obj !== 'object') return { tree: null, focused: null };
  const o = obj as Record<string, unknown>;
  let tree: LayoutNode | null;
  let focused: string | null = null;
  if (o.v === 2) {
    tree = parseNode(o.tree, valid);
    focused = typeof o.focused === 'string' ? o.focused : null;
  } else {
    const panes = (Array.isArray(o.panes) ? o.panes : []).filter(
      (p): p is Id => typeof p === 'string' && valid(p),
    );
    const axis: Axis | undefined = o.axis === 'col' || o.axis === 'row' ? o.axis : undefined;
    tree = fromLegacy({ panes, axis }, colFrac, rowFrac);
  }
  for (const l of leaves(tree).slice(MAX_PANES)) tree = removeLeaf(tree, l.key);
  if (focused !== null && !findLeaf(tree, focused)) focused = null;
  return { tree, focused };
}
/** Tiled order: ids present in `order` first (in that order), unknown ids appended in `sessions` order; ids in
 *  `order` with no session are skipped. */
export function applyTileOrder<T extends { id: Id }>(sessions: T[], order: Id[]): T[] {
  const byId = new Map(sessions.map((s) => [s.id, s]));
  const out: T[] = [];
  const seen = new Set<Id>();
  for (const id of order) {
    const s = byId.get(id);
    if (s && !seen.has(id)) {
      out.push(s);
      seen.add(id);
    }
  }
  for (const s of sessions) if (!seen.has(s.id)) out.push(s);
  return out;
}
/** `ui.reorderSidebar` semantics over an id list: pull `fromId` out and reinsert at `toId`'s index; unknown ids ⇒ unchanged. */
export function reorderIds(ids: Id[], fromId: Id, toId: Id): Id[] {
  if (fromId === toId) return ids;
  const next = [...ids];
  const from = next.indexOf(fromId);
  const to = next.indexOf(toId);
  if (from < 0 || to < 0) return ids;
  next.splice(from, 1);
  next.splice(to, 0, fromId);
  return next;
}
/** Geometric neighbour for keyboard moves. Candidates = leaves whose rect CENTRE lies on `side` of `key`'s centre
 *  (right: cx > sx; left: cx < sx; down: cy > sy; up: cy < sy). Prefer candidates whose cross-axis span overlaps the
 *  source's span (a tall left column beside a two-row right column moves into the row you are level with); among
 *  those, the nearest centre. null when there is no candidate. */
export function neighbour(rects: Map<string, Rect>, key: string, side: Side): string | null {
  const src = rects.get(key);
  if (!src) return null;
  const cx = (r: Rect): number => r.left + r.width / 2;
  const cy = (r: Rect): number => r.top + r.height / 2;
  const sx = cx(src);
  const sy = cy(src);
  const horizontal = side === 'left' || side === 'right';
  const onSide = (r: Rect): boolean =>
    side === 'right' ? cx(r) > sx : side === 'left' ? cx(r) < sx : side === 'down' ? cy(r) > sy : cy(r) < sy;
  // Cross-axis overlap: for a horizontal move the vertical spans must overlap, and vice versa.
  const overlaps = (r: Rect): boolean =>
    horizontal
      ? r.top < src.top + src.height && r.top + r.height > src.top
      : r.left < src.left + src.width && r.left + r.width > src.left;
  let best: string | null = null;
  let bestOverlap = false;
  let bestDist = Infinity;
  for (const [k, r] of rects) {
    if (k === key || !onSide(r)) continue;
    const ov = overlaps(r);
    const d = Math.hypot(cx(r) - sx, cy(r) - sy);
    if ((ov && !bestOverlap) || (ov === bestOverlap && d < bestDist)) {
      best = k;
      bestOverlap = ov;
      bestDist = d;
    }
  }
  return best;
}
