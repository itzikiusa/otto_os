import { test } from 'node:test';
import assert from 'node:assert/strict';
import { carryPending, parkOpenEditors, pkIndexes, restorePending, rowKey, type CarryFlow } from '../src/modules/database/pending-carry.ts';

// Un-applied edits are keyed by liveRows INDEX. A new result for the same tab
// (a re-run, a refresh after a partial apply) used to drop every one of them,
// and an open inline editor kept pointing at an index that now holds another
// row. pending-carry re-keys them by the primary key across the re-seed.
const patch = (cells: [number, string][]) => ({ cells: new Map(cells), set: new Map(), unset: new Set<string>(), rename: new Map() });

function flow(rows: unknown[][], over: Partial<CarryFlow> = {}): CarryFlow & { parked: number; saved: number } {
  const f = {
    result: { columns: [{ name: 'id' }, { name: 'v' }] },
    liveRows: rows,
    editPkCols: ['id'],
    pending: new Map(),
    editing: null,
    viewer: null,
    viewerEditing: false,
    parked: 0,
    saved: 0,
    parkDraft() {
      this.parked++;
      const e = this.editing;
      if (e) this.pending = new Map(this.pending).set(e.rowIdx, patch([[e.colIdx, e.value]]));
      this.editing = null;
      return this.pending;
    },
    saveViewerEdit() {
      this.saved++;
      this.viewer = null;
      this.viewerEditing = false;
    },
    ...over,
  };
  return f as CarryFlow & { parked: number; saved: number };
}

test('a draft follows its row to the new index after a re-run', () => {
  const before = [[1, 'a'], [2, 'b'], [3, 'c']];
  const f = flow(before);
  f.pending = new Map([[1, patch([[1, 'B']])]]);
  const carried = carryPending(f, before);
  assert.ok(carried);
  // The new result put row id=2 at index 0 and dropped nothing.
  const after = [[2, 'b'], [3, 'c'], [1, 'a']];
  f.liveRows = after;
  assert.equal(restorePending(f, carried, after), 0);
  assert.deepEqual([...f.pending.keys()], [0]);
  assert.equal(f.pending.get(0)?.cells.get(1), 'B');
});

test('a draft whose row left the result is dropped and counted', () => {
  const before = [[1, 'a'], [2, 'b']];
  const f = flow(before);
  f.pending = new Map([[0, patch([[1, 'A']])], [1, patch([[1, 'B']])]]);
  const carried = carryPending(f, before)!;
  const after = [[1, 'a']];
  assert.equal(restorePending(f, carried, after), 1);
  assert.deepEqual([...f.pending.keys()], [0]);
  assert.equal(f.pending.get(0)?.cells.get(1), 'A');
});

test('an open inline editor is parked before the carry, never left on a stale index', () => {
  const before = [[1, 'a'], [2, 'b']];
  const f = flow(before, { editing: { rowIdx: 1, colIdx: 1, value: 'typed' } });
  const carried = carryPending(f, before)!;
  assert.equal(f.editing, null);
  const after = [[2, 'b'], [1, 'a']];
  restorePending(f, carried, after);
  assert.equal(f.pending.get(0)?.cells.get(1), 'typed');
});

test('an editor open on rows the flow no longer holds is closed, not committed', () => {
  const f = flow([[9, 'z']], { editing: { rowIdx: 0, colIdx: 1, value: 'x' } });
  assert.equal(carryPending(f, [[1, 'a']]), null);
  assert.equal(f.editing, null);
  assert.equal(f.parked, 0);
});

test('nothing is carried without a key or without drafts', () => {
  const rows = [[1, 'a']];
  assert.equal(carryPending(flow(rows), rows), null);
  const noKey = flow(rows, { editPkCols: ['missing'] });
  noKey.pending = new Map([[0, patch([[1, 'x']])]]);
  assert.equal(carryPending(noKey, rows), null);
});

test('a non-unique key never doubles a draft', () => {
  const before = [[1, 'a']];
  const f = flow(before);
  f.pending = new Map([[0, patch([[1, 'A']])]]);
  const carried = carryPending(f, before)!;
  restorePending(f, carried, [[1, 'a'], [1, 'a2']]);
  assert.equal(f.pending.size, 1);
});

test('composite and 64-bit keys match exactly', () => {
  assert.deepEqual(pkIndexes([{ name: 'a' }, { name: 'b' }], ['b', 'a']), [1, 0]);
  assert.equal(pkIndexes([{ name: 'a' }], []), null);
  assert.notEqual(rowKey(['9007199254740993'], [0]), rowKey(['9007199254740992'], [0]));
  assert.equal(rowKey([null, 1], [0, 1]), rowKey([undefined, 1], [0, 1]));
});

test('parking for a tab switch saves an open cell-viewer edit first', () => {
  const f = flow([[1, 'a']], { viewer: { edit: { rowIdx: 0, colIdx: 1 } }, viewerEditing: true });
  parkOpenEditors(f);
  assert.equal(f.saved, 1);
  assert.equal(f.parked, 1);
  const g = flow([[1, 'a']], { viewer: { edit: null }, viewerEditing: false });
  parkOpenEditors(g);
  assert.equal(g.saved, 0);
});
