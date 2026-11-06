// Background work, the agent's own turns, subagents and the queue on the
// structured runtime (modules/threads/runtime/projection.ts + timeline.ts).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { applyEvents, emptyProjection, type RuntimeProjection } from '../src/modules/threads/runtime/projection.ts';
import { dayLabel, isRunning, latestTurn, messageAttribution, subagentChildren, timelineRows, turnOutcome, workLabel } from '../src/modules/threads/runtime/timeline.ts';
import type { ProviderTurn, RuntimeEvent, RuntimeEventBody, TurnItem } from '../src/lib/api/types';

const TS = '2026-11-05T10:00:00Z';
const ev = (sequence: number, body: RuntimeEventBody): RuntimeEvent => ({ session_id: 's1', sequence, ts: TS, ...body }) as RuntimeEvent;
const base = { status: 'completed', title: null, started_at: TS, completed_at: TS, updated_at: TS } as const;
const it = (id: string, ordinal: number, turn: string, kind: Record<string, unknown>, extra: Record<string, unknown> = {}): TurnItem =>
  ({ ...base, id, ordinal, turn_id: turn, ...kind, ...extra }) as TurnItem;
const turnEv = (seq: number, t: Partial<ProviderTurn> & { id: string; ordinal: number }): RuntimeEvent =>
  ev(seq, { type: 'provider_turn.updated', provider_turn: { status: 'completed', started_at: TS, completed_at: TS, ...t } as ProviderTurn });

function fold(events: RuntimeEvent[]): RuntimeProjection {
  const { projection, gap } = applyEvents(emptyProjection(), events);
  assert.equal(gap, false);
  return projection;
}

test('the queue and the background list ride their events', () => {
  const p = fold([
    ev(1, {
      type: 'provider_session.updated',
      provider_session: {
        status: 'ready',
        cwd: '/w',
        model: null,
        updated_at: TS,
        last_error: null,
        background_tasks: [{ id: 'b1', kind: 'command', label: 'sleep 300', started_at: TS }],
      },
    }),
    ev(2, { type: 'queue.updated', queued_prompts: [{ id: 'q1', text: 'after', attachments: 0, created_at: TS }] }),
  ]);
  assert.equal(p.session?.background_tasks?.[0].label, 'sleep 300');
  assert.deepEqual(p.queued.map((q) => q.text), ['after']);
  const cleared = applyEvents(p, [ev(3, { type: 'queue.updated', queued_prompts: [] })]).projection;
  assert.deepEqual(cleared.queued, []);
});

test("the agent's own turn is the running turn (Stop, queueing)", () => {
  const p = fold([turnEv(1, { id: 't1', ordinal: 1 }), turnEv(2, { id: 't2', ordinal: 2, status: 'running', origin: 'agent' })]);
  assert.equal(latestTurn(p)?.origin, 'agent');
  assert.equal(isRunning(p), true);
});

test("a subagent's activity renders inside its card, not in the timeline", () => {
  const card = it('t1:tool:a', 2, 't1', { type: 'subagent', tool_name: 'Agent', description: 'PR sweep', background: true, tool_uses: 1 }, { status: 'running' });
  const items = [
    it('u', 1, 't1', { type: 'user_message', text: 'sweep' }),
    card,
    it('t1:sub:a:s1:0', 3, 't1', { type: 'assistant_message', message_id: 's1', text: 'Looking', streaming: false }, { parent_item_id: 't1:tool:a' }),
    it('t1:tool:r', 4, 't1', { type: 'dynamic_tool', tool_name: 'Read', input: {} }, { parent_item_id: 't1:tool:a' }),
    it('a', 5, 't1', { type: 'assistant_message', message_id: 'm', text: 'Waiting for the sweep', streaming: false }),
  ];
  const rows = timelineRows(items);
  const ids = rows.flatMap((r) => (r.kind === 'message' ? [r.item.id] : r.kind === 'day' ? [] : r.parts.flatMap((x) => (x.kind === 'item' ? [x.entry.item.id] : x.entries.map((e) => e.item.id)))));
  assert.deepEqual(ids, ['u', 't1:tool:a', 'a']);
  assert.deepEqual(subagentChildren(items).get('t1:tool:a')?.map((i) => i.id), ['t1:sub:a:s1:0', 't1:tool:r']);
  assert.equal(workLabel(card), 'PR sweep');
});

