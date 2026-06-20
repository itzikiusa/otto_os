// Lightweight shared git state: repo list for the current workspace and the
// primary repo's status (status bar branch + right-panel Git tab use this;
// the Git module manages its own deeper state on top).

import { api } from '../api/client';
import type {
  ConflictFile,
  Id,
  MergeBranchReq,
  MergeConflictStatus,
  MergeResult,
  PrSummary,
  Repo,
  RepoStatusResp,
} from '../api/types';

/** Sub-tab inside an open repo (mirrors RepoView's tab set). */
export type GitSubTab = 'graph' | 'changes' | 'history' | 'prs' | 'review';

/** Shape of the global open-tabs persistence blob. */
interface GitOpenTabsState {
  openRepoIds: string[];
  activeRepoId: string | null;
  /** Per-repo active sub-tab. */
  sub: Record<string, string>;
}

/** GLOBAL (workspace-independent) localStorage key for the Git page's open
 *  repo tabs. Survives Tauri restarts; deliberately NOT keyed by workspace so
 *  the Git page is decoupled from the active workspace. */
const OPEN_TABS_KEY = 'otto_git_open_tabs';
const DEFAULT_SUB: GitSubTab = 'graph';

class GitStore {
  repos: Repo[] = $state([]);
  /** All repos across every workspace the caller may view — powers the
   *  workspace-INDEPENDENT Git page. Kept SEPARATE from `repos` so the always-
   *  mounted per-workspace `loadRepos` (right panel / status bar) can't clobber
   *  the Git page's global list when the active workspace changes. */
  allRepos: Repo[] = $state([]);
  primary: Repo | null = $state(null);
  primaryStatus: RepoStatusResp | null = $state(null);
  prs: PrSummary[] = $state([]);
  prsLoading = $state(false);
  /** Non-null when the last PR fetch failed (e.g. bad token / 401). */
  prError: string | null = $state(null);
  loading = $state(false);
  /** True while detecting a repo from a session's cwd. */
  detecting = $state(false);
  /** Set when the focused session's cwd is not inside a git repo. */
  notARepo = $state(false);
  private loadedFor: Id | null = null;
  private detectedCwd: string | null = null;

  // ── Git page top-level repo tabs (GitKraken-style, workspace-independent) ──
  // The set of repos the user has OPEN as tabs, the active one, and each repo's
  // last-used sub-tab. Persisted globally so it survives reloads/restarts.
  openRepoIds: string[] = $state([]);
  activeRepoId: string | null = $state(null);
  subTab: Record<string, string> = $state({});
  /** True once the page has loaded the global repo list at least once. */
  allReposLoaded = $state(false);

  /** All repos across every workspace the caller may view (root → all). Powers
   *  the workspace-independent Git page; does NOT touch `loadedFor` so a later
   *  per-workspace `loadRepos` (right-panel/status-bar) still runs. */
  async loadAllRepos(force = false): Promise<void> {
    if (this.allReposLoaded && !force) return;
    this.loading = true;
    try {
      this.allRepos = await api.get<Repo[]>('/git/repos');
      this.allReposLoaded = true;
    } catch {
      this.allRepos = [];
    } finally {
      this.loading = false;
    }
  }

  /** Restore open tabs from localStorage, dropping ids no longer present in the
   *  live repo list. Call AFTER `loadAllRepos`. Defaults each restored repo's
   *  sub-tab to 'graph' when absent. */
  restoreOpenTabs(): void {
    const live = new Set(this.allRepos.map((r) => r.id));
    const persisted = this.readOpenTabs();
    const ids = (persisted?.openRepoIds ?? []).filter((id) => live.has(id));
    const sub: Record<string, string> = {};
    for (const id of ids) sub[id] = persisted?.sub?.[id] ?? DEFAULT_SUB;
    this.openRepoIds = ids;
    this.subTab = sub;
    const wanted = persisted?.activeRepoId;
    this.activeRepoId = wanted && ids.includes(wanted) ? wanted : (ids[0] ?? null);
    // Re-persist the pruned set so stale ids don't linger.
    this.persistOpenTabs();
  }

