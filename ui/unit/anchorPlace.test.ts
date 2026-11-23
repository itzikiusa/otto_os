import test from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

function runPlace(opts: {
  vw: number;
  vh: number;
  anchorRect: { top: number; bottom: number; left: number; right: number };
  panelWidth: number;
  panelScrollHeight: number;
  dir?: 'ltr' | 'rtl';
  prefer?: 'above' | 'below';
  align?: 'start' | 'end';
  gap?: number;
}) {
  const mod = loadSource(
    new URL('../src/lib/anchorPlace.ts', import.meta.url),
    {},
    {
      window: { innerWidth: opts.vw, innerHeight: opts.vh },
      getComputedStyle: () => ({ direction: opts.dir ?? 'ltr' }),
    },
  );
  const anchor = {
    getBoundingClientRect: () => ({
      top: opts.anchorRect.top,
      bottom: opts.anchorRect.bottom,
      left: opts.anchorRect.left,
      right: opts.anchorRect.right,
      width: opts.anchorRect.right - opts.anchorRect.left,
      height: opts.anchorRect.bottom - opts.anchorRect.top,
      x: opts.anchorRect.left,
      y: opts.anchorRect.top,
      toJSON: () => ({}),
    }),
  } as unknown as Element;
  const panel = {
    offsetWidth: opts.panelWidth,
    scrollHeight: opts.panelScrollHeight,
  } as unknown as HTMLElement;
  return mod.placeAnchored(anchor, panel, {
    prefer: opts.prefer,
    align: opts.align,
    gap: opts.gap,
  }) as { top: number; left: number; maxH: number };
}

test('placeAnchored caps maxH to viewport height minus 2*margin on very short viewports', () => {
  // vh = 140 -> vh - 2*8 = 124; previously Math.max(160, ...) returned 160 and overflowed!
  const p = runPlace({
    vw: 800,
    vh: 140,
    anchorRect: { top: 60, bottom: 84, left: 100, right: 180 },
    panelWidth: 200,
    panelScrollHeight: 400,
  });
  assert.ok(p.maxH <= 140 - 16, `expected maxH <= 124, got ${p.maxH}`);
  assert.ok(p.top >= 8, `expected top >= 8, got ${p.top}`);
  const renderedH = Math.min(400, p.maxH);
  assert.ok(p.top + renderedH <= 140 - 8, `expected bottom <= 132, got ${p.top + renderedH}`);
});

test('placeAnchored enforces 80px floor only on tiny viewports while still staying clamped at vh=200', () => {
  const p = runPlace({
    vw: 400,
    vh: 200,
    anchorRect: { top: 90, bottom: 114, left: 20, right: 100 },
    panelWidth: 160,
    panelScrollHeight: 320,
  });
  assert.equal(p.maxH, 160);
  assert.ok(p.top >= 8);
  assert.ok(p.top + Math.min(320, p.maxH) <= 200 - 8);
});

test('placeAnchored respects RTL logical align start vs end and clamps left into viewport', () => {
  const ltrStart = runPlace({
    vw: 1000,
    vh: 800,
    anchorRect: { top: 100, bottom: 128, left: 400, right: 480 },
    panelWidth: 200,
    panelScrollHeight: 120,
    dir: 'ltr',
    align: 'start',
  });
  assert.equal(ltrStart.left, 400);

  const rtlStart = runPlace({
    vw: 1000,
    vh: 800,
    anchorRect: { top: 100, bottom: 128, left: 400, right: 480 },
    panelWidth: 200,
    panelScrollHeight: 120,
    dir: 'rtl',
    align: 'start',
  });
  // In RTL, logical start aligns panel right edge with anchor right edge: 480 - 200 = 280.
  assert.equal(rtlStart.left, 280);

  const clampedRight = runPlace({
    vw: 500,
    vh: 800,
    anchorRect: { top: 100, bottom: 128, left: 420, right: 490 },
    panelWidth: 200,
    panelScrollHeight: 120,
    dir: 'ltr',
    align: 'start',
  });
  assert.equal(clampedRight.left, 500 - 8 - 200);
});
