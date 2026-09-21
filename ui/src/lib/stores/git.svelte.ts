// Lightweight shared git state: repo list for the current workspace and the
// primary repo's status (status bar branch + right-panel Git tab use this;
// the Git module manages its own deeper state on top).

import { api } from '../api/client';
import type {
  ConflictFile,
  Id,
  MergeBranchReq,
  MergeConflictStatus,
  MergePreview,
  MergeResult,
  PrListResp,
  PrSummary,
  PullResp,
  Repo,
  RepoStatusResp,
} from '../api/types';

/** Sub-tab inside an open repo (mirrors RepoView's tab set). The old
 *  'changes' and 'history' tabs were folded into the graph (WIP row + detail
 *  panel); persisted values from before that fold are normalized to 'graph'. */
export type GitSubTab = 'graph' | 'prs' | 'review' | 'focus';

/** Map a persisted/legacy sub-tab value onto the current tab set. */
function normSubTab(sub: string | null | undefined): string {
  return sub == null || sub === 'changes' || sub === 'history' ? DEFAULT_SUB : sub;
}

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
/** localStorage key for the auto-fetch toggle/interval (per-device). */
const AUTO_FETCH_KEY = 'otto_git_auto_fetch';
const DEFAULT_AUTO_FETCH_SEC = 120;
const ACTIVE_AUTO_FETCH_SEC = 30;
const DEFAULT_SUB: GitSubTab = 'graph';

/** May an auto-fetch round run right now?
 *
 *  `document.hidden` alone is not enough: Otto is multi-window, and a BACKGROUND
 *  window is fully "visible" to the browser, so a second window parked on the
 *  Git page kept firing a fetch per open repo every 10 s forever — each one a
 *  `git fetch --prune` + `status` spawn plus a synchronous Keychain read on the
 *  daemon's runtime, which is load the window in front pays for (investigation
 *  H4/WP3). So we also require FOCUS: exactly the window the user is looking at
 *  polls. `hasFocus` is absent in SSR/jsdom — treat that as "allowed" so tests
 *  and headless renders behave as before. */
function autoFetchAllowed(): boolean {
  if (typeof document === 'undefined') return true;
  if (document.hidden) return false;
  return typeof document.hasFocus === 'function' ? document.hasFocus() : true;
}

/** Read the persisted auto-fetch config, guarded for SSR/test environments with
 *  no localStorage. Defaults: enabled, every two minutes (selected repo every 30s). */
function readAutoFetchConfig(): { enabled: boolean; intervalSec: number } {
  const def = { enabled: true, intervalSec: DEFAULT_AUTO_FETCH_SEC };
  if (typeof localStorage === 'undefined') return def;
  try {
    const raw = localStorage.getItem(AUTO_FETCH_KEY);
    if (!raw) return def;
    const p = JSON.parse(raw) as Partial<{ enabled: boolean; intervalSec: number }>;
    return {
      enabled: typeof p.enabled === 'boolean' ? p.enabled : def.enabled,
      intervalSec:
        typeof p.intervalSec === 'number' && Number.isFinite(p.intervalSec)
          ? Math.max(30, p.intervalSec) : def.intervalSec,
    };
  } catch {
    return def;
  }
}

/** Cheap structural equality for a repo status — scalars first, then a light
 *  per-file signature. Lets `setStatus` skip no-op writes so a fetch that found
 *  nothing new doesn't recompute the `$derived` status / re-render the toolbar +
 *  tab chips every round. NOTE: this is load-bearing for ALL status propagation
 *  (manual fetch/pull/push/checkout/merge route through `setStatus` too), so keep
 *  it in lockstep with the `RepoStatusResp` shape — any newly-rendered field must
 *  be compared here or a real change could be silently suppressed. */
function statusEq(a: RepoStatusResp, b: RepoStatusResp): boolean {
  if (
    a.branch !== b.branch ||
    a.upstream !== b.upstream ||
    a.ahead !== b.ahead ||
    a.behind !== b.behind ||
    (a.op_in_progress ?? null) !== (b.op_in_progress ?? null) ||
    a.changes.length !== b.changes.length
  ) {
    return false;
  }
  for (let i = 0; i < a.changes.length; i++) {
    const x = a.changes[i];
    const y = b.changes[i];
    if (
      x.path !== y.path ||
      x.orig_path !== y.orig_path ||
      x.kind !== y.kind ||
      x.staged !== y.staged ||
      x.unstaged !== y.unstaged
    ) {
      return false;
    }
  }
  return true;
}

