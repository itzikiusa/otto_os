// The new-thread stage's pure rules (NewThreadStage.svelte, NewThreadMetaRow,
// newThreadMenus.ts): which folder a new thread starts in until the person
// picks one, what a folder is called, which open threads already work in a
// checkout, and the one caution next to the access chip. Import-free (types
// only), unit-tested in unit/threadsStage.test.ts.
import type { RuntimeAccessMode } from '../../lib/api/types';

const norm = (p: string | null | undefined): string => (p ?? '').trim().replace(/\/+$/, '');

/** macOS reports temp and var folders under `/private`; the same folder must
 *  not show twice (a workspace root as `/var/…`, its repo as `/private/var/…`). */
export function canonicalPath(p: string | null | undefined): string {
  return norm(p).replace(/^\/private(?=\/(?:var|tmp|etc)(?:\/|$))/, '');
}

const inside = (path: string, root: string): boolean => {
  const p = canonicalPath(path);
  const r = canonicalPath(root);
  return !!p && !!r && (p === r || p.startsWith(`${r}/`));
};

interface RepoLike {
  name: string;
  path: string;
}
interface WorkspaceLike {
  name: string;
  root_path: string;
}

/** Where the folder came from: `last` = the project last used in this
 *  workspace, `repo` = one of its repos, `workspace` = its root, `home` = no
 *  workspace or nothing better (shown as "Home", never silently). */
export type StageFolderKind = 'last' | 'repo' | 'workspace' | 'home';

export interface StageFolderInput {
  /** The open workspace; null while it loads, or with none / Scratch. */
  workspace: WorkspaceLike | null;
  /** The open workspace's repos. */
  repos: readonly RepoLike[];
  /** Folders of this person's threads, most recent first. */
  recentCwds: readonly string[];
  /** The folder of the last thread started in this workspace (or Scratch). */
  lastProject: string | null;
  homePath: string;
}

/** The folder a new thread starts in until the person picks one. Re-derived
 *  whenever its inputs change, so a workspace that loads after the stage opens
 *  still wins over the home folder. */
export function defaultStageFolder(i: StageFolderInput): { path: string; kind: StageFolderKind } {
  const home = norm(i.homePath) || '~';
  const last = norm(i.lastProject);
  if (!i.workspace) {
    return last && last !== home ? { path: last, kind: 'last' } : { path: home, kind: 'home' };
  }
  const root = norm(i.workspace.root_path);
  const ofWs = (p: string): boolean => (!!root && root !== home && inside(p, root)) || i.repos.some((r) => inside(p, r.path));
  if (last && ofWs(last)) return { path: last, kind: 'last' };
  for (const cwd of i.recentCwds) {
    const repo = deepestRepo(cwd, i.repos);
    if (repo) return { path: norm(repo.path), kind: 'repo' };
  }
  // The workspace root when it IS a repo, else its first repo, else the root.
  const rootRepo = root ? i.repos.find((r) => canonicalPath(r.path) === canonicalPath(root)) : undefined;
  const first = rootRepo ?? i.repos[0];
  if (first) return { path: norm(first.path), kind: 'repo' };
  if (root) return { path: root, kind: root === home ? 'home' : 'workspace' };
  return { path: home, kind: 'home' };
}

function deepestRepo<R extends RepoLike>(cwd: string, repos: readonly R[]): R | null {
  let best: R | null = null;
  for (const r of repos) {
    if (inside(cwd, r.path) && (!best || canonicalPath(r.path).length > canonicalPath(best.path).length)) best = r;
  }
  return best;
}

export interface FolderNames {
  homePath: string;
  repos: readonly RepoLike[];
  workspaces: readonly WorkspaceLike[];
}

/** A folder's name for people: "Home" for the home folder, a registered
 *  repo's name, then a workspace's name, then the folder's own name. */
