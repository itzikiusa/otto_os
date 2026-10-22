// product/attachmentUrls.svelte.ts — the authed object-URL cache shared by the
// Overview body preview and AttachmentsPanel (architecture-5-04): concurrent
// loads share one fetch, a reset() revokes everything and frees a blob that
// lands afterwards (a story switch), and forget() revokes one URL.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deferred, loadSource } from './sourceHarness.ts';

function fixture() {
  const pending = new Map<string, ReturnType<typeof deferred<string>>>();
  const fetched: string[] = [];
  const revoked: string[] = [];
  const authedBlobUrl = (path: string) => {
    fetched.push(path);
    const d = deferred<string>();
    pending.set(path.split('/').at(-1)!, d);
    return d.promise;
  };
  const { AttachmentUrls } = loadSource(new URL('../src/modules/product/attachmentUrls.svelte.ts', import.meta.url), {
    '../../lib/api/client': { authedBlobUrl },
    '../../lib/loadError': { loadErrorText: (e: unknown) => String(e instanceof Error ? e.message : e) },
  }, { URL: { revokeObjectURL: (u: string) => revoked.push(u) } });
  return { atts: new AttachmentUrls(), pending, fetched, revoked };
}

test('overlapping loads of one id share a single fetch', async () => {
  const f = fixture();
  const a = f.atts.load('x1');
  const b = f.atts.load('x1');
  assert.equal(f.fetched.length, 1);
  assert.equal(f.atts.loading.x1, true);
  f.pending.get('x1')!.resolve('blob:1');
  assert.equal(await a, 'blob:1');
  assert.equal(await b, 'blob:1');
  assert.equal(f.atts.urls.x1, 'blob:1');
  assert.equal(f.atts.loading.x1, false);
  // Cached: no second fetch.
  assert.equal(await f.atts.load('x1'), 'blob:1');
  assert.equal(f.fetched.length, 1);
});

test('a fetch that lands after reset() is revoked, never cached', async () => {
  const f = fixture();
  const a = f.atts.load('x1');
  f.atts.reset(); // story switch
  f.pending.get('x1')!.resolve('blob:stale');
  assert.equal(await a, null);
  assert.deepEqual(f.revoked, ['blob:stale']);
  assert.equal(f.atts.urls.x1, undefined);
  // The new story's load fetches again.
  const b = f.atts.load('x1');
  assert.equal(f.fetched.length, 2);
  f.pending.get('x1')!.resolve('blob:fresh');
  assert.equal(await b, 'blob:fresh');
});

test('reset() and forget() revoke what was cached; a failure is recorded', async () => {
  const f = fixture();
  const a = f.atts.load('a');
  const b = f.atts.load('b');
  f.pending.get('a')!.resolve('blob:a');
  f.pending.get('b')!.resolve('blob:b');
  await Promise.all([a, b]);
  f.atts.forget('a');
  assert.deepEqual(f.revoked, ['blob:a']);
  assert.equal(f.atts.urls.a, undefined);
  f.atts.reset();
  assert.deepEqual(f.revoked, ['blob:a', 'blob:b'], 'forget() dropped `a` from the revoke list');
  const c = f.atts.load('c');
  f.pending.get('c')!.reject(new Error('Load failed'));
  assert.equal(await c, null);
  assert.equal(f.atts.errors.c, 'Load failed');
  assert.equal(f.atts.loading.c, false);
});
