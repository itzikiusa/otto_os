// Your review on a PR (modules/git/pr-review-state.ts): read from the
// reviewer list, and the optimistic patch applied when Approve / Request
// changes lands — so the header and reviewer list change at once.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  REVIEW_RECONCILE_WINDOW_MS,
  createReviewReconciler,
  myReview,
  myReviewer,
  reconcileConfirmedReview,
  reviewSummaryFrom,
  withMyReview,
  withoutMyReview,
} from '../src/modules/git/pr-review-state.ts';
import {
  canonicalizeModelSelection,
  formatModelChipLabel,
  stripEmbeddedEffortTier,
  unifyModelCatalog,
} from '../src/lib/providerModels.ts';
import type { PrDetail, PrMergeCheck, PrReviewer, ProviderModelOption } from '../src/lib/api/types';

const me = (n: string) => n.toLowerCase() === 'itzik';
const rv = (name: string, approved = false, changes_requested = false): PrReviewer => ({ name, approved, changes_requested, avatar_url: null, reviewed_at: null });
const pr = (reviewers: PrReviewer[], approved_by: string[] = []) => ({ reviewers, approved_by }) as unknown as PrDetail;

test('reads your current opinion, changes requested first', () => {
  assert.equal(myReview(pr([rv('Ann', true), rv('Itzik')]), me), null);
  assert.equal(myReview(pr([rv('Itzik', true)]), me), 'approved');
  assert.equal(myReview(pr([rv('Itzik', false, true)]), me), 'changes');
  // GitLab lists approvers only in approved_by on some reads.
  assert.equal(myReview(pr([], ['itzik']), me), 'approved');
});

test('request changes after an approval flips the opinion and drops the approval', () => {
  const next = withMyReview(pr([rv('Ann', true), rv('Itzik', true)], ['Ann', 'Itzik']), 'changes', me, 'itzik');
  assert.deepEqual(next.reviewers.map((r) => [r.name, r.approved, r.changes_requested]), [['Ann', true, false], ['Itzik', false, true]]);
  assert.deepEqual(next.approved_by, ['Ann']);
  assert.equal(myReview(next, me), 'changes');
});

test('approving when not yet a reviewer adds you', () => {
  const next = withMyReview(pr([rv('Ann')]), 'approved', me, 'itzik');
  assert.deepEqual(next.reviewers.map((r) => [r.name, r.approved]), [['Ann', false], ['itzik', true]]);
  assert.deepEqual(next.approved_by, ['itzik']);
  assert.ok(next.reviewers[1].reviewed_at);
  assert.equal(next.reviewers[1].is_me, true);
});

// Bitbucket: the account username is an email, reviewers are display names —
// no handle matches, only the daemon's `is_me` flag says which row is you.
const bbMe = (n: string) => n.toLowerCase() === 'itzik@example.com';
const flagged = (name: string, is_me: boolean, approved = false, changes = false): PrReviewer => ({ ...rv(name, approved, changes), is_me });

test('is_me finds you when your display name matches no handle', () => {
  assert.equal(myReview(pr([flagged('Ann Lee', false, true), flagged('Itzik Cohen', true, true)], ['Ann Lee', 'Itzik Cohen']), bbMe), 'approved');
  assert.equal(myReview(pr([flagged('Itzik Cohen', true, false, true)]), bbMe), 'changes');
  assert.equal(myReview(pr([flagged('Ann Lee', false, true), flagged('Itzik Cohen', true)]), bbMe), null);
});

test('is_me wins over a name match; the handle match is only the fallback', () => {
  // A colleague whose display name happens to equal one of your handles.
  assert.equal(myReviewer([flagged('Itzik', false, true), flagged('Itzik Cohen', true)], me)?.name, 'Itzik Cohen');
  // No reviewer flagged (older daemon, or the identity lookup failed): handles.
  assert.equal(myReviewer([flagged('Itzik', false, true)], me)?.name, 'Itzik');
  assert.equal(myReviewer([rv('Itzik')], me)?.name, 'Itzik');
  assert.equal(myReviewer([flagged('Itzik Cohen', false)], bbMe), undefined);
});

test('approving patches the is_me reviewer in place, never a duplicate', () => {
  const next = withMyReview(pr([flagged('Ann Lee', false), flagged('Itzik Cohen', true)]), 'approved', bbMe, 'itzik@example.com');
  assert.deepEqual(next.reviewers.map((r) => [r.name, r.approved, r.is_me]), [['Ann Lee', false, false], ['Itzik Cohen', true, true]]);
  assert.deepEqual(next.approved_by, ['Itzik Cohen']);
  assert.equal(myReview(next, bbMe), 'approved');
});

