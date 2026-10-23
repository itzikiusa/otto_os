import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

// architecture-1-09: GraphView's selection (git/graph/graphSelection.svelte.ts)
// and viewport (git/graph/graphViewport.svelte.ts) behind narrow hosts. The
// graph has no row-to-row arrow navigation (rows are Tab stops); what crosses
// the window edge is the jump to a commit that is not rendered yet — a sidebar
// ref click, `gitBridge.focus`, the deep link — so that is what is pinned.

const git = (name: string) => new URL(`../src/modules/git/${name}`, import.meta.url);
const svelte = { untrack: (fn: () => unknown) => fn(), tick: async () => {} };
const lanes = loadSource(git('graph-lanes.ts'), {});
const listWindow = loadSource(new URL('../src/lib/listWindow.svelte.ts', import.meta.url), {
  svelte,
  './findProviders': { registerFindProvider: () => () => {} },
});

const commit = (i: number) => ({
  sha: `s${i}`, short_sha: `s${i}`, subject: `c${i}`, author: 'a', date: '', refs: [] as string[],
  parents: [`s${i + 1}`],
});
const history = (n: number) => Array.from({ length: n }, (_, i) => commit(i));

function viewport(n: number) {
  const frames: (() => void)[] = [];
  const scrolled: string[] = [];
  const mod = loadSource(git('graph/graphViewport.svelte.ts'), {
    svelte,
    '../../../lib/listWindow.svelte': listWindow,
    '../graph-lanes': lanes,
  }, {
    window: {},
    requestAnimationFrame: (fn: () => void) => { frames.push(fn); return frames.length; },
    cancelAnimationFrame() {},
    CSS: { escape: (s: string) => s },
  });
  const vp = mod.createGraphViewport({
    commits: history(n), laneNames: new Map(), loadingMore: false, hasMore: false,
    loadMore() {}, flushResync() {},
  });
  // A 640 px scroller whose rows start 54 px down (sticky header + WIP row).
  const panel = {
    scrollTop: 0, clientHeight: 640, scrollHeight: n * 32,
    getBoundingClientRect: () => ({ top: 100 }),
    querySelector: (sel: string) => ({ scrollIntoView: () => scrolled.push(sel) }),
  };
  vp.panelEl = panel;
  vp.topSpacerEl = { getBoundingClientRect: () => ({ top: 154 - panel.scrollTop }) };
  return { vp, panel, frames, scrolled, mod };
}

test('jumping to a commit past the rendered window lands it mid-viewport, inside the next window', () => {
  const { vp, panel, frames, scrolled, mod } = viewport(3000);
  const idx = 2000;
  // Before the jump the target is far outside the window rendered at the top.
  const before = vp.windowAt(0);
  assert.ok(idx >= before.end, `row ${idx} already rendered (${before.start}-${before.end})`);
  vp.scrollToCommit(`s${idx}`);
  // Row offset from its INDEX at the fixed 32 px pitch, centred: 54 + 2000·32 − (640 − 32)/2.
  assert.equal(panel.scrollTop, 54 + idx * 32 - 304);
  const after = vp.windowAt(panel.scrollTop);
  assert.ok(after.start <= idx && idx < after.end, `row ${idx} not in ${after.start}-${after.end}`);
  assert.ok(after.start >= mod.OVERSCAN, 'the window moved with the scroll');
  // Next frame: corrected against the rendered row, then it pulses.
  assert.equal(vp.pulseSha, null);
  frames.shift()!();
  assert.deepEqual(scrolled, ['[data-sha="s2000"]']);
  assert.equal(vp.pulseSha, 's2000');
});

test('a jump near either edge of history never scrolls past the start', () => {
  const { vp, panel } = viewport(3000);
  vp.scrollToCommit('s2');
  assert.equal(panel.scrollTop, 0);
  const last = vp.windowAt(panel.scrollTop);
  assert.ok(last.start === 0 && last.end > 2);
  vp.scrollToCommit('s2999');
  assert.equal(panel.scrollTop, 54 + 2999 * 32 - 304);
  assert.equal(vp.windowAt(panel.scrollTop).end, 3000);
  // A sha that isn't laid out (not loaded) leaves the scroller alone.
  vp.scrollToCommit('missing');
  assert.equal(panel.scrollTop, 54 + 2999 * 32 - 304);
});

