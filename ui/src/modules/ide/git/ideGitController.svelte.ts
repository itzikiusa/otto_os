// Lightweight reactive adapter connecting the IDE's multi-root project folders
// to Otto's existing `git` store and `gitApi` (`ui/src/lib/api/git.ts`).
// Zero reinvented git state; zero idle polling (Otto's FSEvents watcher pushes
// `repo_status_changed` over WebSocket automatically).

import { api, ApiError } from '../../../lib/api/client';
import { gitApi } from '../../../lib/api/git';
import type {
  DiscardReq,
  FileChange,
  Repo,
  RepoResolveResp,
  RepoStatusResp,
} from '../../../lib/api/types';
import { loadErrorText } from '../../../lib/loadError';
import { git } from '../../../lib/stores/git.svelte';
import { SCRATCH_WORKSPACE_ID, ws } from '../../../lib/stores/workspace.svelte';
import { toastError } from '../../../lib/toastError';
import {
  ide,
  ideBasename,
  type IdeBottomTab,
  type IdeFileReadResp,
  type IdeProjectRoot,
} from '../ideStore.svelte';

export type IdeGitLetter = 'M' | 'A' | 'D' | 'U' | 'R' | 'C';

export interface IdeGitHistoryTab {
  key: string;
  absPath: string;
  relPath: string;
  name: string;
  repoId: string;
  rootPath: string;
  initialMode: 'history' | 'blame';
  modeSeq: number;
}

function normSlash(p: string): string {
  return p.replace(/\/+$/, '') || '/';
}

function changeToLetter(c: FileChange): IdeGitLetter {
  if (c.kind === 'conflicted') return 'C';
  if (c.kind === 'untracked') return 'U';
  if (c.kind === 'added') return 'A';
  if (c.kind === 'deleted') return 'D';
  if (c.kind === 'renamed') return 'R';
  return 'M';
}

const LETTER_RANK: Record<IdeGitLetter, number> = {
  C: 6,
  D: 5,
  M: 4,
  R: 3,
  A: 2,
  U: 1,
};

class IdeGitController {
  repoByRoot = $state<Record<string, Repo>>({});
  selectedRootPath = $state<string | null>(null);
  detectingCount = $state(0);
  activeSubTab = $state<string>('changes');
  historyTabs = $state<IdeGitHistoryTab[]>([]);
  focusedChangePath = $state<string | null>(null);
  focusCommitSeq = $state(0);

  private inFlightRoots = new Set<string>();
  private nonRepoRoots = new Set<string>();
  private activeProbes = 0;
  private probeQueue: Array<() => void> = [];
  private histModeSeq = 0;

  /** All unique detected Otto `Repo` objects across attached IDE project folders. */
  readonly repos = $derived.by((): Repo[] => {
    const seen = new Set<string>();
    const out: Repo[] = [];
    for (let i = 0; i < ide.projectRoots.length; i++) {
      const r = this.repoByRoot[normSlash(ide.projectRoots[i].path)];
      if (r && !seen.has(r.id)) {
        seen.add(r.id);
        out.push(r);
      }
    }
    return out;
  });

  /** Active project root selected in the IDE Git tool panel (defaults to active file's root). */
  readonly activeRoot = $derived.by((): IdeProjectRoot | null => {
    const roots = ide.projectRoots;
    if (roots.length === 0) return null;
    if (this.selectedRootPath) {
      const picked = roots.find((r) => normSlash(r.path) === normSlash(this.selectedRootPath!));
      if (picked) return picked;
    }
    const curFile = ide.activePath;
    if (curFile) {
      const matched = this.rootContainingPath(curFile);
      if (matched) return matched;
    }
    return roots[0] ?? null;
  });

  readonly activeRepo = $derived.by((): Repo | null => {
    const root = this.activeRoot;
    if (!root) return this.repos[0] ?? null;
    return this.repoByRoot[normSlash(root.path)] ?? this.repos[0] ?? null;
  });

  readonly activeStatus = $derived.by((): RepoStatusResp | null => {
    const r = this.activeRepo;
    return r ? (git.statusById[r.id] ?? null) : null;
  });