test('request changes on Bitbucket drops your display-name approval', () => {
  const next = withMyReview(pr([flagged('Ann Lee', false, true), flagged('Itzik Cohen', true, true)], ['Ann Lee', 'Itzik Cohen']), 'changes', bbMe, 'Itzik');
  assert.deepEqual(next.approved_by, ['Ann Lee']);
  assert.equal(next.reviewers.length, 2);
  assert.equal(myReview(next, bbMe), 'changes');
});

test('the reload after an optimistic approve still reads as approved', () => {
  // Optimistic add (not yet a reviewer, flags present on others), then the
  // server's own list with the real display name and `is_me`.
  const optimistic = withMyReview(pr([flagged('Ann Lee', false)]), 'approved', bbMe, 'Itzik');
  assert.equal(myReview(optimistic, bbMe), 'approved');
  const reloaded = pr([flagged('Ann Lee', false), flagged('Itzik Cohen', true, true)], ['Itzik Cohen']);
  assert.equal(myReview(reloaded, bbMe), 'approved');
});

test('withdrawing an approval keeps you as a reviewer, drops the approval', () => {
  const start = pr([flagged('Ann Lee', false, true), flagged('Itzik Cohen', true, true)], ['Ann Lee', 'Itzik Cohen']);
  const next = withoutMyReview(start, 'approved', bbMe);
  assert.equal(myReview(next, bbMe), null);
  assert.deepEqual(next.approved_by, ['Ann Lee']);
  assert.deepEqual(next.reviewers.map((r) => [r.name, r.approved]), [['Ann Lee', true], ['Itzik Cohen', false]]);
});

test('withdrawing a change request clears it; approve → withdraw → approve round-trips', () => {
  const changes = withMyReview(pr([flagged('Ann Lee', false)]), 'changes', bbMe, 'Itzik');
  assert.equal(myReview(changes, bbMe), 'changes');
  const cleared = withoutMyReview(changes, 'changes', bbMe);
  assert.equal(myReview(cleared, bbMe), null);
  assert.equal(cleared.reviewers.length, 2);
  assert.equal(myReview(withMyReview(cleared, 'approved', bbMe, 'Itzik'), bbMe), 'approved');
  // Withdrawing what isn't yours to withdraw changes nothing.
  const untouched = withoutMyReview(pr([flagged('Ann Lee', false, true)], ['Ann Lee']), 'approved', bbMe);
  assert.deepEqual(untouched.approved_by, ['Ann Lee']);
});

test('optimistic review updates PrSummary.review and Bitbucket merge_checks immediately', () => {
  const checks: PrMergeCheck[] = [
    { key: 'approvals', label: 'At least 1 approval', passed: false, detail: '0 of 1' },
    { key: 'no_changes_requested', label: 'No changes requested', passed: true, detail: null },
  ];
  const initial = {
    reviewers: [flagged('Ann Lee', false)],
    approved_by: [],
    review: reviewSummaryFrom([flagged('Ann Lee', false)], []),
    merge_checks: checks,
    updated_at: '2026-11-15T08:00:00Z',
  } as unknown as PrDetail;

  const approved = withMyReview(initial, 'approved', bbMe, 'Itzik');
  assert.deepEqual(approved.review, { status: 'approved', approvals: 1 });
  assert.deepEqual(approved.merge_checks, [
    { key: 'approvals', label: 'At least 1 approval', passed: true, detail: '1 of 1' },
    { key: 'no_changes_requested', label: 'No changes requested', passed: true, detail: null },
  ]);

  const changes = withMyReview(approved, 'changes', bbMe, 'Itzik');
  assert.deepEqual(changes.review, { status: 'changes_requested', approvals: 0 });
  assert.deepEqual(changes.merge_checks, [
    { key: 'approvals', label: 'At least 1 approval', passed: false, detail: '0 of 1' },
    { key: 'no_changes_requested', label: 'No changes requested', passed: false, detail: '1 reviewer requested changes' },
  ]);

  const withdrawn = withoutMyReview(changes, 'changes', bbMe);
  assert.deepEqual(withdrawn.review, { status: 'waiting', approvals: 0 });
  assert.deepEqual(withdrawn.merge_checks, [
    { key: 'approvals', label: 'At least 1 approval', passed: false, detail: '0 of 1' },
    { key: 'no_changes_requested', label: 'No changes requested', passed: true, detail: null },
  ]);
});

