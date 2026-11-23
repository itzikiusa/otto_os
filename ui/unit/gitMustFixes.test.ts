// Wave 1 (MUST) regression tests for ui/src/modules/git/.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { register } from 'node:module';
import { componentFunctions } from './componentFunctions.ts';
import { deferred, loadSource } from './sourceHarness.ts';
import { agentRowKey, agentRowRetryAction, canStopReviewAgent } from '../src/modules/git/reviewAgentsRow.ts';
import { pickDefaultBase } from '../src/modules/git/localReviewRefs.ts';
import { prCandidateRepos } from '../src/modules/git/focusPrHelpers.ts';
import { lsGet, lsRemove, lsSet } from '../src/lib/storage.ts';
import * as gitopsPlan from '../src/modules/git/gitopsPlan.ts';

const hook =
  'export async function resolve(s, c, n) {' +
  "  if (/^\\.{1,2}\\//.test(s) && !/\\.[a-z]+$/i.test(s)) { try { return await n(s + '.ts', c); } catch {} }" +
  '  return n(s, c);' +
  '}';
register('data:text/javascript,' + encodeURIComponent(hook));

const git = (file: string) => new URL(`../src/modules/git/${file}`, import.meta.url);

// ── 1. ReviewPanel fallback poll + MAX_POLLS + PR-switch state reset ─────────

test('ReviewPanel.poll loads findings & readiness when review finishes via fallback poll', async () => {
  const loadedReadiness: string[] = [];
  const loadedDiff: Array<[string, number]> = [];
  let scheduled = 0;
  const doneReview = {
    id: 'rev-1',
    status: 'done',
    agents: [{ name: 'Correctness', status: 'done' }, { name: 'Summarizer', status: 'done' }],
    comments: [{ id: 'c1', path: 'a.ts', line: 10, body: 'bug', state: 'draft', posted: false }],
  };
  const s = componentFunctions(git('ReviewPanel.svelte'), ['poll'], {
    repoId: 'r1',
    prNumber: 42,
    reviewGeneration: 1,
    disposed: false,
    pollCount: 0,
    pollStale: false,
    MAX_POLLS: 5,
    api: { get: async () => doneReview },
    adoptReview: () => true,
    schedulePoll: () => { scheduled++; },
    loadDiff: (rid: string, num: number) => { loadedDiff.push([rid, num]); },
    loadFindingsAndReadiness: (id: string) => { loadedReadiness.push(id); },
  });
  await s.poll();
  assert.deepEqual(loadedReadiness, ['rev-1']);
  assert.deepEqual(loadedDiff, [['r1', 42]]);
  assert.equal(scheduled, 0);
  assert.equal(s.pollCount, 0);
  assert.equal(s.pollStale, false);

  // Even with 0 comments, loadFindingsAndReadiness still runs (loadDiff skipped).
  loadedReadiness.length = 0;
  loadedDiff.length = 0;
  s.api.get = async () => ({ ...doneReview, id: 'rev-2', comments: [] });
  await s.poll();
  assert.deepEqual(loadedReadiness, ['rev-2']);
  assert.deepEqual(loadedDiff, []);
});

test('ReviewPanel.poll enforces MAX_POLLS and resumePolling resets stale state', async () => {
  let gets = 0;
  let scheduled = 0;
  const runningReview = {
    id: 'rev-run',
    status: 'running',
    agents: [{ name: 'Correctness', status: 'running' }],
    comments: [],
  };
  const s = componentFunctions(git('ReviewPanel.svelte'), ['poll', 'resumePolling'], {
    repoId: 'r1',
    prNumber: 7,
    reviewGeneration: 1,
    disposed: false,
    pollCount: 0,
    pollStale: false,
    MAX_POLLS: 2,
    api: { get: async () => (gets++, runningReview) },
    adoptReview: () => true,
    schedulePoll: () => { scheduled++; },
    loadDiff: () => {},
    loadFindingsAndReadiness: () => {},
  });
  await s.poll(); // pollCount=1 < 2 -> schedules
  assert.equal(scheduled, 1);
  assert.equal(s.pollStale, false);
  await s.poll(); // pollCount=2 >= 2 -> stops polling, marks stale
  assert.equal(scheduled, 1);
  assert.equal(s.pollStale, true);
  await s.poll(); // already at MAX_POLLS -> no further request
  assert.equal(gets, 2);

  s.resumePolling();
  await Promise.resolve();
  assert.equal(gets, 3);
  assert.equal(s.pollStale, false);
});

