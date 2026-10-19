import { test } from 'node:test';
import assert from 'node:assert/strict';
import { PointerZoom } from '../src/modules/canvas/pointerZoom.ts';

test('pinch keeps the diagram point under the moving midpoint', () => {
  const pointers = new PointerZoom();
  pointers.down(1, 100, 100);
  pointers.down(2, 200, 100);
  const moved = pointers.move(2, 300, 100, { scale: 1, tx: 0, ty: 0 });
  assert.deepEqual(moved, { scale: 2, tx: -100, ty: -100 });
  assert.equal(150 * moved.scale + moved.tx, 200);
  pointers.up(1);
  assert.deepEqual(pointers.move(2, 310, 120, moved), { scale: 2, tx: -90, ty: -80 });
});

test('cancelled and unknown pointers cannot move the diagram; scale stays bounded', () => {
  const pointers = new PointerZoom();
  const initial = { scale: 1, tx: 0, ty: 0 };
  assert.deepEqual(pointers.move(9, 100, 100, initial), initial);
  pointers.down(1, 0, 0);
  pointers.down(2, 1, 0);
  const moved = pointers.move(2, 1000, 0, initial);
  assert.equal(moved.scale, 6);
  pointers.up(1);
  pointers.up(2);
  assert.equal(pointers.active, false);
  assert.deepEqual(pointers.move(2, 2000, 0, moved), moved);
});
