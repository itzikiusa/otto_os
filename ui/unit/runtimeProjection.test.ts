// The structured-runtime projection (modules/threads/runtime/projection.ts):
// whole-entity upserts by id, sequence dedupe + gap detection, replay
// (events and snapshot), the approval lifecycle and interrupt.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { applyEvents, applyReplay, emptyProjection, type RuntimeProjection } from '../src/modules/threads/runtime/projection.ts';
import { isRunning, pendingApprovals, timelineRows, workLabel } from '../src/modules/threads/runtime/timeline.ts';
import type { RuntimeEvent, RuntimeEventBody, RuntimeRequest, TurnItem } from '../src/lib/api/types';

const TS = '2026-11-04T10:00:00Z';
const ev = (sequence: number, body: RuntimeEventBody): RuntimeEvent => ({ session_id: 's1', sequence, ts: TS, ...body }) as RuntimeEvent;

const base = { turn_id: 't1', status: 'running', title: null, started_at: TS, completed_at: null, updated_at: TS } as const;
const assistant = (text: string, streaming: boolean, ordinal = 2): TurnItem => ({
  ...base,
  id: 'a1',
  ordinal,
  type: 'assistant_message',
  message_id: 'm1',
  text,
  streaming,
});
const user = (ordinal = 1): TurnItem => ({ ...base, id: 'u1', ordinal, status: 'completed', type: 'user_message', text: 'hi' });
const turn = (sequence: number, status: 'running' | 'completed' | 'interrupted') =>
  ev(sequence, { type: 'provider_turn.updated', provider_turn: { id: 't1', ordinal: 1, status, started_at: TS, completed_at: null } });

function fold(events: RuntimeEvent[], from: RuntimeProjection = emptyProjection()): RuntimeProjection {
  const { projection, gap } = applyEvents(from, events);
  assert.equal(gap, false);
  return projection;
}

test('streaming upserts replace the item by id; duplicates are skipped', () => {
  let p = fold([
    ev(1, { type: 'turn_item.updated', turn_item: user() }),
    ev(2, { type: 'turn_item.updated', turn_item: assistant('Hel', true) }),
    ev(3, { type: 'turn_item.updated', turn_item: assistant('Hello', true) }),
  ]);
  assert.deepEqual(p.items.map((i) => i.id), ['u1', 'a1']);
  // A replayed duplicate (sequence 2) must not roll the text back.
  p = fold([ev(2, { type: 'turn_item.updated', turn_item: assistant('Hel', true) })], p);
  const a = p.items[1];
  assert.equal(a.type === 'assistant_message' && a.text, 'Hello');
  assert.equal(p.lastSequence, 3);
});

test('items keep ordinal order even when a later ordinal arrives first', () => {
  const p = fold([
    ev(1, { type: 'turn_item.updated', turn_item: assistant('x', false, 5) }),
    ev(2, { type: 'turn_item.updated', turn_item: user(1) }),
  ]);
  assert.deepEqual(p.items.map((i) => i.ordinal), [1, 5]);
});

test('a batch is folded in sequence order; a jump reports a gap and stops', () => {
  const p = fold([
    ev(2, { type: 'turn_item.updated', turn_item: assistant('Hello', true) }),
    ev(1, { type: 'turn_item.updated', turn_item: assistant('Hel', true) }),
  ]);
  const a = p.items[0];
  assert.equal(a.type === 'assistant_message' && a.text, 'Hello');
  const { projection, gap } = applyEvents(p, [ev(5, { type: 'turn_item.updated', turn_item: user() })]);
  assert.equal(gap, true);
  assert.equal(projection.lastSequence, 2, 'nothing past the gap is applied');
  assert.equal(projection.items.length, 1);
});

test('replay events fill the gap and may skip ahead (restart floor)', () => {
  const p = fold([ev(1, { type: 'turn_item.updated', turn_item: user() })]);
  const next = applyReplay(p, {
    kind: 'events',
    events: [ev(1, { type: 'turn_item.updated', turn_item: user() }), ev(70_000, { type: 'turn_item.updated', turn_item: assistant('ok', false) })],
    last_sequence: 70_000,
  });
  assert.equal(next.lastSequence, 70_000);
  assert.deepEqual(next.items.map((i) => i.id), ['u1', 'a1']);
  // Live folding continues contiguously after the replay.
  assert.equal(applyEvents(next, [turn(70_001, 'completed')]).gap, false);
});

