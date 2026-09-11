// Pure split-layout tree ops (node:test, Node's built-in type stripping — the
// `.ts` extension on the import is required in strip mode).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  MAX_PANES,
  FRAC_MIN,
  FRAC_MAX,
  leaf,
  split,
  leaves,
  findLeaf,
  parentOf,
  insertBeside,
  removeLeaf,
  swapLeaves,
  moveLeaf,
  setFrac,
  replaceSession,
  retain,
  applyPreset,
  fromLegacy,
  toLegacy,
  parseLayout,
  applyTileOrder,
  reorderIds,
  neighbour,
  type LayoutNode,
  type Leaf,
  type Split,
  type Rect,
} from '../src/lib/stores/splitLayout.ts';

const sessions = (t: LayoutNode | null): string[] => leaves(t).map((l) => l.session);
const asSplit = (t: LayoutNode | null): Split => {
  assert.ok(t && t.kind === 'split', 'expected a split node');
  return t;
};
const asLeaf = (t: LayoutNode | null): Leaf => {
  assert.ok(t && t.kind === 'leaf', 'expected a leaf node');
  return t;
};
/** Effective share of every leaf = product of the fractions down its path. */
function shares(t: LayoutNode, axis: 'col' | 'row', acc = 1, out = new Map<string, number>()): Map<string, number> {
  if (t.kind === 'leaf') {
    out.set(t.key, acc);
    return out;
  }
  if (t.axis !== axis) {
    shares(t.a, axis, acc, out);
    shares(t.b, axis, acc, out);
    return out;
  }
  shares(t.a, axis, acc * t.frac, out);
  shares(t.b, axis, acc * (1 - t.frac), out);
  return out;
}
const depth = (t: LayoutNode): number => (t.kind === 'leaf' ? 0 : 1 + Math.max(depth(t.a), depth(t.b)));
const ids = (n: number): string[] => Array.from({ length: n }, (_, i) => `s${i + 1}`);

test('MAX_PANES is the user decision (15, not 6)', () => {
  assert.equal(MAX_PANES, 15);
});

test('insertBeside: left/up put node in a, right/down in b, axis col vs row', () => {
  const A = leaf('A');
  const B = leaf('B');
  for (const [side, axis, first] of [
    ['left', 'col', 'B'],
    ['right', 'col', 'A'],
    ['up', 'row', 'B'],
    ['down', 'row', 'A'],
  ] as const) {
    const t = asSplit(insertBeside(A, A.key, B, side));
    assert.equal(t.axis, axis, side);
    assert.equal(asLeaf(t.a).session, first, side);
    assert.deepEqual(sessions(t).sort(), ['A', 'B']);
  }
  // Unknown target ⇒ unchanged (same object).
  assert.equal(insertBeside(A, 'nope', B, 'left'), A);
});

test('removeLeaf collapses the parent and keeps the sibling subtree key and frac', () => {
  const A = leaf('A');
  const B = leaf('B');
  const C = leaf('C');
  const inner = split('row', B, C, 0.3);
  const root = split('col', A, inner, 0.6);
  const after = asSplit(removeLeaf(root, A.key));
  assert.equal(after.key, inner.key);
  assert.equal(after.frac, 0.3);
  assert.deepEqual(sessions(after), ['B', 'C']);
  // Removing from the inner split keeps the root and collapses the inner into its sibling.
  const after2 = asSplit(removeLeaf(root, B.key));
  assert.equal(after2.key, root.key);
  assert.equal(asLeaf(after2.b).key, C.key);
  // Unknown key ⇒ identity; the last leaf ⇒ null.
  assert.equal(removeLeaf(root, 'nope'), root);
  assert.equal(removeLeaf(A, A.key), null);
});

test('swapLeaves swaps sessions, not keys', () => {
  const A = leaf('A');
  const B = leaf('B');
  const C = leaf('C');
  const root = split('col', A, split('row', B, C));
  const out = swapLeaves(root, A.key, C.key);
  assert.deepEqual(leaves(out).map((l) => l.key), [A.key, B.key, C.key]);
  assert.deepEqual(sessions(out), ['C', 'B', 'A']);
  assert.equal(swapLeaves(root, A.key, A.key), root);
  assert.equal(swapLeaves(root, A.key, 'nope'), root);
});

test('moveLeaf same key / unknown target is identity', () => {
  const A = leaf('A');
  const B = leaf('B');
  const root = split('col', A, B);
  assert.equal(moveLeaf(root, A.key, A.key, 'left'), root);
  assert.equal(moveLeaf(root, A.key, 'nope', 'left'), root);
  assert.equal(moveLeaf(root, 'nope', B.key, 'left'), root);
  assert.equal(moveLeaf(A, A.key, A.key, 'right'), A);
});

