import { test } from 'node:test';
import assert from 'node:assert/strict';
import { motionMs, reducedMotion, scrollBehavior } from '../src/lib/motion.ts';

const g = globalThis as { window?: unknown };

function withReduce(reduce: boolean | 'throws', fn: () => void): void {
  const prev = g.window;
  g.window = {
    matchMedia: (q: string) => {
      if (reduce === 'throws') throw new Error('no matchMedia');
      return { matches: reduce && q === '(prefers-reduced-motion: reduce)' };
    },
  };
  try {
    fn();
  } finally {
    g.window = prev;
  }
}

test('script motion is smooth by default and instant under reduced motion', () => {
  withReduce(false, () => {
    assert.equal(reducedMotion(), false);
    assert.equal(scrollBehavior(), 'smooth');
    assert.equal(motionMs(160), 160);
  });
  withReduce(true, () => {
    assert.equal(reducedMotion(), true);
    assert.equal(scrollBehavior(), 'auto');
    assert.equal(motionMs(160), 0);
  });
});

test('reducedMotion never throws without a usable matchMedia', () => {
  withReduce('throws', () => assert.equal(reducedMotion(), false));
  const prev = g.window;
  delete g.window;
  try {
    assert.equal(reducedMotion(), false);
  } finally {
    g.window = prev;
  }
});
