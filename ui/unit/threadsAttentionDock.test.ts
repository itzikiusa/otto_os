// One structured thread as a person meets it: what waits on them (the
// attention dock), how Stop is reached by key, one calm failure state, a
// child's reply read as "Reply from …", and a turn's tokens in words.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  attentionHeadline, attentionItems, attentionPillText, currentIndex, dockKeyAction, keyDecision, keyHint, stepId,
} from '../src/modules/threads/runtime/attentionDock.ts';
import { composerStopKey, ESC_WINDOW_MS, isStopChord } from '../src/modules/threads/runtime/stopKeys.ts';
import { failureCause, failureView, turnErrorItem } from '../src/modules/threads/runtime/turnFailure.ts';
import { formatTokens, threadTokenTotal, turnTokens, turnUsageText } from '../src/modules/threads/runtime/turnUsage.ts';
import { messageAttribution } from '../src/modules/threads/runtime/timeline.ts';
import { queuedLabel } from '../src/modules/threads/runtime/queueRows.ts';
import { followPillText } from '../src/modules/threads/runtime/follow.ts';

const live = { type: 'live' } as const;
const req = (id: string, kind: string, at: string, extra: Record<string, unknown> = {}) =>
  ({ id, kind, status: 'pending', created_at: at, response_capability: live, ...extra }) as never;

test('the dock lists a secret first, then approvals and questions oldest first', () => {
  const items = attentionItems(
    [
      req('q2', 'question', '2026-11-16T10:00:05Z'),
      req('a1', 'command', '2026-11-16T10:00:01Z'),
      req('done', 'command', '2026-11-16T09:00:00Z', { status: 'resolved' }),
      req('gone', 'question', '2026-11-16T09:00:00Z', { response_capability: { type: 'not_resumable' } }),
      req('a3', 'file_change', '2026-11-16T10:00:09Z'),
    ],
    { request_id: 's1' },
  );
  assert.deepEqual(items.map((i) => [i.kind, i.id]), [['secret', 's1'], ['approval', 'a1'], ['question', 'q2'], ['approval', 'a3']]);
  assert.equal(items[1].requestKind, 'command');
  assert.deepEqual(attentionItems([], null), []);
});

test('the current item is the picked one while it waits, else the first; ‹ › wrap', () => {
  const items = attentionItems([req('a', 'command', '2026-11-16T10:00:00Z'), req('b', 'command', '2026-11-16T10:00:01Z')], null);
  assert.equal(currentIndex(items, null), 0);
  assert.equal(currentIndex(items, 'b'), 1);
  assert.equal(currentIndex(items, 'answered-and-gone'), 0);
  assert.equal(currentIndex([], 'a'), -1);
  assert.equal(stepId(items, 'b', 1), 'a');
  assert.equal(stepId(items, null, -1), 'b');
  assert.equal(stepId([], null, 1), null);
});

test('the headline says who wants what; the pill names a waiting request', () => {
  const [approval] = attentionItems([req('a', 'command', '2026-11-16T10:00:00Z')], null);
  assert.equal(attentionHeadline(approval, 'Codex'), 'Codex wants to run');
  assert.equal(attentionHeadline({ kind: 'question', id: 'q', at: null }, 'Claude'), 'Claude asks');
  assert.equal(attentionHeadline({ kind: 'secret', id: 's', at: null }, 'Claude'), 'Claude asks for a secret');
  assert.equal(attentionPillText([approval]), 'Approval needed');
  assert.equal(attentionPillText([approval, { kind: 'question', id: 'q', at: null }]), 'Needs your answer');
  assert.equal(attentionPillText([]), null);
  // The pill: a waiting request outranks a message count.
  assert.equal(followPillText(3, 0, false, 'Approval needed'), 'Approval needed');
  assert.equal(followPillText(3, 0, false), '3 new messages');
});

const key = (k: string, mods: Partial<{ metaKey: boolean; ctrlKey: boolean; altKey: boolean; shiftKey: boolean }> = {}) =>
  ({ key: k, metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, ...mods });