test('moveLeaf removes then inserts (3 leaves stay 3, new position)', () => {
  const A = leaf('A');
  const B = leaf('B');
  const C = leaf('C');
  const root = split('col', A, split('col', B, C));
  const out = moveLeaf(root, A.key, C.key, 'down');
  assert.equal(leaves(out).length, 3);
  assert.deepEqual(sessions(out), ['B', 'C', 'A']);
  const p = parentOf(out, A.key);
  assert.ok(p && p.axis === 'row' && asLeaf(p.a).key === C.key && asLeaf(p.b).key === A.key);
  assert.equal(findLeaf(out, A.key)?.key, A.key, 'the moved leaf keeps its key');
});

test('setFrac clamps to 0.1 and 0.9', () => {
  const root = split('col', leaf('A'), leaf('B'));
  assert.equal(asSplit(setFrac(root, root.key, 0)).frac, FRAC_MIN);
  assert.equal(asSplit(setFrac(root, root.key, 1)).frac, FRAC_MAX);
  assert.equal(asSplit(setFrac(root, root.key, 0.42)).frac, 0.42);
  assert.equal(asSplit(setFrac(root, root.key, NaN)).frac, 0.5);
  assert.deepEqual(setFrac(root, 'nope', 0.2), root, 'unknown key ⇒ structurally unchanged');
});

test('fromLegacy n=1..4 shapes and fractions', () => {
  assert.equal(fromLegacy({ panes: [] }, 0.3, 0.7), null);
  assert.equal(asLeaf(fromLegacy({ panes: ['A'] }, 0.3, 0.7)).session, 'A');
  const two = asSplit(fromLegacy({ panes: ['A', 'B'], axis: 'row' }, 0.3, 0.7));
  assert.equal(two.axis, 'row');
  assert.equal(two.frac, 0.7);
  const twoCol = asSplit(fromLegacy({ panes: ['A', 'B'] }, 0.3, 0.7));
  assert.equal(twoCol.axis, 'col');
  assert.equal(twoCol.frac, 0.3);
  const three = asSplit(fromLegacy({ panes: ['A', 'B', 'C'], axis: 'col' }, 0.3, 0.7));
  assert.equal(three.axis, 'row');
  assert.equal(three.frac, 0.7);
  const top = asSplit(three.a);
  assert.equal(top.axis, 'col');
  assert.equal(top.frac, 0.3);
  assert.deepEqual(sessions(top), ['A', 'B']);
  assert.equal(asLeaf(three.b).session, 'C');
  const four = asSplit(fromLegacy({ panes: ['A', 'B', 'C', 'D', 'E'] }, 0.3, 0.7));
  assert.equal(four.axis, 'row');
  assert.deepEqual(sessions(four.a), ['A', 'B']);
  assert.deepEqual(sessions(four.b), ['C', 'D']);
  assert.equal(asSplit(four.b).frac, 0.3);
});

test('toLegacy(fromLegacy(x)) round-trips panes+axis for n=1,2 and panes for n=3,4', () => {
  for (const n of [1, 2]) {
    for (const axis of ['col', 'row'] as const) {
      const v1 = { panes: ids(n), axis };
      const back = toLegacy(fromLegacy(v1, 0.4, 0.6));
      assert.deepEqual(back.panes, v1.panes);
      if (n === 2) assert.equal(back.axis, axis);
    }
  }
  for (const n of [3, 4]) {
    assert.deepEqual(toLegacy(fromLegacy({ panes: ids(n), axis: 'col' }, 0.4, 0.6)).panes, ids(n));
  }
  assert.deepEqual(toLegacy(null), { panes: [], axis: 'col' });
});

test('parseLayout: null → null tree; garbage → null', () => {
  const ok = () => true;
  assert.deepEqual(parseLayout(null, ok, 0.5, 0.5), { tree: null, focused: null });
  assert.deepEqual(parseLayout('{not json', ok, 0.5, 0.5), { tree: null, focused: null });
  assert.deepEqual(parseLayout('42', ok, 0.5, 0.5), { tree: null, focused: null });
  assert.deepEqual(parseLayout('{"v":2,"tree":{"kind":"blob"}}', ok, 0.5, 0.5), { tree: null, focused: null });
});

test('parseLayout: v1 → fromLegacy', () => {
  const r = parseLayout(JSON.stringify({ panes: ['A', 'B', 'dead', 'C'], axis: 'col' }), (id) => id !== 'dead', 0.3, 0.7);
  const root = asSplit(r.tree);
  assert.equal(root.axis, 'row');
  assert.equal(root.frac, 0.7);
  assert.deepEqual(sessions(root), ['A', 'B', 'C']);
  assert.equal(asSplit(root.a).frac, 0.3);
  assert.equal(r.focused, null);
});

