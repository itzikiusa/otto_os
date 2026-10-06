import { test } from 'node:test';
import assert from 'node:assert/strict';
import { componentFunctions } from './componentFunctions.ts';
import { deferred, loadSource } from './sourceHarness.ts';
import { splitRemoteRef } from '../src/modules/git/refTracking.ts';
import { editLines, editSeed, isCrlfBlock } from '../src/modules/git/conflictEdit.ts';
import { alreadyOnPr } from '../src/modules/git/reviewPost.ts';
import { carryViewState, carryViewed } from '../src/modules/git/diff-viewstate.ts';
import { ROW_KEY_SEP, buildFileRows } from '../src/modules/git/diff-model.ts';
import { prNumberFromUrl } from '../src/modules/run-with-otto/runStatus.ts';

// Review S15 / S20 (iteration 2): destructive and outward git flows must act on
// exactly what the user saw and confirmed. Each test drives the real component
// function (componentFunctions) or the pure helper it delegates to.

const git = (name: string) => new URL(`../src/modules/git/${name}`, import.meta.url);
const plural = (n: number, w: string) => `${n} ${w}${n === 1 ? '' : 's'}`;

// ── S15-01 · remote branch delete targets the ref's OWN remote ──────────────

test('splitRemoteRef keeps the remote and the slash-bearing branch name', () => {
  assert.deepEqual(splitRemoteRef('upstream/feature/x'), { remote: 'upstream', branch: 'feature/x' });
  assert.deepEqual(splitRemoteRef('origin/main'), { remote: 'origin', branch: 'main' });
  assert.deepEqual(splitRemoteRef('main'), { remote: 'origin', branch: 'main' });
});

test('deleting upstream/feature-x names and pushes to upstream, never origin', async () => {
  const asks: string[] = [];
  const calls: { path: string; body: Record<string, unknown>; target: string }[] = [];
  const s = componentFunctions(git('GraphView.svelte'), ['deleteRemoteBranch'], {
    confirmer: { ask: async (text: string) => (asks.push(text), true) },
    mutate: async (path: string, body: Record<string, unknown>, _ok: string, target: string) => {
      calls.push({ path, body, target });
    },
  });
  const { remote, branch } = splitRemoteRef('upstream/feature-x');
  await s.deleteRemoteBranch(branch, remote);
  assert.match(asks[0], /upstream\/feature-x/);
  assert.doesNotMatch(asks[0], /origin/);
  assert.equal(calls[0].body.remote_name, 'upstream');
  assert.equal(calls[0].body.name, 'feature-x');
  assert.equal(calls[0].body.local, false);
  assert.equal(calls[0].target, 'upstream/feature-x');
});

// ── S15-02 · a partial discard never widens to the whole hunk ───────────────

function hunkFixture() {
  const posts: Record<string, unknown>[] = [];
  const diff0 = { files: [] };
  const file = { path: 'a.ts', fingerprint: 'fp' };
  const state: Record<string, any> = {
    wip: { onapplied() {} },
    repoId: 'r1',
    applying: false,
    diff: diff0,
    sel: { path: 'a.ts', hunk: 0, lines: new Set([3, 1]), anchor: 1 },
    selRaw: null,
    plural,
    toasts: { info() {}, error() {} },
    ApiError: class extends Error {},
    api: { post: async (_u: string, body: Record<string, unknown>) => (posts.push(body), { status: {}, diff: diff0 }) },
  };
  return { state, posts, file, diff0 };
}

test('discard sends the lines picked BEFORE the confirm and names the count', async () => {
  const f = hunkFixture();
  let asked = '';
  f.state.confirmer = { ask: async (t: string) => ((asked = t), true) };
  const s = componentFunctions(git('DiffViewer.svelte'), ['applyHunk'], f.state);
  await s.applyHunk(f.file, 0, { header: '@@' }, 'discard');
  assert.match(asked, /2 changed lines/);
  assert.deepEqual([...(f.posts[0].lines as number[])], [1, 3]);
});

test('a re-fetch while the discard confirm is open discards nothing', async () => {
  const f = hunkFixture();
  const s = componentFunctions(git('DiffViewer.svelte'), ['applyHunk'], {
    ...f.state,
    confirmer: {
      ask: async () => {
        // repo_status_changed → WipPanel re-fetched: new diff object, sel gone.
        s.diff = { files: [] };
        s.sel = null;
        return true;
      },
    },
  });
  await s.applyHunk(f.file, 0, { header: '@@' }, 'discard');
  assert.equal(f.posts.length, 0, 'nothing may be sent — least of all a whole-hunk discard');
});