test('ReviewPanel.load resets per-PR transient state and resolves pending postConfirm', async () => {
  let resolvedWith: unknown = 'unset';
  const s = componentFunctions(git('ReviewPanel.svelte'), ['load', 'cancelPendingPostConfirm'], {
    reviewGeneration: 0,
    disposed: false,
    loading: false,
    loadError: 'old error',
    pollCount: 15,
    pollStale: true,
    editingBody: { c1: 'unsaved' },
    diffExpanded: { c1: true },
    historyExpanded: { 0: true },
    actionBusy: { c1: 'edit' },
    postConfirm: { plan: {}, done: (v: unknown) => { resolvedWith = v; } },
    review: null,
    history: [],
    lastReviewJson: '',
    lastAdopted: null,
    mergeReadiness: { ok: true },
    readinessError: 'err',
    triage: { ok: true },
    resetSnippets: () => {},
    schedulePoll: () => {},
    loadDiff: () => {},
    loadFindingsAndReadiness: () => {},
    api: { get: async () => [] },
    ApiError: class extends Error {},
    loadErrorText: (e: Error) => e.message,
  });
  await s.load('r2', 99);
  assert.deepEqual({ ...s.editingBody }, {});
  assert.deepEqual({ ...s.diffExpanded }, {});
  assert.deepEqual({ ...s.historyExpanded }, {});
  assert.deepEqual({ ...s.actionBusy }, {});
  assert.equal(s.pollCount, 0);
  assert.equal(s.pollStale, false);
  assert.equal(s.postConfirm, null);
  assert.equal(resolvedWith, null);
});

// ── 2. ReviewAgents row keying & retry-button guard ──────────────────────────

test('ReviewAgents row keys disambiguate duplicate agent names', () => {
  assert.notEqual(agentRowKey(0, 'New agent'), agentRowKey(1, 'New agent'));
  assert.equal(agentRowKey(0, 'New agent'), '0:New agent');
  assert.equal(agentRowKey(1, 'New agent'), '1:New agent');
});

test('ReviewAgents never shows Retry summary on non-summarizer rows', () => {
  // 3 rows: indices 0, 1 are reviewers; index 2 is the summarizer.
  for (const status of ['pending', 'running', 'waiting']) {
    assert.equal(agentRowRetryAction(0, 3, status, 'done'), null, `reviewer ${status} in done view`);
    assert.equal(agentRowRetryAction(1, 3, status, 'running'), null, `reviewer ${status} in running view`);
  }
  for (const status of ['done', 'error', 'skipped']) {
    assert.equal(agentRowRetryAction(0, 3, status, 'done'), 'retry-agent');
    assert.equal(agentRowRetryAction(1, 3, status, 'running'), 'retry-agent');
  }
  // Trailing summarizer row:
  assert.equal(agentRowRetryAction(2, 3, 'pending', 'running'), null);
  assert.equal(agentRowRetryAction(2, 3, 'running', 'running'), null);
  assert.equal(agentRowRetryAction(2, 3, 'pending', 'done'), 'retry-summary');
  assert.equal(agentRowRetryAction(2, 3, 'done', 'done'), 'retry-summary');
  assert.equal(agentRowRetryAction(2, 3, 'error', 'running'), 'retry-summary');
  // Stop button: only non-summarizer running/waiting rows.
  assert.equal(canStopReviewAgent(0, 3, 'running'), true);
  assert.equal(canStopReviewAgent(0, 3, 'waiting'), true);
  assert.equal(canStopReviewAgent(0, 3, 'done'), false);
  assert.equal(canStopReviewAgent(2, 3, 'running'), false);
});