test('parseLayout: v2 with a dead leaf drops it and collapses', () => {
  const A = leaf('A');
  const B = leaf('B');
  const C = leaf('C');
  const inner = split('row', B, C, 0.25);
  const root = split('col', A, inner, 0.6);
  const raw = JSON.stringify({ v: 2, tree: root, focused: C.key });
  const r = parseLayout(raw, (id) => id !== 'B', 0.5, 0.5);
  const t = asSplit(r.tree);
  assert.equal(t.key, root.key);
  assert.equal(t.frac, 0.6);
  assert.equal(asLeaf(t.b).key, C.key, 'the inner split collapsed into C');
  assert.equal(r.focused, C.key);
  // Fractions out of range are clamped, non-string keys regenerated.
  const raw2 = JSON.stringify({ v: 2, tree: { kind: 'split', axis: 'col', frac: 5, a: { kind: 'leaf', session: 'A' }, b: { kind: 'leaf', key: 7, session: 'B' } } });
  const t2 = asSplit(parseLayout(raw2, () => true, 0.5, 0.5).tree);
  assert.equal(t2.frac, FRAC_MAX);
  assert.equal(typeof t2.key, 'string');
  assert.equal(typeof asLeaf(t2.b).key, 'string');
});

test('parseLayout: v2 with a missing focused → null focused', () => {
  const A = leaf('A');
  const B = leaf('B');
  const r = parseLayout(JSON.stringify({ v: 2, tree: split('col', A, B), focused: 'gone' }), () => true, 0.5, 0.5);
  assert.equal(r.focused, null);
  assert.deepEqual(sessions(r.tree), ['A', 'B']);
  const r2 = parseLayout(JSON.stringify({ v: 2, tree: split('col', A, B), focused: B.key }), () => true, 0.5, 0.5);
  assert.equal(r2.focused, B.key);
});

test('parseLayout: leaves beyond MAX_PANES are trimmed', () => {
  const tree = applyPreset(ids(MAX_PANES + 3), 'cols');
  const r = parseLayout(JSON.stringify({ v: 2, tree, focused: null }), () => true, 0.5, 0.5);
  assert.equal(leaves(r.tree).length, MAX_PANES);
  assert.deepEqual(sessions(r.tree), ids(MAX_PANES));
});

test('applyPreset cols/rows for n=2..15: every leaf share is 1/n and depth ≤ ⌈log₂ n⌉', () => {
  for (let n = 2; n <= MAX_PANES; n++) {
    for (const [preset, axis] of [
      ['cols', 'col'],
      ['rows', 'row'],
    ] as const) {
      const t = applyPreset(ids(n), preset);
      assert.ok(t);
      assert.deepEqual(sessions(t), ids(n), `${preset} n=${n} keeps order`);
      for (const [, share] of shares(t, axis)) assert.ok(Math.abs(share - 1 / n) < 1e-9, `${preset} n=${n} share ${share}`);
      assert.ok(depth(t) <= Math.ceil(Math.log2(n)), `${preset} n=${n} depth ${depth(t)}`);
      // Balanced halving never produces a fraction that clampFrac would have altered.
      const walk = (x: LayoutNode): void => {
        if (x.kind === 'leaf') return;
        assert.ok(x.frac >= 1 / 3 - 1e-9 && x.frac <= 2 / 3 + 1e-9, `${preset} n=${n} frac ${x.frac}`);
        walk(x.a);
        walk(x.b);
      };
      walk(t);
    }
  }
  assert.equal(applyPreset([], 'cols'), null);
  assert.equal(asLeaf(applyPreset(['A'], 'grid')).session, 'A');
});

test('applyPreset one-two-below / one-two-beside shapes', () => {
  const below = asSplit(applyPreset(ids(3), 'one-two-below'));
  assert.equal(below.axis, 'row');
  assert.equal(asLeaf(below.a).session, 's1');
  assert.equal(asSplit(below.b).axis, 'col');
  const beside = asSplit(applyPreset(ids(3), 'one-two-beside'));
  assert.equal(beside.axis, 'col');
  assert.equal(asLeaf(beside.a).session, 's1');
  assert.equal(asSplit(beside.b).axis, 'row');
});