  /** Total changed files across detected project repos (drives the Git dock tab badge). */
  readonly totalChangeCount = $derived.by((): number => {
    let sum = 0;
    for (let i = 0; i < this.repos.length; i++) {
      const st = git.statusById[this.repos[i].id];
      if (st) sum += st.changes.length;
    }
    return sum;
  });

  /** Fast O(1) lookup maps for `IdeProjectTree.svelte`: file status letters + dirty parent folders. */
  readonly treeGitOverlay = $derived.by(
    (): { files: Map<string, IdeGitLetter>; dirtyDirs: Set<string> } => {
      const files = new Map<string, IdeGitLetter>();
      const dirtyDirs = new Set<string>();
      const seenRepos = new Set<string>();

      for (const [, repo] of Object.entries(this.repoByRoot)) {
        if (!repo || seenRepos.has(repo.id)) continue;
        seenRepos.add(repo.id);
        const st = git.statusById[repo.id];
        if (!st || st.changes.length === 0) continue;
        const repoRoot = normSlash(repo.path);
        for (let i = 0; i < st.changes.length; i++) {
          const ch = st.changes[i];
          const rel = ch.path.replace(/^\/+/, '').replace(/\/+$/, '');
          if (!rel) continue;
          const abs = `${repoRoot}/${rel}`;
          const letter = changeToLetter(ch);
          const prev = files.get(abs);
          if (!prev || LETTER_RANK[letter] > LETTER_RANK[prev]) files.set(abs, letter);

          let slash = abs.lastIndexOf('/');
          while (slash >= repoRoot.length) {
            const dir = abs.slice(0, slash);
            if (dir) dirtyDirs.add(dir);
            if (dir.length <= repoRoot.length) break;
            slash = dir.lastIndexOf('/');
          }
        }
      }
      return { files, dirtyDirs };
    },
  );

  rootContainingPath(absPath: string): IdeProjectRoot | null {
    const clean = normSlash(absPath);
    let best: IdeProjectRoot | null = null;
    for (let i = 0; i < ide.projectRoots.length; i++) {
      const r = ide.projectRoots[i];
      const rp = normSlash(r.path);
      if (clean === rp || clean.startsWith(`${rp}/`)) {
        if (!best || rp.length > normSlash(best.path).length) best = r;
      }
    }
    return best;
  }

  async resolveRepoForPath(absPath: string): Promise<Repo | null> {
    const root = this.rootContainingPath(absPath) ?? this.activeRoot;
    if (!root) return this.activeRepo;
    return (await this.ensureRootDetected(root.path)) ?? this.activeRepo;
  }

  /** Sync active/expanded roots without flooding 35+ requests on cold multi-repo project open. */
  syncVisibleRoots(opts: { probeAll?: boolean } = {}): void {
    if (ide.target.kind !== 'local' || !ide.target.root) return;
    const roots = ide.projectRoots;
    const activeFile = ide.activePath;
    let secondaryBudget = opts.probeAll ? 64 : 4;

    for (let i = 0; i < roots.length; i++) {
      const root = roots[i];
      const norm = normSlash(root.path);
      if (this.repoByRoot[norm] || this.nonRepoRoots.has(norm) || this.inFlightRoots.has(norm)) {
        continue;
      }
      const containsActive =
        Boolean(activeFile) && (activeFile === norm || activeFile!.startsWith(`${norm}/`));
      const isExpanded = Boolean(ide.expandedDirs[norm]);
      if (root.isPrimary || containsActive) {
        void this.ensureRootDetected(norm);
      } else if ((opts.probeAll || isExpanded) && secondaryBudget > 0) {
        secondaryBudget -= 1;
        void this.ensureRootDetected(norm);
      }
    }
  }

  selectRoot(rootPath: string): void {
    const clean = normSlash(rootPath);
    this.selectedRootPath = clean;
    void this.ensureRootDetected(clean, true);
  }