test('a shift-click range picks changed lines only (S15-20)', () => {
  const lines = [{ origin: 'del' }, { origin: 'context' }, { origin: 'context' }, { origin: 'add' }];
  const s = componentFunctions(git('DiffViewer.svelte'), ['selectLine'], {
    wip: {},
    diff: {},
    sel: { path: 'a', hunk: 0, lines: new Set([0]), anchor: 0 },
    selRaw: null,
  });
  s.selectLine({ shiftKey: true }, 'a', 0, 3, lines[3], lines);
  assert.deepEqual([...s.selRaw.s.lines].sort(), [0, 3]);
});

// ── S15-03 / S15-14 · conflict hand-edit keeps its text and line endings ────

test('conflict edit round-trips CRLF and treats an empty editor as a deliberate empty block', () => {
  const ours = ['a\r', 'b\r'];
  assert.equal(isCrlfBlock(ours, ['c\r'], []), true);
  assert.equal(isCrlfBlock(['a', 'b'], ['c\r'], []), false);
  assert.equal(editSeed(ours), 'a\nb');
  assert.deepEqual(editLines('a\nfixed', true), ['a\r', 'fixed\r']);
  assert.deepEqual(editLines('a\nfixed', false), ['a', 'fixed']);
  assert.deepEqual(editLines('', true), []);
});

test('"Apply edit" makes the hand-edited text the resolution', () => {
  const s = componentFunctions(git('ConflictHunk.svelte'), ['stopEdit', 'cancelEdit', 'toggleLine'], {
    editLines,
    editText: 'A1\nA2 fixed',
    crlf: false,
    manual: null,
    touched: false,
    editing: true,
    oursSel: [true, false],
    theirsSel: [],
  });
  s.stopEdit();
  assert.deepEqual(s.manual, ['A1', 'A2 fixed']);
  assert.equal(s.editing, false);
  assert.equal(s.touched, true);
  // A later pick is a new decision and replaces the edit.
  s.toggleLine('ours', 1);
  assert.equal(s.manual, null);
});

test('taking an EMPTY side records the choice (the checkbox no longer desyncs)', () => {
  const s = componentFunctions(git('ConflictHunk.svelte'), ['toggleSide'], {
    ours: [],
    theirs: ['x'],
    oursSel: [],
    theirsSel: [false],
    oursAll: false,
    theirsAll: false,
    emptyTaken: { ours: false, theirs: false },
    touched: false,
    manual: null,
  });
  s.toggleSide('ours');
  assert.equal(s.emptyTaken.ours, true);
  assert.equal(s.touched, true);
});

// ── S15-04 / S20-06 · Create PR drafts the chosen Source and guards Cancel ──

function createPrFixture(reply: Record<string, unknown>) {
  const posts: { url: string; body: Record<string, unknown> }[] = [];
  const warns: string[] = [];
  const state: Record<string, any> = {
    repoId: 'r1',
    source: 'feature-x',
    target: 'main',
    title: '',
    description: '',
    drafting: false,
    draftSessionId: null,
    draftElapsed: 0,
    draftStartedAt: null,
    draftedAt: null,
    alive: true,
    setInterval: () => 0,
    clearInterval: () => {},
    Date,
    api: { post: async (url: string, body: Record<string, unknown>) => (posts.push({ url, body }), reply) },
    toasts: { warn: (t: string) => warns.push(t), info() {} },
    toastError() {},
    confirmer: { ask: async () => true },
  };
  return { state, posts, warns };
}

test('Draft sends the selected Source as head and never re-points Source', async () => {
  const f = createPrFixture({ title: 'T', description: 'D', source_branch: 'develop', target_branch: 'main' });
  const s = componentFunctions(git('CreatePr.svelte'), ['draftWithAgent'], f.state);
  await s.draftWithAgent();
  assert.deepEqual({ ...f.posts[0].body }, { base: 'main', head: 'feature-x' });
  assert.equal(s.source, 'feature-x', 'the PR head must stay what the user picked');
  assert.equal(f.warns.length, 1, 'a draft of another branch is called out');
});

