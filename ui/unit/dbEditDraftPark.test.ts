import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';
import * as plural from '../src/lib/plural.ts';

// A half-typed cell edit used to vanish when the grid's view state was parked
// (switching to Structure, another tab or module): only the already-parked
// pending changes were saved. `parkDraft` turns a changed draft into one.
function flow() {
  const { EditFlow } = loadSource(new URL('../src/modules/database/EditFlow.svelte.ts', import.meta.url), {
    '../../lib/toast.svelte': { toasts: { info() {}, success() {}, error() {} } },
    './review-batch': {},
    '../../lib/stores/database.svelte': { database: {} },
    './results-format': { cellStr: (v: unknown) => String(v) },
    './edit-types': { adapterFor: () => ({}) },
    './edit-sql': { scopeDatabase: (n: string | null) => n }, './edit-mongo': {}, '../../lib/jsonTree/expansion-plan': {},
    './insert-flow.svelte': { InsertFlow: class { target = null; form = null; } },
    '../../lib/toastError': { toastError() {} }, '../../lib/plural': plural,
  });
  const f = new EditFlow();
  Object.assign(f, {
    editable: true, result: { columns: [{ name: 'id' }, { name: 'v' }] }, engine: 'mysql',
    statement: 'SELECT * FROM t', ranNode: 'app', editDb: 'app', editTable: 't', editPkCols: ['id'],
    connectionId: 'c1', liveRows: [[1, 'a'], [2, 'b']],
  });
  return f;
}

test('a changed in-progress cell edit is parked as a pending change', () => {
  const f = flow();
  f.beginEdit(1, 1);
  f.editing.value = 'b2';
  const pending = f.parkDraft();
  assert.equal(pending.get(1)?.cells.get(1), 'b2');
  assert.equal(f.editing, null);
});

test('an unchanged draft parks nothing', () => {
  const f = flow();
  f.beginEdit(0, 1);
  assert.equal(f.parkDraft().size, 0);
  assert.equal(f.editing, null);
});

test('a draft whose row is gone is dropped, never thrown on', () => {
  const f = flow();
  f.beginEdit(1, 1);
  f.editing.value = 'zz';
  f.liveRows = [[1, 'a']];
  assert.equal(f.parkDraft().size, 0);
  assert.equal(f.editing, null);
});

test('with no edit open the pending map is returned as is', () => {
  const f = flow();
  f.parkValue(0, 1, 'x');
  assert.equal(f.parkDraft().get(0)?.cells.get(1), 'x');
});
