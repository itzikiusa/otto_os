// The git graph's quiet re-syncs (arch-1-09: split out of GraphView.svelte,
// via graphData): an auto-fetch round (`git.refsRev`), a refresh request from
// the toolbar / merge / recovery (`gitBridge.graphRefresh`), a re-sync parked
// while the graph was hidden, and the throttled background history reload.
// graphData owns the refs and history; this decides WHEN to re-read them.

import { untrack } from 'svelte';
import type { RefsResp } from '../../../lib/api/types';
import { git } from '../../../lib/stores/git.svelte';
import { gitBridge } from '../gitBridge.svelte';
import { withoutRemoteHeads } from '../ref-menus';

export interface GraphSyncHost {
  readonly repoId: string;
  readonly refs: RefsResp | null;
  /** Settles when the current repo's first load (or revalidation) is done. */
  readonly initialLoad: Promise<void>;
  graphVisible(): boolean;
  historyMoved(prev: RefsResp | null, next: RefsResp): boolean;
  setRefs(next: RefsResp): void;
  noteBackgroundRead(e: unknown | null): void;
  reloadGraph(): Promise<void>;
  refreshAfter(): Promise<void>;
}

// Background-triggered history re-reads (auto-fetch, repo watcher) are
// THROTTLED: each is a `log --all -n ≥10k` walk + 2–3 MB of JSON + a full
// lane relayout, and an agent committing in a loop or a fetch bringing one
// upstream commit at a time used to trigger one per event. At most one runs
// per BG_RELOAD_GAP_MS; triggers inside the gap (or during a run) coalesce
// into ONE trailing reload. User actions still reload immediately.
const BG_RELOAD_GAP_MS = 4_000;

export function createGraphSync(h: GraphSyncHost) {
  // Auto-fetch runs `fetch --prune` on the daemon but deliberately doesn't
  // remount the graph — re-sync QUIETLY (no loading flags, no flicker) when a
  // round completes, so a ref that moved outside this view (a teammate's branch,
  // a new tag, a branch deleted in a session terminal) shows up without a manual
  // Fetch. `resyncRefs` pays only the cheap `/refs` call unless something
  // actually moved. Plain (non-$state) bookkeeping: writing tracked state here
  // would re-trigger the effect.
  let refsSyncSeen: { id: string; rev: number } | null = null;
  /** This view just made the change the store's `refsRev` bump announces (and
   *  re-syncs itself right after): mark the bump seen, so it isn't re-read. */
  function adoptRefsRev(): void {
    refsSyncSeen = { id: h.repoId, rev: git.refsRev[h.repoId] ?? 0 };
  }
  $effect(() => {
    const id = h.repoId;
    const rev = git.refsRev[id] ?? 0;
    if (!refsSyncSeen || refsSyncSeen.id !== id) {
      // First observation for this repo — graphData's loader effect owns it.
      refsSyncSeen = { id, rev };
      return;
    }
    if (refsSyncSeen.rev !== rev) {
      refsSyncSeen = { id, rev };
      untrack(() => requestResync());
    }
  });

  // Toolbar / merge / recovery ask for a refresh through the bridge instead of
  // remounting this component. A request that predates this mount is already
  // covered by the mount's own load.
  let refreshSeen = gitBridge.graphRefresh?.nonce ?? 0;
  $effect(() => {
    const req = gitBridge.graphRefresh;
    if (!req || req.nonce === refreshSeen) return;
    refreshSeen = req.nonce;
    if (req.repoId !== h.repoId) return;
    untrack(() => void h.refreshAfter().catch(h.noteBackgroundRead));
  });

  // A graph nobody can see (window hidden, another module/sub-tab covering
  // it, the phone accordion folded) doesn't re-sync on every auto-fetch
  // round: the request is parked and replayed once it is visible again.
  let resyncPending = false;
  function requestResync(): void {
    if (!h.graphVisible()) {
      resyncPending = true;
      return;
    }
    resyncPending = false;
    void resyncRefs().catch(h.noteBackgroundRead);
  }
  function flushResync(): void {
    if (resyncPending && h.graphVisible()) requestResync();
  }
  $effect(() => {
    const onVis = () => flushResync();
    document.addEventListener('visibilitychange', onVis);
    return () => document.removeEventListener('visibilitychange', onVis);
  });


  /** Quiet post-auto-fetch re-sync: read the CHEAP `/refs` first and replay the
   *  EXPENSIVE fan-out (`log --all` + stashes + worktrees) only when history
   *  actually moved. `refsRev` is bumped after every successful auto-fetch —
   *  the store can't tell whether anything changed (`statusEq` sees only HEAD
   *  and its upstream), so the decision lives here, where the cheap-vs-expensive
   *  split is visible (investigation H4/WP3). */
  async function resyncRefs(): Promise<void> {
    const id = h.repoId;
    // Never race the mount's own first load: with nothing held yet every
    // response looks "moved" and would discard the in-flight first page.
    await h.initialLoad;
    if (id !== h.repoId) return;
    const { refs: raw, error: miss } = await git.refs(id);
    if (id !== h.repoId) return;
    // A miss keeps what we have and tries next round; two in a row say so.
    h.noteBackgroundRead(miss);
    if (!raw) return;
    const next = withoutRemoteHeads(raw);
    if (!h.historyMoved(h.refs, next)) {
      h.setRefs(next);
      return;
    }
    // History moved: land the new refs together WITH the re-read page (one
    // layout pass, whose name-change floor sees the moved refs). Applying them
    // first re-laid out every held row under the new names, and the splice
    // after it could then never converge with that pass.
    pendingRefs = { id, refs: next };
    await backgroundReload();
  }

  /** Refs read by {@link resyncRefs} that wait for the history re-read they
   *  triggered, so both land in one layout pass. */
  let pendingRefs: { id: string; refs: RefsResp } | null = null;
  function flushPendingRefs(): void {
    const p = pendingRefs;
    pendingRefs = null;
    if (p && p.id === h.repoId) h.setRefs(p.refs);
  }

  let bgLast = 0;
  let bgTimer: ReturnType<typeof setTimeout> | null = null;
  let bgRunning = false;
  let bgPending = false;
  async function backgroundReload(): Promise<void> {
    if (bgRunning) {
      // Throttled: don't hold the new refs back for the trailing reload.
      flushPendingRefs();
      bgPending = true;
      return;
    }
    const wait = bgLast + BG_RELOAD_GAP_MS - Date.now();
    if (wait > 0) {
      flushPendingRefs();
      bgTimer ??= setTimeout(() => {
        bgTimer = null;
        void backgroundReload();
      }, wait);
      return;
    }
    bgRunning = true;
    bgLast = Date.now();
    try {
      await h.reloadGraph();
    } finally {
      bgRunning = false;
      if (bgPending) {
        bgPending = false;
        void backgroundReload();
      }
    }
  }
  $effect(() => () => {
    if (bgTimer) clearTimeout(bgTimer);
    bgTimer = null;
  });

  return { adoptRefsRev, flushResync, flushPendingRefs, resyncRefs, backgroundReload };
}
