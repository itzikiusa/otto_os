import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';
import * as plural from '../src/lib/plural.ts';

// The grid's "Review & apply" (EditFlow.runReview) against a stub store: what
// a successful write does to the parked drafts, and which cancel reasons toast.
const reviewBatch = loadSource(new URL('../src/modules/database/review-batch.ts', import.meta.url), {}, {}, { relative: true });

type Built = { title: string; sql: string; diff: unknown[]; stmtRows: number[] };
function setup(
  write: (sql: string) => unknown,
  opts: { tabMoves?: boolean; engine?: string; rows?: number; buildUpdate?: (rows: { rowIdx: number }[]) => Built } = {},
) {
  const toasts: string[] = [];
  const reruns: unknown[] = [];
  const sent: string[] = [];
  const database = {
    tab: { id: 'tab-1' },
    selectedConnId: 'c1',
    activeDb: 'app',
    runManagedStatement: async (sql: string) => {
      sent.push(sql);
      const r = await write(sql);
      if (opts.tabMoves) database.tab = { id: 'tab-2' };
      return r;
    },
    runQuery: async (...a: unknown[]) => { reruns.push(a); return null; },
  };
  const adapter = {
    buildUpdate: opts.buildUpdate ?? ((rows: { rowIdx: number }[]) => ({ title: 'Review UPDATE', sql: `UPDATE t SET -- rows ${rows.map((r) => r.rowIdx).join(',')}`, diff: [], stmtRows: rows.map((r) => r.rowIdx) })),
    buildDuplicate: () => ({ title: 'Review INSERT', sql: 'INSERT INTO t VALUES (1)' }),
  };
  const { EditFlow } = loadSource(new URL('../src/modules/database/EditFlow.svelte.ts', import.meta.url), {
    '../../lib/toast.svelte': { toasts: { info: (t: string) => toasts.push(`info:${t}`), success: (t: string) => toasts.push(`success:${t}`), error: (t: string) => toasts.push(`error:${t}`) } },
    './review-batch': reviewBatch,
    '../../lib/stores/database.svelte': { database },
    './results-format': {}, './edit-types': { adapterFor: () => adapter },
    './edit-sql': { scopeDatabase: (n: string | null) => n }, './edit-mongo': {}, '../../lib/jsonTree/expansion-plan': {},
    '../../lib/toastError': { toastError: (t: string) => toasts.push(`error:${t}`) }, '../../lib/plural': plural,
  });
  const flow = new EditFlow();
  const n = opts.rows ?? 2;
  Object.assign(flow, {
    editable: true, result: { columns: [{ name: 'id' }, { name: 'v' }] }, engine: opts.engine ?? 'mysql',
    statement: 'SELECT * FROM t', ranNode: 'app', editDb: 'app', editTable: 't', editPkCols: ['id'],
    connectionId: 'c1', liveRows: Array.from({ length: n }, (_, i) => [i + 1, String.fromCharCode(97 + i)]),
  });
  for (let i = 0; i < n; i++) flow.parkValue(i, 1, `v${i}`);
  return { flow, toasts, reruns, sent };
}

/** One `UPDATE …;` per row — what the SQL adapter builds. */
const perRowUpdate = (rows: { rowIdx: number }[]): Built => ({
  title: 'Review UPDATE',
  sql: rows.map((r) => `UPDATE t SET v = 'x' WHERE id = ${r.rowIdx + 1};`).join('\n'),
  diff: [],
  stmtRows: rows.map((r) => r.rowIdx),
});
const ok = (statement: string) => ({ columns: [], rows: [], affected_rows: 1, statement });
const failed = (statement: string) => ({ columns: [], rows: [], errored: true, statement, message: 'Duplicate entry' });

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

test('a 3-row SQL review whose 2nd statement fails runs in one transaction: nothing applied, drafts and modal kept', async () => {
  // The driver stops at the failing statement and the BEGIN-marked session is
  // closed, not committed: the 200 says errored, the UI must not say Applied.
  const h = setup(() => ({ ...ok('BEGIN'), more_results: [ok('UPDATE … id = 1'), failed('UPDATE … id = 2')] }), {
    rows: 3, buildUpdate: perRowUpdate,
  });
  h.flow.reviewPending();
  await h.flow.runReview();
  assert.match(h.sent[0], /^BEGIN;\n/);
  assert.match(h.sent[0], /\nCOMMIT;$/);
  assert.equal(h.flow.pending.size, 3, 'a rolled-back batch wrote nothing: every draft stays pending');
  assert.ok(h.flow.reviewSql, 'the review modal stays open');
  assert.match(h.flow.reviewError ?? '', /Duplicate entry/);
  assert.match(h.flow.reviewError ?? '', /UPDATE … id = 2/);
  assert.match(h.flow.reviewError ?? '', /rolled back/);
  assert.equal(h.reruns.length, 0);
  assert.deepEqual(h.toasts, ['error:The write stopped partway']);
});

test('a Mongo review that fails partway clears only the drafts whose statements completed', async () => {
  const h = setup(() => ({ ...ok('updateOne #1'), more_results: [failed('updateOne #2')] }), {
    engine: 'mongodb', rows: 3, buildUpdate: perRowUpdate,
  });
  h.flow.reviewPending();
  await h.flow.runReview();
  assert.doesNotMatch(h.sent[0], /BEGIN/, 'no batch transaction on Mongo');
  assert.deepEqual([...h.flow.pending.keys()].sort(), [1, 2], 'row 0 was written; rows 1–2 stay pending');
  assert.ok(h.flow.reviewSql);
  assert.match(h.flow.reviewError ?? '', /1 statement before it ran/);
  assert.deepEqual(h.toasts, ['error:The write stopped partway']);
});

test('a review the person edited keeps every draft when it fails partway (no statement → row mapping)', async () => {
  const h = setup(() => ({ ...ok('a'), more_results: [failed('b')] }), { engine: 'mongodb', rows: 2, buildUpdate: perRowUpdate });
  h.flow.reviewPending();
  h.flow.setReviewSql(`${h.flow.reviewSql!.sql}\n// edited`);
  await h.flow.runReview();
  assert.equal(h.flow.pending.size, 2);
});

test('reviewBatchSql wraps only multi-statement MySQL / Postgres reviews that manage no transaction', () => {
  const two = "UPDATE t SET v = 1 WHERE id = 1;\nUPDATE t SET v = 2 WHERE id = 2;";
  assert.equal(reviewBatch.reviewBatchSql('postgres', two).atomic, true);
  assert.equal(reviewBatch.reviewBatchSql('mysql', 'UPDATE t SET v = 1 WHERE id = 1;').atomic, false);
  assert.equal(reviewBatch.reviewBatchSql('mongodb', two).atomic, false);
  assert.equal(reviewBatch.reviewBatchSql('mysql', `START TRANSACTION;\n${two}\nCOMMIT;`).atomic, false);
  // A `;` inside a string literal is not a statement break.
  assert.equal(reviewBatch.reviewBatchSql('mysql', "UPDATE t SET v = 'a;b' WHERE id = 1;").atomic, false);
});

test('completedRows: a row split over several statements counts only once all of them ran', () => {
  assert.deepEqual([...reviewBatch.completedRows([0, 0, 1, 2], 1)], []);
  assert.deepEqual([...reviewBatch.completedRows([0, 0, 1, 2], 3)].sort(), [0, 1]);
});
