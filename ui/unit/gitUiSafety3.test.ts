import { test } from 'node:test';
import assert from 'node:assert/strict';
import { componentFunctions } from './componentFunctions.ts';
import { fileIsCrlf, markAction } from '../src/modules/git/conflictEdit.ts';
import { carryViewState, carryViewed, fileStamp, unchangedPaths } from '../src/modules/git/diff-viewstate.ts';
import { amendPublishedAt } from '../src/modules/git/refTracking.ts';
import { pinnedHead } from '../src/modules/git/pr-cache.ts';

// Review S15 (iteration 3): conflict data loss, remote targeting, the
// duplicate-post guard, carried diff state and the merge pin. Each test drives
// the real component function (componentFunctions) or the pure helper it uses.

const git = (name: string) => new URL(`../src/modules/git/${name}`, import.meta.url);

// ── S15-301 · a binary / absent conflict is never "marked resolved" ─────────

const conflictFile = (over: Record<string, unknown> = {}) => ({
  path: 'logo.png',
  is_binary: false,
  worktree_present: true,
  ours_present: true,
  theirs_present: true,
  trailing_newline: true,
  segments: [],
  ...over,
});

test('markAction refuses binary and absent files, keeps marker-free text as-is', () => {
  assert.equal(markAction(null, 0, 0), null);
  assert.equal(markAction(conflictFile({ is_binary: true }), 0, 0), null, 'binary: segments are empty');
  assert.equal(markAction(conflictFile({ worktree_present: false }), 0, 0), null, 'deleted-by-us');
  assert.equal(markAction(conflictFile(), 0, 0), 'keep', 'marker-free text stages its bytes');
  assert.equal(markAction(conflictFile(), 2, 1), null, 'undecided conflict');
  assert.equal(markAction(conflictFile(), 2, 2), 'compose');
  assert.equal(markAction(conflictFile({ is_binary: true }), 2, 2), null);
});

function paneFixture(file: Record<string, unknown>, mark: 'compose' | 'keep' | null) {
  const posts: { url: string; body: unknown }[] = [];
  const writes: string[] = [];
  const s = componentFunctions(git('ConflictFilePane.svelte'), ['markResolved', 'takeSide', 'composeContent'], {
    file,
    mark,
    canMark: mark !== null,
    saving: false,
    repoId: 'r1',
    path: String(file.path),
    choices: [],
    confirmer: { ask: async () => true },
    toasts: { success() {} },
    toastError() {},
    onresolved() {},
    api: { post: async (url: string, body: unknown) => void posts.push({ url, body }) },
    git: { resolveConflict: async (_r: string, _p: string, content: string) => void writes.push(content) },
  });
  return { s, posts, writes };
}

test('"Mark file resolved" never writes an empty body over a binary or absent file', async () => {
  for (const f of [conflictFile({ is_binary: true }), conflictFile({ path: 'gone.txt', worktree_present: false })]) {
    const { s, posts, writes } = paneFixture(f, markAction(f, 0, 0));
    await s.markResolved();
    assert.deepEqual(writes, [], `${f.path}: no content write`);
    assert.deepEqual(posts, [], `${f.path}: no side taken implicitly`);
  }
});

test('a marker-free text file is marked by staging its bytes (side keep), not recomposed', async () => {
  const f = conflictFile({ path: 'fixed.txt', segments: [{ kind: 'context', lines: ['a\r', 'b\r'] }] });
  const { s, posts, writes } = paneFixture(f, markAction(f, 0, 0));
  await s.markResolved();
  assert.deepEqual(writes, []);
  // JSON: the handler's object literals come from the vm realm.
  assert.deepEqual(JSON.parse(JSON.stringify(posts)), [{ url: '/repos/r1/conflict/resolve', body: { path: 'fixed.txt', side: 'keep' } }]);
});

// ── S15-308 · an empty-sided hunk in a CRLF file stays CRLF ─────────────────

test('fileIsCrlf reads the whole file, so an empty-sided hunk inherits CRLF', () => {
  const segs = [
    { kind: 'context' as const, lines: ['one\r', 'two\r', 'three\r'] },
    { kind: 'conflict' as const, ours: [], theirs: ['last'], base: [] },
  ];
  assert.equal(fileIsCrlf(segs), true);
  assert.equal(fileIsCrlf([{ kind: 'context', lines: ['a', 'b'] }]), false);
  assert.equal(fileIsCrlf([]), false);
});

// ── S15-303 · "local + remote" deletes on the row's own remote ──────────────

