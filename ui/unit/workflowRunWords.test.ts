// The words of a run (ux.md §2): every run state, attention kind, step phase
// and deliverable state has a word and a tone (test-plan guard G2), the sub
// line's rows, the machine-wait clause at 25 %, the inbox's order, row titles
// and the error card's split — runs/runWords.ts.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  ago, attentionWord, childWord, deliverableWord, deliverablesInOrder, errorParts, inboxFolds, ordinal, runLine3, runTitle, runWord,
  stepDuration, stepWord, subLine, type InboxRun,
} from '../src/modules/workflows/runs/runWords.ts';
import type { RunOverview, RunState } from '../src/lib/api/types';
import { at, node, T0 } from './workflowRunFixtures.ts';

const NOW = T0 + 12 * 60_000;
const ms = (min: number) => T0 + min * 60_000;
const ZERO = { queued_ms: 0, waiting_ms: 0, with_you_ms: 0, working_ms: 0 };
function ov(state: RunState, extra: Partial<RunOverview> = {}): RunOverview {
  return { rev: 1, state, headline: '', steps_done: 0, steps_total: 0, attention: [], deliverables: [], usage: {} as RunOverview['usage'], time: ZERO, elapsed_ms: 0, ...extra };
}
const approval = { kind: 'approval' as const, text: 'Approve posting 6 comments', since: at(9), node_id: 'n9', name: 'Post' };

test('every run state has its word and tone', () => {
  const want: [RunState, string, string][] = [
    ['needs_you', 'Approval', 'warning'], ['failed', 'Failed', 'danger'], ['waiting', 'Waiting', ''], ['canceled', 'Canceled', ''],
    ['succeeded', 'Done', 'success'], ['paused', 'Paused', ''], ['blocked', 'Blocked', 'warning'], ['exhausted', 'Out of budget', 'warning'],
  ];
  for (const [state, word, tone] of want) {
    const o = ov(state, state === 'needs_you' ? { attention: [approval] } : {});
    assert.deepEqual(runWord(o), { word, tone }, state);
  }
  assert.equal(runWord(ov('running'), { startedAt: at(0), now: NOW }).word, 'Working 12m');
  assert.equal(runWord(ov('queued', { queue: { position: 2, ahead: 1, cap: 2 } })).word, 'Queued · 2nd');
  assert.deepEqual(runWord(ov('unknown'), { raw: 'rolling_back' }), { word: 'Rolling back', tone: '' });
});

test('the oldest wait names the needs-you word', () => {
  assert.deepEqual(['approval', 'question', 'verify', 'usage_limit', 'unknown'].map(attentionWord), ['Approval', 'Question', 'Verify', 'Limited', 'Needs you']);
  assert.equal(ordinal(1) + ordinal(2) + ordinal(3) + ordinal(11) + ordinal(22), '1st2nd3rd11th22nd');
});

test('step words follow phase, status and error class', () => {
  const w = (n: Parameters<typeof stepWord>[0], run: RunState = 'running', kind?: string) => stepWord(n, { now: NOW, run, kind });
  assert.equal(w(node('a', 'running', 10, null, { phase: 'working' })).word, 'Working 2m');
  assert.equal(w(node('a', 'running', 10, null, { phase: 'queued' })).word, 'Queued');
  assert.equal(w(node('a', 'running', 11, null, { phase: 'waiting', wait: { code: 'slots', text: 'Waiting for a free agent slot', since: at(11) } })).word, 'Waiting 1m');
  assert.equal(w(node('a', 'running', 10, null, { phase: 'starting' })).word, 'Starting');
  assert.equal(w(node('a', 'running', 10, null, { phase: 'finishing' })).word, 'Finishing');
  assert.deepEqual(w(node('a', 'running', 10, null, { phase: 'needs_you', attention: { kind: 'question', text: '?', since: at(11) } })), { word: 'Question', tone: 'warning', glyph: 'needs' });
  assert.equal(w(node('a', 'success', 0, 192_000)).word, '3m 12s');
  assert.equal(w(node('a', 'success', 0, 400)).word, '0.4s');
  assert.equal(w(node('a', 'success', 0, 10, { output: { result: true } }), 'running', 'condition').word, 'Yes');
  assert.deepEqual(w(node('a', 'error', 0, 26 * 60_000)), { word: 'Failed · 26m', tone: 'danger', glyph: 'failed' });
  assert.equal(w(node('a', 'error', 0, 60_000, { error_class: 'timeout' })).word, 'Timed out · 1m');
  assert.equal(w(node('a', 'error', 0, null, { error_class: 'auth' })).word, 'Signed out');
  assert.equal(w(node('a', 'skipped', 0, 10, { error_class: 'canceled' }), 'canceled').word, 'Stopped');
  assert.equal(w(node('a', 'skipped', null, null, { brief: 'Only one repository' })).word, 'Skipped');
  assert.equal(w(node('a', 'skipped', null, null), 'canceled').word, 'Not run');
  assert.equal(w(node('a', 'pending', null, null), 'failed').word, 'Not run');
  assert.equal(w(node('a', 'pending', null, null), 'running').word, '');
  assert.equal(stepDuration(60_000), '1m');
});

