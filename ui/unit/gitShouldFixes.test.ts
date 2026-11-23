// Wave 2 (SHOULD — S-39) unit tests for ui/src/modules/git/.
// Pure TypeScript helper & module tests only (zero .svelte regex reads).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { register } from 'node:module';
import { loadSource } from './sourceHarness.ts';
import type { ReviewConfig } from '../src/lib/api/types.ts';

const hook =
  'export async function resolve(s, c, n) {' +
  "  if (/^\\.{1,2}\\//.test(s) && !/\\.[a-z]+$/i.test(s)) { try { return await n(s + '.ts', c); } catch {} }" +
  '  return n(s, c);' +
  '}';
register('data:text/javascript,' + encodeURIComponent(hook));

const helpers = await import('../src/modules/git/gitUiHelpers.ts');

// ── 1. Compact PR header on Code & AI review tabs ────────────────────────────

test('isCompactPrHeaderTab collapses PR header only on code and review tabs', () => {
  assert.equal(helpers.isCompactPrHeaderTab('code'), true);
  assert.equal(helpers.isCompactPrHeaderTab('review'), true);
  assert.equal(helpers.isCompactPrHeaderTab('summary'), false);
  assert.equal(helpers.isCompactPrHeaderTab('timeline'), false);
});

// ── 2. Provider-aware PR close/decline copy ──────────────────────────────────

test('prCloseLabels uses Close copy on GitHub/GitLab and Decline copy on Bitbucket', () => {
  const gh = helpers.prCloseLabels('github');
  assert.equal(gh.button, 'Close PR…');
  assert.equal(gh.busy, 'Closing…');
  assert.equal(gh.verb, 'Close PR');
  assert.equal(gh.title(42), 'Close pull request #42?');
  assert.equal(gh.menu, 'Close pull request…');
  assert.equal(gh.toastDone, 'closed');
  assert.equal(gh.toastFail, 'Couldn’t close the PR');

  const gl = helpers.prCloseLabels('gitlab');
  assert.equal(gl.button, 'Close PR…');
  assert.equal(gl.verb, 'Close PR');

  const unknown = helpers.prCloseLabels(null);
  assert.equal(unknown.button, 'Close PR…');

  const bb = helpers.prCloseLabels('bitbucket');
  assert.equal(bb.button, 'Decline…');
  assert.equal(bb.busy, 'Declining…');
  assert.equal(bb.verb, 'Decline PR');
  assert.equal(bb.title(7), 'Decline PR #7?');
  assert.equal(bb.menu, 'Decline pull request…');
  assert.equal(bb.toastDone, 'declined');
  assert.equal(bb.toastFail, 'Couldn’t decline the PR');
});

// ── 3. ReviewConfigModal helpers (shared by PR & Local Review) ───────────────

test('repoReviewConfigBadge describes global, preset, and repo-custom scopes', () => {
  assert.equal(helpers.repoReviewConfigBadge(null), '');
  assert.equal(helpers.repoReviewConfigBadge({ scope: 'global', preset_name: null }), '');
  assert.equal(helpers.repoReviewConfigBadge({ scope: 'preset', preset_name: 'Backend' }), 'Backend');
  assert.equal(helpers.repoReviewConfigBadge({ scope: 'preset', preset_name: null }), 'preset');
  assert.equal(helpers.repoReviewConfigBadge({ scope: 'custom', preset_name: null }), 'repo config');
});

test('normalizeReviewAgent and toggleAgentProvider always preserve at least one provider', () => {
  const norm = helpers.normalizeReviewAgent(
    { name: 'Sec', provider: 'codex', model: '', prompt: '' },
    'claude',
  );
  assert.deepEqual(norm.providers, ['codex']);

  const fallbackNorm = helpers.normalizeReviewAgent(
    { name: 'Empty', provider: '', providers: [], model: '', prompt: '' },
    'claude',
  );
  assert.deepEqual(fallbackNorm.providers, ['claude']);

  assert.deepEqual(helpers.toggleAgentProvider(['claude'], 'codex', 'claude'), ['claude', 'codex']);
  assert.deepEqual(helpers.toggleAgentProvider(['claude', 'codex'], 'claude', 'claude'), ['codex']);
  // Removing the last provider falls back instead of returning an empty array.
  assert.deepEqual(helpers.toggleAgentProvider(['codex'], 'codex', 'claude'), ['claude']);
});

test('sameReviewConfig ignores custom_presets and legacy provider field while catching drift', () => {
  const base: ReviewConfig = {
    agents: [{ name: 'Correctness', provider: 'claude', providers: ['claude'], model: '', prompt: 'check' }],
    summarizer: { name: 'Summarizer', provider: 'claude', providers: [], model: '', prompt: 'merge' },
    custom_presets: [],
  };
  const withPresets: ReviewConfig = {
    ...base,
    agents: [{ name: 'Correctness', provider: '', providers: ['claude'], model: '', prompt: 'check' }],
    custom_presets: [{ name: 'Saved', provider: 'codex', providers: ['codex'], model: '', prompt: 'x' }],
  };
  assert.equal(helpers.sameReviewConfig(base, withPresets, 'claude'), true);

  const drifted: ReviewConfig = {
    ...base,
    agents: [{ name: 'Correctness', provider: 'claude', providers: ['claude', 'codex'], model: '', prompt: 'check' }],
  };
  assert.equal(helpers.sameReviewConfig(base, drifted, 'claude'), false);
});

// ── 4. ReviewPanel done-state input collapse ─────────────────────────────────

