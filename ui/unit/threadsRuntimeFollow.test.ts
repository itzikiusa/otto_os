// The runtime timeline's "new messages" pill and the queued-message Edit
// that must never overwrite what the person is typing.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { followPillText, unreadSince } from '../src/modules/threads/runtime/follow.ts';
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

const { mergeQueuedEdit } = loadSource(new URL('../src/modules/threads/runtime/runtimeComposer.ts', import.meta.url), {
  '../../../lib/toastError': { toastError: () => {} },
  './hostCommands': { runHostSlash: () => false },
  '../../../lib/stores/runtimeDrafts.svelte': { runtimeDrafts: {} },
});

test('Edit on a queued message keeps the unsent draft and adds the message below it', () => {
  assert.equal(mergeQueuedEdit('', 'queued one'), 'queued one');
  assert.equal(mergeQueuedEdit('  \n', 'queued one'), 'queued one');
  assert.equal(mergeQueuedEdit('half a thought', 'queued one'), 'half a thought\nqueued one');
  assert.equal(mergeQueuedEdit('draft\n\n', 'queued one'), 'draft\nqueued one');
});
