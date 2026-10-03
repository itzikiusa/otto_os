import { test } from 'node:test';
import assert from 'node:assert/strict';
import { annosHash, type Anno } from '../src/modules/snip/annotations.ts';

const rect = (id: number, x: number): Anno =>
  ({ id, tool: 'rect', color: '#ff0000', stroke: 4, font: 24, x1: x, y1: 0, x2: x + 10, y2: 10 }) as Anno;

test('annosHash is stable for equal states and changes on any edit (upload skip key)', () => {
  assert.equal(annosHash([]), annosHash([]));
  assert.equal(annosHash([rect(1, 0)]), annosHash([rect(1, 0)]));
  assert.notEqual(annosHash([rect(1, 0)]), annosHash([rect(1, 1)]));
  assert.notEqual(annosHash([]), annosHash([rect(1, 0)]));
  // Undo back to a saved state yields the saved fingerprint again.
  const saved = annosHash([rect(1, 0)]);
  assert.equal(annosHash([rect(1, 5)].map((a) => ({ ...a, x1: 0, x2: 10 }))), saved);
});