test('dock keys answer only when they cannot be typing', () => {
  const ctx = { kind: 'approval' as const, composerEmpty: true, inDock: false, inTextField: true, count: 2 };
  assert.equal(dockKeyAction(key('Enter', { metaKey: true }), ctx), 'accept');
  assert.equal(dockKeyAction(key('Backspace', { ctrlKey: true }), ctx), 'decline');
  // Text in the composer: ⌘↵ / ⌘⌫ belong to the composer.
  assert.equal(dockKeyAction(key('Enter', { metaKey: true }), { ...ctx, composerEmpty: false }), null);
  assert.equal(dockKeyAction(key('Backspace', { metaKey: true }), { ...ctx, composerEmpty: false }), null);
  // Plain keys never answer; no modifier = typing.
  assert.equal(dockKeyAction(key('Enter'), ctx), null);
  assert.equal(dockKeyAction(key('y'), ctx), null);
  assert.equal(dockKeyAction(key('Enter', { metaKey: true, altKey: true }), ctx), null);
  // Inside the dock: its buttons answer and arrows step; a text field there does not.
  const dock = { ...ctx, composerEmpty: false, inDock: true, inTextField: false };
  assert.equal(dockKeyAction(key('Enter', { metaKey: true }), dock), 'accept');
  assert.equal(dockKeyAction(key('ArrowRight'), dock), 'next');
  assert.equal(dockKeyAction(key('ArrowLeft'), dock), 'prev');
  assert.equal(dockKeyAction(key('ArrowRight'), { ...dock, count: 1 }), null);
  assert.equal(dockKeyAction(key('ArrowRight'), ctx), null, 'arrows in the composer move the caret');
  assert.equal(dockKeyAction(key('Enter', { metaKey: true }), { ...dock, inTextField: true }), null);
  // A question form: ⌘↵ inside it sends; from the composer nothing.
  const q = { ...dock, kind: 'question' as const, inTextField: true };
  assert.equal(dockKeyAction(key('Enter', { metaKey: true }), q), 'send');
  assert.equal(dockKeyAction(key('Enter', { metaKey: true }), { ...q, inDock: false, composerEmpty: true }), null);
  assert.equal(dockKeyAction(key('Enter', { metaKey: true }), { ...ctx, kind: null }), null);
  assert.equal(dockKeyAction({ ...key('Enter', { metaKey: true }), isComposing: true }, ctx), null);
});

test('a keyboard answer uses the agent’s own accept / decline, or none when it offers neither', () => {
  assert.equal(keyDecision([], 'accept'), 'accept');
  assert.equal(keyDecision([{ decision: 'accept', label: 'Yes' }, { decision: 'decline', label: 'No' }], 'decline'), 'decline');
  assert.equal(keyDecision([{ decision: 'accept_for_session', label: 'For this session' }], 'accept'), null);
  assert.equal(keyHint('accept', true), '⌘↵');
  assert.equal(keyHint('decline', false), 'Ctrl+⌫');
});

test('Esc stops a running turn: at once in an empty composer, twice with text in it', () => {
  const st = { lastEscAt: -Infinity };
  const esc = key('Escape');
  assert.equal(composerStopKey(esc, { running: false, composerEmpty: true, now: 0 }, st), false, 'nothing runs');
  assert.equal(composerStopKey(esc, { running: true, composerEmpty: true, now: 0 }, st), true);
  assert.equal(composerStopKey(esc, { running: true, composerEmpty: false, now: 1000 }, st), false, 'one stray Esc keeps the turn');
  assert.equal(composerStopKey(esc, { running: true, composerEmpty: false, now: 1000 + ESC_WINDOW_MS - 1 }, st), true);
  // Too slow: the second press starts a new pair.
  assert.equal(composerStopKey(esc, { running: true, composerEmpty: false, now: 5000 }, st), false);
  assert.equal(composerStopKey(esc, { running: true, composerEmpty: false, now: 5000 + ESC_WINDOW_MS + 1 }, st), false);
  assert.equal(composerStopKey(key('Escape', { shiftKey: true }), { running: true, composerEmpty: true, now: 9000 }, st), false);
  assert.equal(composerStopKey(key('.', { metaKey: true }), { running: true, composerEmpty: false, now: 9999 }, st), true);
  assert.equal(isStopChord(key('.', { ctrlKey: true })), true);
  assert.equal(isStopChord(key('.')), false);
  assert.equal(isStopChord(key('.', { metaKey: true, shiftKey: true })), false);
});

test('a failure reads as its cause, the raw text kept for Details', () => {
  assert.equal(failureCause({ class: 'provider_error', message: 'unsupported control request: initialize' }), 'cli_unsupported');
  assert.equal(failureCause({ class: 'handshake_timeout', message: 'no answer' }), 'cli_unsupported');
  assert.equal(failureCause({ class: 'usage_limit', message: '' }), 'usage_limit');
  assert.equal(failureCause({ class: 'provider_error', message: "You've hit your usage limit · resets 5pm" }), 'usage_limit');
  assert.equal(failureCause({ class: 'provider_error', message: 'Invalid API key · Please run /login' }), 'signed_out');
  assert.equal(failureCause({ class: 'provider_error', message: 'spawn claude ENOENT' }), 'cli_missing');
  assert.equal(failureCause({ class: 'transport_error', message: 'pipe' }), 'connection');
  assert.equal(failureCause({ class: 'provider_error', message: 'model overloaded' }), 'provider');
  assert.equal(failureCause(null), 'provider');
  const v = failureView({ class: 'provider_error', message: ' unsupported control request: initialize ' }, 'Claude Code');
  assert.equal(v.title, 'This version of Claude Code isn’t supported');
  assert.match(v.hint, /Update Claude Code/);
  assert.equal(v.detail, 'unsupported control request: initialize');
  assert.equal(failureView(null, 'Codex').detail, '');
});

