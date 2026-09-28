import { test } from 'node:test';
import assert from 'node:assert/strict';
import { listRange } from '../src/modules/git/list-range.ts';

// The window arithmetic behind the WIP panel, the PR file-nav tree (and the
// shared ListWindow): a list of n uniform rows inside an OUTER scroller.

test('small lists render whole (find-in-page keeps working)', () => {
  assert.deepEqual(listRange(300, 26, 5000, 800, 20, 300), { start: 0, end: 300, top: 0, bottom: 0 });
});

test('a 20k-row list mounts only the viewport plus overscan, spacers cover the rest', () => {
  const n = 20_000;
  const rowH = 26;
  const r = listRange(n, rowH, 0, 800, 20, 300);
  assert.equal(r.start, 0);
  assert.equal(r.end, Math.ceil(800 / rowH) + 20);
  assert.ok(r.end - r.start <= 600, 'mounted rows stay bounded');
  // Spacers + mounted rows always add up to the full list height.
  assert.equal(r.top + (r.end - r.start) * rowH + r.bottom, n * rowH);

  const mid = listRange(n, rowH, 10_000 * rowH, 800, 20, 300);
  assert.equal(mid.start, 10_000 - 20);
  assert.ok(mid.end > 10_000 + 800 / rowH);
  assert.ok(mid.end - mid.start <= 600);
  assert.equal(mid.top + (mid.end - mid.start) * rowH + mid.bottom, n * rowH);
});

test('the list starting below the fold (negative rel) still renders its head', () => {
  // The staged section begins 2000px down the scroller: rel = scrollTop - 2000.
  const r = listRange(5_000, 26, -2000, 800, 20, 300);
  assert.equal(r.start, 0);
  assert.ok(r.end >= 1 && r.end <= 21, `only the overscan head, got ${r.end}`);
});

test('scrolled past the end clamps to the tail and never inverts', () => {
  const r = listRange(1_000, 26, 10_000_000, 800, 20, 300);
  assert.ok(r.start <= r.end);
  assert.equal(r.end, 1_000);
  assert.equal(r.bottom, 0);
});
