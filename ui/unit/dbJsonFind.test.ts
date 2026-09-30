import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

// ⌘F over the JSON / Vertical result views (src/modules/database/json-find.ts):
// the find model covers lines inside CLOSED branches, past "show more" slices
// and inside clipped strings, and a reveal opens exactly the path to a line.

const bson = loadSource(new URL('../src/modules/database/bson.ts', import.meta.url), {});
const jf = loadSource(new URL('../src/modules/database/json-find.ts', import.meta.url), { './bson': bson });

type Top = [string, string, unknown];
// The module runs in its own vm realm: compare its arrays structurally.
const plain = (v: unknown): unknown => JSON.parse(JSON.stringify(v));
const tops = (obj: Record<string, unknown>): Top[] => Object.entries(obj).map(([k, v]) => [k, k, v]);

const doc = {
  _id: { $oid: '64b7f0c2a1b2c3d4e5f60718' },
  brand_id: 7,
  lobby: {
    structure: Array.from({ length: 120 }, (_, i) => ({ games: [{ name: i === 87 ? 'Book of Ra' : `game-${i}` }] })),
  },
  note: 'x'.repeat(250) + 'needle',
  empty: [],
  missing: null,
};

test('lines cover keys and values, including collapsed and past-chunk branches', () => {
  const lines = jf.recordLines(tops(doc), 'json');
  assert.equal(lines.length, jf.countLines(tops(doc)));
  const hit = lines.find((l: { text: string }) => l.text.includes('Book of Ra'));
  assert.equal(hit.path, 'lobby.structure.87.games.0.name');
  assert.equal(hit.text, 'name\n"Book of Ra"');
  assert.ok(lines.some((l: { text: string }) => l.text === 'brand_id\n7'));
  assert.ok(lines.some((l: { text: string }) => l.text === '_id\nObjectId("64b7f0c2a1b2c3d4e5f60718")'));
  assert.ok(lines.some((l: { text: string }) => l.text === 'empty\n[]'));
  assert.ok(lines.some((l: { text: string }) => l.text === 'missing\nnull'));
  // Array-index labels are not searchable: `structure.87` is an empty line.
  assert.equal(lines.find((l: { path: string }) => l.path === 'lobby.structure.87').text, '');
  const note = lines.find((l: { path: string }) => l.path === 'note');
  assert.ok(note.long && note.text.endsWith('needle"'));
});

test('vertical flavor draws raw strings and ∅', () => {
  const lines = jf.recordLines([['c', 'c', null], ['s', 's', 'hi']], 'vertical');
  assert.deepEqual(plain(lines.map((l: { text: string }) => l.text)), ['c\n∅', 's\nhi']);
});

test('LineIndex maps flat rows across records (empty records included) and caches lazily', () => {
  const recs: Record<string, unknown>[] = [{ a: 1 }, {}, { b: { c: 2 } }];
  let built = 0;
  const idx = new jf.LineIndex(recs.length, (i: number) => (built++, tops(recs[i])), 'json');
  assert.equal(idx.count(), 3);
  assert.deepEqual([0, 1, 2].map((i) => [idx.locate(i).rec, idx.locate(i).line.path]), [
    [0, 'a'],
    [2, 'b'],
    [2, 'b.c'],
  ]);
  assert.equal(idx.locate(3), null);
  assert.ok(built <= 6);
});

test('reveal opens every ancestor and grows the slices it sits past', () => {
  const t = tops(doc);
  const line = jf.recordLines(t, 'json').find((l: { path: string }) => l.path === 'lobby.structure.87.games.0.name');
  const steps = jf.revealSteps(t, line, 50, true);
  assert.deepEqual(plain(steps.opens), ['', 'lobby', 'lobby.structure', 'lobby.structure.87', 'lobby.structure.87.games', 'lobby.structure.87.games.0']);
  assert.deepEqual(plain(steps.shown), [['lobby.structure', 100]]);
  // Vertical: no chunked record root.
  assert.deepEqual(jf.revealSteps(t, line, 50, false).opens[0], 'lobby');
  // A top-level field past the root's first slice (JSON view).
  const wide = tops(Object.fromEntries(Array.from({ length: 60 }, (_, i) => [`f${i}`, i])));
  const last = jf.recordLines(wide, 'json')[55];
  assert.deepEqual(plain(jf.revealSteps(wide, last, 50, true).shown), [['', 100]]);
});

test('LineIndex.text: lower-cased, cached per record across searches, budget spills to a rolling cache', () => {
  const recs: Record<string, unknown>[] = [{ Name: 'Book Of Ra' }, { b: { C: 'X' } }, { d: 1 }];
  let built = 0;
  const idx = new jf.LineIndex(recs.length, (i: number) => (built++, tops(recs[i])), 'json');
  const scan = () => Array.from({ length: idx.count() }, (_, i) => idx.text(i));
  assert.deepEqual(plain(scan()), ['name\n"book of ra"', 'b', 'c\n"x"', 'd\n1']);
  const afterFirst = built;
  scan(); // the next keystroke re-scans cached texts: nothing is rebuilt
  assert.equal(built, afterFirst);
  assert.equal(idx.cachedRecords(), 3);

  const saved = jf.LineIndex.TEXT_BUDGET;
  jf.LineIndex.TEXT_BUDGET = 17; // room for record 0 (17 chars) only
  try {
    const small = new jf.LineIndex(recs.length, (i: number) => tops(recs[i]), 'json');
    const texts = Array.from({ length: small.count() }, (_, i) => small.text(i));
    assert.equal(texts[3], 'd\n1', 'past the budget texts still come back');
    assert.equal(small.cachedRecords(), 1);
  } finally {
    jf.LineIndex.TEXT_BUDGET = saved;
  }
});
