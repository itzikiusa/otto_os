import assert from 'node:assert/strict';
import { register } from 'node:module';
import { test } from 'node:test';
import type { TurnItem } from '../src/lib/api/types';

const hook =
  'export async function resolve(s, c, n) {' +
  "  if (/^\\.{1,2}\\//.test(s) && !/\\.[a-z]+$/i.test(s)) { try { return await n(s + '.ts', c); } catch {} }" +
  '  return n(s, c);' +
  '}';
register('data:text/javascript,' + encodeURIComponent(hook));

const { formatDuration, formatMessageClock, formatMessageTooltip } = await import(
  '../src/modules/threads/runtime/timeline.ts'
);
const { formatDuration: workLogFormatDuration, itemDuration } = await import(
  '../src/modules/threads/runtime/workLog.ts'
);
const { parseUserMessageItemWithBadges } = await import(
  '../src/modules/threads/runtime/mentionMatcher.ts'
);

test('formatMessageClock formats valid ISO timestamps and returns empty string for null or invalid values', () => {
  assert.equal(formatMessageClock(null), '');
  assert.equal(formatMessageClock(undefined), '');
  assert.equal(formatMessageClock(''), '');
  assert.equal(formatMessageClock('not-a-timestamp'), '');

  const clock = formatMessageClock('2026-11-14T12:34:56.000Z');
  assert.ok(clock.length >= 4, `expected formatted local clock, got "${clock}"`);
  assert.match(clock, /\d{1,2}:\d{2}/);
});

test('formatMessageTooltip formats sent user messages, streaming replies, and settled assistant turns', () => {
  assert.equal(formatMessageTooltip(null), '');
  assert.equal(formatMessageTooltip('invalid-iso'), '');

  const userTip = formatMessageTooltip('2026-11-14T10:05:09.000Z');
  assert.match(userTip, /^Sent \d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}$/);

  const streamingTip = formatMessageTooltip('2026-11-14T10:05:09.000Z', null, true);
  assert.match(streamingTip, /^Streaming since \d{2}:\d{2}:\d{2}$/);
  assert.equal(formatMessageTooltip(null, null, true), '');

  const settledWithSpan = formatMessageTooltip(
    '2026-11-14T10:05:09.000Z',
    '2026-11-14T10:05:23.400Z',
    false,
  );
  assert.match(
    settledWithSpan,
    /^Started \d{2}:\d{2}:\d{2} · Completed \d{2}:\d{2}:\d{2} \(14s\) · \d{4}-\d{2}-\d{2}$/,
  );

  // Sub-second assistant replies omit `(0s)` while keeping exact seconds in the tooltip.
  const settledSubSec = formatMessageTooltip(
    '2026-11-14T10:05:09.100Z',
    '2026-11-14T10:05:09.600Z',
    false,
  );
  assert.match(
    settledSubSec,
    /^Started \d{2}:\d{2}:\d{2} · Completed \d{2}:\d{2}:\d{2} · \d{4}-\d{2}-\d{2}$/,
  );

  // Settled reply when only completedAt is available.
  const endOnly = formatMessageTooltip(null, '2026-11-14T10:05:23.000Z', false);
  assert.match(endOnly, /^Completed \d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}$/);
});

test('itemDuration suppresses misleading 0ms on legacy persisted reasoning rows while preserving measurable durations', () => {
  const makeReasoning = (
    status: TurnItem['status'],
    started_at: string | null,
    completed_at: string | null,
  ): TurnItem => ({
    id: 'r-1',
    turn_id: 't-1',
    ordinal: 1,
    title: null,
    type: 'reasoning',
    status,
    text: 'Thinking',
    streaming: status === 'running',
    started_at,
    completed_at,
    updated_at: completed_at ?? started_at ?? '2026-11-14T10:00:00.000Z',
  });

  // Legacy persisted reasoning items where started_at === completed_at (0ms) or <50ms return null.
  assert.equal(
    itemDuration(makeReasoning('completed', '2026-11-14T10:00:00.000Z', '2026-11-14T10:00:00.000Z')),
    null,
  );
  assert.equal(
    itemDuration(makeReasoning('completed', '2026-11-14T10:00:00.000Z', '2026-11-14T10:00:00.020Z')),
    null,
  );

  // Measurable reasoning durations (>=50ms) render cleanly.
  assert.equal(
    itemDuration(makeReasoning('completed', '2026-11-14T10:00:00.000Z', '2026-11-14T10:00:04.250Z')),
    '4s',
  );
  assert.equal(
    itemDuration(makeReasoning('completed', '2026-11-14T10:00:00.000Z', '2026-11-14T10:00:00.320Z')),
    '320ms',
  );

  // Live reasoning rows return null from itemDuration (RuntimeWorkRow ticks live elapsed separately).
  assert.equal(
    itemDuration(makeReasoning('running', '2026-11-14T10:00:00.000Z', '2026-11-14T10:00:05.000Z')),
    null,
  );

  // Non-reasoning tool rows continue formatting sub-50ms execution times.
  const fastCmd: TurnItem = {
    id: 'c-1',
    turn_id: 't-1',
    ordinal: 2,
    title: null,
    type: 'command_execution',
    status: 'completed',
    input: 'pwd',
    exit_code: 0,
    error: undefined,
    started_at: '2026-11-14T10:00:00.000Z',
    completed_at: '2026-11-14T10:00:00.012Z',
    updated_at: '2026-11-14T10:00:00.012Z',
  };
  assert.equal(itemDuration(fastCmd), '12ms');
});

test('formatDuration is shared between timeline and workLog and parseUserMessageItemWithBadges memoizes settled TurnItems', () => {
  assert.equal(formatDuration, workLogFormatDuration);
  assert.equal(formatDuration(320), '320ms');
  assert.equal(formatDuration(14_200), '14s');
  assert.equal(formatDuration(3_600_000), '1h 0m');

  const settledUserItem = {
    id: 'u-1',
    text: '<ide_context>\n<file path="cmd/main.go" lines="1-20" label="@main.go">package main</file>\n</ide_context>\n\nRefactor entrypoint',
  };
  const first = parseUserMessageItemWithBadges(settledUserItem);
  const second = parseUserMessageItemWithBadges(settledUserItem);
  assert.equal(first, second, 'settled TurnItem reference should hit WeakMap cache');
  assert.equal(first.prose, 'Refactor entrypoint');
  assert.equal(first.badges.length, 1);
  assert.equal(first.badges[0].label, '@main.go');
});