  /** Open `repoId` as a tab (or just activate it if already open). */
  openRepoTab(repoId: string, sub?: string): void {
    if (!this.openRepoIds.includes(repoId)) {
      this.openRepoIds = [...this.openRepoIds, repoId];
    }
    if (this.subTab[repoId] == null) {
      this.subTab = { ...this.subTab, [repoId]: sub ?? DEFAULT_SUB };
    } else if (sub) {
      this.subTab = { ...this.subTab, [repoId]: sub };
    }
    this.activeRepoId = repoId;
    this.persistOpenTabs();
  }

  /** Close a repo tab; pick a sensible neighbour as the new active tab. */
  closeRepoTab(repoId: string): void {
    const idx = this.openRepoIds.indexOf(repoId);
    if (idx === -1) return;
    this.openRepoIds = this.openRepoIds.filter((id) => id !== repoId);
    const { [repoId]: _drop, ...rest } = this.subTab;
    this.subTab = rest;
    if (this.activeRepoId === repoId) {
      // Prefer the previous tab, else the next, else none.
      this.activeRepoId =
        this.openRepoIds[Math.min(idx, this.openRepoIds.length - 1)] ?? null;
    }
    this.persistOpenTabs();
  }

  /** Activate an already-open repo tab. */
  activateRepoTab(repoId: string): void {
    if (!this.openRepoIds.includes(repoId)) return;
    this.activeRepoId = repoId;
    this.persistOpenTabs();
  }

  /** Move `repoId` to the position currently held by `targetIdx` (drag-reorder). */
  reorderRepoTab(repoId: string, targetIdx: number): void {
    const from = this.openRepoIds.indexOf(repoId);
    if (from === -1) return;
    const next = [...this.openRepoIds];
    next.splice(from, 1);
    const clamped = Math.max(0, Math.min(targetIdx, next.length));
    next.splice(clamped, 0, repoId);
    this.openRepoIds = next;
    this.persistOpenTabs();
  }

  /** Set the active sub-tab for a repo (graph/changes/history/prs/review). */
  setSubTab(repoId: string, sub: string): void {
    this.subTab = { ...this.subTab, [repoId]: sub };
    this.persistOpenTabs();
  }

  /** The currently-active sub-tab for a repo (default 'graph'). */
  subTabFor(repoId: string): string {
    return this.subTab[repoId] ?? DEFAULT_SUB;
  }

  private persistOpenTabs(): void {
    if (typeof localStorage === 'undefined') return;
    try {
      const blob: GitOpenTabsState = {
        openRepoIds: this.openRepoIds,
        activeRepoId: this.activeRepoId,
        sub: this.subTab,
      };
      localStorage.setItem(OPEN_TABS_KEY, JSON.stringify(blob));
    } catch {
      /* storage full / unavailable — non-fatal */
    }
  }

  private readOpenTabs(): GitOpenTabsState | null {
    if (typeof localStorage === 'undefined') return null;
    const raw = localStorage.getItem(OPEN_TABS_KEY);
    if (!raw) return null;
    try {
      const p = JSON.parse(raw) as Partial<GitOpenTabsState>;
      return {
        openRepoIds: Array.isArray(p.openRepoIds) ? p.openRepoIds : [],
        activeRepoId: typeof p.activeRepoId === 'string' ? p.activeRepoId : null,
        sub: p.sub && typeof p.sub === 'object' ? p.sub : {},
      };
    } catch {
      return null;
    }
  }

  async loadRepos(workspaceId: Id, force = false): Promise<void> {
    if (!force && this.loadedFor === workspaceId) return;
    this.loadedFor = workspaceId;
    this.loading = true;
    try {
      this.repos = await api.get<Repo[]>(`/workspaces/${workspaceId}/repos`);
      this.primary = this.repos[0] ?? null;
      this.primaryStatus = null;
      this.prs = [];
      if (this.primary) {
        await this.selectPrimary(this.primary);
      }
    } catch {
      this.repos = [];
      this.primary = null;
      this.primaryStatus = null;
      this.prs = [];
    } finally {
      this.loading = false;
    }
  }

