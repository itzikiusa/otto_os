import { test } from 'node:test';
import assert from 'node:assert/strict';
import { tableWindow } from '../src/modules/kubernetes/monitor/tableWindow.ts';

test('a 5k-workload table renders only the rows near the viewport', () => {
  const w = tableWindow({ count: 5000, rowH: 50, viewTop: 0, viewH: 800, overscan: 8 });
  assert.equal(w.start, 0);
  assert.equal(w.end, 16 + 8 + 1);
  assert.equal(w.padTop, 0);
  assert.equal(w.padBottom, (5000 - w.end) * 50);
  assert.ok(w.end - w.start <= 40, 'bounded DOM');
});

test('scrolling deep keeps the total height and a bounded slice', () => {
  const w = tableWindow({ count: 5000, rowH: 50, viewTop: 100_000, viewH: 800, overscan: 8 });
  assert.equal(w.start, 2000 - 8);
  assert.equal(w.end, 2016 + 8 + 1);
  assert.equal(w.padTop + (w.end - w.start) * 50 + w.padBottom, 5000 * 50);
});

test('the expanded detail row shifts every later row by its height', () => {
  const detailH = 600;
  // Row 10 expanded: rows 11+ sit 600 px lower.
  const w = tableWindow({ count: 1000, rowH: 50, viewTop: 11 * 50 + detailH, viewH: 100, overscan: 0, expanded: 10, detailH });
  assert.equal(w.start, 11);
  assert.equal(w.padTop, 11 * 50 + detailH);
  assert.equal(w.padTop + (w.end - w.start) * 50 + w.padBottom, 1000 * 50 + detailH);
  // Expanded row above the window: its detail is folded into the top spacer.
  const above = tableWindow({ count: 1000, rowH: 50, viewTop: 20_000, viewH: 100, overscan: 0, expanded: 10, detailH });
  assert.equal(above.padTop, above.start * 50 + detailH);
});

test('the table starting below the fold and empty tables are safe', () => {
  const w = tableWindow({ count: 100, rowH: 50, viewTop: -300, viewH: 800, overscan: 2 });
  assert.equal(w.start, 0);
  assert.ok(w.end <= 100 && w.end > 0);
  assert.deepEqual(tableWindow({ count: 0, rowH: 50, viewTop: 0, viewH: 800 }), { start: 0, end: 0, padTop: 0, padBottom: 0 });
});