test('children and deliverables have their words', () => {
  const c = { id: 'x', label: 'Correctness · Codex', provider: 'codex' };
  assert.equal(childWord({ ...c, status: 'running', started_at: at(6) }, NOW).word, 'Working 6m');
  assert.equal(childWord({ ...c, status: 'success', note: '14 findings' }, NOW).word, 'Done · 14 findings');
  assert.deepEqual(['queued', 'pending', 'error', 'skipped', 'needs_you'].map((status) => childWord({ ...c, status: status as never }, NOW).word), ['Queued', 'Queued', 'Failed', 'Skipped', 'Needs you']);
  const d = (state: string, detail?: string) => deliverableWord({ label: 'x', state: state as never, detail });
  assert.deepEqual(d('done', '5 of 5 posted'), { word: '5 of 5 posted', tone: 'success' });
  assert.deepEqual(d('partial', '3 of 4 posted'), { word: '3 of 4 posted', tone: 'warning' });
  assert.deepEqual(d('failed', 'no permission'), { word: 'Not posted: no permission', tone: 'danger' });
  assert.deepEqual(d('pending_approval'), { word: 'Waiting for approval', tone: 'warning' });
  assert.deepEqual(d('unknown'), { word: 'Not confirmed', tone: '' });
  const order = deliverablesInOrder([{ label: 'a', state: 'done' }, { label: 'b', state: 'failed' }, { label: 'c', state: 'pending_approval' }]);
  assert.deepEqual(order.map((x) => x.label), ['b', 'c', 'a']);
});

test('the sub line, one row per state', () => {
  const run = { startedAt: at(0), now: NOW };
  assert.equal(subLine(ov('running', { typical_ms: 25 * 60_000 }), run), 'Usually about 25m');
  assert.equal(subLine(ov('running', { typical_ms: 25 * 60_000 }), { ...run, waitingSteps: 1 }), 'Usually about 25m · 1 waiting');
  assert.equal(subLine(ov('needs_you', { attention: [approval] }), run), 'Waiting 3m · the run continues when you decide');
  assert.equal(subLine(ov('needs_you', { attention: [{ ...approval, kind: 'question' }] }), run), 'Waiting 3m · the run continues when you answer');
  assert.match(subLine(ov('needs_you', { attention: [{ ...approval, kind: 'usage_limit', resets_at: at(90) }] }), run), /^Otto continues it at /);
  assert.equal(subLine(ov('queued', { queue: { position: 2, ahead: 1, cap: 2 } }), run), 'Both run slots are busy');
  assert.equal(subLine(ov('failed', { typical_ms: 25 * 60_000 }), { startedAt: at(0), finishedAt: at(31), now: ms(91) }), 'After 31m · 1h ago · usually about 25m');
  assert.match(subLine(ov('succeeded', { typical_ms: 25 * 60_000 }), { startedAt: at(0), finishedAt: at(18), now: NOW }), /^Finished at .+ · 7m faster than usual$/);
  assert.doesNotMatch(subLine(ov('succeeded', { typical_ms: 19 * 60_000 }), { startedAt: at(0), finishedAt: at(18), now: NOW }), /usual/);
  assert.equal(subLine(ov('canceled'), { startedAt: at(0), finishedAt: at(14), now: ms(134) }), 'After 14m · 2h ago');
});

test('a finished run waiting 25 % of its time on the machine says so', () => {
  const fin = { startedAt: at(0), finishedAt: at(40), now: ms(41) };
  assert.doesNotMatch(subLine(ov('canceled', { time: { ...ZERO, waiting_ms: 9.9 * 60_000 } }), fin), /machine/);
  assert.match(subLine(ov('canceled', { time: { ...ZERO, waiting_ms: 10 * 60_000 } }), fin), /· 10m of it waiting for the machine$/);
});

