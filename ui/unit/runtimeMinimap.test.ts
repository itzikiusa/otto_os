// The thread minimap's ticks and position (runtime/minimap.ts).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { currentTick, minimapTicks } from '../src/modules/threads/runtime/minimap.ts';

const it = (id: string, type: string, extra: Record<string, unknown> = {}) => ({ id, type, ordinal: 0, status: 'completed', title: null, started_at: null, completed_at: null, updated_at: '', ...extra }) as never;
const turn = (id: string, status: string) => ({ id, ordinal: 0, status, started_at: null, completed_at: null }) as never;

test('one tick per message the person typed, marked failed or approval', () => {
  const items = [
    it('u1', 'user_message', { text: '\n  Fix the login bug\nmore', turn_id: 't1' }),
    it('a1', 'assistant_message', { text: 'ok', turn_id: 't1' }),
    it('u2', 'user_message', { text: 'Run the tests', turn_id: 't2' }),
    it('r2', 'approval_request', { request_id: 'x', turn_id: 't2' }),
    it('u3', 'user_message', { text: '[Message from thread "Other" (codex, id x). Answer it with otto.reply]\n\nhi', turn_id: 't3' }),
    it('u4', 'user_message', { text: '', attachments: [{}], turn_id: 't4' }),
    it('u5', 'user_message', { text: 'sub', parent_item_id: 'p' }),
  ];
  assert.deepEqual(minimapTicks(items, [turn('t1', 'failed'), turn('t2', 'completed')]), [
    { id: 'u1', text: 'Fix the login bug', mark: 'failed' },
    { id: 'u2', text: 'Run the tests', mark: 'approval' },
    { id: 'u4', text: 'Attachment', mark: null },
  ]);
});

test('the current tick is the last message above the reading line', () => {
  assert.equal(currentTick([-500, -100, 50, 400], 600), 2);
  assert.equal(currentTick([10, 400], 600), 0);
  assert.equal(currentTick([NaN, -10, NaN], 600), 1);
  assert.equal(currentTick([-900, -400, 500], 600, true), 2, 'at the end: the last turn');
});