  /** Open or focus a closable `History: <filename> ×` tab in the IDE Git tool window. */
  async openFileHistoryTab(absPath: string, mode: 'history' | 'blame' = 'history'): Promise<void> {
    const cleanAbs = normSlash(ide.resolvePath(absPath));
    const root = this.rootContainingPath(cleanAbs) ?? this.activeRoot;
    if (root) this.selectedRootPath = normSlash(root.path);
    const repo = root ? await this.ensureRootDetected(root.path) : this.activeRepo;
    if (!repo) {
      ide.openBottomTab('git' as IdeBottomTab);
      return;
    }
    const relPath = this.repoRelativePath(repo, cleanAbs) || ideBasename(cleanAbs);
    const key = `hist:${cleanAbs}`;
    this.histModeSeq += 1;
    const entry: IdeGitHistoryTab = {
      key,
      absPath: cleanAbs,
      relPath,
      name: ideBasename(cleanAbs),
      repoId: repo.id,
      rootPath: root ? normSlash(root.path) : normSlash(repo.path),
      initialMode: mode,
      modeSeq: this.histModeSeq,
    };
    const existingIdx = this.historyTabs.findIndex((t) => t.key === key);
    if (existingIdx >= 0) {
      this.historyTabs = this.historyTabs.map((t, i) => (i === existingIdx ? entry : t));
    } else {
      this.historyTabs = [...this.historyTabs.slice(-7), entry];
    }
    this.activeSubTab = key;
    ide.openBottomTab('git' as IdeBottomTab);
  }

  closeFileHistoryTab(key: string): void {
    const idx = this.historyTabs.findIndex((t) => t.key === key);
    if (idx < 0) return;
    const nextTabs = this.historyTabs.filter((t) => t.key !== key);
    this.historyTabs = nextTabs;
    if (this.activeSubTab === key) {
      const neighbor = nextTabs[Math.min(idx, nextTabs.length - 1)];
      this.activeSubTab = neighbor ? neighbor.key : 'changes';
    }
  }

  /** Open `Local Changes` focused on a specific file (and optionally focus the commit input). */
  async openChangesForFile(absPath: string, opts: { focusCommit?: boolean } = {}): Promise<void> {
    const cleanAbs = normSlash(ide.resolvePath(absPath));
    const root = this.rootContainingPath(cleanAbs) ?? this.activeRoot;
    if (root) this.selectedRootPath = normSlash(root.path);
    const repo = root ? await this.ensureRootDetected(root.path) : this.activeRepo;
    if (repo) {
      const rel = this.repoRelativePath(repo, cleanAbs);
      if (rel) this.focusedChangePath = rel;
    }
    if (opts.focusCommit) this.focusCommitSeq += 1;
    this.activeSubTab = 'changes';
    ide.openBottomTab('git' as IdeBottomTab);
  }

  async ensureRootDetected(rawRootPath: string, force = false): Promise<Repo | null> {
    if (ide.target.kind !== 'local' || !rawRootPath) return null;
    const rootPath = normSlash(rawRootPath);
    if (!force && this.repoByRoot[rootPath]) {
      git.ensureStatus(this.repoByRoot[rootPath].id);
      return this.repoByRoot[rootPath];
    }
    if (!force && (this.nonRepoRoots.has(rootPath) || this.inFlightRoots.has(rootPath))) {
      return this.repoByRoot[rootPath] ?? null;
    }

    const known = [...git.repos, ...git.allRepos].find((r) => {
      const rp = normSlash(r.path);
      return rootPath === rp || rootPath.startsWith(`${rp}/`);
    });
    if (known) {
      this.repoByRoot = { ...this.repoByRoot, [rootPath]: known };
      git.ensureStatus(known.id);
      return known;
    }

    this.inFlightRoots.add(rootPath);
    this.detectingCount += 1;
    try {
      const detected = await this.enqueueProbe(async () => {
        const wsId =
          ide.target.workspace_id ||
          ws.current?.id ||
          ws.workspaces.find((w) => w.id !== SCRATCH_WORKSPACE_ID)?.id ||
          ws.workspaces[0]?.id ||
          null;
        if (wsId) {
          try {
            return await api.bg.post<Repo>(`/workspaces/${encodeURIComponent(wsId)}/repos/detect`, {
              path: rootPath,
            });
          } catch {
            /* fall through to resolve */
          }
        }
        try {
          const resolved = await api.bg.get<RepoResolveResp>(
            `/git/repos/resolve?ref=${encodeURIComponent(rootPath)}`,
          );
          const r = resolved.repo;
          return {
            id: r.id,
            workspace_id: r.workspace_id,
            name: r.name || ideBasename(r.path),
            path: r.path,
            remote_url: r.remote_url,
            provider: r.provider,
            git_account_id: null,
            created_at: '',
          };
        } catch {
          return null;
        }
      });

      if (detected) {
        this.nonRepoRoots.delete(rootPath);
        this.repoByRoot = { ...this.repoByRoot, [rootPath]: detected };
        void git.refreshStatus(detected.id);
        return detected;
      }
      this.nonRepoRoots.add(rootPath);
      return null;
    } finally {
      this.inFlightRoots.delete(rootPath);
      this.detectingCount = Math.max(0, this.detectingCount - 1);
    }
  }

