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
import { isBroadNonProjectRootClient, normalizeMacPrivatePath } from '../../../lib/fileUri';
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
import { onIdeTargetReset } from '../store/ideStateSlices.svelte';
import { detectTextEol, normalizeEditorLf } from '../store/ideTypes';

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
  return normalizeMacPrivatePath(p).replace(/\/+$/, '') || '/';
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
  selectedRepoId = $state<string | null>(null);
  detectingCount = $state(0);
  activeSubTab = $state<string>('changes');
  historyTabs = $state<IdeGitHistoryTab[]>([]);
  focusedChangePath = $state<string | null>(null);
  focusCommitSeq = $state(0);

  private inFlightRoots = new Map<string, Promise<Repo | null>>();
  private nonRepoRoots = new Set<string>();
  private lastHandledLiveRev: Record<string, number> = {};
  private activeProbes = 0;
  private probeQueue: Array<() => void> = [];
  private histModeSeq = 0;
  private manualRepoForActivePath: string | null = null;

  constructor() {
    onIdeTargetReset(() => this.reset());
    if (typeof window !== 'undefined') {
      window.addEventListener('otto:git-worktree-discarded', (ev: Event) => {
        const det = (ev as CustomEvent<{ repoId?: string; paths?: string[] }>).detail;
        if (!det?.repoId) return;
        const repo = [
          ...this.repos,
          ...Object.values(this.repoByRoot),
          ...git.repos,
          ...git.allRepos,
        ].find((r) => r?.id === det.repoId);
        if (!repo) return;
        const base = normSlash(repo.path);
        if (det.paths && det.paths.length > 0) {
          for (let i = 0; i < det.paths.length; i++) {
            const abs = `${base}/${det.paths[i].replace(/^\/+/, '')}`;
            const tab = ide.tabs.find((t) => normSlash(t.path) === normSlash(abs));
            if (tab && !tab.dirty) void this.reloadIdeTabIfOpen(tab.path, { onlyIfChangedOnDisk: true });
          }
        } else {
          void this.reloadCleanOpenTabsForRepo(repo.path);
        }
      });
    }
  }

  reset(): void {
    this.repoByRoot = {};
    this.selectedRootPath = null;
    this.selectedRepoId = null;
    this.activeSubTab = 'changes';
    this.historyTabs = [];
    this.focusedChangePath = null;
    this.nonRepoRoots.clear();
    this.lastHandledLiveRev = {};
    this.manualRepoForActivePath = null;
  }

  /** Reactive bridge invoked from `IdeEditorArea` so external `git restore` / CLI checkouts sync clean open IDE tabs. */
  syncExternalRepoChanges(): void {
    if (ide.target.kind !== 'local' || ide.tabs.length === 0) return;
    const seen = new Set<string>();
    const candidates = [...Object.values(this.repoByRoot), ...this.repos];
    for (let i = 0; i < ide.tabs.length; i++) {
      const known = this.findKnownRepoForPath(ide.tabs[i].path);
      if (known) candidates.push(known);
    }
    for (let i = 0; i < candidates.length; i++) {
      const repo = candidates[i];
      if (!repo || seen.has(repo.id)) continue;
      seen.add(repo.id);
      // A repo whose status was read without a live bump has no rev yet: record 0 so its FIRST
      // external change (rev 1) is not swallowed as the baseline.
      const rev = git.liveRev[repo.id] ?? 0;
      const prev = this.lastHandledLiveRev[repo.id];
      this.lastHandledLiveRev[repo.id] = rev;
      if (prev === undefined || rev <= prev) continue;
      for (let j = 0; j < ide.tabs.length; j++) {
        const tab = ide.tabs[j];
        if (tab.dirty || tab.loading || tab.path.startsWith('untitled:')) continue;
        const rel = this.repoRelativePath(repo, tab.path);
        if (rel && git.liveTouches(repo.id, rel)) {
          void this.reloadIdeTabIfOpen(tab.path, { onlyIfChangedOnDisk: true });
        }
      }
    }
  }

  /** All unique detected Otto `Repo` objects across attached IDE project folders (including nested repos). */
  readonly repos = $derived.by((): Repo[] => {
    const seen = new Set<string>();
    const out: Repo[] = [];
    const pushRepo = (r: Repo | null | undefined): void => {
      if (!r || seen.has(r.id) || isBroadNonProjectRootClient(r.path)) return;
      seen.add(r.id);
      out.push(r);
    };
    const rootPaths = ide.projectRoots.map((r) => normSlash(r.path)).filter((p) => !isBroadNonProjectRootClient(p));
    for (let i = 0; i < rootPaths.length; i++) {
      pushRepo(this.repoByRoot[rootPaths[i]]);
    }
    for (const [, r] of Object.entries(this.repoByRoot)) {
      if (!r || isBroadNonProjectRootClient(r.path)) continue;
      const rp = normSlash(r.path);
      if (
        rootPaths.length === 0 ||
        rootPaths.some((rootP) => rp === rootP || rp.startsWith(`${rootP}/`) || rootP.startsWith(`${rp}/`))
      ) {
        pushRepo(r);
      }
    }
    for (const r of [...git.repos, ...git.allRepos]) {
      if (!r?.path || isBroadNonProjectRootClient(r.path)) continue;
      const rp = normSlash(r.path);
      if (rootPaths.some((rootP) => rp === rootP || rp.startsWith(`${rootP}/`) || rootP.startsWith(`${rp}/`))) {
        pushRepo(r);
      }
    }
    return out;
  });

  /** Find the deepest known `Repo` containing `absPath` synchronously if already loaded. */
  findKnownRepoForPath(absPath: string): Repo | null {
    const clean = normSlash(absPath);
    let best: Repo | null = null;
    const candidates = [...Object.values(this.repoByRoot), ...this.repos, ...git.repos, ...git.allRepos];
    for (let i = 0; i < candidates.length; i++) {
      const r = candidates[i];
      if (!r?.path || isBroadNonProjectRootClient(r.path)) continue;
      const rp = normSlash(r.path);
      if (clean === rp || clean.startsWith(`${rp}/`)) {
        if (!best || rp.length > normSlash(best.path).length) best = r;
      }
    }
    return best;
  }

  /** True once `git.statusById` has populated live status for the repo owning `absPath`. */
  isRepoStatusLoaded(absPath: string): boolean {
    const repo = this.findKnownRepoForPath(absPath);
    return Boolean(repo && git.statusById[repo.id] != null);
  }

  /** Compact reactive digest of `absPath`'s git repo/branch/change/ref state for editor diff gutter refresh. */
  fileStatusDigest(absPath: string): string {
    const clean = normSlash(absPath);
    const repo = this.findKnownRepoForPath(clean);
    if (!repo) return `norepo:${this.detectingCount}`;
    const refsGen = git.refsRev[repo.id] ?? 0;
    const st = git.statusById[repo.id];
    if (!st) return `${repo.id}:nostatus:${refsGen}`;
    const repoRoot = normSlash(repo.path);
    const rel = clean === repoRoot ? '' : clean.slice(repoRoot.length + 1);
    const ch = st.changes.find((c) => c.path.replace(/^\/+/, '').replace(/\/+$/, '') === rel);
    return `${repo.id}:${st.branch}:${st.ahead}:${st.behind}:${refsGen}:${ch ? `${ch.kind}:${ch.staged ? 1 : 0}:${ch.unstaged ? 1 : 0}` : 'clean'}`;
  }

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
    const curFile = ide.activePath ? normSlash(ide.activePath) : null;
    if (this.selectedRepoId && curFile === this.manualRepoForActivePath) {
      const manual = this.repos.find((r) => r.id === this.selectedRepoId);
      if (manual) return manual;
    }
    if (curFile) {
      const byFile = this.findKnownRepoForPath(curFile);
      if (byFile) return byFile;
    }
    if (this.selectedRepoId) {
      const manual = this.repos.find((r) => r.id === this.selectedRepoId);
      if (manual) return manual;
    }
    const root = this.activeRoot;
    if (!root) return this.repos[0] ?? null;
    const direct = this.repoByRoot[normSlash(root.path)];
    if (direct && !isBroadNonProjectRootClient(direct.path)) return direct;
    const rootNorm = normSlash(root.path);
    const nested = this.repos.find((r) => normSlash(r.path).startsWith(`${rootNorm}/`));
    return nested ?? this.repos[0] ?? null;
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

      for (const repo of [...Object.values(this.repoByRoot), ...this.repos]) {
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
    const clean = normSlash(ide.resolvePath(absPath));
    const known = this.findKnownRepoForPath(clean);
    if (known) {
      git.ensureStatus(known.id);
      return known;
    }
    // Never hand back a repo that does not contain `clean` (a plain folder must not act on another repo).
    const owns = (r: Repo | null): r is Repo => !!r && this.repoRelativePath(r, clean) !== null;
    const root = this.rootContainingPath(clean) ?? this.activeRoot;
    if (root) {
      const fromRoot = await this.ensureRootDetected(root.path);
      if (owns(fromRoot)) return fromRoot;
    }
    // A directory target (e.g. an unprobed child repo under an umbrella root) probes itself; a file probes its folder.
    const slash = clean.lastIndexOf('/');
    const isDir =
      ide.childrenByDir[clean] !== undefined ||
      (ide.childrenByDir[clean.slice(0, slash)] ?? ide.treeRootEntries).some((e) => normSlash(e.path) === clean && e.kind === 'dir');
    const dir = isDir ? clean : slash > 0 ? clean.slice(0, slash) : '';
    if (dir && (!root || dir.length > normSlash(root.path).length)) {
      const fromDir = await this.ensureRootDetected(dir);
      if (owns(fromDir)) return fromDir;
    }
    return this.findKnownRepoForPath(clean);
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
    const known = this.repoByRoot[clean] ?? this.repos.find((r) => normSlash(r.path) === clean);
    this.selectedRepoId = known?.id ?? null;
    this.manualRepoForActivePath = ide.activePath ? normSlash(ide.activePath) : null;
    void this.ensureRootDetected(clean, true);
  }

  selectRepo(repoId: string): void {
    const repo = this.repos.find((r) => r.id === repoId) ?? null;
    this.selectedRepoId = repoId || null;
    this.manualRepoForActivePath = ide.activePath ? normSlash(ide.activePath) : null;
    if (repo) {
      const root = this.rootContainingPath(repo.path);
      if (root) this.selectedRootPath = normSlash(root.path);
      void git.refreshStatus(repo.id, { force: true });
    }
  }

  /** Open or focus a closable `History: <filename> ×` tab in the IDE Git tool window. */
  async openFileHistoryTab(absPath: string, mode: 'history' | 'blame' = 'history'): Promise<void> {
    const cleanAbs = normSlash(ide.resolvePath(absPath));
    const root = this.rootContainingPath(cleanAbs) ?? this.activeRoot;
    if (root) this.selectedRootPath = normSlash(root.path);
    const repo = await this.resolveRepoForPath(cleanAbs);
    if (!repo) {
      ide.openBottomTab('git' as IdeBottomTab);
      return;
    }
    const relPath = this.repoRelativePath(repo, cleanAbs);
    if (!relPath) return;
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
    const repo = await this.resolveRepoForPath(cleanAbs);
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
    if (isBroadNonProjectRootClient(rootPath)) {
      this.nonRepoRoots.add(rootPath);
      return null;
    }
    if (force) this.nonRepoRoots.delete(rootPath);
    if (!force && this.repoByRoot[rootPath] && !isBroadNonProjectRootClient(this.repoByRoot[rootPath].path)) {
      git.ensureStatus(this.repoByRoot[rootPath].id);
      return this.repoByRoot[rootPath];
    }
    if (!force && this.nonRepoRoots.has(rootPath)) {
      return this.repoByRoot[rootPath] ?? null;
    }
    const existingProbe = !force ? this.inFlightRoots.get(rootPath) : undefined;
    if (existingProbe) {
      return existingProbe;
    }

    // Only short-circuit on EXACT repo root match so nested repositories and
    // folders under a broad parent always run `git rev-parse --show-toplevel`.
    const exactKnown = [...git.repos, ...git.allRepos].find(
      (r) => r?.path && !isBroadNonProjectRootClient(r.path) && normSlash(r.path) === rootPath,
    );
    if (exactKnown && !force) {
      this.repoByRoot = { ...this.repoByRoot, [rootPath]: exactKnown };
      git.upsertRepo(exactKnown);
      git.ensureStatus(exactKnown.id);
      return exactKnown;
    }

    let probePromise: Promise<Repo | null> | null = null;
    probePromise = (async (): Promise<Repo | null> => {
      this.detectingCount += 1;
      try {
        let sawDefinitiveNonRepo = false;
        let hadWorkspaceForDetect = false;
        const detected = await this.enqueueProbe(async () => {
          const wsId =
            ide.target.workspace_id ||
            ws.current?.id ||
            ws.workspaces.find((w) => w.id !== SCRATCH_WORKSPACE_ID)?.id ||
            ws.workspaces[0]?.id ||
            null;
          if (wsId) {
            hadWorkspaceForDetect = true;
            try {
              return await api.bg.post<Repo>(`/workspaces/${encodeURIComponent(wsId)}/repos/detect`, {
                path: rootPath,
              });
            } catch (err) {
              if (err instanceof ApiError && (err.status === 400 || err.status === 404 || err.status === 422)) {
                sawDefinitiveNonRepo = true;
              }
            }
          }
          try {
            const resolved = await api.bg.get<RepoResolveResp>(
              `/git/repos/resolve?ref=${encodeURIComponent(rootPath)}`,
            );
            const r = resolved.repo;
            const rp = normSlash(r.path);
            if (resolved.matched_by !== 'path' || (rootPath !== rp && !rootPath.startsWith(`${rp}/`))) {
              return null;
            }
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
          } catch (err) {
            if (
              hadWorkspaceForDetect &&
              err instanceof ApiError &&
              (err.status === 400 || err.status === 404 || err.status === 422)
            ) {
              sawDefinitiveNonRepo = true;
            }
            return null;
          }
        });

        if (detected && !isBroadNonProjectRootClient(detected.path)) {
          this.nonRepoRoots.delete(rootPath);
          const canonRepoRoot = normSlash(detected.path);
          this.repoByRoot = {
            ...this.repoByRoot,
            [rootPath]: detected,
            [canonRepoRoot]: detected,
          };
          git.upsertRepo(detected);
          void git.refreshStatus(detected.id, { force });
          return detected;
        }
        if ((sawDefinitiveNonRepo && hadWorkspaceForDetect) || (detected && isBroadNonProjectRootClient(detected.path))) {
          this.nonRepoRoots.add(rootPath);
          this.probeUmbrellaChildren(rootPath);
        }
        return null;
      } finally {
        if (this.inFlightRoots.get(rootPath) === probePromise) {
          this.inFlightRoots.delete(rootPath);
        }
        this.detectingCount = Math.max(0, this.detectingCount - 1);
      }
    })();

    this.inFlightRoots.set(rootPath, probePromise);
    return probePromise;
  }

  /** When an attached folder is a multi-repo umbrella (not itself a Git repo), probe its immediate child dirs. */
  private probeUmbrellaChildren(umbrellaRoot: string): void {
    const primaryNorm = ide.target.root ? normSlash(ide.target.root) : '';
    const entries =
      umbrellaRoot === primaryNorm
        ? ide.treeRootEntries
        : (ide.childrenByDir[umbrellaRoot] ?? ide.childrenByDir[`${umbrellaRoot}/`] ?? []);
    const childDirs = entries
      .filter((e) => e.kind === 'dir' && !e.name.startsWith('.') && e.name !== 'node_modules' && e.name !== 'target')
      .slice(0, 12);
    for (let i = 0; i < childDirs.length; i++) {
      const cp = normSlash(childDirs[i].path);
      if (!this.repoByRoot[cp] && !this.nonRepoRoots.has(cp) && !this.inFlightRoots.has(cp)) {
        void this.ensureRootDetected(cp);
      }
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
    const curFile = ide.activePath;
    if (curFile) {
      await this.resolveRepoForPath(curFile);
    }
    const root = this.activeRoot;
    if (root) {
      await this.ensureRootDetected(root.path, true);
    }
    const targets = new Map<string, Repo>();
    if (this.activeRepo) targets.set(this.activeRepo.id, this.activeRepo);
    for (let i = 0; i < this.repos.length; i++) targets.set(this.repos[i].id, this.repos[i]);
    await Promise.all([...targets.values()].map((r) => git.refreshStatus(r.id, { force: true })));
  }

  /** Immediately refresh Git status for the repository containing `absPath` after an IDE save. */
  onIdeFileSaved(absPath: string): void {
    if (ide.target.kind !== 'local' || !absPath || absPath.startsWith('untitled:')) return;
    const known = this.findKnownRepoForPath(absPath);
    if (known) {
      void git.refreshStatus(known.id, { force: true });
      return;
    }
    void this.resolveRepoForPath(absPath).then((repo) => {
      if (repo) void git.refreshStatus(repo.id, { force: true });
    });
  }

  /** Reload clean open tabs inside `repoPath` after checkout / pull / stash operations. */
  async reloadCleanOpenTabsForRepo(repoPath: string): Promise<void> {
    const base = normSlash(repoPath);
    for (let i = 0; i < ide.tabs.length; i++) {
      const t = ide.tabs[i];
      const tp = normSlash(t.path);
      if (!t.dirty && (tp === base || tp.startsWith(`${base}/`))) {
        await this.reloadIdeTabIfOpen(t.path);
      }
    }
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
    const repo = [
      ...this.repos,
      ...Object.values(this.repoByRoot),
      ...git.repos,
      ...git.allRepos,
    ].find((r) => r?.id === repoId);
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
  async reloadIdeTabIfOpen(
    absPath: string,
    opts: { onlyIfChangedOnDisk?: boolean } = {},
  ): Promise<void> {
    const clean = normSlash(ide.resolvePath(absPath));
    const existing = ide.tabs.find((t) => normSlash(t.path) === clean);
    if (!existing) return;
    let fresh: IdeFileReadResp;
    try {
      const root =
        this.rootContainingPath(clean)?.path ||
        this.findKnownRepoForPath(clean)?.path ||
        ide.target.root;
      fresh = await api.get<IdeFileReadResp>(
        `/ide/fs/file?${ide.targetQuery({ root, path: clean })}`,
      );
    } catch (e) {
      if (e instanceof ApiError && e.status === 404) {
        existing.dirty = false;
        existing.savedContent = existing.content;
        await ide.closeTab(existing.path);
      } else {
        existing.error = loadErrorText(e);
        toastError(`Couldn’t reload ${ideBasename(existing.path)}`, e);
      }
      return;
    }

    const detectedEol = detectTextEol(fresh.content);
    const lfContent = normalizeEditorLf(fresh.content);
    if (
      opts.onlyIfChangedOnDisk &&
      !existing.dirty &&
      existing.sha256 &&
      fresh.sha256 === existing.sha256 &&
      existing.content === lfContent
    ) {
      return;
    }
    existing.error = null;
    existing.eol = detectedEol;
    try {
      if (ide.activePath === existing.path || ide.splitPath === existing.path) {
        const applied = await ide.replaceBufferContent(existing.path, lfContent);
        if (!applied) ide.updateBuffer(existing.path, lfContent);
      } else {
        ide.updateBuffer(existing.path, lfContent);
      }
    } catch {
      ide.updateBuffer(existing.path, lfContent);
    }
    existing.content = lfContent;
    existing.savedContent = lfContent;
    existing.dirty = false;
    existing.sha256 = fresh.sha256;
    if (typeof window !== 'undefined') {
      window.dispatchEvent(
        new CustomEvent('otto:ide-git-file-reloaded', {
          detail: { path: existing.path, content: lfContent },
        }),
      );
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