test('a draft landing after the sheet closed touches nothing', async () => {
  const f = createPrFixture({ title: 'T', description: 'D', source_branch: 'feature-x' });
  let asked = 0;
  f.state.title = 'typed';
  f.state.confirmer = { ask: async () => (asked++, true) };
  const s = componentFunctions(git('CreatePr.svelte'), ['draftWithAgent'], f.state);
  const run = s.draftWithAgent();
  s.alive = false;
  await run;
  assert.equal(asked, 0, 'no ghost confirm may supersede another dialog');
  assert.equal(s.title, 'typed');
});

test('Cancel mid-push is inert; mid-draft it asks before discarding', async () => {
  let closed = 0;
  let asked = '';
  const s = componentFunctions(git('CreatePr.svelte'), ['requestClose'], {
    busy: true,
    drafting: false,
    title: '',
    description: '',
    reviewers: [],
    revQuery: '',
    onclose: () => closed++,
    confirmer: { ask: async (_t: string, o: { title: string }) => ((asked = o.title), false) },
  });
  await s.requestClose();
  assert.equal(closed, 0);
  s.busy = false;
  s.drafting = true;
  await s.requestClose();
  assert.equal(asked, 'Stop drafting and discard?');
  assert.equal(closed, 0);
});

test('confirming "Stop drafting and discard?" stops the drafting session (S20-305)', async () => {
  const posts: string[] = [];
  let closed = 0;
  const base = {
    busy: false,
    drafting: true,
    title: '',
    description: '',
    reviewers: [],
    revQuery: '',
    liveDraftId: 'sess-draft',
    onclose: () => closed++,
    api: { post: async (url: string) => (posts.push(url), {}) },
  };
  // Keep editing: nothing is stopped, nothing closes.
  const keep = componentFunctions(git('CreatePr.svelte'), ['requestClose', 'stopDraftSession'], {
    ...base,
    confirmer: { ask: async () => false },
  });
  await keep.requestClose();
  assert.deepEqual(posts, []);
  assert.equal(closed, 0);
  // Discard: the agent's PTY is killed (row kept) and the sheet closes.
  const s = componentFunctions(git('CreatePr.svelte'), ['requestClose', 'stopDraftSession'], {
    ...base,
    confirmer: { ask: async () => true },
  });
  await s.requestClose();
  await new Promise((r) => setTimeout(r, 0));
  assert.deepEqual(posts, ['/sessions/sess-draft/kill']);
  assert.equal(closed, 1);
  // A failed stop (already exited) still closes and never throws.
  const failing = componentFunctions(git('CreatePr.svelte'), ['requestClose', 'stopDraftSession'], {
    ...base,
    api: { post: async () => { throw new Error('gone'); } },
    confirmer: { ask: async () => true },
  });
  await failing.requestClose();
  assert.equal(closed, 2);
});

// ── S15-06 / S15-07 · review comments post exactly what was shown, once ─────

test('posting a comment with an open edit sends nothing', async () => {
  let calls = 0;
  const s = componentFunctions(git('ReviewPanel.svelte'), ['postComment'], {
    editingBody: { c1: 'softer text' },
    toasts: { warn() {} },
    api: { post: async () => (calls++, {}), get: async () => (calls++, {}) },
    confirmOutward: async () => true,
  });
  assert.equal(await s.postComment({ id: 'c1', state: 'draft', posted: false, body: 'harsh' }), false);
  assert.equal(calls, 0);
});

test('a retry is skipped when the earlier attempt already created the comment', () => {
  const c = { id: 'c', review_id: 'r', path: 'a.ts', line: 4, severity: 'warn', body: 'Use a guard  here', state: 'approved', posted: false, created_at: '' } as const;
  const pr = (path: string | null, line: number | null, body: string) => ({ id: 'p', author: 'me', body, path, line, created_at: '', replies: [], resolved: false });
  assert.equal(alreadyOnPr(c, [pr('a.ts', 4, 'Use a guard here')]), true);
  assert.equal(alreadyOnPr(c, [pr(null, null, '`a.ts:4` — Use a guard here')]), true, 'general-comment fallback');
  assert.equal(alreadyOnPr(c, [pr('a.ts', 9, 'Use a guard here')]), false, 'another anchor is another comment');
  assert.equal(alreadyOnPr(c, [{ ...pr('b.ts', 1, 'x'), replies: [pr(null, null, 'Use a guard here')] }]), true);
  assert.equal(alreadyOnPr(c, []), false);
});

