// The git graph's selection (arch-1-09: split out of GraphView.svelte): the
// selected commit or WIP row, the deferred diff load behind the detail pane,
// the first-parent "branch line" highlight, and the reveal that pages history
// in, selects a ref's commit and scrolls it into view (sidebar rows, the
// `gitBridge.focus` request and the `#/git/<repo>/graph/<branch>` deep link).

import { tick, untrack } from 'svelte';
import type { CommitInfo, RefBranch, RefTag, RefsResp, RepoStatusResp, StashInfo } from '../../../lib/api/types';
import { toasts } from '../../../lib/toast.svelte';
import { git } from '../../../lib/stores/git.svelte';
import { router } from '../../../lib/router.svelte';
import { gitBridge } from '../../../lib/diff/gitBridge.svelte';
import { routeGraphRef } from '../deepLink';
import { prefetchCommitSummary } from '../graph-commit-diff.svelte';
import type { RefChip } from '../ref-menus';

export interface GraphSelectionHost {
  readonly repoId: string;
  readonly status: RepoStatusResp;
  /** The detail pane would cover the graph (a sheet: a phone, a narrow
   *  tablet): History doesn't land on it, and a ref picked in the Branches
   *  list only shows its commit on the graph (a tap on the row opens it). */
  readonly compact: boolean;
  /** Phone: push the graph on screen before a jump from the Branches list. */
  showGraph(): Promise<void>;
  readonly refs: RefsResp | null;
  readonly commits: CommitInfo[];
  readonly commitsLoading: boolean;
  readonly bySha: Map<string, CommitInfo>;
  readonly firstParentChild: Map<string, string>;
  /** Is `sha` in the loaded history? */
  has(sha: string): boolean;
  loadUntil(sha: string): Promise<boolean>;
  chipsFor(c: CommitInfo): RefChip[];
  branchTipCommit(b: RefBranch): CommitInfo | null;
  scrollToCommit(sha: string): void;
  /** Focus the commit row for `sha` (null: the WIP row) once it renders. */
  focusRow(sha: string | null): void;
  isHeadCommit(c: CommitInfo): boolean;
}

export type GraphSelection = ReturnType<typeof createGraphSelection>;

/** How long a branch-row click holds its diff, so the lead click of a
 *  double-click (= checkout) never asks for one. */
export const DIFF_DEFER_MS = 200;

