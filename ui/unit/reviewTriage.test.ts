import { test } from 'node:test';
import assert from 'node:assert/strict';
import { advancedOf, mergeReviewConfig, syncProviders } from '../src/modules/git/reviewConfigMerge.ts';
import { groupByFile, postAllPlan, postAllSkip, triageIndex } from '../src/modules/git/reviewTriage.ts';
import { canBulk, groupFindings, moveFocus } from '../src/modules/git/findingsTriage.ts';
import type { Finding, ReviewComment, ReviewConfig, ReviewTriage } from '../src/lib/api/types/review.ts';

const agent = { name: 'Security', provider: 'claude', providers: ['claude'], model: '', prompt: 'p' };

test('saving the agents modal keeps every field it does not edit', () => {
  const loaded = {
    cross_provider_check: false,
    agents: [agent],
    summarizer: { ...agent, name: 'Summarizer' },
    custom_presets: [],
    mode: 'orchestrator',
    max_attempts: 5,
    timeout_secs: 900,
    medium_pr_lines: 800,
    large_pr_lines: 4000,
    previous_review_id: 'r1',
    // A field a newer daemon added that this UI does not know.
    future_knob: { on: true },
  } as unknown as ReviewConfig;
  const edited = mergeReviewConfig(
    loaded,
    { cross_provider_check: true, agents: [{ ...agent, name: 'Perf' }], summarizer: loaded.summarizer, custom_presets: [] },
    advancedOf(loaded),
  ) as unknown as Record<string, unknown>;
  assert.equal(edited.cross_provider_check, true);
  assert.equal((edited.agents as { name: string }[])[0].name, 'Perf');
  for (const k of ['mode', 'max_attempts', 'timeout_secs', 'medium_pr_lines', 'large_pr_lines', 'previous_review_id']) {
    assert.deepEqual(edited[k], (loaded as unknown as Record<string, unknown>)[k], k);
  }
  assert.deepEqual(edited.future_knob, { on: true });
});

test('an Advanced field cleared in the modal falls back to the default; a new one is saved', () => {
  const loaded = { agents: [], summarizer: agent, max_attempts: 5, mode: 'orchestrator' } as unknown as ReviewConfig;
  const out = mergeReviewConfig(loaded, { agents: [], summarizer: agent }, { ...advancedOf(loaded), max_attempts: null, mode: null, timeout_secs: 600 });
  assert.equal('max_attempts' in out, false);
  assert.equal('mode' in out, false);
  assert.equal(out.timeout_secs, 600);
  // NaN / 0 from a bad number input never reaches the daemon.
  const bad = mergeReviewConfig(null, { agents: [], summarizer: agent }, { ...advancedOf(null), large_pr_lines: Number.NaN, medium_pr_lines: 0 });
  assert.equal('large_pr_lines' in bad, false);
  assert.equal('medium_pr_lines' in bad, false);
});

test('provider sync keeps `provider` = the first of `providers`', () => {
  assert.deepEqual(syncProviders({ ...agent, providers: ['codex', 'claude'] }, 'claude').provider, 'codex');
  assert.deepEqual(syncProviders({ ...agent, provider: '', providers: [] }, 'agy').providers, ['agy']);
});

const comment = (id: string, path: string | null, line: number | null, body: string, severity: ReviewComment['severity'] = 'warn'): ReviewComment => ({
  id, review_id: 'r2', path, line, severity, body, state: 'draft', posted: false, created_at: '2026-11-12T00:00:00Z',
});

test('Post all skips dismissed findings, copies already on the PR and unverified claims', () => {
  const cs = [
    comment('ok', 'a.rs', 1, 'Real issue', 'bug'),
    comment('fp', 'a.rs', 2, 'Dismissed'),
    comment('dup', 'b.rs', 3, 'Posted before'),
    comment('unv', 'b.rs', 4, '[unverified] Maybe a race'),
    comment('noTriage', null, null, 'General note', 'info'),
  ];
  const t: ReviewTriage = {
    review_id: 'r2',
    prior: [],
    comments: [
      { comment_id: 'ok', finding_id: 'f1', finding_status: 'open', unverified: false, on_pr: null },
      { comment_id: 'fp', finding_id: 'f2', finding_status: 'false_positive', unverified: false, on_pr: null },
      { comment_id: 'dup', finding_id: 'f3', finding_status: 'open', unverified: false, on_pr: { review_id: 'r1', comment_id: 'c9' } },
      { comment_id: 'unv', finding_id: null, finding_status: null, unverified: true, on_pr: null },
    ],
  };
  const ix = triageIndex(t);
  assert.equal(postAllSkip(cs[1], ix.get('fp')), 'dismissed');
  assert.equal(postAllSkip(cs[2], ix.get('dup')), 'on_pr');
  const plan = postAllPlan(cs, ix);
  assert.deepEqual(plan.send.map((c) => c.id), ['ok', 'noTriage'], 'an untriaged comment still posts');
  assert.deepEqual(plan.unverified.map((c) => c.id), ['unv']);
  assert.equal(plan.dismissed, 1);
  assert.equal(plan.onPr, 1);
  // Before the triage loads, the body mark alone keeps an unverified claim out.
  assert.deepEqual(postAllPlan(cs, triageIndex(null)).unverified.map((c) => c.id), ['unv']);
});

test('the confirm groups by file (general last), bugs first inside a file', () => {
  const g = groupByFile([
    comment('g', null, null, 'general'),
    comment('b2', 'b.rs', 9, 'x', 'info'),
    comment('b1', 'b.rs', 20, 'y', 'bug'),
    comment('a1', 'a.rs', 5, 'z'),
  ]);
  assert.deepEqual(g.map((x) => x.path), ['a.rs', 'b.rs', null]);
  assert.deepEqual(g[1].comments.map((c) => c.id), ['b1', 'b2']);
});

const finding = (id: string, path: string | null, severity: Finding['severity'], status: Finding['status'], line = 1): Finding =>
  ({ id, path, severity, status, line, title: id, body: id }) as unknown as Finding;

test('findings group by file or severity, severity-sorted inside a group', () => {
  const fs = [
    finding('low-a', 'a.rs', 'low', 'open'),
    finding('crit-b', 'b.rs', 'critical', 'open'),
    finding('high-a', 'a.rs', 'high', 'accepted', 9),
    finding('gen', null, 'info', 'open'),
  ];
  const byFile = groupFindings(fs, 'file');
  assert.deepEqual(byFile.map((g) => g.label), ['a.rs', 'b.rs', 'General']);
  assert.deepEqual(byFile[0].items.map((f) => f.id), ['high-a', 'low-a']);
  const bySev = groupFindings(fs, 'severity');
  assert.deepEqual(bySev.map((g) => g.key), ['critical', 'high', 'low', 'info']);
});

test('bulk actions follow the legal status transitions; j/k stay inside the list', () => {
  assert.equal(canBulk(finding('a', null, 'high', 'open'), 'accept'), true);
  assert.equal(canBulk(finding('a', null, 'high', 'accepted'), 'accept'), false);
  assert.equal(canBulk(finding('a', null, 'high', 'verified'), 'waive'), false);
  assert.equal(canBulk(finding('a', null, 'high', 'verified'), 'false_positive'), true);
  assert.equal(canBulk(finding('a', null, 'high', 'waived'), 'false_positive'), false);
  assert.equal(moveFocus(-1, 'j', 3), 0);
  assert.equal(moveFocus(2, 'j', 3), 2);
  assert.equal(moveFocus(0, 'k', 3), 0);
  assert.equal(moveFocus(0, 'j', 0), -1);
});
