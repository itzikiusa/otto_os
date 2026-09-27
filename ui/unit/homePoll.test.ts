import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

const lib = loadSource(new URL('../src/lib/poll.ts', import.meta.url), {});
const live = loadSource(new URL('../src/lib/live.ts', import.meta.url), { './poll': lib });
const { poll } = loadSource(new URL('../src/modules/home/boxes/poll.ts', import.meta.url), {
  '../../../lib/poll': lib,
  '../../../lib/live': live,
});

test('now() during an in-flight fetch coalesces into one rerun, not a new chain', async () => {
  let calls = 0;
  const releases: (() => void)[] = [];
  const poller = poll(() => {
    calls += 1;
    return new Promise<boolean>((resolve) => { releases.push(() => resolve(true)); });
  }, 60_000);
  assert.equal(calls, 1);
  // Live events calling now() while the first fetch is still pending.
  poller.now(); poller.now(); poller.now();
  assert.equal(calls, 1, 'no overlapping fetch');
  releases[0]();
  await new Promise((r) => setTimeout(r, 0));
  assert.equal(calls, 2, 'exactly one queued rerun');
  releases[1]();
  await new Promise((r) => setTimeout(r, 0));
  assert.equal(calls, 2, 'then back on the single scheduled cadence');
  poller.stop();
});