  private enqueueProbe<T>(fn: () => Promise<T>): Promise<T> {
    return new Promise<T>((resolve, reject) => {
      const run = (): void => {
        this.activeProbes += 1;
        fn()
          .then(resolve, reject)
          .finally(() => {
            this.activeProbes -= 1;
            const next = this.probeQueue.shift();
            if (next) next();
          });
      };
      if (this.activeProbes < 3) run();
      else this.probeQueue.push(run);
    });
  }

  applyStatus(repoId: string, status: RepoStatusResp): void {
    git.setStatus(repoId, status);
  }

  async refreshActive(): Promise<void> {
    const root = this.activeRoot;
    if (root && !this.repoByRoot[normSlash(root.path)]) {
      await this.ensureRootDetected(root.path, true);
    }
    const repo = this.activeRepo;
    if (repo) await git.refreshStatus(repo.id);
  }

  async stagePaths(repoId: string, paths: string[], stage: boolean): Promise<RepoStatusResp> {
    const s = await gitApi.stage(repoId, paths, stage);
    git.setStatus(repoId, s);
    return s;
  }

  async discardPaths(repoId: string, paths: string[], keepStaged: boolean): Promise<RepoStatusResp> {
    const req: DiscardReq = keepStaged ? { paths, keep_staged: true } : { paths };
    const s = await api.post<RepoStatusResp>(`/repos/${encodeURIComponent(repoId)}/discard`, req);
    git.setStatus(repoId, s);
    const repo = this.repos.find((r) => r.id === repoId);
    if (repo) {
      const base = normSlash(repo.path);
      await ide.loadTree();
      for (let i = 0; i < paths.length; i++) {
        await this.reloadIdeTabIfOpen(`${base}/${paths[i].replace(/^\/+/, '')}`);
      }
    }
    return s;
  }

  /** Refresh an open IDE tab in place from disk (or close it if deleted by discard). */
  async reloadIdeTabIfOpen(absPath: string): Promise<void> {
    const clean = normSlash(ide.resolvePath(absPath));
    const existing = ide.tabs.find((t) => normSlash(t.path) === clean);
    if (!existing) return;
    try {
      const root = this.rootContainingPath(clean)?.path || ide.target.root;
      const fresh = await api.get<IdeFileReadResp>(
        `/ide/fs/file?${ide.targetQuery({ root, path: clean })}`,
      );
      if (ide.activePath === existing.path) {
        await ide.applyAgentEdit(existing.path, fresh.content);
      } else {
        ide.updateBuffer(existing.path, fresh.content);
      }
      existing.content = fresh.content;
      existing.savedContent = fresh.content;
      existing.dirty = false;
      existing.sha256 = fresh.sha256;
    } catch (e) {
      if (e instanceof ApiError && e.status === 404) {
        existing.dirty = false;
        existing.savedContent = existing.content;
        await ide.closeTab(existing.path);
      } else {
        existing.error = loadErrorText(e);
        toastError(`Couldn’t reload ${ideBasename(existing.path)}`, e);
      }
    }
  }

  repoRelativePath(repo: Repo, absPath: string): string | null {
    const rp = normSlash(repo.path);
    const ap = normSlash(absPath);
    if (ap === rp) return '';
    if (ap.startsWith(`${rp}/`)) return ap.slice(rp.length + 1);
    return null;
  }
}

export const ideGit = new IdeGitController();
