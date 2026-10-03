import { test } from 'node:test';
import assert from 'node:assert/strict';
import { CELL_MAX, previewJson } from '../src/modules/database/json-preview.ts';

const clip = (s: string): string => (s.length > CELL_MAX ? s.slice(0, CELL_MAX) + '…' : s);

const samples: unknown[] = [
  {},
  [],
  { a: 1, b: 'x', c: null, d: true, e: [1, 2, { f: 'g' }], h: {} },
  [1, 'two', null, false, [], {}, { k: [3] }],
  { skip: undefined, fn: () => 1, keep: 1, arr: [undefined, () => 2] },
  { quote: 'he said "hi"\n\ttab   é 😀', n: -0.5, big: 1e21, nan: NaN, inf: Infinity },
  { $oid: '64b7f0c2a1b2c3d4e5f60718', nested: { deep: { deeper: { deepest: [1, [2, [3]]] } } } },
  { when: new Date(0) },
];

test('short values match JSON.stringify exactly (compact and pretty)', () => {
  for (const v of samples) {
    assert.equal(previewJson(v), JSON.stringify(v), JSON.stringify(v));
    assert.equal(previewJson(v, CELL_MAX, true), JSON.stringify(v, null, 2));
  }
});

test('long values render the same clipped cell text as a full stringify', () => {
  const doc = {
    _id: 'x'.repeat(40),
    items: Array.from({ length: 2000 }, (_, i) => ({ i, name: `item-${i}`, tags: ['a', 'b'] })),
    blob: 'y'.repeat(100_000),
  };
  assert.equal(clip(previewJson(doc)), clip(JSON.stringify(doc)));
  assert.equal(clip(previewJson(doc, CELL_MAX, true)), clip(JSON.stringify(doc, null, 2)));
  const longString = ['z'.repeat(5000)];
  assert.equal(clip(previewJson(longString)), clip(JSON.stringify(longString)));
});

test('a preview never grows past max + 1 characters', () => {
  const doc = { rows: Array.from({ length: 50_000 }, (_, i) => ({ i })) };
  const p = previewJson(doc, 100);
  assert.equal(p.length, 101);
  assert.equal(p, JSON.stringify(doc).slice(0, 101));
});

test('bounded work: a 50 KB document previews far faster than it serializes', () => {
  const docs = Array.from({ length: 40 }, (_, d) => ({
    d,
    body: Array.from({ length: 600 }, (_, i) => ({ i, text: `line ${i} of doc ${d}`, ok: i % 2 === 0 })),
  }));
  assert.ok(JSON.stringify(docs[0]).length > 20_000);
  // Warm both paths, then compare 40 cells' worth of work.
  previewJson({ warm: 1 }, 64);
  JSON.stringify(docs[0]);
  // Best of 7 rounds each: a single ~1 ms sample loses to scheduler noise when
  // the suite runs in parallel on a loaded machine.
  const best = (fn: () => void): number => {
    let min = Infinity;
    for (let r = 0; r < 7; r++) {
      const t = performance.now();
      fn();
      min = Math.min(min, performance.now() - t);
    }
    return min;
  };
  const bounded = best(() => {
    for (const doc of docs) previewJson(doc, CELL_MAX - 1); // uncached max
  });
  const full = best(() => {
    for (const doc of docs) JSON.stringify(doc);
  });
  assert.ok(bounded < full, `bounded ${bounded.toFixed(2)} ms vs full ${full.toFixed(2)} ms`);
});

test('the default-size preview is cached per object', () => {
  const doc = { a: 'b'.repeat(2000) };
  const first = previewJson(doc);
  assert.equal(previewJson(doc), first);
  // A mutation is NOT seen (results are immutable snapshots) — the cache is by identity.
  (doc as { a: string }).a = 'changed';
  assert.equal(previewJson(doc), first);
  assert.notEqual(previewJson({ a: 'changed' }), first);
});
