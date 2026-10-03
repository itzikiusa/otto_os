import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { FILE_REF_PREFIX, filesForSave, refSha, sha256Hex } from '../src/modules/canvas/canvasFileRefs.ts';
import { DEFAULT_FILE_CACHE_BYTES, FileCache } from '../src/modules/canvas/canvasFileCache.ts';

const img = `data:image/png;base64,${'A'.repeat(3 * 1024 * 1024)}`;
const sha = createHash('sha256').update(img).digest('hex');

test('refSha accepts only full sha256 refs', () => {
  assert.equal(refSha(FILE_REF_PREFIX + sha), sha);
  assert.equal(refSha(img), null);
  assert.equal(refSha(`${FILE_REF_PREFIX}abc`), null);
  assert.equal(refSha(undefined), null);
});

test('the client digest is the server content address (sha256 of the data-URL text)', async () => {
  assert.equal(await sha256Hex(img), sha);
});

test('a 3 MB image goes inline once, then every autosave sends a ref', () => {
  const known = new Map<string, string>();
  const files = { f1: { id: 'f1', mimeType: 'image/png', dataURL: img } };
  // First save: the server hasn't seen it.
  const first = filesForSave(files, known);
  assert.equal(first.inline.length, 1);
  assert.equal(first.files.f1.dataURL, img);
  // Learned after the save lands → the next 5 edits are small.
  known.set('f1', sha);
  for (let i = 0; i < 5; i++) {
    const next = filesForSave(files, known);
    assert.equal(next.inline.length, 0);
    const body = JSON.stringify({ doc: { source: JSON.stringify({ elements: [{ id: `e${i}` }], files: next.files }) } });
    assert.ok(body.length < 200 * 1024, `autosave body ${body.length} B`);
    assert.equal(next.files.f1.mimeType, 'image/png', 'other file fields are kept');
  }
});

test('an unresolved ref passes through and is not re-learned as inline', () => {
  const ref = { id: 'f2', dataURL: FILE_REF_PREFIX + sha };
  const out = filesForSave({ f2: ref }, new Map());
  assert.equal(out.files.f2.dataURL, ref.dataURL);
  assert.equal(out.inline.length, 0);
});

// A fetcher that serves `size`-char data URLs and counts calls per sha.
function countingFetcher(size: number) {
  const calls = new Map<string, number>();
  const fetcher = async (sha: string) => {
    calls.set(sha, (calls.get(sha) ?? 0) + 1);
    return `data:image/png;base64,${sha}`.padEnd(size, 'A');
  };
  return { calls, fetcher };
}
const tick = () => new Promise((r) => setImmediate(r));

test('the file cache defaults to a 64 MB bound', () => {
  assert.equal(DEFAULT_FILE_CACHE_BYTES, 64 * 1024 * 1024);
});

test('the file cache is a byte-bounded LRU: past the cap the oldest sha is refetched', async () => {
  const MB = 1024 * 1024;
  const { calls, fetcher } = countingFetcher(3 * MB);
  const cache = new FileCache(fetcher, 8 * MB); // room for two 3 MB files
  const board = {};
  await cache.get('a', board);
  await cache.get('b', board);
  await tick();
  assert.equal(cache.bytes, 6 * MB);
  await cache.get('a', board); // hit: 'a' becomes most recent, 'b' the oldest
  assert.equal(calls.get('a'), 1, 'a hit is not refetched');
  await cache.get('c', board); // 9 MB > 8 MB → evict the LRU ('b')
  await tick();
  assert.equal(cache.bytes, 6 * MB, 'total stays under the cap');
  assert.ok(!cache.has('b') && cache.has('a') && cache.has('c'));
  await cache.get('b', board);
  assert.equal(calls.get('b'), 2, 'the evicted sha is fetched again');
});

test('a board unmount releases its files unless another open board uses them', async () => {
  const { calls, fetcher } = countingFetcher(1024);
  const cache = new FileCache(fetcher);
  const one = {};
  const two = {};
  await cache.get('shared', one);
  await cache.get('only1', one);
  await cache.get('shared', two);
  await tick();
  assert.equal(cache.bytes, 2048);
  cache.release(one);
  assert.ok(!cache.has('only1'), "board one's own file is freed");
  assert.ok(cache.has('shared'), 'still used by board two');
  assert.equal(cache.bytes, 1024);
  cache.release(two);
  assert.equal(cache.bytes, 0);
  assert.ok(!cache.has('shared'));
  await cache.get('shared', {});
  assert.equal(calls.get('shared'), 2, 'a reopen fetches again (the HTTP cache serves it)');
});

test('a failed or mid-flight-released fetch leaves nothing behind', async () => {
  let fail = true;
  const cache = new FileCache(async (sha) => {
    if (fail) throw new Error('boom');
    return `data:${sha}`;
  });
  await assert.rejects(cache.get('x', {}));
  await tick();
  assert.ok(!cache.has('x'), 'failures are not cached');
  fail = false;
  const board = {};
  const p = cache.get('y', board);
  cache.release(board); // unmounted before the fetch resolved
  await p;
  await tick();
  assert.ok(!cache.has('y'));
  assert.equal(cache.bytes, 0, 'a late resolve is not counted');
});
