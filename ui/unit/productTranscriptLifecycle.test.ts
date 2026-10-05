import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';
import { deferred, loadSource } from './sourceHarness.ts';
import { componentFunctions } from './componentFunctions.ts';

function fixture(count: number) {
  const rows = Array.from({ length: count }, (_, index) => ({ id: `old-${index}`, story_id: 'A',
    title: `Transcript ${index}`, body: `Body ${index}`, created_by: 'user', created_at: '2026-01-01T00:00:00Z' }));
  const calls: string[] = [];
  const summary = (row: typeof rows[number]) => { const { body, ...rest } = row; return { ...rest, body_bytes: body.length }; };
  const api = {
    get: async (path: string): Promise<any> => {
      calls.push(path);
      const url = new URL(path, 'http://fixture.test');
      if (url.pathname.endsWith('/transcripts/search')) return { items: [], next_cursor: null };
      if (url.pathname.startsWith('/product/transcripts/')) return rows.find(row => row.id === url.pathname.split('/').at(-1));
      const cursor = url.searchParams.get('cursor');
      const offset = cursor ? rows.findIndex(row => row.id === cursor) + 1 : 0;
      const page = rows.slice(offset, offset + 50);
      return { items: page.map(summary), next_cursor: offset + 50 < rows.length ? page.at(-1)!.id : null };
    },
    post: async (_path: string, request: { title: string; body: string }) => {
      const row = { ...rows[0], ...request, id: 'new-import' }; rows.unshift(row); return row;
    },
  };
  const { product } = loadSource(new URL('../src/lib/stores/product.svelte.ts', import.meta.url), {
    '../api/client': { api }, '../loadError': { loadErrorText: String },
    './workspace.svelte': { ws: { currentId: 'workspace' } }, '../lazyModule': { announceModule() {} },
  }, { Blob });
  product.selectedId = 'A';
  return { product, rows, api, calls, summary };
}

for (const count of [50, 51, 100]) {
  for (const older of count > 50 ? [false, true] : [false]) {
    test(`import into ${count} transcripts from ${older ? 'an older' : 'the first'} page preserves ordinary cursor reachability`, async () => {
      const f = fixture(count);
      await f.product.loadTranscripts();
      if (older) await f.product.loadTranscripts(f.product.transcriptNextCursor);
      await f.product.addTranscript({ title: 'Imported', body: 'New body' });
      const seen: string[] = [];
      for (let page = 0; page < 5; page++) {
        assert.ok(f.product.transcripts.length <= 50);
        seen.push(...f.product.transcripts.map((row: { id: string }) => row.id));
        if (!f.product.transcriptNextCursor) break;
        await f.product.loadTranscripts(f.product.transcriptNextCursor);
      }
      assert.deepEqual(seen, f.rows.map(row => row.id));
      assert.equal(new Set(seen).size, count + 1);
      while (f.product.transcriptPreviousCursors.length) await f.product.previousTranscriptPage();
      assert.equal(f.product.transcripts[0].id, 'new-import');
    });
  }
}

test('a delayed pre-import page cannot replace the refreshed first page after POST', async () => {
  const f = fixture(100), delayed = deferred<unknown>();
  await f.product.loadTranscripts();
  const get = f.api.get;
  const oldCursor = f.product.transcriptNextCursor;
  f.api.get = async path => path.includes(`cursor=${oldCursor}`) ? delayed.promise : get(path);
  const old = f.product.loadTranscripts(oldCursor);
  await f.product.addTranscript({ title: 'Imported', body: 'New body' });
  delayed.resolve({ items: f.rows.filter(row => row.id.startsWith('old-')).slice(50).map(f.summary), next_cursor: null });
  await old;
  assert.equal(f.product.transcripts[0].id, 'new-import');
  assert.equal(f.product.transcriptPreviousCursors.length, 0);
});

test('leaving an off-page search reveal restores the ordinary boundary item and cursor', async () => {
  const f = fixture(51);
  await f.product.loadTranscripts();
  const before = f.product.transcripts.map((row: { id: string }) => row.id);
  const cursor = f.product.transcriptNextCursor;
  await f.product.revealTranscript(f.summary(f.rows[50]));
  f.product.releaseTranscriptSearch();
  assert.deepEqual(Array.from(f.product.transcripts, (row: any) => row.id), Array.from(before));
  assert.equal(f.product.transcriptNextCursor, cursor);
  await f.product.loadTranscripts(cursor);
  assert.equal(f.product.transcripts[0].id, 'old-50');
});

/** Register the actual Overview $effect, including its actual reveal closure,
 * into the production registry. DOM adapters only identify the mounted row. */
