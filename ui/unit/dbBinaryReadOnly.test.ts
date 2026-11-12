import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';
import * as plural from '../src/lib/plural.ts';

// A binary column's cells are base64 TEXT on the wire; a text box over them
// would write the base64 instead of the bytes. Every way a cell draft gets
// parked — the inline editor (beginEdit → commitEdit), the cell viewer's Save
// (commitEdit), the menu's Set NULL / Set empty (parkValue) and Vertical path
// ops (canEditPath) — must refuse a binary column.
function flow() {
  const { EditFlow } = loadSource(new URL('../src/modules/database/EditFlow.svelte.ts', import.meta.url), {
    '../../lib/toast.svelte': { toasts: { info() {}, success() {}, error() {} } },
    './review-batch': {},
    '../../lib/stores/database.svelte': { database: {} },
    './results-format': { cellStr: (v: unknown) => String(v), SET_NULL: '\u0000otto:set-null', SET_EMPTY: '\u0000otto:set-empty' },
    './edit-types': { adapterFor: () => ({}) },
    './edit-sql': { scopeDatabase: (n: string | null) => n }, './edit-mongo': {}, '../../lib/jsonTree/expansion-plan': {},
    './insert-flow.svelte': { InsertFlow: class { target = null; form = null; } },
    '../../lib/toastError': { toastError() {} }, '../../lib/plural': plural,
  });
  const f = new EditFlow();
  Object.assign(f, {
    editable: true,
    result: { columns: [{ name: 'id' }, { name: 'name' }, { name: 'body', type_hint: 'BLOB', binary: true }] },
    engine: 'mysql', statement: 'SELECT * FROM t', ranNode: 'app', editDb: 'app', editTable: 't', editPkCols: ['id'],
    connectionId: 'c1', liveRows: [[1, 'a', 'AAEC/w==']],
  });
  return f;
}

test('a binary column is not editable; a text column of the same result is', () => {
  const f = flow();
  assert.equal(f.isEditableCell(1), true);
  assert.equal(f.isEditableCell(2), false);
  assert.equal(f.isEditableCell(0), false, 'the key stays read-only too');
});

test('the inline editor does not open on a binary cell', () => {
  const f = flow();
  f.beginEdit(0, 2);
  assert.equal(f.editing, null);
  f.beginEdit(0, 1);
  assert.ok(f.editing, 'the text column still opens');
});

test('a draft forced onto a binary cell is never parked (commitEdit / viewer Save)', () => {
  const f = flow();
  f.editing = { rowIdx: 0, colIdx: 2, value: 'not the bytes' };
  f.commitEdit();
  assert.equal(f.pending.size, 0);
  assert.equal(f.editing, null);
});

test('Set NULL / Set empty (parkValue) refuse a binary cell', () => {
  const f = flow();
  f.parkValue(0, 2, '\u0000otto:set-null');
  assert.equal(f.pending.size, 0);
  f.parkValue(0, 1, '\u0000otto:set-null');
  assert.equal(f.pending.get(0)?.cells.get(1), '\u0000otto:set-null');
});

test('the cell viewer opens a binary cell read-only, as bytes', () => {
  const f = flow();
  f.openCell('AAEC/w==', 0, 2);
  assert.equal(f.viewer?.binary, true);
  assert.equal(f.viewer?.edit, null);
});