const INITIAL_AUTO_FETCH = readAutoFetchConfig();

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

  // ── Per-repo status (single source of truth) ───────────────────────────────
  // Each open tab's branch chip (GitTabs), the active repo's toolbar (RepoView)
  // and the auto-fetch loop all read/write THIS map, so one fetch updates every
  // view. `null` is an in-flight / load-attempted marker.
  statusById: Record<string, RepoStatusResp | null> = $state({});

  // ── Auto-fetch: a quiet background `git fetch` for the OPEN tabs so each tab's
  // ahead/behind chip stays live. Polls only the repos the user has open
  // throughout the authenticated app, pausing in hidden/unfocused windows. The
  // on/off toggle is persisted per-device. ──
  autoFetchEnabled = $state(INITIAL_AUTO_FETCH.enabled);
  autoFetchIntervalSec = $state(INITIAL_AUTO_FETCH.intervalSec);
  /** Per-repo ref-refresh signal, bumped after EVERY successful fetch.
   *  `setStatus` deliberately skips no-op status writes, and `statusEq` only
   *  compares HEAD and its one tracking ref — so a fetch that advances
   *  `origin/feature-x`, adds a remote branch, moves a tag, or `--prune`s a
   *  branch the current HEAD doesn't track changes NO status field. A view
   *  caching refs (GraphView) would stay stale until remount, so it gets the
   *  signal unconditionally and decides for itself whether anything actually
   *  moved: it re-reads the cheap `/refs` and only replays the expensive
   *  `log --all -n 10000` + stashes + worktrees fan-out when the ref
   *  fingerprint differs (investigation H4/WP3). A pruned ref disappears from
   *  `/refs`, so that case is covered too. */
  refsRev: Record<string, number> = $state({});
  private autoFetchTimer: ReturnType<typeof setTimeout> | null = null;
  private autoFetchRunning = false;
  private autoFetchInFlight = false;
  private autoFetchGen = 0;
  private lastFetchAt: Record<string, number> = {};
  private retryFetchAt: Record<string, number> = {};
  private autoFetchFailStreak: Record<string, number> = {};
  private fetches = new Map<string, Promise<RepoStatusResp>>();
  private manualFetches = new Set<string>();
  private fetchQueue: (() => void)[] = [];
  private fetchCount = 0;
  private tabsInitialized: Promise<void> | null = null;

  /** Shared bootstrap: both the shell and deep-linked Git page can await it. */
  initializeOpenTabs(): Promise<void> {
    return this.tabsInitialized ??= (async () => {
      await this.loadAllRepos();
      if (this.allReposLoaded) {
        this.restoreOpenTabs();
        this.requestAutoFetch();
      } else {
        this.tabsInitialized = null; // Retry a failed startup when Git opens later.
      }
    })();
  }

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
    for (const id of ids) sub[id] = normSubTab(persisted?.sub?.[id]);
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
    this.requestAutoFetch();
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
    this.requestAutoFetch();
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

  /** Set the active sub-tab for a repo (graph/prs/review/focus). */
  setSubTab(repoId: string, sub: string): void {
    this.subTab = { ...this.subTab, [repoId]: sub };
    this.persistOpenTabs();
  }

  /** The currently-active sub-tab for a repo (default 'graph'; legacy
   *  'changes'/'history' values normalize to 'graph'). */
  subTabFor(repoId: string): string {
    return normSubTab(this.subTab[repoId]);
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
      const s = await api.get<RepoStatusResp>(`/repos/${repo.id}/status`);
      this.primaryStatus = s;
      this.setStatus(repo.id, s);
    } catch {
      this.primaryStatus = null;
    }
    void this.loadPrs(repo.id);
  }

  async loadPrs(repoId: Id): Promise<void> {
    this.prsLoading = true;
    this.prError = null;
    try {
      this.prs = (await api.get<PrListResp>(`/repos/${repoId}/prs?state=open`)).items;
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
      const s = await api.get<RepoStatusResp>(`/repos/${this.primary.id}/status`);
      this.primaryStatus = s;
      this.setStatus(this.primary.id, s);
      void this.loadPrs(this.primary.id);
    } catch {
      /* keep stale status */
    }
  }

  // ── Per-repo status cache + auto-fetch ─────────────────────────────────────

  /** Write a repo's status, but ONLY when it actually changed, so a no-op fetch
   *  doesn't needlessly recompute the `$derived` status + re-render the toolbar
   *  and tab chips. Keeps `primaryStatus` in sync when this repo is the
   *  right-panel primary. Returns whether anything actually changed, so a caller
   *  that wants to do more work only on a real change can ask. (The auto-fetch
   *  loop deliberately does NOT gate `refsRev` on it — `statusEq` compares only
   *  HEAD and its upstream, so it cannot see most ref movement; see `refsRev`.) */
  setStatus(repoId: string, s: RepoStatusResp): boolean {
    const prev = this.statusById[repoId];
    if (prev && statusEq(prev, s)) return false;
    this.statusById[repoId] = s;
    if (this.primary?.id === repoId) this.primaryStatus = s;
    return true;
  }

  /** Fetch (cheap, local) status for a repo and store it. */
  async refreshStatus(repoId: string): Promise<void> {
    try {
      const s = await api.get<RepoStatusResp>(`/repos/${repoId}/status`);
      this.setStatus(repoId, s);
    } catch {
      /* keep stale status */
    }
  }

  /** Lazily load a repo's status once (used by the tab strip). `null` marks an
   *  in-flight / attempted load so we don't refetch on every render. */
  ensureStatus(repoId: string): void {
    if (repoId in this.statusById) return;
    this.statusById[repoId] = null;
    void this.refreshStatus(repoId);
  }

  /** Manual and background callers share one request per repo and two network
   *  slots per window. Errors reach manual callers; background workers swallow
   *  them. Every success invalidates refs, even when HEAD status is unchanged. */
  fetchRepo(repoId: string, reason: 'manual' | 'background' = 'manual'): Promise<RepoStatusResp> {
    // A manual request can promote a queued background fetch. Explicit work
    // remains valid even if its tab closes or automatic fetching is paused.
    if (reason === 'manual') this.manualFetches.add(repoId);
    const existing = this.fetches.get(repoId);
    if (existing) return existing;
    const generation = this.autoFetchGen;
    const result = new Promise<RepoStatusResp>((resolve, reject) => {
      this.fetchQueue.push(() => {
        if (!this.manualFetches.has(repoId) && (
          generation !== this.autoFetchGen || !this.autoFetchRunning ||
          !this.autoFetchEnabled || !autoFetchAllowed() || !this.openRepoIds.includes(repoId)
        )) {
          this.fetches.delete(repoId);
          reject(new Error('Background fetch no longer needed'));
          return; // Canceled work never consumes a network slot or retry budget.
        }
        this.fetchCount++;
        void (async () => {
          try {
            const status = await api.post<RepoStatusResp>(`/repos/${repoId}/fetch`);
            this.setStatus(repoId, status);
            this.refsRev[repoId] = (this.refsRev[repoId] ?? 0) + 1;
            this.lastFetchAt[repoId] = Date.now();
            this.autoFetchFailStreak[repoId] = 0;
            this.retryFetchAt[repoId] = 0;
            resolve(status);
          } catch (error) {
            const streak = (this.autoFetchFailStreak[repoId] ?? 0) + 1;
            this.autoFetchFailStreak[repoId] = streak;
            this.retryFetchAt[repoId] = Date.now() + Math.min(600_000, 60_000 * 2 ** Math.min(streak - 1, 4));
            reject(error);
          } finally {
            this.fetches.delete(repoId);
            this.manualFetches.delete(repoId);
            this.fetchCount--;
            this.drainFetchQueue();
          }
        })();
      });
    });
    this.fetches.set(repoId, result);
    this.drainFetchQueue();
    return result;
  }

  private drainFetchQueue(): void {
    while (this.fetchCount < 2 && this.fetchQueue.length) this.fetchQueue.shift()!();
  }

  /** Shell lifetime, independent of which module is currently shown. */
  startAutoFetch(): void {
    if (this.autoFetchRunning) return;
    this.autoFetchRunning = true;
    this.requestAutoFetch();
  }

  stopAutoFetch(): void {
    this.autoFetchRunning = false;
    this.autoFetchGen++;
    if (this.autoFetchTimer !== null) clearTimeout(this.autoFetchTimer);
    this.autoFetchTimer = null;
  }

  setAutoFetch(enabled: boolean): void {
    this.autoFetchEnabled = enabled;
    this.persistAutoFetch();
    if (this.autoFetchTimer !== null) clearTimeout(this.autoFetchTimer);
    this.autoFetchTimer = null;
    this.requestAutoFetch();
  }

  /** Focus/tab activation wakes the loop without bypassing freshness/backoff. */
  requestAutoFetch(): void {
    if (!this.autoFetchRunning || !this.autoFetchEnabled) return;
    this.scheduleAutoFetch(0);
  }

  private scheduleAutoFetch(ms = 5000): void {
    if (this.autoFetchTimer !== null) clearTimeout(this.autoFetchTimer);
    const gen = this.autoFetchGen;
    this.autoFetchTimer = setTimeout(() => {
      this.autoFetchTimer = null;
      void this.autoFetchTick(gen);
    }, ms);
  }

  private async autoFetchTick(gen: number): Promise<void> {
    if (gen !== this.autoFetchGen || !this.autoFetchRunning || !this.autoFetchEnabled) return;
    if (!this.autoFetchInFlight && autoFetchAllowed()) {
      this.autoFetchInFlight = true;
      const pending = new Set(this.openRepoIds);
      const worker = async (): Promise<void> => {
        while (pending.size && gen === this.autoFetchGen && this.autoFetchEnabled && autoFetchAllowed()) {
          // Re-evaluate the selected tab between requests, so a newly focused
          // repo goes first even when another repository's network is slow.
          const id = this.activeRepoId && pending.has(this.activeRepoId)
            ? this.activeRepoId : pending.values().next().value!;
          pending.delete(id);
          if (!this.openRepoIds.includes(id)) continue;
          const interval = id === this.activeRepoId ? ACTIVE_AUTO_FETCH_SEC : Math.max(30, this.autoFetchIntervalSec);
          if (Date.now() < (this.retryFetchAt[id] ?? 0)) continue;
          if (this.lastFetchAt[id] != null && Date.now() - this.lastFetchAt[id] < interval * 1000) continue;
          try { await this.fetchRepo(id, 'background'); } catch { /* quiet background retry with backoff */ }
        }
      };
      try { await Promise.all([worker(), worker()]); }
      finally { this.autoFetchInFlight = false; }
    }
    if (this.autoFetchRunning && this.autoFetchEnabled && gen === this.autoFetchGen) this.scheduleAutoFetch();
  }

  private persistAutoFetch(): void {
    if (typeof localStorage === 'undefined') return;
    try {
      localStorage.setItem(
        AUTO_FETCH_KEY,
        JSON.stringify({ enabled: this.autoFetchEnabled, intervalSec: this.autoFetchIntervalSec }),
      );
    } catch {
      /* storage full / unavailable — non-fatal */
    }
  }

  // ── Local merge + conflict resolution ─────────────────────────────────────
  // Thin wrappers over the daemon's merge/conflict endpoints. Conflicts come
  // back from `mergeBranch`/`completeMerge` as a NORMAL result (status:
  // 'conflicts'), not an error — callers branch on `result.status`.

  /** Pull the repo. `autoStash` wraps a dirty tree in stash → pull → pop (the
   *  retry offered after a 409 "commit or stash first" refusal). */
  pull(repoId: Id, autoStash = false): Promise<PullResp> {
    return api.post<PullResp>(`/repos/${repoId}/pull`, autoStash ? { auto_stash: true } : undefined);
  }

  /** Merge `req.source` into `req.target`. Conflicts are a normal 200 result. */
  mergeBranch(repoId: Id, req: MergeBranchReq): Promise<MergeResult> {
    return api.post<MergeResult>(`/repos/${repoId}/merge`, req);
  }

  /** Dry-run a merge of `source` into `target` (no working-tree mutation). */
  mergePreview(repoId: Id, source: string, target: string): Promise<MergePreview> {
    return api.post<MergePreview>(`/repos/${repoId}/merge/preview`, { source, target });
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