test('a finished turn: stopped is neutral, a failed step with an answer is partly done', () => {
  const stopped = fold([turnEv(1, { id: 't1', ordinal: 1, status: 'interrupted' })]);
  assert.deepEqual(turnOutcome(stopped, latestTurn(stopped)), { kind: 'stopped' });
  const stalled = fold([
    turnEv(1, { id: 't1', ordinal: 1, status: 'interrupted' }),
    ev(2, { type: 'turn_item.updated', turn_item: it('t1:stalled', 1, 't1', { type: 'system_notice', message: 'Stopped: …' }, { status: 'interrupted' }) }),
  ]);
  assert.deepEqual(turnOutcome(stalled, latestTurn(stalled)), { kind: 'stalled' });
  const items = [
    it('u', 1, 't1', { type: 'user_message', text: 'deploy it' }),
    it('c', 2, 't1', { type: 'command_execution', input: 'make deploy', error: 'boom' }, { status: 'failed' }),
    it('a', 3, 't1', { type: 'assistant_message', message_id: 'm', text: 'Built, but the deploy failed', streaming: false }),
  ];
  const partial = fold([turnEv(1, { id: 't1', ordinal: 1 }), ...items.map((i, n) => ev(n + 2, { type: 'turn_item.updated', turn_item: i }))]);
  const o = turnOutcome(partial, latestTurn(partial));
  assert.equal(o?.kind, 'partial');
  assert.equal(o?.kind === 'partial' && o.retryText, 'deploy it');
  assert.deepEqual(o?.kind === 'partial' && o.failed.map((f) => f.id), ['c']);
  // A clean turn, or a failure with no answer (the error item says it), is not "partly done".
  const clean = fold([turnEv(1, { id: 't1', ordinal: 1 }), ev(2, { type: 'turn_item.updated', turn_item: items[2] })]);
  assert.equal(turnOutcome(clean, latestTurn(clean)), null);
  const noAnswer = fold([turnEv(1, { id: 't1', ordinal: 1 }), ev(2, { type: 'turn_item.updated', turn_item: items[1] })]);
  assert.equal(turnOutcome(noAnswer, latestTurn(noAnswer)), null);
});

test('day separators appear only when a thread spans days; framed messages are attributed', () => {
  const at = (id: string, iso: string, type: 'user_message' | 'assistant_message' = 'user_message') =>
    ({ id, turn_id: 't', ordinal: 0, status: 'completed', title: null, started_at: iso, completed_at: iso, updated_at: iso, type, text: id }) as TurnItem;
  const one = timelineRows([at('a', '2026-11-05T10:00:00'), at('b', '2026-11-05T11:00:00', 'assistant_message')]);
  assert.equal(one.filter((r) => r.kind === 'day').length, 0);
  const two = timelineRows([at('a', '2026-11-04T10:00:00'), at('b', '2026-11-05T11:00:00')]);
  assert.deepEqual(two.map((r) => r.kind), ['day', 'message', 'day', 'message']);
  const now = new Date('2026-11-05T12:00:00');
  assert.equal(dayLabel(Date.parse('2026-11-05T09:00:00'), now), 'Today');
  assert.equal(dayLabel(Date.parse('2026-11-04T09:00:00'), now), 'Yesterday');
  assert.match(dayLabel(Date.parse('2025-12-29T09:00:00'), now), /2025/);
  assert.deepEqual(messageAttribution('[Message from Scheduled · Brief (t-1)]: Two builds failed'), { from: 'Scheduled · Brief', body: 'Two builds failed' });
  assert.deepEqual(messageAttribution('[Message from Scout]: done'), { from: 'Scout', body: 'done' });
  assert.deepEqual(
    messageAttribution('[Message from thread "Fix CI" (claude, id 01X). Answer it with the otto_thread_message tool (session_id 01X); see every linked thread with otto_thread_list.]\n\nPlease rebase'),
    { from: 'Fix CI', body: 'Please rebase' },
  );
  assert.equal(messageAttribution('just my own [note]: hi'), null);
});