// ── 3. Guarded storage helpers survive throwing localStorage ─────────────────

test('lsGet/lsSet/lsRemove never throw when localStorage is blocked', () => {
  const prev = (globalThis as Record<string, unknown>).localStorage;
  const throwing = {
    getItem() { throw new Error('SecurityError'); },
    setItem() { throw new Error('QuotaExceededError'); },
    removeItem() { throw new Error('SecurityError'); },
  };
  Object.defineProperty(globalThis, 'localStorage', { value: throwing, configurable: true, writable: true });
  try {
    assert.equal(lsGet('otto_git_clone_dir'), null);
    assert.doesNotThrow(() => lsSet('otto_git_clone_dir', '/tmp/repos'));
    assert.doesNotThrow(() => lsRemove('otto_pr_draft:r1'));
  } finally {
    if (prev === undefined) delete (globalThis as Record<string, unknown>).localStorage;
    else Object.defineProperty(globalThis, 'localStorage', { value: prev, configurable: true, writable: true });
  }
});

// ── 5. graphActions outward confirmations ────────────────────────────────────

test('graphActions uses confirmOutward for tag push, local+remote branch delete, and remote branch delete', async () => {
  const outwardCalls: Array<Record<string, unknown>> = [];
  const localAsks: string[] = [];
  const mutations: Array<{ path: string; body: Record<string, unknown> }> = [];
  let outwardAnswer = true;
  const promptAnswers = ['v2.0.0'];

  const mod = loadSource(git('graph/graphActions.svelte.ts'), {
    '../../../lib/api/client': {
      api: { post: async () => ({ changes: [] }), get: async () => ({}) },
      ApiError: class extends Error {},
      isDirtyGitRefusal: () => false,
    },
    '../gitopsPlan': gitopsPlan,
    '../../../lib/toast.svelte': { toasts: { success() {}, warn() {}, error() {}, info() {} } },
    '../../../lib/confirm.svelte': {
      confirmer: {
        ask: async (msg: string) => { localAsks.push(msg); return true; },
        promptText: async () => promptAnswers.shift() ?? null,
      },
    },
    '../../../lib/confirmOutward': {
      confirmOutward: async (o: Record<string, unknown>) => {
        outwardCalls.push(o);
        return outwardAnswer;
      },
    },
    '../../../lib/toastError': { toastError() {} },
    '../../../lib/plural': { plural: (n: number, w: string) => `${n} ${w}` },
    '../../../lib/clipboard': { copyTextOrThrow: async () => {} },
    '../../../lib/stores/git.svelte': { git: { allRepos: [], checkout: async () => ({ changes: [] }) } },
    '../../../lib/stores/workspace.svelte': { ws: { currentId: 'w1' } },
    '../pullFlow': { runPull: async () => {} },
  });

  const a = mod.createGraphActions({
    repoId: 'r1',
    workspaceId: 'w1',
    onstatus() {},
    checkoutBusy: '',
    openWtBusy: '',
    worktrees: [],
    submodules: [],
    currentRepoPath: '/repo',
    refKnowledge: { remoteNames: new Set(), localNames: new Set(), currentBranch: 'main' },
    normPath: (p: string) => p,
    adoptRefsRev() {},
    refreshAfter: async () => {},
    refreshAfterCheckout: async () => {},
    mutate: async (path: string, body: Record<string, unknown>) => { mutations.push({ path, body }); return true; },
    holdDiff() {},
    stashShortMsg: () => '',
  });

  // createTagAt: declining outward push still creates the tag locally with push:false.
  outwardAnswer = false;
  await a.createTagAt('abcdef123456', false);
  assert.equal(outwardCalls.length, 1);
  assert.equal(outwardCalls[0].verb, 'Push tag');
  assert.match(String(outwardCalls[0].where), /origin.*v2\.0\.0/);
  assert.ok(outwardCalls[0].who);
  assert.equal(mutations[0].path, '/tag');
  assert.equal(mutations[0].body.push, false);

  // deleteLocalBranch(alsoRemote=true) uses confirmOutward with danger:true.
  outwardAnswer = true;
  await a.deleteLocalBranch('feat', true, 'upstream');
  assert.equal(outwardCalls.length, 2);
  assert.equal(outwardCalls[1].danger, true);
  assert.match(String(outwardCalls[1].where), /upstream.*feat/);
  assert.ok(outwardCalls[1].who);
  assert.equal(localAsks.length, 0);

  // deleteLocalBranch(alsoRemote=false) stays local confirmer.ask.
  await a.deleteLocalBranch('feat-local', false);
  assert.equal(outwardCalls.length, 2);
  assert.equal(localAsks.length, 1);

  // deleteRemoteBranch uses confirmOutward with danger:true.
  await a.deleteRemoteBranch('old-branch', 'upstream');
  assert.equal(outwardCalls.length, 3);
  assert.equal(outwardCalls[2].danger, true);
  assert.match(String(outwardCalls[2].where), /upstream.*old-branch/);
  assert.ok(outwardCalls[2].who);
});

