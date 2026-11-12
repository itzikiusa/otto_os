import { test } from 'node:test';
import assert from 'node:assert/strict';
import { triggerStatusLine } from '../src/modules/workflows/triggerStatus.ts';

const ago = (iso: string) => (iso === '2026-11-12T08:00:00Z' ? '2h ago' : '?');

test('a trigger that never fired says so', () => {
  assert.deepEqual(triggerStatusLine({}, ago), { text: 'Never fired', tone: 'dim' });
  assert.deepEqual(triggerStatusLine({ last_fired_at: null, last_outcome: null }, ago), { text: 'Never fired', tone: 'dim' });
});

test('a fired trigger says when and what it came to', () => {
  assert.deepEqual(triggerStatusLine({ last_fired_at: '2026-11-12T08:00:00Z', last_outcome: 'started' }, ago),
    { text: 'Last fired 2h ago · started', tone: 'ok' });
  assert.deepEqual(triggerStatusLine({ last_fired_at: '2026-11-12T08:00:00Z', last_outcome: 'queued' }, ago),
    { text: 'Last fired 2h ago · queued', tone: 'ok' });
  assert.equal(triggerStatusLine({ last_fired_at: '2026-11-12T08:00:00Z', last_outcome: 'started' }, () => '').text,
    'Last fired just now · started');
});

test('a refused attempt shows the reason', () => {
  assert.deepEqual(
    triggerStatusLine({ last_fired_at: '2026-11-12T08:00:00Z', last_outcome: 'rejected', last_detail: 'previous run of this workflow still active' }, ago),
    { text: 'Last attempt rejected: previous run of this workflow still active', tone: 'warn' },
  );
  assert.equal(triggerStatusLine({ last_fired_at: 'x', last_outcome: 'rejected', last_detail: ' ' }, ago).text, 'Last attempt rejected');
});
