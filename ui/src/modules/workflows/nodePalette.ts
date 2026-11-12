// The Add-node palette's filtering and grouping, kept pure so it is unit-tested
// without mounting the popover.
import type { NodeTypeSpec } from '../../lib/api/types';

export interface PaletteGroup {
  category: string;
  types: NodeTypeSpec[];
}

/** Node types matching `query` (label, description, category or kind;
 *  case-insensitive, every word must match), grouped by category in the
 *  catalog's own order. */
export function groupNodeTypes(types: readonly NodeTypeSpec[], query: string): PaletteGroup[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  const groups = new Map<string, NodeTypeSpec[]>();
  for (const t of types) {
    const hay = `${t.label} ${t.description} ${t.category} ${t.kind}`.toLowerCase();
    if (!words.every((w) => hay.includes(w))) continue;
    const key = t.category || 'Other';
    const list = groups.get(key);
    if (list) list.push(t);
    else groups.set(key, [t]);
  }
  return [...groups].map(([category, list]) => ({ category, types: list }));
}
