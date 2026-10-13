import { test } from 'node:test';
import assert from 'node:assert/strict';
import { gridStep, type GridView } from '../src/lib/components/termGrid.ts';

const base: GridView = {
  differs: false,
  owner: 'self',
  owed: false,
  snapshotAsked: false,
  primary: true,
  readOnly: false,
  visible: true,
};

test('another window re-sizing the PTY makes the pane owe a rebuild and offers a take-over', () => {
  const s = gridStep({ ...base, differs: true, owner: 'other' });
  assert.deepEqual(s, { owed: true, rebuild: false, claim: false, foreign: true });
});

test('once the PTY is back at our grid and ours, ONE rebuild runs', () => {
  const s = gridStep({ ...base, owed: true });
  assert.deepEqual(s, { owed: false, rebuild: true, claim: false, foreign: false });
  // …and nothing more after it.
  assert.equal(gridStep({ ...base, owed: s.owed }).rebuild, false);
});

test('a snapshot already on its way is the rebuild (no second one on attach)', () => {
  const s = gridStep({ ...base, owed: true, snapshotAsked: true });
  assert.equal(s.rebuild, false);
  assert.equal(s.owed, true, 'still owed until that snapshot lands');
});

test('the other viewer leaving: the visible primary pane takes the grid back', () => {
  assert.equal(gridStep({ ...base, differs: true, owner: 'none' }).claim, true);
  assert.equal(gridStep({ ...base, differs: true, owner: 'none', visible: false }).claim, false, 'never from a hidden tab');
  assert.equal(gridStep({ ...base, differs: false, owner: 'none' }).claim, false, 'same grid: nothing to take');
});

test('tiles, previews and read-only viewers never claim or show the take-over', () => {
  for (const v of [{ primary: false }, { readOnly: true }]) {
    const s = gridStep({ ...base, ...v, differs: true, owner: 'none' });
    assert.equal(s.claim, false);
    assert.equal(gridStep({ ...base, ...v, differs: true, owner: 'other' }).foreign, false);
  }
  // A tile still owes a rebuild, so focusing it (which claims) repaints it.
  assert.equal(gridStep({ ...base, primary: false, differs: true, owner: 'other' }).owed, true);
});

test('our own pending local refit (we own the PTY) owes nothing', () => {
  assert.deepEqual(gridStep({ ...base, differs: true, owner: 'self' }), { owed: false, rebuild: false, claim: false, foreign: false });
});