export function createGraphSelection(h: GraphSelectionHost) {
  let selectedSha = $state<string | null>(null);
  let selectedCommit = $state.raw<CommitInfo | null>(null);
  // The commit whose diff the detail pane loads (GraphCommitDiff owns the
  // fetches, their AbortController and the stale-response guard). It trails
  // `selectedSha` by DIFF_DEFER_MS on a branch-row click.
  let diffSha = $state<string | null>(null);
  /** Selected but deliberately not loaded (a double-click checked it out). */
  let diffHeld = $state(false);
  let diffTimer: ReturnType<typeof setTimeout> | null = null;

  // ── WIP (working tree) row selection ──────────────────────────────────────
  // GitKraken-style: a dirty working tree renders as a dashed row pinned above
  // the graph; selecting it opens the staging/commit panel in the detail pane
  // (this replaced the separate "Changes" view).
  let wipSelected = $state(false);
  /** Selected from the Branches list on a compact screen: marked, not opened. */
  let quiet = $state(false);
  /** The detail pane is open for EITHER a commit or the WIP row. */
  const detailOpen = $derived((selectedSha !== null && !quiet) || wipSelected);

  function selectWip(): void {
    if (wipSelected) clearSelection();
    else openWip();
  }
  function openWip(): void {
    wipSelected = true;
    selectedSha = null;
    selectedCommit = null;
    cancelDiff();
  }

  // A commit (or an outside `git commit`) can leave the tree clean while the
  // WIP panel is open — close it so the pane doesn't show a stale empty WIP.
  $effect(() => {
    if (wipSelected && h.status.changes.length === 0) wipSelected = false;
  });

  // Agent UI control: `git.requestWip` (lib/uiCommands/git.ts) opens this
  // repo's WIP panel; WipPanel then takes the request (file / composer).
  $effect(() => {
    const r = git.wipRequest;
    if (!r || r.repoId !== h.repoId || h.status.changes.length === 0) return;
    untrack(() => {
      if (!wipSelected) openWip();
    });
  });

  /** Select the checked-out commit (HEAD; the newest loaded one without it). */
  function selectHead(): void {
    const head = h.commits.find((c) => h.isHeadCommit(c)) ?? h.commits[0];
    if (head && selectedSha !== head.sha) selectCommit(head);
  }

  // After a commit from the WIP panel: select it once the graph has it (the
  // next step — Create pull request — sits on the checked-out commit).
  let committedSha = $state<string | null>(null);
  $effect(() => {
    const c = committedSha ? h.bySha.get(committedSha) : undefined;
    if (!c) return;
    committedSha = null;
    untrack(() => selectedSha !== c.sha && selectCommit(c));
  });

  /** Open the detail on the work: the WIP row with uncommitted changes, else
   *  the checked-out commit (landing, and ⌥⌘B on a closed pane). */
  function reopen(): void {
    if (h.status.changes.length > 0) openWip();
    else selectHead();
  }

  // Landing: History opens on the work, so the detail pane is never an empty
  // "select a commit" — where it would be a sheet over the graph it stays
  // shut (a phone lands on the Branches list). Once per mount; a branch deep
  // link or a reveal request then moves it.
  let landed = false;
  $effect(() => {
    if (landed) return;
    const dirty = h.status.changes.length > 0;
    if (!dirty && (h.commitsLoading || h.commits.length === 0)) return;
    landed = true;
    // After a tick: the Detail pane has measured its row by then (compact).
    untrack(() => void tick().then(() => {
      if (!wipSelected && !selectedSha && !h.compact) reopen();
    }));
  });

  /** Select a commit (opens the detail, lights its spine). `deferDiff` holds the
   *  diff load for DIFF_DEFER_MS — used by controls whose double-click checks
   *  out, so that gesture's lead click costs nothing. */
  function selectCommit(commit: CommitInfo, opts: { deferDiff?: boolean; quiet?: boolean } = {}): void {
    if (selectedSha === commit.sha) {
      // clicking again deselects (a quietly marked row opens instead)
      if (quiet && !opts.quiet) quiet = false;
      else clearSelection();
      return;
    }
    wipSelected = false;
    quiet = !!opts.quiet;
    selectedSha = commit.sha;
    selectedCommit = commit;
    scheduleDiff(commit.sha, opts.deferDiff ? DIFF_DEFER_MS : 0);
  }

  /** Abort for a summary prefetched during the double-click window; dropped
   *  (NOT aborted) once the pane mounts and joins it. */
  let prefetchStop: (() => void) | null = null;
  function scheduleDiff(sha: string, delay: number): void {
    cancelDiff();
    if (delay <= 0) {
      startDiff(sha);
      return;
    }
    // The pane waits out the double-click window, but its file list need
    // not: warm the (small, numstat-only) summary now so the list paints as
    // soon as the window closes instead of a round-trip after it. A
    // double-click (checkout) aborts it in `holdDiff` — only the summary is
    // ever asked for, never a patch.
    prefetchStop = prefetchCommitSummary(h.repoId, sha);
    diffTimer = setTimeout(() => {
      diffTimer = null;
      if (selectedSha === sha) {
        prefetchStop = null;
        startDiff(sha);
      }
    }, delay);
  }
  let diffStartedAt = 0;
  function startDiff(sha: string): void {
    diffStartedAt = performance.now();
    diffSha = sha;
  }

  /** Drop any pending/in-flight diff (unmounting the loader aborts it). */
  function cancelDiff(): void {
    if (diffTimer) clearTimeout(diffTimer);
    diffTimer = null;
    prefetchStop?.();
    prefetchStop = null;
    diffSha = null;
    diffHeld = false;
  }

  // A pending deferred-diff timer must not fire into an unmounted pane.
  $effect(() => () => {
    if (diffTimer) clearTimeout(diffTimer);
  });

  /** A checkout gesture (double-click) on the selected ref: never load its diff
   *  under the checkout — the pane offers it on demand instead. */
  function holdDiff(): void {
    // Pending, or started by the lead click of a SLOW double-click. A diff the
    // user has been reading for a while stays put.
    const recent = diffSha !== null && performance.now() - diffStartedAt < 600;
    if (diffTimer === null && !recent) return;
    cancelDiff();
    if (selectedSha) diffHeld = true;
  }

  function clearSelection(): void {
    wipSelected = false;
    quiet = false;
    selectedSha = null;
    selectedCommit = null;
    cancelDiff();
  }

  /** The detail pane's close (✕, the phone's diff header, WIP's close): the
   *  control removes itself, so focus moves to the row it described — the
   *  commit row, or the WIP row — never down to <body>. */
  function closeDetail(): void {
    const sha = selectedSha;
    clearSelection();
    h.focusRow(sha);
  }

  // ── Branch-line highlight (press a commit → see which branch it's on) ──────
  // Walk the FIRST-PARENT spine through the selected commit (ancestors via
  // parents[0], descendants via the first child whose parents[0] is this commit).
  // That contiguous spine reads as "the branch"; everything off it dims.
  const highlightSpine = $derived.by((): Set<string> | null => {
    if (!selectedSha || h.commits.length === 0) return null;
    const set = new Set<string>();
    let cur: string | undefined = selectedSha;
    while (cur && !set.has(cur)) {
      set.add(cur);
      cur = h.bySha.get(cur)?.parents[0];
    }
    cur = h.firstParentChild.get(selectedSha);
    while (cur && !set.has(cur)) {
      set.add(cur);
      cur = h.firstParentChild.get(cur);
    }
    return set;
  });
  // The branch the spine resolves to (for the detail-header "on <branch>" hint).
  // The ancestor half of the spine is exact; the first-parent DESCENDANT half is
  // best-effort (a commit has no single canonical branch — at a fork we follow
  // the newest child), so this hint is indicative, not authoritative.
  // First head/local ref found along the spine wins, else the first remote.
  const selectedBranch = $derived.by((): string | null => {
    const spine = highlightSpine;
    if (!spine) return null;
    let remoteFallback: string | null = null;
    for (const c of h.commits) {
      if (!spine.has(c.sha)) continue;
      for (const chip of h.chipsFor(c)) {
        if (chip.kind === 'head' || chip.kind === 'local') return chip.label;
        if (chip.kind === 'remote' && remoteFallback === null) remoteFallback = chip.label;
      }
    }
    return remoteFallback;
  });

  // ── Reveal a ref on the graph ─────────────────────────────────────────────
  /** Ref whose commit we're currently paging history to reach (drives the row's
   *  busy state on very deep jumps). */
  let revealBusy = $state('');

  /** Jump the graph to `sha`: page history in until the commit is loaded, select
   *  it (opens the detail + lights its spine) and scroll it into view. This is
   *  what a sidebar branch/tag click does — the point is to SHOW WHERE the ref
   *  sits, which a plain selection never did. */
  async function revealSha(sha: string, label: string, opts: { deferDiff?: boolean; quiet?: boolean } = {}): Promise<void> {
    if (!sha) return;
    revealBusy = label;
    try {
      const found = await h.loadUntil(sha);
      if (!found) {
        toasts.error('Not in this graph', `${label} points at a commit that isn’t reachable here.`);
        return;
      }
      const c = h.bySha.get(sha);
      if (!c) return;
      // Selecting is synchronous (highlight + detail header); the diff pane
      // loads on its own, deferred for a branch row whose double-click is a
      // checkout. Never await a diff before the graph moves.
      if (selectedSha !== c.sha) selectCommit(c, opts);
      h.scrollToCommit(sha);
    } finally {
      revealBusy = '';
    }
  }

  // A panel with no prop path to the graph (blame gutter, file history) asks it
  // to select a commit through `gitBridge`. `nonce` makes re-selecting the same
  // sha re-fire; a sha outside this repo's loaded history is a no-op with a
  // note, never a jump into the wrong repo's graph.
  let focusSeen = 0;
  $effect(() => {
    const req = gitBridge.focus;
    if (!req || req.nonce === focusSeen) return;
    focusSeen = req.nonce;
    if (req.repoId !== h.repoId) return;
    if (untrack(() => h.has(req.sha))) {
      void revealSha(req.sha, req.sha.slice(0, 8));
    } else {
      toasts.info('Commit not in loaded history', req.sha.slice(0, 8));
    }
  });

  /** Branch row SINGLE-click: JUMP to the branch's tip in the graph — scroll it
   *  into view, light its spine and open the commit detail — paging history in
   *  first if the tip is older than what's loaded. Checkout stays on
   *  double-click. Prefers the sha the refs response carries; falls back to
   *  matching decorations for daemons that predate the field. */
  async function selectBranchRow(b: RefBranch): Promise<void> {
    const sha = b.sha || h.branchTipCommit(b)?.sha;
    if (!sha) return;
    await h.showGraph();
    void revealSha(sha, b.name, { deferDiff: true, quiet: h.compact });
  }

  /** Tag row SINGLE-click: same jump-to-it-on-the-graph as a branch. The tag's
   *  action menu is on right-click (and the row's ⋯ button), because a
   *  left-click that only opened a menu never showed WHERE the tag is. */
  async function selectTagRow(t: RefTag): Promise<void> {
    const sha = t.sha || h.commits.find((c) => c.refs.some((r) => r === `tag: ${t.name}`))?.sha;
    if (!sha) return;
    await h.showGraph();
    void revealSha(sha, t.name, { quiet: h.compact });
  }

  /** Click a stash row → select its commit in the graph if it's present (the top
   *  stash usually is via `--all`; older reflog-only stashes won't be). */
  async function selectStash(s: StashInfo): Promise<void> {
    const c = h.bySha.get(s.sha);
    if (!c) return;
    await h.showGraph();
    if (selectedSha !== c.sha) selectCommit(c, { quiet: h.compact });
    h.scrollToCommit(c.sha);
  }

  // A deep link `#/git/<repo>/graph/<branch>` (Run with Otto's "View branch
  // diff", S20-301) jumps to that branch's tip — once per link — as soon as
  // the refs and the first history page are in.
  const routeRef = $derived(routeGraphRef(router.parts, h.repoId));
  let routeRefSeen = '';
  $effect(() => {
    const name = routeRef;
    const r: RefsResp | null = h.refs;
    if (!name) {
      routeRefSeen = ''; // leaving the link re-arms it for the next visit
      return;
    }
    if (!r || h.commitsLoading || name === routeRefSeen) return;
    routeRefSeen = name;
    untrack(() => {
      const b =
        r.local.find((x) => x.name === name) ??
        r.remote.find((x) => x.name === name) ??
        r.remote.find((x) => x.name.endsWith(`/${name}`));
      if (b) selectBranchRow(b);
      else toasts.info('Branch not found', `${name} isn’t in this repository any more (it may have been deleted).`);
    });
  });

  return {
    get selectedSha() { return selectedSha; },
    get selectedCommit() { return selectedCommit; },
    get diffSha() { return diffSha; },
    get diffHeld() { return diffHeld; },
    get wipSelected() { return wipSelected; },
    get detailOpen() { return detailOpen; },
    get highlightSpine() { return highlightSpine; },
    get selectedBranch() { return selectedBranch; },
    get revealBusy() { return revealBusy; },
    selectWip,
    reopen,
    /** A commit just made: select it once the graph has loaded it. */
    selectWhenLoaded(sha: string) { committedSha = sha; },
    selectCommit,
    scheduleDiff,
    holdDiff,
    clearSelection,
    closeDetail,
    revealSha,
    selectBranchRow,
    selectTagRow,
    selectStash,
  };
}
