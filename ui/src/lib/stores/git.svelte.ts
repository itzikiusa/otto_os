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

class GitStore {
  repos: Repo[] = $state([]);
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