// ── 6. Stale async fetch guards across git views ─────────────────────────────

test('LocalReviewPanel drops stale loadRefs, loadExisting, poll and startReview results after repo switch', async () => {
  const refsDef = deferred<{ refs: { remote: { name: string }[]; local: { name: string }[] }; error: null }>();
  const existingDef = deferred<unknown[]>();
  const pollDef = deferred<{ id: string; status: string; comments: unknown[] }>();
  const startDef = deferred<{ id: string; status: string; comments: unknown[] }>();

  const s = componentFunctions(git('LocalReviewPanel.svelte'), ['loadRefs', 'loadExisting', 'poll', 'startReview'], {
    repoId: 'r1',
    alive: true,
    disposed: false,
    refsGen: 0,
    reviewGen: 0,
    refsLoading: false,
    refsError: null,
    refs: null,
    selectedBase: 'main',
    allBranches: [{ name: 'main', value: 'main' }],
    pickDefaultBase,
    reviewLoading: false,
    historyError: null,
    history: [],
    review: null,
    starting: false,
    pollTimer: null,
    pollCount: 0,
    MAX_POLLS: 10,
    budgetWarned: false,
    checked: {},
    git: { refs: () => refsDef.promise },
    api: {
      get: (url: string) => (url.endsWith('/local-reviews') ? existingDef.promise : pollDef.promise),
      post: () => startDef.promise,
    },
    ApiError: class extends Error {},
    loadErrorText: (e: Error) => e.message,
    schedulePoll() {},
    initChecked() {},
    toasts: { warn() {} },
    toastError() {},
  });

  const pRefs = s.loadRefs('r1');
  const pExisting = s.loadExisting('r1');
  const pPoll = s.poll();
  const pStart = s.startReview();

  // Switch repo mid-flight.
  s.repoId = 'r2';
  refsDef.resolve({ refs: { remote: [], local: [{ name: 'main' }] }, error: null });
  existingDef.resolve([{ id: 'old-run', status: 'running', comments: [] }]);
  pollDef.resolve({ id: 'old-poll', status: 'done', comments: [] });
  startDef.resolve({ id: 'old-start', status: 'running', comments: [] });
  await Promise.all([pRefs, pExisting, pPoll, pStart]);

  assert.equal(s.refs, null);
  assert.deepEqual(s.history, []);
  assert.equal(s.review, null);
});

test('ConflictResolverView.reconcile drops stale responses after repo switch or dispose', async () => {
  const def = deferred<{ op: string; conflicted_files: string[] }>();
  const s = componentFunctions(git('ConflictResolverView.svelte'), ['reconcile'], {
    repoId: 'r1',
    disposed: false,
    statusGen: 1,
    op: 'merge',
    pending: ['a.ts'],
    resolved: new Set<string>(),
    git: { getMergeStatus: () => def.promise },
  });
  const p = s.reconcile();
  s.repoId = 'r2';
  def.resolve({ op: 'rebase', conflicted_files: ['b.ts'] });
  await p;
  assert.equal(s.op, 'merge');
  assert.deepEqual(s.pending, ['a.ts']);
});

