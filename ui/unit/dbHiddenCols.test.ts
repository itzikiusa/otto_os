import { test } from 'node:test';
import assert from 'node:assert/strict';
import { hideColumn, moveColumn, visibleOrder, withHidden } from '../src/modules/database/grid-format.ts';

// Header menu → Hide column / Show all columns: per query tab, not persisted.
const cols = [{ name: 'id' }, { name: 'name' }, { name: 'email' }, { name: 'ts' }];
const all = cols.map((c) => c.name);

test('hiding adds a column, never the last visible one, never twice', () => {
  assert.deepEqual(hideColumn([], 'email', all), ['email']);
  assert.equal(hideColumn(['email'], 'email', all), null);
  assert.equal(hideColumn([], 'nope', all), null);
  assert.deepEqual(hideColumn(['id', 'name'], 'email', all), ['id', 'name', 'email']);
  assert.equal(hideColumn(['id', 'name', 'email'], 'ts', all), null, 'one column always stays');
});

test('the display order drops hidden columns and keeps the arranged order', () => {
  assert.deepEqual(visibleOrder([3, 0, 1, 2], cols, ['name']), [3, 0, 2]);
  assert.deepEqual(visibleOrder([0, 1, 2, 3], cols, []), [0, 1, 2, 3]);
});

test('dragging among visible columns keeps the hidden ones for Show all', () => {
  const full = [0, 1, 2, 3];
  const visible = visibleOrder(full, cols, ['name']); // [0, 2, 3]
  const moved = moveColumn(visible, 2, 0); // ts first
  const next = withHidden(moved, full);
  assert.deepEqual(next, [3, 0, 2, 1]);
  assert.equal(next.length, cols.length, 'the full order stays complete, so GridView keeps using it');
  assert.deepEqual(visibleOrder(next, cols, ['name']), [3, 0, 2]);
});