test('a comment declined mid "Post all" is skipped', async () => {
  const posted: string[] = [];
  const review = {
    comments: [
      { id: 'a', state: 'draft', posted: false, body: 'a', path: null, line: null },
      { id: 'b', state: 'draft', posted: false, body: 'b', path: null, line: null },
    ],
  };
  const s = componentFunctions(git('ReviewPanel.svelte'), ['postAllDrafts', 'isPostable'], {
    editingAny: false,
    postingAll: false,
    review,
    prNumber: 7,
    prWhere: 'repo · PR #7',
    PR_WHO: '',
    plural,
    commentPreview: (c: { body: string }) => c.body,
    confirmOutward: async () => true,
    toasts: { success() {}, error() {}, warn() {} },
    postComment: async (c: { id: string }) => {
      posted.push(c.id);
      // The user declines `b` while `a` posts.
      review.comments = review.comments.map((x) => (x.id === 'b' ? { ...x, state: 'declined' } : x));
      return true;
    },
  });
  await s.postAllDrafts();
  assert.deepEqual(posted, ['a']);
});

// ── S15-08 · a closed WIP panel never applies a draft ────────────────────────

test('a commit draft landing after unmount opens no confirm and writes nothing', async () => {
  const reply = deferred<{ message: string; from_staged: boolean }>();
  let asked = 0;
  const s = componentFunctions(git('WipPanel.svelte'), ['followDraft', 'applyDraft'], {
    alive: true,
    repoId: 'r1',
    subject: 'typed',
    body: '',
    drafting: false,
    draftSessionId: null,
    draftElapsed: 0,
    draftStartedAt: null,
    draftedAt: null,
    Date,
    setInterval: () => 0,
    clearInterval: () => {},
    confirmer: { ask: async () => (asked++, true) },
    toasts: { info() {} },
    toastError() {},
  });
  const run = s.followDraft(reply.promise, 'r1');
  s.alive = false; // the user clicked a commit in the graph
  reply.resolve({ message: 'feat: x', from_staged: true });
  await run;
  assert.equal(asked, 0);
  assert.equal(s.subject, 'typed');
});

// ── S15-09 · local review: only timer polls spend the budget ─────────────────

test('bus polls keep refreshing after the timer budget is spent, toasting once', async () => {
  let gets = 0;
  let toasts = 0;
  const s = componentFunctions(git('LocalReviewPanel.svelte'), ['poll'], {
    alive: true,
    pollCount: 0,
    MAX_POLLS: 2,
    budgetWarned: false,
    repoId: 'r1',
    history: [],
    review: null,
    toasts: { warn: () => toasts++ },
    api: { get: async () => (gets++, { status: 'running', comments: [] }) },
    schedulePoll() {},
    initChecked() {},
  });
  await s.poll();
  await s.poll();
  await s.poll(); // over budget
  await s.poll();
  assert.equal(gets, 2);
  assert.equal(toasts, 1);
  await s.poll(true); // a review_changed event
  assert.equal(gets, 3, 'a bus poll always re-reads');
  assert.equal(s.pollCount, 0);
});

// ── S15-10 · merge is pinned to the head the checks describe ─────────────────

test('merge sends expected_head_sha and explains a moved head', async () => {
  const bodies: Record<string, unknown>[] = [];
  class ApiError extends Error {
    status: number;
    constructor(status: number, msg: string) {
      super(msg);
      this.status = status;
    }
  }
  const s = componentFunctions(git('PrMergeModal.svelte'), ['doMerge'], {
    canMerge: true,
    number: 5,
    repoId: 'r1',
    strategy: 'squash',
    deleteSource: false,
    pr: { head_sha: 'abc123' },
    merging: false,
    error: null,
    disposed: false,
    ApiError,
    toasts: { success() {}, error() {} },
    onmerged() {},
    api: {
      post: async (_u: string, b: Record<string, unknown>) => {
        bodies.push(b);
        throw new ApiError(409, 'PR head changed — re-check');
      },
    },
  });
  await s.doMerge();
  assert.equal(bodies[0].expected_head_sha, 'abc123');
  assert.match(s.error, /changed since these checks ran/);
});