test('shouldCollapseReviewInputs hides inputs on done until re-run disclosure is opened', () => {
  assert.equal(helpers.shouldCollapseReviewInputs('done', false), true);
  assert.equal(helpers.shouldCollapseReviewInputs('done', true), false);
  assert.equal(helpers.shouldCollapseReviewInputs(undefined, false), false);
  assert.equal(helpers.shouldCollapseReviewInputs('error', false), false);
  assert.equal(helpers.shouldCollapseReviewInputs('cancelled', false), false);
});

// ── 5. Inline PR comment & reply composer ⌘↵ submit + outward notice ─────────

test('isComposerSubmitKey accepts Meta/Ctrl+Enter outside IME composition only', () => {
  assert.equal(helpers.isComposerSubmitKey({ key: 'Enter', metaKey: true, ctrlKey: false }), true);
  assert.equal(helpers.isComposerSubmitKey({ key: 'Enter', metaKey: false, ctrlKey: true }), true);
  assert.equal(helpers.isComposerSubmitKey({ key: 'Enter', metaKey: false, ctrlKey: false }), false);
  assert.equal(helpers.isComposerSubmitKey({ key: 'a', metaKey: true, ctrlKey: false }), false);
  assert.equal(helpers.isComposerSubmitKey({ key: 'Enter', metaKey: true, ctrlKey: false, isComposing: true }), false);
});

test('prCommentPostHint names PR destination and visibility for post, reply, and edit', () => {
  assert.equal(
    helpers.prCommentPostHint('PR #18 on otto', 'post'),
    'Posts to PR #18 on otto · visible to the author and reviewers',
  );
  assert.equal(
    helpers.prCommentPostHint('PR #18 on otto', 'reply'),
    'Replies on PR #18 on otto · visible to the author and reviewers',
  );
  assert.equal(
    helpers.prCommentPostHint('PR #18 on otto', 'edit'),
    'Updates your comment on PR #18 on otto · visible to the author and reviewers',
  );
  assert.equal(
    helpers.prCommentPostHint('', 'post'),
    'Posts to this PR · visible to the author and reviewers',
  );
});

// ── 6. DiffNav RTL drag inversion ────────────────────────────────────────────

test('computeSidebarResizeWidth flips horizontal drag delta in RTL and clamps bounds', () => {
  // LTR: dragging right (+40px) widens sidebar.
  assert.equal(helpers.computeSidebarResizeWidth(240, 200, 240, false), 280);
  // RTL: dragging right (+40px) narrows sidebar; dragging left (-40px) widens it.
  assert.equal(helpers.computeSidebarResizeWidth(240, 200, 240, true), 200);
  assert.equal(helpers.computeSidebarResizeWidth(240, 200, 160, true), 280);
  // Clamping at min (180) and max (520).
  assert.equal(helpers.computeSidebarResizeWidth(240, 200, 0, false), 180);
  assert.equal(helpers.computeSidebarResizeWidth(240, 200, 900, false), 520);
  assert.equal(helpers.computeSidebarResizeWidth(240, 900, 0, true), 520);
});

// ── 7. WipPanel commit draft Stop / cancelDraft ──────────────────────────────

test('commitComposer.cancelDraft aborts signal, clears in-flight entry, and blocks parking', async () => {
  const store = new Map<string, string>();
  const sessionStorage = {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => void store.set(k, v),
    removeItem: (k: string) => void store.delete(k),
  };
  const c = loadSource(
    new URL('../src/modules/git/commitComposer.ts', import.meta.url),
    {},
    { sessionStorage },
  );

  let capturedSignal: AbortSignal | undefined;
  let resolveRun!: (v: { message: string; from_staged: boolean }) => void;
  const p = c.startDraft('repo-1', (signal: AbortSignal) => {
    capturedSignal = signal;
    return new Promise((r) => { resolveRun = r; });
  });

  assert.equal(c.draftInFlight('repo-1'), p);
  assert.equal(capturedSignal?.aborted, false);

  c.cancelDraft('repo-1');
  assert.equal(capturedSignal?.aborted, true, 'AbortSignal aborted on Stop');
  assert.equal(c.draftInFlight('repo-1'), null, 'in-flight map cleared immediately');

  // Even if the underlying promise later resolves, it must not park into sessionStorage.
  resolveRun({ message: 'late draft', from_staged: true });
  await p;
  await new Promise((r) => setImmediate(r));
  assert.equal(c.takePendingDraft('repo-1'), null);
});

// ── 8. Focus target on PR tab close & GitPage accounts fetch gating ──────────

test('nextTabAfterClose picks adjacent tab or null when closing the last tab', () => {
  assert.equal(helpers.nextTabAfterClose([10, 20, 30], 20), 30);
  assert.equal(helpers.nextTabAfterClose([10, 20, 30], 30), 20);
  assert.equal(helpers.nextTabAfterClose([10, 20, 30], 10), 20);
  assert.equal(helpers.nextTabAfterClose([10], 10), null);
  assert.equal(helpers.nextTabAfterClose([], 10), null);
});

test('shouldFetchGitAccounts loads on mount and when addOpen turns true, not on close', () => {
  assert.equal(helpers.shouldFetchGitAccounts(false, false), true, 'initial mount loads accounts');
  assert.equal(helpers.shouldFetchGitAccounts(true, false), true, 'initial mount with modal open loads accounts');
  assert.equal(helpers.shouldFetchGitAccounts(true, true), true, 'opening Add Repository refreshes accounts');
  assert.equal(helpers.shouldFetchGitAccounts(false, true), false, 'closing Add Repository does not re-fetch');
});