test('FocusView.loadPrs drops earlier overlapping run when a newer loadPrs starts', async () => {
  const first = deferred<{ items: Array<{ number: number; updated_at: string; author: string }> }>();
  const second = deferred<{ items: Array<{ number: number; updated_at: string; author: string }> }>();
  const firstStarted = deferred<void>();
  let call = 0;
  const repo = { id: 'r1', provider: 'github', forge: 'github', git_account_id: 'acc1' };
  const s = componentFunctions(git('FocusView.svelte'), ['loadPrs'], {
    AbortController,
    prsRevision: 0,
    prsAbort: null,
    prsLoading: false,
    prErrors: 0,
    prRows: [],
    myHandles: new Set(),
    git: { allReposLoaded: true, allRepos: [repo], loadAllRepos: async () => {} },
    prCandidateRepos,
    api: {
      get: async () => [{ provider: 'github', username: 'alice' }],
      bg: {
        get: () => {
          if (call++ === 0) {
            firstStarted.resolve();
            return first.promise;
          }
          return second.promise;
        },
      },
    },
    mapLimit: async (items: unknown[], _limit: number, fn: (x: unknown) => Promise<unknown>) =>
      Promise.all(items.map(fn)),
  });
  const p1 = s.loadPrs();
  await firstStarted.promise;
  const p2 = s.loadPrs();
  second.resolve({ items: [{ number: 2, updated_at: '2026-01-02', author: 'alice' }] });
  await p2;
  first.resolve({ items: [{ number: 1, updated_at: '2026-01-01', author: 'alice' }] });
  await p1;
  assert.deepEqual([...s.prRows.map((r: { pr: { number: number } }) => r.pr.number)], [2]);
  assert.equal(s.prsLoading, false);
});

test('FindingsBoard.loadDetail discards stale response when gen advances', async () => {
  const slow = deferred<{ finding: { id: string; status: string }; events: unknown[] }>();
  const fast = deferred<{ finding: { id: string; status: string }; events: unknown[] }>();
  let n = 0;
  const patched: string[] = [];
  const s = componentFunctions(git('FindingsBoard.svelte'), ['loadDetail'], {
    gen: 1,
    detailGen: new Map<string, number>(),
    detailLoading: {},
    detailError: {},
    details: {},
    getFinding: () => (n++ === 0 ? slow.promise : fast.promise),
    patchFinding: (f: { id: string; status: string }) => patched.push(f.status),
  });
  const p1 = s.loadDetail('f1');
  const p2 = s.loadDetail('f1');
  fast.resolve({ finding: { id: 'f1', status: 'fixed' }, events: [] });
  await p2;
  slow.resolve({ finding: { id: 'f1', status: 'open' }, events: [] });
  await p1;
  assert.equal(s.details.f1.finding.status, 'fixed');
  assert.deepEqual(patched, ['fixed']);
});

// ── 8. PR Markdown renders _italic_, GFM tables, and GFM task lists ──────────

test('renderPrMarkdown renders _No description._, GFM tables, and GFM task lists via renderMarkdownGfm', async () => {
  const prevDOMParser = (globalThis as Record<string, unknown>).DOMParser;
  class MinimalDOMParser {
    parseFromString(src: string) {
      const inner = src.replace(/^<body>/, '').replace(/<\/body>$/, '');
      return {
        body: { innerHTML: inner, querySelectorAll: () => [] },
        querySelectorAll: () => [],
      };
    }
  }
  (globalThis as Record<string, unknown>).DOMParser = MinimalDOMParser;
  try {
    const MD_MODULE: string = '../src/lib/md.ts';
    const mdLib = (await import(MD_MODULE)) as { renderMarkdownGfm: (md: string) => string };
    const { renderPrMarkdown } = loadSource(git('prMarkdown.ts'), {
      '../../lib/md': mdLib,
      '../../lib/agentEmojiRender.svelte': { agentEmojiHtml: (_md: string, html: string) => html },
    });

    const emptyDesc = renderPrMarkdown('_No description._');
    assert.match(emptyDesc, /<em>No description\.<\/em>/);
    assert.doesNotMatch(emptyDesc, /_No description\._/);

    const tableHtml = renderPrMarkdown('| Col A | Col B |\n|---|---|\n| 1 | 2 |\n');
    assert.match(tableHtml, /<table>/);
    assert.match(tableHtml, /<th>Col A<\/th>/);
    assert.match(tableHtml, /<td>1<\/td>/);

    const taskHtml = renderPrMarkdown('- [x] Done item\n- [ ] Todo item\n');
    assert.match(taskHtml, /type="checkbox"/);
    assert.match(taskHtml, /checked=""/);
  } finally {
    if (prevDOMParser === undefined) delete (globalThis as Record<string, unknown>).DOMParser;
    else (globalThis as Record<string, unknown>).DOMParser = prevDOMParser;
  }
});

