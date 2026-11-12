// PR-mode helpers for DiffViewer (the T3-style Code tab): persisted view
// preferences and the file-header status glyph. Kept out of DiffViewer.svelte
// so the renderer stays focused on the windowed row model.
import type { FileDiff } from '../../lib/api/types';

/** Persisted PR diff preference keys. */
export const PREF_TREE = 'otto_pr_tree';
export const PREF_HIDE_TESTS = 'otto_pr_hide_tests';
export const PREF_WRAP = 'otto_diff_wrap';

/** A boolean preference ('1' / '0'); `dflt` when unset or storage is blocked. */
export function readPref(key: string, dflt: boolean): boolean {
  try {
    const v = localStorage.getItem(key);
    return v === null ? dflt : v === '1';
  } catch {
    return dflt;
  }
}

export function writePref(key: string, v: boolean): void {
  try {
    localStorage.setItem(key, v ? '1' : '0');
  } catch {
    /* blocked storage: the choice just doesn't persist */
  }
}

export type FileStatusKind = 'added' | 'deleted' | 'renamed' | 'modified';

/** The file-header status glyph. The glyph shape and the spoken label carry
 *  the meaning; the token colour only reinforces it. */
export function fileStatus(f: Pick<FileDiff, 'status' | 'old_path'>): { kind: FileStatusKind; glyph: string; label: string } {
  const st = f.status ?? (f.old_path ? 'renamed' : null);
  if (st === 'added') return { kind: 'added', glyph: '+', label: 'Added' };
  if (st === 'deleted') return { kind: 'deleted', glyph: '−', label: 'Deleted' };
  if (st === 'renamed' || st === 'copied') return { kind: 'renamed', glyph: '→', label: st === 'copied' ? 'Copied' : 'Renamed' };
  return { kind: 'modified', glyph: '•', label: 'Modified' };
}

/** Directory node of the file navigator tree. */
export interface NavDir {
  /** Compressed display label, e.g. "apps/sinatra/src". */
  label: string;
  /** Full prefix path (unique key), e.g. "apps/sinatra/src". */
  path: string;
  dirs: NavDir[];
  files: FileDiff[];
}

/** Group files by directory (GitHub-style), compressing single-child chains
 *  ("apps" → "sinatra" → "src" renders once as "apps/sinatra/src"); dirs
 *  first, then files, both A→Z. */
export function buildNavTree(files: readonly FileDiff[]): NavDir {
  interface Node {
    dirs: Map<string, Node>;
    files: FileDiff[];
  }
  const root: Node = { dirs: new Map(), files: [] };
  for (const f of files) {
    const parts = f.path.split('/');
    let cur = root;
    for (const p of parts.slice(0, -1)) {
      let next = cur.dirs.get(p);
      if (!next) {
        next = { dirs: new Map(), files: [] };
        cur.dirs.set(p, next);
      }
      cur = next;
    }
    cur.files.push(f);
  }
  const emit = (node: Node, label: string, path: string): NavDir => {
    let n = node;
    let lbl = label;
    let pth = path;
    while (n.files.length === 0 && n.dirs.size === 1) {
      const [k, child] = [...n.dirs.entries()][0];
      lbl = lbl === '' ? k : `${lbl}/${k}`;
      pth = pth === '' ? k : `${pth}/${k}`;
      n = child;
    }
    const dirs = [...n.dirs.entries()]
      .sort(([a], [b]) => a.localeCompare(b))
      .map(([k, child]) => emit(child, k, pth === '' ? k : `${pth}/${k}`));
    const fs = [...n.files].sort((a, b) => a.path.localeCompare(b.path));
    return { label: lbl, path: pth, dirs, files: fs };
  };
  return emit(root, '', '');
}

/** The first file after `current` in `order` (wrapping) that is not viewed —
 *  "Next unviewed" in the Files panel and the phone's file stepper. */
export function nextUnviewed(order: readonly string[], viewed: ReadonlySet<string>, current: string | null): string | null {
  const at = current ? order.indexOf(current) : -1;
  for (let k = 1; k <= order.length; k++) {
    const f = order[(at + k) % order.length];
    if (f !== undefined && !viewed.has(f)) return f;
  }
  return null;
}