function selection(opts: { mobile?: boolean; loaded?: (sha: string) => boolean } = {}) {
  const log: string[] = [];
  const toasts: string[] = [];
  const commits = history(50);
  const bySha = new Map(commits.map((c) => [c.sha, c]));
  const mod = loadSource(git('graph/graphSelection.svelte.ts'), {
    svelte,
    '../../../lib/toast.svelte': { toasts: { error: (t: string, b: string) => toasts.push(`${t}: ${b}`), info: (t: string) => toasts.push(t) } },
    '../../../lib/stores/git.svelte': { git: { wipRequest: null } },
    '../../../lib/router.svelte': { router: { parts: [] } },
    '../gitBridge.svelte': { gitBridge: { focus: null } },
    '../deepLink': { routeGraphRef: () => null },
    '../graph-commit-diff.svelte': { prefetchCommitSummary: (_r: string, sha: string) => { log.push(`prefetch ${sha}`); return () => log.push(`abort ${sha}`); } },
  });
  const host = {
    repoId: 'r1', status: { changes: [] }, isMobile: !!opts.mobile, commitsOpen: true,
    refs: null, commits, commitsLoading: false, bySha, firstParentChild: new Map(),
    has: (sha: string) => bySha.has(sha),
    loadUntil: async (sha: string) => { log.push(`loadUntil ${sha}`); return (opts.loaded ?? ((s) => bySha.has(s)))(sha); },
    chipsFor: () => [],
    branchTipCommit: () => null,
    scrollToCommit: (sha: string) => log.push(`scroll ${sha}`),
    focusRow: (sha: string | null) => log.push(`focus ${sha}`),
  };
  return { sel: mod.createGraphSelection(host), host, log, toasts, commits, mod };
}

test('a sidebar branch click pages history in, selects the tip with its diff deferred, then scrolls', async (t) => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const { sel, log, mod } = selection();
  sel.selectBranchRow({ name: 'topic', sha: 's40', remote: false });
  await new Promise<void>((r) => queueMicrotask(r));
  await new Promise<void>((r) => queueMicrotask(r));
  assert.equal(sel.selectedSha, 's40');
  assert.deepEqual(log, ['loadUntil s40', 'prefetch s40', 'scroll s40']);
  // The diff waits out the double-click window, then loads.
  assert.equal(sel.diffSha, null);
  t.mock.timers.tick(mod.DIFF_DEFER_MS);
  assert.equal(sel.diffSha, 's40');
});

test('a double-click inside the deferral holds the diff and aborts the prefetch', (t) => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const { sel, log, commits, mod } = selection();
  sel.selectCommit(commits[3], { deferDiff: true });
  sel.holdDiff();
  t.mock.timers.tick(mod.DIFF_DEFER_MS * 2);
  assert.equal(sel.diffSha, null);
  assert.equal(sel.diffHeld, true);
  assert.deepEqual(log, ['prefetch s3', 'abort s3']);
  // Resuming loads it on demand.
  sel.scheduleDiff('s3', 0);
  assert.equal(sel.diffSha, 's3');
  assert.equal(sel.diffHeld, false);
});

test('an unreachable ref says so and leaves the selection alone', async () => {
  const { sel, log, toasts } = selection({ loaded: () => false });
  await sel.revealSha('dead', 'old-tag');
  assert.equal(sel.selectedSha, null);
  assert.deepEqual(log, ['loadUntil dead']);
  assert.deepEqual(toasts, ['Not in this graph: old-tag points at a commit that isn’t reachable here.']);
  assert.equal(sel.revealBusy, '');
});

test('on a phone, selecting folds the commit list and closing re-opens it; a second click deselects', () => {
  const { sel, host, commits } = selection({ mobile: true });
  sel.selectCommit(commits[1]);
  assert.equal(host.commitsOpen, false);
  assert.equal(sel.diffSha, 's1');
  sel.selectCommit(commits[1]);
  assert.equal(sel.selectedSha, null);
  assert.equal(sel.diffSha, null);
  assert.equal(host.commitsOpen, true);
  sel.selectWip();
  assert.equal(sel.wipSelected, true);
  assert.equal(host.commitsOpen, false);
  sel.clearSelection();
  assert.equal(sel.wipSelected, false);
  assert.equal(host.commitsOpen, true);
});

test('closing the detail pane hands focus to the row it described (the WIP row for WIP)', () => {
  const { sel, log, commits } = selection();
  sel.selectCommit(commits[7]);
  sel.closeDetail();
  assert.equal(sel.selectedSha, null);
  sel.selectWip();
  sel.closeDetail();
  assert.equal(sel.wipSelected, false);
  assert.deepEqual(log, ['focus s7', 'focus null']);
});

test('the viewport focuses that row once rendered, else the WIP row, else the scroller', async () => {
  const { vp, panel } = viewport(10);
  const focused: string[] = [];
  const el = (name: string) => ({ focus: () => focused.push(name) });
  let rendered = true;
  panel.querySelector = ((sel: string) => (rendered ? el(sel) : null)) as never;
  (panel as unknown as { focus: () => void }).focus = () => focused.push('panel');
  await vp.focusRow('s3');
  await vp.focusRow(null);
  rendered = false;
  await vp.focusRow('s4');
  assert.deepEqual(focused, ['[data-sha="s3"] .graph-select', '.wip-row', 'panel']);
});
