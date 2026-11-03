// The WIP panel's file-list model (split out of WipPanel.svelte): the folder
// tree, the flat list, the visible-row walk the windowed sections render, and
// the status letters. Pure — no Svelte state — so the row shapes are
// unit-tested without mounting the panel.
import type { FileChange } from '../../../lib/api/types';

/** One section of the panel. Untracked files are their own section (git
 *  can't diff them hunk by hunk, and "Discard" deletes them), but they stage
 *  and discard exactly like the Unstaged list (`keep_staged`). */
export type WipSection = 'unstaged' | 'untracked' | 'staged';

export type TFile = { type: 'file'; name: string; dir: string; change: FileChange };
export type TFolder = {
  type: 'folder';
  name: string;
  path: string;
  children: TNode[];
  files: FileChange[];
};
export type TNode = TFile | TFolder;
/** A VISIBLE row (collapsed subtrees skipped): what each section windows. */
export type Row = { node: TNode; depth: number; key: string };

/** `a/b/c.ts` → `{ name: 'c.ts', dir: 'a/b' }` (dir '' at the root). */
export function splitPath(path: string): { name: string; dir: string } {
  const i = path.lastIndexOf('/');
  return i === -1 ? { name: path, dir: '' } : { name: path.slice(i + 1), dir: path.slice(0, i) };
}

/** git's status letter per kind (VS Code / GitHub Desktop convention). */
export const KIND_LETTER: Record<FileChange['kind'], string> = {
  modified: 'M',
  added: 'A',
  deleted: 'D',
  renamed: 'R',
  untracked: 'U',
  conflicted: '!',
};
/** The word behind the letter — the badge's tooltip and accessible name, so
 *  colour and letter are never the only signal. */
export const KIND_WORD: Record<FileChange['kind'], string> = {
  modified: 'Modified',
  added: 'Added',
  deleted: 'Deleted',
  renamed: 'Renamed',
  untracked: 'Untracked',
  conflicted: 'Conflicted',
};

// One collator for every compare: `localeCompare` re-resolves the locale
// per call, which dominated sorting a 20k-file tree.
const collator = new Intl.Collator();
function sortLevel(nodes: TNode[]): void {
  nodes.sort((a, b) => {
    if (a.type !== b.type) return a.type === 'folder' ? -1 : 1;
    return collator.compare(a.name, b.name);
  });
  for (const n of nodes) if (n.type === 'folder') sortLevel(n.children);
}

export function buildTree(changes: FileChange[]): TNode[] {
  const rootChildren: TNode[] = [];
  const folders = new Map<string, TFolder>();
  for (const c of changes) {
    const parts = c.path.split('/');
    let children = rootChildren;
    let prefix = '';
    for (let i = 0; i < parts.length - 1; i++) {
      prefix = prefix ? `${prefix}/${parts[i]}` : parts[i];
      let folder = folders.get(prefix);
      if (!folder) {
        folder = { type: 'folder', name: parts[i], path: prefix, children: [], files: [] };
        folders.set(prefix, folder);
        children.push(folder);
      }
      folder.files.push(c);
      children = folder.children;
    }
    children.push({ type: 'file', name: parts[parts.length - 1], dir: prefix, change: c });
  }
  sortLevel(rootChildren);
  return rootChildren;
}

/** A folder the user never toggled starts collapsed when its subtree holds
 *  more than this many changes — an un-ignored `node_modules/` or `dist/`
 *  lists tens of thousands of files (20k rows ≈ 200k DOM nodes ≈ 1.5 s to
 *  mount); its count stays visible. */
export const AUTO_COLLAPSE_FILES = 200;

/** The tree as the flat list of VISIBLE rows, so each section mounts a
 *  bounded slice instead of recursing everything. */
export function treeRows(nodes: TNode[], collapsed: (n: TFolder) => boolean): Row[] {
  const out: Row[] = [];
  const walk = (level: TNode[], depth: number) => {
    for (const n of level) {
      if (n.type === 'folder') {
        out.push({ node: n, depth, key: `d:${n.path}` });
        if (!collapsed(n)) walk(n.children, depth + 1);
      } else {
        out.push({ node: n, depth, key: `f:${n.change.path}` });
      }
    }
  };
  walk(nodes, 0);
  return out;
}

/** List view: every file at depth 0, sorted by file name, then directory —
 *  the filename is what the eye scans for; the directory reads dimmed. */
export function listRows(changes: FileChange[]): Row[] {
  const files: TFile[] = changes.map((c) => ({ type: 'file', ...splitPath(c.path), change: c }));
  files.sort((a, b) => collator.compare(a.name, b.name) || collator.compare(a.dir, b.dir));
  return files.map((f) => ({ node: f, depth: 0, key: `f:${f.change.path}` }));
}

/** `+12 −3` line counts per path, from a `summary=true` diff. */
export type LineCounts = Map<string, { added: number | null; deleted: number | null }>;
