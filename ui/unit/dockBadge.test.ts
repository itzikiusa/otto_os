// Dock badge writer (lib/dockBadge.ts): debounced, change-only, flushable.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { badgeWriter } from '../src/lib/dockBadge.ts';

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

test('bursts collapse into one write of the final count', async () => {
  const writes: number[] = [];
  const w = badgeWriter((n) => void writes.push(n), 20);
  w.set(1);
  w.set(2);
  w.set(3);
  await wait(40);
  assert.deepEqual(writes, [3]);
});

test('an unchanged count is not re-sent; flush writes at once', async () => {
  const writes: number[] = [];
  const w = badgeWriter((n) => void writes.push(n), 20);
  w.set(2);
  await wait(40);
  w.set(2);
  await wait(40);
  assert.deepEqual(writes, [2]);
  w.set(5);
  w.flush(0); // teardown wins over the pending 5
  await wait(40);
  assert.deepEqual(writes, [2, 0]);
});