test('applyPreset grid column count 2/3/4 at n=4/9/15', () => {
  const colsOf = (t: LayoutNode): number => {
    // Every row is a `col` chain (or a leaf); the first row's leaf count is the column count.
    let row = t;
    while (row.kind === 'split' && row.axis === 'row') row = row.a;
    return leaves(row).length;
  };
  assert.equal(colsOf(applyPreset(ids(4), 'grid')!), 2);
  assert.equal(colsOf(applyPreset(ids(9), 'grid')!), 3);
  assert.equal(colsOf(applyPreset(ids(15), 'grid')!), 4);
  assert.deepEqual(sessions(applyPreset(ids(15), 'grid')), ids(15));
});

test('replaceSession maps every matching leaf, removes on null, dedupes a replaced leaf whose target already exists', () => {
  const A1 = leaf('A');
  const A2 = leaf('A');
  const B = leaf('B');
  const root = split('col', A1, split('row', A2, B));
  assert.deepEqual(sessions(replaceSession(root, 'A', 'C')), ['C', 'B'], 'second replaced leaf dedupes against the first');
  assert.deepEqual(sessions(replaceSession(root, 'A', 'B')), ['B'], 'target already on screen ⇒ the replaced leaves vanish');
  assert.deepEqual(sessions(replaceSession(root, 'A', null)), ['B']);
  assert.equal(replaceSession(root, 'Z', 'C'), root, 'no match ⇒ identity');
  assert.equal(replaceSession(null, 'A', 'B'), null);
  const one = asLeaf(replaceSession(A1, 'A', 'B'));
  assert.equal(one.key, A1.key);
  assert.equal(one.session, 'B');
});

test('retain returns the same object when nothing changes', () => {
  const root = split('col', leaf('A'), leaf('B'));
  assert.equal(retain(root, () => true), root);
  assert.deepEqual(sessions(retain(root, (id) => id === 'A')), ['A']);
  assert.equal(retain(root, () => false), null);
  assert.equal(retain(null, () => true), null);
});

test('applyTileOrder: known first in order, unknown appended, dead ids skipped', () => {
  const S = ids(4).map((id) => ({ id }));
  const out = applyTileOrder(S, ['s3', 'dead', 's1', 's3']);
  assert.deepEqual(out.map((s) => s.id), ['s3', 's1', 's2', 's4']);
  assert.deepEqual(applyTileOrder(S, []).map((s) => s.id), ids(4));
});

test('reorderIds pulls from and reinserts at to; unknown ids unchanged', () => {
  const list = ids(4);
  assert.deepEqual(reorderIds(list, 's4', 's1'), ['s4', 's1', 's2', 's3']);
  assert.deepEqual(reorderIds(list, 's1', 's3'), ['s2', 's3', 's1', 's4']);
  assert.equal(reorderIds(list, 's1', 's1'), list);
  assert.equal(reorderIds(list, 'x', 's1'), list);
  assert.equal(reorderIds(list, 's1', 'x'), list);
});

test('neighbour on a 2×2 rect map (right/left/down/up + null at the edge) and cross-axis overlap preference', () => {
  const R = (left: number, top: number, width: number, height: number): Rect => ({ left, top, width, height });
  const grid = new Map<string, Rect>([
    ['tl', R(0, 0, 100, 100)],
    ['tr', R(100, 0, 100, 100)],
    ['bl', R(0, 100, 100, 100)],
    ['br', R(100, 100, 100, 100)],
  ]);
  assert.equal(neighbour(grid, 'tl', 'right'), 'tr');
  assert.equal(neighbour(grid, 'tr', 'left'), 'tl');
  assert.equal(neighbour(grid, 'tl', 'down'), 'bl');
  assert.equal(neighbour(grid, 'bl', 'up'), 'tl');
  assert.equal(neighbour(grid, 'tl', 'left'), null);
  assert.equal(neighbour(grid, 'tl', 'up'), null);
  assert.equal(neighbour(grid, 'br', 'right'), null);
  assert.equal(neighbour(grid, 'nope', 'right'), null);
  // Tall left column beside a two-row right column: moving right from the
  // column picks the row you are level with, not the nearer centre.
  const tall = new Map<string, Rect>([
    ['left', R(0, 0, 100, 200)],
    ['rt', R(100, 0, 100, 100)],
    ['rb', R(100, 100, 100, 100)],
    ['far', R(200, 300, 100, 100)],
  ]);
  const out = neighbour(tall, 'left', 'right');
  assert.ok(out === 'rt' || out === 'rb');
  assert.notEqual(out, 'far');
  // Cross-axis overlap wins over a closer centre that does not overlap.
  const skew = new Map<string, Rect>([
    ['src', R(0, 0, 100, 50)],
    ['near-no-overlap', R(110, 60, 100, 50)],
    ['far-overlap', R(300, 0, 100, 50)],
  ]);
  assert.equal(neighbour(skew, 'src', 'right'), 'far-overlap');
});
