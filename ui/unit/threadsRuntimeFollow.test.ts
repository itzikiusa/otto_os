// The runtime timeline's "new messages" pill and the queued-message Edit
// that must never overwrite what the person is typing.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { followAfterScroll, followPillText, renderWindow, timelineRowKey, unreadSince } from '../src/modules/threads/runtime/follow.ts';
import { loadSource } from './sourceHarness.ts';

test('unread counts messages after the last one seen at the bottom', () => {
  assert.equal(unreadSince(['a', 'b', 'c'], 'a'), 2);
  assert.equal(unreadSince(['a', 'b', 'c'], 'c'), 0);
  assert.equal(unreadSince(['a', 'b'], null), 0);
  assert.equal(unreadSince(['a', 'b'], 'gone'), 0);
});

test('the pill names new messages first, then activity, then the way back', () => {
  assert.equal(followPillText(1, 5, true), '1 new message');
  assert.equal(followPillText(3, 0, false), '3 new messages');
  assert.equal(followPillText(0, 2, false), 'New activity');
  assert.equal(followPillText(0, 0, true), 'Jump to latest');
  assert.equal(followPillText(0, 0, false), null);
});

test('only the reader lets go of the end; reaching it again re-arms following', () => {
  // A small drag up while streaming releases (it used to snap back inside the 40 px band).
  assert.equal(followAfterScroll(true, 20, true, true), false);
  assert.equal(followAfterScroll(true, 60, true, false), false);
  // Content shrinking / scroll anchoring moves scrollTop without the reader: still pinned.
  assert.equal(followAfterScroll(true, 600, false, true), true);
  // The reader's tap collapsed something at the end: clamped to the end, still pinned.
  assert.equal(followAfterScroll(true, 0, true, true), true);
  // Scrolled up stays scrolled up until the reader is back in the end band.
  assert.equal(followAfterScroll(false, 600, false, false), false);
  assert.equal(followAfterScroll(false, 10, true, false), true);
  assert.equal(followAfterScroll(false, 10, false, false), true);
});

const { mergeQueuedEdit } = loadSource(new URL('../src/modules/threads/runtime/runtimeComposer.ts', import.meta.url), {
  '../../../lib/toastError': { toastError: () => {} },
  '../../../lib/telemetry': { startMeasurement: () => null },
  './hostCommands': { runHostSlash: () => false },
  './timeline': { errorText: (s: string) => s },
  '../../../lib/stores/runtimeDrafts.svelte': { runtimeDrafts: {} },
});

test('Edit on a queued message keeps the unsent draft and adds the message below it', () => {
  assert.equal(mergeQueuedEdit('', 'queued one'), 'queued one');
  assert.equal(mergeQueuedEdit('  \n', 'queued one'), 'queued one');
  assert.equal(mergeQueuedEdit('half a thought', 'queued one'), 'half a thought\nqueued one');
  assert.equal(mergeQueuedEdit('draft\n\n', 'queued one'), 'draft\nqueued one');
});

test('renderWindow keeps newly prepended older rows anchored when holdFrom advances after loadMore', () => {
  const mk = (n: number) => ({ kind: 'message', item: { id: `m-${n}` } });
  const initial = Array.from({ length: 300 }, (_, i) => mk(i));
  // Initially capped to newest 200 (100..299); scrolled up locks holdFrom = m-100.
  const win1 = renderWindow(initial, 200, 'm-100', timelineRowKey);
  assert.equal(timelineRowKey(win1[0]), 'm-100');
  // User reaches top -> rowCap grows to 400 revealing m-0..m-299, and holdFrom advances to m-0.
  const win2 = renderWindow(initial, 400, 'm-0', timelineRowKey);
  assert.equal(timelineRowKey(win2[0]), 'm-0');
  // 150 more live rows arrive at the bottom (total 450 > cap 400): m-0 stays pinned at the top (0 rows dropped).
  const grown = [...initial, ...Array.from({ length: 150 }, (_, i) => mk(300 + i))];
  const win3 = renderWindow(grown, 400, 'm-0', timelineRowKey);
  assert.equal(timelineRowKey(win3[0]), 'm-0');
  assert.equal(win3.length, 450);
});
