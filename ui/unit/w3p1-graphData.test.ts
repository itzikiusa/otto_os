import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

// architecture-1-09: the refs + history state left GraphView for
// git/graph/graphData.svelte.ts. The splice-safety rules that
// gitGraphSplice.test.ts pinned on the component (lostRef / setRefs /
// applyGraph) run here against the real module: a moved, removed or NEW ref
// whose tip is outside the fresh page forces a full re-read of the history.

const git = (name: string) => new URL(`../src/modules/git/${name}`, import.meta.url);
const list = (...shas: string[]) => shas.map((sha, i) => ({
  sha, short_sha: sha, subject: sha, author: 'a', date: '', refs: [], parents: shas[i + 1] ? [shas[i + 1]] : [],
}));

function data(firstPage: number) {
  const src = loadSource(git('graph/graphData.svelte.ts'), {
    svelte: { untrack: (fn: () => unknown) => fn() },
    '../../../lib/api/client': { api: {} },
    '../../../lib/toast.svelte': { toasts: {} },
    '../../../lib/toastError': { toastError() {} },
    '../../../lib/stores/git.svelte': { git: { refsRev: {} } },
    '../../../lib/api/gitOverview': { readOverview: () => Promise.reject(new Error('no overview in this fixture')) },
    '../../../lib/diff/gitBridge.svelte': { gitBridge: { stashCount: {} } },
    '../graph-cache': { graphCache: { get: () => null, set() {} } },
    '../graph-splice': { spliceHistory: () => null },
    '../ref-menus': loadSource(git('ref-menus.ts'), {}),
    './graphSync.svelte': { createGraphSync: () => ({ flushPendingRefs() {}, adoptRefsRev() {}, flushResync() {} }) },
  });
  // Exercise the rules at a 2-commit "first page".
  const d = src.createGraphData({ repoId: 'r1', onstatus() {}, graphVisible: () => true });
  assert.equal(src.FIRST_PAGE, 200);
  const reloads: boolean[] = [];
  d.reloadGraph = async (full = false) => { reloads.push(full); };
  // Seed `commits` the way a reload lands them (gen 0 = the current load).
  const seed = (commits: ReturnType<typeof list>) =>
    d.applyGraph({ gen: 0, want: firstPage, more: null, commits, error: null, stashes: null, worktrees: null });
  return { d, reloads, seed };
}

// graphData checks moved tips against its FIRST_PAGE (2000) prefix: pad the
// histories so "outside the fresh page" means past 2000 commits.
const pad = (n: number, tag: string) => Array.from({ length: n }, (_, i) => `${tag}${i}`);

test('rewinding a same-name ref outside the fresh prefix invalidates the retained tail', () => {
  const { d, seed } = data(2000);
  seed(list(...pad(2000, 'recent-'), 'old-tip', 'base'));
  const refs = (sha: string) => ({ local: [{ name: 'old-branch', sha }], remote: [], tags: [] });
  assert.equal(d.lostRef(refs('old-tip'), refs('base')), true);
  assert.equal(d.lostRef(refs('old-tip'), refs('old-tip')), false);
  // Fast growth whose old tip is inside the checked prefix keeps the splice path.
  assert.equal(d.lostRef(refs('recent-0'), refs('new-tip')), false);
  assert.equal(d.lostRef({ local: [], remote: [], tags: [{ name: 'v1', sha: 'old-tip' }] },
    { local: [], remote: [], tags: [{ name: 'v1', sha: 'base' }] }), true);
});

test('a moved tip displaced from the fresh page also invalidates a spliced tail', () => {
  const { d, reloads, seed } = data(2000);
  seed(list(...pad(1999, 'recent-'), 'old-tip', 'base'));
  d.setRefs({ local: [{ name: 'side', sha: 'old-tip' }], remote: [], tags: [] });
  d.setRefs({ local: [{ name: 'side', sha: 'base' }], remote: [], tags: [] });
  // New unrelated commits pushed the old tip below the fresh boundary.
  d.applyGraph({ gen: 0, want: 2000, more: false, commits: list('new', ...pad(1999, 'recent-'), 'old-tip', 'base'), error: null, stashes: null, worktrees: null });
  assert.deepEqual(reloads, [true]);
});

test('a NEW ref whose tip is older than the fresh page re-reads the history in full', () => {
  const { d, reloads, seed } = data(2000);
  seed(list(...pad(2000, 'recent-'), 'old-feat'));
  d.setRefs({ local: [{ name: 'main', sha: 'recent-0' }], remote: [], tags: [] });
  // A fetched branch whose (weeks-old) commits sit below the page.
  d.setRefs({ local: [{ name: 'main', sha: 'recent-0' }], remote: [{ name: 'origin/old-feature', sha: 'old-feat' }], tags: [] });
  d.applyGraph({ gen: 0, want: 2000, more: false, commits: list(...pad(2000, 'recent-'), 'old-feat'), error: null, stashes: null, worktrees: null });
  assert.deepEqual(reloads, [true]);
});