const row = (id: string, state: RunState, extra: Partial<InboxRun> = {}): InboxRun => ({
  id, workflowId: 'wf', workflowName: 'PR Reviewer', status: 'running', startedAt: at(0), finishedAt: null, durationMs: null,
  overview: null, state, message: null, repo: null, branch: null, pr: null, currentStep: null, error: null, ...extra,
});

test('inbox: needs you oldest first, queued last by position, unread failures first', () => {
  const runs = [
    row('n2', 'needs_you', { overview: ov('needs_you', { attention: [{ ...approval, since: at(5) }] }) }),
    row('n1', 'blocked', { overview: ov('blocked', { hold: { kind: 'blocked', reason: 'r', since: at(1), node_ids: [] } }) }),
    row('q2', 'queued', { overview: ov('queued', { queue: { position: 2, ahead: 1, cap: 2 } }) }),
    row('q1', 'queued', { overview: ov('queued', { queue: { position: 1, ahead: 0, cap: 2 } }) }),
    row('r1', 'running', { startedAt: at(3) }),
    row('p1', 'paused'),
    row('d1', 'succeeded', { finishedAt: at(9) }),
    row('f1', 'failed', { finishedAt: at(2) }),
    row('f2', 'failed', { finishedAt: at(8) }),
  ];
  const f = inboxFolds(runs, (r) => r.id === 'f1');
  assert.deepEqual(f.needs.map((r) => r.id), ['n1', 'n2']);
  assert.deepEqual(f.running.map((r) => r.id), ['r1', 'p1', 'q1', 'q2']);
  assert.deepEqual(f.recent.map((r) => r.id), ['f1', 'd1', 'f2']);
});

test('row titles and line 3 say what the run is about, in names', () => {
  const pr = ov('running', { headline: 'Multi-agent PR review — 9 of 12 reviewers done', typical_ms: 25 * 60_000, subject: { pr: { repo: 'accounts-service', number: 88, url: 'u', title: 'Refresh tokens' } }, origin: { kind: 'chat', user_name: 'Eli Cohen', channel_name: '#pr-reviews' } });
  assert.equal(runTitle(row('a', 'running', { overview: pr })), 'PR #88 · Refresh tokens');
  assert.equal(runTitle(row('a', 'running', { overview: ov('running', { subject: { pr: { repo: 'api', number: 4, url: 'u' } } }) })), 'api #4');
  assert.equal(runTitle(row('a', 'running', { overview: ov('running', { subject: { ticket: { key: 'WEB-1', url: 'u' } } }) })), 'WEB-1');
  assert.equal(runTitle(row('a', 'running', { message: '\n fix the login page\nmore' })), 'fix the login page');
  assert.equal(runTitle(row('a', 'running')), 'PR Reviewer run');
  assert.equal(runLine3(row('a', 'running', { overview: pr }), NOW), 'Multi-agent PR review — 9 of 12 reviewers done · usually ~25m · Eli in #pr-reviews');
  assert.equal(runLine3(row('a', 'queued', { overview: ov('queued', { headline: 'Queued — 2nd in line' }) }), NOW), '2nd in line');
  assert.equal(runLine3(row('a', 'succeeded', { finishedAt: at(10), overview: ov('succeeded', { headline: 'Passed — 86/100' }) }), ms(130)), 'Passed — 86/100 · 2h ago');
  assert.equal(runLine3(row('a', 'failed', { error: 'boom\nstack' }), NOW), 'Failed: boom');
  const waits = ov('needs_you', { headline: 'Needs you — approve posting 6 comments', attention: [approval] });
  assert.equal(runLine3(row('a', 'needs_you', { overview: waits }), NOW), 'Approve posting 6 comments · 3m ago', 'the word says "needs you"; the rest starts a sentence');
  assert.equal(ago(25 * 3_600_000), '1d ago');
  assert.equal(ago(30_000), 'just now');
});

test('the error card leads with the step’s own first sentence', () => {
  assert.deepEqual(errorParts('review_run: The summarizer passed its 20-minute deadline. 65 findings are saved but not verified.'), {
    title: 'The summarizer passed its 20-minute deadline.', body: '65 findings are saved but not verified.',
  });
  assert.deepEqual(errorParts('exit code 2'), { title: 'exit code 2', body: '' });
});
