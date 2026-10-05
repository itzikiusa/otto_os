import { test } from 'node:test';
import assert from 'node:assert/strict';
import { componentFunctions } from './componentFunctions.ts';
import { deferred } from './sourceHarness.ts';
import { countOccurrences } from '../src/lib/findProviders.ts';

function fixture(search: (query: string, limit: number, signal: AbortSignal) => Promise<{ row: number; count: number }[]>) {
  const provider = { root: () => null, count: () => 100, text: () => '', reveal() {}, rowElement: () => null, search };
  const root = {};
  let active = [{ provider, root }];
  const state = componentFunctions(new URL('../src/lib/components/FindInPage.svelte', import.meta.url), ['runSearch', 'dropRanges'], {
    query: 'oldest transcript', MAX_MATCHES: 5000, truncated: false, currentRange: null,
    searched: '', ranges: [], rowMatches: [], totalCount: 0, currentIdx: 0,
    searchSequence: 0, searchAbort: null, navSeq: 0, searching: false, searchError: '',
    findInPage: { open: true }, AbortController,
    clearHighlights() {}, activeFindProviders: () => active, releaseFindProviders() {},
    countOccurrences, getContentRoots: () => [], applyHighlights() {}, goTo() {},
  });
  state.replaceProvider = (next: typeof search) => { active = [{ provider: { ...provider, search: next }, root: {} }]; };
  return state;
}

test('page find includes an unloaded provider match beyond the first transcript page', async () => {
  const request = deferred<{ row: number; count: number }[]>();
  const state = fixture(() => request.promise);
  const searching = state.runSearch();
  request.resolve([{ row: 99, count: 2 }]); await searching;
  assert.equal(state.totalCount, 2);
  assert.equal(state.rowMatches[0].row, 99);
  assert.equal(state.rowMatches[1].nth, 1);
});

test('a late old async provider query cannot replace the current matches', async () => {
  const old = deferred<{ row: number; count: number }[]>();
  const state = fixture(query => query === 'oldest transcript' ? old.promise : Promise.resolve([{ row: 3, count: 1 }]));
  const pending = state.runSearch();
  state.query = 'new query'; await state.runSearch();
  old.resolve([{ row: 99, count: 2 }]); await pending;
  assert.equal(state.totalCount, 1);
  assert.equal(state.rowMatches[0].row, 3);
  assert.equal(state.searched, 'new query');
});

test('remote find failure is visible rather than reporting whole-history absence', async () => {
  const state = fixture(async () => { throw new Error('Transcript search unavailable'); });
  await state.runSearch();
  assert.match(state.searchError, /Transcript search unavailable/);
});

test('a failed departed provider cannot report its error in the replacement provider', async () => {
  const old = deferred<{ row: number; count: number }[]>();
  const state = fixture(() => old.promise);
  const pending = state.runSearch();
  state.replaceProvider(async () => [{ row: 7, count: 1 }]);
  old.reject(new Error('Error owned by departed provider')); await pending;
  assert.equal(state.searchError, '');
});

test('closing find aborts pending provider search and prevents late results', async () => {
  const request = deferred<{ row: number; count: number }[]>();
  let signal: AbortSignal | undefined;
  const state = fixture((_query, _limit, abort) => { signal = abort; return request.promise; });
  const pending = state.runSearch();
  state.findInPage.open = false; state.dropRanges();
  assert.equal(signal?.aborted, true);
  request.resolve([{ row: 99, count: 1 }]); await pending;
  assert.equal(state.rowMatches.length, 0);
  assert.equal(state.searching, false);
});