// ── 9. setInterval handle stored & cleared on CreatePr & WipPanel ────────────

test('CreatePr.draftWithAgent tracks draftTick and clears it via clearDraftTick on unmount/finish', async () => {
  const req = deferred<{ title: string; description: string; source_branch: string; session_id: null }>();
  const cleared: number[] = [];
  let nextId = 100;
  const s = componentFunctions(git('CreatePr.svelte'), ['draftWithAgent', 'clearDraftTick'], {
    repoId: 'r1',
    source: 'feat',
    target: 'main',
    title: '',
    description: '',
    alive: true,
    drafting: false,
    draftSessionId: null,
    draftedAt: null,
    draftElapsed: 0,
    draftStartedAt: null,
    draftTick: null,
    Date,
    setInterval: () => ++nextId,
    clearInterval: (id: number) => { cleared.push(id); },
    api: { post: () => req.promise },
    toasts: { warn() {}, info() {} },
    toastError() {},
    confirmer: { ask: async () => true },
  });
  const p = s.draftWithAgent();
  assert.equal(s.draftTick, 101);
  // Simulate unmount mid-draft: clearDraftTick clears the live interval immediately.
  s.alive = false;
  s.clearDraftTick();
  assert.equal(s.draftTick, null);
  assert.deepEqual(cleared, [101]);
  req.resolve({ title: 'T', description: 'D', source_branch: 'feat', session_id: null });
  await p;
  assert.equal(s.drafting, false);
});

test('WipPanel.followDraft stores draftTick so unmount can clear it mid-draft', async () => {
  const req = deferred<{ message: string; from_staged: boolean }>();
  const cleared: number[] = [];
  let nextId = 200;
  const s = componentFunctions(git('WipPanel.svelte'), ['followDraft', 'applyDraft'], {
    alive: true,
    repoId: 'r1',
    subject: '',
    body: '',
    drafting: false,
    draftSessionId: null,
    draftElapsed: 0,
    draftStartedAt: null,
    draftedAt: null,
    draftTick: null,
    Date,
    setInterval: () => ++nextId,
    clearInterval: (id: number) => { cleared.push(id); },
    confirmer: { ask: async () => true },
    toasts: { info() {} },
    toastError() {},
  });
  const p = s.followDraft(req.promise, 'r1');
  assert.equal(s.draftTick, 201);
  req.resolve({ message: 'feat: test', from_staged: true });
  await p;
  assert.equal(s.draftTick, null);
  assert.deepEqual(cleared, [201]);
});

// ── 10. GitToolbar noRemote guard in pullMenu ────────────────────────────────

test('GitToolbar.pullMenu refuses to open when noRemote is true', () => {
  let shown = 0;
  const s = componentFunctions(git('GitToolbar.svelte'), ['pullMenu'], {
    noRemote: true,
    inHeader: true,
    busy: '',
    detached: false,
    NO_REMOTE: 'Add a remote first',
    ctxMenu: { show: () => { shown++; } },
    doFetch() {},
    doPull() {},
  });
  s.pullMenu({});
  assert.equal(shown, 0);

  s.noRemote = false;
  s.pullMenu({});
  assert.equal(shown, 1);
});
