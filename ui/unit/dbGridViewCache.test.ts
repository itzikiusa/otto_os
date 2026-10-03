import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  SCAN_MAX,
  colScanFor,
  dropColScans,
  dropScan,
  memoFilter,
  memoSort,
  prebuild,
  rowScanText,
  scanFor,
  scanProgress,
  type ViewRow,
} from '../src/modules/database/grid-view-cache.ts';

const cell = (v: unknown): string => (typeof v === 'object' ? JSON.stringify(v) : String(v));
const rowsOf = (n: number): unknown[][] =>
  Array.from({ length: n }, (_, i) => [i, `Name${i}`, i % 3 === 0 ? null : { k: i }]);

/** A manual scheduler: slices run only when the test says so. */
function manual() {
  const q: (() => void)[] = [];
  return {
    schedule: (fn: () => void) => {
      q.push(fn);
      return () => {
        const i = q.indexOf(fn);
        if (i >= 0) q.splice(i, 1);
      };
    },
    runOne: () => q.shift()?.(),
    get pending() {
      return q.length;
    },
  };
}

test('row scan text skips NULLs, lower-cases and clips', () => {
  assert.equal(rowScanText([1, null, 'AbC', { x: 1 }], cell), '1\u0000abc\u0000{"x":1}\u0000');
  assert.equal(rowScanText(['x'.repeat(SCAN_MAX + 10)], cell).length, SCAN_MAX);
});

test('scanFor builds once per rows array and is reused', () => {
  const rows = rowsOf(10);
  let calls = 0;
  const counting = (v: unknown) => {
    calls++;
    return cell(v);
  };
  const a = scanFor(rows, counting);
  const first = calls;
  const b = scanFor(rows, counting);
  assert.equal(a, b);
  assert.equal(calls, first, 'second call reads the cache');
  assert.equal(a[4], '4\u0000name4\u0000{"k":4}\u0000');
});

test('prebuild fills the search text in time-boxed slices, scanFor finishes the rest', () => {
  const rows = rowsOf(2000);
  const m = manual();
  let t = 0;
  // Each `now()` advances 1 ms: an 8 ms slice builds a few 256-row chunks.
  const cancel = prebuild(rows, 'search', cell, { schedule: m.schedule, sliceMs: 3, now: () => t++ });
  assert.equal(scanProgress(rows), 0, 'nothing runs synchronously');
  m.runOne();
  const after1 = scanProgress(rows);
  assert.ok(after1 > 0 && after1 < rows.length, `one slice built ${after1}`);
  assert.equal(m.pending, 1, 'next slice scheduled');
  // The first key arrives before the slices finish: scanFor completes it.
  const scan = scanFor(rows, cell);
  assert.equal(scanProgress(rows), rows.length);
  assert.equal(scan[1999], rowScanText(rows[1999], cell));
  m.runOne(); // the stale slice sees the work done and stops
  assert.equal(m.pending, 0);
  cancel();
});

test('cancel stops a column prebuild; dropColScans frees all but the kept columns', () => {
  const rows = rowsOf(3000);
  const m = manual();
  let t = 0;
  const cancel = prebuild(rows, 2, cell, { schedule: m.schedule, sliceMs: 2, now: () => t++ });
  m.runOne();
  const got = scanProgress(rows, 2);
  assert.ok(got > 0 && got < rows.length);
  cancel();
  assert.equal(m.pending, 0, 'the scheduled slice was cancelled');
  const col = colScanFor(rows, 2, cell);
  assert.equal(col[0], null, 'NULL stays null');
  assert.equal(col[1], '{"k":1}');
  colScanFor(rows, 1, cell);
  dropColScans(rows, new Set([1]));
  assert.equal(scanProgress(rows, 2), 0);
  assert.equal(scanProgress(rows, 1), rows.length);
});

test('dropScan frees the search text', () => {
  const rows = rowsOf(5);
  scanFor(rows, cell);
  dropScan(rows);
  assert.equal(scanProgress(rows), 0);
});

test('memoFilter / memoSort return the cached view for the same key (tab switch back)', () => {
  const rows = rowsOf(100);
  const all: ViewRow[] = rows.map((row, idx) => ({ row, idx }));
  let filters = 0;
  let sorts = 0;
  const filter = () => {
    filters++;
    return all.filter((r) => (r.idx as number) % 2 === 0);
  };
  const f1 = memoFilter(rows, 'even', filter);
  const f2 = memoFilter(rows, 'even', filter);
  assert.equal(f1, f2);
  assert.equal(filters, 1);
  const sort = () => {
    sorts++;
    return [...f1].reverse();
  };
  const s1 = memoSort(rows, f1, '0|desc', sort);
  assert.equal(memoSort(rows, f1, '0|desc', sort), s1);
  assert.equal(sorts, 1);
  // A different key or a different base recomputes.
  memoSort(rows, f1, '0|asc', sort);
  assert.equal(sorts, 2);
  memoFilter(rows, 'odd', () => all.filter((r) => r.idx % 2 === 1));
  assert.notEqual(memoFilter(rows, 'even', filter), f1, 'only the last view per result is kept');
  // Another rows array (another tab) has its own entry.
  const other = rowsOf(100);
  memoFilter(other, 'even', filter);
  assert.equal(filters, 3);
});
