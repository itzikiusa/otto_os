// The project tree's git overlay (M / ? letters, dirty-folder dots), keyed the way the TREE
// spells paths. Two spellings meet here:
//  - the daemon reports a repo's path as git's `--show-toplevel`, the REAL path
//    (`/private/var/…` for a macOS temp folder, `/Volumes/x/code/app` behind a symlinked
//    `~/code`), while
//  - the tree lists entries under the root the person opened (`/var/…`, `~/code/app`).
// Every key goes through `gitTreeKey` (one normaliser for both sides), and a repo whose
// toplevel is spelled differently from the project root it was detected for is ALSO keyed
// under that root's spelling (`ideGit.repoByRoot` records which root found which repo).

import type { FileChange } from '../../../lib/api/types';
import { normalizeMacPrivatePath } from '../../../lib/fileUri';

export type IdeGitLetter = 'M' | 'A' | 'D' | 'U' | 'R' | 'C';

export interface IdeTreeGitOverlay {
  files: Map<string, IdeGitLetter>;
  dirtyDirs: Set<string>;
  /** Key prefixes (repo roots, every spelling) whose status is loaded: a path under one with
   *  no entry in `files` is clean, not unknown. */
  loadedRoots: string[];
}

export interface IdeTreeGitRepoInput {
  /** The repo path as the daemon reports it (git's toplevel). */
  path: string;
  /** Project roots this repo was detected for, as the tree spells them. */
  aliases: readonly string[];
  /** Null while the repo's status is not loaded yet. */
  changes: readonly Pick<FileChange, 'path' | 'kind'>[] | null;
}

/** The one path key both the overlay and the tree use: macOS `/private/{var,tmp}` folded,
 *  no trailing slash. */
export function gitTreeKey(p: string): string {
  return normalizeMacPrivatePath(p).replace(/\/+$/, '') || '/';
}

export function changeToLetter(c: Pick<FileChange, 'kind'>): IdeGitLetter {
  if (c.kind === 'conflicted') return 'C';
  if (c.kind === 'untracked') return 'U';
  if (c.kind === 'added') return 'A';
  if (c.kind === 'deleted') return 'D';
  if (c.kind === 'renamed') return 'R';
  return 'M';
}

const LETTER_RANK: Record<IdeGitLetter, number> = { C: 6, D: 5, M: 4, R: 3, A: 2, U: 1 };

const under = (p: string, root: string): boolean => p === root || p.startsWith(`${root}/`);

/** Where the repo's toplevel sits in `alias`'s spelling, or null when `alias` is already the
 *  same spelling (inside, equal to or above the repo). A symlinked alias is a repo root or a
 *  folder inside it: the deepest alias segment named like the repo's folder is the toplevel;
 *  with no such segment the alias itself is taken as the toplevel. */
export function treeAliasRoot(repoPath: string, alias: string): string | null {
  const repoKey = gitTreeKey(repoPath);
  const aliasKey = gitTreeKey(alias);
  if (under(aliasKey, repoKey) || under(repoKey, aliasKey)) return null;
  const base = repoKey.slice(repoKey.lastIndexOf('/') + 1);
  const segs = aliasKey.split('/');
  for (let i = segs.length - 1; i > 0; i--) {
    if (segs[i] === base) return segs.slice(0, i + 1).join('/') || '/';
  }
  return aliasKey;
}

/** Fast lookup maps for the tree, built once per status change (not per row). */
export function buildIdeTreeGitOverlay(repos: readonly IdeTreeGitRepoInput[]): IdeTreeGitOverlay {
  const files = new Map<string, IdeGitLetter>();
  const dirtyDirs = new Set<string>();
  const loadedRoots: string[] = [];
  for (const repo of repos) {
    if (!repo.changes) continue;
    const prefixes = [gitTreeKey(repo.path)];
    for (const a of repo.aliases) {
      const p = treeAliasRoot(repo.path, a);
      if (p && !prefixes.includes(p)) prefixes.push(p);
    }
    for (const root of prefixes) {
      if (!loadedRoots.includes(root)) loadedRoots.push(root);
      for (const ch of repo.changes) {
        const rel = ch.path.replace(/^\/+/, '').replace(/\/+$/, '');
        if (!rel) continue;
        const abs = root === '/' ? `/${rel}` : `${root}/${rel}`;
        const letter = changeToLetter(ch);
        const prev = files.get(abs);
        if (!prev || LETTER_RANK[letter] > LETTER_RANK[prev]) files.set(abs, letter);
        for (let slash = abs.lastIndexOf('/'); slash > root.length; slash = abs.lastIndexOf('/', slash - 1)) {
          dirtyDirs.add(abs.slice(0, slash));
        }
        dirtyDirs.add(root);
      }
    }
  }
  return { files, dirtyDirs, loadedRoots };
}

/** The tree's letter for `absPath`: a letter, null (clean, status loaded) or undefined (no
 *  loaded repo status covers it — the caller may fall back to the listing's own mark). */
export function ideTreeGitLetter(overlay: IdeTreeGitOverlay, absPath: string): IdeGitLetter | null | undefined {
  const key = gitTreeKey(absPath);
  const hit = overlay.files.get(key);
  if (hit) return hit;
  return overlay.loadedRoots.some((r) => under(key, r)) ? null : undefined;
}

export function ideTreeDirIsDirty(overlay: IdeTreeGitOverlay, absPath: string): boolean {
  return overlay.dirtyDirs.has(gitTreeKey(absPath));
}
