import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  cellMatchesFilter,
  columnKind,
  moveColumn,
  rowNumberWidthCh,
} from '../src/modules/database/grid-format.ts';

test('column kinds from engine type hints', () => {
  for (const t of ['BIGINT', 'int unsigned', 'DECIMAL(10,2)', 'Nullable(UInt64)', 'double precision', 'numeric']) {
    assert.equal(columnKind(t), 'num', t);
  }
  assert.equal(columnKind('tinyint(1)'), 'bool');
  assert.equal(columnKind('BOOLEAN'), 'bool');
  assert.equal(columnKind('DATETIME'), 'time');
  assert.equal(columnKind('jsonb'), 'json');
  assert.equal(columnKind('VARCHAR(20)'), 'text');
  // Untyped (Mongo / computed) columns classify from their values.
  assert.equal(columnKind(null, [1, 2, null, 3.5]), 'num');
  assert.equal(columnKind(undefined, [{ a: 1 }, [1]]), 'json');
  assert.equal(columnKind(undefined, [{ $oid: 'abc' }]), 'text');
  assert.equal(columnKind(undefined, [1, 'x']), 'text');
  assert.equal(columnKind(undefined, []), 'text');
});

test('row-number column width depends on the row count only', () => {
  assert.equal(rowNumberWidthCh(5), 4);
  assert.equal(rowNumberWidthCh(999), 4);
  assert.equal(rowNumberWidthCh(1000), 5);
  assert.equal(rowNumberWidthCh(100000), 7);
});

test('moveColumn reorders display positions', () => {
  assert.deepEqual(moveColumn([0, 1, 2, 3], 0, 2), [1, 2, 0, 3]);
  assert.deepEqual(moveColumn([0, 1, 2, 3], 3, 0), [3, 0, 1, 2]);
  assert.deepEqual(moveColumn([0, 1, 2], 1, 1), [0, 1, 2]);
  assert.deepEqual(moveColumn([0, 1, 2], 5, 0), [0, 1, 2]);
});

test('header filter: contains, exact, comparisons, NULL', () => {
  assert.equal(cellMatchesFilter('Shipped', false, 'ship'), true);
  assert.equal(cellMatchesFilter('Shipped', false, '=ship'), false);
  assert.equal(cellMatchesFilter('paid', false, '=paid'), true);
  assert.equal(cellMatchesFilter('12.5', false, '>10'), true);
  assert.equal(cellMatchesFilter('9', false, '>=10'), false);
  assert.equal(cellMatchesFilter('abc', false, '<3'), false);
  assert.equal(cellMatchesFilter('', true, 'NULL'), true);
  assert.equal(cellMatchesFilter('x', false, 'null'), false);
  assert.equal(cellMatchesFilter('x', false, '!null'), true);
  assert.equal(cellMatchesFilter('', true, 'x'), false);
  assert.equal(cellMatchesFilter('anything', false, '   '), true);
});
