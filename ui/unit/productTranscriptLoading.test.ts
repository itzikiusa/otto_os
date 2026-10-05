import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deferred, loadSource } from './sourceHarness.ts';
import { componentFunctions } from './componentFunctions.ts';

function fixture(bodySize = 256 * 1024, count = 100) {
  const body = 'x'.repeat(bodySize);
  const rows = Array.from({ length: count }, (_, i) => ({ id: `transcript-${i}`, story_id: 'A', title: `Transcript ${i}`,
    body: `${body}\n${i === count - 1 ? 'needle in oldest collapsed transcript' : 'ordinary content'}`, created_at: new Date(Date.UTC(2026, 0, 1, 0, 0, i)).toISOString() }));
  const calls: string[] = [];
  let transferred = 0;
  const api = { get: async (path: string) => {
    calls.push(path);
    const url = new URL(path, 'http://fixture.test');
    let response: unknown;
    if (url.pathname.endsWith('/transcripts/search')) {
      response = { items: rows.filter(row => `${row.title}\n${row.body}`.includes(url.searchParams.get('q')!))
        .map(({ body, ...row }) => ({ ...row, body_bytes: Buffer.byteLength(body), match_count: 1 })), next_cursor: null };
    } else if (url.pathname === '/product/stories/A/transcripts') {
      const offset = url.searchParams.has('cursor') ? 50 : 0;
      response = url.searchParams.get('summary') === 'true'
        ? { items: rows.slice(offset, offset + 50).map(({ body, ...row }) => ({ ...row, body_bytes: Buffer.byteLength(body) })), next_cursor: offset + 50 < rows.length ? 'after-49' : null }
        : rows;
    } else if (url.pathname.startsWith('/product/transcripts/')) {
      response = rows.find(row => row.id === url.pathname.split('/').at(-1));
    } else throw new Error(`Unexpected request ${path}`);
    transferred += Buffer.byteLength(JSON.stringify(response));
    return response;
  } };
  const { product } = loadSource(new URL('../src/lib/stores/product.svelte.ts', import.meta.url), {
    '../api/client': { api }, '../loadError': { loadErrorText: String },
    './workspace.svelte': { ws: { currentId: 'workspace' } }, '../lazyModule': { announceModule() {} },
  });
  product.selectedId = 'A'; product.detail = { story: { id: 'A', source_kind: 'draft' } };
  return { product, calls, rows, api, transferred: () => transferred };
}

test('opening collapsed transcript history transfers only a bounded thin first page', async () => {
  const f = fixture(); await f.product.loadTranscripts();
  assert.ok(f.transferred() < 64 * 1024, `transport serialized ${f.transferred()} bytes for collapsed history`);
  assert.ok(f.product.transcripts.length <= 50);
  assert.ok(f.product.transcripts.every((row: { body?: string }) => row.body === undefined));
  assert.equal(f.calls.filter(path => path.startsWith('/product/transcripts/')).length, 0);
});

test('expanding a thin transcript fetches its body through the actual Overview handler', async () => {
  const f = fixture();
  const { body: _body, ...summary } = f.rows[0];
  f.product.transcripts = [summary];
  const overview = componentFunctions(new URL('../src/modules/product/OverviewTab.svelte', import.meta.url), ['toggleTranscript'], {
    product: f.product, expandedTranscripts: {},
  });
  await overview.toggleTranscript(summary.id);
  assert.ok(f.calls.includes(`/product/transcripts/${summary.id}`), 'expansion must load the requested body, not render an absent summary field');
  assert.equal(overview.expandedTranscripts[summary.id], true);
});

