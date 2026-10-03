import { test } from 'node:test';
import assert from 'node:assert/strict';
import { annosHash, uploadNeeded, type Anno } from '../src/modules/snip/annotations.ts';

const rect = (id: number, x2 = 40): Anno => ({
  id,
  tool: 'rect',
  x1: 0,
  y1: 0,
  x2,
  y2: 30,
  color: '#e5484d',
  stroke: 3,
  font: 16,
});

/** The editor's copy loop, minus the canvas: flatten + upload only when the
 *  gate says the annotations changed since the last successful save. */
function simulate(states: Anno[][]): number {
  let saved = annosHash([]);
  let uploads = 0;
  for (const annos of states) {
    const next = uploadNeeded(annos, saved);
    if (next === null) continue;
    uploads += 1;
    saved = next;
  }
  return uploads;
}

test('an unchanged annotation list makes zero uploads', () => {
  // Opening the editor and idling (or select-clicking) never uploads.
  assert.equal(simulate([[], [], []]), 0);
  // One real edit uploads once; idles after it re-run the gate for free.
  const one = [rect(1)];
  assert.equal(simulate([one, one, [...one], [rect(1)]]), 1);
});

test('only a state that differs from the last save uploads', () => {
  const a = [rect(1)];
  const b = [rect(1), rect(2, 80)];
  // save a → edit to b (upload) → undo to a (upload: differs from b) →
  // redo to b (upload) → b again (skip).
  assert.equal(simulate([a, b, a, b, b]), 4);
  // Edit then undo before the idle save fires: the saved state is reached
  // again, so nothing is sent.
  assert.equal(uploadNeeded([], annosHash([])), null);
});

test('a real change is detected (geometry, text and order)', () => {
  const base = annosHash([rect(1)]);
  assert.notEqual(uploadNeeded([rect(1, 41)], base), null);
  assert.notEqual(uploadNeeded([{ ...rect(1), text: 'hi' }], base), null);
  assert.notEqual(uploadNeeded([rect(2), rect(1)], annosHash([rect(1), rect(2)])), null);
});