  /** Make `repo` the primary repo and load its status + PRs. */
  async selectPrimary(repo: Repo): Promise<void> {
    this.primary = repo;
    if (!this.repos.some((r) => r.id === repo.id)) this.repos = [...this.repos, repo];
    try {
      this.primaryStatus = await api.get<RepoStatusResp>(`/repos/${repo.id}/status`);
    } catch {
      this.primaryStatus = null;
    }
    void this.loadPrs(repo.id);
  }

  async loadPrs(repoId: Id): Promise<void> {
    this.prsLoading = true;
    this.prError = null;
    try {
      this.prs = await api.get<PrSummary[]>(`/repos/${repoId}/prs?state=open`);
    } catch (e) {
      this.prs = [];
      // Surface the upstream reason (e.g. a 401 bad token) instead of a
      // misleading "no pull requests".
      this.prError = e instanceof Error ? e.message : String(e);
    } finally {
      this.prsLoading = false;
    }
  }

  /**
   * Detect (and register, idempotently) the git repo containing `cwd` and make
   * it primary. Used by the right-panel Git tab so being inside a repo "just
   * works" without manual registration. No-op if cwd unchanged.
   */
  async detectFor(workspaceId: Id, cwd: string, force = false): Promise<void> {
    if (!cwd) return;
    if (!force && this.detectedCwd === cwd && this.primary) return;
    this.detectedCwd = cwd;
    this.detecting = true;
    this.notARepo = false;
    try {
      const repo = await api.post<Repo>(`/workspaces/${workspaceId}/repos/detect`, { path: cwd });
      await this.selectPrimary(repo);
    } catch {
      // cwd isn't inside a git repo — fall back to any registered repo.
      this.notARepo = !this.primary;
    } finally {
      this.detecting = false;
    }
  }

  async refreshPrimary(): Promise<void> {
    if (!this.primary) return;
    try {
      this.primaryStatus = await api.get<RepoStatusResp>(`/repos/${this.primary.id}/status`);
      void this.loadPrs(this.primary.id);
    } catch {
      /* keep stale status */
    }
  }

  // ── Local merge + conflict resolution ─────────────────────────────────────
  // Thin wrappers over the daemon's merge/conflict endpoints. Conflicts come
  // back from `mergeBranch`/`completeMerge` as a NORMAL result (status:
  // 'conflicts'), not an error — callers branch on `result.status`.

  /** Merge `req.source` into `req.target`. Conflicts are a normal 200 result. */
  mergeBranch(repoId: Id, req: MergeBranchReq): Promise<MergeResult> {
    return api.post<MergeResult>(`/repos/${repoId}/merge`, req);
  }

  /** Current merge state (whether a merge is in progress + conflicted files). */
  getMergeStatus(repoId: Id): Promise<MergeConflictStatus> {
    return api.get<MergeConflictStatus>(`/repos/${repoId}/merge/status`);
  }

  /** Load the segmented view of one conflicted file. */
  getConflictFile(repoId: Id, path: string): Promise<ConflictFile> {
    return api.get<ConflictFile>(`/repos/${repoId}/conflict?path=${encodeURIComponent(path)}`);
  }

  /** Write the resolved content for one file and stage it. */
  resolveConflict(repoId: Id, path: string, content: string): Promise<RepoStatusResp> {
    return api.post<RepoStatusResp>(`/repos/${repoId}/conflict/resolve`, { path, content });
  }

  /** Finish an in-progress merge by creating the merge commit. */
  completeMerge(repoId: Id, message?: string): Promise<MergeResult> {
    return api.post<MergeResult>(`/repos/${repoId}/merge/commit`, { message: message ?? null });
  }

  /** Abort an in-progress merge, restoring the pre-merge state. */
  abortMerge(repoId: Id): Promise<RepoStatusResp> {
    return api.post<RepoStatusResp>(`/repos/${repoId}/merge/abort`);
  }
}

export const git = new GitStore();
