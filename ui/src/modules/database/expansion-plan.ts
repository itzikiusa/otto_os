// Expansion state + planner for the nested-document views (Vertical / JSON).
// Nothing here is Svelte — ResultsGrid owns the `$state`, the views derive a
// per-record plan from it, and the pure path helpers below are shared with the
// edit flow (dotted-path patches, the replaceOne diff).
import { bsonScalar } from './bson';
import type { TypedKind } from './edit-types';

export interface ExpansionState {
  /** Per-path open/closed overrides, keyed by `stickyKey(path)` so an override
   *  on `items.0.meta` also applies to `items.7.meta` — and to the same path in
   *  every other record. */
  overrides: Map<string, boolean>;
  /** How containers open by default: within a node budget, all, or none. */
  mode: 'budget' | 'all' | 'none';
  budget: number;
  /** Bumped by every mutation (toggle / expand-all / …) so `$derived` plans
   *  recompute even though the Map is mutated in place. */
  version: number;
}

/** Rows a record may auto-open by default. Replaces the old depth-2 / >20-keys
 *  cutoff: what stays shut is decided by SIZE, not by where a field sits. */
export const DEFAULT_BUDGET = 400;
/** Children an open array renders per "show more" slice. */
export const CHUNK = 50;

export function newExpansionState(): ExpansionState {
  return { overrides: new Map(), mode: 'budget', budget: DEFAULT_BUDGET, version: 0 };
}

/** Normalise a dotted path's array indices to `[]` (`items.3.sku` →
 *  `items.[].sku`) so one override covers every element of an array. */
export function stickyKey(path: string): string {
  return path.replace(/(^|\.)\d+(?=\.|$)/g, '$1[]');
}

// The two mutations live here (not in the views) so a child component never
// assigns into the `$state` object ResultsGrid owns — the Map is edited in
// place and `version` is what the derived plans key on.
/** Record a user toggle — sticky by normalised path across every record. */
export function setOverride(st: ExpansionState, path: string, open: boolean): void {
  st.overrides.set(stickyKey(path), open);
  st.version++;
}
/** Expand all / Collapse all / Reset: switch the default and forget every
 *  per-path toggle (a "collapse all" that kept earlier opens wouldn't be one). */
export function setExpansionMode(st: ExpansionState, mode: ExpansionState['mode']): void {
  st.overrides.clear();
  st.mode = mode;
  st.version++;
}

/** A value the tree recurses into: an object/array that is NOT a BSON sentinel. */
function isContainer(v: unknown): v is Record<string, unknown> | unknown[] {
  return v !== null && typeof v === 'object' && bsonScalar(v) === null;
}
/** `[key, child]` pairs of a container (array elements by index). */
export function entriesOf(v: unknown): [string, unknown][] {
  if (!isContainer(v)) return [];
  return Array.isArray(v)
    ? v.map((x, i): [string, unknown] => [String(i), x])
    : Object.entries(v);
}

/** Total tree nodes under `v` (the value itself excluded), stopping early at
 *  `cap` — the Expand-all warning and the planner only need "bigger than X". */
export function estimateNodes(v: unknown, cap = 20_000): number {
  let n = 0;
  const stack: unknown[] = [v];
  while (stack.length > 0 && n < cap) {
    const cur = stack.pop();
    if (!isContainer(cur)) continue;
    const kids = Array.isArray(cur) ? cur : Object.values(cur);
    n += kids.length;
    for (const k of kids) if (isContainer(k)) stack.push(k);
  }
  return Math.min(n, cap);
}

/** Which container paths of ONE record render open. Breadth-first over the
 *  record's entries (the root row is not a node), so a document's top-level
 *  shape always wins over a deep branch:
 *
 *  - an override (`stickyKey(path)` → true/false) always wins and is free;
 *  - `all` opens everything, `none` only the overridden-open paths;
 *  - `budget` opens a container when its direct entries fit the remaining
 *    budget and charges them. When the WHOLE subtree fits, every descendant
 *    opens too; otherwise the branch opens one level and each child opens
 *    only if its own subtree fits — so a wrapper around a huge branch
 *    (`blob: { structure: [300 × {…}] }`) shows `structure` as one closed
 *    summary instead of pushing a 300-row slab into every record, while a
 *    small sibling subdocument next to it still opens.
 *    Children of an override-opened branch stay closed until asked: the
 *    override is sticky across every drawn record, so auto-descending there
 *    would multiply the cost by the batch size.
 *
 *  Paths are dotted with literal array indices (`items.3.sku`); an open
 *  container queues only its first `CHUNK` children because that's all it
 *  renders before its "show more". */
export function planExpansion(obj: Record<string, unknown>, st: ExpansionState): Set<string> {
  const open = new Set<string>();
  let remaining = st.budget;
  // [path, value, auto, wholeOnly] — `auto`: eligible for budget expansion
  // (false under an override-opened branch); `wholeOnly`: opens only when its
  // entire subtree fits (children of a shallow-opened branch).
  const queue: [string, unknown, boolean, boolean][] = Object.entries(obj).map(
    ([k, v]) => [k, v, true, false],
  );
  let qi = 0;
  while (qi < queue.length) {
    const [path, v, auto, wholeOnly] = queue[qi++];
    if (!isContainer(v)) continue;
    const entries = entriesOf(v);
    const size = entries.length;
    if (size === 0) continue;
    const ov = st.overrides.get(stickyKey(path));
    if (ov === false) continue;
    let childAuto: boolean;
    let childWholeOnly = false;
    if (ov === true || st.mode === 'all') {
      childAuto = st.mode === 'all';
    } else if (st.mode === 'none' || !auto || size > remaining) {
      continue;
    } else {
      // Capped walk: past `remaining` the exact subtree size is irrelevant.
      const whole = estimateNodes(v, remaining + 1) <= remaining;
      if (wholeOnly && !whole) continue;
      remaining -= size;
      childAuto = true;
      childWholeOnly = !whole;
    }
    open.add(path);
    for (const [k, child] of entries.slice(0, CHUNK)) {
      queue.push([`${path}.${k}`, child, childAuto, childWholeOnly]);
    }
  }
  return open;
}

