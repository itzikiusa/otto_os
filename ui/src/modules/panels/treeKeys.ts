// The side-panel file tree's keyboard model (APG tree pattern, as
// lib/design/scene3d/Hierarchy.svelte does it), pure so `node --test` covers
// it (ui/unit/p06-fileTreeKeys.test.ts). Rows are the flattened VISIBLE rows,
// in order; the tree is windowed, so a target row may not be mounted yet —
// the caller scrolls it into the window before focusing it.

export interface TreeKeyRow {
  depth: number;
  isDir: boolean;
  open: boolean;
}

export type TreeKeyAction = { kind: 'none' } | { kind: 'toggle' } | { kind: 'focus'; index: number };

const NONE: TreeKeyAction = { kind: 'none' };
const focus = (index: number): TreeKeyAction => ({ kind: 'focus', index });

/** What `key` on row `i` does. ↑/↓ step, Home/End jump; → opens a closed
 *  folder or moves into an open one's first child; ← closes an open folder or
 *  moves to the parent. In RTL ← and → swap. */
export function treeKeyAction(key: string, rows: readonly TreeKeyRow[], i: number, rtl = false): TreeKeyAction {
  const row = rows[i];
  if (!row) return NONE;
  const k = rtl && key === 'ArrowRight' ? 'ArrowLeft' : rtl && key === 'ArrowLeft' ? 'ArrowRight' : key;
  switch (k) {
    case 'ArrowDown':
      return i + 1 < rows.length ? focus(i + 1) : NONE;
    case 'ArrowUp':
      return i > 0 ? focus(i - 1) : NONE;
    case 'Home':
      return focus(0);
    case 'End':
      return focus(rows.length - 1);
    case 'ArrowRight':
      if (!row.isDir) return NONE;
      if (!row.open) return { kind: 'toggle' };
      return (rows[i + 1]?.depth ?? -1) > row.depth ? focus(i + 1) : NONE;
    case 'ArrowLeft':
      if (row.isDir && row.open) return { kind: 'toggle' };
      for (let j = i - 1; j >= 0; j--) if (rows[j].depth < row.depth) return focus(j);
      return NONE;
    default:
      return NONE;
  }
}

/** The row that holds the tree's ONE Tab stop: the active row while it is
 *  MOUNTED (inside `[start, end)` of the window), else the first mounted row —
 *  so scrolling the active row out of the window never leaves the tree with no
 *  focusable row at all. */
export function tabRowIn(paths: readonly string[], active: string | null, start: number, end: number): string | null {
  const i = active === null ? -1 : paths.indexOf(active);
  if (i >= start && i < end) return active;
  return paths[start] ?? null;
}