test('the failure block stands in for its turn’s last top-level error row', () => {
  const base = { status: 'failed', title: null, started_at: null, completed_at: null, updated_at: '' };
  const items = [
    { ...base, id: 'e0', turn_id: 't0', ordinal: 1, type: 'error', failure: { class: 'x', message: 'old' } },
    { ...base, id: 'e1', turn_id: 't1', ordinal: 2, type: 'error', failure: { class: 'x', message: 'a' } },
    { ...base, id: 'sub', turn_id: 't1', ordinal: 3, type: 'error', parent_item_id: 'agent', failure: { class: 'x', message: 'b' } },
    { ...base, id: 'c', turn_id: 't1', ordinal: 4, type: 'command_execution', input: 'ls' },
  ] as never;
  assert.equal(turnErrorItem(items, 't1')?.id, 'e1');
  assert.equal(turnErrorItem(items, 't9'), null);
  assert.equal(turnErrorItem(items, null), null);
});

test('a child’s reply woken into its parent reads "Reply from <thread>" (the daemon’s exact text)', () => {
  // crates/otto-agent-protocol/src/runtime/children.rs `wake_text_how`.
  const wake =
    '[Reply from thread "Fix the build" (codex, id 01JTEST0000000000000000000): its turn finished. Answer it with the ' +
    'otto_thread_message tool (session_id 01JTEST0000000000000000000); read it again with otto_thread_read.]\n\nAll green [3 tests].';
  assert.deepEqual(messageAttribution(wake), { from: 'Fix the build', body: 'All green [3 tests].', reply: true });
  const stopped = wake.replace('its turn finished', 'its turn was stopped').replace('Fix the build', 'Odd ] title');
  assert.deepEqual(messageAttribution(stopped), { from: 'Odd ] title', body: 'All green [3 tests].', reply: true });
  // Unchanged shapes keep their exact result (no `reply` key).
  assert.deepEqual(
    messageAttribution('[Opened by thread "Plan" (claude, id 01X). Answer it with the otto_thread_message tool (session_id 01X); see every linked thread with otto_thread_list.]\n\nGo'),
    { from: 'Plan', body: 'Go', task: true },
  );
  assert.equal(messageAttribution('[Reply from thread "x"] not a header'), null);
  // Today's daemon text (runtime/children.rs `wake_text_how`, runtime/delegation.rs `task_header`).
  assert.deepEqual(
    messageAttribution('[Reply from subagent "Fix the build" (codex, id 01X): its turn finished. Use it and carry on; send it follow-up work with otto_thread_message only if more is needed.]\n\nAll green.'),
    { from: 'Fix the build', body: 'All green.', reply: true },
  );
  assert.deepEqual(
    messageAttribution('[Subagent task from thread "Plan" (claude, id 01X). Do the work yourself; start no subagents unless this task says to. When your turn ends your reply goes back to that thread on its own — do not message it.]\n\nGo'),
    { from: 'Plan', body: 'Go', task: true },
  );
  // The queue bar reads the same way.
  assert.equal(queuedLabel(wake), 'Queued reply from Fix the build: All green [3 tests].');
  assert.equal(queuedLabel('plain words'), 'Queued: plain words');
});

test('token usage reads in words, per turn and per thread', () => {
  assert.equal(turnTokens({ input_tokens: 12_000, output_tokens: 400 }), 12_400);
  assert.equal(turnTokens({ input_tokens: 0, output_tokens: 0 }), null);
  assert.equal(turnTokens(undefined), null);
  assert.equal(formatTokens(840), '840 tokens');
  assert.equal(formatTokens(1), '1 token');
  assert.equal(formatTokens(12_400), '12.4k tokens');
  assert.equal(formatTokens(12_000), '12k tokens');
  assert.equal(formatTokens(250_400), '250k tokens');
  assert.equal(formatTokens(1_240_000), '1.2M tokens');
  assert.equal(formatTokens(0), '');
  assert.equal(turnUsageText({ status: 'running', turn_token_usage: { input_tokens: 5, output_tokens: 5 } }), '');
  assert.equal(turnUsageText({ status: 'completed', turn_token_usage: { input_tokens: 1500, output_tokens: 0 } }), '1.5k tokens');
  assert.equal(turnUsageText(null), '');
  assert.equal(threadTokenTotal([{ turn_token_usage: { input_tokens: 100, output_tokens: 50 } }, {}, { turn_token_usage: { input_tokens: 1, output_tokens: 1 } }]), 152);
});
