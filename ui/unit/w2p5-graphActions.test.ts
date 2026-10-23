import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

// architecture-1-09: the git graph's actions live in graph/graphActions.svelte.ts
// behind a host (GraphView's state as getters, its refresh helpers). These run
// the REAL handlers against a fake git client / API and a recording host.

class DirtyRefusal extends Error {}

function setup(opts: { checkout?: (body: Record<string, unknown>) => Promise<unknown>; post?: (url: string, body: any) => Promise<unknown>; confirm?: boolean[]; outward?: boolean } = {}) {
  const log: string[] = [];
  const toasts: { kind: string; title: string; body?: string }[] = [];
  const asks: string[] = [];
  const answers = [...(opts.confirm ?? [])];
  const checkouts: Record<string, unknown>[] = [];
  const posts: { url: string; body: any }[] = [];
  const mutations: { path: string; body: any; title: string; detail?: string }[] = [];
  const imports: Record<string, unknown> = {
    '../../../lib/api/client': {
      api: { post: (url: string, body: any) => { posts.push({ url, body }); return (opts.post ?? (async () => []))(url, body); } },
      isDirtyGitRefusal: (e: unknown) => e instanceof DirtyRefusal,
    },
    '../../../lib/toast.svelte': {
      toasts: {
        success: (title: string, body?: string) => toasts.push({ kind: 'success', title, body }),
        warn: (title: string, body?: string) => toasts.push({ kind: 'warn', title, body }),
        error: (title: string, body?: string) => toasts.push({ kind: 'error', title, body }),
      },
    },
    '../../../lib/confirm.svelte': { confirmer: { ask: async (text: string) => { asks.push(text); return answers.shift() ?? true; } } },
    '../../../lib/confirmOutward': { confirmOutward: async (o: { verb: string }) => { asks.push(o.verb); return opts.outward ?? true; } },
    '../../../lib/toastError': { toastError: (title: string, e: unknown) => toasts.push({ kind: 'error', title, body: String((e as Error)?.message ?? e) }) },
    '../../../lib/plural': { plural: (n: number, w: string) => `${n} ${w}${n === 1 ? '' : 's'}` },
    '../../../lib/clipboard': { copyTextOrThrow: async () => {} },
    '../../../lib/stores/git.svelte': {
      git: {
        allRepos: [],
        checkout: (_id: string, body: Record<string, unknown>) => { checkouts.push(body); return (opts.checkout ?? (async () => ({ changes: [] })))(body); },
      },
    },
    '../../../lib/stores/workspace.svelte': { ws: { currentId: 'w1' } },
    '../pullFlow': { runPull: async () => { log.push('pull'); } },
  };
  const mod = loadSource(new URL('../src/modules/git/graph/graphActions.svelte.ts', import.meta.url), imports);
  const busy: string[] = [];
  const host = {
    repoId: 'r1',
    workspaceId: 'w1',
    onstatus: () => log.push('onstatus'),
    _busy: '',
    get checkoutBusy() { return this._busy; },
    set checkoutBusy(v: string) { this._busy = v; busy.push(v); },
    openWtBusy: '',
    worktrees: [{ path: '/repo', is_main: true }] as any[],
    submodules: [] as any[],
    currentRepoPath: '/repo',
    refKnowledge: { remoteNames: new Set<string>(), localNames: new Set<string>(['main', 'feature-x']), currentBranch: 'main' },
    normPath: (p: string) => p.replace(/\/+$/, ''),
    adoptRefsRev: () => log.push('adoptRefsRev'),
    refreshAfter: async (s?: unknown) => { log.push(s ? 'refreshAfter(status)' : 'refreshAfter()'); },
    refreshAfterCheckout: async (_s: unknown, stashed: boolean) => { log.push(`refreshAfterCheckout(stashed=${stashed})`); },
    mutate: async (path: string, body: any, title: string, detail?: string) => { mutations.push({ path, body, title, detail }); },
    holdDiff: () => log.push('holdDiff'),
    stashShortMsg: (s: { message: string }) => s.message,
  };
  const a = mod.createGraphActions(host);
  return { a, host, log, toasts, asks, checkouts, posts, mutations, busy };
}

test('checkout success: switches, adopts the refs bump, refreshes refs and clears the busy flag', async () => {
  const t = setup();
  await t.a.checkout('feature-x', false);
  assert.deepEqual(t.checkouts.map((b) => ({ ...b })), [{ branch: 'feature-x', create: false }]);
  assert.deepEqual(t.log, ['adoptRefsRev', 'refreshAfterCheckout(stashed=false)']);
  assert.deepEqual(t.toasts.map((x) => x.title), ['Switched to feature-x']);
  assert.deepEqual(t.busy, ['feature-x', ''], 'busy while switching, cleared after');
});

