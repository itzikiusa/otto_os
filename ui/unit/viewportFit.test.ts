import { test } from 'node:test';
import assert from 'node:assert/strict';
import { fitToVisualViewport } from '../src/lib/viewportFit.ts';

test('the app follows the visible viewport and an iOS keyboard pan is undone', () => {
  const handlers: Record<string, () => void> = {};
  const vv = { height: 932, scale: 1, offsetTop: 0, addEventListener: (t: string, f: () => void) => (handlers[t] = f) };
  const scrolls: number[][] = [];
  const win = { visualViewport: vv as never, scrollY: 0, scrollTo: (x: number, y: number) => scrolls.push([x, y]) };
  const props: Record<string, string> = {};
  fitToVisualViewport(win, { style: { setProperty: (k, v) => (props[k] = v) } });
  assert.equal(props['--app-h'], '932px');
  assert.deepEqual(scrolls, []);
  // Keyboard up: the visible viewport shrinks and iOS pans the page up.
  Object.assign(vv, { height: 520.4, offsetTop: 300 });
  handlers.resize();
  assert.equal(props['--app-h'], '520px');
  assert.deepEqual(scrolls, [[0, 0]]);
  // A pan left behind after the keyboard closed is undone too.
  Object.assign(vv, { height: 932, offsetTop: 0 });
  win.scrollY = 120;
  handlers.scroll();
  assert.equal(props['--app-h'], '932px');
  assert.equal(scrolls.length, 2);
  // Pinch-zoom is the user's: left alone.
  Object.assign(vv, { scale: 2, height: 466 });
  handlers.resize();
  assert.equal(props['--app-h'], '932px');
  assert.equal(scrolls.length, 2);
});