test('a NEW ref whose tip is inside the fresh page keeps the splice path', () => {
  const { d, reloads, seed } = data(2000);
  seed(list('recent', 'mid', 'base'));
  d.setRefs({ local: [{ name: 'main', sha: 'recent' }], remote: [], tags: [] });
  d.setRefs({ local: [{ name: 'main', sha: 'recent' }, { name: 'topic', sha: 'fresh' }], remote: [], tags: [] });
  d.applyGraph({ gen: 0, want: 2000, more: false, commits: list('fresh', 'recent', 'mid', 'base'), error: null, stashes: null, worktrees: null });
  assert.deepEqual(reloads, []);
  assert.deepEqual(d.commits.map((c: { sha: string }) => c.sha), ['fresh', 'recent', 'mid', 'base']);
  // The tip was consumed: a later unrelated splice doesn't re-read for it.
  d.applyGraph({ gen: 0, want: 2000, more: false, commits: list('fresh', 'recent', 'mid', 'base'), error: null, stashes: null, worktrees: null });
  assert.deepEqual(reloads, []);
});

test('two failed background refreshes in a row raise the stale bar; a success clears it', () => {
  const { d, seed } = data(2000);
  seed(list('a', 'b'));
  const miss = { gen: 0, want: 2000, more: null, commits: null, error: new Error('offline'), stashes: null, worktrees: null };
  d.applyGraph(miss);
  assert.equal(d.refreshError, null);
  d.applyGraph(miss);
  assert.ok(d.refreshError);
  seed(list('a', 'b'));
  assert.equal(d.refreshError, null);
  d.noteBackgroundRead(new Error('x'));
  assert.equal(d.refsStaleError, null);
  d.noteBackgroundRead(new Error('x'));
  assert.ok(d.refsStaleError);
  d.noteBackgroundRead(null);
  assert.equal(d.refsStaleError, null);
});

test('a re-sync whose refs prove the splice unsafe reads the history ONCE, only as much as was held', async () => {
  // A stale branch (tip far below the first page) deleted outside Otto. It
  // used to read a first page, then the old 10k "first page" on top of it.
  const urls: string[] = [];
  let pending: unknown = { local: [{ name: 'main', sha: 'c0' }], remote: [], tags: [] };
  const src = loadSource(git('graph/graphData.svelte.ts'), {
    svelte: { untrack: (fn: () => unknown) => fn() },
    '../../../lib/api/client': {
      api: {
        get: async (url: string) => {
          urls.push(url);
          const limit = Number(/limit=(\d+)/.exec(url)?.[1] ?? 0);
          return url.includes('/log?') ? list(...pad(limit, 'c')) : [];
        },
      },
    },
    '../../../lib/toast.svelte': { toasts: {} },
    '../../../lib/toastError': { toastError() {} },
    '../../../lib/stores/git.svelte': { git: { refsRev: {} } },
    '../../../lib/api/gitOverview': { readOverview: () => Promise.reject(new Error('no overview in this fixture')) },
    '../../../lib/diff/gitBridge.svelte': { gitBridge: { stashCount: {} } },
    '../graph-cache': { graphCache: { get: () => null, set() {} } },
    '../graph-splice': loadSource(git('graph-splice.ts'), {}),
    '../ref-menus': loadSource(git('ref-menus.ts'), {}),
    './graphSync.svelte': {
      createGraphSync: (h: { setRefs(r: unknown): void }) => ({
        adoptRefsRev() {},
        flushResync() {},
        pendingRefsFor: () => pending,
        flushPendingRefs() {
          if (pending) h.setRefs(pending);
          pending = null;
        },
      }),
    },
  });
  const d = src.createGraphData({ repoId: 'r1', onstatus() {}, graphVisible: () => true });
  d.applyGraph({ gen: 0, want: 1200, more: true, commits: list(...pad(1200, 'c')), error: null, stashes: null, worktrees: null });
  d.setRefs({ local: [{ name: 'main', sha: 'c0' }, { name: 'stale', sha: 'c900' }], remote: [], tags: [] });
  await d.reloadGraph();
  assert.deepEqual(urls.filter((u) => u.includes('/log?')), ['/repos/r1/log?all=true&limit=1200']);
  assert.equal(d.commits.length, 1200);
  assert.equal(d.hasMore, true);
  // The refs landed and the next ordinary move splices one page again.
  urls.length = 0;
  pending = { local: [{ name: 'main', sha: 'c0' }], remote: [], tags: [] };
  await d.reloadGraph();
  assert.deepEqual(urls.filter((u) => u.includes('/log?')), ['/repos/r1/log?all=true&limit=200']);
});
