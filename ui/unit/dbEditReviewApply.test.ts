import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';
import * as plural from '../src/lib/plural.ts';

// The grid's "Review & apply" (EditFlow.runReview) against a stub store: what
// a successful write does to the parked drafts, and which cancel reasons toast.
function setup(write: () => unknown, opts: { tabMoves?: boolean } = {}) {
  const toasts: string[] = [];
  const reruns: unknown[] = [];
  const database = {
    tab: { id: 'tab-1' },
    selectedConnId: 'c1',
    activeDb: 'app',
    runManagedStatement: async () => {
      const r = await write();
      if (opts.tabMoves) database.tab = { id: 'tab-2' };
      return r;
    },
    runQuery: async (...a: unknown[]) => { reruns.push(a); return null; },
  };
  const adapter = {
    buildUpdate: (rows: { rowIdx: number }[]) => ({ title: 'Review UPDATE', sql: `UPDATE t SET -- rows ${rows.map((r) => r.rowIdx).join(',')}` }),
    buildDuplicate: () => ({ title: 'Review INSERT', sql: 'INSERT INTO t VALUES (1)' }),
  };
  const { EditFlow } = loadSource(new URL('../src/modules/database/EditFlow.svelte.ts', import.meta.url), {
    '../../lib/toast.svelte': { toasts: { info: (t: string) => toasts.push(`info:${t}`), success: (t: string) => toasts.push(`success:${t}`) } },
    '../../lib/stores/database.svelte': { database },
    './results-format': {}, './edit-types': { adapterFor: () => adapter },
    './edit-sql': { scopeDatabase: (n: string | null) => n }, './edit-mongo': {}, '../../lib/jsonTree/expansion-plan': {},
    '../../lib/toastError': { toastError: (t: string) => toasts.push(`error:${t}`) }, '../../lib/plural': plural,
  });
  const flow = new EditFlow();
  Object.assign(flow, {
    editable: true, result: { columns: [{ name: 'id' }, { name: 'v' }] }, engine: 'mysql',
    statement: 'SELECT * FROM t', ranNode: 'app', editDb: 'app', editTable: 't', editPkCols: ['id'],
    connectionId: 'c1', liveRows: [[1, 'a'], [2, 'b']],
  });
  flow.parkValue(0, 1, 'x');
  flow.parkValue(1, 1, 'y');
  return { flow, toasts, reruns };
}

test('a successful edit review clears the drafts it applied, even when the refresh is skipped', async () => {
  const h = setup(() => ({ columns: [], rows: [], affected_rows: 2 }), { tabMoves: true });
  h.flow.reviewPending();
  assert.ok(h.flow.reviewSql);
  await h.flow.runReview();
  assert.equal(h.flow.pending.size, 0, 'applied drafts must not come back as pending');
  assert.equal(h.reruns.length, 0, 'tab moved: the refresh is skipped');
  assert.deepEqual(h.toasts, ['success:Applied', 'info:Applied in another tab']);
});

test('a duplicate-row review leaves the parked drafts alone', async () => {
  const h = setup(() => ({ columns: [], rows: [], affected_rows: 1 }));
  h.flow.duplicateRow(0);
  await h.flow.runReview();
  assert.equal(h.flow.pending.size, 2);
});

for (const [canceled, expected] of [['declined', ['info:Write canceled']], ['mismatch', []], ['stale', []]] as const) {
  test(`a canceled write (${canceled}) keeps the drafts and the modal; toasts only on a plain decline`, async () => {
    const h = setup(() => ({ canceled }));
    h.flow.reviewPending();
    await h.flow.runReview();
    assert.equal(h.flow.pending.size, 2);
    assert.ok(h.flow.reviewSql, 'the review modal stays open');
    assert.deepEqual(h.toasts, expected);
  });
}
