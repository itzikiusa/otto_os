/// <reference lib="dom" />
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { MAX_SCALE, MIN_SCALE, PanZoom, fitTransform, zoomAt, type Transform } from '../src/modules/canvas/pointerZoom.ts';

// The shared pan/zoom behind MermaidCanvas + D2Canvas: a fake 400×300 surface
// at (10, 20) on the page and a plain transform cell standing in for the rune.
function harness(start: Transform = { scale: 1, tx: 0, ty: 0 }) {
  let view = start;
  let adjusted = false;
  const surface = { getBoundingClientRect: () => ({ left: 10, top: 20, width: 400, height: 300 }) } as unknown as HTMLElement;
  const dragging: boolean[] = [];
  const pz = new PanZoom({
    surface: () => surface, get: () => view, set: (t) => (view = t),
    adjusted: () => adjusted, setAdjusted: (on) => (adjusted = on), onDragging: (on) => dragging.push(on),
  });
  return { pz, view: () => view, adjusted: () => adjusted, dragging };
}
const ev = <T>(o: Record<string, unknown>) => ({ preventDefault() {}, ...o }) as unknown as T;

test('fit centres the diagram with a margin and caps tiny diagrams', () => {
  assert.deepEqual(fitTransform(400, 300, 800, 600), { scale: 0.46, tx: 16, ty: 12 });
  assert.equal(fitTransform(400, 300, 10, 10).scale, 4, 'fit stops at FIT_MAX_SCALE');
});

test('zoomAt keeps the anchor point still and clamps to the scale bounds', () => {
  const t = zoomAt({ scale: 1, tx: 0, ty: 0 }, 2, 100, 50);
  assert.deepEqual(t, { scale: 2, tx: -100, ty: -50 });
  assert.equal(zoomAt(t, 100, 0, 0).scale, MAX_SCALE);
  assert.equal(zoomAt(t, 0.0001, 0, 0).scale, MIN_SCALE);
});

test('wheel pans, ctrl+wheel zooms about the cursor, and both mark the view adjusted', () => {
  const h = harness();
  h.pz.wheel(ev<WheelEvent>({ ctrlKey: false, deltaX: 5, deltaY: 7, clientX: 0, clientY: 0 }));
  assert.deepEqual(h.view(), { scale: 1, tx: -5, ty: -7 });
  assert.equal(h.adjusted(), true);
  h.pz.wheel(ev<WheelEvent>({ ctrlKey: true, deltaX: 0, deltaY: -Math.log(2) / 0.0015, clientX: 110, clientY: 70 }));
  const v = h.view();
  assert.ok(Math.abs(v.scale - 2) < 1e-9);
  // The surface point under the cursor (100, 50) is the same diagram point before and after.
  assert.ok(Math.abs((100 - v.tx) / v.scale - (100 + 5)) < 1e-9);
  h.pz.fitView();
  assert.equal(h.adjusted(), false, 'fitView hands control back to auto-fit');
});

test('arrow keys pan only from the surface itself; drag pans and reports the grab', () => {
  const h = harness();
  const target = {};
  h.pz.key(ev<KeyboardEvent>({ key: 'ArrowRight', target, currentTarget: target }));
  assert.deepEqual(h.view(), { scale: 1, tx: -40, ty: 0 });
  h.pz.key(ev<KeyboardEvent>({ key: 'ArrowRight', target: {}, currentTarget: target }));
  assert.equal(h.view().tx, -40, 'a key from a child control is left alone');

  const el = { setPointerCapture() {}, hasPointerCapture: () => true, releasePointerCapture() {} };
  h.pz.pointerDown(ev<PointerEvent>({ pointerId: 1, clientX: 60, clientY: 70, target: { closest: () => null }, currentTarget: el }));
  h.pz.pointerMove(ev<PointerEvent>({ pointerId: 1, clientX: 80, clientY: 75 }));
  assert.deepEqual(h.view(), { scale: 1, tx: -20, ty: 5 });
  h.pz.pointerUp(ev<PointerEvent>({ pointerId: 1, currentTarget: el }));
  h.pz.pointerMove(ev<PointerEvent>({ pointerId: 1, clientX: 500, clientY: 500 }));
  assert.deepEqual(h.view(), { scale: 1, tx: -20, ty: 5 }, 'no movement after release');
  assert.deepEqual(h.dragging, [true, false]);
});
