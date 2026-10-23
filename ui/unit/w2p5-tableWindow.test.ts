import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

// architecture-6-05r: the k8s Monitor table window is the shared `listRange`
// with its `extra` band (an expanded row's detail row), not a second copy of
// the range math. These tests pin (a) agreement with the old hand-rolled
// window wherever the old one was right, and (b) the expanded-row case it got
// wrong: a viewport inside the detail band mapped to rows far below it, so the
// expanded row (and its detail) unmounted while on screen.

const stubs = {
  svelte: { tick: async () => {}, untrack: (fn: () => unknown) => fn() },
  './findProviders': { registerFindProvider: () => () => {} },
};
const k8s = loadSource(new URL('../src/modules/kubernetes/monitor/tableWindow.ts', import.meta.url), stubs, {}, { relative: true });
const lib = loadSource(new URL('../src/lib/listWindow.svelte.ts', import.meta.url), stubs);

type Win = { start: number; end: number; padTop: number; padBottom: number };
type In = { count: number; rowH: number; viewTop: number; viewH: number; expanded?: number; detailH?: number; overscan?: number };

/** The window the Monitor table used before this change (verbatim math). */
function oldTableWindow(p: In): Win {
  const { count, rowH } = p;
  if (count <= 0 || rowH <= 0) return { start: 0, end: 0, padTop: 0, padBottom: 0 };
  const overscan = Math.max(0, p.overscan ?? 8);
  const exp = p.expanded ?? -1;
  const detailH = exp >= 0 && exp < count ? Math.max(0, p.detailH ?? 0) : 0;
  const indexAt = (y: number): number => {
    const afterDetail = (exp + 1) * rowH + detailH;
    const i = detailH > 0 && y >= afterDetail ? exp + 1 + Math.floor((y - afterDetail) / rowH) : Math.floor(y / rowH);
    return Math.max(0, Math.min(count - 1, i));
  };
  const first = indexAt(Math.max(0, p.viewTop));
  const last = indexAt(Math.max(0, p.viewTop + Math.max(0, p.viewH)));
  const start = Math.max(0, first - overscan);
  const end = Math.min(count, last + overscan + 1);
  const top = (i: number): number => i * rowH + (detailH > 0 && i > exp ? detailH : 0);
  return { start, end, padTop: top(start), padBottom: Math.max(0, top(count) - top(end)) };
}

const tableWindow = k8s.tableWindow as (p: In) => Win;
const total = (p: In) => p.count * p.rowH + ((p.expanded ?? -1) >= 0 ? (p.detailH ?? 0) : 0);
const top = (p: In, i: number) => i * p.rowH + ((p.expanded ?? -1) >= 0 && i > p.expanded! ? (p.detailH ?? 0) : 0);

test('a 5k-workload table renders only the rows near the viewport', () => {
  const w = tableWindow({ count: 5000, rowH: 50, viewTop: 0, viewH: 800, overscan: 8 });
  assert.equal(w.start, 0);
  // Rows 0..15 fill [0, 800); row 16 starts exactly AT the bottom edge and is
  // not visible — the old window mounted it as a ninth overscan row.
  assert.equal(w.end, 16 + 8);
  assert.equal(w.padBottom, (5000 - w.end) * 50);
  assert.ok(w.end - w.start <= 40, 'bounded DOM');
});

test('agrees with the old window off the detail band (identical start, end within its one spare row)', () => {
  for (const exp of [-1, 3, 40]) {
    for (let viewTop = -300; viewTop <= 4000; viewTop += 37) {
      for (const viewH of [0, 120, 800]) {
        const p: In = { count: 200, rowH: 25, viewTop, viewH, overscan: 4, expanded: exp, detailH: exp >= 0 ? 160 : 0 };
        const bandTop = (exp + 1) * 25;
        const inBand = (y: number) => exp >= 0 && y >= bandTop && y < bandTop + 160;
        if (inBand(Math.max(0, viewTop)) || inBand(Math.max(0, viewTop + viewH))) continue;
        const a = oldTableWindow(p);
        const b = tableWindow(p);
        assert.equal(b.start, a.start, `start @${viewTop}/${viewH}/${exp}`);
        assert.ok(a.end - b.end === 0 || a.end - b.end === 1, `end @${viewTop}/${viewH}/${exp}: ${a.end} vs ${b.end}`);
        assert.equal(b.padTop, top(p, b.start));
        assert.equal(b.padTop + (top(p, b.end) - top(p, b.start)) + b.padBottom, total(p), 'spacers + rows = full height');
      }
    }
  }
});

test('a viewport inside the expanded detail band keeps the expanded row mounted', () => {
  const detailH = 2000; // a tall detail row (logs/events), far more than the overscan
  const p: In = { count: 1000, rowH: 50, viewTop: 11 * 50 + 700, viewH: 400, overscan: 2, expanded: 10, detailH };
  const w = tableWindow(p);
  assert.ok(w.start <= 10 && w.end > 10, `expanded row 10 mounted: [${w.start}, ${w.end})`);
  assert.equal(w.padTop, w.start * 50);
  // The old math read y=1250 as row 25 and unmounted rows 0..22, the detail
  // with them, under a 1150 + 2000 px top spacer.
  const old = oldTableWindow(p);
  assert.ok(old.start > 10, 'the old window dropped the expanded row');
});

test('the expanded detail row shifts every later row by its height', () => {
  const detailH = 600;
  const w = tableWindow({ count: 1000, rowH: 50, viewTop: 11 * 50 + detailH, viewH: 100, overscan: 0, expanded: 10, detailH });
  assert.equal(w.start, 11);
  assert.equal(w.padTop, 11 * 50 + detailH);
  const above = tableWindow({ count: 1000, rowH: 50, viewTop: 20_000, viewH: 100, overscan: 0, expanded: 10, detailH });
  assert.equal(above.padTop, above.start * 50 + detailH);
});

test('the table starting below the fold and empty tables are safe', () => {
  const w = tableWindow({ count: 100, rowH: 50, viewTop: -300, viewH: 800, overscan: 2 });
  assert.equal(w.start, 0);
  assert.ok(w.end <= 100 && w.end > 0);
  assert.deepEqual({ ...tableWindow({ count: 0, rowH: 50, viewTop: 0, viewH: 800 }) }, { start: 0, end: 0, padTop: 0, padBottom: 0 });
  assert.deepEqual({ ...tableWindow({ count: 10, rowH: 0, viewTop: 0, viewH: 800 }) }, { start: 0, end: 0, padTop: 0, padBottom: 0 });
});

test('listRange without `extra` is unchanged (every existing caller)', () => {
  const r = lib.listRange(5000, 26, 2600, 800, 20, 300);
  assert.deepEqual({ ...r }, { start: 80, end: 151, top: 80 * 26, bottom: (5000 - 151) * 26 });
  assert.deepEqual({ ...lib.listRange(5000, 26, 2600, 800, 20, 300, { after: -1, h: 999 }) }, { ...r });
  assert.deepEqual({ ...lib.listRange(10, 26, 0, 800, 20, 300) }, { start: 0, end: 10, top: 0, bottom: 0 });
});
