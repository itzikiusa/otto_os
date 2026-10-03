import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  ACTIVITY_ITEM_CAP,
  activityEpochChanged,
  approvalConcernsFeed,
  mergeActivity,
} from '../src/modules/personal-agents/activityFeed.ts';

const item = (seq: number) => ({
  seq,
  at: new Date(1_700_000_000_000 + seq * 1000).toISOString(),
  kind: 'tool_call' as const,
  tool: `t${seq}`,
  detail: '',
  session_id: null,
  approval_id: null,
});
const run = (id: string) => ({ id, status: 'success', started_at: '2026-10-01T00:00:00Z' }) as never;
const base = (over: Record<string, unknown> = {}) =>
  ({
    now: { run: null, session_status: null },
    items: [],
    approvals: [],
    runs: [],
    seq: 0,
    ...over,
  }) as never;

test('an incremental answer prepends only new items and keeps the runs it did not carry', () => {
  const first = mergeActivity(null, base({ items: [item(2), item(1)], runs: [run('r1')], seq: 2 }));
  const next = mergeActivity(first, base({ items: [item(3), item(2)], runs: null, seq: 3 }));
  assert.deepEqual(
    next.items.map((i) => i.seq),
    [3, 2, 1],
  );
  assert.equal(next.runs?.length, 1, 'runs kept when the answer omits them');
  assert.equal(next.seq, 3);
  const withRuns = mergeActivity(next, base({ runs: [run('r2'), run('r1')], seq: 3 }));
  assert.equal(withRuns.runs?.length, 2, 'runs replaced when carried');
});

test('the merged ring is capped like the daemon ring', () => {
  let state = mergeActivity(null, base());
  for (let s = 1; s <= ACTIVITY_ITEM_CAP + 20; s++) {
    state = mergeActivity(state, base({ items: [item(s)], runs: null, seq: s }));
  }
  assert.equal(state.items.length, ACTIVITY_ITEM_CAP);
  assert.equal(state.items[0].seq, ACTIVITY_ITEM_CAP + 20);
});

test('only approval events for approvals the feed shows trigger a refresh', () => {
  const data = base({ approvals: [{ approval_id: 'a1', status: 'pending' }] });
  assert.equal(approvalConcernsFeed(data, 'a1'), true);
  assert.equal(approvalConcernsFeed(data, 'other'), false);
  assert.equal(approvalConcernsFeed(data, undefined), false);
  assert.equal(approvalConcernsFeed(null, 'a1'), false);
});

test('a daemon restart (new epoch) resets the cursor and items instead of freezing the feed', () => {
  const before = mergeActivity(
    null,
    base({ items: [item(41), item(40)], runs: [run('r1')], seq: 41, epoch: 'boot-a' }),
  );
  // The new process restarted its counter at 1: under max() the cursor
  // would stay at 41 and every later answer (seq 1, 2, …) would be dropped.
  const after = mergeActivity(before, base({ items: [item(2), item(1)], runs: null, seq: 2, epoch: 'boot-b' }));
  assert.deepEqual(
    after.items.map((i) => i.seq),
    [2, 1],
    'old-process items dropped, new ones shown',
  );
  assert.equal(after.seq, 2, 'cursor follows the new process');
  assert.equal(after.epoch, 'boot-b');
  assert.equal(after.runs?.length, 1, 'durable run history kept when omitted');
  const next = mergeActivity(after, base({ items: [item(3)], runs: null, seq: 3, epoch: 'boot-b' }));
  assert.deepEqual(
    next.items.map((i) => i.seq),
    [3, 2, 1],
  );
});

test('the daemon-side reset flag is honoured even without an epoch to compare', () => {
  const before = mergeActivity(null, base({ items: [item(9)], seq: 9 }));
  assert.equal(activityEpochChanged(before, base({ seq: 0 })), false);
  const after = mergeActivity(before, base({ items: [item(1)], runs: null, seq: 1, reset: true }));
  assert.deepEqual(
    after.items.map((i) => i.seq),
    [1],
  );
  assert.equal(after.seq, 1);
});