// ── S15-12 / S15-13 / S15-21 / S15-22 · diff model + view state ─────────────

test('row keys never contain NUL (CSS.escape maps it to U+FFFD)', () => {
  const file = { path: 'a.ts', old_path: null, is_binary: false, hunks: [], hunks_omitted: false, too_large: false, status: 'modified' };
  const rows = buildFileRows({ file, eff: file, collapsed: true, first: true } as never);
  assert.ok(rows.length > 0);
  for (const r of rows) assert.ok(!r.key.includes('\u0000') && r.key.includes(ROW_KEY_SEP));
});

test('PR diff view state carries across a reload for paths that still exist', () => {
  const composer = { path: 'a.ts', oldLine: null, newLine: 3, line: 3, side: 'new' as const };
  const prev = {
    overrides: { 'a.ts': false, 'gone.ts': true },
    composer,
    uncapped: new Map([['a.ts', new Set([1])], ['gone.ts', new Set([0])]]),
    full: new Set(['a.ts', 'gone.ts']),
  };
  const paths = new Set(['a.ts', 'b.ts']);
  const next = carryViewState(prev, paths);
  assert.deepEqual(next.overrides, { 'a.ts': false });
  assert.equal(next.composer, composer);
  assert.deepEqual([...next.uncapped.keys()], ['a.ts']);
  assert.deepEqual([...next.full], ['a.ts']);
  assert.equal(carryViewState({ ...prev, composer: { ...composer, path: 'gone.ts' } }, paths).composer, null);
  assert.deepEqual([...carryViewed(new Set(['a.ts', 'gone.ts']), paths)], ['a.ts']);
});

test('a lazily fetched file never borrows another file’s hunks', async () => {
  const other = { path: 'other.ts', old_path: null, hunks: [{ header: '@@' }] };
  const mod = loadSource(git('diff-load.ts'), { '../../lib/api/client': { api: {} } });
  const me = { path: 'me.ts', old_path: null };
  assert.equal(mod.pick({ files: [other] }, me), null);
  const renamed = { path: 'new.ts', old_path: 'me.ts', hunks: [] };
  assert.equal(mod.pick({ files: [renamed] }, me), renamed);
  assert.equal(mod.pick({ files: [] }, me), null);
});

// ── S15-24 · bare-key diff navigation ────────────────────────────────────────

test('⌘] / Ctrl+N are left to the app; a bare ] moves to the next file', () => {
  const moves: number[] = [];
  const s = componentFunctions(git('DiffViewer.svelte'), ['onDiffKeydown'], { navToFile: (d: number) => moves.push(d) });
  const ev = (key: string, mods: Record<string, boolean> = {}) => ({ key, target: { tagName: 'DIV' }, preventDefault() {}, ...mods });
  s.onDiffKeydown(ev(']', { metaKey: true }));
  s.onDiffKeydown(ev('n', { ctrlKey: true }));
  assert.deepEqual(moves, []);
  s.onDiffKeydown(ev(']'));
  assert.deepEqual(moves, [1]);
});

// ── S15-05 / S20-09 · Run with Otto ──────────────────────────────────────────

test('a repo pick from another workspace is dropped when the repo list loads', async () => {
  const s = componentFunctions(new URL('../src/modules/run-with-otto/RunLauncher.svelte', import.meta.url), ['loadRepos'], {
    wsId: 'wsB',
    repoId: 'repo-of-A',
    repos: [],
    reposError: '',
    reposLoading: false,
    loadErrorText: String,
    api: { get: async () => [{ id: 'repo-of-B' }] },
  });
  await s.loadRepos('wsB');
  assert.equal(s.repoId, '');
  assert.deepEqual(s.repos.map((r: { id: string }) => r.id), ['repo-of-B']);
});

test('PR numbers parse from every forge URL shape', () => {
  assert.equal(prNumberFromUrl('https://github.com/o/r/pull/42'), 42);
  assert.equal(prNumberFromUrl('https://gitlab.com/o/r/-/merge_requests/7'), 7);
  assert.equal(prNumberFromUrl('https://bitbucket.org/o/r/pull-requests/13/overview'), 13);
  assert.equal(prNumberFromUrl('https://example.com/o/r'), null);
  assert.equal(prNumberFromUrl(undefined), null);
});
