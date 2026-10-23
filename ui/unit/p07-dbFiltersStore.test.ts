import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

// architecture-1-10: the quick-filter chips moved out of the DatabaseStore
// façade into lib/stores/database/filters.svelte.ts. These run the real
// sub-store (and its real statement / chip helpers) against a fake host.

function setup(statement: string, engine: string | null = 'mysql') {
  const { DbFiltersStore } = loadSource(
    new URL('../src/lib/stores/database/filters.svelte.ts', import.meta.url),
    {},
    {},
    { relative: true },
  );
  const tab = { statement, filters: [] as unknown[] };
  let persisted = 0;
  const set: string[] = [];
  const f = new DbFiltersStore({
    activeTab: () => tab,
    engine: () => engine,
    setStatement: (s: string) => { set.push(s); tab.statement = s; },
    persistTabs: () => { persisted++; },
  });
  return { f, tab, set, persisted: () => persisted };
}

test('a cell filter folds the hand-written WHERE into a raw chip, then owns the WHERE', () => {
  const h = setup('SELECT * FROM t WHERE a > 1');
  h.f.addQuickFilter('b', 'x', 'include');
  assert.equal(h.f.filters.length, 2);
  assert.match(h.tab.statement, /WHERE a > 1 AND `b` = 'x'/);
  h.f.addQuickFilter('b', 'y', 'include');
  assert.match(h.tab.statement, /`b` IN \('x', 'y'\)/, 'repeated equals collapse into IN');
  h.f.toggleFilterMode(1);
  assert.match(h.tab.statement, /`b` NOT IN/);
  h.f.clearFilters();
  assert.doesNotMatch(h.tab.statement, /WHERE/);
  assert.ok(h.persisted() >= 4, 'every chip change persists the tab');
});

test('Postgres chips quote identifiers with double quotes', () => {
  const h = setup('SELECT * FROM t', 'postgres');
  h.f.addQuickFilter('name', 'o\'neil', 'exclude');
  assert.match(h.tab.statement, /"name" <> 'o''neil'/);
});

test('a cell "query by value" drops the chips without rebuilding the WHERE from them', () => {
  const h = setup('SELECT * FROM t');
  h.f.addQuickFilter('a', 1, 'include');
  h.f.setStatementFromCellFilter('SELECT * FROM t WHERE c = 2');
  assert.equal(h.f.filters.length, 0);
  assert.deepEqual(h.set, ['SELECT * FROM t WHERE c = 2']);
});