test('transcript cache enforces both byte and entry bounds and rehydrates evicted bodies', async () => {
  const f = fixture(700_000, 6);
  for (const row of f.rows) {
    await f.product.loadTranscriptBody(row.id);
    const retained = Object.values(f.product.transcriptBodies) as string[];
    assert.ok(retained.length <= 4);
    assert.ok(retained.reduce((sum, body) => sum + body.length * 2, 0) <= 4 * 1024 * 1024);
  }
  assert.equal(f.product.transcriptBodies[f.rows[0].id], undefined);
  const before = f.calls.length;
  await f.product.loadTranscriptBody(f.rows[0].id);
  assert.equal(f.calls.length, before + 1);
  assert.equal(f.product.transcriptBodies[f.rows[0].id], f.rows[0].body);
});

test('summary pagination stays bounded and older collapsed search reveals the correct body', async () => {
  const f = fixture();
  await f.product.loadTranscripts();
  await f.product.loadTranscripts(f.product.transcriptNextCursor);
  assert.equal(f.product.transcripts.length, 50);
  assert.equal(f.product.transcripts[0].id, 'transcript-50');
  await f.product.previousTranscriptPage();
  assert.equal(f.product.transcripts[0].id, 'transcript-0');
  const matches = await f.product.searchTranscripts('needle', 5000, new AbortController().signal);
  assert.equal(matches.length, 1);
  const matched = f.product.transcriptSearchRows[matches[0].row];
  assert.equal(matched.id, 'transcript-99');
  assert.equal(f.product.transcriptBodies[matched.id], undefined, 'search does not download matched bodies');
  await f.product.revealTranscript(matched);
  assert.ok(f.product.transcripts.some((row: { id: string }) => row.id === matched.id));
  assert.ok(f.product.transcripts.length <= 50);
  assert.equal(f.product.transcriptBodies[matched.id], f.rows[99].body);
});

test('failed body loading can retry without losing the selected summary', async () => {
  const f = fixture(); const get = f.api.get;
  let fail = true;
  f.api.get = async path => { if (fail) { fail = false; throw new Error('Body unavailable'); } return get(path); };
  f.product.transcripts = [{ id: 'transcript-0', title: 'Transcript 0' }];
  await f.product.loadTranscriptBody('transcript-0');
  assert.match(f.product.transcriptBodyErrors['transcript-0'], /Body unavailable/);
  assert.equal(f.product.transcripts[0].id, 'transcript-0');
  await f.product.loadTranscriptBody('transcript-0');
  assert.equal(f.product.transcriptBodies['transcript-0'], f.rows[0].body);
});

test('late transcript body belongs to the original story lifetime', async () => {
  const f = fixture(); const pendingBody = deferred<unknown>();
  f.api.get = async path => path.startsWith('/product/transcripts/') ? pendingBody.promise : { story: { id: 'B' } };
  const pending = f.product.loadTranscriptBody('transcript-0');
  await f.product.select('B');
  pendingBody.resolve(f.rows[0]); await pending;
  assert.equal(Object.keys(f.product.transcriptBodies).length, 0);
  assert.equal(Object.keys(f.product.transcriptBodyErrors).length, 0);
});

test('an oversized body stays out of the cache and remains downloadable in full', async () => {
  const f = fixture(2 * 1024 * 1024 + 1, 1);
  const row = f.rows[0];
  await f.product.loadTranscriptBody(row.id);
  assert.equal(Object.keys(f.product.transcriptBodies).length, 0);
  assert.match(f.product.transcriptBodyErrors[row.id], /Download/);
  let download: Blob | undefined;
  const overview = componentFunctions(new URL('../src/modules/product/OverviewTab.svelte', import.meta.url), ['downloadTranscript'], {
    product: f.product, Blob,
    URL: { createObjectURL: (blob: Blob) => { download = blob; return 'blob:transcript'; }, revokeObjectURL() {} },
    document: { createElement: () => ({ href: '', download: '', click() {} }) },
    toasts: { error: (title: string) => assert.fail(title) },
  });
  await overview.downloadTranscript(row);
  assert.equal(await download!.text(), row.body);
  assert.equal(Object.keys(f.product.transcriptBodies).length, 0);
});
