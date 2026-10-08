import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deferred, loadSource } from './sourceHarness.ts';

function fixture(api: Record<string, unknown>) {
  return loadSource(new URL('../src/lib/stores/assistant.svelte.ts', import.meta.url), {
    svelte: { untrack: (fn: () => unknown) => fn() },
    '../api/client': { api: {}, ApiError: class extends Error {} },
    '../api/assistant': { assistantApi: api },
    '../../modules/assistant/model': { needsYouByThread: () => ({}), reduceNeedsYou: (s: unknown) => s,
      upsertNewer: (rows: any[], row: any) => [...rows.filter((r) => r.id !== row.id), row] },
  }).assistant;
}
const rows = Array.from({ length: 100 }, (_, i) => ({ id: `t${i}`, updated_at: '2' }));

test('thread paging retains rows on failure, retries the same offset and deduplicates', async () => {
  let fail = true; const offsets: number[] = [];
  const store = fixture({ threads: async (offset = 0) => {
    offsets.push(offset);
    if (!offset) return rows;
    if (fail) throw new Error('offline');
    return [rows[99], { id: 'older', updated_at: '1' }];
  } });
  await store.loadThreads(); await store.loadMoreThreads();
  assert.equal(store.threads.data.length, 100); assert.match(store.threadsMoreError, /offline/);
  fail = false; await store.loadMoreThreads();
  assert.equal(store.threads.data.length, 101); assert.equal(store.threadsHasMore, false);
  assert.deepEqual(offsets, [0, 100, 100]);
});

test('refresh invalidates an older page and selected older thread survives refresh', async () => {
  const page = deferred<any[]>();
  const store = fixture({ threads: async (offset = 0) => offset ? page.promise : rows,
    thread: async (id: string) => ({ id, updated_at: '1' }) });
  await store.loadThreads(); await store.loadSelectedThread('older');
  const pending = store.loadMoreThreads(); await store.loadThreads();
  page.resolve([{ id: 'obsolete' }]); await pending;
  assert.equal(store.threads.data.some((t: any) => t.id === 'obsolete'), false);
  assert.equal(store.thread('older')?.id, 'older');
  assert.equal(store.threadsHasMore, true);
});

test('selected thread late failure cannot replace a newer selection', async () => {
  const old = deferred<any>();
  const store = fixture({ thread: (id: string) => id === 'old' ? old.promise : Promise.resolve({ id, updated_at: '1' }) });
  const pending = store.loadSelectedThread('old'); await store.loadSelectedThread('new');
  old.reject(new Error('gone')); await pending;
  assert.equal(store.thread('new')?.id, 'new'); assert.equal(store.selectedThread.state, 'ready');
});