test('reconcileConfirmedReview shields UI from lagging read replicas after approve/withdraw', () => {
  // Bitbucket read replica still says Itzik hasn't approved yet right after POST /approve.
  const staleReplica = pr([flagged('Ann Lee', false), flagged('Itzik Cohen', true, false, false)], []);
  const rec1 = reconcileConfirmedReview(staleReplica, 'approved', bbMe, 'Itzik');
  assert.equal(rec1.caughtUp, false);
  assert.equal(myReview(rec1.detail, bbMe), 'approved');
  assert.deepEqual(rec1.detail.review, { status: 'approved', approvals: 1 });

  // Replica catches up on the retry.
  const caughtUpReplica = pr([flagged('Ann Lee', false), flagged('Itzik Cohen', true, true, false)], ['Itzik Cohen']);
  const rec2 = reconcileConfirmedReview(caughtUpReplica, 'approved', bbMe, 'Itzik');
  assert.equal(rec2.caughtUp, true);
  assert.equal(rec2.detail, caughtUpReplica);

  // Lagging replica still says approved right after DELETE /approve.
  const rec3 = reconcileConfirmedReview(caughtUpReplica, null, bbMe, 'Itzik');
  assert.equal(rec3.caughtUp, false);
  assert.equal(myReview(rec3.detail, bbMe), null);
  assert.deepEqual(rec3.detail.review, { status: 'waiting', approvals: 0 });
});

test('createReviewReconciler schedules retry when replica lags and clears once caught up', () => {
  const rec = createReviewReconciler();
  rec.note('repo-1', 101, 'approved');
  const stale = pr([flagged('Itzik Cohen', true, false, false)], []);
  let scheduled = false;
  const out = rec.reconcile('repo-1', 101, stale, bbMe, 'Itzik', () => { scheduled = true; });
  assert.equal(myReview(out, bbMe), 'approved');
  assert.equal(scheduled, false); // timer pending, not fired synchronously
  const caughtUp = pr([flagged('Itzik Cohen', true, true, false)], ['Itzik Cohen']);
  const out2 = rec.reconcile('repo-1', 101, caughtUp, bbMe, 'Itzik');
  assert.equal(out2, caughtUp);
  rec.clear();
});

test('reconcileConfirmedReview preserves server updated_at and createReviewReconciler retries 1.5s/3s/4.5s, expires at 12s, and reports ownReviewSettled', () => {
  let nowMs = 50_000;
  let nextId = 0;
  const timers = new Map<number, { fn: () => void; ms: number }>();
  const rec = createReviewReconciler({
    now: () => nowMs,
    setTimer: (fn, ms) => {
      const id = ++nextId;
      timers.set(id, { fn, ms });
      return id as unknown as ReturnType<typeof setTimeout>;
    },
    clearTimer: (id) => {
      timers.delete(id as unknown as number);
    },
  });

  const stale = {
    reviewers: [flagged('Itzik Cohen', true, false, false)],
    approved_by: [],
    updated_at: '2026-11-15T09:00:00Z',
  } as unknown as PrDetail;

  const direct = reconcileConfirmedReview(stale, 'approved', bbMe, 'Itzik');
  assert.equal(direct.caughtUp, false);
  assert.equal(direct.serverUpdatedAt, '2026-11-15T09:00:00Z');
  assert.equal(direct.detail.updated_at, '2026-11-15T09:00:00Z');

  rec.note('repo-1', 101, 'approved');
  let retryCalls = 0;
  const expectedDelays = [1500, 3000, 4500];
  for (const expectedMs of expectedDelays) {
    const r = rec.reconcileWithMeta('repo-1', 101, stale, bbMe, 'Itzik', () => {
      retryCalls += 1;
    });
    assert.equal(r.ownReviewSettled, false);
    assert.equal(r.detail.updated_at, '2026-11-15T09:00:00Z');
    assert.equal(timers.size, 1);
    const [tid, entry] = [...timers.entries()][0];
    assert.equal(entry.ms, expectedMs);
    timers.delete(tid);
    entry.fn();
  }
  assert.equal(retryCalls, 3);

  // 4th lagging read within window still shields UI, but schedules no 4th timer.
  const r4 = rec.reconcileWithMeta('repo-1', 101, stale, bbMe, 'Itzik', () => {
    retryCalls += 1;
  });
  assert.equal(myReview(r4.detail, bbMe), 'approved');
  assert.equal(timers.size, 0);
  assert.equal(retryCalls, 3);

  // Replica catches up with server's bumped `updated_at`: `ownReviewSettled: true` suppresses false "Updated" pill!
  const caughtUp = {
    reviewers: [flagged('Itzik Cohen', true, true, false)],
    approved_by: ['Itzik Cohen'],
    updated_at: '2026-11-15T09:00:02Z',
  } as unknown as PrDetail;
  const settled = rec.reconcileWithMeta('repo-1', 101, caughtUp, bbMe, 'Itzik');
  assert.equal(settled.detail, caughtUp);
  assert.equal(settled.ownReviewSettled, true);

  // Subsequent poll after someone else comments is NOT treated as own review settling.
  const laterComment = { ...caughtUp, updated_at: '2026-11-15T09:05:00Z' } as PrDetail;
  const after = rec.reconcileWithMeta('repo-1', 101, laterComment, bbMe, 'Itzik');
  assert.equal(after.ownReviewSettled, false);

  // Window expiry after `REVIEW_RECONCILE_WINDOW_MS` (12s) drops stale write override cleanly.
  rec.note('repo-1', 101, 'approved');
  nowMs += REVIEW_RECONCILE_WINDOW_MS + 1;
  const expired = rec.reconcileWithMeta('repo-1', 101, stale, bbMe, 'Itzik', () => {
    retryCalls += 1;
  });
  assert.equal(expired.detail, stale);
  assert.equal(myReview(expired.detail, bbMe), null);
  assert.equal(timers.size, 0);
});

