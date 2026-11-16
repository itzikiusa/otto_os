// Lightweight shared git state: repo list for the current workspace and the
// primary repo's status (status bar branch + right-panel Git tab use this;
// the Git module manages its own deeper state on top).

import { api } from '../api/client';
import { isBroadNonProjectRootClient } from '../fileUri';
import { latestOnly } from '../latest';
import { loadErrorText } from '../loadError';
import { mergeTouched, touches, type TouchedPaths } from '../gitLivePaths';
import {
  ACTIVE_AUTO_FETCH_SEC, RECENT_USE_MS, autoFetchAllowed, readAutoFetchConfig, writeAutoFetchConfig,
} from '../gitAutoFetchConfig';
import { gitApi, statusEq, type RefsRead } from '../api/git';
import type {
  CheckoutReq,
  ConflictFile,
  Id,
  MergeBranchReq,
  MergeConflictStatus,
  MergePreview,
  MergeResult,
  PullMode,
  PullReq,
  PullResp,
  PushReq,
  Repo,
  RepoStatusResp,
} from '../api/types';

/** Sub-tab inside an open repo (mirrors RepoView's tab set). The old
 *  'changes' and 'history' tabs were folded into the graph (WIP row + detail
 *  panel); persisted values from before that fold are normalized to 'graph'. */
export type GitSubTab = 'graph' | 'prs' | 'review';

/** Map a persisted/legacy sub-tab value onto the current tab set (Focus is
 *  a page-level view now, no longer a repo sub-tab). */
