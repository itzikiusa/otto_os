import { test } from 'node:test';
import assert from 'node:assert/strict';
import { PREFETCH_CAP, isLoopbackHost, prefetchQueue, shouldPrefetch } from '../src/lib/prefetchPlan.ts';

const KNOWN = new Set(['agents', 'home', 'git', 'database', 'vault', 'api', 'kubernetes', 'aws', 'usage', 'settings', 'plugin']);
const keyOf = (id: string): string | null => {
  const m = id.split('/')[0];
  if (m === 'connections') return 'database';
  return KNOWN.has(m) ? m : null;
};

test('favorites first, then the visible order, mapped + deduped + capped', () => {
  const q = prefetchQueue(['agents', 'home', 'git', 'connections', 'vault', 'api', 'kubernetes', 'aws'], ['aws', 'hidden-one'], keyOf, {
    skip: ['agents'],
  });
  assert.deepEqual(q, ['aws', 'home', 'git', 'database', 'vault', 'api']);
  assert.equal(q.length, PREFETCH_CAP);
});

test('modules the user cannot see (RBAC / hidden) are never warmed', () => {
  const q = prefetchQueue(['home', 'git'], ['usage'], keyOf, { cap: 10 });
  assert.deepEqual(q, ['home', 'git', 'settings']);
});

test('plugin routes and unknown ids are skipped', () => {
  assert.deepEqual(prefetchQueue(['plugin/x', 'nope', 'home'], [], keyOf, { cap: 2 }), ['home', 'settings']);
});

test('only a local daemon on a non-phone without Save-Data prefetches', () => {
  const href = 'tauri://localhost/index.html';
  assert.equal(shouldPrefetch({ isPhone: false, base: 'http://127.0.0.1:7700', href }), true);
  assert.equal(shouldPrefetch({ isPhone: true, base: 'http://127.0.0.1:7700', href }), false);
  assert.equal(shouldPrefetch({ isPhone: false, saveData: true, base: 'http://127.0.0.1:7700', href }), false);
  assert.equal(shouldPrefetch({ isPhone: false, base: '', href: 'https://otto.example.ts.net/' }), false);
  assert.equal(shouldPrefetch({ isPhone: false, base: '', href: 'http://localhost:5173/' }), true);
  assert.ok(isLoopbackHost('[::1]'));
  assert.ok(!isLoopbackHost('192.168.1.4'));
});
