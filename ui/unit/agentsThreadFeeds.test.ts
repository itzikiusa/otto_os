// The Agents inbox's thread list (threadFeeds): read once per app load, but a
// failed read is retried by the next visit, and another identity reads again.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deferred, loadSource } from './sourceHarness.ts';

test('a failed thread list is read again on the next visit; a new identity reads again', async () => {
  let token = 't1';
  const reads: ReturnType<typeof deferred<void>>[] = [];
  let mounts = 0;
  const runtimeThreads = { mount: () => void mounts++, loadAll: () => { const d = deferred<void>(); reads.push(d); return d.promise; } };
  const { startThreadFeeds, threadFeeds } = loadSource(new URL('../src/modules/agents/threadFeeds.svelte.ts', import.meta.url), {
    '../../lib/api/client': { getToken: () => token },
    '../threads/runtime/runtimeThreads.svelte': { runtimeThreads },
    '../threads/threadNav.svelte': {},
    '../threads/threadInbox.svelte': { inboxLive: { mount: () => void mounts++ } },
  });
  const settle = () => new Promise((r) => setImmediate(r));
  await startThreadFeeds();
  await startThreadFeeds(); // a second caller while the read is in flight
  assert.equal(reads.length, 1);
  reads[0].reject(new Error('daemon starting'));
  await settle();
  assert.equal(threadFeeds.settled, true, 'the coach is not held back by a failed read');
  await startThreadFeeds(); // the revisit retries
  assert.equal(reads.length, 2);
  reads[1].resolve();
  await settle();
  await startThreadFeeds(); // listed: a revisit reads nothing
  assert.equal(reads.length, 2);
  token = 't2'; // signed in as someone else
  await startThreadFeeds();
  assert.equal(reads.length, 3);
  assert.equal(mounts, 2, 'the live feeds start once');
});