test('local + remote delete sends remote_name on the safe and the forced request', async () => {
  const asks: string[] = [];
  const posts: Record<string, unknown>[] = [];
  const forced: Record<string, unknown>[] = [];
  class NotMerged extends Error {}
  const s = componentFunctions(git('GraphView.svelte'), ['deleteLocalBranch'], {
    repoId: 'r1',
    confirmer: { ask: async (text: string) => (asks.push(text), true) },
    api: {
      post: async (_u: string, body: Record<string, unknown>) => {
        posts.push(body);
        throw new NotMerged('error: the branch is not fully merged');
      },
    },
    mutate: async (_p: string, body: Record<string, unknown>) => void forced.push(body),
    refreshAfter: async () => {},
    toasts: { success() {} },
    toastError() {},
  });
  await s.deleteLocalBranch('feature-x', true, 'upstream');
  assert.match(asks[0], /upstream\/feature-x/);
  assert.doesNotMatch(asks[0], /origin/);
  assert.equal(posts[0].remote_name, 'upstream');
  assert.equal(forced[0].remote_name, 'upstream');
  assert.equal(forced[0].force, true);

  // A local-only delete sends no remote_name at all.
  posts.length = 0;
  await s.deleteLocalBranch('feature-x', false).catch(() => {});
  assert.equal('remote_name' in posts[0], false);
});

// ── S15-302 · a copy found on the PR is recorded posted ────────────────────

test('the retry guard marks a found copy posted so it stops being postable', async () => {
  const patches: { url: string; body: unknown }[] = [];
  const c = { id: 'c1', state: 'approved', posted: false, body: 'Use a guard', path: null, line: null };
  const state: Record<string, any> = {
    repoId: 'r1',
    prNumber: 7,
    review: { comments: [c] },
    history: [],
    toasts: { info() {} },
    toastError() {},
    alreadyOnPr: () => true,
    patchCommentInReview: (r: { comments: (typeof c)[] }, u: typeof c) => ({
      comments: r.comments.map((x) => (x.id === u.id ? u : x)),
    }),
    api: {
      get: async () => ({ comments: [{ body: 'Use a guard' }] }),
      patch: async (url: string, body: unknown) => (patches.push({ url, body }), { ...c, posted: true }),
    },
  };
  const s = componentFunctions(git('ReviewPanel.svelte'), ['lookupOnPr'], state);
  assert.equal(await s.lookupOnPr(c, true), true);
  assert.deepEqual(JSON.parse(JSON.stringify(patches)), [{ url: '/pr-review-comments/c1', body: { mark_posted: true } }]);
  assert.equal(s.review.comments[0].posted, true);
});

// ── S15-304 · carried diff state only for unchanged files ───────────────────

const fd = (path: string, fingerprint?: string, header = '@@ -1 +1 @@') => ({
  path,
  old_path: null,
  is_binary: false,
  hunks: [{ header, lines: [] }],
  added: 1,
  deleted: 1,
  status: 'modified' as const,
  ...(fingerprint ? { fingerprint } : {}),
});

test('a push that changed auth.rs drops its viewed mark, composer and expansions', () => {
  const prev = { files: [fd('auth.rs', 'aaa'), fd('ok.rs', 'bbb'), fd('sum.rs')] };
  const next = { files: [fd('auth.rs', 'ccc'), fd('ok.rs', 'bbb'), fd('sum.rs', undefined, '@@ -1 +1,2 @@')] };
  const same = unchangedPaths(prev, next);
  assert.deepEqual([...same], ['ok.rs']);
  assert.deepEqual([...carryViewed(new Set(['auth.rs', 'ok.rs', 'sum.rs']), same)], ['ok.rs']);
  const composer = { path: 'auth.rs', oldLine: null, newLine: 120, line: 120, side: 'new' as const };
  const carried = carryViewState(
    { overrides: {}, composer, uncapped: new Map([['auth.rs', new Set([3])], ['ok.rs', new Set([1])]]), full: new Set() },
    same,
  );
  assert.equal(carried.composer, null, 'line 120 holds different code now');
  assert.deepEqual([...carried.uncapped.keys()], ['ok.rs']);
  assert.equal(unchangedPaths(null, next).size, 0);
  assert.equal(fileStamp(fd('x', 'f1')), fileStamp(fd('x', 'f1', '@@ other @@')), 'fingerprint wins');
});

// ── S15-305 · the merge pin is the head the modal read itself ───────────────

test('pinnedHead prefers the modal’s live read over the parent’s copy', () => {
  assert.equal(pinnedHead('live', 'stale'), 'live');
  assert.equal(pinnedHead(null, 'stale'), 'stale');
  assert.equal(pinnedHead(null, null), null);
});

// ── S15-28 · amend warning from the daemon's --contains answer ──────────────

test('amendPublishedAt names where HEAD is published, upstream or not', () => {
  // No answer yet: the upstream heuristic.
  assert.equal(amendPublishedAt('origin/main', 0, null), 'origin/main');
  assert.equal(amendPublishedAt('origin/main', 2, null), null);
  assert.equal(amendPublishedAt(null, 0, null), null);
  // Pushed under another name, branch has no upstream.
  assert.equal(amendPublishedAt(null, 0, ['upstream/other']), 'upstream/other');
  // The daemon says nothing holds HEAD (ahead==0 vs a stale upstream).
  assert.equal(amendPublishedAt('origin/main', 0, []), null);
  assert.equal(amendPublishedAt('origin/main', 0, ['fork/x', 'origin/main']), 'origin/main (+1 more)');
});
