import { test } from 'node:test';
import assert from 'node:assert/strict';
import { lazyModule } from '../src/lib/lazyModule.ts';

test('lazyModule: loads once, queues calls in order, then runs synchronously', async () => {
  let loads = 0;
  const seen: string[] = [];
  const m = lazyModule(async () => {
    loads += 1;
    return { push: (s: string) => seen.push(s) };
  });
  assert.equal(m.peek(), null);
  m.use((x) => x.push('a'));
  m.use((x) => x.push('b'));
  assert.equal(seen.length, 0);
  await new Promise((r) => setTimeout(r, 0));
  assert.deepEqual(seen, ['a', 'b']);
  m.use((x) => x.push('c')); // loaded: synchronous
  assert.deepEqual(seen, ['a', 'b', 'c']);
  assert.equal(loads, 1);
  assert.ok(m.peek());
});

test('lazyModule: a failed import drops its calls and retries on the next use', async () => {
  let attempt = 0;
  const seen: number[] = [];
  const m = lazyModule(async () => {
    attempt += 1;
    if (attempt === 1) throw new Error('offline');
    return { n: attempt };
  });
  m.use((x) => seen.push(x.n));
  await new Promise((r) => setTimeout(r, 0));
  assert.equal(seen.length, 0);
  m.use((x) => seen.push(x.n));
  await new Promise((r) => setTimeout(r, 0));
  assert.deepEqual(seen, [2]);
});
