// The git graph's data (arch-1-09: split out of GraphView.svelte): the refs
// tree, the paged commit history, the stash / worktree / submodule side lists,
// and every way they are (re)loaded — first mount, the per-repo snapshot,
// the refresh after a mutating action, and (through graphSync) the quiet
// auto-fetch re-syncs and throttled background reloads. GraphView passes a host (the repo id, the status
// sink, whether the graph is visible) and renders what this holds.

import { untrack } from 'svelte';
import { api } from '../../../lib/api/client';
import type {
  CleanupBaseResp,
  CommitInfo,
  RefBranch,
  RefsResp,
  RepoStatusResp,
  StashInfo,
  SubmoduleInfo,
  WorktreeInfo,
} from '../../../lib/api/types';
import { toasts } from '../../../lib/toast.svelte';
import { toastError } from '../../../lib/toastError';
import { loadErrorText } from '../../../lib/loadError';
import { git } from '../../../lib/stores/git.svelte';
import { gitBridge } from '../gitBridge.svelte';
import { graphCache, type GraphSnapshot } from '../graph-cache';
import { spliceHistory } from '../graph-splice';
import { headShaOf, withoutRemoteHeads } from '../ref-menus';
import { createGraphSync } from './graphSync.svelte';

/** GraphView's side of the data: which repo, where a returned status goes,
 *  and whether anyone can see the graph (a hidden one parks its re-syncs). */
export interface GraphDataHost {
  readonly repoId: string;
  onstatus(s: RepoStatusResp): void;
  graphVisible(): boolean;
}

// ── History paging ──────────────────────────────────────────────────────────
// There is no ceiling on how far back the graph can reach. The page is big
// enough that essentially every real repo loads its ENTIRE history in one
// request (6k commits = one page), so clicking a ref never waits on a chain of
// round-trips. Paging survives only as a backstop for genuinely huge repos —
// every row renders as real DOM, so an unbounded fetch on a 100k-commit repo
// would lock the webview on mount.
export const PAGE = 10000;
/** The first page of a cold mount, and what a background reload re-reads
 *  (perf R3): 2k commits paint at once; the next PAGE is prefetched right
 *  after, so a ref click still rarely waits. A reload splices this onto the
 *  held tail (graph-splice.ts). */
export const FIRST_PAGE = 2000;

export interface GraphFetch {
  gen: number;
  want: number;
  /** Set when the result was spliced: `hasMore` carries over from before. */
  more: boolean | null;
  commits: CommitInfo[] | null;
  error: unknown;
  stashes: StashInfo[] | null;
  worktrees: WorktreeInfo[] | null;
}

export type GraphData = ReturnType<typeof createGraphData>;

function sameJson(a: unknown, b: unknown): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

