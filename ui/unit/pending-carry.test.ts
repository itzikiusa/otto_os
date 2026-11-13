import { test } from 'node:test';
import assert from 'node:assert/strict';
import { carryOrDrop, carryScope, restorePending, sameQueryText, type CarryFlow } from '../src/modules/database/pending-carry.ts';

// Un-applied edits follow their row (by primary key) to a new result of the
// same tab — but only when the new result came from the SAME query, scope node
// and connection. Otherwise a draft made on `users_stg` row 5 would land on
// `users` row 5, because the edit's target table is resolved from the new
// statement. The grid drops them and says so instead.
const patch = (cells: [number, string][]) => ({ cells: new Map(cells), set: new Map(), unset: new Set<string>(), rename: new Map() });

function flow(rows: unknown[][]): CarryFlow {
  return {
    result: { columns: [{ name: 'id' }, { name: 'email' }] },
    liveRows: rows,
    editPkCols: ['id'],
    pending: new Map([[1, patch([[1, 'new@x']])]]),
    editing: null,
    viewer: null,
    viewerEditing: false,
    parkDraft() {
      return this.pending;
    },
    saveViewerEdit() {},
  };
}

const rows = [[4, 'a@x'], [5, 'b@x']];

/** Run the grid's decision for a new result with the same columns. */
function rerun(before: string, after: string): { dropped: number; restored: number; f: CarryFlow } {
  const f = flow(rows);
  const moved = carryOrDrop(f, rows, before === after);
  const next = [[5, 'b@x'], [4, 'a@x']];
  f.pending = new Map();
  f.liveRows = next;
  const lost = moved.carry ? restorePending(f, moved.carry, next) : 0;
  return { dropped: moved.dropped + lost, restored: f.pending.size, f };
}

test('same columns but a different statement carries nothing and counts the drop', () => {
  const a = carryScope('SELECT id,email FROM users_stg', 'app', 'c1');
  const b = carryScope('SELECT id,email FROM users', 'app', 'c1');
  assert.notEqual(a, b);
  const r = rerun(a, b);
  assert.equal(r.restored, 0);
  assert.equal(r.dropped, 1);
});

test('a different connection or scope node carries nothing', () => {
  const sql = 'SELECT id,email FROM users';
  assert.notEqual(carryScope(sql, 'app', 'c1'), carryScope(sql, 'app', 'c2'));
  assert.notEqual(carryScope(sql, 'app', 'c1'), carryScope(sql, 'app_stg', 'c1'));
  assert.notEqual(carryScope(sql, 'app', 'c1'), carryScope(sql, null, 'c1'));
  const r = rerun(carryScope(sql, 'app', 'c1'), carryScope(sql, 'app_stg', 'c1'));
  assert.equal(r.restored, 0);
  assert.equal(r.dropped, 1);
});

test('an unchanged statement after a plain refresh still carries the draft onto its row', () => {
  const s = carryScope('SELECT id,email FROM users', 'app', 'c1');
  const r = rerun(s, carryScope('SELECT id,email FROM users', 'app', 'c1'));
  assert.equal(r.dropped, 0);
  // Row id=5 moved from index 1 to index 0; the draft followed it.
  assert.deepEqual([...r.f.pending.keys()], [0]);
  assert.equal(r.f.pending.get(0)?.cells.get(1), 'new@x');
});

test('whitespace-only and trailing-semicolon differences still carry', () => {
  const a = carryScope('SELECT id, email\n  FROM users', 'app', 'c1');
  const b = carryScope('  SELECT id,  email FROM\tusers ;', 'app', 'c1');
  // `id, email` vs `id,  email`: only the run length differs.
  assert.equal(a, b);
  assert.equal(rerun(a, b).restored, 1);
});

test('identifiers, literals and the spaces inside a literal are compared exactly', () => {
  assert.notEqual(sameQueryText('SELECT * FROM Users'), sameQueryText('SELECT * FROM users'));
  assert.notEqual(sameQueryText("SELECT * FROM t WHERE n = 'a  b'"), sameQueryText("SELECT * FROM t WHERE n = 'a b'"));
  assert.notEqual(sameQueryText('SELECT * FROM t WHERE id = 5'), sameQueryText('SELECT * FROM t WHERE id = 6'));
  // An escaped quote does not end the literal early.
  assert.equal(sameQueryText("SELECT 'it\\'s  x'  ,1"), "SELECT 'it\\'s  x' ,1");
});

test('a dropped set parks the open inline editor first so it is counted too', () => {
  const f = flow(rows);
  f.pending = new Map();
  f.editing = { rowIdx: 0, colIdx: 1, value: 'typed' };
  f.parkDraft = function () {
    const e = this.editing!;
    this.pending = new Map([[e.rowIdx, patch([[e.colIdx, e.value]])]]);
    this.editing = null;
    return this.pending;
  };
  const moved = carryOrDrop(f, rows, false);
  assert.equal(moved.carry, null);
  assert.equal(moved.dropped, 1);
});