function normSubTab(sub: string | null | undefined): string {
  return sub == null || sub === 'changes' || sub === 'history' || sub === 'focus' ? DEFAULT_SUB : sub;
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
/** A fetch's own status answers a status read this soon after it. */
const FETCH_STATUS_REUSE_MS = 2000;
/** A status read younger than this is shared by a new caller (same burst). */
const STATUS_SHARE_MS = 300;
const DEFAULT_SUB: GitSubTab = 'graph';
const INITIAL_AUTO_FETCH = readAutoFetchConfig();
const isBroadRepo = (r: { path?: string | null }): boolean => Boolean(r.path) && isBroadNonProjectRootClient(r.path);

class GitStore {
  repos: Repo[] = $state([]);
  /** All repos across every workspace the caller may view — powers the
   *  workspace-INDEPENDENT Git page. Kept SEPARATE from `repos` so the always-
   *  mounted per-workspace `loadRepos` (right panel / status bar) can't clobber
   *  the Git page's global list when the active workspace changes. */
  allRepos: Repo[] = $state([]);
  primary: Repo | null = $state(null);
  primaryStatus: RepoStatusResp | null = $state.raw(null);
  loading = $state(false);
  /** True while detecting a repo from a session's cwd. */
  detecting = $state(false);
  /** Set when the focused session's cwd is not inside a git repo. */
  notARepo = $state(false);
  /** The workspace repo list failed to load (S13-305): the panel offers
   *  Retry and the app retries on window focus. Null when loaded. */
  reposError: string | null = $state(null);
  private loadedFor: Id | null = null;
  private detectedCwd: string | null = null;
  /** The repo the last successful {@link detectFor} made primary — a
   *  `loadRepos` that lands after it must not replace it with `repos[0]`. */
  private detectedRepoId: Id | null = null;
  // Stale-response guards (S13-03): switching workspace / focused session
  // A→B used to let A's slower status / repo list land last and show repo
  // A's branch under repo B's name.
  private reposGen = latestOnly();
  private primaryGen = latestOnly();
  private detectGen = latestOnly();

  // ── Git page top-level repo tabs (GitKraken-style, workspace-independent) ──
  // The set of repos the user has OPEN as tabs, the active one, and each repo's
  // last-used sub-tab. Persisted globally so it survives reloads/restarts.
  openRepoIds: string[] = $state([]);
  activeRepoId: string | null = $state(null);
  subTab: Record<string, string> = $state({});
  /** True once the page has loaded the global repo list at least once. */
  allReposLoaded = $state(false);
  /** Last failed {@link loadAllRepos} (human text). While set, `allRepos` is the
   *  last KNOWN list (or empty on a first failure) — never "you have no repos". */
  allReposError: string | null = $state(null);

  // ── Per-repo status (single source of truth) ───────────────────────────────
  // Each open tab's branch chip (GitTabs), the active repo's toolbar (RepoView)
  // and the auto-fetch loop all read/write THIS map, so one fetch updates every
  // view. `null` is an in-flight / load-attempted marker.
  statusById: Record<string, RepoStatusResp | null> = $state.raw({});
  /** Per-repo status-load failure (human text); cleared on the next success. A
   *  stale `statusById` entry stays in place alongside it. */
  statusErrorById: Record<string, string> = $state({});

  // ── Auto-fetch: background `git fetch` for OPEN tabs (pauses when hidden) ──
  autoFetchEnabled = $state(INITIAL_AUTO_FETCH.enabled);
  autoFetchIntervalSec = $state(INITIAL_AUTO_FETCH.intervalSec);
  /** Per-repo ref-refresh signal, bumped after every fetch/commit/checkout/pull
   *  so GraphView and IDE Git Log re-read `/refs` even when HEAD is unchanged. */
  refsRev: Record<string, number> = $state({});

  /** A mutation moved refs or history: every open graph of `repoId` re-syncs. */
  bumpRefs(repoId: string): void {
    this.refsRev[repoId] = (this.refsRev[repoId] ?? 0) + 1;
  }

  /** The one `/refs` read (never throws — see `gitApi.readRefs`). */
  refs(repoId: Id): Promise<RefsRead> {
    return gitApi.readRefs(repoId);
  }
  private autoFetchTimer: ReturnType<typeof setTimeout> | null = null;
  private autoFetchRunning = false;
  private autoFetchInFlight = false;
  private autoFetchGen = 0;
  private lastFetchAt: Record<string, number> = {};
  /** Last time each repo was opened/selected/manually fetched (see RECENT_USE_MS). */
  private repoUsedAt: Record<string, number> = {};
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

  /** Register or update an already-resolved repo in memory immediately. */
  upsertRepo(repo: Repo): void {
    if (!repo?.id || isBroadRepo(repo)) return;
    const idxAll = this.allRepos.findIndex((r) => r.id === repo.id);
    this.allRepos = idxAll >= 0 ? this.allRepos.map((r, i) => (i === idxAll ? repo : r)) : [...this.allRepos, repo];
    if (!this.loadedFor || repo.workspace_id === this.loadedFor) {
      const idxWs = this.repos.findIndex((r) => r.id === repo.id);
      this.repos = idxWs >= 0 ? this.repos.map((r, i) => (i === idxWs ? repo : r)) : [...this.repos, repo];
    }
  }

  /** All repos across every workspace the caller may view (root → all). */
  async loadAllRepos(force = false): Promise<void> {
    if (this.allReposLoaded && !force) return;
    this.loading = true;
    try {
      const fetched = (await api.get<Repo[]>('/git/repos')).filter((r) => !isBroadRepo(r));
      const byId = new Map(fetched.map((r) => [r.id, r]));
      for (const id of this.openRepoIds) {
        const mem = this.allRepos.find((r) => r.id === id);
        if (mem && !byId.has(id) && !isBroadRepo(mem)) byId.set(id, mem);
      }
      this.allRepos = [...byId.values()];
      this.allReposLoaded = true;
      this.allReposError = null;
    } catch (e) {
      this.allReposError = loadErrorText(e);
    } finally {
      this.loading = false;
    }
  }

  /** Restore open tabs from localStorage, preserving any tabs opened in memory
   *  before bootstrap completed and dropping missing or broad non-project ids. */
  restoreOpenTabs(): void {
    const live = new Set(this.allRepos.filter((r) => !isBroadRepo(r)).map((r) => r.id));
    const persisted = this.readOpenTabs();
    const seen = new Set<string>();
    const ids = [...(persisted?.openRepoIds ?? []), ...this.openRepoIds].filter(
      (id) => live.has(id) && !seen.has(id) && (seen.add(id), true),
    );
    const sub: Record<string, string> = {};
    for (const id of ids) sub[id] = normSubTab(this.subTab[id] ?? persisted?.sub?.[id]);
    const memActive = this.activeRepoId;
    this.openRepoIds = ids;
    this.subTab = sub;
    const wanted = memActive && ids.includes(memActive) ? memActive : persisted?.activeRepoId;
    this.activeRepoId = wanted && ids.includes(wanted) ? wanted : (ids[0] ?? null);
    this.persistOpenTabs();
  }

  /** Open `repoId` as a tab (hydrating saved tabs first if bootstrap hasn't run). */
  openRepoTab(repoId: string, sub?: string): void {
    if (!this.allReposLoaded && this.openRepoIds.length === 0) {
      const persisted = this.readOpenTabs();
      if (persisted) {
        this.openRepoIds = persisted.openRepoIds;
        this.subTab = { ...persisted.sub };
      }
    }
    if (!this.openRepoIds.includes(repoId)) {
      this.openRepoIds = [...this.openRepoIds, repoId];
    }
    if (this.subTab[repoId] == null || sub) this.subTab = { ...this.subTab, [repoId]: normSubTab(sub) };
    this.arrive(repoId);
  }

  /** A tab became active. The status bar's branch follows it (it showed the session repo's). */
  private arrive(repoId: string): void {
    this.activeRepoId = repoId;
    this.repoUsedAt[repoId] = Date.now();
    this.persistOpenTabs();
    this.fetchOnArrival();
    const repo = this.allRepos.find((r) => r.id === repoId);
    if (repo && this.primary?.id !== repoId) void this.selectPrimary(repo);
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
    this.arrive(repoId);
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

  /** Set the active sub-tab for a repo (graph/prs/review). */
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
    const t = this.reposGen.begin();
    try {
      const rawRepos = await api.get<Repo[]>(`/workspaces/${workspaceId}/repos`);
      if (!t.current) return;
      const repos = rawRepos.filter((r) => !isBroadRepo(r));
      this.reposError = null;
      const kept = this.primary && !isBroadRepo(this.primary) ? this.primary : null;
      if (kept && kept.workspace_id === workspaceId && (kept.id === this.detectedRepoId || repos.some((r) => r.id === kept.id))) {
        this.repos = repos.some((r) => r.id === kept.id) ? repos : [...repos, kept];
        return;
      }
      this.repos = repos;
      this.primary = this.repos[0] ?? null;
      this.primaryStatus = null;
      if (this.primary) {
        await this.selectPrimary(this.primary);
      }
    } catch (e) {
      if (!t.current) return;
      this.repos = [];
      this.primary = null;
      this.primaryStatus = null;
      // Not sticky: the next loadRepos for this workspace retries — the
      // panel's Retry, or App's retry on window focus (S13-305).
      this.loadedFor = null;
      this.reposError = loadErrorText(e);
    } finally {
      if (t.current) this.loading = false;
    }
  }

  /** Make `repo` the primary repo and load its status. A later selection
   *  supersedes this one: its status is then dropped (the per-repo status
   *  cache still takes it — it is keyed by repo). The PR tab and FocusView
   *  fetch their own PR lists. */
  async selectPrimary(repo: Repo): Promise<void> {
    const t = this.primaryGen.begin();
    this.primary = repo;
    if (!this.repos.some((r) => r.id === repo.id)) this.repos = [...this.repos, repo];
    try {
      const s = this.freshFetchStatus(repo.id) ?? await gitApi.status(repo.id);
      this.setStatus(repo.id, s);
      if (!t.current || this.primary?.id !== repo.id) return;
      this.primaryStatus = s;
    } catch {
      if (!t.current || this.primary?.id !== repo.id) return;
      this.primaryStatus = null;
    }
  }

  /** Detect (and register, idempotently) the git repo containing `cwd` and make it primary. */
  async detectFor(workspaceId: Id, cwd: string, force = false): Promise<void> {
    if (!cwd) return;
    if (isBroadNonProjectRootClient(cwd)) {
      this.detectedCwd = cwd;
      this.notARepo = !this.primary;
      return;
    }
    if (!force && this.detectedCwd === cwd && this.primary) return;
    this.detectedCwd = cwd;
    this.detecting = true;
    this.notARepo = false;
    const t = this.detectGen.begin();
    try {
      const repo = await api.post<Repo>(`/workspaces/${workspaceId}/repos/detect`, { path: cwd });
      if (!t.current) return;
      if (isBroadRepo(repo)) {
        this.notARepo = !this.primary;
        return;
      }
      this.detectedRepoId = repo.id;
      await this.selectPrimary(repo);
    } catch {
      if (!t.current) return;
      this.notARepo = !this.primary;
    } finally {
      if (t.current) this.detecting = false;
    }
  }

  /** Retry a failed workspace load (and the cwd detect that may have fallen
   *  back with it). */
  async retryRepos(workspaceId: Id, cwd?: string | null): Promise<void> {
    await this.loadRepos(workspaceId, true);
    if (cwd) await this.detectFor(workspaceId, cwd, true);
  }

  async refreshPrimary(): Promise<void> {
    const repo = this.primary;
    if (!repo) return;
    try {
      const s = this.freshFetchStatus(repo.id) ?? await gitApi.status(repo.id);
      this.setStatus(repo.id, s);
      if (this.primary?.id !== repo.id) return; // primary moved on meanwhile
      this.primaryStatus = s;
    } catch {
      /* keep stale status */
    }
  }

  // ── Per-repo status cache + auto-fetch ─────────────────────────────────────

  /** Write a repo's status only when it changed; returns whether it changed. */
  setStatus(repoId: string, s: RepoStatusResp): boolean {
    const prev = this.statusById[repoId];
    if (prev && statusEq(prev, s)) return false;
    this.statusById = { ...this.statusById, [repoId]: s };
    if (this.primary?.id === repoId) this.primaryStatus = s;
    return true;
  }

  /** The status a fetch returned moments ago (it ran `git status` right after
   *  the fetch), so a caller doesn't spawn a second walk for the same state. */
  private freshFetchStatus(repoId: string): RepoStatusResp | null {
    const at = this.lastFetchAt[repoId];
    const s = this.statusById[repoId];
    return s && at != null && Date.now() - at < FETCH_STATUS_REUSE_MS ? s : null;
  }

  /** In-flight status reads per repo. RepoView's mount, `ensureStatus` and a
   *  live refresh used to overlap into 2–3 identical GETs (and each one a full
   *  `git status` walk on the daemon). */
  private statusFlights = new Map<string, { p: Promise<void>; at: number; next?: Promise<void> }>();

  /** Fetch local status for a repo and store it (`force` bypasses client/server memo caches). */
  refreshStatus(repoId: string, opts?: { force?: boolean }): Promise<void> {
    const force = Boolean(opts?.force);
    const f = this.statusFlights.get(repoId);
    if (f && !force) {
      if (Date.now() - f.at < STATUS_SHARE_MS) return f.p;
      return (f.next ??= f.p.then(() => this.refreshStatus(repoId)));
    }
    const p: Promise<void> = this.readStatus(repoId, force).finally(() => {
      if (this.statusFlights.get(repoId)?.p === p) this.statusFlights.delete(repoId);
    });
    this.statusFlights.set(repoId, { p, at: Date.now() });
    return p;
  }

  private async readStatus(repoId: string, fresh = false): Promise<void> {
    try {
      const s = await gitApi.status(repoId, fresh ? { fresh: true } : undefined);
      this.setStatus(repoId, s);
      if (fresh) {
        this.liveChanged = { ...this.liveChanged, [repoId]: null };
        this.liveRev = { ...this.liveRev, [repoId]: (this.liveRev[repoId] ?? 0) + 1 };
      }
      if (repoId in this.statusErrorById) {
        const { [repoId]: _cleared, ...rest } = this.statusErrorById;
        this.statusErrorById = rest;
      }
    } catch (e) {
      this.statusErrorById = { ...this.statusErrorById, [repoId]: loadErrorText(e) };
    }
  }

  // ── Live working-tree changes ──────────────────────────────────────────────
  // The daemon watches every repo whose status/fetch we asked for and sends
  // `repo_status_changed` ~150–400 ms after an edit, a CLI `git add`, commit or
  // checkout (otto-git watch.rs). Before, local edits only surfaced with the
  // next auto-fetch round — up to 30 s, and a network fetch to notice them.

  /** Bumped per applied change, so views showing file CONTENT (the open diff)
   *  can re-read it even when the change list itself is unchanged. */
  liveRev: Record<string, number> = $state({});
  /** The repo-relative paths the latest `liveRev` bump covers (`null` =
   *  unknown: re-read anything). Set BEFORE the bump; see `liveTouches`. */
  liveChanged: Record<string, string[] | null> = $state({});
  /** Paths of events not yet taken by a refresh (union; `null` = unknown). */
  private livePending = new Map<string, TouchedPaths>();
  private liveInFlight = new Set<string>();
  private liveDirty = new Set<string>();
  private liveHiddenWait = false;

  /** A watched repo changed on disk: re-read its local status if we show it.
   *  `paths` (absent = unknown) is what the burst touched, so the open diff
   *  re-reads only when its file is among them (`liveTouches`). */
  applyRepoChanged(repoId: string, paths?: string[] | null): void {
    const shown = this.openRepoIds.includes(repoId) || this.primary?.id === repoId || repoId in this.statusById;
    if (!shown) return;
    this.livePending.set(repoId, mergeTouched(this.livePending.get(repoId), paths));
    if (typeof document !== 'undefined' && document.hidden) {
      // Hidden window: remember it, read once when it's visible again.
      this.liveDirty.add(repoId);
      if (!this.liveHiddenWait) {
        this.liveHiddenWait = true;
        const onVisible = (): void => {
          if (document.hidden) return;
          document.removeEventListener('visibilitychange', onVisible);
          this.liveHiddenWait = false;
          for (const id of [...this.liveDirty]) {
            this.liveDirty.delete(id);
            void this.liveRefresh(id);
          }
        };
        document.addEventListener('visibilitychange', onVisible);
      }
      return;
    }
    void this.liveRefresh(repoId);
  }

  /** One status read per repo at a time; a change landing meanwhile re-reads
   *  once more afterwards, so a burst never queues N requests. */
  private async liveRefresh(repoId: string): Promise<void> {
    if (this.liveInFlight.has(repoId)) {
      this.liveDirty.add(repoId);
      return;
    }
    this.liveInFlight.add(repoId);
    // Taken BEFORE the read: a change landing during it stays pending for the
    // dirty re-run (whose diff read then comes after it).
    const had = this.livePending.has(repoId);
    const taken = had ? this.livePending.get(repoId) : new Set<string>();
    this.livePending.delete(repoId);
    try {
      await this.refreshStatus(repoId);
      // A ref change made OUTSIDE Otto (a terminal commit, an agent's session,
      // another git client fetching on its own timer) arrives only here:
      // `[]` = remote refs / tags only, `null` = HEAD / branch refs / unknown.
      // Re-sync the graph too — graphSync reads the cheap `/refs` first and
      // re-reads history only when something actually moved.
      if (had && (taken == null || taken.size === 0)) this.bumpRefs(repoId);
      this.liveChanged = { ...this.liveChanged, [repoId]: taken == null ? null : [...taken] };
      this.liveRev = { ...this.liveRev, [repoId]: (this.liveRev[repoId] ?? 0) + 1 };
    } finally {
      this.liveInFlight.delete(repoId);
      if (this.liveDirty.delete(repoId)) void this.liveRefresh(repoId);
    }
  }

  /** Did the latest live change of `repoId` touch `path`? (Unknown → yes.) */
  liveTouches(repoId: string, path: string): boolean {
    return touches(this.liveChanged[repoId], path);
  }

  /** Lazily load a repo's status once (used by the tab strip). `null` marks an
   *  in-flight / attempted load so we don't refetch on every render. */
  ensureStatus(repoId: string): void {
    if (repoId in this.statusById) return;
    this.statusById = { ...this.statusById, [repoId]: null };
    void this.refreshStatus(repoId);
  }

  /** Manual and background callers share one request per repo and two network
   *  slots per window. Errors reach manual callers; background workers swallow
   *  them. Every success invalidates refs, even when HEAD status is unchanged. */
  fetchRepo(repoId: string, reason: 'manual' | 'background' = 'manual'): Promise<RepoStatusResp> {
    // A manual request can promote a queued background fetch. Explicit work
    // remains valid even if its tab closes or automatic fetching is paused.
    if (reason === 'manual') {
      this.manualFetches.add(repoId);
      this.repoUsedAt[repoId] = Date.now();
    }
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
            this.bumpRefs(repoId);
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

  /** Owned by the shell while its visible module is Git. */
  startAutoFetch(): void {
    if (this.autoFetchRunning) return;
    this.autoFetchRunning = true;
    this.fetchOnArrival();
  }

  /** Wake auto-fetch on arrival (or fetch once when periodic polling is paused). */
  private fetchOnArrival(): void {
    if (this.autoFetchEnabled) {
      this.requestAutoFetch();
      return;
    }
    const id = this.activeRepoId;
    if (!this.autoFetchRunning || !id || !autoFetchAllowed()) return;
    const now = Date.now();
    if (this.fetches.has(id) || now < (this.retryFetchAt[id] ?? 0)) return;
    if (now - (this.lastFetchAt[id] ?? 0) < ACTIVE_AUTO_FETCH_SEC * 1000) return;
    // Explicit (manual-path) work: a paused loop would cancel a background one.
    this.fetchRepo(id).catch(() => {});
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
          const active = id === this.activeRepoId;
          // First sighting counts as a use, so restored tabs refresh once.
          const used = (this.repoUsedAt[id] ??= Date.now());
          if (!active && Date.now() - used > RECENT_USE_MS) continue;
          const interval = active ? ACTIVE_AUTO_FETCH_SEC : Math.max(60, this.autoFetchIntervalSec);
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
    writeAutoFetchConfig(this.autoFetchEnabled, this.autoFetchIntervalSec);
  }

  // ── Local merge + conflict resolution ─────────────────────────────────────

  /** Pull the repo, store fresh status, and bump `refsRev`. */
  async pull(repoId: Id, autoStash = false, mode?: PullMode): Promise<PullResp> {
    const body: PullReq = {};
    if (autoStash) body.auto_stash = true;
    if (mode) body.mode = mode;
    const r = await gitApi.pull(repoId, Object.keys(body).length ? body : undefined);
    this.setStatus(repoId, r.status);
    this.bumpRefs(repoId);
    return r;
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

  // ── Commit (lifted from WipPanel so an agent-driven commit — agent UI
  // control, lib/uiCommands/git.ts — takes the exact same path) ─────────────

  /** Commit the index. `sign` absent → the repo's `commit.gpgsign` decides.
   *  Refreshes the shared status and bumps `refsRev` so an open graph re-syncs
   *  (the branch ref moved). Throws on failure; the caller reports it. */
  async commit(
    repoId: Id,
    opts: { message: string; amend?: boolean; sign?: boolean },
  ): Promise<{ sha: string; status: RepoStatusResp }> {
    const body: { message: string; amend: boolean; sign?: boolean } = {
      message: opts.message,
      amend: opts.amend ?? false,
    };
    if (opts.sign !== undefined) body.sign = opts.sign;
    const r = await api.post<{ sha: string }>(`/repos/${repoId}/commit`, body);
    const status = await gitApi.status(repoId);
    this.setStatus(repoId, status);
    this.bumpRefs(repoId);
    return { sha: r.sha, status };
  }

  /** Stage (or unstage) paths; the fresh status is stored and returned. */
  async stage(repoId: Id, paths: string[], stage = true): Promise<RepoStatusResp> {
    const s = await gitApi.stage(repoId, paths, stage);
    this.setStatus(repoId, s);
    return s;
  }

  // Post-mutation contract: every push / checkout / pull — the toolbar, the
  // graph, Create PR or an agent command — stores the fresh status and bumps
  // `refsRev` HERE, so no call site can skip half of it (architecture-6-04).
  // `pushFlow.runPush` is the UX wrapper (toasts, rejected-push dialog).

  /** Push (or publish) a branch — the checked-out one unless `body.branch`. */
  async push(repoId: Id, body: PushReq = {}): Promise<RepoStatusResp> {
    const s = await gitApi.push(repoId, body);
    this.setStatus(repoId, s);
    this.bumpRefs(repoId);
    return s;
  }

  /** Switch (or create and switch to) a branch. */
  async checkout(repoId: Id, body: CheckoutReq): Promise<RepoStatusResp> {
    const s = await gitApi.checkout(repoId, body);
    this.setStatus(repoId, s);
    this.bumpRefs(repoId);
    return s;
  }

  // ── WIP-panel requests (agent UI control, TTL-bounded one-slot mailbox) ───
  wipRequest: WipRequest | null = $state(null);

  /** Ask the Git page to open `repoId`'s WIP panel (and select / prefill). */
  requestWip(repoId: Id, opts: Omit<WipRequest, 'repoId' | 'at'> = {}): void {
    this.wipRequest = { ...opts, repoId, at: Date.now() };
  }

  /** WipPanel: consume the pending request for `repoId` (null when none/stale). */
  takeWipRequest(repoId: Id): WipRequest | null {
    const r = this.wipRequest;
    if (!r || r.repoId !== repoId) return null;
    this.wipRequest = null;
    return Date.now() - r.at <= WIP_REQUEST_TTL_MS ? r : null;
  }
}

/** A pending WIP-panel request — see {@link GitStore.requestWip}. */
export interface WipRequest {
  repoId: Id;
  /** Select this file's diff. */
  path?: string | null;
  /** With `path`: show the staged side of a partially staged file. */
  staged?: boolean;
  /** Prefill the commit composer. */
  subject?: string;
  body?: string;
  at: number;
}

/** How long an untaken {@link WipRequest} stays applicable. */
const WIP_REQUEST_TTL_MS = 15_000;

export const git = new GitStore();
