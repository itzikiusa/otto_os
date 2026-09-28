import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readNativePaneContext, paneBounds, LatestPaneLayout, nativePanePresence } from '../src/lib/nativePanePolicy.ts';

test('native embedded context requires Tauri and an injected host, never query flags', () => {
  assert.equal(readNativePaneContext(false, { host: 'main' }), null);
  assert.equal(readNativePaneContext(true, null), null);
  assert.equal(readNativePaneContext(true, { host: 'https://remote' }), null);
  assert.deepEqual(readNativePaneContext(true, { host: 'main' }), { host: 'main' });
  assert.deepEqual(readNativePaneContext(true, { host: 'w12' }), { host: 'w12' });
});

test('native pane geometry rejects hidden or invalid rectangles and clips to viewport', () => {
  assert.equal(paneBounds({ x: 0, y: 0, width: 0, height: 20 }, 900, 700), null);
  assert.equal(paneBounds({ x: NaN, y: 0, width: 100, height: 20 }, 900, 700), null);
  assert.deepEqual(paneBounds({ x: -4, y: 30, width: 500, height: 800 }, 900, 700), { x: 0, y: 30, width: 496, height: 670 });
});

test('native presence excludes hidden pane even while WebKit reports visible and focused', () => {
  assert.deepEqual(nativePanePresence(true, true, false), { visible: false, focused: false });
  assert.deepEqual(nativePanePresence(true, false, true), { visible: true, focused: false });
  assert.deepEqual(nativePanePresence(true, true, true), { visible: true, focused: true });
});

test('layout coalesces during IPC and final hide cannot be overtaken by stale geometry', async () => {
  const sent: number[] = [];
  let finish!: () => void;
  const layout = new LatestPaneLayout<number>(async (value) => {
    sent.push(value);
    if (value === 1) await new Promise<void>((resolve) => { finish = resolve; });
  });
  layout.push(1); layout.push(2); layout.push(3);
  assert.deepEqual(sent, [1]);
  finish(); await layout.flush();
  assert.deepEqual(sent, [1, 3]);
  layout.stop(); layout.push(4); await layout.flush();
  assert.deepEqual(sent, [1, 3]);
});

test('layout continues with newest state after rejected native IPC', async () => {
  const sent: number[] = [];
  const layout = new LatestPaneLayout<number>(async (value) => { sent.push(value); if (value === 1) throw new Error('closed'); });
  layout.push(1); layout.push(2); await layout.flush();
  assert.deepEqual(sent, [1, 2]);
});

test('detached surfaces remain addressable when the host split is narrow', async () => {
  const { paneSurfaceVisible } = await import('../src/lib/nativePanePolicy.ts');
  assert.equal(paneSurfaceVisible(false, true, true), true);
  assert.equal(paneSurfaceVisible(false, false, true), false);
  assert.equal(paneSurfaceVisible(true, false, false), false);
  assert.equal(paneSurfaceVisible(true, false, undefined), true);
});

test('native layout scales CSS geometry by page zoom in window logical points', () => {
  const rect = { x: 400, y: 40, width: 300, height: 600 };
  for (const zoom of [0.6, 1.5, 2]) {
    assert.deepEqual(paneBounds(rect, 1000, 700, zoom), {
      x: 400 * zoom, y: 40 * zoom, width: 300 * zoom, height: 600 * zoom,
    });
  }
});

test('overlay focus restoration is cancelled when detach or a new overlay wins the race', async () => {
  const { mayRestorePaneFocus } = await import('../src/lib/nativePanePolicy.ts');
  assert.equal(mayRestorePaneFocus('attached', true, false), true);
  assert.equal(mayRestorePaneFocus('primary', true, false), false);
  assert.equal(mayRestorePaneFocus('side', true, false), false);
  assert.equal(mayRestorePaneFocus('attached', false, false), false);
  assert.equal(mayRestorePaneFocus('attached', true, true), false);
});

test('native menus honor primary window focus after macOS window cycling leaves a stale side flag', async () => {
  const { nativeSideFocused } = await import('../src/lib/nativePanePolicy.ts');
  const { sideMenuTarget } = await import('../src/lib/sidePane.ts');
  const staleSideReport = true;
  assert.equal(sideMenuTarget('close-tab', nativeSideFocused(staleSideReport, true), 'connections'), 'main');
  assert.equal(sideMenuTarget('select-all', nativeSideFocused(staleSideReport, true), 'connections'), 'main');
  assert.equal(sideMenuTarget('close-tab', nativeSideFocused(true, false), 'connections'), 'close-pane');
  assert.equal(sideMenuTarget('select-all', nativeSideFocused(true, false), 'connections'), 'side');
});