test('a dirty tree asks, then stash → switch → restore, and the refresh knows it stashed', async () => {
  let n = 0;
  const t = setup({ checkout: async () => { if (n++ === 0) throw new DirtyRefusal('dirty'); return { changes: [] }; } });
  await t.a.checkout('feature-x', false);
  assert.match(t.asks[0], /stash them, switch, and restore/);
  assert.equal(t.checkouts[1].auto_stash, true);
  assert.deepEqual(t.log, ['adoptRefsRev', 'refreshAfterCheckout(stashed=true)']);
  assert.match(t.toasts[0].body ?? '', /recovery backup remains in Stashes/);
  assert.equal(t.host.checkoutBusy, '');
});

test('a failed checkout shows the error toast, refreshes nothing and clears the busy flag', async () => {
  const t = setup({ checkout: async () => { throw new Error('fatal: invalid reference'); } });
  await t.a.checkout('nope', false);
  assert.deepEqual(t.toasts.map((x) => [x.kind, x.title]), [['error', 'Couldn’t check out the branch']]);
  assert.deepEqual(t.log, [], 'no refresh, no refs adoption');
  assert.equal(t.host.checkoutBusy, '');
});

test('a failed worktree prune shows the error and keeps the listed worktrees', async () => {
  const t = setup({ post: async () => { throw new Error('remote: denied'); } });
  const before = t.host.worktrees;
  await t.a.wtPrune();
  assert.equal(t.posts[0].url, '/repos/r1/worktrees/prune');
  assert.equal(t.host.worktrees, before, 'state untouched');
  assert.deepEqual(t.toasts.map((x) => [x.kind, x.title]), [['error', 'Couldn’t prune the worktrees']]);
});

test('pushing a tag confirms where it goes first; declining sends nothing', async () => {
  const declined = setup({ outward: false });
  await declined.a.pushTag('v1.2.0');
  assert.deepEqual(declined.asks, ['Push tag']);
  assert.deepEqual(declined.mutations, []);
  const t = setup();
  await t.a.pushTag('v1.2.0');
  assert.deepEqual(t.mutations.map((m) => [m.path, { ...m.body }, m.title]), [['/tag/push', { name: 'v1.2.0' }, 'Tag pushed']]);
});

test('deleting upstream/feature-x names and pushes to upstream, never origin', async () => {
  const t = setup();
  await t.a.deleteRemoteBranch('feature-x', 'upstream');
  assert.match(t.asks[0], /upstream\/feature-x/);
  assert.doesNotMatch(t.asks[0], /origin/);
  assert.deepEqual({ ...t.mutations[0].body }, { name: 'feature-x', remote: true, remote_name: 'upstream', local: false, force: true });
  assert.equal(t.mutations[0].detail, 'upstream/feature-x');
});

test('local + remote delete: safe first, then an explicit force on "not fully merged"', async () => {
  const t = setup({ post: async () => { throw new Error('error: the branch is not fully merged'); }, confirm: [true, true] });
  await t.a.deleteLocalBranch('feature-x', true, 'upstream');
  assert.equal(t.posts[0].body.remote_name, 'upstream');
  assert.equal('force' in t.posts[0].body, false, 'the first request is the safe delete');
  assert.match(t.asks[1], /aren’t merged anywhere else/);
  assert.deepEqual({ ...t.mutations[0].body }, { name: 'feature-x', remote: true, remote_name: 'upstream', force: true });
  // Declining the force keeps the branch: no second request.
  const kept = setup({ post: async () => { throw new Error('not fully merged'); }, confirm: [true, false] });
  await kept.a.deleteLocalBranch('feature-x', false);
  assert.equal('remote_name' in kept.posts[0].body, false);
  assert.deepEqual(kept.mutations, []);
});

test('a remote-tracking checkout of a non-origin remote creates the branch AT that ref', async () => {
  const t = setup({ post: async () => ({ changes: [] }) });
  await t.a.checkoutRemoteRef('upstream/topic');
  assert.deepEqual([t.posts[0].url, { ...t.posts[0].body }], ['/repos/r1/branch', { name: 'topic', start_point: 'upstream/topic', checkout: true }]);
  assert.deepEqual(t.log, ['refreshAfterCheckout(stashed=false)']);
  // A local twin exists: switch to it, never create.
  const twin = setup();
  await twin.a.checkoutRemoteRef('origin/feature-x');
  assert.deepEqual(twin.checkouts.map((b) => ({ ...b })), [{ branch: 'feature-x', create: false }]);
});