function overviewProvider(product: any) {
  const registry = loadSource(new URL('../src/lib/findProviders.ts', import.meta.url), {}, {
    Node: { DOCUMENT_POSITION_FOLLOWING: 4 }, getComputedStyle: () => ({ position: 'fixed' }),
  });
  let scrolls = 0;
  const rowElement = { scrollIntoView() { scrolls++; } };
  const list = { isConnected: true, checkVisibility: () => true, contains: () => false, querySelector: () => rowElement };
  const source = readFileSync(new URL('../src/modules/product/OverviewTab.svelte', import.meta.url), 'utf8').split('<script lang="ts">')[1].split('</script>')[0];
  const file = ts.createSourceFile('Overview.ts', source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
  const effect = file.statements.find(statement => ts.isExpressionStatement(statement)
    && ts.isCallExpression(statement.expression) && statement.expression.expression.getText(file) === '$effect'
    && statement.expression.arguments[0]?.getText(file).includes('registerFindProvider')) as ts.ExpressionStatement;
  assert.ok(effect, 'test must execute the real Overview provider registration');
  const callback = (effect.expression as ts.CallExpression).arguments[0].getText(file);
  let provider: any;
  const state: Record<string, any> = { product, transcriptListEl: list, expandedTranscripts: {},
    tick: async () => {}, CSS: { escape: (value: string) => value },
    registerFindProvider: (value: any) => { provider = value; return registry.registerFindProvider(value); } };
  runInNewContext(ts.transpileModule(`globalThis.cleanup = (${callback})();`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText, state);
  return { state, provider, registry, rowElement, scrolls: () => scrolls };
}

test('real registry unregister stops Product search before its next cursor GET', async () => {
  const f = fixture(2), page = deferred<unknown>();
  let reads = 0;
  f.api.get = async () => { reads++; return reads === 1 ? page.promise : { items: [], next_cursor: null }; };
  const overview = overviewProvider(f.product);
  const pending = overview.provider.search('body', 5000, new AbortController().signal);
  overview.state.cleanup();
  page.resolve({ items: [{ ...f.summary(f.rows[0]), match_count: 1 }], next_cursor: 'more' });
  await pending;
  assert.equal(reads, 1, 'unregister must release search generation before another HTTP page');
  assert.equal(f.product.transcriptSearchRows.length, 0);
});

test('real registry unregister releases retained search rows even when the pending read rejects', async () => {
  const f = fixture(2), page = deferred<unknown>();
  f.product.transcriptSearchRows = [f.summary(f.rows[0])];
  f.api.get = () => page.promise;
  const overview = overviewProvider(f.product);
  const pending = overview.provider.search('body', 5000, new AbortController().signal);
  overview.state.cleanup(); page.reject(new Error('Departed lookup failed'));
  await pending.catch(() => {});
  assert.equal(f.product.transcriptSearchRows.length, 0);
});

for (const action of ['close', 'no-results query', 'normal reveal']) {
  test(`real asynchronous reveal honors ${action} before expanding, painting or scrolling`, async () => {
    const f = fixture(1), body = deferred<unknown>();
    const bodyStarted = deferred<void>();
    f.api.get = async path => {
      if (path.startsWith('/product/transcripts/')) { bodyStarted.resolve(); return body.promise; }
      return { items: [], next_cursor: null };
    };
    f.product.transcripts = [f.summary(f.rows[0])];
    f.product.transcriptSearchRows = [f.summary(f.rows[0])];
    const overview = overviewProvider(f.product);
    let paints = 0;
    const state = componentFunctions(new URL('../src/lib/components/FindInPage.svelte', import.meta.url), ['goTo', 'runSearch', 'dropRanges'], {
      query: 'Body', searched: 'body', MAX_MATCHES: 5000, navSeq: 0,
      rowMatches: [{ p: overview.provider, row: 0, nth: 0 }], ranges: [], totalCount: 1, currentIdx: 0,
      currentRange: null, truncated: false, searching: false, searchError: '', searchSequence: 0,
      searchAbort: new AbortController(), AbortController, findInPage: { open: true },
      clearHighlights() {}, releaseFindProviders: overview.registry.releaseFindProviders,
      activeFindProviders: overview.registry.activeFindProviders, countOccurrences: overview.registry.countOccurrences,
      getContentRoots: () => [], nextFrame: async () => {},
      applyHighlights: () => { paints++; }, applyCurrent: () => { paints++; },
      locateInElement: () => ({ node: { parentElement: overview.rowElement }, offset: 0 }), makeRange: () => ({}),
    });
    const reveal = state.goTo(0); await bodyStarted.promise;
    if (action === 'close') { state.findInPage.open = false; state.dropRanges(); }
    else if (action === 'no-results query') { state.query = 'absent'; await state.runSearch(); }
    const paintsBeforeResponse = paints;
    body.resolve(f.rows[0]); await reveal;
    if (action === 'normal reveal') {
      assert.equal(overview.state.expandedTranscripts['old-0'], true);
      assert.ok(paints > paintsBeforeResponse);
      assert.equal(overview.scrolls(), 1);
    } else {
      assert.notEqual(overview.state.expandedTranscripts['old-0'], true);
      assert.equal(paints, paintsBeforeResponse, 'canceled reveal must not repaint');
      assert.equal(overview.scrolls(), 0);
    }
    overview.state.cleanup();
  });
}