export function folderLabel(path: string | null | undefined, n: FolderNames): string {
  const p = canonicalPath(path);
  const home = canonicalPath(n.homePath);
  if (!p || p === '~' || (home && p === home)) return 'Home';
  const repo = n.repos.find((r) => canonicalPath(r.path) === p);
  if (repo?.name.trim()) return repo.name.trim();
  const w = n.workspaces.find((x) => canonicalPath(x.root_path) === p);
  if (w?.name.trim()) return w.name.trim();
  return p.split('/').filter(Boolean).pop() ?? p;
}

/** A path's parent for a dim menu hint, kept short from the START so the
 *  distinguishing tail stays (`…/code/billing` → `…/code`). */
export function parentHint(path: string, homePath: string, max = 22): string {
  const p = canonicalPath(path);
  const home = canonicalPath(homePath);
  let parent = p.slice(0, Math.max(0, p.lastIndexOf('/'))) || '/';
  if (home && (parent === home || parent.startsWith(`${home}/`))) parent = `~${parent.slice(home.length)}`;
  return parent.length > max ? `…${parent.slice(parent.length - (max - 1))}` : parent;
}

export interface FolderCandidate {
  path: string;
  label: string;
  /** The parent folder, shown when it tells two same-named rows apart. */
  hint?: string;
}

/** Menu rows for folders: one row per folder (the `/private` twin folded in),
 *  and where two rows share a name, the parent says which is which. */
export function folderCandidates(
  entries: readonly { path: string | null | undefined; label?: string }[],
  n: FolderNames,
  limit = 10,
): FolderCandidate[] {
  const out: FolderCandidate[] = [];
  const seen = new Set<string>();
  for (const e of entries) {
    const path = norm(e.path);
    const key = canonicalPath(path);
    if (!key || seen.has(key)) continue;
    seen.add(key);
    out.push({ path, label: e.label?.trim() || folderLabel(path, n) });
    if (out.length >= limit) break;
  }
  const count = new Map<string, number>();
  for (const c of out) count.set(c.label.toLowerCase(), (count.get(c.label.toLowerCase()) ?? 0) + 1);
  return out.map((c) => ((count.get(c.label.toLowerCase()) ?? 0) > 1 ? { ...c, hint: parentHint(c.path, n.homePath) } : c));
}

interface ThreadRowLike {
  id: string;
  cwd: string;
  status: string;
  meta?: Record<string, unknown> | null;
}

/** Statuses with a live agent that may edit files. A dormant thread's agent
 *  is stopped (it wakes only on a message), and a closed one is in History. */
const LIVE = new Set(['starting', 'idle', 'working', 'awaiting_permission']);

/** Open, writable threads whose folder is in `checkout` (a repo or worktree
 *  root, or the plain folder): a second agent there edits the same files. */
export function threadsInCheckout(rows: readonly ThreadRowLike[], checkout: string | null | undefined, exceptId?: string | null): ThreadRowLike[] {
  const root = canonicalPath(checkout);
  if (!root) return [];
  return rows.filter((r) => r.id !== exceptId && LIVE.has(r.status) && r.meta?.read_only !== true && inside(r.cwd, root));
}

/** The muted line beside the access chip when Full access meets a folder git
 *  can't undo changes in; null otherwise. */
export function accessCaution(mode: RuntimeAccessMode | null, folder: { home: boolean; repo: boolean | null }): string | null {
  if (mode !== 'full-access') return null;
  if (folder.home) return 'Full access in your home folder: the agent can change any file without asking.';
  if (folder.repo === false) return 'Full access outside a git repository: changes can’t be undone with git.';
  return null;
}

/** The access a new thread starts in, as shown on the stage and sent with
 *  the start: the person's pick, else the saved default (Settings › Providers
 *  › "Skip permission prompts" on = Full access), else Supervised. */
export function stageAccess(picked: RuntimeAccessMode | null, skipPermissions: boolean | null): RuntimeAccessMode {
  if (picked) return picked;
  return skipPermissions ? 'full-access' : 'approval-required';
}
