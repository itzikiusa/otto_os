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

test('a failed chunk load is retried; another identity is not "settled" until its own read answers', async () => {
  let token = 't1';
  let navFails = 1;
  const reads: ReturnType<typeof deferred<void>>[] = [];
  const runtimeThreads = { mount() {}, loadAll: () => { const d = deferred<void>(); reads.push(d); return d.promise; } };
  const imports: Record<string, unknown> = {
    '../../lib/api/client': { getToken: () => token },
    '../threads/runtime/runtimeThreads.svelte': { runtimeThreads },
    '../threads/threadInbox.svelte': { inboxLive: { mount() {} } },
  };
  Object.defineProperty(imports, '../threads/threadNav.svelte', { enumerable: true, get: () => { if (navFails-- > 0) throw new Error('chunk failed'); return {}; } });
  const { startThreadFeeds, threadFeeds } = loadSource(new URL('../src/modules/agents/threadFeeds.svelte.ts', import.meta.url), imports);
  const settle = () => new Promise((r) => setImmediate(r));
  await assert.rejects(startThreadFeeds(), /chunk failed/);
  await startThreadFeeds(); // the next caller loads the chunk again
  reads[0].resolve();
  await settle();
  assert.equal(threadFeeds.settled, true);
  token = 't2';
  await startThreadFeeds();
  assert.equal(threadFeeds.settled, false, 't2 has not been answered yet');
  reads[1].resolve();
  await settle();
  assert.equal(threadFeeds.settled, true);
});

test('an identity change while the list is read reads for the new identity; the old answer settles nothing (R3-006)', async () => {
  let token = 't1';
  const reads: ReturnType<typeof deferred<void>>[] = [];
  const runtimeThreads = { mount() {}, loadAll: () => { const d = deferred<void>(); reads.push(d); return d.promise; } };
  const { startThreadFeeds, threadFeeds } = loadSource(new URL('../src/modules/agents/threadFeeds.svelte.ts', import.meta.url), {
    '../../lib/api/client': { getToken: () => token },
    '../threads/runtime/runtimeThreads.svelte': { runtimeThreads },
    '../threads/threadNav.svelte': {},
    '../threads/threadInbox.svelte': { inboxLive: { mount() {} } },
  });
  const settle = () => new Promise((r) => setImmediate(r));
  await startThreadFeeds();
  token = 't2';
  await startThreadFeeds(); // t1's read is still in flight
  reads[0].resolve();
  await settle();
  assert.equal(reads.length, 2, 't2 is read once t1 answers');
  assert.equal(threadFeeds.settled, false, "t1's answer does not settle t2");
  reads[1].resolve();
  await settle();
  assert.equal(threadFeeds.settled, true);
});