export function createGraphData(h: GraphDataHost) {
  // Server payloads below are `$state.raw`: they are only ever REPLACED, never
  // mutated in place, and a deep proxy over 10k commits (or a big refs list)
  // made every lane-layout pass 7-10x slower for no benefit.
  let refs: RefsResp | null = $state.raw(null);
  let refsLoading = $state(true);
  /** Human cause of a failed `/refs` load — the sidebar shows it with Retry
   *  instead of passing an empty tree off as "No local branches". */
  let refsError = $state<string | null>(null);

  let commits: CommitInfo[] = $state.raw([]);
  let commitsLoading = $state(true);
  /** The history load failed with nothing on screen — rendered inline with
   *  Retry, never as "No commits found." (a later-page failure keeps the rows
   *  and just retries on the next scroll). */
  let commitsError = $state<string | null>(null);
  /** Background / post-action refreshes that failed while history is on
   *  screen. One miss stays quiet (the next tick usually heals it); two in a
   *  row raise a slim "showing the last good load" bar with Retry instead of
   *  leaving the graph silently stale. */
  let refreshFailures = 0;
  let refreshError = $state<string | null>(null);
  /** The same rule for the quiet `/refs` + side-list re-reads (auto-fetch,
   *  revalidate, post-action refresh), counted on their own: a miss used to
   *  vanish into `.catch(() => {})`, leaving branches stale with nothing on
   *  screen. Any success clears it. */
  let refsMisses = 0;
  let refsStaleError = $state<string | null>(null);

  /** False once a short page comes back — that's the root of history. */
  let hasMore = $state(true);
  /** Guards against overlapping page fetches (scroll fires far faster than IO). */
  let loadingMore = $state(false);
  /** Bumped on every repo switch / full reload so an in-flight page from the
   *  PREVIOUS repo can't append its commits onto the new one. */
  let loadGen = 0;
  /** How many rows have been REQUESTED so far — the `skip` cursor. Deliberately
   *  not `commits.length`: dedupe can drop rows from a page, and reusing the
   *  deduped length would re-request the same offset forever (a page of pure
   *  duplicates would spin `loadUntil` without ever advancing). */
  let skipCursor = 0;
  /** The page fetch currently in flight, so a ref-click that needs more history
   *  JOINS it instead of bailing out (bailing made a click during a scroll-load
   *  falsely report the commit as unreachable). */
  let inflight: Promise<boolean> | null = null;

  // ── Stashes (read-only `git stash list`), worktrees + submodules ─────────
  // Sidebar sections; best-effort — a failure just leaves a section empty.
  let stashes: StashInfo[] = $state.raw([]);
  /** The stash list has been read for the current repo (vs the reset `[]`). */
  let stashesKnown = $state(false);
  let worktrees: WorktreeInfo[] = $state.raw([]);
  let submodules: SubmoduleInfo[] = $state.raw([]);

  function noteBackgroundRead(e: unknown | null): void {
    if (e === null) {
      refsMisses = 0;
      refsStaleError = null;
    } else if (++refsMisses >= 2) {
      refsStaleError = loadErrorText(e);
    }
  }

  /** Append the next page of history. Returns false when nothing more arrived.
   *  `until` (a sha) makes it ONE daemon walk that stops right after that
   *  commit — how a jump to an old ref reaches it, instead of a chain of
   *  10k-commit `--skip` pages that each re-walk everything before them. */
  function loadMore(until?: string, limit = PAGE): Promise<boolean> {
    if (inflight) return inflight;
    if (!hasMore) return Promise.resolve(false);
    const p = loadPage(until, limit);
    inflight = p;
    loadingMore = true;
    // Release the slot by PROMISE IDENTITY, never by generation: a reload that
    // bumps `loadGen` while a page is in flight would otherwise strand `inflight`
    // set forever, and every later loadMore() would hand back this one stale,
    // already-settled promise — freezing history at whatever had loaded.
    void p.finally(() => {
      if (inflight === p) {
        inflight = null;
        loadingMore = false;
      }
    });
    return p;
  }

  /** (Re)load the refs tree. A failure keeps an empty tree (the graph's chip
   *  lookups need one) but records the cause so the sidebar can say so. */
  function loadRefs(id: string): Promise<void> {
    refsLoading = true;
    refsError = null;
    return git
      .refs(id)
      .then(({ refs: r, error }) => {
        if (id !== h.repoId) return;
        if (r) setRefs(withoutRemoteHeads(r));
        else {
          setRefs({ local: [], remote: [], tags: [] });
          refsError = loadErrorText(error);
        }
      })
      .finally(() => (refsLoading = false));
  }

  /** JSON of the refs currently held — compared before every assignment so a
   *  no-op refresh (the 30 s auto-fetch round, most checkouts) never hands the
   *  lane layout, chips and sidebar a new object to recompute from. */
  let refsKey = '';
  /** Assign `refs` only when the content changed. */
  function setRefs(next: RefsResp): void {
    const key = JSON.stringify(next);
    if (refs !== null && key === refsKey) return;
    if (refs !== null && !refRemoved) refRemoved = lostRef(refs, next);
    refsKey = key;
    refs = next;
  }

  /** A removed ref, or a moved ref outside the prefix checked by the splice,
   *  can orphan commits in the retained tail. Re-read that history in full.
   *  A NEW ref (a fetched branch, a fork remote) can make commits reachable
   *  that are older than the fresh page and absent from the held tail: its
   *  tip must land in the fresh prefix, or the history is re-read too. */
  let refRemoved = false;
  const movedRefTips = new Set<string>();
  const newRefTips = new Set<string>();
  function lostRef(prev: RefsResp, next: RefsResp): boolean {
    const entries = (r: RefsResp): [string, string | undefined][] => [
      ...r.local.map((b): [string, string | undefined] => [`l:${b.name}`, b.sha]),
      ...r.remote.map((b): [string, string | undefined] => [`r:${b.name}`, b.sha]),
      ...r.tags.map((t): [string, string | undefined] => [`t:${t.name}`, t.sha]),
    ];
    const have = new Map(entries(next));
    for (const [key, sha] of entries(prev)) {
      if (sha && have.has(key) && have.get(key) !== sha) movedRefTips.add(sha);
    }
    const had = new Set(entries(prev).map(([key]) => key));
    for (const [key, sha] of have) if (sha && !had.has(key)) newRefTips.add(sha);
    const checked = new Set(commits.slice(0, FIRST_PAGE).map((c) => c.sha));
    return entries(prev).some(([key, sha]) => !have.has(key) ||
      (have.get(key) !== sha && (!sha || !checked.has(sha))));
  }

  /** Retry a failed first page (the inline error's Retry). */
  function retryCommits(): void {
    commitsLoading = true;
    hasMore = true;
    void loadMore().finally(() => (commitsLoading = false));
  }

  /** Every sha in `commits` (plus any batched page not flushed yet). Kept
   *  across pages so dedupe and "is it loaded?" are O(1) instead of an O(N)
   *  rebuild per page. Rebuilt whenever `commits` is replaced wholesale. */
  let seenShas = new Set<string>();
  /** >0 while `loadUntil` pages toward an old commit: pages collect in
   *  `pendingPages` and land in ONE assignment (one lane layout), not one
   *  full re-layout per 10k-commit page. */
  let batching = 0;
  let pendingPages: CommitInfo[] = [];
  let pendingGen = 0;

  /** Replace the whole history (first page / reload / cache restore). */
  function setCommits(next: CommitInfo[]): void {
    pendingPages = [];
    seenShas = new Set(next.map((c) => c.sha));
    commits = next;
  }

  function appendCommits(fresh: CommitInfo[]): void {
    for (const c of fresh) seenShas.add(c.sha);
    if (batching > 0) {
      if (pendingGen !== loadGen) pendingPages = [];
      pendingGen = loadGen;
      for (const c of fresh) pendingPages.push(c);
    } else {
      commits = commits.concat(fresh);
    }
  }

  function flushPending(): void {
    if (pendingPages.length > 0 && pendingGen === loadGen) commits = commits.concat(pendingPages);
    pendingPages = [];
  }

  async function loadPage(until?: string, limit = PAGE): Promise<boolean> {
    const gen = loadGen;
    try {
      // Only a sha is a valid target (the daemon 400s anything else).
      const to = until && /^[0-9a-f]{4,64}$/i.test(until) ? `&until=${until}` : '';
      const page = await api.get<CommitInfo[]>(
        `/repos/${h.repoId}/log?all=true&limit=${limit}&skip=${skipCursor}${to}`,
      );
      if (gen !== loadGen) return false; // repo changed under us — drop it
      commitsError = null;
      skipCursor += page.length;
      if (page.length < limit) hasMore = false;
      if (page.length > 0) {
        // `--all` + skip can re-emit a commit if refs moved between pages; dedupe
        // so the lane algorithm never sees the same sha twice.
        const fresh = page.filter((c) => !seenShas.has(c.sha));
        if (fresh.length > 0) appendCommits(fresh);
      }
      return page.length > 0;
    } catch (e) {
      // Leave `hasMore` alone: a transient failure shouldn't permanently declare
      // the end of history — the next scroll retries.
      if (gen === loadGen && commits.length === 0) commitsError = loadErrorText(e);
      return false;
    }
  }

  /** Page forward until `sha` is loaded (or history runs out). Used when a ref
   *  the user clicked points at a commit older than everything loaded so far. */
  async function loadUntil(sha: string): Promise<boolean> {
    if (seenShas.has(sha)) return true;
    batching++;
    try {
      // Usually one round trip: the `until` walk ends AT the commit. The
      // loop only repeats when it joined a scroll page already in flight.
      while (hasMore) {
        if (!(await loadMore(sha))) break;
        if (seenShas.has(sha)) break;
      }
    } finally {
      batching--;
      if (batching === 0) flushPending();
    }
    return seenShas.has(sha);
  }

  /** Settles when the current repo's first load (or cache revalidation) is
   *  done. The auto-fetch re-sync waits on it, so a fetch round landing
   *  mid-mount can't throw away the in-flight first page and re-request it. */
  let initialLoad: Promise<void> = Promise.resolve();

  $effect(() => {
    const id = h.repoId;
    // UNTRACKED: the loaders read `hasMore` (and pages write it). Reading that
    // inside the effect would subscribe this loader to it — so the first short
    // page (`hasMore = false`) re-ran the whole effect, which resets `commits`
    // and re-fetches skip=0, silently throwing away every page the user had
    // scrolled in. The repo id read above is this effect's only real dependency.
    untrack(() => {
      initialLoad = mountRepo(id);
    });
  });

  async function mountRepo(id: string): Promise<void> {
    // Reset paging for the new repo and invalidate any in-flight page.
    loadGen++;
    loadingMore = false;
    inflight = null;
    commitsError = null;
    refsError = null;
    refreshFailures = 0;
    refreshError = null;

    const snap = graphCache.get(id);
    if (snap) {
      // Stale-while-revalidate: paint the last graph for this repo now, then
      // let the cheap `/refs` check decide whether history needs a re-read.
      refsKey = '';
      setRefs(snap.refs);
      setCommits(snap.commits);
      stashes = snap.stashes;
      stashesKnown = snap.stashesKnown;
      worktrees = snap.worktrees;
      submodules = snap.submodules;
      hasMore = snap.hasMore;
      skipCursor = snap.skipCursor;
      refsLoading = false;
      commitsLoading = false;
      await revalidate(id);
      return;
    }

    refsLoading = true;
    commitsLoading = true;
    refs = null;
    setCommits([]);
    stashes = [];
    hasMore = true;
    skipCursor = 0;

    // Stashes / worktrees / submodules are best-effort: a failure (or empty
    // list) just leaves the section empty; it must never block the graph.
    stashesKnown = false;
    worktrees = [];
    submodules = [];
    void reloadSideLists(id, true);
    const gen = loadGen;
    await Promise.all([
      loadRefs(id),
      loadMore(undefined, FIRST_PAGE).finally(() => {
        commitsLoading = false;
        // Prefetch the next page once the first one has painted.
        setTimeout(() => {
          if (gen === loadGen && id === h.repoId && hasMore) void loadMore();
        }, 0);
      }),
    ]);
  }

  /** Stashes + worktrees + submodules — not part of the refs fingerprint, so a
   *  revalidation re-reads them quietly (each assignment is skipped when
   *  nothing changed). */
  async function reloadSideLists(id: string, withSubmodules: boolean): Promise<void> {
    let miss: unknown | null = null;
    const fail = (e: unknown): void => {
      miss ??= e;
    };
    await Promise.all([
      api
        .get<StashInfo[]>(`/repos/${id}/stashes`)
        .then((s) => {
          if (id !== h.repoId) return;
          if (!sameJson(stashes, s)) stashes = s;
          stashesKnown = true;
        })
        .catch(fail),
      api
        .get<WorktreeInfo[]>(`/repos/${id}/worktrees`)
        .then((w) => {
          if (id === h.repoId && !sameJson(worktrees, w)) worktrees = w;
        })
        .catch(fail),
      withSubmodules
        ? api
            .get<SubmoduleInfo[]>(`/repos/${id}/submodules`)
            .then((s) => {
              if (id === h.repoId && !sameJson(submodules, s)) submodules = s;
            })
            .catch(fail)
        : Promise.resolve(),
    ]);
    if (id === h.repoId) noteBackgroundRead(miss);
  }

  /** After painting a cached graph: refs first, history only if it moved. */
  async function revalidate(id: string): Promise<void> {
    const { refs: raw, error: miss } = await git.refs(id);
    if (id !== h.repoId) return;
    noteBackgroundRead(miss);
    if (raw && applyRefs(withoutRemoteHeads(raw))) {
      await reloadGraph();
      return;
    }
    await reloadSideLists(id, true);
  }

  // Keep the per-repo snapshot current (see graph-cache.ts). Only a complete,
  // error-free state is worth restoring.
  $effect(() => {
    const id = h.repoId;
    const r = refs;
    const c = commits;
    const st = stashes;
    const wt = worktrees;
    const sm = submodules;
    const more = hasMore;
    const known = stashesKnown;
    if (!r || refsError || commitsError || refsLoading || commitsLoading || c.length === 0) return;
    untrack(() =>
      graphCache.set(id, {
        refs: r,
        commits: c,
        stashes: st,
        stashesKnown: known,
        worktrees: wt,
        submodules: sm,
        hasMore: more,
        skipCursor,
      } satisfies GraphSnapshot),
    );
  });

  // Publish the stash count for the toolbar's Pop (see gitBridge.stashCount).
  // Only once the list is known for THIS repo — the loader resets it to [].
  $effect(() => {
    const id = h.repoId;
    const n = stashes.length;
    if (stashesKnown) gitBridge.stashCount[id] = n;
  });

  // Quiet re-syncs: auto-fetch rounds, bridge refresh requests, parked
  // re-syncs of a hidden graph and the throttled background reload
  // (graphSync.svelte.ts).
  const sync = createGraphSync({
    get repoId() { return h.repoId; },
    get refs() { return refs; },
    get initialLoad() { return initialLoad; },
    graphVisible: () => h.graphVisible(),
    historyMoved,
    setRefs,
    noteBackgroundRead,
    reloadGraph: () => self.reloadGraph(),
    refreshAfter: () => refreshAfter(),
  });

  /** Does moving from `prev` to `next` change what `log --all` returns? Only
   *  when a ref was REMOVED or MOVED (commits may become unreachable or new
   *  ones appear), a new ref points outside the loaded history, or the tags
   *  (whose decorations come from the log) changed. A plain checkout (only
   *  `is_current` flips), or a new branch/remote on an already-loaded commit,
   *  moves nothing — HEAD and branch chips come from the typed refs. `true`
   *  whenever it can't tell (nothing held, or a daemon predating
   *  `RefBranch.sha`), which degrades to an unconditional re-read. */
  function historyMoved(prev: RefsResp | null, next: RefsResp): boolean {
    if (!prev) return true;
    const pb = [...prev.local, ...prev.remote];
    const nb = [...next.local, ...next.remote];
    if (pb.some((b) => !b.sha) || nb.some((b) => !b.sha)) return true;
    const key = (b: RefBranch) => `${b.remote ? 'r' : 'l'}:${b.name}`;
    const nextSha = new Map(nb.map((b) => [key(b), b.sha]));
    for (const b of pb) if (nextSha.get(key(b)) !== b.sha) return true;
    // Only a NEW ref can bring in unseen history (moved/removed ones were
    // caught above). An UNCHANGED ref whose tip lies beyond the loaded page
    // (a stale branch older than the first 10k commits) must not count: a
    // re-read with the same `limit` would never page it in, so treating it
    // as "moved" re-read `log --all` on every auto-fetch / checkout / mount.
    const prevKeys = new Set(pb.map(key));
    for (const b of nb) if (!prevKeys.has(key(b)) && !seenShas.has(b.sha!)) return true;
    const tags = (r: RefsResp) => r.tags.map((t) => `${t.name}@${t.sha ?? ''}`).join('\n');
    return tags(prev) !== tags(next);
  }

  /** Take a fresh refs response; returns whether the history must be re-read. */
  function applyRefs(next: RefsResp): boolean {
    const moved = historyMoved(refs, next);
    setRefs(next);
    return moved;
  }


  /** After a mutating graph op: propagate the returned status (if any) to the
   *  parent and re-query refs + log so the graph reflects the change. Both
   *  responses are applied in ONE synchronous block, so the lane layout runs
   *  once instead of once per assignment. */
  async function refreshAfter(status?: RepoStatusResp): Promise<void> {
    if (status) h.onstatus(status);
    const id = h.repoId;
    const [{ refs: r, error: miss }, g] = await Promise.all([git.refs(id), fetchGraph(id)]);
    if (id !== h.repoId) return;
    noteBackgroundRead(miss);
    if (r) setRefs(withoutRemoteHeads(r));
    applyGraph(g);
  }

  /** After a checkout: the switch moves no commit, so read `/refs` (HEAD and
   *  the current branch come from it) and re-read history only if it moved —
   *  not `log --all -n 10000` just to move the `HEAD ->` decoration. */
  async function refreshAfterCheckout(status: RepoStatusResp, stashed: boolean): Promise<void> {
    h.onstatus(status);
    const id = h.repoId;
    const hadHead = headShaOf(refs) !== null;
    const { refs: raw } = await git.refs(id);
    if (id !== h.repoId) return;
    // A miss falls back to the full refresh, which records it.
    if (!raw) {
      await refreshAfter();
      return;
    }
    const moved = applyRefs(withoutRemoteHeads(raw));
    // Detached before or after: HEAD then lives only in the log decorations.
    if (moved || !hadHead || headShaOf(refs) === null) {
      await self.reloadGraph();
      return;
    }
    if (stashed) await reloadSideLists(id, false);
  }

  /** The EXPENSIVE half of a refresh: the `log --all` page plus stashes and
   *  worktrees. Fetch only — [`applyGraph`] assigns. */
  async function fetchGraph(id: string, full = false): Promise<GraphFetch> {
    // Read ONE page and splice it onto the history already held (older
    // commits are immutable — see graph-splice.ts). Only when the splice
    // can't be proven safe (rewritten/deleted history inside the page) does
    // it re-read exactly as much as the user had paged in, so a refresh never
    // yanks the graph back to the first page — and never MORE: the old
    // `ceil((n + 1) / PAGE)` grew the request by a page per refresh.
    const held = commits;
    const heldMore = hasMore;
    loadGen++;
    loadingMore = false;
    inflight = null;
    const gen = loadGen;
    const readLog = (limit: number) =>
      api
        .get<CommitInfo[]>(`/repos/${id}/log?all=true&limit=${limit}`)
        .then((v) => ({ v, e: null as unknown }))
        .catch((e: unknown) => ({ v: null, e }));
    if (full) refRemoved = false;
    const [c0, st, wt] = await Promise.all([
      readLog(full ? Math.max(PAGE, held.length) : FIRST_PAGE),
      api.get<StashInfo[]>(`/repos/${id}/stashes`).catch(() => null),
      api.get<WorktreeInfo[]>(`/repos/${id}/worktrees`).catch(() => null),
    ]);
    let c = c0;
    let want = full ? Math.max(PAGE, held.length) : FIRST_PAGE;
    let more: boolean | null = null;
    if (!full && c.v && c.v.length >= FIRST_PAGE && held.length > FIRST_PAGE) {
      const spliced = spliceHistory(held, c.v);
      if (spliced) {
        c = { v: spliced, e: null };
        more = heldMore; // the tail (and whether more exists) is unchanged
      } else if (gen === loadGen) {
        want = held.length;
        c = await readLog(want);
      }
    }
    return { gen, want, more, commits: c.v, error: c.e, stashes: st, worktrees: wt };
  }

  function applyGraph(g: GraphFetch): void {
    if (g.gen !== loadGen) return; // a newer reload / repo switch owns the state
    const freshPrefix = new Set(g.commits?.slice(0, FIRST_PAGE).map((c) => c.sha));
    const outside = (tips: Set<string>): boolean => [...tips].some((sha) => !freshPrefix.has(sha));
    if (g.more !== null && (refRemoved || outside(movedRefTips) || outside(newRefTips))) {
      // Spliced, but a ref vanished meanwhile (refs land beside the log in
      // refreshAfter): the held tail may hold commits nothing reaches now.
      // Or a new ref's tip is below the fresh page: its commits are in
      // neither the page nor the held tail.
      void self.reloadGraph(true);
      return;
    }
    if (g.commits) {
      movedRefTips.clear();
      newRefTips.clear();
      setCommits(g.commits);
      commitsError = null;
      skipCursor = g.commits.length;
      hasMore = g.more ?? g.commits.length >= g.want;
      refreshFailures = 0;
      refreshError = null;
    } else if (commits.length === 0) {
      commitsError = loadErrorText(g.error);
    } else if (++refreshFailures >= 2) {
      refreshError = loadErrorText(g.error);
    }
    if (g.stashes) {
      if (!sameJson(stashes, g.stashes)) stashes = g.stashes;
      stashesKnown = true;
    }
    if (g.worktrees && !sameJson(worktrees, g.worktrees)) worktrees = g.worktrees;
  }

  async function reloadGraph(full = false): Promise<void> {
    const id = h.repoId;
    const g = await fetchGraph(id, full);
    if (id !== h.repoId) return;
    // Same synchronous block as the history, so the layout runs once.
    sync.flushPendingRefs();
    applyGraph(g);
  }

  /** Run a mutating POST that returns RepoStatusResp, then refresh + toast.
   *  Errors are surfaced via toast and swallowed (the graph is left untouched). */
  async function mutate(path: string, body: unknown, okTitle: string, okDetail?: string): Promise<void> {
    try {
      const s = await api.post<RepoStatusResp>(`/repos/${h.repoId}${path}`, body);
      await refreshAfter(s);
      // A 200 that left an operation mid-flight (a conflicting cherry-pick /
      // revert) is NOT a success — say what happened and where to finish it.
      if (s.op_in_progress) {
        toasts.warn(
          `${okTitle} — conflicts`,
          'Unresolved conflicts remain. Open "Resolve conflicts" to finish or abort.',
        );
      } else {
        toasts.success(okTitle, okDetail);
      }
    } catch (e) {
      toastError('Couldn’t complete the Git action', e);
      // A failed mutation can still have partially landed — most notably a
      // "local + remote" delete whose LOCAL half succeeded before the remote
      // push errored. Re-sync refs/log so the graph reflects what's actually
      // there instead of showing a branch that's already gone (and offering to
      // delete it again). Best-effort: the toast above already reported the
      // failure, so a refresh error here must stay silent.
      await refreshAfter().catch(() => {});
    }
  }

  // ── Cleanup base branch (drives the "merged → safe to delete" indicators) ──
  // The resolved base comes back on every refs response (`base_branch`); a
  // branch is flagged when its tip is contained in that base. Changing the
  // base persists per-repo and re-derives the flags by refetching refs.
  /** Set (or clear, on null) the per-repo cleanup base branch. Indicator-only
   *  — never deletes a branch. */
  async function setCleanupBase(name: string | null): Promise<void> {
    try {
      const resp = await api.put<CleanupBaseResp>(`/repos/${h.repoId}/cleanup-base`, {
        base_branch: name,
      });
      await refreshAfter();
      toasts.success(name ? 'Cleanup base branch set' : 'Cleanup base reset', resp.resolved ?? 'default branch');
    } catch (e) {
      toastError('Couldn’t set the cleanup base branch', e);
    }
  }

  const self = {
    get refs() { return refs; },
    get refsLoading() { return refsLoading; },
    get refsError() { return refsError; },
    get refsStaleError() { return refsStaleError; },
    get commits() { return commits; },
    get commitsLoading() { return commitsLoading; },
    get commitsError() { return commitsError; },
    get refreshError() { return refreshError; },
    get hasMore() { return hasMore; },
    get loadingMore() { return loadingMore; },
    get stashes() { return stashes; },
    get worktrees() { return worktrees; },
    set worktrees(v: WorktreeInfo[]) { worktrees = v; },
    get submodules() { return submodules; },
    set submodules(v: SubmoduleInfo[]) { submodules = v; },
    /** Is `sha` in the loaded history (a batched page counts)? */
    has(sha: string): boolean { return seenShas.has(sha); },
    noteBackgroundRead,
    loadMore,
    loadRefs,
    loadUntil,
    setRefs,
    lostRef,
    retryCommits,
    adoptRefsRev: () => sync.adoptRefsRev(),
    flushResync: () => sync.flushResync(),
    refreshAfter,
    refreshAfterCheckout,
    applyGraph,
    /** Replaceable so a test can observe the full re-read `applyGraph` asks for. */
    reloadGraph,
    mutate,
    setCleanupBase,
  };
  return self;
}