test('a snapshot replaces the projection unless it is older than what is applied', () => {
  const p = fold([ev(1, { type: 'turn_item.updated', turn_item: user() }), turn(2, 'running')]);
  const snapshot = {
    provider_session: null,
    provider_thread: null,
    turns: [],
    turn_items: [assistant('done', false, 2), user(1)],
    runtime_requests: [],
    plans: [],
  };
  const older = applyReplay(p, { kind: 'snapshot', snapshot_sequence: 1, snapshot });
  assert.equal(older, p);
  const fresh = applyReplay(p, { kind: 'snapshot', snapshot_sequence: 9, snapshot });
  assert.equal(fresh.lastSequence, 9);
  assert.deepEqual(fresh.items.map((i) => i.id), ['u1', 'a1'], 'sorted by ordinal');
  assert.equal(isRunning(fresh), false);
});

test('approval lifecycle: pending pairs with its item, resolves away, not_resumable cannot answer', () => {
  const request = (status: 'pending' | 'resolved', live = true): RuntimeRequest => ({
      id: 'r1',
      turn_id: 't1',
      kind: 'command',
      status,
      response_capability: live ? { type: 'live' } : { type: 'not_resumable', reason: 'daemon restarted' },
      created_at: TS,
      resolved_at: null,
    });
  const item: TurnItem = {
    ...base,
    id: 'ap1',
    ordinal: 3,
    status: 'waiting',
    type: 'approval_request',
    request_id: 'r1',
    request_kind: 'command',
    prompt: 'rm -rf build',
    options: [
      { decision: 'accept', label: 'Approve' },
      { decision: 'accept_for_session', label: 'Always allow this session' },
      { decision: 'decline', label: 'Decline' },
    ],
  };
  let p = fold([
    ev(1, { type: 'turn_item.updated', turn_item: item }),
    ev(2, { type: 'runtime_request.updated', runtime_request: request('pending') }),
  ]);
  const [a] = pendingApprovals(p);
  assert.equal(a.requestId, 'r1');
  assert.equal(a.prompt, 'rm -rf build');
  assert.equal(a.options.length, 3);
  assert.equal(a.canRespond, true);
  assert.equal(pendingApprovals(fold([ev(3, { type: 'runtime_request.updated', runtime_request: { ...request('pending', false) } })], p))[0].canRespond, false);
  p = fold([ev(3, { type: 'runtime_request.updated', runtime_request: { ...request('resolved'), decision: 'accept' } })], p);
  assert.deepEqual(pendingApprovals(p), []);
});

test('interrupt: running until the terminal event; command output accumulates', () => {
  let p = fold([
    turn(1, 'running'),
    ev(2, { type: 'turn_item.updated', turn_item: { ...base, id: 'c1', ordinal: 2, type: 'command_execution', input: 'make' } }),
    ev(3, { type: 'command_output.delta', item_id: 'c1', chunk: 'a\n' }),
    ev(4, { type: 'command_output.delta', item_id: 'c1', chunk: 'b\n' }),
  ]);
  assert.equal(isRunning(p), true);
  assert.equal(p.output.c1, 'a\nb\n');
  p = fold([ev(5, { type: 'turn.terminal', turn_id: 't1', status: 'interrupted' })], p);
  assert.equal(isRunning(p), false);
  assert.equal(p.turns[0].status, 'interrupted');
});

test('timeline groups consecutive work items between messages', () => {
  const work = (id: string, ordinal: number): TurnItem => ({ ...base, id, ordinal, type: 'reasoning', text: 'hmm', streaming: false });
  const rows = timelineRows([user(1), work('w1', 2), work('w2', 3), assistant('ok', false, 4)]);
  assert.deepEqual(
    rows.map((r) => (r.kind === 'work' ? r.items.map((i) => i.id).join('+') : r.item.id)),
    ['u1', 'w1+w2', 'a1'],
  );
  assert.equal(workLabel(work('w', 1)), 'Thinking');
});