test('providerModels scopes tier-suffix unification to agy and WeakMap-memoizes catalog arrays', () => {
  const agyRaw: ProviderModelOption[] = [
    { id: 'gemini-4-carbon-low', label: 'Gemini 4 Carbon (Low)', description: 'Fast', default_effort: null, supported_efforts: [] },
    { id: 'gemini-4-carbon-medium', label: 'Gemini 4 Carbon (Medium)', description: 'Balanced', default_effort: null, supported_efforts: [] },
    { id: 'gemini-4-carbon-high', label: 'Gemini 4 Carbon (High)', description: 'Deep', default_effort: null, supported_efforts: [] },
  ];
  const unified1 = unifyModelCatalog('agy', agyRaw);
  const unified2 = unifyModelCatalog('agy', agyRaw);
  assert.equal(unified1, unified2, 'expected WeakMap memoization to return identical array instance');
  assert.equal(unified1.length, 1);
  assert.equal(unified1[0].id, 'gemini-4-carbon');
  assert.equal(unified1[0].label, 'Gemini 4 Carbon');
  assert.deepEqual(unified1[0].supported_efforts, ['low', 'medium', 'high']);

  const meta = { provider_models: { agy: agyRaw } };
  assert.deepEqual(canonicalizeModelSelection('agy', 'gemini-4-carbon-high', null, meta), {
    model: 'gemini-4-carbon',
    effort: 'high',
  });
  assert.equal(formatModelChipLabel('agy', 'gemini-4-carbon-high', meta), 'Gemini 4 Carbon');

  // Non-`agy` providers MUST NEVER rewrite custom model IDs ending in `-low` / `-high` / `-max`!
  const customModels: ProviderModelOption[] = [
    { id: 'custom-sonnet-high', label: 'Custom Sonnet (High)', description: 'Gateway route', default_effort: null, supported_efforts: [] },
    { id: 'gpt-5-mini-low', label: 'GPT-5 Mini (Low)', description: 'Low latency route', default_effort: null, supported_efforts: [] },
    { id: 'qwen-coder-max', label: 'Qwen Coder Max', description: 'Alibaba flagship', default_effort: null, supported_efforts: [] },
  ];
  for (const provider of ['claude', 'codex', 'opencode', 'pi']) {
    assert.equal(stripEmbeddedEffortTier(provider, 'custom-sonnet-high', 'Custom Sonnet (High)'), null);
    const out = unifyModelCatalog(provider, customModels);
    assert.equal(out, unifyModelCatalog(provider, customModels));
    assert.deepEqual(
      out.map((m) => m.id),
      ['custom-sonnet-high', 'gpt-5-mini-low', 'qwen-coder-max'],
    );
    const nonAgyMeta = { provider_models: { [provider]: customModels } };
    assert.deepEqual(canonicalizeModelSelection(provider, 'qwen-coder-max', null, nonAgyMeta), {
      model: 'qwen-coder-max',
      effort: null,
    });
    assert.equal(formatModelChipLabel(provider, 'unlisted-gateway-high', nonAgyMeta), 'unlisted-gateway-high');
  }
});
