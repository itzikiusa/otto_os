// Wave 3 (CAN — C-11) unit tests for ui/src/modules/git/.
// Pure TypeScript helper & module tests only (zero .svelte regex reads).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { register } from 'node:module';

const hook =
  'export async function resolve(s, c, n) {' +
  "  if (/^\\.{1,2}\\//.test(s) && !/\\.[a-z]+$/i.test(s)) { try { return await n(s + '.ts', c); } catch {} }" +
  '  return n(s, c);' +
  '}';
register('data:text/javascript,' + encodeURIComponent(hook));

const { clampLinePickPos } = await import('../src/modules/git/gitUiHelpers.ts');

test('clampLinePickPos places the button under the selection when it fits', () => {
  const pos = clampLinePickPos(
    { top: 100, bottom: 120, left: 240 },
    { width: 220, height: 34 },
    { width: 1280, height: 800 },
    8,
  );
  assert.deepEqual(pos, { top: 128, left: 240 });
});

test('clampLinePickPos re-clamps wide range labels near the right edge inside the viewport', () => {
  const rect = { top: 200, bottom: 220, left: 900 };
  const viewport = { width: 1000, height: 700 };
  // Frame-1 fallback width (200px) leaves left at 792; once mounted at 280px
  // wide, re-clamping shifts left to 712 so the right edge stays inside 1000-8.
  const estimate = clampLinePickPos(rect, { width: 200, height: 34 }, viewport, 8);
  const measured = clampLinePickPos(rect, { width: 280, height: 34 }, viewport, 8);
  assert.equal(estimate.left, 792);
  assert.equal(measured.left, 712);
  assert.ok(measured.left + 280 <= viewport.width - 8);
});

test('clampLinePickPos flips above the selection near the bottom edge and floors at gap', () => {
  const flipped = clampLinePickPos(
    { top: 650, bottom: 680, left: 40 },
    { width: 220, height: 40 },
    { width: 1000, height: 700 },
    8,
  );
  assert.equal(flipped.top, 602);
  assert.equal(flipped.left, 40);

  // Selection taller/wider than a very tight viewport still floors at gap (never negative).
  const floored = clampLinePickPos(
    { top: 10, bottom: 690, left: 300 },
    { width: 400, height: 44 },
    { width: 320, height: 500 },
    8,
  );
  assert.deepEqual(floored, { top: 8, left: 8 });
});
