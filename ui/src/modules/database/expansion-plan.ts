// Expansion state for the nested-document views (Vertical / JSON). Phase 0
// ships only the shape + the two pure path helpers; the planner
// (`planExpansion`) and the node-budget machinery land with the Vertical
// rendering work. Nothing here is Svelte — ResultsGrid owns the `$state`.
import { bsonScalar } from './bson';

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

export function newExpansionState(): ExpansionState {
  return { overrides: new Map(), mode: 'budget', budget: 400, version: 0 };
}

/** Normalise a dotted path's array indices to `[]` (`items.3.sku` →
 *  `items.[].sku`) so one override covers every element of an array. */
export function stickyKey(path: string): string {
  return path.replace(/(^|\.)\d+(?=\.|$)/g, '$1[]');
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
