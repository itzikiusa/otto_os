import { test } from 'node:test';
import assert from 'node:assert/strict';
import { planSection } from '../src/modules/git/ref-budget.ts';

const leaves = (n: number) => Array.from({ length: n }, (_, i) => i);
const mounted = (p: ReturnType<typeof planSection<number>>) =>
  p.loose + p.folders.reduce((a, f) => a + 1 + f.shown, 0);

test('the budget is per section, not per folder (2k branches in 40 folders)', () => {
  const folders = Array.from({ length: 40 }, (_, i) => ({ name: `team${i}`, leaves: leaves(50) }));
  const p = planSection(leaves(10), folders, 150, () => false);
  assert.ok(mounted(p) <= 150, `mounted ${mounted(p)} rows`);
  assert.equal(p.loose, 10);
  assert.equal(p.hidden, 2000 - (mounted(p) - 10 - p.folders.length));
});

test('collapsed folders cost their header only and hide nothing', () => {
  const folders = [
    { name: 'a', leaves: leaves(100) },
    { name: 'b', leaves: leaves(100) },
  ];
  const p = planSection([], folders, 150, (n) => n === 'a');
  assert.deepEqual(
    p.folders.map((f) => [f.folder.name, f.shown]),
    [['a', 0], ['b', 100]],
  );
  assert.equal(p.hidden, 0);
});

test('a small section mounts everything', () => {
  const p = planSection(leaves(3), [{ name: 'f', leaves: leaves(4) }], 150, () => false);
  assert.equal(mounted(p), 3 + 1 + 4);
  assert.equal(p.hidden, 0);
});

test('growing the budget reveals more rows', () => {
  const folders = [{ name: 'f', leaves: leaves(1000) }];
  const a = planSection([], folders, 150, () => false);
  const b = planSection([], folders, 750, () => false);
  assert.equal(a.folders[0].shown, 149);
  assert.equal(b.folders[0].shown, 749);
  assert.equal(b.hidden, 251);
});
