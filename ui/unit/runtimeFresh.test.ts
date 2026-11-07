import assert from 'node:assert/strict';
import { test } from 'node:test';
import { applyEvents, applyReplay, emptyProjection } from '../src/modules/threads/runtime/projection.ts';
import { timelineRows } from '../src/modules/threads/runtime/timeline.ts';
import type { RuntimeEvent, TurnItem } from '../src/lib/api/types';

const ts = '2026-11-07T12:00:00Z';
const base = { title: null, status: 'completed', started_at: ts, completed_at: ts, updated_at: ts } as const;
const items: TurnItem[] = [
  { ...base, id: 'old', turn_id: 'old-turn', ordinal: 1, type: 'user_message', text: 'Earlier question' },
  { ...base, id: 'boundary', turn_id: 'new-turn', ordinal: 2, type: 'session_boundary' },
  { ...base, id: 'new', turn_id: 'new-turn', ordinal: 3, type: 'user_message', text: 'Fresh question' },
];
const events = items.map((turn_item, n): RuntimeEvent => ({
  session_id: 'same-visible-thread', sequence: n + 1, ts, type: 'turn_item.updated', turn_item,
}));

test('a committed fresh session divides retained history instead of becoming tool work', () => {
  const { projection } = applyEvents(emptyProjection(), events);
  const rows = timelineRows(projection.items);
  assert.deepEqual(rows.map((r) => r.kind), ['message', 'session', 'message']);
  assert.deepEqual(projection.items.map((i) => i.id), ['old', 'boundary', 'new']);
});

test('replaying a session boundary cannot duplicate it or drop the earlier messages', () => {
  const { projection } = applyEvents(emptyProjection(), events);
  const recovered = applyReplay(projection, { kind: 'events', events, last_sequence: 3 });
  assert.deepEqual(timelineRows(recovered.items).map((r) => r.kind), ['message', 'session', 'message']);
});

test('arming or a failed fresh send without a boundary event does not invent a divider', () => {
  const { projection } = applyEvents(emptyProjection(), events.slice(0, 1));
  assert.deepEqual(timelineRows(projection.items).map((r) => r.kind), ['message']);
});