/** Flatten a value into `dotted.path → leaf` entries. Only leaves are emitted:
 *  arrays contribute their elements by index, objects their fields; a BSON
 *  sentinel (`{"$oid": …}` etc. — `bsonScalar(v) !== null`) is a leaf, as is an
 *  empty container. Used by the record diff / replaceOne diff. */
export function flattenPaths(v: unknown, prefix = ''): Map<string, unknown> {
  const out = new Map<string, unknown>();
  const walk = (val: unknown, path: string): void => {
    if (val !== null && typeof val === 'object' && bsonScalar(val) === null) {
      const entries = Array.isArray(val)
        ? val.map((x, i) => [String(i), x] as const)
        : Object.entries(val as Record<string, unknown>);
      if (entries.length > 0) {
        for (const [k, child] of entries) walk(child, path ? `${path}.${k}` : k);
        return;
      }
    }
    out.set(path, val);
  };
  walk(v, prefix);
  return out;
}

/** The value at a dotted path (`undefined` when any segment is missing). */
export function getAtPath(root: unknown, path: string): unknown {
  let cur: unknown = root;
  for (const seg of path.split('.')) {
    if (cur === null || typeof cur !== 'object') return undefined;
    cur = (cur as Record<string, unknown>)[seg];
  }
  return cur;
}

/** Structured clone of `root` with ONE dotted-path edit applied — the SQL
 *  Vertical editor's way of turning a nested edit inside a JSON column into a
 *  whole-column draft (one `UPDATE … SET col = '<json>'`, no second path).
 *  `set` creates missing intermediate objects (an array when the next segment
 *  is numeric); `unset` splices array elements out; `rename` moves a field to
 *  a sibling key (the last segment of `newPath`). Unknown paths are no-ops. */
export function applyPatchAtPath(
  root: unknown,
  path: string,
  op: 'set' | 'unset' | 'rename',
  value?: unknown,
  newPath?: string,
): unknown {
  const segs = path.split('.');
  const last = segs.pop()!;
  const isIdx = (s: string): boolean => /^\d+$/.test(s);
  // A scalar/NULL column becomes a fresh container on `set` (an array when the
  // first segment is an index); every other op on it is a no-op.
  const out: unknown =
    root !== null && typeof root === 'object'
      ? structuredClone(root)
      : op === 'set'
        ? isIdx(segs[0] ?? last)
          ? []
          : {}
        : root;
  if (out === null || typeof out !== 'object') return out;
  // Walk (creating on `set`) to the parent container of the last segment.
  let parent: unknown = out;
  for (let i = 0; i < segs.length; i++) {
    const seg = segs[i];
    let next = (parent as Record<string, unknown>)[seg];
    if (next === null || typeof next !== 'object') {
      if (op !== 'set') return out;
      next = isIdx(segs[i + 1] ?? last) ? [] : {};
      (parent as Record<string, unknown>)[seg] = next;
    }
    parent = next;
  }
  if (parent === null || typeof parent !== 'object') return out;
  if (Array.isArray(parent)) {
    const i = Number(last);
    if (!isIdx(last)) return out;
    if (op === 'set') parent[i] = value;
    else if (op === 'unset' && i < parent.length) parent.splice(i, 1);
    // `rename` inside an array has no meaning — a no-op.
    return out;
  }
  const o = parent as Record<string, unknown>;
  if (op === 'set') o[last] = value;
  else if (op === 'unset') delete o[last];
  else if (op === 'rename' && newPath && last in o) {
    const to = newPath.split('.').pop()!;
    if (to !== last) {
      // Rebuild so the renamed field keeps its position (key order matters to
      // the reader of a JSON column).
      const rebuilt: Record<string, unknown> = {};
      for (const [k, v] of Object.entries(o)) rebuilt[k === last ? to : k] = v;
      for (const k of Object.keys(o)) delete o[k];
      Object.assign(o, rebuilt);
    }
  }
  return out;
}

/** The typed-editor kind a stored value pre-selects: BSON sentinels map to
 *  their type, plain JSON scalars to theirs, containers (and the sentinels the
 *  editor has no dedicated input for — UUID, binary, timestamp) to raw JSON. */
export function valueKind(v: unknown): TypedKind {
  if (v === null || v === undefined) return 'null';
  switch (typeof v) {
    case 'boolean':
      return 'bool';
    case 'number':
    case 'bigint':
      return 'number';
    case 'string':
      return 'string';
    case 'object': {
      if (Array.isArray(v)) return 'json';
      const keys = Object.keys(v as object);
      if (keys.length === 1) {
        switch (keys[0]) {
          case '$oid':
            return 'objectId';
          case '$date':
            return 'date';
          case '$numberLong':
            return 'long';
          case '$numberDecimal':
            return 'decimal';
        }
      }
      return 'json';
    }
    default:
      return 'string';
  }
}
