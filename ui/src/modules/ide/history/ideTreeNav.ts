// Pure helpers for the project tree (IdeProjectTree.svelte): keyboard navigation over the
// flattened visible rows (one Tab stop, arrow keys like a native tree) and the filter's
// match set, computed once per query instead of re-scanning subtrees for every row.

import type { IdeFsEntry } from '../store/ideTypes';

/** Row height the tree's virtualiser and its CSS share: comfortable touch targets on a
 *  coarse pointer (phone, tablet), the dense desktop row otherwise. */
export const TREE_ROW_H_FINE = 26;
export const TREE_ROW_H_COARSE = 40;

export function treeRowHeight(coarsePointer: boolean): number {
  return coarsePointer ? TREE_ROW_H_COARSE : TREE_ROW_H_FINE;
}

/** The slice of a flattened row the navigation needs. `loadError` rows (a folder's inline
 *  "Couldn’t load" + Retry) are skipped by the arrow keys. */
export interface TreeNavRow {
  depth: number;
  isDir: boolean;
  expanded: boolean;
  loadError?: string;
}

export type TreeNavAction =
  | { kind: 'focus'; index: number }
  | { kind: 'expand'; index: number }
  | { kind: 'collapse'; index: number }
  | { kind: 'menu'; index: number };

function step(rows: readonly TreeNavRow[], from: number, dir: 1 | -1): number {
  for (let i = from + dir; i >= 0 && i < rows.length; i += dir) if (!rows[i].loadError) return i;
  return from;
}

/**
 * What a key does on row `index` (WAI-ARIA tree pattern): ↑/↓ move, Home/End jump,
 * → expands a closed folder or enters an open one, ← collapses an open folder or goes to
 * the parent, ⇧F10 / the context-menu key open the row's menu. Enter/Space stay with the
 * row's button (open file / toggle folder). Null: not a tree key.
 */
export function treeNavAction(
  rows: readonly TreeNavRow[],
  index: number,
  key: string,
  shiftKey = false,
): TreeNavAction | null {
  const row = rows[index];
  if (!row) return null;
  switch (key) {
    case 'ArrowDown': return { kind: 'focus', index: step(rows, index, 1) };
    case 'ArrowUp': return { kind: 'focus', index: step(rows, index, -1) };
    case 'Home': return { kind: 'focus', index: rows[0]?.loadError ? step(rows, 0, 1) : 0 };
    case 'End': {
      const last = rows.length - 1;
      return { kind: 'focus', index: rows[last]?.loadError ? step(rows, last, -1) : last };
    }
    case 'ArrowRight':
      if (!row.isDir) return null;
      return row.expanded ? { kind: 'focus', index: step(rows, index, 1) } : { kind: 'expand', index };
    case 'ArrowLeft': {
      if (row.isDir && row.expanded) return { kind: 'collapse', index };
      for (let i = index - 1; i >= 0; i--) if (rows[i].depth < row.depth && !rows[i].loadError) return { kind: 'focus', index: i };
      return null;
    }
    case 'ContextMenu': return { kind: 'menu', index };
    case 'F10': return shiftKey ? { kind: 'menu', index } : null;
    default: return null;
  }
}

/**
 * Paths that match the filter `q` themselves or contain a loaded descendant that does:
 * one post-order pass over the loaded tree per query (the old per-row recursive check was
 * O(rows × subtree) on every keystroke). Empty `q`: null (everything matches).
 */
export function treeFilterMatches(
  q: string,
  roots: readonly (readonly IdeFsEntry[])[],
  childrenByDir: Readonly<Record<string, readonly IdeFsEntry[] | undefined>>,
): Set<string> | null {
  const needle = q.trim().toLowerCase();
  if (!needle) return null;
  const hits = new Set<string>();
  const seen = new Set<string>();
  const visit = (entry: IdeFsEntry): boolean => {
    if (seen.has(entry.path)) return hits.has(entry.path);
    seen.add(entry.path);
    let hit = entry.name.toLowerCase().includes(needle);
    const kids = entry.kind === 'dir' ? childrenByDir[entry.path] : undefined;
    if (kids) for (let i = 0; i < kids.length; i++) if (visit(kids[i])) hit = true;
    if (hit) hits.add(entry.path);
    return hit;
  };
  for (const list of roots) for (let i = 0; i < list.length; i++) visit(list[i]);
  return hits;
}
